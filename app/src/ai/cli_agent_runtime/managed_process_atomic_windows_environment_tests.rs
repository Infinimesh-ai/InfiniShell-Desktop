//! 原生对照专用的只读环境观察；所有字段都是诊断，不参与授权或清理判定。

#![cfg(all(test, windows))]

use std::io::{self, Write as _};
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use serde::Serialize;
use serde_json::{Value, json};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    TokenElevation, TokenIntegrityLevel, TokenIsAppContainer, TokenSessionId,
};
use windows::Win32::System::Diagnostics::Debug::{CREATE_PROCESS_DEBUG_EVENT, DEBUG_EVENT};
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW, UOI_FLAGS, UOI_NAME,
    USEROBJECTFLAGS,
};
use windows::Win32::System::SystemServices::{
    PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY, PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessId,
    GetProcessMitigationPolicy, GetThreadId, OpenProcessToken, ProcessStrictHandleCheckPolicy,
    ProcessSystemCallDisablePolicy,
};
use windows::Win32::UI::WindowsAndMessaging::WSF_VISIBLE;
use windows::core::Error as WindowsError;

const NAME_UNITS: usize = 512;
const TOKEN_WORDS: usize = 64;

fn unavailable(reason: &'static str) -> Value {
    json!({ "reason": reason })
}

fn api_error(error: WindowsError) -> Value {
    let hresult = error.code().0;
    let bits = hresult as u32;
    let os_code = (bits & 0xffff_0000 == 0x8007_0000).then_some(bits & 0xffff);
    // 不格式化系统错误正文，避免将对象名称或其他上下文写入诊断。
    json!({ "reason": "query_failed", "hresult": hresult, "os_code": os_code })
}

fn reported<T: Serialize>(result: Result<T, Value>) -> Value {
    match result {
        Ok(value) => json!({ "value": value }),
        Err(error) => json!({ "error": error }),
    }
}

fn safe_role(role: &str) -> &'static str {
    match role {
        "root" => "root",
        "node" => "node",
        "codex" => "codex",
        "console" => "console",
        "bound-other" => "bound-other",
        _ => "unknown",
    }
}

fn name_payload(buffer: &[u16], bytes: u32) -> Result<&[u16], Value> {
    let bytes = bytes as usize;
    if bytes == 0 || bytes % size_of::<u16>() != 0 || bytes > NAME_UNITS * size_of::<u16>() {
        return Err(unavailable("name_length_out_of_bounds"));
    }
    let data = buffer
        .get(..bytes / size_of::<u16>())
        .ok_or_else(|| unavailable("name_length_out_of_bounds"))?;
    let Some((&0, name)) = data.split_last() else {
        return Err(unavailable("name_not_terminated"));
    };
    if name.is_empty() || name.contains(&0) {
        return Err(unavailable("name_payload_invalid"));
    }
    Ok(name)
}

fn object_name(handle: HANDLE) -> Result<Vec<u16>, Value> {
    let mut buffer = [0u16; NAME_UNITS];
    let mut needed = 0;
    unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(buffer.as_mut_ptr().cast()),
            size_of::<[u16; NAME_UNITS]>() as u32,
            Some(&mut needed),
        )
    }
    .map_err(api_error)?;
    // 固定上限、单次查询；不依据外部返回的长度无限分配或重试。
    name_payload(&buffer, needed).map(<[u16]>::to_vec)
}

fn name_class(name: &[u16], expected: &str, matched: &'static str) -> &'static str {
    let expected = expected.as_bytes();
    if name.len() == expected.len()
        && name.iter().zip(expected).all(|(&unit, &byte)| {
            u8::try_from(unit).is_ok_and(|unit| unit.eq_ignore_ascii_case(&byte))
        })
    {
        matched
    } else {
        "other"
    }
}

fn station_visible(handle: HANDLE) -> Result<bool, Value> {
    let mut flags = USEROBJECTFLAGS::default();
    let mut needed = 0;
    unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_FLAGS,
            Some((&mut flags as *mut USEROBJECTFLAGS).cast()),
            size_of::<USEROBJECTFLAGS>() as u32,
            Some(&mut needed),
        )
    }
    .map_err(api_error)?;
    if needed as usize != size_of::<USEROBJECTFLAGS>() {
        return Err(unavailable("flags_length_mismatch"));
    }
    Ok(flags.dwFlags & WSF_VISIBLE as u32 != 0)
}

