//! 专属 launchd job 的资源域；成员枚举只用于发现，最终必须确认已知 CID 被内核回收。

use std::ffi::{CStr, c_void};
use std::io;
use std::mem;
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const PID_UNIQUE: libc::c_int = 17;
const PID_COALITION: libc::c_int = 20;
const MAX_DISCOVERY_PIDS: usize = 1_048_576;

type PidInfo =
    unsafe extern "C" fn(libc::c_int, libc::c_int, u64, *mut c_void, libc::c_int) -> libc::c_int;
type ListPids = unsafe extern "C" fn(*mut c_void, libc::c_int) -> libc::c_int;
type SignalToken = unsafe extern "C" fn(*mut [u32; 8], libc::c_int) -> libc::c_int;
type ResourceUsage = unsafe extern "C" fn(u64, *mut c_void, usize) -> libc::c_int;

#[derive(Clone, Copy)]
struct Api {
    pid_info: PidInfo,
    list_pids: ListPids,
    signal: SignalToken,
    usage: ResourceUsage,
}

impl Api {
    fn load() -> io::Result<Self> {
        static API: OnceLock<Result<Api, String>> = OnceLock::new();
        API.get_or_init(|| Self::load_once().map_err(|error| error.to_string()))
            .as_ref()
            .copied()
            .map_err(|message| io::Error::new(io::ErrorKind::Unsupported, message.clone()))
    }

