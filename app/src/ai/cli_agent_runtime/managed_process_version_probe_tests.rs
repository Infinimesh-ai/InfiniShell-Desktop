use super::super::{
    AtomicDirectoryIdentity, ExpectedFileIdentity, ManagedEnvironment, PreparedLaunchBinding,
};
use super::*;

#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
use std::os::unix::fs::{PermissionsExt as _, symlink};

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn fixture() -> (tempfile::TempDir, Manifest) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let executable = root
        .join(format!(".infinishell-npm-{}", Uuid::new_v4()))
        .join("bin/claude.exe");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, b"controlled candidate").unwrap();
    let generation = Uuid::new_v4();
    let cwd = prepare_directory(&root, generation).unwrap();
    let manifest = Manifest {
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:1234".parse().unwrap(),
        expected_files: vec![ExpectedFileIdentity::capture(&executable).unwrap()],
        grok_stdio_eof: None,
        executable,
        arguments: vec!["--version".into()],
        atomic_cwd: Some(AtomicDirectoryIdentity::capture(&cwd).unwrap()),
        cwd,
        isolated_home: None,
        isolated_state_dir: None,
        environment: None,
        atomic_launch_kind: Some(AtomicLaunchKind::ClaudeNpmVersionProbeV1),
        #[cfg(windows)]
        child_image: None,
        #[cfg(all(windows, target_arch = "x86_64"))]
        grok_npm_source: None,
    };
    (temporary, manifest)
}

#[test]
fn probe_binding_does_not_inherit_generic_update_authority() {
    let binding = PreparedLaunchBinding::claude_npm_version_probe("a".repeat(64)).unwrap();
    assert!(binding.validate_update_execution().is_err());
    assert!(!binding.is_native_file());
    assert_eq!(binding.kind_name(), Some("claude_npm_version_probe_v1"));
}

