use super::*;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;

use nix::fcntl::{FcntlArg, FdFlag, fcntl};

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

#[test]
fn serial_observation_cannot_replace_a_pinned_native_session() {
    let directory = directory();
    let first = Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap();
    let second = Uuid::parse_str("10000000-0000-4000-8000-000000000002").unwrap();
    {
        let guard = lock_directory(directory.path()).unwrap();
        pin_native_session(&guard, "manifest-a", first).unwrap();
    }
    let guard = lock_directory(directory.path()).unwrap();
    assert!(pin_native_session(&guard, "manifest-a", second).is_err());
    assert_eq!(
        bound_native_session(&guard, "manifest-a").unwrap(),
        Some(first)
    );
    pin_native_session(&guard, "manifest-a", first).unwrap();
}

#[test]
fn pinned_session_cannot_move_to_another_manifest() {
    let directory = directory();
    let guard = lock_directory(directory.path()).unwrap();
    let native = Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap();
    pin_native_session(&guard, "manifest-a", native).unwrap();
    assert!(bound_native_session(&guard, "manifest-b").is_err());
    assert!(pin_native_session(&guard, "manifest-b", native).is_err());
}

#[test]
fn legacy_manifest_without_tty_does_not_gain_readonly_evidence() {
    let directory = directory();
    let manifest = legacy_manifest(directory.path());
    let bytes = serde_json::to_vec(&manifest).unwrap();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap()
            .get("tty_path")
            .is_none()
    );
    assert!(
        serde_json::from_slice::<Manifest>(&bytes)
            .unwrap()
            .tty_path
            .is_none()
    );
    let guard = lock_directory(directory.path()).unwrap();
    assert_eq!(bound_native_session(&guard, "legacy").unwrap(), None);
}

#[test]
fn tty_evidence_rejects_an_ordinary_file_or_another_device() {
    let directory = directory();
    let path = directory.path().join("not-tty");
    fs::write(&path, b"fixture").unwrap();
    assert!(validate_tty_path(&path, 0).is_err());
    assert!(validate_tty_path(Path::new("/dev/null"), u64::MAX).is_err());
}

#[test]
fn tui_helper_inherits_all_three_terminal_descriptors() {
    const ROOT: &str = "INFINISHELL_CODEX_TUI_STDIO_FIXTURE";
    if let Some(root) = std::env::var_os(ROOT) {
        // 仅隔离夹具拥有 PTY；不修改并行测试进程的全局标准描述符。
        let root = PathBuf::from(root);
        let mut manifest = legacy_manifest(&root);
        manifest.app = root.join("check-tty.sh");
        let status = helper_command(&manifest, &root.join("manifest.json"), "fixture", "tui")
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
        assert!(status.success(), "TUI helper 丢失终端描述符：{status}");
        fs::write(root.join("verified"), b"stdin stdout stderr").unwrap();
        return;
    }
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let script = root.join("check-tty.sh");
    fs::write(&script, b"#!/bin/sh\n[ -t 0 ] && [ -t 1 ] && [ -t 2 ]\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let pair = nix::pty::openpty(None, None).unwrap();
    let (master, slave) = unsafe {
        (
            File::from_raw_fd(pair.master),
            File::from_raw_fd(pair.slave),
        )
    };
    for file in [&master, &slave] {
        fcntl(file.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).unwrap();
    }
    let mut fixture = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "remote_server::cli_image_codex_owned_launch::tests::tui_helper_inherits_all_three_terminal_descriptors",
            "--nocapture",
        ])
        .env(ROOT, &root)
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = fixture.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = fixture.kill();
            let _ = fixture.wait();
            panic!("TUI 标准描述符夹具超时");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "TUI 标准描述符夹具失败：{status}");
    assert_eq!(
        fs::read(root.join("verified")).unwrap(),
        b"stdin stdout stderr"
    );
}

struct NativeSocketFixture {
    directory: tempfile::TempDir,
    listener: UnixListener,
    requested: PathBuf,
    physical: PathBuf,
    renamed_physical: Option<PathBuf>,
    created_daemon_root: bool,
}

