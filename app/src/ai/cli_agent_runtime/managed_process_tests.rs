use super::*;
use std::task::Context;

use futures::executor::block_on;
use futures::task::noop_waker_ref;

fn child_with_tcp_control(state: &Path) -> (ManagedChild, TcpStream) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let control = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    (
        ManagedChild {
            stdin: None,
            stdout: None,
            process: None,
            control: Some(control),
            state_dir: state.to_owned(),
            generation: Uuid::new_v4(),
        },
        peer,
    )
}

#[test]
fn normal_exit_wait_keeps_control_open_until_cancelled() {
    let state = tempfile::tempdir().unwrap();
    let (child, mut peer) = child_with_tcp_control(state.path());
    let generation = child.generation;
    // 只让真实等待逻辑停在未返回的状态，不伪造进程退出或内核清理回执。
    let mut waiting = Box::pin(child.wait_for_confirmed_exit(
        futures::future::pending(),
        CLEANUP_TIMEOUT + HANDSHAKE_TIMEOUT,
    ));
    let mut ctx = Context::from_waker(noop_waker_ref());
    assert!(waiting.as_mut().poll(&mut ctx).is_pending());

    peer.set_nonblocking(true).unwrap();
    let mut byte = [0];
    assert_eq!(
        peer.peek(&mut byte).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    peer.set_nonblocking(false).unwrap();
    drop(waiting);

    assert_eq!(peer.read(&mut byte).unwrap(), 0);
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
}

#[test]
fn normal_exit_wait_timeout_closes_control_without_claiming_cleanup() {
    let state = tempfile::tempdir().unwrap();
    let (child, mut peer) = child_with_tcp_control(state.path());
    let generation = child.generation;

    let error = block_on(child.wait_for_confirmed_exit(futures::future::pending(), Duration::ZERO))
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).unwrap(), 0);
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
}

#[test]
fn forced_finish_writes_stop_before_control_eof() {
    let state = tempfile::tempdir().unwrap();
    let (child, mut peer) = child_with_tcp_control(state.path());
    let generation = child.generation;

    let error = block_on(child.finish()).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    let mut byte = [0];
    peer.read_exact(&mut byte).unwrap();
    assert_eq!(byte, [2]);
    assert_eq!(peer.read(&mut byte).unwrap(), 0);
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
}

#[test]
fn normal_finish_without_a_process_never_writes_stop_or_claims_exit() {
    let state = tempfile::tempdir().unwrap();
    let (child, mut peer) = child_with_tcp_control(state.path());
    let generation = child.generation;

    let error = block_on(child.finish_after_stdin_close()).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).unwrap(), 0);
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
}

fn fixture(state: &Path, generation: Uuid) -> (PathBuf, ExitReceipt) {
    let directory = create_generation_directory(state, generation).unwrap();
    let manifest = Manifest {
        isolated_home: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: std::env::current_exe().unwrap(),
        arguments: Vec::new(),
        cwd: state.to_owned(),
    };
    let bytes = serde_json::to_vec(&manifest).unwrap();
    write_new_record(&directory.join("manifest.json"), &bytes).unwrap();
    let receipt = ExitReceipt {
        version: 1,
        generation,
        cleanup_confirmed: true,
        containment: "linux_subtree".to_owned(),
        exit_reason: ExitReason::HostDisconnected,
        exit_code: None,
        manifest_sha256: sha256(&bytes),
    };
    (directory, receipt)
}

#[test]
fn not_started_claim_blocks_late_spawn_and_never_replaces_a_launch() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let executable = std::env::current_exe().unwrap();
    let receipt =
        record_not_started(state.path(), generation, &executable, &[], state.path()).unwrap();
    assert_eq!(receipt.containment, "not_started");
    assert_eq!(
        confirmed_exit(state.path(), generation).unwrap(),
        Some(receipt)
    );
    assert!(create_generation_directory(state.path(), generation).is_err());
    assert!(
        run_worker(
            &generation_directory(state.path(), generation).join("manifest.json"),
            false
        )
        .is_err()
    );
    let started_generation = Uuid::new_v4();
    let (directory, _) = fixture(state.path(), started_generation);
    let before = fs::read(directory.join("manifest.json")).unwrap();
    assert!(
        record_not_started(
            state.path(),
            started_generation,
            &executable,
            &[],
            state.path()
        )
        .is_err()
    );
    assert_eq!(fs::read(directory.join("manifest.json")).unwrap(), before);
    assert!(
        confirmed_exit(state.path(), started_generation)
            .unwrap()
            .is_none()
    );
}

