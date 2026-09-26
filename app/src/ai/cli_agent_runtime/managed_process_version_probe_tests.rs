use super::super::{
    AtomicDirectoryIdentity, ExpectedFileIdentity, ManagedEnvironment, PreparedLaunchBinding,
};
use super::*;

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
