//! 本轮私有桌面与新登录窗口站授权；不修改运行器既有对象，不启用任何特权。

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, size_of_val};
#[cfg(any(test, feature = "test-util"))]
use std::sync::Mutex;

use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
#[cfg(any(test, feature = "test-util"))]
use windows::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, SECURITY_MAX_SID_SIZE, WinBuiltinAdministratorsSid,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, FreeSid, GetSecurityDescriptorControl, GetSecurityDescriptorOwner,
    GetTokenInformation, GetUserObjectSecurity, IsValidSecurityDescriptor,
    LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED, SE_SACL_PRESENT,
    SE_SELF_RELATIVE, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR_RELATIVE, SetUserObjectSecurity,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, GetProcessWindowStation, GetThreadDesktop,
    GetUserObjectInformationW, HDESK, HWINSTA, SetThreadDesktop, UOI_FLAGS, UOI_NAME,
    USEROBJECTFLAGS,
};
#[cfg(any(test, feature = "test-util"))]
use windows::Win32::System::StationsAndDesktops::{
    CloseWindowStation, CreateWindowStationW, SetProcessWindowStation,
};
#[cfg(any(test, feature = "test-util"))]
use windows::Win32::System::Threading::STARTUPINFOW;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId, OpenProcessToken};
#[cfg(any(test, feature = "test-util"))]
use windows::Win32::UI::WindowsAndMessaging::CWF_CREATE_ONLY;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, WSF_VISIBLE};
use windows::core::{BOOL, Error as WindowsError, PCWSTR, PWSTR};

use super::{handle, owned, wide};

const STATION_OWNER_ACCESS: u32 = 0x000f037f;
const DESKTOP_OWNER_ACCESS: u32 = 0x000f01ff;
// 固定诊断掩码不声称是 Node 初始化的完整最低权限；不授予改 DACL、owner 或删除权。
const STATION_CONTAINER_ACCESS: u32 = 0x00020023;
const DESKTOP_CONTAINER_ACCESS: u32 = 0x00020083;
const SD_WORDS: usize = 16 * 1024;
#[cfg(any(test, feature = "test-util"))]
static CREATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug)]
struct DesktopApiError {
    stage: &'static str,
    hresult: i32,
}
impl std::fmt::Display for DesktopApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "私有桌面 {} 失败：HRESULT=0x{:08x}",
            self.stage, self.hresult as u32
        )
    }
}
impl std::error::Error for DesktopApiError {}
fn api_error(stage: &'static str, error: WindowsError) -> io::Error {
    // 系统正文可能包含对象名称；只保留固定阶段和完整 HRESULT。
    io::Error::other(DesktopApiError {
        stage,
        hresult: error.code().0,
    })
}
pub(in crate::windows) fn diagnostic_code(error: &io::Error) -> Option<(&'static str, i32)> {
    error
        .get_ref()?
        .downcast_ref::<DesktopApiError>()
        .map(|error| (error.stage, error.hresult))
}

#[derive(Debug)]
struct DesktopCheckError {
    stage: &'static str,
    message: &'static str,
    caller_station_matched: Option<bool>,
}
impl std::fmt::Display for DesktopCheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}
impl std::error::Error for DesktopCheckError {}
fn check_error(stage: &'static str, message: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        DesktopCheckError {
            stage,
            message,
            caller_station_matched: None,
        },
    )
}
pub(in crate::windows) fn diagnostic_check(error: &io::Error) -> Option<&'static str> {
    error
        .get_ref()?
        .downcast_ref::<DesktopCheckError>()
        .map(|error| error.stage)
}
pub(in crate::windows) fn diagnostic_caller_station_matched(error: &io::Error) -> Option<bool> {
    error
        .get_ref()?
        .downcast_ref::<DesktopCheckError>()
        .and_then(|error| error.caller_station_matched)
}

pub(in crate::windows) fn verify_expected_station_name(
    name: &str,
    expected_station: &str,
    caller_station: &str,
) -> io::Result<()> {
    if !name.eq_ignore_ascii_case(expected_station) {
        // 只在原首项失败后比较已读取的名称，不查询对象或继续后续安全校验。
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            DesktopCheckError {
                stage: "new_logon_station_name_mismatch",
                message: "新登录窗口站或 AppContainer SID 不匹配",
                caller_station_matched: Some(name.eq_ignore_ascii_case(caller_station)),
            },
        ));
    }
    Ok(())
}