#[test]
fn fixed_environment_has_no_inherited_auth_proxy_or_runtime_injection() {
    let root = Path::new("/private-probe");
    let values = environment(root)
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(values.len(), 13);
    assert_eq!(
        values.get(std::ffi::OsStr::new("HOME")),
        Some(&root.join("home").into_os_string())
    );
    assert_eq!(
        values.get(std::ffi::OsStr::new("CLAUDE_CONFIG_DIR")),
        Some(&root.join("config").into_os_string())
    );
    for name in [
        "ANTHROPIC_API_KEY",
        "HTTP_PROXY",
        "NODE_OPTIONS",
        "npm_config_registry",
        "DYLD_INSERT_LIBRARIES",
        "LD_PRELOAD",
    ] {
        assert!(!values.contains_key(std::ffi::OsStr::new(name)));
    }
}

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn version_probe_rejects_install_command_and_environment_overrides() {
    let (temporary, mut manifest) = fixture();
    let root = temporary.path().canonicalize().unwrap();
    validate(&manifest, &root).unwrap();
    manifest.arguments = vec!["install".into(), "2.1.280".into()];
    assert!(validate(&manifest, &root).is_err());
    manifest.arguments = vec!["--version".into()];
    manifest.environment = Some(ManagedEnvironment::default());
    assert!(validate(&manifest, &root).is_err());
}

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn version_probe_rejects_another_generation_and_dependency_native_path() {
    let (temporary, mut manifest) = fixture();
    let root = temporary.path().canonicalize().unwrap();
    let original_generation = manifest.generation;
    manifest.generation = Uuid::new_v4();
    assert!(validate(&manifest, &root).is_err());
    manifest.generation = original_generation;
    manifest.executable = manifest.executable.parent().unwrap().join("claude");
    assert!(validate(&manifest, &root).is_err());
}

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn version_probe_rejects_home_symlink_replacement() {
    let (temporary, manifest) = fixture();
    let root = temporary.path().canonicalize().unwrap();
    let home = manifest.cwd.join("home");
    fs::remove_dir(&home).unwrap();
    std::os::unix::fs::symlink(&root, &home).unwrap();
    assert!(validate(&manifest, &root).is_err());
}

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn network_denial_is_applied_only_inside_a_dedicated_worker_process() {
    let name = format!(
        "{}::network_boundary_worker",
        module_path!().split_once("::").unwrap().1
    );
    let output = command::blocking::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--ignored", "--nocapture"])
        .env_clear()
        .env("ISP_NPM_PROBE_NETWORK_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
#[ignore = "仅由父测试在独立子进程中执行不可逆网络限制"]
fn network_boundary_worker() {
    assert_eq!(
        std::env::var("ISP_NPM_PROBE_NETWORK_CHILD").as_deref(),
        Ok("1")
    );
    // 先打开 socket，证明限制不只阻止新建 socket；连接目标在本机，无网络数据发送。
    let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    assert!(socket >= 0);
    deny_network().unwrap();
    let mut address: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    address.sin_family = libc::AF_INET as _;
    address.sin_port = 9_u16.to_be();
    address.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
    let connected = unsafe {
        libc::connect(
            socket,
            (&address as *const libc::sockaddr_in).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    let error = io::Error::last_os_error();
    unsafe { libc::close(socket) };
    assert_eq!(connected, -1);
    assert!(matches!(
        error.raw_os_error(),
        Some(libc::EPERM | libc::EACCES)
    ));
}

#[test]
fn captured_executable_must_match_the_independently_verified_release_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().canonicalize().unwrap().join("program");
    fs::write(&path, b"official").unwrap();
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(b"official").into();
    ExpectedFileIdentity::capture_release_image(&path, 8, digest).unwrap();
    fs::write(&path, b"modified").unwrap();
    assert!(ExpectedFileIdentity::capture_release_image(&path, 8, digest).is_err());
}

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
))]
#[test]
fn grok_npm_probe_requires_a_complete_platform_specific_image_pair() {
    let (temporary, mut manifest) = fixture();
    let root = temporary.path().canonicalize().unwrap();
    manifest.atomic_launch_kind = Some(AtomicLaunchKind::GrokNpmVersionProbeV1);
    manifest.executable = root
        .join(format!(".infinishell-grok-npm-{}", Uuid::new_v4()))
        .join("bin/grok-native");
    fs::create_dir_all(manifest.executable.parent().unwrap()).unwrap();
    fs::write(&manifest.executable, b"manifest validation fixture").unwrap();
    manifest.expected_files = vec![ExpectedFileIdentity::capture(&manifest.executable).unwrap()];
    let old_mac = "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d";
    let current_mac = "e8daa302364c9c3b6a5546d511cfbd1ab5e5d407a9b04282f660665ea405f9f3";
    let linux = "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407";
    // 这里只核准入字段；实际原件读取和执行另由租约及真实事务验证。
    for (size, digest, accepted) in [
        (145_657_952, old_mac, cfg!(target_os = "macos")),
        (150_374_256, current_mac, cfg!(target_os = "macos")),
        (165_967_424, linux, cfg!(target_os = "linux")),
        (145_657_952, current_mac, false),
        (150_374_256, old_mac, false),
        (150_374_256, linux, false),
    ] {
        manifest.expected_files[0].size = size;
        manifest.expected_files[0].sha256 = digest.to_owned();
        assert_eq!(validate(&manifest, &root).is_ok(), accepted);
    }
}

