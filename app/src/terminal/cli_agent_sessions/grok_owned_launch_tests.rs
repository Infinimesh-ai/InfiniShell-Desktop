use super::*;
use std::os::unix::fs::symlink;

fn manifest(directory: &Path) -> LaunchManifest {
    LaunchManifest {
        version: 1,
        launch_id: Uuid::from_u128(1),
        session_id: Uuid::from_u128(2),
        history: None,
        executable: PathBuf::from("/private/tmp/grok"),
        app_executable: PathBuf::from("/private/tmp/InfiniShell.app/Contents/MacOS/infinishell"),
        cwd: PathBuf::from("/private/tmp/中文 项目"),
        socket_path: directory.join("leader.sock"),
        socket_directory: None,
        boot_session: Uuid::from_u128(3).to_string(),
        shell: ProcessIdentity {
            pid: 10,
            pid_version: 20,
            unique_id: 30,
            resource_cid: 40,
        },
        tty_device: 50,
    }
}

#[test]
fn native_argv_keeps_explicit_leader_and_native_approval_without_shell_syntax() {
    let manifest = manifest(Path::new("/private/tmp/owned"));
    assert_eq!(
        native_arguments(&manifest),
        Vec::<OsString>::from([
            "--leader".into(),
            "--minimal".into(),
            "--no-alt-screen".into(),
            "--cwd".into(),
            "/private/tmp/中文 项目".into(),
            "--leader-socket".into(),
            "/private/tmp/owned/leader.sock".into(),
            "--session-id".into(),
            "00000000-0000-0000-0000-000000000002".into(),
            "--model".into(),
            "grok-4.7".into(),
        ])
    );
}

#[test]
fn exec_transition_requires_same_process_lifetime_and_new_image_version() {
    let before = ProcessIdentity {
        pid: 10,
        pid_version: 20,
        unique_id: 30,
        resource_cid: 40,
    };
    let after = MacosProcessIdentity {
        pid: 10,
        pid_version: 22,
        unique_id: 30,
        resource_cid: 40,
    };
    assert!(exec_identity_matches(&before, after));
    assert!(!exec_identity_matches(
        &before,
        MacosProcessIdentity {
            pid_version: 20,
            ..after
        }
    ));
    assert!(!exec_identity_matches(
        &before,
        MacosProcessIdentity { pid: 11, ..after }
    ));
    assert!(!exec_identity_matches(
        &before,
        MacosProcessIdentity {
            unique_id: 31,
            ..after
        }
    ));
    assert!(!exec_identity_matches(
        &before,
        MacosProcessIdentity {
            resource_cid: 41,
            ..after
        }
    ));
}

#[test]
fn another_exec_cannot_be_used_as_process_exit_evidence() {
    let before = MacosProcessIdentity {
        pid: 10,
        pid_version: 20,
        unique_id: 30,
        resource_cid: 40,
    };
    assert!(same_process_lifetime(
        before,
        MacosProcessIdentity {
            pid_version: 24,
            ..before
        }
    ));
    assert!(!same_process_lifetime(
        before,
        MacosProcessIdentity {
            unique_id: 31,
            ..before
        }
    ));
}

#[test]
fn private_receipt_rejects_group_access_and_symlink_replacement() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = directory.path().canonicalize().unwrap();
    let path = root.join("exec.json");
    write_new(&path, b"{}").unwrap();
    assert_eq!(read_private(&path).unwrap(), b"{}");
    assert!(write_new(&path, b"replacement").is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(read_private(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(read_private(&path).is_err());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let replacement = root.join("other.json");
    fs::rename(&path, &replacement).unwrap();
    symlink(&replacement, &path).unwrap();
    assert!(read_private(&path).is_err());
}

#[test]
fn private_receipt_has_bounded_size() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.path().canonicalize().unwrap().join("exec.json");
    write_new(&path, &vec![b'a'; 65537]).unwrap();
    assert!(read_private(&path).is_err());
}

#[test]
fn unreserved_or_already_dispatched_launch_cannot_be_taken() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut launch = GrokOwnedLaunch {
        manifest: manifest(&root),
        directory: root.clone(),
        manifest_sha256: "digest".into(),
        reservation: None,
        phase: LaunchPhase::Prepared,
        processes: None,
    };
    assert!(launch.take_launch_argv().is_err());
    launch.phase = LaunchPhase::Dispatched;
    assert!(launch.take_launch_argv().is_err());
    assert!(!root.join("dispatched").exists());
}

