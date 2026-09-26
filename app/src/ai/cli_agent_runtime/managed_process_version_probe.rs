//! Claude 2.1.280 包入口的窄版本探针；各包来源分别验证布局，不继承用户环境。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use uuid::Uuid;

use super::{AtomicLaunchKind, Manifest};

pub(super) fn is_probe(kind: Option<AtomicLaunchKind>) -> bool {
    matches!(
        kind,
        Some(
            AtomicLaunchKind::ClaudeNpmVersionProbeV1
                | AtomicLaunchKind::ClaudeHomebrewVersionProbeV1
                | AtomicLaunchKind::ClaudeWingetVersionProbeV1
                | AtomicLaunchKind::CodexHomebrewVersionProbeV1
                | AtomicLaunchKind::GrokHomebrewVersionProbeV1
                | AtomicLaunchKind::CodexNpmVersionProbeV1
                | AtomicLaunchKind::CodexWindowsNpmVersionProbeV1
                | AtomicLaunchKind::CodexWindowsWingetVersionProbeV1
                | AtomicLaunchKind::GrokWindowsWingetVersionProbeV1
                | AtomicLaunchKind::GrokWindowsNpmVersionProbeV1
                | AtomicLaunchKind::ClaudeWindowsNpmVersionProbeV1
                | AtomicLaunchKind::GrokNpmVersionProbeV1
        )
    )
}