impl NativeSocketFixture {
    fn new() -> Self {
        let directory = directory();
        let root = directory.path().canonicalize().unwrap();
        let parent = root.join("long-codex-owned-alias-for-real-unix-socket-connection-regression-that-exceeds-both-macos-and-linux-sockaddr-un-path-limits");
        fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
        let requested = parent.join("control.sock");
        let daemon_root = fs::canonicalize("/tmp")
            .unwrap()
            .join(format!("codex-daemon-{}", unsafe { libc::geteuid() }));
        let created_daemon_root = match fs::DirBuilder::new().mode(0o700).create(&daemon_root) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("无法创建本轮 socket 目录：{error}"),
        };
        // 既有官方目录只核对，不修改其 owner 或权限。
        private_metadata(&daemon_root, true).unwrap();
        let physical = daemon_root.join(digest(requested.as_os_str().as_bytes()));
        let listener = UnixListener::bind(&physical).unwrap();
        fs::set_permissions(&physical, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&physical, &requested).unwrap();
        Self {
            directory,
            listener,
            requested,
            physical,
            renamed_physical: None,
            created_daemon_root,
        }
    }
}

impl Drop for NativeSocketFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.physical);
        if let Some(path) = &self.renamed_physical {
            let _ = fs::remove_file(path);
        }
        if self.created_daemon_root {
            let _ = fs::remove_dir(self.physical.parent().unwrap());
        }
    }
}

#[test]
fn long_socket_alias_connects_through_verified_physical_listener() {
    let fixture = NativeSocketFixture::new();
    assert!(fixture.requested.as_os_str().len() > 108);
    assert!(UnixStream::connect(&fixture.requested).is_err());
    let lease = SocketLease::capture(&fixture.requested).unwrap();

    let mut client = lease.connect().unwrap();
    let (mut server, _) = fixture.listener.accept().unwrap();
    client.write_all(b"physical").unwrap();
    let mut received = [0; 8];
    server.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"physical");
    lease.validate().unwrap();
}