#[test]
fn shell_pid_without_a_real_pty_cannot_create_owned_launch() {
    let file = tempfile::NamedTempFile::new().unwrap();
    assert!(GrokOwnedPty::capture(std::process::id() as i32, file.path()).is_err());
}

fn persisted_launch() -> (tempfile::TempDir, PathBuf, LaunchManifest, String) {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = directory.path().canonicalize().unwrap();
    let manifest = manifest(&root);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let checksum = digest(&bytes);
    write_new(&root.join("launch.json"), &bytes).unwrap();
    (directory, root, manifest, checksum)
}

#[test]
fn recovery_never_makes_an_unsent_or_dispatched_command_replayable() {
    let (_directory, root, _, checksum) = persisted_launch();
    let mut restored = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
    assert_eq!(restored.phase, LaunchPhase::RecoveredUnsent);
    assert!(restored.take_launch_argv().is_err());
    assert!(!root.join("dispatched").exists());
    write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
    let mut restored = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
    assert_eq!(restored.phase, LaunchPhase::Dispatched);
    assert!(restored.take_launch_argv().is_err());
    assert!(restored.reservation.is_none());
}

#[test]
fn recovery_rejects_changed_manifest_and_unknown_dispatch_receipt() {
    let (_directory, root, _, checksum) = persisted_launch();
    assert!(GrokOwnedLaunch::restore(&root.join("launch.json"), &"0".repeat(64)).is_err());
    write_new(&root.join("dispatched"), b"another-launch").unwrap();
    assert!(GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).is_err());
    assert_eq!(
        read_private(&root.join("dispatched")).unwrap(),
        b"another-launch"
    );
}

#[test]
fn absent_private_receipt_does_not_hide_a_dangling_symlink() {
    let (_directory, root, _, _) = persisted_launch();
    assert_eq!(
        read_optional_private(&root.join("bound.json")).unwrap(),
        None
    );
    symlink(root.join("missing"), root.join("bound.json")).unwrap();
    assert!(read_optional_private(&root.join("bound.json")).is_err());
}

fn process_receipts(manifest: &LaunchManifest, checksum: &str) -> (ExecReceipt, BoundProcesses) {
    let before = ProcessIdentity {
        pid: 100,
        pid_version: 200,
        unique_id: 300,
        resource_cid: 400,
    };
    let receipt = ExecReceipt {
        version: 1,
        launch_id: manifest.launch_id,
        manifest_sha256: checksum.to_owned(),
        process: before.clone(),
        process_group: 100,
        tty_device: manifest.tty_device,
    };
    let bound = BoundProcesses {
        version: 1,
        launch_id: manifest.launch_id,
        manifest_sha256: checksum.to_owned(),
        tui: ProcessIdentity {
            pid_version: 201,
            ..before
        },
        leader: ProcessIdentity {
            pid: 101,
            pid_version: 202,
            unique_id: 301,
            resource_cid: 400,
        },
    };
    (receipt, bound)
}