// NULL token 查询本次调用线程的有效身份，不安装 token 或启用权限。
#[cfg(any(test, feature = "test-util"))]
fn calling_thread_administrator_membership() -> Result<bool, u32> {
    let mut sid = [0u32; SECURITY_MAX_SID_SIZE.div_ceil(4) as usize];
    let mut size = size_of_val(&sid) as u32;
    unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            None,
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut size,
        )
    }
    .map_err(|error| error.code().0 as u32)?;
    let mut member = BOOL::default();
    unsafe { CheckTokenMembership(None, PSID(sid.as_mut_ptr().cast()), &mut member) }
        .map_err(|error| error.code().0 as u32)?;
    Ok(member.as_bool())
}

// 只编码布尔值或数值错误码；API 查询失败不能降为非成员。
#[cfg(any(test, feature = "test-util"))]
fn administrator_membership_json(result: Result<bool, u32>) -> String {
    match result {
        Ok(member) => format!(r#"{{"status":"ok","member":{member},"hresult":null}}"#),
        Err(code) => format!(r#"{{"status":"query_error","member":null,"hresult":{code}}}"#),
    }
}

#[cfg(any(test, feature = "test-util"))]
fn invalid(stage: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, stage)
}

fn sid_text(sid: PSID) -> io::Result<String> {
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }
        .map_err(|error| api_error("sid_string", error))?;
    let result = unsafe { text.to_string() }
        .map_err(|_| check_error("sid_encoding", "私有桌面 SID 编码无效"));
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    result
}

fn current_user_sid() -> io::Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|error| api_error("open_token", error))?;
    let token = owned(token);
    let mut storage = [0usize; 128];
    let mut used = 0;
    unsafe {
        GetTokenInformation(
            handle(&token),
            TokenUser,
            Some(storage.as_mut_ptr().cast()),
            size_of_val(&storage) as u32,
            &mut used,
        )
    }
    .map_err(|error| api_error("token_user", error))?;
    if !(size_of::<TOKEN_USER>()..=size_of_val(&storage)).contains(&(used as usize)) {
        return Err(check_error("token_user_length", "私有桌面 token 长度无效"));
    }
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let bytes = unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), used as usize) };
    let offset = (user.User.Sid.0 as usize)
        .checked_sub(bytes.as_ptr() as usize)
        .filter(|offset| *offset >= size_of::<TOKEN_USER>())
        .ok_or_else(|| check_error("token_user_sid_bounds", "私有桌面 token SID 越界"))?;
    bounded_sid(bytes, offset)?;
    sid_text(user.User.Sid)
}

struct Descriptor {
    pointer: PSECURITY_DESCRIPTOR,
    size: usize,
}

impl Descriptor {
    fn new(owner: &str, container: &str, owner_access: u32, access: u32) -> io::Result<Self> {
        let sddl = wide(format!(
            "O:{owner}D:P(A;;0x{owner_access:08x};;;{owner})(A;;0x{access:08x};;;{container})S:(ML;;NW;;;LW)"
        ).as_ref())?;
        let mut pointer = PSECURITY_DESCRIPTOR::default();
        let mut size = 0;
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut pointer,
                Some(&mut size),
            )
        }
        .map_err(|error| api_error("build_sd", error))?;
        let result = Self {
            pointer,
            size: size as usize,
        };
        relative_parts(result.bytes())?;
        Ok(result)
    }

    fn bytes(&self) -> &[u8] {
        // 长度由成功的系统转换返回，指针由本对象独占持有。
        unsafe { std::slice::from_raw_parts(self.pointer.0.cast(), self.size) }
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.pointer.0,
            bInheritHandle: false.into(),
        }
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(Some(HLOCAL(self.pointer.0))) };
    }
}

/// 仅由已核实新 LUID 与原创建父进程的第二段 helper 建立。
/// 窗口站句柄来自当前进程，始终借用并持有到进程退出，不按名称重开替代。
pub(in crate::windows) struct NewLogonDesktop {
    station: HWINSTA,
    original_desktop: HDESK,
    desktop: Option<HDESK>,
    station_sd: Descriptor,
    desktop_sd: Descriptor,
}