#[test]
fn replaced_socket_alias_cannot_connect_to_original_listener() {
    let fixture = NativeSocketFixture::new();
    let lease = SocketLease::capture(&fixture.requested).unwrap();
    fs::rename(
        &fixture.requested,
        fixture.directory.path().join("retained-alias"),
    )
    .unwrap();
    symlink(&fixture.physical, &fixture.requested).unwrap();

    assert!(lease.connect().is_err());
    fixture.listener.set_nonblocking(true).unwrap();
    assert_eq!(
        fixture.listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn replaced_physical_socket_cannot_receive_a_connection() {
    let mut fixture = NativeSocketFixture::new();
    let lease = SocketLease::capture(&fixture.requested).unwrap();
    let retained = fixture.physical.with_extension("retained");
    fs::rename(&fixture.physical, &retained).unwrap();
    fixture.renamed_physical = Some(retained);
    let replacement = UnixListener::bind(&fixture.physical).unwrap();
    fs::set_permissions(&fixture.physical, fs::Permissions::from_mode(0o600)).unwrap();

    assert!(lease.connect().is_err());
    replacement.set_nonblocking(true).unwrap();
    assert_eq!(
        replacement.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn tui_arguments_use_the_pinned_physical_socket_and_reject_replacement() {
    let fixture = NativeSocketFixture::new();
    let root = fixture.requested.parent().unwrap();
    let manifest = legacy_manifest(root);
    assert!(arguments(&manifest, "tui").is_err());
    let lease = SocketLease::capture(&fixture.requested).unwrap();
    write_new(&root.join("socket.json"), &lease.stamp().unwrap()).unwrap();
    let server = arguments(&manifest, "server").unwrap();
    assert_eq!(server[2], format!("unix://{}", fixture.requested.display()));
    let tui = arguments(&manifest, "tui").unwrap();
    assert_eq!(tui[1], format!("unix://{}", fixture.physical.display()));
    // 按原生 TUI 的直接 connect 路径验收，不能由宿主连接函数替它解析别名。
    let _client = UnixStream::connect(tui[1].strip_prefix("unix://").unwrap()).unwrap();
    let _accepted = fixture.listener.accept().unwrap();
    fs::rename(&fixture.requested, root.join("retained-alias")).unwrap();
    symlink(&fixture.physical, &fixture.requested).unwrap();
    assert!(arguments(&manifest, "tui").is_err());
    let mut actual = vec![b"codex".to_vec()];
    actual.extend(tui.iter().map(|argument| argument.as_bytes().to_vec()));
    assert!(!arguments_match(&actual, &manifest, "tui"));
}

fn legacy_manifest(directory: &Path) -> Manifest {
    let token = Token::capture(std::process::id() as i32).unwrap();
    let reservation = Reservation {
        version: 1,
        id: Uuid::new_v4(),
        key_sha256: digest(Uuid::new_v4().as_bytes()),
        host: "fixture-host".into(),
        terminal_session: 7,
        cwd: directory.into(),
        app: directory.join("absent-app"),
        app_sha256: "fixture-app".into(),
        notifications: None,
    };
    write_new(&directory.join("ticket.json"), &reservation).unwrap();
    Manifest {
        version: 1,
        ticket: reservation.id,
        reservation_sha256: digest(&read_private(&directory.join("ticket.json")).unwrap()),
        executable: directory.join("absent-codex"),
        cwd: reservation.cwd,
        codex_home: directory.join("unchanged-home"),
        app: reservation.app,
        wrapper: token.clone(),
        shell: token,
        group: 7,
        tty_device: 11,
        tty_path: None,
        socket: directory.join("control.sock"),
        notifications: None,
    }
}

#[test]
fn normalized_notification_hash_matches_existing_native_discovery_receipts() {
    // 已归档原生 hooks/list 的旧命令金样；不是拿本实现生成本实现的期望值。
    let trusted: serde_json::Value = serde_json::from_str(include_str!(
        "../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST.json"
    ))
    .unwrap();
    for event in [
        NotificationEvent::SessionStart,
        NotificationEvent::UserPromptSubmit,
        NotificationEvent::PermissionRequest,
        NotificationEvent::PostToolUse,
        NotificationEvent::Stop,
    ] {
        let (_, label, script) = event.fields();
        let key = format!("warp@codex-warp:hooks/hooks.json:{label}:0:0");
        let receipt = trusted["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == key)
            .unwrap();
        let command = format!("bash \"$PLUGIN_ROOT/scripts/{script}\"");
        assert_eq!(receipt["command"], command);
        assert_eq!(
            notification_hash(event, &command).unwrap(),
            receipt["currentHash"].as_str().unwrap()
        );
    }
}

#[test]
fn native_cli_parser_preserves_five_notification_keys_and_exact_trust() {
    let directory = directory();
    let root = directory
        .path()
        .join("通知 ' $(touch INJECTED) `touch INJECTED`");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    let arguments = plan.arguments();
    assert_eq!(arguments.len(), 12);
    let mut merged = toml::Table::new();
    for pair in arguments.chunks_exact(2) {
        assert_eq!(pair[0], "-c");
        // 固定 0.156.1 只对右侧解析 TOML，左侧 path.split('.') 不识别引号。
        // 若把整条参数当 TOML 解析，旧版含点状态键的真实启动错误会被漏掉。
        let (path, value) = pair[1].split_once('=').unwrap();
        let segments = path.split('.').collect::<Vec<_>>();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], "hooks");
        let parsed: toml::Table = format!("value={value}").parse().unwrap();
        assert!(
            merged
                .insert(segments[1].into(), parsed["value"].clone())
                .is_none()
        );
    }
    assert_eq!(merged.len(), 6);
    let states = merged["state"].as_table().unwrap();
    assert_eq!(states.len(), 5);
    for hook in &plan.hooks {
        let state = states[&hook.key].as_table().unwrap();
        assert_eq!(state.len(), 2);
        assert_eq!(state["enabled"].as_bool(), Some(true));
        assert_eq!(
            state["trusted_hash"].as_str(),
            Some(hook.normalized_hash.as_str())
        );
        let handler = &merged[hook.event.fields().0].as_array().unwrap()[0]["hooks"][0];
        assert_eq!(handler["type"].as_str(), Some("command"));
        assert_eq!(handler["timeout"].as_integer(), Some(600));
        assert_eq!(handler["async"].as_bool(), Some(false));
        assert_eq!(handler.as_table().unwrap().len(), 4);
        let command = shlex::split(handler["command"].as_str().unwrap()).unwrap();
        assert_eq!(command.len(), 2);
        assert_eq!(command[0], "bash");
        assert_eq!(
            Path::new(&command[1]).parent(),
            Some(plan.root.join("scripts").as_path())
        );
    }
    assert_eq!(plan.files.len(), 8);
    assert!(!root.join(".codex").exists());
}

#[test]
fn changed_notification_bytes_are_rejected() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    fs::write(plan.root.join("scripts/on-session-start.sh"), b"exit 0\n").unwrap();
    assert!(plan.verify(&root).is_err());
}

#[test]
fn same_bytes_replaced_notification_inode_is_rejected() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    let path = plan.root.join("scripts/on-session-start.sh");
    let original = File::open(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert_ne!(
        original.metadata().unwrap().ino(),
        fs::metadata(&path).unwrap().ino()
    );
    assert!(plan.verify(&root).is_err());
}

#[test]
fn undeclared_notification_resource_is_rejected() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    fs::write(plan.root.join("scripts/additional.sh"), b"exit 0\n").unwrap();
    assert!(plan.verify(&root).is_err());
}

#[test]
fn serialized_notification_plan_cannot_add_an_arbitrary_command() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    let mut value = serde_json::to_value(&plan).unwrap();
    value["hooks"][0]["command"] = serde_json::Value::String("touch unreviewed".into());
    let changed: NotificationPlan = serde_json::from_value(value).unwrap();
    assert!(changed.verify(&root).is_err());
    let mut value = serde_json::to_value(plan).unwrap();
    value["extra_argv"] = serde_json::json!(["--dangerously-bypass-approvals-and-sandbox"]);
    assert!(serde_json::from_value::<NotificationPlan>(value).is_err());
}