#[test]
fn missing_or_interrupted_receipt_never_confirms_exit() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
    let (directory, receipt) = fixture(state.path(), generation);
    write_new_record(
        &directory.join("partial.tmp"),
        &serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
    write_receipt(&directory, &receipt).unwrap();
    assert_eq!(
        confirmed_exit(state.path(), generation).unwrap(),
        Some(receipt)
    );
}

#[test]
fn generation_is_claimed_once_even_after_confirmed_exit() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, receipt) = fixture(state.path(), generation);
    assert!(create_generation_directory(state.path(), generation).is_err());
    write_receipt(&directory, &receipt).unwrap();
    assert!(create_generation_directory(state.path(), generation).is_err());
    assert!(write_receipt(&directory, &receipt).is_err());
    assert!(create_generation_directory(state.path(), Uuid::new_v4()).is_ok());
}

#[test]
fn old_generation_and_changed_manifest_cannot_unlock_resume() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, mut receipt) = fixture(state.path(), generation);
    receipt.generation = Uuid::new_v4();
    write_receipt(&directory, &receipt).unwrap();
    assert!(confirmed_exit(state.path(), generation).is_err());
    fs::remove_file(directory.join("exit.json")).unwrap();
    receipt.generation = generation;
    receipt.manifest_sha256 = "changed".to_owned();
    write_receipt(&directory, &receipt).unwrap();
    assert!(confirmed_exit(state.path(), generation).is_err());
}

#[test]
fn pid_and_lease_files_are_not_exit_proofs() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, mut receipt) = fixture(state.path(), generation);
    write_new_record(
        &directory.join("pid"),
        std::process::id().to_string().as_bytes(),
    )
    .unwrap();
    write_new_record(&directory.join("lease"), b"unlocked").unwrap();
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
    receipt.cleanup_confirmed = false;
    write_receipt(&directory, &receipt).unwrap();
    assert!(confirmed_exit(state.path(), generation).is_err());
}

#[test]
fn legacy_process_group_receipt_does_not_hide_native_crash_descendants() {
    let state = tempfile::tempdir().unwrap();
    for exit_code in [None, Some(37)] {
        let generation = Uuid::new_v4();
        let (directory, mut receipt) = fixture(state.path(), generation);
        receipt.containment = "unix_process_group".to_owned();
        receipt.exit_code = exit_code;
        // 即使旧 worker 错误写出 true，也不能据此启动同一原生会话的新进程。
        assert!(receipt.cleanup_confirmed);
        write_receipt(&directory, &receipt).unwrap();
        assert!(confirmed_exit(state.path(), generation).is_err());
    }
    for containment in ["linux_subtree", "windows_job"] {
        let generation = Uuid::new_v4();
        let (directory, mut receipt) = fixture(state.path(), generation);
        receipt.containment = containment.to_owned();
        receipt.exit_code = Some(37);
        write_receipt(&directory, &receipt).unwrap();
        assert_eq!(
            confirmed_exit(state.path(), generation).unwrap(),
            Some(receipt)
        );
    }
}

#[test]
fn sqlite_scopes_do_not_share_exit_receipts() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, receipt) = fixture(first.path(), generation);
    write_receipt(&directory, &receipt).unwrap();
    assert!(confirmed_exit(first.path(), generation).unwrap().is_some());
    assert!(confirmed_exit(second.path(), generation).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn linked_or_publicly_readable_receipts_are_rejected() {
    use std::os::unix::fs::PermissionsExt as _;
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, receipt) = fixture(state.path(), generation);
    write_receipt(&directory, &receipt).unwrap();
    let path = directory.join("exit.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(confirmed_exit(state.path(), generation).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&path, directory.join("outside.json")).unwrap();
    std::os::unix::fs::symlink(directory.join("outside.json"), &path).unwrap();
    assert!(confirmed_exit(state.path(), generation).is_err());
}

#[test]
fn only_one_concurrent_not_started_claim_can_close_a_generation() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            let state = state.path().to_owned();
            thread::spawn(move || {
                barrier.wait();
                record_not_started(
                    &state,
                    generation,
                    &std::env::current_exe().unwrap(),
                    &[],
                    &state,
                )
                .is_ok()
            })
        })
        .collect();
    let succeeded = handles
        .into_iter()
        .map(|handle| usize::from(handle.join().unwrap()))
        .sum::<usize>();
    assert_eq!(succeeded, 1);
    assert_eq!(
        confirmed_exit(state.path(), generation)
            .unwrap()
            .unwrap()
            .containment,
        "not_started"
    );
}

