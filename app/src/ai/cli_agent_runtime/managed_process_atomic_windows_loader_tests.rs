//! 固定 Node 对照的只读 loader 观察；不编入产品程序。

use std::io::Write as _;

use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;

use super::*;

const MAX_TRACE_BYTES: usize = 24 * 1024;
const MAX_DEBUG_STRING_BYTES: usize = 512;

#[derive(Debug, Default)]
pub(super) struct LoaderTrace {
    emitted_bytes: usize,
    dropped_events: usize,
    dll_counts: HashMap<u32, usize>,
    pub(super) bound_console_exit_events: usize,
}

impl LoaderTrace {
    pub(super) fn observe(&mut self, event: &DEBUG_EVENT, session: &WindowsImageDebugSession) {
        let role = session
            .npm_diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.roles.get(&event.dwProcessId))
            .copied()
            .unwrap_or(NpmProcessRole::Unknown)
            .as_str();
        let details = match event.dwDebugEventCode {
            LOAD_DLL_DEBUG_EVENT => {
                *self.dll_counts.entry(event.dwProcessId).or_default() += 1;
                // 仅查询原事件句柄，不接管、不复制、不关闭，也不按路径重新打开。
                let information = unsafe { event.u.LoadDll };
                serde_json::json!({
                    "event": "load_dll",
                    "basename": dll_basename(information.hFile),
                })
            }
            EXCEPTION_DEBUG_EVENT => {
                let information = unsafe { event.u.Exception };
                serde_json::json!({
                    "event": "exception",
                    "code": format!("0x{:08x}", information.ExceptionRecord.ExceptionCode.0 as u32),
                    "first_chance": information.dwFirstChance != 0,
                    "initial_breakpoint_candidate": information.ExceptionRecord.ExceptionCode == EXCEPTION_BREAKPOINT
                        && !session.initial_breakpoints.contains(&event.dwProcessId),
                })
            }
            OUTPUT_DEBUG_STRING_EVENT => {
                let information = unsafe { event.u.DebugString };
                let declared_bytes = usize::from(information.nDebugStringLength);
                let mut bytes = vec![0_u8; declared_bytes.min(MAX_DEBUG_STRING_BYTES)];
                let mut read = 0;
                // 只读固定夹具的原 CREATE 句柄副本；不按 PID 重开或写入进程内存。
                let result = session
                    .processes
                    .get(&event.dwProcessId)
                    .map(|process| unsafe {
                        ReadProcessMemory(
                            HANDLE(process.as_raw_handle()),
                            information.lpDebugStringData.0.cast(),
                            bytes.as_mut_ptr().cast(),
                            bytes.len(),
                            Some(&mut read),
                        )
                    });
                let read_ok = result.as_ref().is_some_and(|result| result.is_ok());
                bytes.truncate(read.min(bytes.len()));
                // 调试文本可能含完整路径；仅输出错误码及 DLL 基名，不输出原文。
                let text = decode_debug_string(&bytes, information.fUnicode != 0);
                serde_json::json!({
                    "event": "debug_string",
                    "declared_bytes_lower16": declared_bytes,
                    "read_bytes": bytes.len(),
                    "read_ok": read_ok,
                    "capped": declared_bytes > MAX_DEBUG_STRING_BYTES,
                    "tokens": loader_tokens(&text),
                })
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                // 角色仅在原 CREATE 的身份、Job 和令牌全部核验后赋值。
                // 这里只记收到 EXIT；调用方还必须要求完整 drain 和清理成功。
                if role == "console"
                    && session
                        .npm_diagnostics
                        .as_ref()
                        .is_some_and(|diagnostics| !diagnostics.cleanup)
                {
                    self.bound_console_exit_events += 1;
                }
                serde_json::json!({
                    "event": "loader_exit_summary",
                    "dll_events": self.dll_counts.remove(&event.dwProcessId).unwrap_or(0),
                    "initial_breakpoint_observed": session.initial_breakpoints.contains(&event.dwProcessId),
                    "dropped_events": self.dropped_events,
                })
            }
            CREATE_PROCESS_DEBUG_EVENT
            | CREATE_THREAD_DEBUG_EVENT
            | EXIT_THREAD_DEBUG_EVENT
            | UNLOAD_DLL_DEBUG_EVENT
            | RIP_EVENT => return,
            _ => return,
        };
        let output = serde_json::json!({"role": role, "details": details}).to_string();
        let is_summary = event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT;
        if reserve_trace_bytes(&mut self.emitted_bytes, output.len(), is_summary) {
            // stderr 失败也不能 panic 并绕过尚未继续事件的正常清理路径。
            let _ = writeln!(io::stderr().lock(), "atomic_windows_loader={output}");
        } else {
            self.dropped_events += 1;
        }
    }
}

fn dll_basename(handle: HANDLE) -> Option<String> {
    let mut path = vec![0_u16; MAX_WINDOWS_PATH_U16];
    let length = unsafe { GetFinalPathNameByHandleW(handle, &mut path, VOLUME_NAME_DOS) } as usize;
    if length == 0 || length >= path.len() {
        return None;
    }
    let path = PathBuf::from(OsString::from_wide(&path[..length]));
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.len() <= 128)
        .map(str::to_owned)
}

fn reserve_trace_bytes(total: &mut usize, message_bytes: usize, is_summary: bool) -> bool {
    let Some(next) = total
        .checked_add(message_bytes)
        .and_then(|size| size.checked_add(32))
    else {
        return false;
    };
    // 为本夹具至多三个进程的退出摘要保留 1 KiB，截断计数仍可见。
    let limit = if is_summary {
        MAX_TRACE_BYTES
    } else {
        MAX_TRACE_BYTES - 1024
    };
    if next > limit {
        return false;
    }
    *total = next;
    true
}

fn decode_debug_string(bytes: &[u8], unicode: bool) -> String {
    if unicode {
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn loader_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| {
        !character.is_ascii_alphanumeric()
            && character != '.'
            && character != '_'
            && character != '-'
    })
    .filter(|token| {
        token.len() <= 128
            && (token.to_ascii_lowercase().ends_with(".dll")
                || token.strip_prefix("0x").is_some_and(|hex| {
                    !hex.is_empty()
                        && hex.len() <= 8
                        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                }))
    })
    .take(16)
    .map(str::to_owned)
    .collect()
}

#[test]
fn trace_budget_rejects_overflow_without_consuming_remaining_space() {
    let mut total = MAX_TRACE_BYTES - 64;
    assert!(!reserve_trace_bytes(&mut total, usize::MAX, true));
    assert_eq!(total, MAX_TRACE_BYTES - 64);
    assert!(!reserve_trace_bytes(&mut total, 32, false));
    assert!(reserve_trace_bytes(&mut total, 32, true));
    assert!(!reserve_trace_bytes(&mut total, 0, true));
}

#[test]
fn loader_debug_tokens_discard_paths_and_unrelated_text() {
    assert_eq!(
        loader_tokens(r"user secret C:\private\kernel32.dll failed 0xc0000142 token=secret"),
        ["kernel32.dll", "0xc0000142"]
    );
    assert!(loader_tokens("user_secret 0xnothex secret.exe").is_empty());
}

#[test]
fn loader_debug_unicode_ignores_incomplete_last_unit() {
    assert_eq!(decode_debug_string(&[b'A', 0, b'B'], true), "A");
    assert_eq!(decode_debug_string(b"0xc0000142", false), "0xc0000142");
}