#[test]
fn recovery_preserves_owned_process_lifetimes_without_reauthenticating_permissions() {
    let (_directory, root, manifest, checksum) = persisted_launch();
    let (receipt, bound) = process_receipts(&manifest, &checksum);
    write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
    write_new(
        &root.join("exec.json"),
        &serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    write_new(
        &root.join("bound.json"),
        &serde_json::to_vec(&bound).unwrap(),
    )
    .unwrap();
    let mut restored = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
    let (tui, leader) = restored.processes.unwrap();
    assert_eq!(tui, kernel_identity(&bound.tui));
    assert_eq!(leader, kernel_identity(&bound.leader));
    assert!(restored.take_launch_argv().is_err());
    assert_eq!(restored.session_id(), manifest.session_id);
}

#[test]
fn recovery_rejects_bound_processes_without_their_exact_exec_transition() {
    for mutation in [
        "launch",
        "digest",
        "lifetime",
        "unchanged_image",
        "same_process",
        "undispatched",
    ] {
        let (_directory, root, manifest, checksum) = persisted_launch();
        let (receipt, mut bound) = process_receipts(&manifest, &checksum);
        match mutation {
            "launch" => bound.launch_id = Uuid::new_v4(),
            "digest" => bound.manifest_sha256 = "0".repeat(64),
            "lifetime" => bound.tui.unique_id += 1,
            "unchanged_image" => bound.tui.pid_version = receipt.process.pid_version,
            "same_process" => bound.leader.pid = bound.tui.pid,
            "undispatched" => {}
            other => panic!("未知夹具 {other}"),
        }
        if mutation != "undispatched" {
            write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
        }
        write_new(
            &root.join("exec.json"),
            &serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        write_new(
            &root.join("bound.json"),
            &serde_json::to_vec(&bound).unwrap(),
        )
        .unwrap();
        assert!(
            GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).is_err(),
            "{mutation}"
        );
        assert!(root.join("bound.json").exists());
    }
}

#[test]
fn cancelled_launch_keeps_a_durable_non_replayable_receipt() {
    warpui::App::test((), |mut app| async move {
        let (_directory, root, _, checksum) = persisted_launch();
        let mut launch = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
        app.update(|ctx| launch.cancel_before_dispatch(ctx).unwrap());
        assert!(root.join("launch.json").exists());
        assert!(root.join("retired.json").exists());
        let mut recovered = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
        assert!(recovered.is_retired());
        assert!(!recovered.was_dispatched());
        assert!(recovered.take_launch_argv().is_err());
        app.update(|ctx| recovered.cancel_before_dispatch(ctx).unwrap());
    });
}

#[test]
fn apparent_unsent_launch_with_exec_evidence_cannot_be_cancelled() {
    warpui::App::test((), |mut app| async move {
        for name in ["exec.json", "exec-failed.json", "leader.sock", "dispatched"] {
            let (_directory, root, _, checksum) = persisted_launch();
            let mut launch =
                GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
            write_new(&root.join(name), b"unexpected").unwrap();
            assert!(
                app.update(|ctx| launch.cancel_before_dispatch(ctx))
                    .is_err()
            );
            assert!(!root.join("retired.json").exists());
        }
    });
}

#[test]
fn retired_receipt_cannot_change_dispatch_or_manifest_identity() {
    for mutation in ["version", "launch", "sha", "dispatch"] {
        let (_directory, root, manifest, checksum) = persisted_launch();
        let mut receipt = RetiredLaunch {
            version: 1,
            launch_id: manifest.launch_id,
            manifest_sha256: checksum.clone(),
            dispatched: false,
        };
        match mutation {
            "version" => receipt.version = 2,
            "launch" => receipt.launch_id = Uuid::new_v4(),
            "sha" => receipt.manifest_sha256 = "0".repeat(64),
            "dispatch" => receipt.dispatched = true,
            other => panic!("未知夹具 {other}"),
        }
        write_new(
            &root.join("retired.json"),
            &serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(
            GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn retired_after_reboot_keeps_known_launch_history() {
    warpui::App::test((), |mut app| async move {
        let (_directory, root, _, checksum) = persisted_launch();
        write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
        let mut launch = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
        // 夹具的 boot ID 与当前机器不同，旧进程不可能跨系统启动继续存活。
        assert_ne!(launch.manifest.boot_session, macos_boot_session().unwrap());
        app.update(|ctx| launch.release_after_exit(ctx).unwrap());
        let mut recovered = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
        assert!(recovered.is_retired());
        assert!(recovered.take_launch_argv().is_err());
    });
}

#[test]
fn an_alive_process_never_produces_a_retired_receipt() {
    warpui::App::test((), |mut app| async move {
        let (_directory, root, _, checksum) = persisted_launch();
        write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
        let mut launch = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
        launch.manifest.boot_session = macos_boot_session().unwrap();
        let process = macos_process_identity(std::process::id() as i32).unwrap();
        launch.processes = Some((process, process));
        assert!(app.update(|ctx| launch.release_after_exit(ctx)).is_err());
        assert!(!root.join("retired.json").exists());
    });
}

fn durable_manifest(state: &Path) -> (tempfile::TempDir, tempfile::TempDir, LaunchManifest) {
    let (directory, socket) = create_launch_directories(state).unwrap();
    let root = directory.path().canonicalize().unwrap();
    let socket_root = socket.path().canonicalize().unwrap();
    let metadata = fs::symlink_metadata(&socket_root).unwrap();
    let mut value = manifest(&root);
    value.version = 2;
    value.socket_path = socket_root.join("leader.sock");
    value.socket_directory = Some(SocketDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    });
    (directory, socket, value)
}

#[test]
fn long_profile_keeps_manifest_persistent_and_native_socket_short() {
    let state = tempfile::tempdir().unwrap();
    let root = state
        .path()
        .canonicalize()
        .unwrap()
        .join("profile-".repeat(30));
    fs::create_dir(&root).unwrap();
    let (directory, socket, manifest) = durable_manifest(&root);
    assert!(
        directory
            .path()
            .starts_with(root.join("grok-owned-terminal"))
    );
    assert!(directory.path().as_os_str().len() > 104);
    assert!(manifest.socket_path.as_os_str().len() < 104);
    assert_eq!(socket.path().parent(), Some(Path::new("/private/tmp")));
    validate_manifest(&directory.path().join("launch.json"), &manifest).unwrap();
    validate_socket_directory(&manifest).unwrap();
}

#[test]
fn socket_removal_does_not_remove_recovery_history_or_replay_launch() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (directory, socket, manifest) = durable_manifest(&root);
    let path = directory.path().join("launch.json");
    let bytes = serde_json::to_vec(&manifest).unwrap();
    write_new(&path, &bytes).unwrap();
    drop(socket);
    let mut restored = GrokOwnedLaunch::restore(&path, &digest(&bytes)).unwrap();
    assert_eq!(restored.phase, LaunchPhase::RecoveredUnsent);
    assert!(restored.take_launch_argv().is_err());
    assert!(path.exists());
    assert!(!manifest.socket_path.parent().unwrap().exists());
}

#[test]
fn durable_recovery_parent_rejects_symlinks_and_shared_write_access() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let alias = root.join("alias");
    symlink(&root, &alias).unwrap();
    assert!(create_launch_directories(&alias).is_err());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o770)).unwrap();
    assert!(create_launch_directories(&root).is_err());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let private = root.join("grok-owned-terminal");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(create_launch_directories(&root).is_err());
}