#[test]
fn control_channel_rejects_a_wrong_generation_or_token_before_authorization() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _) = fixture(state.path(), generation);
    let (manifest, _) = read_manifest(&directory.join("manifest.json")).unwrap();
    for wrong_generation in [false, true] {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let expected = manifest.clone();
        let server = thread::spawn(move || accept_authorized(&listener, &expected).is_err());
        let mut client = TcpStream::connect(address).unwrap();
        configure_stream(&client).unwrap();
        let nonce = Uuid::new_v4();
        client
            .write_all(if wrong_generation {
                nonce.as_bytes()
            } else {
                manifest.generation.as_bytes()
            })
            .unwrap();
        client
            .write_all(if wrong_generation {
                manifest.token.as_bytes()
            } else {
                nonce.as_bytes()
            })
            .unwrap();
        let mut response = [0];
        assert!(client.read_exact(&mut response).is_err());
        assert!(server.join().unwrap());
    }
}

#[test]
fn accepted_control_stream_waits_for_delayed_worker_readiness() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _) = fixture(state.path(), generation);
    let (manifest, _) = read_manifest(&directory.join("manifest.json")).unwrap();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let expected = manifest.clone();
    let server = thread::spawn(move || {
        let mut stream = accept_authorized(&listener, &expected).unwrap();
        let mut ready = [0];
        stream.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [1]);
    });
    let mut client = connect_authorized(address, &manifest).unwrap();
    // 真实 worker 在建立组/Job 后才报告 ready，不能把这段等待误当作 WouldBlock 失败。
    thread::sleep(Duration::from_millis(100));
    client.write_all(&[1]).unwrap();
    server.join().unwrap();
}

#[test]
fn isolated_environment_keeps_managed_paths_and_excludes_cli_configuration_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let environment: std::collections::HashMap<_, _> =
        isolated_environment(directory.path()).into_iter().collect();
    assert_eq!(
        environment.get(&OsString::from("GROK_HOME")),
        Some(&directory.path().join("grok").into_os_string())
    );
    assert_eq!(
        environment.get(&OsString::from("HOME")),
        Some(&directory.path().join("home").into_os_string())
    );
    assert!(!environment.contains_key(&OsString::from(EXEC_CONTROL_ENV)));
    assert!(!environment.contains_key(&OsString::from("DYLD_INSERT_LIBRARIES")));
    assert!(!environment.contains_key(&OsString::from("LD_PRELOAD")));
    assert!(!environment.contains_key(&OsString::from("GROK_API_KEY")));
}

#[test]
fn confirmed_receipt_removes_isolated_auth_but_preserves_session_storage() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, mut receipt) = fixture(state.path(), generation);
    let home = state
        .path()
        .canonicalize()
        .unwrap()
        .join("grok-managed")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(home.join("grok")).unwrap();
    fs::write(home.join("grok/auth.json"), b"offline opaque test cache").unwrap();
    fs::write(home.join("grok/session-test"), b"history").unwrap();
    let path = directory.join("manifest.json");
    let (mut manifest, _) = read_manifest(&path).unwrap();
    manifest.isolated_home = Some(home.clone());
    let bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(path, &bytes).unwrap();
    receipt.manifest_sha256 = sha256(&bytes);
    // 此处只测试文件生命周期，不表示真实进程或平台 containment 已通过。
    write_receipt(&directory, &receipt).unwrap();
    assert!(!home.join("grok/auth.json").exists());
    assert_eq!(
        fs::read(home.join("grok/session-test")).unwrap(),
        b"history"
    );
}

#[test]
fn isolated_receipt_accepts_real_state_path_aliases_without_changing_ownership() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, mut receipt) = fixture(state.path(), generation);
    let home = state
        .path()
        .canonicalize()
        .unwrap()
        .join("grok-managed")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(home.join("grok")).unwrap();
    let manifest_path = directory.join("manifest.json");
    let (mut manifest, _) = read_manifest(&manifest_path).unwrap();
    manifest.isolated_home = Some(home);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &bytes).unwrap();
    receipt.manifest_sha256 = sha256(&bytes);
    write_receipt(&directory, &receipt).unwrap();

    // 使用真实存在的目录别名；Windows 普通路径也会与 canonicalize 的扩展前缀不同。
    let child = state.path().join("path-alias");
    fs::create_dir(&child).unwrap();
    let alias = child.join("..");
    assert_eq!(
        confirmed_exit(&alias, generation).unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        confirmed_exit(state.path(), generation).unwrap(),
        Some(receipt.clone())
    );
    #[cfg(unix)]
    {
        let parent = tempfile::tempdir().unwrap();
        let link = parent.path().join("state-link");
        std::os::unix::fs::symlink(state.path(), &link).unwrap();
        assert_eq!(confirmed_exit(&link, generation).unwrap(), Some(receipt));
    }
}