fn token_u32(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<u32, Value> {
    let mut value = 0u32;
    let mut needed = 0;
    unsafe {
        GetTokenInformation(
            token,
            class,
            Some((&mut value as *mut u32).cast()),
            size_of::<u32>() as u32,
            &mut needed,
        )
    }
    .map_err(api_error)?;
    if needed as usize != size_of::<u32>() {
        return Err(unavailable("token_length_mismatch"));
    }
    Ok(value)
}

fn integrity_rid(bytes: &[u8], offset: usize) -> Result<u32, Value> {
    let sid = bytes
        .get(offset..)
        .ok_or_else(|| unavailable("integrity_sid_out_of_bounds"))?;
    let header = sid
        .get(..8)
        .ok_or_else(|| unavailable("integrity_sid_out_of_bounds"))?;
    let count = usize::from(header[1]);
    if header[0] != 1 || !(1..=15).contains(&count) || header[2..] != [0, 0, 0, 0, 0, 16] {
        return Err(unavailable("integrity_sid_invalid"));
    }
    let rid = sid
        .get(8 + (count - 1) * 4..8 + count * 4)
        .ok_or_else(|| unavailable("integrity_sid_out_of_bounds"))?;
    Ok(u32::from_le_bytes([rid[0], rid[1], rid[2], rid[3]]))
}

fn token_integrity(token: HANDLE) -> Result<Value, Value> {
    let mut buffer = [0usize; TOKEN_WORDS];
    let mut needed = 0;
    unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            size_of::<[usize; TOKEN_WORDS]>() as u32,
            &mut needed,
        )
    }
    .map_err(api_error)?;
    let length = needed as usize;
    if !(size_of::<TOKEN_MANDATORY_LABEL>()..=size_of::<[usize; TOKEN_WORDS]>()).contains(&length) {
        return Err(unavailable("integrity_length_out_of_bounds"));
    }
    let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let offset = (label.Label.Sid.0 as usize)
        .checked_sub(buffer.as_ptr() as usize)
        .filter(|offset| *offset >= size_of::<TOKEN_MANDATORY_LABEL>())
        .ok_or_else(|| unavailable("integrity_sid_out_of_bounds"))?;
    // SID 指针先限制在系统已写入的有界缓冲中，仅输出完整性 RID，绝不输出 SID。
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), length) };
    let rid = integrity_rid(bytes, offset)?;
    let class = match rid {
        0 => "untrusted",
        0x1000 => "low",
        0x2000 => "medium",
        0x2100 => "medium_plus",
        0x3000 => "high",
        0x4000 => "system",
        0x5000 => "protected",
        _ => "other",
    };
    Ok(json!({ "rid": rid, "class": class }))
}

fn process_token(process: HANDLE) -> Result<Value, Value> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(api_error)?;
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let handle = HANDLE(token.as_raw_handle());
    Ok(json!({
        "session_id": reported(token_u32(handle, TokenSessionId)),
        "integrity": reported(token_integrity(handle)),
        "is_appcontainer": reported(token_u32(handle, TokenIsAppContainer)),
        "elevation": reported(token_u32(handle, TokenElevation)),
    }))
}

fn system_call_policy(process: HANDLE) -> Result<Value, Value> {
    let mut policy = PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY::default();
    unsafe {
        GetProcessMitigationPolicy(
            process,
            ProcessSystemCallDisablePolicy,
            (&mut policy as *mut PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY).cast(),
            size_of::<PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY>(),
        )
    }
    .map_err(api_error)?;
    let flags = unsafe { policy.Anonymous.Flags };
    Ok(json!({ "flags": flags, "disallow_win32k_system_calls": flags & 1 != 0 }))
}

fn strict_handle_policy(process: HANDLE) -> Result<Value, Value> {
    let mut policy = PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY::default();
    unsafe {
        GetProcessMitigationPolicy(
            process,
            ProcessStrictHandleCheckPolicy,
            (&mut policy as *mut PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY).cast(),
            size_of::<PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY>(),
        )
    }
    .map_err(api_error)?;
    let flags = unsafe { policy.Anonymous.Flags };
    Ok(json!({
        "flags": flags,
        "raise_exception_on_invalid_handle": flags & 1 != 0,
        "permanently_enabled": flags & 2 != 0,
    }))
}

fn process_observation(process: HANDLE) -> Value {
    json!({
        "token": reported(process_token(process)),
        "system_call_policy": reported(system_call_policy(process)),
        "strict_handle_policy": reported(strict_handle_policy(process)),
    })
}