    fn load_once() -> io::Result<Self> {
        struct Library(*mut c_void);
        impl Drop for Library {
            fn drop(&mut self) {
                unsafe { libc::dlclose(self.0) };
            }
        }
        fn library(path: &CStr) -> io::Result<Library> {
            let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
            if handle.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "当前系统无法加载托管资源域接口",
                ));
            }
            Ok(Library(handle))
        }
        fn symbol(library: &Library, name: &CStr) -> io::Result<*mut c_void> {
            let address = unsafe { libc::dlsym(library.0, name.as_ptr()) };
            if address.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "当前系统缺少托管资源域接口",
                ));
            }
            Ok(address)
        }
        let processes = library(c"/usr/lib/libproc.dylib")?;
        let system = library(c"/usr/lib/libSystem.B.dylib")?;
        // 原型核对过固定 XNU 的准确签名；显式加载避免测试宿主未链接 libproc 时误判。
        let api = unsafe {
            Self {
                pid_info: mem::transmute::<*mut c_void, PidInfo>(symbol(
                    &processes,
                    c"proc_pidinfo",
                )?),
                list_pids: mem::transmute::<*mut c_void, ListPids>(symbol(
                    &processes,
                    c"proc_listallpids",
                )?),
                signal: mem::transmute::<*mut c_void, SignalToken>(symbol(
                    &processes,
                    c"proc_signal_with_audittoken",
                )?),
                usage: mem::transmute::<*mut c_void, ResourceUsage>(symbol(
                    &system,
                    c"coalition_info_resource_usage",
                )?),
            }
        };
        // OnceLock 只保留这一组动态库引用，保证缓存函数指针在进程结束前一直有效。
        mem::forget(processes);
        mem::forget(system);
        Ok(api)
    }

    fn read<T>(&self, pid: i32, flavor: i32) -> io::Result<T> {
        let mut value = mem::MaybeUninit::<T>::zeroed();
        let size = unsafe {
            (self.pid_info)(
                pid,
                flavor,
                0,
                value.as_mut_ptr().cast(),
                mem::size_of::<T>() as i32,
            )
        };
        if size != mem::size_of::<T>() as i32 {
            return Err(io::Error::other("托管进程身份读取不完整"));
        }
        Ok(unsafe { value.assume_init() })
    }

    fn identity(&self, pid: i32) -> io::Result<MacosProcessIdentity> {
        let first: Unique = self.read(pid, PID_UNIQUE)?;
        let coalition: CoalitionInfo = self.read(pid, PID_COALITION)?;
        let second: Unique = self.read(pid, PID_UNIQUE)?;
        if first.unique_id != second.unique_id
            || first.pid_version != second.pid_version
            || pid <= 0
        {
            return Err(io::Error::other("托管进程身份在查询期间改变"));
        }
        Ok(MacosProcessIdentity {
            pid,
            pid_version: first.pid_version as u32,
            unique_id: first.unique_id,
            resource_cid: coalition.ids[0],
        })
    }

    fn usage(&self, cid: u64) -> io::Result<Option<[u64; 2]>> {
        if cid == 0 {
            return Err(io::Error::other("托管资源域 ID 无效"));
        }
        let mut values = [0u64; 2];
        let result =
            unsafe { (self.usage)(cid, values.as_mut_ptr().cast(), mem::size_of_val(&values)) };
        if result == 0 {
            return Ok(Some(values));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        }
    }

    fn all_pids(&self) -> io::Result<Vec<i32>> {
        let estimate = unsafe { (self.list_pids)(std::ptr::null_mut(), 0) };
        if estimate <= 0 {
            return Err(io::Error::other("无法发现托管资源域成员"));
        }
        let mut capacity = estimate as usize + 64;
        loop {
            if capacity > MAX_DISCOVERY_PIDS {
                return Err(io::Error::other("托管进程发现超过安全容量"));
            }
            let mut pids = vec![0i32; capacity];
            let count = unsafe {
                (self.list_pids)(
                    pids.as_mut_ptr().cast(),
                    (capacity * mem::size_of::<i32>()) as i32,
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if (count as usize) < capacity {
                pids.truncate(count as usize);
                return Ok(pids);
            }
            capacity = capacity.saturating_mul(2);
        }
    }
}

#[repr(C)]
struct Unique {
    uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    pid_version: i32,
    original_parent_pid_version: i32,
    reserved: [u64; 2],
}

#[repr(C)]
struct CoalitionInfo {
    ids: [u64; 2],
    reserved: [u64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacosProcessIdentity {
    pub pid: i32,
    pub pid_version: u32,
    pub unique_id: u64,
    pub resource_cid: u64,
}

pub fn macos_boot_session() -> io::Result<String> {
    let mut buffer = [0u8; 128];
    let mut size = buffer.len();
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size < 2 || size > buffer.len() || buffer[size - 1] != 0 {
        return Err(io::Error::other("无法核对系统启动身份"));
    }
    let value = std::str::from_utf8(&buffer[..size - 1]).map_err(io::Error::other)?;
    if value.len() != 36
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err(io::Error::other("系统启动身份格式无效"));
    }
    Ok(value.to_owned())
}

pub fn macos_peer_identity(stream: &UnixStream) -> io::Result<MacosProcessIdentity> {
    let mut token = [0u32; 8];
    let mut size = mem::size_of_val(&token) as libc::socklen_t;
    // LOCAL_PEERTOKEN 在连接建立时由内核固定，不能相信握手中自报的 PID。
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            0,
            6,
            token.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result != 0
        || size as usize != mem::size_of_val(&token)
        || token[1] != unsafe { libc::geteuid() }
    {
        return Err(io::Error::other("托管 IPC 对端身份不可验证"));
    }
    let identity = Api::load()?.identity(token[5] as i32)?;
    if identity.pid_version != token[7] {
        return Err(io::Error::other("托管 IPC 对端 PID 已失效"));
    }
    Ok(identity)
}

pub fn macos_process_identity(pid: i32) -> io::Result<MacosProcessIdentity> {
    Api::load()?.identity(pid)
}

/// 调用方必须先证明此进程归本次任务所有；内核 token 只解决身份重用，不能授予所有权。
/// 不按进程组或共享 coalition 扩大信号范围，也不在接口缺失时退化为裸 PID kill。
pub fn macos_signal_owned_process(
    expected: MacosProcessIdentity,
    boot_session: &str,
    signal: i32,
) -> io::Result<()> {
    if macos_boot_session()? != boot_session || expected.pid <= 0 {
        return Err(io::Error::other("拒绝向旧启动或无效进程发送信号"));
    }
    let api = Api::load()?;
    if api.identity(expected.pid)? != expected {
        return Err(io::Error::other("拒绝向身份已改变的进程发送信号"));
    }
    let mut token = [0u32; 8];
    token[5] = expected.pid as u32;
    token[7] = expected.pid_version;
    if unsafe { (api.signal)(&mut token, signal) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// 只能由已认证、尚未派生 CLI 的专属 job wrapper 建立；普通共享域无法领取。
pub struct MacosCoalition {
    api: Api,
    cid: u64,
    boot_session: String,
}

impl MacosCoalition {
    pub fn claim(wrapper: MacosProcessIdentity) -> io::Result<Self> {
        let api = Api::load()?;
        let boot_session = macos_boot_session()?;
        if api.identity(wrapper.pid)? != wrapper
            || wrapper.resource_cid == api.identity(std::process::id() as i32)?.resource_cid
            || api
                .usage(wrapper.resource_cid)?
                .is_none_or(|values| values[0].checked_sub(values[1]) != Some(1))
        {
            return Err(io::Error::other("托管 job 尚未建立独占资源域"));
        }
        Ok(Self {
            api,
            cid: wrapper.resource_cid,
            boot_session,
        })
    }

    pub fn id(&self) -> u64 {
        self.cid
    }

    pub fn boot_session(&self) -> &str {
        &self.boot_session
    }

    pub fn identity(&self, pid: i32) -> io::Result<MacosProcessIdentity> {
        if macos_boot_session()? != self.boot_session {
            return Err(io::Error::other("托管资源域属于另一次系统启动"));
        }
        let identity = self.api.identity(pid)?;
        if identity.resource_cid != self.cid {
            return Err(io::Error::other("进程不属于此托管资源域"));
        }
        Ok(identity)
    }

    pub fn signal_member(&self, identity: MacosProcessIdentity, signal: i32) -> io::Result<()> {
        if identity.resource_cid != self.cid || self.identity(identity.pid)? != identity {
            return Err(io::Error::other("拒绝向身份已改变的进程发送信号"));
        }
        let mut token = [0u32; 8];
        token[5] = identity.pid as u32;
        token[7] = identity.pid_version;
        if unsafe { (self.api.signal)(&mut token, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn is_destroyed(cid: u64, boot_session: &str) -> io::Result<bool> {
        if macos_boot_session()? != boot_session {
            return Err(io::Error::other("托管资源域属于另一次系统启动"));
        }
        Ok(Api::load()?.usage(cid)?.is_none())
    }

    /// 调用方先移除自己创建的 job；任何枚举遗漏仍须等最终 CID 销毁，超时绝不成功。
    pub fn terminate_and_confirm(&self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if Self::is_destroyed(self.cid, &self.boot_session)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "托管资源域尚未确认销毁",
                ));
            }
            for pid in self.api.all_pids()? {
                // 不可读、已退出或复用的候选不是清理成功；最终 CID 仍存在就继续或超时。
                let Ok(identity) = self.api.identity(pid) else {
                    continue;
                };
                if identity.resource_cid == self.cid {
                    if let Err(error) = self.signal_member(identity, libc::SIGKILL) {
                        if self
                            .api
                            .identity(pid)
                            .is_ok_and(|current| current == identity)
                        {
                            return Err(error);
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
#[path = "managed_macos_tests.rs"]
mod tests;