#[test]
fn replacement_socket_directory_is_neither_connected_nor_removed() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let backup = root.join("original-socket-directory");
    fs::rename(socket.path(), &backup).unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(socket.path())
        .unwrap();
    let sentinel = socket.path().join("keep.txt");
    fs::write(&sentinel, b"keep replacement").unwrap();
    assert!(validate_socket_directory(&manifest).is_err());
    assert!(cleanup_socket_directory(&manifest, None).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"keep replacement");
}

#[test]
fn socket_cleanup_preserves_unknown_files_and_non_socket_replacements() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    fs::write(&manifest.socket_path, b"not a socket").unwrap();
    assert!(cleanup_socket_directory(&manifest, None).is_err());
    assert_eq!(fs::read(&manifest.socket_path).unwrap(), b"not a socket");
    fs::remove_file(&manifest.socket_path).unwrap();
    let sentinel = socket.path().join("keep.txt");
    fs::write(&sentinel, b"keep unknown").unwrap();
    assert!(cleanup_socket_directory(&manifest, None).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"keep unknown");
}

#[test]
fn durable_cancel_removes_socket_directory_but_keeps_retired_manifest() {
    warpui::App::test((), |mut app| async move {
        let state = tempfile::tempdir().unwrap();
        let root = state.path().canonicalize().unwrap();
        let (directory, socket, manifest) = durable_manifest(&root);
        let path = directory.path().join("launch.json");
        let bytes = serde_json::to_vec(&manifest).unwrap();
        write_new(&path, &bytes).unwrap();
        let mut launch = GrokOwnedLaunch::restore(&path, &digest(&bytes)).unwrap();
        app.update(|ctx| launch.cancel_before_dispatch(ctx).unwrap());
        assert!(!socket.path().exists());
        assert!(path.exists());
        let mut restored = GrokOwnedLaunch::restore(&path, &digest(&bytes)).unwrap();
        assert!(restored.is_retired());
        assert!(restored.take_launch_argv().is_err());
    });
}

#[test]
fn manifest_versions_cannot_mix_legacy_and_durable_socket_contracts() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (directory, _socket, mut value) = durable_manifest(&root);
    let path = directory.path().join("launch.json");
    value.version = 1;
    assert!(validate_manifest(&path, &value).is_err());
    value.version = 3;
    assert!(validate_manifest(&path, &value).is_err());
    value.version = 2;
    value.socket_directory = None;
    assert!(validate_manifest(&path, &value).is_err());
    let legacy = manifest(directory.path());
    validate_manifest(&path, &legacy).unwrap();
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("socket_directory")
            .is_none()
    );
}

