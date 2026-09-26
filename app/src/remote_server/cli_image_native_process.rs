//! 原生图片消费者的只读 Unix 进程与 socket 身份；不启动进程、不读取认证文件。

use std::fs;
#[cfg(target_os = "linux")]
use std::fs::File;
use std::io;
#[cfg(target_os = "linux")]
use std::io::Read as _;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(super) enum Configuration {
    Codex,
    Claude,
}

pub(super) struct Process {
    pub(super) pid: i32,
    pub(super) identity: String,
    pub(super) uid: u32,
    pub(super) group: i32,
    pub(super) foreground_group: i32,
    pub(super) tty: u64,
    pub(super) executable: PathBuf,
    pub(super) executable_identity: (u64, u64),
    pub(super) arguments: Vec<Vec<u8>>,
    pub(super) config_home: PathBuf,
}

fn environment_home(
    entries: impl Iterator<Item = Vec<u8>>,
    configuration: Configuration,
) -> io::Result<PathBuf> {
    let (variable, default) = match configuration {
        Configuration::Codex => (b"CODEX_HOME=".as_slice(), ".codex"),
        Configuration::Claude => (b"CLAUDE_CONFIG_DIR=".as_slice(), ".claude"),
    };
    let mut codex = None;
    let mut home = None;
    for entry in entries {
        if let Some(value) = entry.strip_prefix(variable) {
            codex = Some(value.to_vec());
        }
        if let Some(value) = entry.strip_prefix(b"HOME=") {
            home = Some(value.to_vec());
        }
    }
    use std::os::unix::ffi::OsStringExt;
    if matches!(configuration, Configuration::Claude) && codex.as_ref().is_some_and(Vec::is_empty) {
        return Err(invalid());
    }
    let path = match codex.filter(|value| !value.is_empty()) {
        Some(value) => PathBuf::from(std::ffi::OsString::from_vec(value)),
        None => {
            PathBuf::from(std::ffi::OsString::from_vec(home.ok_or_else(invalid)?)).join(default)
        }
    };
    if !path.is_absolute() {
        return Err(invalid());
    }
    fs::canonicalize(path)
}

#[cfg(target_os = "linux")]
pub(super) fn process(pid: i32, configuration: Configuration) -> io::Result<Process> {
    use command::managed::LinuxProcessHandle;
    use std::os::unix::ffi::OsStrExt;
    let handle = LinuxProcessHandle::capture(pid)?;
    let before = handle.snapshot()?;
    let mut environment = Vec::new();
    File::open(format!("/proc/{pid}/environ"))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut environment)?;
    if environment.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let config_home = environment_home(
        environment.split(|byte| *byte == 0).map(ToOwned::to_owned),
        configuration,
    )?;
    let after = handle.snapshot()?;
    if before.identity != after.identity {
        return Err(invalid());
    }
    Ok(Process {
        pid,
        identity: format!("{:?}", before.identity),
        uid: before.identity.uid,
        group: before.process_group,
        foreground_group: before.foreground_group,
        tty: before.tty_device,
        executable: before.executable,
        executable_identity: (
            before.identity.executable_device,
            before.identity.executable_inode,
        ),
        arguments: before
            .arguments
            .iter()
            .map(|arg| arg.as_bytes().to_vec())
            .collect(),
        config_home,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn foreground_pids() -> io::Result<Vec<i32>> {
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if let Ok(handle) = command::managed::LinuxProcessHandle::capture(pid)
            && handle.snapshot().is_ok_and(|snapshot| {
                snapshot.identity.uid == unsafe { libc::geteuid() }
                    && snapshot.tty_device != 0
                    && snapshot.process_group > 0
                    && snapshot.process_group == snapshot.foreground_group
            })
        {
            pids.push(pid);
        }
        if pids.len() > 1024 {
            return Err(invalid());
        }
    }
    Ok(pids)
}

#[cfg(target_os = "linux")]
pub(super) fn peer_pid(stream: &UnixStream) -> io::Result<i32> {
    Ok(command::managed::linux_peer_handle(stream)?.identity().pid)
}

#[cfg(target_os = "macos")]
pub(super) fn process(pid: i32, configuration: Configuration) -> io::Result<Process> {
    let before = command::managed::macos_process_identity(pid)?;
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    if unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            std::mem::size_of_val(&info) as i32,
        )
    } != std::mem::size_of_val(&info) as i32
    {
        return Err(invalid());
    }
    let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let count = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    if count <= 0 || count as usize >= path.len() {
        return Err(invalid());
    }
    path.truncate(count as usize);
    if path.last() == Some(&0) {
        path.pop();
    }
    use std::os::unix::ffi::OsStringExt;
    let executable = PathBuf::from(std::ffi::OsString::from_vec(path));
    let metadata = fs::metadata(&executable)?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut bytes = vec![0u8; 1024 * 1024];
    let mut size = bytes.len();
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || size < 5
        || size >= bytes.len()
    {
        return Err(invalid());
    }
    bytes.truncate(size);
    let argc = i32::from_ne_bytes(bytes[..4].try_into().map_err(|_| invalid())?);
    if !(1..=32768).contains(&argc) {
        return Err(invalid());
    }
    let mut offset = 4
        + bytes[4..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(invalid)?
        + 1;
    while bytes.get(offset) == Some(&0) {
        offset += 1;
    }
    let mut arguments = Vec::new();
    for _ in 0..argc {
        let end = offset
            + bytes
                .get(offset..)
                .ok_or_else(invalid)?
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(invalid)?;
        arguments.push(bytes[offset..end].to_vec());
        offset = end + 1;
    }
    let config_home = environment_home(
        bytes
            .get(offset..)
            .ok_or_else(invalid)?
            .split(|byte| *byte == 0)
            .map(ToOwned::to_owned),
        configuration,
    )?;
    if command::managed::macos_process_identity(pid)? != before {
        return Err(invalid());
    }
    Ok(Process {
        pid,
        identity: format!("{:?}", before),
        uid: info.pbi_uid,
        group: info.pbi_pgid as i32,
        foreground_group: info.e_tpgid as i32,
        tty: info.e_tdev as u64,
        executable,
        executable_identity: (metadata.dev(), metadata.ino()),
        arguments,
        config_home,
    })
}

#[cfg(target_os = "macos")]
pub(super) fn foreground_pids() -> io::Result<Vec<i32>> {
    let mut pids = vec![0i32; 65536];
    let size =
        unsafe { libc::proc_listpids(1, 0, pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
    if size < 0 || size as usize >= pids.len() * 4 || size % 4 != 0 {
        return Err(invalid());
    }
    pids.truncate(size as usize / 4);
    pids.retain(|pid| {
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        *pid > 0
            && unsafe {
                libc::proc_pidinfo(
                    *pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    std::mem::size_of_val(&info) as i32,
                )
            } == std::mem::size_of_val(&info) as i32
            && info.pbi_uid == unsafe { libc::geteuid() }
            && info.e_tdev != 0
            && info.pbi_pgid > 0
            && info.pbi_pgid == info.e_tpgid
    });
    Ok(pids)
}

#[cfg(target_os = "macos")]
pub(super) fn peer_pid(stream: &UnixStream) -> io::Result<i32> {
    Ok(command::managed::macos_peer_identity(stream)?.pid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn process(_pid: i32, _configuration: Configuration) -> io::Result<Process> {
    Err(invalid())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn foreground_pids() -> io::Result<Vec<i32>> {
    Err(invalid())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn peer_pid(_stream: &UnixStream) -> io::Result<i32> {
    Err(invalid())
}

fn invalid() -> io::Error {
    io::Error::other("native image process identity is unavailable")
}