fn valid_entry(manifest: &Manifest) -> bool {
    let parent = manifest.executable.parent();
    let stage = parent
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    match manifest.atomic_launch_kind {
        Some(AtomicLaunchKind::GrokNpmVersionProbeV1) => {
            cfg!(any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )) && manifest.executable.ends_with("bin/grok-native")
                && stage
                    .and_then(|name| name.strip_prefix(".infinishell-grok-npm-"))
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .is_some_and(|id| !id.is_nil())
        }
        Some(AtomicLaunchKind::ClaudeNpmVersionProbeV1) => {
            cfg!(unix)
                && manifest.executable.ends_with("bin/claude.exe")
                && stage
                    .and_then(|name| name.strip_prefix(".infinishell-npm-"))
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .is_some_and(|id| !id.is_nil())
        }
        Some(AtomicLaunchKind::ClaudeHomebrewVersionProbeV1) => {
            let version = parent
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let package = parent.and_then(Path::parent);
            let name = package
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let candidate = version == Some("2.1.280")
                && name
                    .and_then(|name| name.strip_prefix(".infinishell-brew-"))
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .is_some_and(|id| !id.is_nil());
            manifest
                .executable
                .file_name()
                .is_some_and(|name| name == "claude")
                && if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                    package.and_then(Path::parent)
                        == Some(Path::new("/home/linuxbrew/.linuxbrew/Caskroom"))
                        && (candidate
                            || matches!(version, Some("2.1.278" | "2.1.280"))
                                && name == Some("claude-code@latest"))
                } else {
                    cfg!(all(target_os = "macos", target_arch = "aarch64")) && candidate
                }
        }
        Some(AtomicLaunchKind::ClaudeWingetVersionProbeV1) => {
            cfg!(windows)
                && manifest
                    .executable
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_prefix(".infinishell-winget-"))
                    .and_then(|name| name.strip_suffix(".exe"))
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .is_some_and(|id| !id.is_nil())
                && parent.and_then(Path::file_name).is_some_and(|name| {
                    name == "Anthropic.ClaudeCode_Microsoft.Winget.Source_8wekyb3d8bbwe"
                })
        }
        Some(AtomicLaunchKind::GrokHomebrewVersionProbeV1) => {
            let version = parent
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let package = parent.and_then(Path::parent);
            let name = package
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let (target, caskroom) = match (std::env::consts::OS, std::env::consts::ARCH) {
                ("macos", "aarch64") => ("macos-aarch64", "/opt/homebrew/Caskroom"),
                ("linux", "x86_64") => ("linux-x86_64", "/home/linuxbrew/.linuxbrew/Caskroom"),
                _ => return false,
            };
            version.is_some_and(|version| {
                manifest
                    .executable
                    .file_name()
                    .is_some_and(|name| name == format!("grok-{version}-{target}").as_str())
            }) && (version == Some("1.0.41")
                && name
                    .and_then(|name| name.strip_prefix(".infinishell-brew-"))
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .is_some_and(|id| !id.is_nil())
                || matches!(version, Some("1.0.40" | "1.0.41")) && name == Some("grok-build"))
                && package.and_then(Path::parent) == Some(Path::new(caskroom))
        }
        Some(AtomicLaunchKind::CodexHomebrewVersionProbeV1) => {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                super::codex_cask_linux_probe::valid_entry(manifest)
            }
            #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
            {
                let version = parent.and_then(Path::parent);
                let package = version.and_then(Path::parent);
                let version = version
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str());
                let stage = package
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str());
                cfg!(target_os = "macos")
                    && manifest.executable.ends_with("bin/codex")
                    && (version == Some("0.156.1")
                        && stage
                            .and_then(|name| name.strip_prefix(".infinishell-brew-"))
                            .and_then(|id| Uuid::parse_str(id).ok())
                            .is_some_and(|id| !id.is_nil())
                        || matches!(version, Some("0.155.1" | "0.156.1"))
                            && stage == Some("codex")
                            && package.and_then(Path::parent).is_some_and(|path| {
                                matches!(
                                    path.to_str(),
                                    Some("/opt/homebrew/Caskroom" | "/usr/local/Caskroom")
                                )
                            }))
            }
        }
        Some(AtomicLaunchKind::CodexNpmVersionProbeV1) => {
            #[cfg(unix)]
            {
                super::npm_probe::entry_root(&manifest.executable, &manifest.arguments).is_ok()
            }
            #[cfg(not(unix))]
            {
                false
            }
        }
        Some(AtomicLaunchKind::GrokWindowsNpmVersionProbeV1) => {
            #[cfg(windows)]
            {
                super::grok_npm_windows_probe::valid_entry(
                    &manifest.executable,
                    &manifest.arguments,
                )
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Some(AtomicLaunchKind::CodexWindowsWingetVersionProbeV1) => {
            #[cfg(windows)]
            {
                super::codex_winget_windows_probe::valid_entry(
                    &manifest.executable,
                    &manifest.arguments,
                )
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Some(AtomicLaunchKind::GrokWindowsWingetVersionProbeV1) => {
            #[cfg(windows)]
            {
                super::grok_winget_windows_probe::valid_entry(
                    &manifest.executable,
                    &manifest.arguments,
                )
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Some(AtomicLaunchKind::CodexWindowsNpmVersionProbeV1) => {
            #[cfg(windows)]
            {
                super::npm_windows_probe::valid_entry(&manifest.executable, &manifest.arguments)
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Some(AtomicLaunchKind::ClaudeWindowsNpmVersionProbeV1) => {
            #[cfg(windows)]
            {
                super::claude_npm_windows_probe::valid_entry(
                    &manifest.executable,
                    &manifest.arguments,
                )
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Some(
            AtomicLaunchKind::NativeFile
            | AtomicLaunchKind::WindowsReviewedProjectCommandV1
            | AtomicLaunchKind::UnixReviewedProjectCommandV1,
        )
        | None => false,
    }
}

pub(super) fn supported_platform() -> io::Result<()> {
    if cfg!(all(
        any(target_os = "macos", target_os = "linux", windows),
        any(target_arch = "aarch64", target_arch = "x86_64")
    )) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "此平台尚无包版本探针合同",
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
    #[cfg(unix)]
    {
        fs::File::open(&root)?.sync_all()?;
        fs::File::open(
            root.parent()
                .ok_or_else(|| io::Error::other("版本探针缺少父目录"))?,
        )?
        .sync_all()?;
    }
    Ok(root)
}

pub(super) fn validate(manifest: &Manifest, state_dir: &Path) -> io::Result<()> {
    supported_platform()?;
    let codex_windows_winget =
        manifest.atomic_launch_kind == Some(AtomicLaunchKind::CodexWindowsWingetVersionProbeV1);
    let grok_windows_winget =
        manifest.atomic_launch_kind == Some(AtomicLaunchKind::GrokWindowsWingetVersionProbeV1);
    let codex_npm = manifest.atomic_launch_kind == Some(AtomicLaunchKind::CodexNpmVersionProbeV1);
    let codex_windows_npm =
        manifest.atomic_launch_kind == Some(AtomicLaunchKind::CodexWindowsNpmVersionProbeV1);
    let claude_windows_npm =
        manifest.atomic_launch_kind == Some(AtomicLaunchKind::ClaudeWindowsNpmVersionProbeV1);
    let grok_windows_npm =
        manifest.atomic_launch_kind == Some(AtomicLaunchKind::GrokWindowsNpmVersionProbeV1);
    let valid_arguments = (codex_npm
        || codex_windows_npm
        || claude_windows_npm
        || grok_windows_npm
        || codex_windows_winget
        || grok_windows_winget)
        && valid_entry(manifest)
        || manifest.arguments == [OsString::from("--version")]
        || manifest.atomic_launch_kind == Some(AtomicLaunchKind::CodexHomebrewVersionProbeV1)
            && matches!(manifest.arguments.as_slice(), [command, shell] if command == "completion" && matches!(shell.to_str(), Some("bash" | "zsh" | "fish")))
        || manifest.atomic_launch_kind == Some(AtomicLaunchKind::GrokHomebrewVersionProbeV1)
            && matches!(manifest.arguments.as_slice(), [command, shell] if command == "completions" && matches!(shell.to_str(), Some("bash" | "zsh" | "fish")));
    if !valid_entry(manifest)
        || !valid_arguments
        || manifest.environment.is_some()
        || manifest.isolated_home.is_some()
        || manifest.isolated_state_dir.is_some()
        || !manifest.executable.is_absolute()
        || manifest
            .executable
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(io::Error::other("Claude 包版本探针的入口或参数不匹配"));
    }
    // 永久未启动记录没有候选文件/私有目录；它只证明从未执行，不可派生进程。
    if !manifest.launch_allowed {
        return Ok(());
    }
    if manifest.atomic_launch_kind == Some(AtomicLaunchKind::GrokNpmVersionProbeV1) {
        let expected = if cfg!(target_os = "macos") {
            (
                145657952,
                "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d",
            )
        } else {
            (
                165967424,
                "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407",
            )
        };
        if manifest
            .expected_files
            .first()
            .is_none_or(|file| file.size != expected.0 || file.sha256 != expected.1)
        {
            return Err(io::Error::other("Grok npm 探针不属于固定发行映像"));
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if manifest.atomic_launch_kind == Some(AtomicLaunchKind::CodexHomebrewVersionProbeV1) {
        super::codex_cask_linux_probe::validate(manifest)?;
    }
    let expected = directory(state_dir, manifest.generation)?;
    if manifest.cwd != expected
        || !(codex_npm
            || codex_windows_npm
            || claude_windows_npm
            || grok_windows_npm
            || codex_windows_winget
            || grok_windows_winget)
            && (manifest.expected_files.len() != 1
                || manifest.expected_files[0].path != manifest.executable
                || manifest.expected_files[0].canonical_path != manifest.executable)
        || manifest
            .atomic_cwd
            .as_ref()
            .is_none_or(|identity| identity.path != expected)
    {
        return Err(io::Error::other(
            "Claude 包版本探针未绑定公共入口和私有目录",
        ));
    }
    if cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && manifest.atomic_launch_kind == Some(AtomicLaunchKind::ClaudeHomebrewVersionProbeV1)
    {
        let (length, digest) = match manifest
            .executable
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
        {
            Some("2.1.278") => (
                234_119_480,
                "5c4735937844e84f8a93306e841a5b0e12252909b07870f789b190468da147ab",
            ),
            Some("2.1.280") => (
                233_709_640,
                "1e08503dbdf3c2cb0d706d32f3408277388d1c76ef108673e8fe42c1b322925b",
            ),
            _ => return Err(io::Error::other("Claude Linux cask 探针版本不匹配")),
        };
        if manifest.expected_files.len() != 1
            || manifest.expected_files[0].size != length
            || manifest.expected_files[0].sha256 != digest
        {
            return Err(io::Error::other("Claude Linux cask 探针不属于固定原生映像"));
        }
    }
    if manifest.atomic_launch_kind == Some(AtomicLaunchKind::GrokHomebrewVersionProbeV1) {
        let version = manifest
            .executable
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str());
        let (length, digest) = match (std::env::consts::OS, version) {
            ("macos", Some("1.0.40")) => (
                145_308_720,
                "3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2",
            ),
            ("macos", Some("1.0.41")) => (
                145_657_952,
                "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d",
            ),
            ("linux", Some("1.0.40")) => (
                165_587_968,
                "92c997dfd109c0672d40d5ae6fbd15835d53ffaf12cf9ea124d22aaef3ff23fc",
            ),
            ("linux", Some("1.0.41")) => (
                165_967_424,
                "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407",
            ),
            _ => return Err(io::Error::other("Grok cask 探针版本不匹配")),
        };
        if manifest.expected_files.len() != 1
            || manifest.expected_files[0].size != length
            || manifest.expected_files[0].sha256 != digest
        {
            return Err(io::Error::other("Grok cask 探针原生映像不匹配"));
        }
    }
    #[cfg(unix)]
    if codex_npm {
        super::npm_probe::validate_contract(
            &manifest.executable,
            &manifest.arguments,
            &manifest.expected_files,
        )?;
    }
    #[cfg(windows)]
    if grok_windows_npm {
        super::grok_npm_windows_probe::validate_manifest(manifest)?;
    }
    #[cfg(windows)]
    if codex_windows_winget {
        super::codex_winget_windows_probe::validate_manifest(manifest)?;
    }
    #[cfg(windows)]
    if grok_windows_winget {
        super::grok_winget_windows_probe::validate_manifest(manifest)?;
    }
    #[cfg(windows)]
    if codex_windows_npm {
        super::npm_windows_probe::validate_manifest(manifest)?;
    }
    #[cfg(windows)]
    if claude_windows_npm {
        super::claude_npm_windows_probe::validate_manifest(manifest)?;
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

pub(super) fn resolved_environment(root: &Path) -> io::Result<Vec<(OsString, OsString)>> {
    let values = environment(root);
    #[cfg(not(windows))]
    {
        Ok(values)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt as _;
        use windows::Win32::System::SystemInformation::{
            GetSystemDirectoryW, GetWindowsDirectoryW,
        };
        let mut system = [0_u16; 32768];
        let mut windows = [0_u16; 32768];
        let system_length = unsafe { GetSystemDirectoryW(Some(&mut system)) } as usize;
        let windows_length = unsafe { GetWindowsDirectoryW(Some(&mut windows)) } as usize;
        if system_length == 0
            || system_length >= system.len()
            || windows_length == 0
            || windows_length >= windows.len()
        {
            return Err(io::Error::other("版本探针无法绑定 Windows 系统目录"));
        }
        let mut values = values;
        values.retain(|(name, _)| name != "PATH");
        values.push(("PATH".into(), OsString::from_wide(&system[..system_length])));
        values.push((
            "SystemRoot".into(),
            OsString::from_wide(&windows[..windows_length]),
        ));
        for (name, relative) in [
            ("USERPROFILE", "home"),
            ("LOCALAPPDATA", "data"),
            ("APPDATA", "config"),
        ] {
            values.push((name.into(), root.join(relative).into_os_string()));
        }
        Ok(values)
    }
}

pub(super) fn cleanup_confirmed(path: &Path, manifest: &Manifest) -> bool {
    if !matches!(
        manifest.atomic_launch_kind,
        Some(
            AtomicLaunchKind::ClaudeWingetVersionProbeV1
                | AtomicLaunchKind::CodexWindowsNpmVersionProbeV1
                | AtomicLaunchKind::CodexWindowsWingetVersionProbeV1
                | AtomicLaunchKind::GrokWindowsWingetVersionProbeV1
                | AtomicLaunchKind::GrokWindowsNpmVersionProbeV1
                | AtomicLaunchKind::ClaudeWindowsNpmVersionProbeV1
        )
    ) || !manifest.launch_allowed
    {
        return true;
    }
    // Job 清空仅证明进程退出；容器 profile 与临时 ACL 也必须由可信 worker 完成恢复。
    path.parent().is_some_and(|directory| {
        let receipt = directory.join("appcontainer-cleanup-v1");
        fs::symlink_metadata(&receipt).is_ok_and(|metadata| {
            metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() == 58
        }) && fs::read(receipt).is_ok_and(|bytes| {
            bytes == b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n"
        })
    })
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
