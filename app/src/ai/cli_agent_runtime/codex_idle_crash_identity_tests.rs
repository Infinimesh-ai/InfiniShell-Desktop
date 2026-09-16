//! 只供空闲崩溃夹具使用的进程身份绑定；不提供按名称或裸 PID 的终止后备路径。

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;
    use std::io;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::path::Path;

    pub const MECHANISM: &str = "linux_pidfd_send_signal";

    pub struct BoundProcess {
        pid: u32,
        descriptor: OwnedFd,
        identity: (String, String),
    }

    fn identity(pid: u32) -> (String, String) {
        let record = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields: Vec<_> = record[record.rfind(')').unwrap() + 2..]
            .split_whitespace()
            .collect();
        assert_ne!(fields[0], "Z", "原生进程已经退出，不能算故障注入");
        (fields[1].to_owned(), fields[19].to_owned())
    }

    impl BoundProcess {
        pub fn open(pid: u32, executable: &Path) -> Self {
            // pidfd 绑定内核进程对象；若内核不支持就失败，禁止降级为 kill(pid)。
            let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
            assert!(
                descriptor >= 0,
                "pidfd_open 失败：{}",
                io::Error::last_os_error()
            );
            let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor as i32) };
            let bound = Self {
                pid,
                descriptor,
                identity: identity(pid),
            };
            bound.assert_current(executable);
            bound
        }

        pub fn assert_current(&self, executable: &Path) {
            let mut poll = libc::pollfd {
                fd: self.descriptor.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            assert_eq!(
                unsafe { libc::poll(&mut poll, 1, 0) },
                0,
                "绑定的进程已退出"
            );
            assert_eq!(identity(self.pid), self.identity, "原生进程身份变化");
            assert_eq!(
                fs::read_link(format!("/proc/{}/exe", self.pid))
                    .unwrap()
                    .canonicalize()
                    .unwrap(),
                executable
            );
        }

        pub fn terminate_and_wait(&self) {
            assert_eq!(
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        self.descriptor.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0u32,
                    )
                },
                0,
                "pidfd 信号失败：{}",
                io::Error::last_os_error()
            );
            self.wait_for_exit(5000);
        }

        pub fn wait_for_exit(&self, timeout_ms: u32) {
            let mut poll = libc::pollfd {
                fd: self.descriptor.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            assert_eq!(
                unsafe { libc::poll(&mut poll, 1, timeout_ms as i32) },
                1,
                "原生进程退出超时"
            );
            assert_ne!(poll.revents & libc::POLLIN, 0, "必须观察到 pidfd 退出事件");
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::path::{Path, PathBuf};

    use windows::Win32::Foundation::{FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessId, GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    };
    use windows::core::PWSTR;

    pub const MECHANISM: &str = "windows_owned_process_handle";

    pub struct BoundProcess {
        handle: OwnedHandle,
        pid: u32,
        created: u64,
    }

    fn created(handle: HANDLE) -> u64 {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
            .unwrap();
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)
    }

    impl BoundProcess {
        pub fn open(pid: u32, executable: &Path) -> Self {
            let raw = unsafe {
                OpenProcess(
                    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )
            }
            .expect("必须取得本次真实 CLI 的进程句柄");
            let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
            let bound = Self {
                handle,
                pid,
                created: created(raw),
            };
            bound.assert_current(executable);
            bound
        }

        pub fn assert_current(&self, executable: &Path) {
            let handle = HANDLE(self.handle.as_raw_handle());
            assert_eq!(
                unsafe { WaitForSingleObject(handle, 0) },
                WAIT_TIMEOUT,
                "原生进程已退出"
            );
            assert_eq!(unsafe { GetProcessId(handle) }, self.pid);
            assert_eq!(created(handle), self.created);
            let mut buffer = vec![0u16; 32768];
            let mut length = buffer.len() as u32;
            unsafe {
                QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    PWSTR(buffer.as_mut_ptr()),
                    &mut length,
                )
            }
            .unwrap();
            let image = PathBuf::from(String::from_utf16(&buffer[..length as usize]).unwrap());
            assert_eq!(image.canonicalize().unwrap(), executable);
        }

        pub fn terminate_and_wait(&self) {
            // 句柄固定了进程对象；不调用 taskkill，也不按 PID 再打开一个可能复用的对象。
            let handle = HANDLE(self.handle.as_raw_handle());
            unsafe { TerminateProcess(handle, 73) }.expect("终止真实原生根进程失败");
            self.wait_for_exit(5000);
            let mut code = 0;
            unsafe { GetExitCodeProcess(handle, &mut code) }.unwrap();
            assert_eq!(code, 73, "必须终止真实 CLI，不能以其他进程退出替代");
        }

        pub fn wait_for_exit(&self, timeout_ms: u32) {
            assert_eq!(
                unsafe { WaitForSingleObject(HANDLE(self.handle.as_raw_handle()), timeout_ms) },
                WAIT_OBJECT_0,
                "绑定进程退出超时"
            );
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;
    use std::io;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::path::{Path, PathBuf};
    use std::ptr;

    use command::managed::{MacosProcessIdentity, macos_process_identity};

    pub const MECHANISM: &str = "darwin_audit_token_pidversion";

    struct Library(*mut c_void);

    impl Drop for Library {
        fn drop(&mut self) {
            unsafe { libc::dlclose(self.0) };
        }
    }

    type SignalWithToken = unsafe extern "C" fn(*const [u32; 8], i32) -> i32;

    pub struct BoundProcess {
        pid: u32,
        unique: MacosProcessIdentity,
        watcher: OwnedFd,
        library: Library,
    }

    impl BoundProcess {
        pub fn open(pid: u32, executable: &Path) -> Self {
            let library = unsafe {
                libc::dlopen(
                    c"/usr/lib/libproc.dylib".as_ptr(),
                    libc::RTLD_NOW | libc::RTLD_LOCAL,
                )
            };
            assert!(!library.is_null(), "平台没有可验证的 libproc");
            let library = Library(library);
            let unique = macos_process_identity(pid as i32).expect("必须读取稳定的原生进程身份");
            let descriptor = unsafe { libc::kqueue() };
            assert!(descriptor >= 0, "无法观察真实根进程退出");
            let watcher = unsafe { OwnedFd::from_raw_fd(descriptor) };
            let event = libc::kevent {
                ident: pid as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: ptr::null_mut(),
            };
            assert_eq!(
                unsafe {
                    libc::kevent(
                        watcher.as_raw_fd(),
                        &event,
                        1,
                        ptr::null_mut(),
                        0,
                        ptr::null(),
                    )
                },
                0,
                "无法注册原生根进程 NOTE_EXIT：{}",
                io::Error::last_os_error()
            );
            let bound = Self {
                pid,
                unique,
                watcher,
                library,
            };
            bound.assert_current(executable);
            bound
        }

        pub fn assert_current(&self, executable: &Path) {
            assert_eq!(
                macos_process_identity(self.pid as i32).unwrap(),
                self.unique,
                "原生进程 PID version、unique ID 或资源域已变化"
            );
            let mut buffer = [0u8; 4096];
            let count = unsafe {
                libc::proc_pidpath(
                    self.pid as i32,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                )
            };
            assert!(count > 0, "无法核对原生进程路径");
            let length = buffer.iter().position(|byte| *byte == 0).unwrap();
            let image = PathBuf::from(std::str::from_utf8(&buffer[..length]).unwrap());
            assert_eq!(image.canonicalize().unwrap(), executable);
        }

        pub fn terminate_and_wait(&self) {
            assert_eq!(
                macos_process_identity(self.pid as i32).unwrap(),
                self.unique,
                "信号前原生进程身份改变，拒绝终止"
            );
            let symbol =
                unsafe { libc::dlsym(self.library.0, c"proc_signal_with_audittoken".as_ptr()) };
            assert!(
                !symbol.is_null(),
                "此 macOS 不支持绑定 PID version 的信号，不能降级为裸 PID"
            );
            let signal: SignalWithToken = unsafe { std::mem::transmute(symbol) };
            let mut token = [0u32; 8];
            token[5] = self.pid;
            token[7] = self.unique.pid_version;
            assert_eq!(
                unsafe { signal(&token, libc::SIGKILL) },
                0,
                "原生进程身份信号失败：{}",
                io::Error::last_os_error()
            );
            self.wait_for_exit(5000);
        }

        pub fn wait_for_exit(&self, timeout_ms: u32) {
            let mut event = libc::kevent {
                ident: 0,
                filter: 0,
                flags: 0,
                fflags: 0,
                data: 0,
                udata: ptr::null_mut(),
            };
            let timeout = libc::timespec {
                tv_sec: (timeout_ms / 1000).into(),
                tv_nsec: ((timeout_ms % 1000) * 1_000_000).into(),
            };
            assert_eq!(
                unsafe {
                    libc::kevent(
                        self.watcher.as_raw_fd(),
                        ptr::null(),
                        0,
                        &mut event,
                        1,
                        &timeout,
                    )
                },
                1,
                "真实原生根进程退出超时"
            );
            let (pid, filter, flags, notes) =
                (event.ident, event.filter, event.flags, event.fflags);
            assert_eq!(pid, self.pid as usize);
            assert_eq!(filter, libc::EVFILT_PROC);
            assert_eq!(flags & libc::EV_ERROR, 0);
            assert_ne!(notes & libc::NOTE_EXIT, 0);
        }
    }
}

pub(super) use platform::{BoundProcess, MECHANISM};