#[test]
fn update_environment_rejects_credentials_permission_flags_and_conflicting_keys() {
    for name in [
        "ANTHROPIC_API_KEY",
        "GROK_API_KEY",
        "LD_PRELOAD",
        "DISABLE_UPDATES",
        "CLAUDE_CONFIG_DIR",
    ] {
        let environment = ManagedEnvironment {
            values: vec![(name.into(), "synthetic".into())],
            remove: vec![],
        };
        assert!(environment.validate().is_err(), "{name}");
    }
    let mut environment = ManagedEnvironment {
        values: vec![("CODEX_RELEASE".into(), "0.155.1".into())],
        remove: vec!["CODEX_MANAGED_BY_NPM".into()],
    };
    assert!(environment.validate().is_ok());
    environment
        .values
        .push(("CODEX_RELEASE".into(), "0.156.0-alpha.7".into()));
    assert!(environment.validate().is_err());
    environment.values.pop();
    environment.remove.push("PATH".into());
    assert!(environment.validate().is_err());
}

#[test]
fn claude_update_configuration_is_bound_to_generation_and_exact_update_command() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let directory = state
        .path()
        .canonicalize()
        .unwrap()
        .join(format!("claude-update-{generation}"));
    let environment = ManagedEnvironment {
        values: vec![(
            "CLAUDE_CONFIG_DIR".into(),
            directory.join("config").into_os_string(),
        )],
        remove: vec![],
    };
    let arguments = [
        "--settings",
        "{\"autoUpdatesChannel\":\"stable\"}",
        "update",
    ]
    .map(OsString::from);
    assert!(
        environment
            .validate_for_generation(state.path(), generation, &arguments)
            .is_ok()
    );
    assert!(
        environment
            .validate_for_generation(state.path(), Uuid::new_v4(), &arguments)
            .is_err()
    );
    assert!(
        environment
            .validate_for_generation(state.path(), generation, &["--print".into()])
            .is_err()
    );
    let changed = [
        "--settings",
        "{\"skipDangerousModePermissionPrompt\":true}",
        "update",
    ]
    .map(OsString::from);
    assert!(
        environment
            .validate_for_generation(state.path(), generation, &changed)
            .is_err()
    );
    let outside = ManagedEnvironment {
        values: vec![(
            "CLAUDE_CONFIG_DIR".into(),
            state.path().join("user").into_os_string(),
        )],
        remove: vec![],
    };
    assert!(
        outside
            .validate_for_generation(state.path(), generation, &arguments)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn claude_update_configuration_rejects_a_redirected_generation_directory() {
    let state = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let directory = state
        .path()
        .canonicalize()
        .unwrap()
        .join(format!("claude-update-{generation}"));
    std::os::unix::fs::symlink(other.path(), &directory).unwrap();
    let environment = ManagedEnvironment {
        values: vec![(
            "CLAUDE_CONFIG_DIR".into(),
            directory.join("config").into_os_string(),
        )],
        remove: vec![],
    };
    let arguments = [
        "--settings",
        "{\"autoUpdatesChannel\":\"latest\"}",
        "update",
    ]
    .map(OsString::from);
    assert!(
        environment
            .validate_for_generation(state.path(), generation, &arguments)
            .is_err()
    );
}

#[test]
fn oversized_launch_record_never_claims_a_generation() {
    let state = tempfile::tempdir().unwrap();
    let existing_generation = Uuid::new_v4();
    let (directory, _) = fixture(state.path(), existing_generation);
    let (mut manifest, before) = read_manifest(&directory.join("manifest.json")).unwrap();
    manifest.generation = Uuid::new_v4();
    manifest.arguments = vec!["x".repeat(MAX_RECORD_BYTES as usize).into()];
    assert!(create_launch_manifest(state.path(), &manifest).is_err());
    assert!(!generation_directory(state.path(), manifest.generation).exists());
    manifest.generation = existing_generation;
    manifest.arguments.clear();
    assert!(create_launch_manifest(state.path(), &manifest).is_err());
    assert_eq!(fs::read(directory.join("manifest.json")).unwrap(), before);
}