impl NewLogonDesktop {
    pub(in crate::windows) fn create(
        expected_station: &str,
        profile: &str,
        container: &str,
        caller_station: &str,
    ) -> io::Result<Self> {
        let profile_name = wide(profile.as_ref())?;
        let derived =
            unsafe { DeriveAppContainerSidFromAppContainerName(PCWSTR(profile_name.as_ptr())) }
                .map_err(|error| api_error("derive_container_sid", error))?;
        let derived_text = sid_text(derived);
        unsafe { FreeSid(derived) };
        if derived_text? != container {
            return Err(check_error(
                "new_logon_profile_sid_mismatch",
                "新站授权 SID 与本轮 profile 不匹配",
            ));
        }
        // 在第二段显式触发 USER32 连接，再核对内核返回的新站；不推断 DLL/TLS 前置行为。
        unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let station = unsafe { GetProcessWindowStation() }
            .map_err(|error| api_error("new_logon_station", error))?;
        let name = object_name(HANDLE(station.0), "new_logon_station_name")?;
        let name = String::from_utf16(&name)
            .map_err(|_| check_error("new_logon_station_name_encoding", "新站名称编码无效"))?;
        verify_expected_station_name(&name, expected_station, caller_station)?;
        if name.eq_ignore_ascii_case(caller_station) {
            return Err(check_error(
                "new_logon_caller_station_reused",
                "新登录窗口站或 AppContainer SID 不匹配",
            ));
        }
        if !station_is_noninteractive(station)? {
            return Err(check_error(
                "new_logon_station_interactive",
                "新登录窗口站或 AppContainer SID 不匹配",
            ));
        }
        let owner = current_user_sid()?;
        let mut storage = vec![0u32; SD_WORDS];
        let mut used = 0;
        unsafe {
            GetUserObjectSecurity(
                HANDLE(station.0),
                &OWNER_SECURITY_INFORMATION.0,
                Some(PSECURITY_DESCRIPTOR(storage.as_mut_ptr().cast())),
                (SD_WORDS * 4) as u32,
                &mut used,
            )
        }
        .map_err(|error| api_error("new_logon_station_owner", error))?;
        if used as usize > SD_WORDS * 4 {
            return Err(check_error(
                "new_logon_owner_descriptor_bounds",
                "新站 owner 描述符长度越界",
            ));
        }
        let mut actual_owner = PSID::default();
        let mut defaulted = BOOL::default();
        unsafe {
            GetSecurityDescriptorOwner(
                PSECURITY_DESCRIPTOR(storage.as_mut_ptr().cast()),
                &mut actual_owner,
                &mut defaulted,
            )
        }
        .map_err(|error| api_error("new_logon_station_owner_sid", error))?;
        if actual_owner.0.is_null() {
            return Err(check_error(
                "new_logon_owner_missing",
                "新站不属于本次本地用户",
            ));
        }
        if sid_text(actual_owner)? != owner {
            return Err(check_error(
                "new_logon_owner_mismatch",
                "新站不属于本次本地用户",
            ));
        }
        if owner == container {
            return Err(check_error(
                "new_logon_owner_is_container",
                "新站不属于本次本地用户",
            ));
        }
        let station_sd = Descriptor::new(
            &owner,
            container,
            STATION_OWNER_ACCESS,
            STATION_CONTAINER_ACCESS,
        )?;
        let desktop_sd = Descriptor::new(
            &owner,
            container,
            DESKTOP_OWNER_ACCESS,
            DESKTOP_CONTAINER_ACCESS,
        )?;
        // 只有 helper 原句柄所持有的新站可以被授权；失败不尝试原站或全局 ACL。
        let requested = DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION
            | LABEL_SECURITY_INFORMATION;
        unsafe { SetUserObjectSecurity(HANDLE(station.0), &requested, station_sd.pointer) }
            .map_err(|error| api_error("new_logon_station_authorize", error))?;
        verify_object(HANDLE(station.0), &station_sd, true)?;
        let original_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
            .map_err(|error| api_error("new_logon_original_desktop", error))?;
        let mut result = Self {
            station,
            original_desktop,
            desktop: None,
            station_sd,
            desktop_sd,
        };
        let name = wide(profile.as_ref())?;
        let attributes = result.desktop_sd.attributes();
        result.desktop = Some(
            unsafe {
                CreateDesktopW(
                    PCWSTR(name.as_ptr()),
                    None,
                    None,
                    DESKTOP_CONTROL_FLAGS(0),
                    DESKTOP_OWNER_ACCESS,
                    Some(&attributes),
                )
            }
            .map_err(|error| api_error("new_logon_desktop_create", error))?,
        );
        if result.desktop == Some(original_desktop) {
            return Err(check_error(
                "new_logon_original_desktop_reused",
                "新建桌面不得复用 helper 初始桌面",
            ));
        }
        result.restore()?;
        result.verify()?;
        Ok(result)
    }

    fn restore(&self) -> io::Result<()> {
        if unsafe { GetProcessWindowStation() }
            .map_err(|error| api_error("new_logon_restore_station", error))?
            != self.station
        {
            return Err(check_error(
                "new_logon_restore_station_changed",
                "新站桌面恢复前窗口站已变化",
            ));
        }
        let current = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
            .map_err(|error| api_error("new_logon_current_desktop", error))?;
        if current != self.original_desktop {
            unsafe { SetThreadDesktop(self.original_desktop) }
                .map_err(|error| api_error("new_logon_restore_desktop", error))?;
        }
        if unsafe { GetThreadDesktop(GetCurrentThreadId()) }
            .map_err(|error| api_error("new_logon_restored_desktop", error))?
            != self.original_desktop
        {
            return Err(check_error(
                "new_logon_original_desktop_not_restored",
                "新站 helper 原桌面恢复未确认",
            ));
        }
        Ok(())
    }