#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
#[test]
fn private_brew_candidate_paths_use_shared_prefix_guard() {
    let (temporary, mut manifest) = fixture();
    let root = temporary.path().canonicalize().unwrap();
    let prefix = root.join("brew");
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(prefix.join("bin/brew"), b"manager fixture").unwrap();
    let stage = format!(".infinishell-brew-{}", Uuid::new_v4());
    for (kind, relative) in [
        (
            AtomicLaunchKind::CodexHomebrewVersionProbeV1,
            format!("Caskroom/{stage}/0.160.0/bin/codex"),
        ),
        (
            AtomicLaunchKind::GrokHomebrewVersionProbeV1,
            format!("Caskroom/{stage}/1.0.41/grok-1.0.41-macos-aarch64"),
        ),
        (
            AtomicLaunchKind::GrokHomebrewVersionProbeV1,
            format!("Caskroom/{stage}/1.0.46/grok-1.0.46-macos-aarch64"),
        ),
        (
            AtomicLaunchKind::ClaudeHomebrewVersionProbeV1,
            format!("Caskroom/{stage}/2.1.280/claude"),
        ),
        (
            AtomicLaunchKind::ClaudeHomebrewVersionProbeV1,
            format!("Caskroom/{stage}/2.1.285/claude"),
        ),
        (
            AtomicLaunchKind::ClaudeHomebrewVersionProbeV1,
            format!("Caskroom/{stage}/2.1.287/claude"),
        ),
    ] {
        manifest.atomic_launch_kind = Some(kind);
        manifest.executable = prefix.join(&relative);
        fs::create_dir_all(manifest.executable.parent().unwrap()).unwrap();
        fs::write(&manifest.executable, b"candidate fixture").unwrap();
        assert!(valid_entry(&manifest));
        let alias = root.join("linked-brew");
        symlink(&prefix, &alias).unwrap();
        manifest.executable = alias.join(&relative);
        assert!(!valid_entry(&manifest));
        fs::remove_file(alias).unwrap();
        manifest.executable = prefix.join(&relative);
        fs::set_permissions(&prefix, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(!valid_entry(&manifest));
        fs::set_permissions(&prefix, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(valid_entry(&manifest));
    }
    manifest.atomic_launch_kind = Some(AtomicLaunchKind::ClaudeHomebrewVersionProbeV1);
    for relative in [
        format!("Caskroom/{stage}/2.1.286/claude"),
        "Caskroom/claude-code/2.1.285/claude".to_owned(),
    ] {
        manifest.executable = prefix.join(relative);
        fs::create_dir_all(manifest.executable.parent().unwrap()).unwrap();
        fs::write(&manifest.executable, b"unapproved entry fixture").unwrap();
        assert!(!valid_entry(&manifest));
    }
}

#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
#[test]
fn private_codex_brew_probe_keeps_reviewed_version_and_caskroom_boundary() {
    let (temporary, mut manifest) = fixture();
    let prefix = temporary.path().canonicalize().unwrap().join("brew");
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(prefix.join("bin/brew"), b"manager fixture").unwrap();
    manifest.atomic_launch_kind = Some(AtomicLaunchKind::CodexHomebrewVersionProbeV1);
    for (relative, accepted) in [
        ("Caskroom/codex/0.155.1/bin/codex", true),
        ("Caskroom/codex/0.156.1/bin/codex", true),
        ("Caskroom/codex/0.160.0/bin/codex", true),
        ("Caskroom/codex/0.159.0/bin/codex", false),
        ("Caskroom/codex/0.161.0/bin/codex", false),
        ("Library/codex/0.160.0/bin/codex", false),
        ("Caskroom/other/0.160.0/bin/codex", false),
    ] {
        manifest.executable = prefix.join(relative);
        fs::create_dir_all(manifest.executable.parent().unwrap()).unwrap();
        fs::write(&manifest.executable, b"installed fixture").unwrap();
        assert_eq!(valid_entry(&manifest), accepted, "{relative}");
    }
}

#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
#[test]
fn private_grok_brew_probe_binds_reviewed_version_to_exact_entry() {
    let (temporary, mut manifest) = fixture();
    let prefix = temporary.path().canonicalize().unwrap().join("brew");
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(prefix.join("bin/brew"), b"manager fixture").unwrap();
    manifest.atomic_launch_kind = Some(AtomicLaunchKind::GrokHomebrewVersionProbeV1);
    for (relative, accepted) in [
        ("Caskroom/grok-build/1.0.40/grok-1.0.40-macos-aarch64", true),
        ("Caskroom/grok-build/1.0.41/grok-1.0.41-macos-aarch64", true),
        ("Caskroom/grok-build/1.0.46/grok-1.0.46-macos-aarch64", true),
        (
            "Caskroom/grok-build/1.0.45/grok-1.0.45-macos-aarch64",
            false,
        ),
        (
            "Caskroom/grok-build/1.0.46/grok-1.0.41-macos-aarch64",
            false,
        ),
        ("Caskroom/grok-build/1.0.46/grok-1.0.46-linux-x86_64", false),
        ("Caskroom/other/1.0.46/grok-1.0.46-macos-aarch64", false),
    ] {
        manifest.executable = prefix.join(relative);
        fs::create_dir_all(manifest.executable.parent().unwrap()).unwrap();
        fs::write(&manifest.executable, b"installed fixture").unwrap();
        assert_eq!(valid_entry(&manifest), accepted, "{relative}");
    }
}