#[test]
fn legacy_manifest_remains_queryable_but_reserved_ticket_cannot_start_again() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let manifest = legacy_manifest(&root);
    let encoded = serde_json::to_vec(&manifest).unwrap();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&encoded)
            .unwrap()
            .get("notifications")
            .is_none()
    );
    let restored = bound_manifest(&root, &encoded).unwrap();
    assert_eq!(
        arguments(&restored, "server").unwrap(),
        vec![
            "app-server".to_string(),
            "--listen".to_string(),
            format!("unix://{}", root.join("control.sock").display())
        ]
    );
    let ticket = root.join("ticket.json");
    assert!(supervise(&ticket, &digest(&read_private(&ticket).unwrap())).is_err());
    assert!(!root.join("entered.json").exists());
    assert!(!root.join("manifest.json").exists());
    assert!(!root.join("server-birth.json").exists());
}

#[test]
fn native_identity_requires_same_typed_flags_for_server_and_tui() {
    let fixture = NativeSocketFixture::new();
    let root = fixture.requested.parent().unwrap().to_path_buf();
    let lease = SocketLease::capture(&fixture.requested).unwrap();
    write_new(&root.join("socket.json"), &lease.stamp().unwrap()).unwrap();
    let mut manifest = legacy_manifest(&root);
    manifest.version = 2;
    manifest.notifications = Some(NotificationPlan::create(&root).unwrap());
    let flags = manifest.notifications.as_ref().unwrap().arguments();
    let helper = helper_command(
        &manifest,
        &root.join("manifest.json"),
        "fixture-hash",
        "server",
    );
    assert!(helper.get_envs().next().is_none());
    for role in ["server", "tui"] {
        let expected = arguments(&manifest, role).unwrap();
        assert_eq!(&expected[..flags.len()], &flags);
        let mut actual = vec![b"codex".to_vec()];
        actual.extend(expected.iter().map(|item| item.as_bytes().to_vec()));
        assert!(arguments_match(&actual, &manifest, role));
        actual.push(b"--dangerously-bypass-approvals-and-sandbox".to_vec());
        assert!(!arguments_match(&actual, &manifest, role));
        actual.pop();
        actual.remove(1);
        assert!(!arguments_match(&actual, &manifest, role));
    }
}

#[test]
fn new_manifest_must_match_the_reserved_notification_plan() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let mut manifest = legacy_manifest(&root);
    let mut reservation: Reservation = read_json(&root.join("ticket.json")).unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    reservation.version = 2;
    reservation.notifications = Some(plan.clone());
    fs::write(
        root.join("ticket.json"),
        serde_json::to_vec(&reservation).unwrap(),
    )
    .unwrap();
    manifest.version = 2;
    manifest.notifications = Some(plan);
    manifest.reservation_sha256 = digest(&read_private(&root.join("ticket.json")).unwrap());
    let encoded = serde_json::to_vec(&manifest).unwrap();
    assert!(bound_manifest(&root, &encoded).is_ok());
    manifest.notifications.as_mut().unwrap().hooks[0].normalized_hash = "sha256:changed".into();
    assert!(bound_manifest(&root, &serde_json::to_vec(&manifest).unwrap()).is_err());
    manifest.notifications = None;
    assert!(bound_manifest(&root, &serde_json::to_vec(&manifest).unwrap()).is_err());
}