    pub(in crate::windows) fn verify(&self) -> io::Result<()> {
        if unsafe { GetProcessWindowStation() }
            .map_err(|error| api_error("new_logon_station_recheck", error))?
            != self.station
        {
            return Err(check_error(
                "new_logon_station_changed",
                "新站持有者切换了窗口站",
            ));
        }
        verify_object(HANDLE(self.station.0), &self.station_sd, true)?;
        let desktop = self
            .desktop
            .ok_or_else(|| check_error("new_logon_desktop_closed", "新登录桌面已关闭"))?;
        verify_object(HANDLE(desktop.0), &self.desktop_sd, false)
    }

    #[cfg(feature = "native-probe-witness")]
    pub(in crate::windows) fn witness_handle(&self) -> io::Result<HDESK> {
        self.verify()?;
        self.desktop
            .ok_or_else(|| check_error("witness_desktop_closed", "取证桌面已关闭"))
    }

    pub(in crate::windows) fn close(&mut self) -> io::Result<()> {
        // 只恢复同一新站的初始桌面；不切换窗口站，也不关闭借用的初始句柄。
        self.restore()?;
        if let Some(desktop) = self.desktop {
            unsafe { CloseDesktop(desktop) }
                .map_err(|error| api_error("new_logon_desktop_close", error))?;
            self.desktop = None;
        }
        // 系统自动连接的 station 不能 CloseWindowStation；所有者进程退出后再由父核消失。
        Ok(())
    }
}

impl Drop for NewLogonDesktop {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn bounded_sid(bytes: &[u8], start: usize) -> io::Result<&[u8]> {
    let header = bytes
        .get(start..)
        .and_then(|tail| tail.get(..8))
        .ok_or_else(|| check_error("sid_header_bounds", "私有桌面 SID 头越界"))?;
    if header[0] != 1 || header[1] > 15 {
        return Err(check_error("sid_structure", "私有桌面 SID 结构无效"));
    }
    bytes
        .get(start..start + 8 + usize::from(header[1]) * 4)
        .ok_or_else(|| check_error("sid_bounds", "私有桌面 SID 越界"))
}

fn bounded_acl(bytes: &[u8], start: usize, kind: u8, count: u16) -> io::Result<&[u8]> {
    let tail = bytes
        .get(start..)
        .ok_or_else(|| check_error("acl_bounds", "私有桌面 ACL 越界"))?;
    let header = tail
        .get(..8)
        .ok_or_else(|| check_error("acl_header_bounds", "私有桌面 ACL 头越界"))?;
    let length = u16::from_le_bytes([header[2], header[3]]) as usize;
    if header[0] != 2 || u16::from_le_bytes([header[4], header[5]]) != count || length < 8 {
        return Err(check_error("acl_structure", "私有桌面 ACL 结构无效"));
    }
    let acl = tail
        .get(..length)
        .ok_or_else(|| check_error("acl_length_bounds", "私有桌面 ACL 长度越界"))?;
    let mut offset = 8;
    for _ in 0..count {
        let ace = acl
            .get(offset..)
            .and_then(|tail| tail.get(..4))
            .ok_or_else(|| check_error("ace_header_bounds", "私有桌面 ACE 头越界"))?;
        let size = u16::from_le_bytes([ace[2], ace[3]]) as usize;
        if ace[0] != kind || ace[1] != 0 || size < 16 {
            return Err(check_error("ace_type", "私有桌面 ACE 类型无效"));
        }
        let ace = acl
            .get(offset..offset + size)
            .ok_or_else(|| check_error("ace_length_bounds", "私有桌面 ACE 长度越界"))?;
        let sid = bounded_sid(ace, 8)?;
        if 8 + sid.len() != size {
            return Err(check_error(
                "ace_sid_length_mismatch",
                "私有桌面 ACE SID 长度不匹配",
            ));
        }
        offset += size;
    }
    if offset != length {
        return Err(check_error(
            "acl_trailing_content",
            "私有桌面 ACL 存在额外内容",
        ));
    }
    Ok(acl)
}

fn relative_parts(bytes: &[u8]) -> io::Result<(&[u8], &[u8], &[u8])> {
    if !(size_of::<SECURITY_DESCRIPTOR_RELATIVE>()..=SD_WORDS * 4).contains(&bytes.len()) {
        return Err(check_error(
            "descriptor_length",
            "私有桌面安全描述符长度无效",
        ));
    }
    let header =
        unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<SECURITY_DESCRIPTOR_RELATIVE>()) };
    if header.Revision != 1
        || header.Control
            & (SE_SELF_RELATIVE | SE_DACL_PROTECTED | SE_DACL_PRESENT | SE_SACL_PRESENT)
            != (SE_SELF_RELATIVE | SE_DACL_PROTECTED | SE_DACL_PRESENT | SE_SACL_PRESENT)
    {
        return Err(check_error(
            "descriptor_control",
            "私有桌面安全描述符控制位无效",
        ));
    }
    let offset = |value: u32| {
        let value = value as usize;
        if value < size_of::<SECURITY_DESCRIPTOR_RELATIVE>() || value >= bytes.len() {
            Err(check_error(
                "descriptor_offset_bounds",
                "私有桌面安全描述符偏移越界",
            ))
        } else {
            Ok(value)
        }
    };
    if header.Group != 0 {
        bounded_sid(bytes, offset(header.Group)?)?;
    }
    let owner = bounded_sid(bytes, offset(header.Owner)?)?;
    let dacl = bounded_acl(bytes, offset(header.Dacl)?, 0, 2)?;
    let label = bounded_acl(bytes, offset(header.Sacl)?, 17, 1)?;
    Ok((owner, dacl, label))
}