fn environment(event: &DEBUG_EVENT) -> Result<Value, Value> {
    if event.dwDebugEventCode != CREATE_PROCESS_DEBUG_EVENT {
        return Err(unavailable("not_create_process_event"));
    }
    let information = unsafe { event.u.CreateProcessInfo };
    if information.hProcess.is_invalid() || information.hThread.is_invalid() {
        return Err(unavailable("missing_event_handle"));
    }
    let process_id = unsafe { GetProcessId(information.hProcess) };
    if process_id == 0 {
        return Err(api_error(WindowsError::from_thread()));
    }
    if process_id != event.dwProcessId {
        return Err(unavailable("process_identity_mismatch"));
    }
    let thread_id = unsafe { GetThreadId(information.hThread) };
    let bound_desktop = if thread_id == 0 {
        Err(api_error(WindowsError::from_thread()))
    } else if thread_id != event.dwThreadId {
        Err(unavailable("thread_identity_mismatch"))
    } else {
        unsafe { GetThreadDesktop(thread_id) }
            .map_err(api_error)
            .and_then(|desktop| object_name(HANDLE(desktop.0)))
    };
    // 这些窗口站与桌面句柄都是借用值，不调用 CloseWindowStation/CloseDesktop。
    let current_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        .map_err(api_error)
        .and_then(|desktop| object_name(HANDLE(desktop.0)));
    let same_name_exact = match (&current_desktop, &bound_desktop) {
        (Ok(current), Ok(bound)) => Some(current == bound),
        _ => None,
    };
    let station = unsafe { GetProcessWindowStation() }
        .map_err(api_error)
        .map(|station| {
            let handle = HANDLE(station.0);
            json!({
                "name_class": reported(object_name(handle)
                    .map(|name| name_class(&name, "WinSta0", "interactive_winsta0"))),
                "visible": reported(station_visible(handle)),
            })
        });
    Ok(json!({
        "runner": process_observation(unsafe { GetCurrentProcess() }),
        "bound_process": process_observation(information.hProcess),
        "runner_window_station": reported(station),
        "runner_desktop": reported(current_desktop
            .map(|name| name_class(&name, "Default", "default"))),
        "bound_thread_desktop": reported(bound_desktop
            .map(|name| name_class(&name, "Default", "default"))),
        "desktop_same_name_exact": same_name_exact,
        "desktop_same_object": "not_established",
        "dacl_simulation": "not_collected",
        "full_runtime_access": "not_established",
    }))
}

/// 仅由已完成 CREATE 映像、Job 和 AppContainer 核验的测试钩子调用。
pub(super) fn observe_bound_process(event: &DEBUG_EVENT, role: &str) {
    // 数值身份只关联本轮日志；查询始终使用已持有的事件句柄，不按 PID 重新打开。
    let observation = json!({
        "role": safe_role(role),
        "driver_pid": unsafe { GetCurrentProcessId() },
        "event_pid": event.dwProcessId,
        "event_tid": event.dwThreadId,
        "observation": reported(environment(event)),
    });
    let _ = writeln!(
        io::stderr().lock(),
        "atomic_windows_environment={observation}"
    );
}

#[test]
fn environment_names_and_roles_never_emit_unrecognized_text() {
    let name: Vec<_> = "private-user-desktop".encode_utf16().collect();
    assert_eq!(name_class(&name, "Default", "default"), "other");
    assert_eq!(safe_role("private-image.exe"), "unknown");
    assert_eq!(safe_role("node"), "node");
    assert_eq!(
        name_class(&[b'D' as u16, b'e' as u16], "Default", "default"),
        "other"
    );
    let name: Vec<_> = "dEfAuLt".encode_utf16().collect();
    assert_eq!(name_class(&name, "Default", "default"), "default");
}

#[test]
fn environment_name_buffers_reject_truncation_and_unbounded_lengths() {
    assert!(name_payload(&[b'A' as u16, 0], 4).is_ok());
    for bytes in [0, 1, 3, 6, u32::MAX] {
        assert!(name_payload(&[b'A' as u16, 0], bytes).is_err());
    }
    assert!(name_payload(&[b'A' as u16, b'B' as u16], 4).is_err());
    assert!(name_payload(&[b'A' as u16, 0, 0], 6).is_err());
    assert!(
        name_payload(
            &vec![b'A' as u16; NAME_UNITS + 1],
            (NAME_UNITS as u32 + 1) * 2
        )
        .is_err()
    );
}

#[test]
fn environment_integrity_rejects_foreign_and_truncated_sids() {
    let mut sid = [1, 1, 0, 0, 0, 0, 0, 16, 0, 0x20, 0, 0];
    assert_eq!(integrity_rid(&sid, 0).unwrap(), 0x2000);
    assert!(integrity_rid(&sid[..11], 0).is_err());
    assert!(integrity_rid(&sid, usize::MAX).is_err());
    sid[7] = 5;
    assert!(integrity_rid(&sid, 0).is_err());
    sid[7] = 16;
    for count in [0, 16, u8::MAX] {
        sid[1] = count;
        assert!(integrity_rid(&sid, 0).is_err());
    }
}

#[test]
fn environment_query_failure_does_not_look_like_zero_flags() {
    let value: Value = reported::<u32>(Err(api_error(WindowsError::from_hresult(
        windows::core::HRESULT::from_win32(5),
    ))));
    assert!(value.get("value").is_none());
    assert_eq!(value["error"]["os_code"], 5);
    assert_eq!(
        environment(&DEBUG_EVENT::default()).unwrap_err()["reason"],
        "not_create_process_event"
    );
}
