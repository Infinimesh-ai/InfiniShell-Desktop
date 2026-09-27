//! 固定版本诊断专用私有窗口站与桌面；不修改运行器既有对象，不启用任何特权。

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, size_of_val};
use std::sync::Mutex;

use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, DACL_SECURITY_INFORMATION,
    GetSecurityDescriptorControl, GetTokenInformation, GetUserObjectSecurity,
    IsValidSecurityDescriptor, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED, SE_SACL_PRESENT,
    SE_SELF_RELATIVE, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR_RELATIVE, SECURITY_MAX_SID_SIZE,
    TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW, DESKTOP_CONTROL_FLAGS,
    GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW, HDESK, HWINSTA,
    SetProcessWindowStation, SetThreadDesktop, UOI_FLAGS, USEROBJECTFLAGS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcessToken, STARTUPINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{CWF_CREATE_ONLY, WSF_VISIBLE};
use windows::core::{BOOL, Error as WindowsError, PCWSTR, PWSTR};

use super::{handle, owned, wide};

const STATION_OWNER_ACCESS: u32 = 0x000f037f;
const DESKTOP_OWNER_ACCESS: u32 = 0x000f01ff;
// 固定诊断掩码不声称是 Node 初始化的完整最低权限；不授予改 DACL、owner 或删除权。
const STATION_CONTAINER_ACCESS: u32 = 0x00020023;
const DESKTOP_CONTAINER_ACCESS: u32 = 0x00020083;
const SD_WORDS: usize = 16 * 1024;
static CREATION_LOCK: Mutex<()> = Mutex::new(());

fn api_error(stage: &'static str, error: WindowsError) -> io::Error {
    // 系统正文可能包含对象名称；只保留固定阶段和完整 HRESULT。
    io::Error::other(format!(
        "私有桌面 {stage} 失败：HRESULT=0x{:08x}",
        error.code().0 as u32
    ))
}

// NULL token 查询本次调用线程的有效身份，不安装 token 或启用权限。
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
fn administrator_membership_json(result: Result<bool, u32>) -> String {
    match result {
        Ok(member) => format!(r#"{{"status":"ok","member":{member},"hresult":null}}"#),
        Err(code) => format!(r#"{{"status":"query_error","member":null,"hresult":{code}}}"#),
    }
}

fn invalid(stage: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, stage)
}

fn sid_text(sid: PSID) -> io::Result<String> {
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }
        .map_err(|error| api_error("sid_string", error))?;
    let result = unsafe { text.to_string() }.map_err(|_| invalid("私有桌面 SID 编码无效"));
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
        return Err(invalid("私有桌面 token 长度无效"));
    }
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let bytes = unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), used as usize) };
    let offset = (user.User.Sid.0 as usize)
        .checked_sub(bytes.as_ptr() as usize)
        .filter(|offset| *offset >= size_of::<TOKEN_USER>())
        .ok_or_else(|| invalid("私有桌面 token SID 越界"))?;
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

fn bounded_sid(bytes: &[u8], start: usize) -> io::Result<&[u8]> {
    let header = bytes
        .get(start..)
        .and_then(|tail| tail.get(..8))
        .ok_or_else(|| invalid("私有桌面 SID 头越界"))?;
    if header[0] != 1 || header[1] > 15 {
        return Err(invalid("私有桌面 SID 结构无效"));
    }
    bytes
        .get(start..start + 8 + usize::from(header[1]) * 4)
        .ok_or_else(|| invalid("私有桌面 SID 越界"))
}

fn bounded_acl(bytes: &[u8], start: usize, kind: u8, count: u16) -> io::Result<&[u8]> {
    let tail = bytes
        .get(start..)
        .ok_or_else(|| invalid("私有桌面 ACL 越界"))?;
    let header = tail
        .get(..8)
        .ok_or_else(|| invalid("私有桌面 ACL 头越界"))?;
    let length = u16::from_le_bytes([header[2], header[3]]) as usize;
    if header[0] != 2 || u16::from_le_bytes([header[4], header[5]]) != count || length < 8 {
        return Err(invalid("私有桌面 ACL 结构无效"));
    }
    let acl = tail
        .get(..length)
        .ok_or_else(|| invalid("私有桌面 ACL 长度越界"))?;
    let mut offset = 8;
    for _ in 0..count {
        let ace = acl
            .get(offset..)
            .and_then(|tail| tail.get(..4))
            .ok_or_else(|| invalid("私有桌面 ACE 头越界"))?;
        let size = u16::from_le_bytes([ace[2], ace[3]]) as usize;
        if ace[0] != kind || ace[1] != 0 || size < 16 {
            return Err(invalid("私有桌面 ACE 类型无效"));
        }
        let ace = acl
            .get(offset..offset + size)
            .ok_or_else(|| invalid("私有桌面 ACE 长度越界"))?;
        let sid = bounded_sid(ace, 8)?;
        if 8 + sid.len() != size {
            return Err(invalid("私有桌面 ACE SID 长度不匹配"));
        }
        offset += size;
    }
    if offset != length {
        return Err(invalid("私有桌面 ACL 存在额外内容"));
    }
    Ok(acl)
}

fn relative_parts(bytes: &[u8]) -> io::Result<(&[u8], &[u8], &[u8])> {
    if !(size_of::<SECURITY_DESCRIPTOR_RELATIVE>()..=SD_WORDS * 4).contains(&bytes.len()) {
        return Err(invalid("私有桌面安全描述符长度无效"));
    }
    let header =
        unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<SECURITY_DESCRIPTOR_RELATIVE>()) };
    if header.Revision != 1
        || header.Control
            & (SE_SELF_RELATIVE | SE_DACL_PROTECTED | SE_DACL_PRESENT | SE_SACL_PRESENT)
            != (SE_SELF_RELATIVE | SE_DACL_PROTECTED | SE_DACL_PRESENT | SE_SACL_PRESENT)
    {
        return Err(invalid("私有桌面安全描述符控制位无效"));
    }
    let offset = |value: u32| {
        let value = value as usize;
        if value < size_of::<SECURITY_DESCRIPTOR_RELATIVE>() || value >= bytes.len() {
            Err(invalid("私有桌面安全描述符偏移越界"))
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
    if actual_parts != expected_parts {
        return Err(invalid("私有桌面 owner、DACL 或低完整性标签读回不匹配"));
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
        return Err(invalid("私有桌面安全描述符读回越界"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(storage.as_ptr().cast(), used as usize) };
    verify_descriptor(bytes, expected.bytes())?;
    let descriptor = PSECURITY_DESCRIPTOR(storage.as_mut_ptr().cast());
    if !unsafe { IsValidSecurityDescriptor(descriptor) }.as_bool() {
        return Err(invalid("私有桌面安全描述符无效"));
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
    if needed as usize != size_of_val(&flags)
        || flags.fInherit.as_bool()
        || station && flags.dwFlags & WSF_VISIBLE as u32 != 0
    {
        return Err(invalid("私有桌面继承或可见状态不匹配"));
    }
    Ok(())
}

struct OriginalObjects {
    station: HWINSTA,
    desktop: HDESK,
}

pub(super) struct PrivateDesktop {
    station: Option<HWINSTA>,
    desktop: Option<HDESK>,
    original: Option<OriginalObjects>,
    startup_name: Vec<u16>,
    station_sd: Descriptor,
    desktop_sd: Descriptor,
}

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

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        // 显式清理失败已向调用方返回；析构只尽力释放自己创建的对象，不关闭借用句柄。
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "windows_appcontainer_desktop_tests.rs"]
pub(super) mod test_cases;