fn verify_descriptor(actual: &[u8], expected: &[u8]) -> io::Result<()> {
    let actual_parts = relative_parts(actual)?;
    let expected_parts = relative_parts(expected)?;
    if actual_parts.0 != expected_parts.0 {
        return Err(check_error(
            "descriptor_owner_mismatch",
            "私有桌面 owner、DACL 或低完整性标签读回不匹配",
        ));
    }
    if actual_parts.1 != expected_parts.1 {
        return Err(check_error(
            "descriptor_dacl_mismatch",
            "私有桌面 owner、DACL 或低完整性标签读回不匹配",
        ));
    }
    if actual_parts.2 != expected_parts.2 {
        return Err(check_error(
            "descriptor_label_mismatch",
            "私有桌面 owner、DACL 或低完整性标签读回不匹配",
        ));
    }
    Ok(())
}

fn verify_object(object: HANDLE, expected: &Descriptor, station: bool) -> io::Result<()> {
    let mut storage = vec![0u32; SD_WORDS];
    let mut used = 0;
    let requested =
        (OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION).0;
    unsafe {
        GetUserObjectSecurity(
            object,
            &requested,
            Some(PSECURITY_DESCRIPTOR(storage.as_mut_ptr().cast())),
            (SD_WORDS * 4) as u32,
            &mut used,
        )
    }
    .map_err(|error| api_error("read_sd", error))?;
    if used as usize > SD_WORDS * 4 {
        return Err(check_error(
            "descriptor_readback_bounds",
            "私有桌面安全描述符读回越界",
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(storage.as_ptr().cast(), used as usize) };
    verify_descriptor(bytes, expected.bytes())?;
    let descriptor = PSECURITY_DESCRIPTOR(storage.as_mut_ptr().cast());
    if !unsafe { IsValidSecurityDescriptor(descriptor) }.as_bool() {
        return Err(check_error("descriptor_invalid", "私有桌面安全描述符无效"));
    }
    let mut control = 0;
    let mut revision = 0;
    unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
        .map_err(|error| api_error("read_sd_control", error))?;
    let mut flags = USEROBJECTFLAGS::default();
    let mut needed = 0;
    unsafe {
        GetUserObjectInformationW(
            object,
            UOI_FLAGS,
            Some((&mut flags as *mut USEROBJECTFLAGS).cast::<c_void>()),
            size_of_val(&flags) as u32,
            Some(&mut needed),
        )
    }
    .map_err(|error| api_error("read_object_flags", error))?;
    if needed as usize != size_of_val(&flags) {
        return Err(check_error(
            "object_flags_length",
            "私有桌面继承或可见状态不匹配",
        ));
    }
    if flags.fInherit.as_bool() {
        return Err(check_error(
            "object_inheritable",
            "私有桌面继承或可见状态不匹配",
        ));
    }
    if station && flags.dwFlags & WSF_VISIBLE as u32 != 0 {
        return Err(check_error(
            "object_station_visible",
            "私有桌面继承或可见状态不匹配",
        ));
    }
    Ok(())
}

fn object_name(object: HANDLE, stage: &'static str) -> io::Result<Vec<u16>> {
    let mut name = [0u16; 256];
    let mut used = 0;
    unsafe {
        GetUserObjectInformationW(
            object,
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            size_of_val(&name) as u32,
            Some(&mut used),
        )
    }
    .map_err(|error| api_error(stage, error))?;
    if used as usize % size_of::<u16>() != 0 {
        return Err(check_error(
            "object_name_length",
            "私有桌面对象名称长度无效",
        ));
    }
    let units = used as usize / size_of::<u16>();
    if !(2..=name.len()).contains(&units)
        || name[units - 1] != 0
        || name[..units - 1]
            .iter()
            .any(|unit| *unit == 0 || *unit == b'\\' as u16)
    {
        return Err(check_error("object_name_invalid", "私有桌面对象名称无效"));
    }
    Ok(name[..units - 1].to_vec())
}

fn station_is_noninteractive(station: HWINSTA) -> io::Result<bool> {
    let mut flags = USEROBJECTFLAGS::default();
    let mut used = 0;
    unsafe {
        GetUserObjectInformationW(
            HANDLE(station.0),
            UOI_FLAGS,
            Some((&mut flags as *mut USEROBJECTFLAGS).cast::<c_void>()),
            size_of_val(&flags) as u32,
            Some(&mut used),
        )
    }
    .map_err(|error| api_error("read_existing_station_flags", error))?;
    if used as usize != size_of_val(&flags) {
        return Err(check_error(
            "station_flags_length",
            "现有窗口站标志长度无效",
        ));
    }
    Ok(flags.dwFlags & WSF_VISIBLE as u32 == 0)
}

#[cfg(any(test, feature = "test-util"))]
pub(super) enum ProbeDesktop {
    NewStation(PrivateDesktop),
    ExistingStation(ExistingStationDesktop),
}

#[cfg(any(test, feature = "test-util"))]
impl ProbeDesktop {
    pub(super) fn verify(&self) -> io::Result<()> {
        match self {
            Self::NewStation(desktop) => desktop.verify(),
            Self::ExistingStation(desktop) => desktop.verify(),
        }
    }

    pub(super) fn configure_startup(&mut self, startup: &mut STARTUPINFOW) -> io::Result<()> {
        match self {
            Self::NewStation(desktop) => desktop.configure_startup(startup),
            Self::ExistingStation(desktop) => desktop.configure_startup(startup),
        }
    }

    pub(super) fn close(&mut self) -> io::Result<()> {
        match self {
            Self::NewStation(desktop) => desktop.close(),
            Self::ExistingStation(desktop) => desktop.close(),
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
pub(super) struct ExistingStationDesktop {
    station: HWINSTA,
    station_name: Vec<u16>,
    original_desktop: HDESK,
    desktop: Option<HDESK>,
    desktop_name: Vec<u16>,
    startup_name: Vec<u16>,
    descriptor: Descriptor,
    restored: bool,
}

#[cfg(any(test, feature = "test-util"))]
impl ExistingStationDesktop {
    pub(super) fn create(profile_name: &str, sid: PSID) -> io::Result<Self> {
        let suffix = profile_name
            .strip_prefix("InfiniShell.Version.")
            .filter(|value| {
                value.len() == 36
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            })
            .ok_or_else(|| invalid("私有桌面代次名称无效"))?;
        let _creation = CREATION_LOCK
            .lock()
            .map_err(|_| invalid("私有桌面创建锁已损坏"))?;
        let station = unsafe { GetProcessWindowStation() }
            .map_err(|error| api_error("existing_station", error))?;
        let station_name = object_name(HANDLE(station.0), "read_existing_station_name")?;
        if !station_is_noninteractive(station)?
            || station_name.len() == 7
                && station_name
                    .iter()
                    .zip("WinSta0".encode_utf16())
                    .all(|(actual, expected)| {
                        u8::try_from(*actual)
                            .is_ok_and(|actual| actual.eq_ignore_ascii_case(&(expected as u8)))
                    })
        {
            return Err(invalid("私有桌面拒绝交互窗口站"));
        }
        let original_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
            .map_err(|error| api_error("original_desktop", error))?;
        let desktop_name = wide(format!("InfiniShell.Probe.{suffix}").as_ref())?;
        let mut startup_name = station_name.clone();
        startup_name.push(b'\\' as u16);
        startup_name.extend_from_slice(&desktop_name);
        let owner = current_user_sid()?;
        let container = sid_text(sid)?;
        if owner == container {
            return Err(invalid("私有桌面用户与容器 SID 相同"));
        }
        let descriptor = Descriptor::new(
            &owner,
            &container,
            DESKTOP_OWNER_ACCESS,
            DESKTOP_CONTAINER_ACCESS,
        )?;
        let mut result = Self {
            station,
            station_name,
            original_desktop,
            desktop: None,
            desktop_name,
            startup_name,
            descriptor,
            restored: false,
        };
        let created = (|| -> io::Result<()> {
            let attributes = result.descriptor.attributes();
            result.desktop = Some(
                unsafe {
                    CreateDesktopW(
                        PCWSTR(result.desktop_name.as_ptr()),
                        None,
                        None,
                        DESKTOP_CONTROL_FLAGS(0),
                        DESKTOP_OWNER_ACCESS,
                        Some(&attributes),
                    )
                }
                .map_err(|error| api_error("create_existing_station_desktop", error))?,
            );
            result.verify()?;
            result.restore()
        })();
        if let Err(error) = created {
            let cleanup = result.close();
            eprintln!(
                "atomic_windows_existing_station_private_object_cleanup={{\"confirmed\":{}}}",
                cleanup.is_ok()
            );
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => io::Error::other(format!("{error}；私有对象清理失败：{cleanup}")),
            });
        }
        Ok(result)
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.restored {
            let current = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
                .map_err(|error| api_error("read_current_desktop", error))?;
            if current != self.original_desktop {
                unsafe { SetThreadDesktop(self.original_desktop) }
                    .map_err(|error| api_error("restore_desktop", error))?;
            }
            let restored = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
                .map_err(|error| api_error("verify_restored_desktop", error))?;
            if restored != self.original_desktop {
                return Err(invalid("私有桌面原始桌面恢复未确认"));
            }
            self.restored = true;
        }
        Ok(())
    }

    pub(super) fn verify(&self) -> io::Result<()> {
        let current = unsafe { GetProcessWindowStation() }
            .map_err(|error| api_error("verify_existing_station", error))?;
        if current != self.station
            || object_name(HANDLE(current.0), "read_existing_station_name")? != self.station_name
            || !station_is_noninteractive(current)?
        {
            return Err(invalid("私有桌面现有窗口站身份不匹配"));
        }
        let desktop = self.desktop.ok_or_else(|| invalid("私有桌面已关闭"))?;
        if object_name(HANDLE(desktop.0), "read_existing_desktop_name")?.as_slice()
            != &self.desktop_name[..self.desktop_name.len() - 1]
        {
            return Err(invalid("私有桌面对象名称不匹配"));
        }
        verify_object(HANDLE(desktop.0), &self.descriptor, false)
    }

    pub(super) fn configure_startup(&mut self, startup: &mut STARTUPINFOW) -> io::Result<()> {
        if !self.restored {
            return Err(invalid("私有桌面原始桌面尚未恢复"));
        }
        self.verify()?;
        startup.lpDesktop = PWSTR(self.startup_name.as_mut_ptr());
        Ok(())
    }

    pub(super) fn close(&mut self) -> io::Result<()> {
        self.restore()?;
        if let Some(desktop) = self.desktop {
            unsafe { CloseDesktop(desktop) }.map_err(|error| api_error("close_desktop", error))?;
            self.desktop = None;
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Drop for ExistingStationDesktop {
    fn drop(&mut self) {
        // 仅关闭本轮创建的桌面；现有窗口站与原桌面句柄均由系统持有。
        let _ = self.close();
    }
}

#[cfg(any(test, feature = "test-util"))]
struct OriginalObjects {
    station: HWINSTA,
    desktop: HDESK,
}

#[cfg(any(test, feature = "test-util"))]
pub(super) struct PrivateDesktop {
    station: Option<HWINSTA>,
    desktop: Option<HDESK>,
    original: Option<OriginalObjects>,
    startup_name: Vec<u16>,
    station_sd: Descriptor,
    desktop_sd: Descriptor,
}

#[cfg(any(test, feature = "test-util"))]
impl PrivateDesktop {
    pub(super) fn create(profile_name: &str, sid: PSID) -> io::Result<Self> {
        let suffix = profile_name
            .strip_prefix("InfiniShell.Version.")
            .filter(|value| {
                value.len() == 36
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            })
            .ok_or_else(|| invalid("私有桌面代次名称无效"))?;
        let station_name = wide(format!("InfiniShell.Probe.{suffix}").as_ref())?;
        let desktop_name = wide("Probe".as_ref())?;
        let startup_name = wide(format!("InfiniShell.Probe.{suffix}\\Probe").as_ref())?;
        let owner = current_user_sid()?;
        let container = sid_text(sid)?;
        if owner == container {
            return Err(invalid("私有桌面用户与容器 SID 相同"));
        }
        let station_sd = Descriptor::new(
            &owner,
            &container,
            STATION_OWNER_ACCESS,
            STATION_CONTAINER_ACCESS,
        )?;
        let desktop_sd = Descriptor::new(
            &owner,
            &container,
            DESKTOP_OWNER_ACCESS,
            DESKTOP_CONTAINER_ACCESS,
        )?;
        // SetProcessWindowStation 是进程级操作；此入口只允许独立单测试 driver 使用。
        let _creation = CREATION_LOCK
            .lock()
            .map_err(|_| invalid("私有桌面创建锁已损坏"))?;
        let original = OriginalObjects {
            station: unsafe { GetProcessWindowStation() }
                .map_err(|error| api_error("original_station", error))?,
            desktop: unsafe { GetThreadDesktop(GetCurrentThreadId()) }
                .map_err(|error| api_error("original_desktop", error))?,
        };
        let mut result = Self {
            station: None,
            desktop: None,
            original: Some(original),
            startup_name,
            station_sd,
            desktop_sd,
        };
        let created = (|| -> io::Result<()> {
            let attributes = result.station_sd.attributes();
            let membership =
                administrator_membership_json(calling_thread_administrator_membership());
            result.station = Some(
                unsafe {
                    CreateWindowStationW(
                        PCWSTR(station_name.as_ptr()),
                        CWF_CREATE_ONLY,
                        STATION_OWNER_ACCESS,
                        Some(&attributes),
                    )
                }
                .map_err(|error| {
                    eprintln!(
                        "atomic_windows_private_desktop_admin={{\"scope\":\"create_station_calling_thread\",\"administrator_membership\":{membership}}}"
                    );
                    let failure = api_error("create_station", error);
                    io::Error::other(format!("{failure}；administrator_membership={membership}"))
                })?,
            );
            eprintln!(
                "atomic_windows_private_desktop_admin={{\"scope\":\"create_station_calling_thread\",\"administrator_membership\":{membership}}}"
            );
            unsafe { SetProcessWindowStation(result.station.unwrap()) }
                .map_err(|error| api_error("select_station", error))?;
            let attributes = result.desktop_sd.attributes();
            result.desktop = Some(
                unsafe {
                    CreateDesktopW(
                        PCWSTR(desktop_name.as_ptr()),
                        None,
                        None,
                        DESKTOP_CONTROL_FLAGS(0),
                        DESKTOP_OWNER_ACCESS,
                        Some(&attributes),
                    )
                }
                .map_err(|error| api_error("create_desktop", error))?,
            );
            result.verify()?;
            result.restore()
        })();
        if let Err(error) = created {
            return Err(match result.close() {
                Ok(()) => error,
                Err(cleanup) => io::Error::other(format!("{error}；私有对象清理失败：{cleanup}")),
            });
        }
        Ok(result)
    }

    fn restore(&mut self) -> io::Result<()> {
        if let Some(original) = &self.original {
            unsafe { SetProcessWindowStation(original.station) }
                .map_err(|error| api_error("restore_station", error))?;
            unsafe { SetThreadDesktop(original.desktop) }
                .map_err(|error| api_error("restore_desktop", error))?;
            let station = unsafe { GetProcessWindowStation() }
                .map_err(|error| api_error("verify_restored_station", error))?;
            let desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
                .map_err(|error| api_error("verify_restored_desktop", error))?;
            if station != original.station || desktop != original.desktop {
                return Err(invalid("私有桌面原始对象恢复未确认"));
            }
            self.original = None;
        }
        Ok(())
    }

    pub(super) fn verify(&self) -> io::Result<()> {
        let station = self.station.ok_or_else(|| invalid("私有窗口站已关闭"))?;
        let desktop = self.desktop.ok_or_else(|| invalid("私有桌面已关闭"))?;
        verify_object(HANDLE(station.0), &self.station_sd, true)?;
        verify_object(HANDLE(desktop.0), &self.desktop_sd, false)
    }

    pub(super) fn configure_startup(&mut self, startup: &mut STARTUPINFOW) -> io::Result<()> {
        if self.original.is_some() {
            return Err(invalid("私有桌面原始对象尚未恢复"));
        }
        self.verify()?;
        startup.lpDesktop = PWSTR(self.startup_name.as_mut_ptr());
        Ok(())
    }

    pub(super) fn close(&mut self) -> io::Result<()> {
        self.restore()?;
        if let Some(desktop) = self.desktop {
            unsafe { CloseDesktop(desktop) }.map_err(|error| api_error("close_desktop", error))?;
            self.desktop = None;
        }
        if let Some(station) = self.station {
            unsafe { CloseWindowStation(station) }
                .map_err(|error| api_error("close_station", error))?;
            self.station = None;
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        // 显式清理失败已向调用方返回；析构只尽力释放自己创建的对象，不关闭借用句柄。
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "windows_appcontainer_desktop_tests.rs"]
pub(super) mod test_cases;
