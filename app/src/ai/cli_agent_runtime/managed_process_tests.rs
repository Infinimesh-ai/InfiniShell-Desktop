use super::*;

fn fixture(state: &Path, generation: Uuid) -> (PathBuf, ExitReceipt) {
    let directory = create_generation_directory(state, generation).unwrap();
    let manifest = Manifest {
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