#[test]
fn native_leader_lock_requires_bound_exited_lifetime_and_exact_bytes() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let lock = socket.path().join("leader.lock");
    write_new(&lock, b"2147483647").unwrap();
    assert!(cleanup_socket_directory(&manifest, None).is_err());
    let exited = MacosProcessIdentity {
        pid: i32::MAX,
        pid_version: 1,
        unique_id: 1,
        resource_cid: 1,
    };
    assert!(identity_exited(exited));
    let wrong = MacosProcessIdentity {
        pid: i32::MAX - 1,
        ..exited
    };
    assert!(cleanup_socket_directory(&manifest, Some(wrong)).is_err());
    assert_eq!(fs::read(&lock).unwrap(), b"2147483647");
    cleanup_socket_directory(&manifest, Some(exited)).unwrap();
    assert!(!socket.path().exists());
}

#[test]
fn live_leader_lock_is_preserved_even_when_pid_text_matches() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let live = macos_process_identity(std::process::id() as i32).unwrap();
    let lock = socket.path().join("leader.lock");
    write_new(&lock, live.pid.to_string().as_bytes()).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(live)).is_err());
    assert!(lock.exists());
}

#[test]
fn leader_lock_alias_and_extra_bytes_are_preserved() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let exited = MacosProcessIdentity {
        pid: i32::MAX,
        pid_version: 1,
        unique_id: 1,
        resource_cid: 1,
    };
    let lock = socket.path().join("leader.lock");
    write_new(&lock, b"2147483647\n").unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
    fs::remove_file(&lock).unwrap();
    let other = root.join("other.lock");
    write_new(&other, b"2147483647").unwrap();
    fs::hard_link(&other, &lock).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
    assert_eq!(fs::read(other).unwrap(), b"2147483647");
    assert!(lock.exists());
}

#[test]
fn lifecycle_capture_never_turns_a_restored_launch_into_input_authority() {
    let (_directory, root, manifest, checksum) = persisted_launch();
    let (receipt, bound) = process_receipts(&manifest, &checksum);
    write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
    write_new(
        &root.join("exec.json"),
        &serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let mut restored = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
    restored.record_owned_processes(&bound, &receipt).unwrap();
    restored.refresh_bound_processes().unwrap();
    assert!(restored.processes.is_some());
    // 已记录生命周期也不能借未知或非原生权限取得可发送输入的 binding。
    let observation = GrokPermissionObservation {
        session_id: manifest.session_id.to_string(),
        cwd: manifest.cwd.to_string_lossy().into_owned(),
        mode: "unknown".into(),
        session_start_event_id: String::new(),
    };
    assert!(matches!(
        restored.bind(Uuid::new_v4(), Uuid::new_v4(), &observation),
        Err(GrokLeaderInputError::InvalidTarget)
    ));
    assert!(restored.take_launch_argv().is_err());
}

#[test]
fn concurrent_same_owned_identity_is_idempotent_but_another_leader_is_rejected() {
    let (_directory, root, manifest, checksum) = persisted_launch();
    let (receipt, bound) = process_receipts(&manifest, &checksum);
    write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
    write_new(
        &root.join("exec.json"),
        &serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let launch = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
    launch.record_owned_processes(&bound, &receipt).unwrap();
    launch.record_owned_processes(&bound, &receipt).unwrap();
    let original = fs::read(root.join("bound.json")).unwrap();
    let mut different = bound;
    different.leader.unique_id += 1;
    assert!(launch.record_owned_processes(&different, &receipt).is_err());
    assert_eq!(fs::read(root.join("bound.json")).unwrap(), original);
}

#[test]
fn no_permission_event_and_no_process_receipt_cannot_create_a_cleanup_target() {
    let (_directory, root, _manifest, checksum) = persisted_launch();
    write_new(&root.join("dispatched"), checksum.as_bytes()).unwrap();
    let mut launch = GrokOwnedLaunch::restore(&root.join("launch.json"), &checksum).unwrap();
    // 旧系统启动清单不能在当前 boot 上捕获同号 PID，也不能留下新的进程记录。
    assert!(launch.capture_owned_processes().is_err());
    launch.stop_leader_after_tui_exit().unwrap();
    assert!(launch.processes.is_none());
    assert!(!root.join("bound.json").exists());
    assert!(!root.join("retired.json").exists());
}
