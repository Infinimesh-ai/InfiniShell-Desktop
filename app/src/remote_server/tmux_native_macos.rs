//! 使用系统 SDK socket 信息与已有托管进程身份；不读取进程环境。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::mem::{self, MaybeUninit};
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use command::managed::{MacosProcessIdentity, macos_peer_handle, macos_process_identity};

use super::{Budget, MAX_FDS, MAX_PIDS, ProcessSnapshot, SocketEndpoint, invalid};

#[repr(C)]
struct FlatProcess {
    pid: i32,
    parent: i32,
    uid: u32,
    group: i32,
    foreground_group: i32,
    session: i32,
    tty: u64,
    image_length: u32,
    cwd_length: u32,
    image: [u8; 4096],
    cwd: [u8; 4096],
}

#[repr(C)]
struct FlatSocket {
    descriptor: i32,
    listening: u32,
    socket: u64,
    protocol: u64,
    peer_socket: u64,
    peer_protocol: u64,
    local_length: u32,
    peer_length: u32,
    local_path: [u8; 256],
    peer_path: [u8; 256],
}

unsafe extern "C" {
    fn isp_tmux_process_snapshot(pid: i32, out: *mut FlatProcess, size: u32) -> i32;
    fn isp_tmux_list_pids(group: i32, out: *mut i32, capacity: u32, count: *mut u32) -> i32;
    fn isp_tmux_socket_inventory(
        pid: i32,
        out: *mut FlatSocket,
        size: u32,
        capacity: u32,
        count: *mut u32,
    ) -> i32;
}

pub(super) struct Process {
    identity: MacosProcessIdentity,
}

impl Process {
    pub(super) fn capture(pid: i32) -> io::Result<Self> {
        let process = Self {
            identity: macos_process_identity(pid)?,
        };
        process.snapshot()?;
        Ok(process)
    }

    pub(super) fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
    }

    #[cfg(test)]
    pub(super) fn identity_for_test(&self) -> MacosProcessIdentity {
        self.identity
    }

    pub(super) fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        if macos_process_identity(self.identity.pid)? != self.identity {
            return Err(invalid("tmux 进程生存期已改变"));
        }
        let mut raw = MaybeUninit::<FlatProcess>::zeroed();
        let result = unsafe {
            isp_tmux_process_snapshot(
                self.identity.pid,
                raw.as_mut_ptr(),
                mem::size_of::<FlatProcess>() as u32,
            )
        };
        check_result(result)?;
        let raw = unsafe { raw.assume_init() };
        let executable = required_path(&raw.image, raw.image_length)?;
        let cwd = required_path(&raw.cwd, raw.cwd_length)?;
        let metadata = fs::metadata(&executable)?;
        if raw.pid != self.identity.pid
            || raw.uid != unsafe { libc::geteuid() }
            || !metadata.is_file()
            || macos_process_identity(self.identity.pid)? != self.identity
        {
            return Err(invalid("tmux 进程映像或内核身份已改变"));
        }
        Ok(ProcessSnapshot {
            pid: raw.pid,
            parent: raw.parent,
            uid: raw.uid,
            group: raw.group,
            foreground_group: raw.foreground_group,
            session: raw.session,
            tty: raw.tty,
            executable,
            executable_file: (metadata.dev(), metadata.ino()),
            cwd,
        })
    }

    pub(super) fn sockets(&self, budget: &Budget) -> io::Result<Vec<SocketEndpoint>> {
        budget.check()?;
        self.snapshot()?;
        let mut raw: Vec<MaybeUninit<FlatSocket>> =
            (0..MAX_FDS).map(|_| MaybeUninit::zeroed()).collect();
        let mut count = 0;
        let result = unsafe {
            isp_tmux_socket_inventory(
                self.identity.pid,
                raw.as_mut_ptr().cast(),
                mem::size_of::<FlatSocket>() as u32,
                MAX_FDS as u32,
                &mut count,
            )
        };
        check_result(result)?;
        if count as usize > MAX_FDS {
            return Err(invalid("tmux socket 数量超出预算"));
        }
        let mut endpoints = Vec::with_capacity(count as usize);
        for entry in raw.into_iter().take(count as usize) {
            let entry = unsafe { entry.assume_init() };
            if entry.listening > 1 || entry.descriptor < 0 {
                return Err(invalid("tmux socket 数据无效"));
            }
            endpoints.push(SocketEndpoint {
                descriptor: entry.descriptor,
                socket: entry.socket,
                protocol: entry.protocol,
                peer_socket: entry.peer_socket,
                peer_protocol: entry.peer_protocol,
                listening: entry.listening == 1,
                local_path: optional_path(&entry.local_path, entry.local_length)?,
                peer_path: optional_path(&entry.peer_path, entry.peer_length)?,
            });
        }
        self.snapshot()?;
        budget.check()?;
        Ok(endpoints)
    }
}

pub(super) fn validate_peer(
    stream: &UnixStream,
    expected_server: &Process,
    budget: &Budget,
) -> io::Result<()> {
    budget.check()?;
    expected_server.snapshot()?;
    let handle = macos_peer_handle(stream)?;
    let process = Process {
        identity: handle.identity(),
    };
    process.snapshot()?;
    if !expected_server.same_identity(&process) {
        return Err(invalid("tmux 新连接不属于已绑定 server"));
    }
    expected_server.snapshot()?;
    budget.check()
}

pub(super) fn pids(group: Option<i32>, budget: &Budget) -> io::Result<Vec<i32>> {
    budget.check()?;
    if group.is_some_and(|value| value <= 0) {
        return Err(invalid("tmux 前台进程组无效"));
    }
    let mut values = vec![0; MAX_PIDS];
    let mut count = 0;
    let result = unsafe {
        isp_tmux_list_pids(
            group.unwrap_or(0),
            values.as_mut_ptr(),
            MAX_PIDS as u32,
            &mut count,
        )
    };
    check_result(result)?;
    if count as usize > MAX_PIDS {
        return Err(invalid("tmux 进程候选超出预算"));
    }
    values.truncate(count as usize);
    values.retain(|pid| *pid > 1);
    budget.check()?;
    Ok(values)
}

fn check_result(result: i32) -> io::Result<()> {
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    Ok(())
}

fn required_path(bytes: &[u8], length: u32) -> io::Result<PathBuf> {
    optional_path(bytes, length)?.ok_or_else(|| invalid("tmux 内核路径为空"))
}

fn optional_path(bytes: &[u8], length: u32) -> io::Result<Option<PathBuf>> {
    let bytes = bytes
        .get(..length as usize)
        .ok_or_else(|| invalid("tmux 内核路径越界"))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.contains(&0) {
        return Err(invalid("tmux 内核路径含 NUL"));
    }
    let path = PathBuf::from(OsString::from_vec(bytes.to_vec()));
    if !path.is_absolute() {
        return Err(invalid("tmux 内核路径不是绝对路径"));
    }
    Ok(Some(path))
}
