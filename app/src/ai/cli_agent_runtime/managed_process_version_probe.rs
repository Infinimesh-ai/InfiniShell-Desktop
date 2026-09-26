//! Claude 2.1.280 npm 公共入口的窄版本探针，不继承普通任务或更新器的环境授权。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use uuid::Uuid;

use super::{AtomicLaunchKind, Manifest};

pub(super) fn supported_platform() -> io::Result<()> {
    if cfg!(all(
        any(target_os = "macos", target_os = "linux"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    )) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "此平台尚无 npm 版本探针合同",
        ))
    }
}

fn directory(state_dir: &Path, generation: Uuid) -> io::Result<PathBuf> {
    Ok(state_dir
        .canonicalize()?
        .join(format!("claude-npm-version-{generation}")))
}

pub(super) fn prepare_directory(state_dir: &Path, generation: Uuid) -> io::Result<PathBuf> {
    supported_platform()?;
    let root = directory(state_dir, generation)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(&root)?;
    for name in ["home", "config", "cache", "data", "tmp"] {
        builder.create(root.join(name))?;
    }
    fs::File::open(&root)?.sync_all()?;
    fs::File::open(
        root.parent()
            .ok_or_else(|| io::Error::other("版本探针缺少父目录"))?,
    )?
    .sync_all()?;
    Ok(root)
}

pub(super) fn validate(manifest: &Manifest, state_dir: &Path) -> io::Result<()> {
    supported_platform()?;
    let stage = manifest
        .executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    if manifest.atomic_launch_kind != Some(AtomicLaunchKind::ClaudeNpmVersionProbeV1)
        || manifest.arguments != [OsString::from("--version")]
        || manifest.environment.is_some()
        || manifest.isolated_home.is_some()
        || manifest.isolated_state_dir.is_some()
        || !manifest.executable.is_absolute()
        || manifest
            .executable
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        || !manifest.executable.ends_with("bin/claude.exe")
        || stage
            .and_then(|name| name.strip_prefix(".infinishell-npm-"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_none()
    {
        return Err(io::Error::other("Claude npm 版本探针的入口或参数不匹配"));
    }
    // 永久未启动记录没有候选文件/私有目录；它只证明从未执行，不可派生进程。
    if !manifest.launch_allowed {
        return Ok(());
    }
    let expected = directory(state_dir, manifest.generation)?;
    if manifest.cwd != expected
        || manifest.expected_files.len() != 1
        || manifest.expected_files[0].path != manifest.executable
        || manifest.expected_files[0].canonical_path != manifest.executable
        || manifest
            .atomic_cwd
            .as_ref()
            .is_none_or(|identity| identity.path != expected)
    {
        return Err(io::Error::other(
            "Claude npm 版本探针未绑定公共入口和私有目录",
        ));
    }
    for path in std::iter::once(expected.clone()).chain(
        ["home", "config", "cache", "data", "tmp"]
            .into_iter()
            .map(|name| expected.join(name)),
    ) {
        if path.canonicalize()? != path {
            return Err(io::Error::other("版本探针目录不能经过链接"));
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() {
            return Err(io::Error::other("版本探针目录类型不匹配"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7777 != 0o700 {
                return Err(io::Error::other("版本探针目录不是当前用户私有目录"));
            }
        }
    }
    Ok(())
}

pub(super) fn environment(root: &Path) -> Vec<(OsString, OsString)> {
    // 从空环境构造：不会继承代理、认证、Node 选项、npm 配置或动态加载器注入。
    let mut values = vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C".into()),
        ("LC_ALL".into(), "C".into()),
        ("DISABLE_AUTOUPDATER".into(), "1".into()),
        (
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
            "1".into(),
        ),
    ];
    values.extend(
        [
            ("HOME", "home"),
            ("CLAUDE_CONFIG_DIR", "config"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
            ("TMPDIR", "tmp"),
            ("TMP", "tmp"),
            ("TEMP", "tmp"),
        ]
        .into_iter()
        .map(|(name, relative)| (name.into(), root.join(relative).into_os_string())),
    );
    values
}

#[cfg(target_os = "macos")]
pub(super) fn deny_network() -> io::Result<()> {
    // 只在已完成父授权握手及签名/闭包验证的 exec worker 中应用，监督者不受影响。
    unsafe extern "C" {
        fn sandbox_init(
            profile: *const libc::c_char,
            flags: u64,
            error: *mut *mut libc::c_char,
        ) -> libc::c_int;
        fn sandbox_free_error(error: *mut libc::c_char);
    }
    let mut message = std::ptr::null_mut();
    let result = unsafe {
        sandbox_init(
            c"(version 1) (allow default) (deny network*)".as_ptr(),
            0,
            &mut message,
        )
    };
    if !message.is_null() {
        unsafe { sandbox_free_error(message) };
    }
    if result != 0 {
        return Err(io::Error::other("版本探针无法建立网络拒绝边界"));
    }
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) fn deny_network() -> io::Result<()> {
    // 过滤器继承到后续 exec/子进程；未知 ABI 和 x32 不能绕过 syscall 号比较。
    let mut filters = network_filter();
    let program = libc::sock_fprog {
        len: filters.len() as u16,
        filter: filters.as_mut_ptr(),
    };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) } != 0
    {
        return Err(io::Error::other("版本探针无法建立网络拒绝边界"));
    }
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn network_filter() -> Vec<libc::sock_filter> {
    let statement = |code, k| libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    };
    let equal = |k, jt, jf| libc::sock_filter {
        code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
        jt,
        jf,
        k,
    };
    #[cfg(target_arch = "x86_64")]
    let architecture = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    let architecture = 0xc00000b7;
    let load = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    let ret = (libc::BPF_RET | libc::BPF_K) as u16;
    let mut filters = vec![
        statement(load, 4),
        equal(architecture, 1, 0),
        statement(ret, libc::SECCOMP_RET_KILL_PROCESS),
        statement(load, 0),
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: 0x40000000,
        },
        statement(ret, libc::SECCOMP_RET_KILL_PROCESS),
    ];
    for syscall in [
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_sendto,
        libc::SYS_sendmsg,
        libc::SYS_sendmmsg,
        libc::SYS_io_uring_setup,
    ] {
        filters.push(equal(syscall as u32, 0, 1));
        filters.push(statement(ret, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32));
    }
    filters.push(statement(ret, libc::SECCOMP_RET_ALLOW));
    filters
}

#[cfg(not(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
pub(super) fn deny_network() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "此平台尚无版本探针网络边界",
    ))
}

#[cfg(test)]
#[path = "managed_process_version_probe_tests.rs"]
mod tests;
