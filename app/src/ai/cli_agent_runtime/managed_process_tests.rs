use super::*;
use std::task::Context;

use futures::executor::block_on;
use futures::task::noop_waker_ref;

#[test]
fn stdout_bridge_flushes_ack_before_the_long_lived_source_reaches_eof() {
    struct BlockingReader {
        chunks: std::sync::mpsc::Receiver<Option<Vec<u8>>>,
        pending: io::Cursor<Vec<u8>>,
    }

    impl io::Read for BlockingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            loop {
                let read = io::Read::read(&mut self.pending, buffer)?;
                if read != 0 {
                    return Ok(read);
                }
                match self.chunks.recv().unwrap() {
                    Some(chunk) => self.pending = io::Cursor::new(chunk),
                    None => return Ok(0),
                }
            }
        }
    }

    struct ObservedWriter {
        bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        flushed: std::sync::mpsc::Sender<()>,
    }

    impl io::Write for ObservedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            let _ = self.flushed.send(());
            Ok(())
        }
    }

    let (chunks, source) = std::sync::mpsc::channel();
    let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (flushed, observed_flush) = std::sync::mpsc::channel();
    let writer_bytes = bytes.clone();
    let bridge = thread::spawn(move || {
        copy_output_with_flush(
            &mut BlockingReader {
                chunks: source,
                pending: io::Cursor::new(Vec::new()),
            },
            &mut ObservedWriter {
                bytes: writer_bytes,
                flushed,
            },
        )
    });

    chunks
        .send(Some(b"INFINISHELL_FAKE_ACK:test\n".to_vec()))
        .unwrap();
    observed_flush.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        bytes.lock().unwrap().as_slice(),
        b"INFINISHELL_FAKE_ACK:test\n"
    );
    assert!(!bridge.is_finished(), "ACK 可见时原生进程仍应保持运行");
    chunks.send(None).unwrap();
    assert_eq!(
        bridge.join().unwrap().unwrap(),
        b"INFINISHELL_FAKE_ACK:test\n".len() as u64
    );
}

#[test]
fn expected_file_identity_rejects_content_or_path_replacement() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    fs::write(&program, b"trusted").unwrap();
    let expected = ExpectedFileIdentity::capture(&program).unwrap();
    validate_expected_files_contract(&program, std::slice::from_ref(&expected)).unwrap();
    verify_expected_files(std::slice::from_ref(&expected)).unwrap();

    fs::write(&program, b"changed").unwrap();
    assert!(verify_expected_files(std::slice::from_ref(&expected)).is_err());

    let helper = state.path().join("helper");
    fs::write(&helper, b"trusted").unwrap();
    let helper = ExpectedFileIdentity::capture(&helper).unwrap();
    assert!(validate_expected_files_contract(&program, &[helper]).is_err());
}

#[cfg(any(unix, windows))]
#[test]
fn expected_file_identity_rejects_same_content_file_replacement() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    let previous = state.path().join("previous");
    fs::write(&program, b"trusted").unwrap();
    let expected = ExpectedFileIdentity::capture(&program).unwrap();

    fs::rename(&program, previous).unwrap();
    fs::write(&program, b"trusted").unwrap();

    assert!(verify_expected_files(&[expected]).is_err());
}

#[cfg(unix)]
#[test]
fn expected_file_identity_metadata_and_digest_use_the_same_open_file() {
    use std::os::unix::fs::MetadataExt as _;

    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    let moved = state.path().join("moved");
    let original = b"trusted-open-object";
    fs::write(&program, original).unwrap();
    let canonical_path = program.canonicalize().unwrap();
    let mut file = open_expected_file(&canonical_path).unwrap();

    fs::rename(&program, &moved).unwrap();
    fs::write(&program, b"replacement-at-original-path").unwrap();

    let identity =
        ExpectedFileIdentity::capture_opened(&program, canonical_path, &mut file).unwrap();
    let opened_metadata = file.metadata().unwrap();
    assert_eq!(identity.sha256, sha256(original));
    assert_eq!(identity.size, original.len() as u64);
    assert_eq!(
        identity.file_id,
        Some(ExpectedFileId {
            volume: opened_metadata.dev(),
            index: opened_metadata.ino(),
        })
    );

    let replacement = ExpectedFileIdentity::capture(&program).unwrap();
    assert_ne!(identity.sha256, replacement.sha256);
    assert_ne!(identity.file_id, replacement.file_id);
}

#[test]
fn expected_file_identity_rejects_non_regular_files() {
    let state = tempfile::tempdir().unwrap();
    assert!(ExpectedFileIdentity::capture(state.path()).is_err());
}

#[cfg(unix)]
#[test]
fn expected_file_identity_preserves_entry_symlink_boundary() {
    let state = tempfile::tempdir().unwrap();
    let target = state.path().join("target");
    let entry = state.path().join("entry");
    fs::write(&target, b"trusted").unwrap();
    std::os::unix::fs::symlink(&target, &entry).unwrap();

    let identity = ExpectedFileIdentity::capture(&entry).unwrap();
    assert_eq!(identity.path, entry);
    assert_eq!(identity.canonical_path, target.canonicalize().unwrap());
    assert!(identity.file_id.is_some());
    assert!(open_expected_file(&entry).is_err());
}

#[test]
fn windows_reparse_attribute_is_not_a_plain_expected_file() {
    assert!(windows_file_attributes_are_plain(0));
    assert!(windows_file_attributes_are_plain(0x20));
    assert!(!windows_file_attributes_are_plain(0x400));
    assert!(!windows_file_attributes_are_plain(0x420));
}

#[test]
fn expected_file_identity_without_file_id_stays_compatible() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    fs::write(&program, b"trusted").unwrap();
    let identity = ExpectedFileIdentity::capture(&program).unwrap();
    let mut value = serde_json::to_value(identity).unwrap();
    value.as_object_mut().unwrap().remove("file_id");

    let decoded: ExpectedFileIdentity = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.file_id, None);
    verify_expected_files(&[decoded]).unwrap();
}

#[test]
fn bound_update_rejects_pathname_execution_before_claiming_generation() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let executable = std::env::current_exe().unwrap();
    let expected = ExpectedFileIdentity::capture(&executable).unwrap();
    let binding = PreparedLaunchBinding::new("a".repeat(64)).unwrap();

    let error = match block_on(spawn_bound_update(
        state.path(),
        generation,
        &executable,
        &[],
        state.path(),
        ManagedEnvironment::default(),
        vec![expected],
        &binding,
    )) {
        Ok(_) => panic!("缺少原子执行原语时不能派生更新进程"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(!generation_directory(state.path(), generation).exists());
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn native_file_binding_is_distinct_from_digest_only_recovery_binding() {
    let digest_only = PreparedLaunchBinding::new("a".repeat(64)).unwrap();
    let native = PreparedLaunchBinding::native_file("a".repeat(64)).unwrap();

    assert!(digest_only.validate_update_execution().is_err());
    assert!(native.validate_update_execution().is_ok());
    assert_ne!(native, digest_only);
}

#[test]
fn bound_recovery_rejects_changed_atomic_execution_kind() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let executable = std::env::current_exe().unwrap();
    let binding = PreparedLaunchBinding::native_file("a".repeat(64)).unwrap();
    record_not_started_with_binding(
        state.path(),
        generation,
        &executable,
        &[],
        state.path(),
        &binding,
    )
    .unwrap();
    let path = generation_directory(state.path(), generation).join(LAUNCH_BINDING_RECORD);
    let mut record: LaunchBindingRecord =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.kind = None;
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();

    assert!(confirmed_exit_with_binding(state.path(), generation, &binding).is_err());
}

#[test]
fn bound_receipt_requires_the_same_manifest_and_journal_digest() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let executable = std::env::current_exe().unwrap();
    let binding = PreparedLaunchBinding::new("a".repeat(64)).unwrap();
    let wrong = PreparedLaunchBinding::new("b".repeat(64)).unwrap();
    let receipt = record_not_started_with_binding(
        state.path(),
        generation,
        &executable,
        &[],
        state.path(),
        &binding,
    )
    .unwrap();

    assert_eq!(
        confirmed_exit_with_binding(state.path(), generation, &binding).unwrap(),
        Some(receipt)
    );
    assert!(confirmed_exit_with_binding(state.path(), generation, &wrong).is_err());
}

#[test]
fn changed_exit_binding_digest_never_confirms_update_recovery() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let executable = std::env::current_exe().unwrap();
    let binding = PreparedLaunchBinding::new("a".repeat(64)).unwrap();
    record_not_started_with_binding(
        state.path(),
        generation,
        &executable,
        &[],
        state.path(),
        &binding,
    )
    .unwrap();
    let path = generation_directory(state.path(), generation).join(EXIT_BINDING_RECORD);
    let mut record: ExitBindingRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.binding_digest = "b".repeat(64);
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();

    assert!(confirmed_exit_with_binding(state.path(), generation, &binding).is_err());
}

#[test]
fn legacy_manifest_without_expected_files_stays_compatible() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let manifest = Manifest {
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: std::env::current_exe().unwrap(),
        arguments: Vec::new(),
        cwd: state.path().to_owned(),
        isolated_home: None,
        isolated_state_dir: None,
        environment: None,
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let mut value = serde_json::to_value(manifest).unwrap();
    value.as_object_mut().unwrap().remove("expected_files");
    value.as_object_mut().unwrap().remove("atomic_launch_kind");
    value.as_object_mut().unwrap().remove("atomic_cwd");
    let decoded: Manifest = serde_json::from_value(value).unwrap();
    assert!(decoded.expected_files.is_empty());
    assert_eq!(decoded.atomic_launch_kind, None);
    assert_eq!(decoded.atomic_cwd, None);
}

#[test]
fn atomic_manifest_never_downgrades_when_binding_record_is_missing() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let cwd = AtomicDirectoryIdentity::capture(state.path()).unwrap();
    let manifest = Manifest {
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: std::env::current_exe().unwrap(),
        arguments: Vec::new(),
        cwd: state.path().to_owned(),
        isolated_home: None,
        isolated_state_dir: None,
        environment: None,
        expected_files: Vec::new(),
        atomic_launch_kind: Some(AtomicLaunchKind::NativeFile),
        atomic_cwd: Some(cwd),
    };
    let (directory, bytes) = create_launch_manifest(state.path(), &manifest).unwrap();

    assert_eq!(
        read_worker_launch_binding(&directory, &manifest, &bytes)
            .unwrap_err()
            .to_string(),
        "managed_process.required_launch_binding_missing"
    );
}

#[test]
#[cfg(unix)]
fn atomic_cwd_identity_rejects_path_replacement() {
    let state = tempfile::tempdir().unwrap();
    let cwd = state.path().join("cwd");
    let previous = state.path().join("previous");
    fs::create_dir(&cwd).unwrap();
    let identity = AtomicDirectoryIdentity::capture(&cwd).unwrap();
    fs::rename(&cwd, &previous).unwrap();
    fs::create_dir(&cwd).unwrap();

    assert_eq!(
        identity.open_verified().unwrap_err().to_string(),
        "managed_process.atomic_cwd_changed"
    );
}

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
        isolated_state_dir: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: std::env::current_exe().unwrap(),
        arguments: Vec::new(),
        cwd: state.to_owned(),
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
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

fn attempted_fixture(state: &Path, generation: Uuid) -> (PathBuf, Manifest, Vec<u8>, SpawnAttempt) {
    let manifest = Manifest {
        isolated_home: None,
        isolated_state_dir: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: std::env::current_exe().unwrap(),
        arguments: Vec::new(),
        cwd: state.to_owned(),
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let (directory, bytes) = create_launch_manifest(state, &manifest).unwrap();
    let attempt = write_spawn_attempt(&directory, generation, &bytes).unwrap();
    (directory, manifest, bytes, attempt)
}

#[test]
fn persisted_spawn_attempt_without_outcome_stays_unconfirmed() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, manifest, bytes, attempt) = attempted_fixture(state.path(), generation);

    assert_eq!(
        read_bound_spawn_attempt(&directory, &manifest, &bytes).unwrap(),
        attempt
    );
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
    assert_eq!(
        process_completion(state.path(), generation).unwrap(),
        ProcessCompletion::Unconfirmed
    );
}

#[test]
fn crash_between_manifest_and_attempt_stays_unconfirmed_and_cannot_launch() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _receipt) = fixture(state.path(), generation);

    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
    assert_eq!(
        process_completion(state.path(), generation).unwrap(),
        ProcessCompletion::Unconfirmed
    );
    assert!(run_worker(&directory.join("manifest.json"), false).is_err());
}

#[test]
fn durable_spawn_rejection_recovers_not_started_without_exit_receipt() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _manifest, bytes, attempt) = attempted_fixture(state.path(), generation);
    write_spawn_rejected(&directory, &attempt).unwrap();

    let expected = not_started_receipt(generation, &bytes);
    assert_eq!(
        confirmed_exit(state.path(), generation).unwrap(),
        Some(expected)
    );
    assert_eq!(
        process_completion(state.path(), generation).unwrap(),
        ProcessCompletion::NotStarted
    );
    assert!(run_worker(&directory.join("manifest.json"), false).is_err());
}

#[test]
fn spawn_rejection_removes_isolated_auth_before_becoming_recoverable() {
    let state = tempfile::tempdir().unwrap();
    let state_path = state.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let home = state_path
        .join("grok-managed")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(home.join("grok")).unwrap();
    let auth = home.join("grok/auth.json");
    fs::write(&auth, b"private").unwrap();
    let manifest = Manifest {
        isolated_home: Some(home.canonicalize().unwrap()),
        isolated_state_dir: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: std::env::current_exe().unwrap(),
        arguments: Vec::new(),
        cwd: state_path.clone(),
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let (directory, bytes) = create_launch_manifest(&state_path, &manifest).unwrap();
    let attempt = write_spawn_attempt(&directory, generation, &bytes).unwrap();

    write_spawn_rejected(&directory, &attempt).unwrap();

    assert!(!auth.exists());
    assert_eq!(
        confirmed_exit(&state_path, generation).unwrap(),
        Some(not_started_receipt(generation, &bytes))
    );
}

#[test]
fn unbound_or_partial_spawn_rejection_never_unlocks_retry() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _manifest, _bytes, attempt) = attempted_fixture(state.path(), generation);
    let mut rejected = SpawnRejected {
        version: 1,
        generation,
        attempt: Uuid::new_v4(),
        manifest_sha256: attempt.manifest_sha256.clone(),
    };
    write_new_record(
        &directory.join(SPAWN_REJECTED_RECORD),
        &serde_json::to_vec(&rejected).unwrap(),
    )
    .unwrap();

    assert!(confirmed_exit(state.path(), generation).is_err());
    assert!(process_completion(state.path(), generation).is_err());
    fs::remove_file(directory.join(SPAWN_REJECTED_RECORD)).unwrap();
    rejected.attempt = attempt.attempt;
    write_new_record(
        &directory.join("spawn-rejected.partial"),
        &serde_json::to_vec(&rejected).unwrap(),
    )
    .unwrap();
    assert!(confirmed_exit(state.path(), generation).unwrap().is_none());
    assert_eq!(
        process_completion(state.path(), generation).unwrap(),
        ProcessCompletion::Unconfirmed
    );
}

#[test]
fn not_started_receipt_for_launch_attempt_requires_bound_rejection() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _manifest, bytes, attempt) = attempted_fixture(state.path(), generation);
    let receipt = not_started_receipt(generation, &bytes);
    write_receipt(&directory, &receipt).unwrap();

    assert!(confirmed_exit(state.path(), generation).is_err());
    write_spawn_rejected(&directory, &attempt).unwrap();
    assert_eq!(
        confirmed_exit(state.path(), generation).unwrap(),
        Some(receipt)
    );
}

#[test]
fn spawn_rejection_cannot_coexist_with_an_executed_exit_receipt() {
    let state = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (directory, _manifest, bytes, attempt) = attempted_fixture(state.path(), generation);
    write_spawn_rejected(&directory, &attempt).unwrap();
    let receipt = ExitReceipt {
        version: 1,
        generation,
        cleanup_confirmed: true,
        containment: "linux_subtree".to_owned(),
        exit_reason: ExitReason::NativeExit,
        exit_code: Some(0),
        manifest_sha256: sha256(&bytes),
    };
    write_receipt(&directory, &receipt).unwrap();

    assert!(confirmed_exit(state.path(), generation).is_err());
    assert!(process_completion(state.path(), generation).is_err());
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
fn host_bound_profile_receipts_preserve_storage_across_process_generations() {
    let application = tempfile::tempdir().unwrap();
    let state = application.path().canonicalize().unwrap();
    let home = state.join("grok-managed").join(Uuid::new_v4().to_string());
    fs::create_dir_all(home.join("grok")).unwrap();
    fs::write(home.join("grok/session-test"), b"preserved history").unwrap();
    for generation in [Uuid::new_v4(), Uuid::new_v4()] {
        let process_state = state
            .join("cli-agent-hosts")
            .join(generation.to_string())
            .join("native");
        let (directory, mut receipt) = fixture(&process_state, generation);
        let path = directory.join("manifest.json");
        let (mut manifest, _) = read_manifest(&path).unwrap();
        manifest.isolated_home = Some(home.clone());
        manifest.isolated_state_dir = Some(state.clone());
        let bytes = serde_json::to_vec(&manifest).unwrap();
        fs::write(&path, &bytes).unwrap();
        receipt.manifest_sha256 = sha256(&bytes);
        read_manifest(&path).unwrap();
        fs::write(home.join("grok/auth.json"), b"offline opaque test cache").unwrap();
        // 只验证受绑定路径的清理合同，不声称启动了原生进程。
        write_receipt(&directory, &receipt).unwrap();
        assert_eq!(
            confirmed_exit(&process_state, generation).unwrap(),
            Some(receipt)
        );
        assert!(!home.join("grok/auth.json").exists());
        assert_eq!(
            fs::read(home.join("grok/session-test")).unwrap(),
            b"preserved history"
        );
    }
}

#[test]
fn persistent_isolation_rejects_wrong_scope_host_generation_and_missing_home() {
    let application = tempfile::tempdir().unwrap();
    let state = application.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let process_state = state
        .join("cli-agent-hosts")
        .join(generation.to_string())
        .join("native");
    let (directory, mut receipt) = fixture(&process_state, generation);
    let path = directory.join("manifest.json");
    let (mut manifest, _) = read_manifest(&path).unwrap();
    let home = state.join("grok-managed").join(Uuid::new_v4().to_string());
    fs::create_dir_all(home.join("grok")).unwrap();
    fs::write(home.join("grok/auth.json"), b"offline opaque test cache").unwrap();
    manifest.isolated_home = Some(home.clone());
    manifest.isolated_state_dir = Some(state.clone());
    for invalid in [
        Manifest {
            isolated_state_dir: None,
            ..manifest.clone()
        },
        Manifest {
            isolated_state_dir: Some(state.parent().unwrap().to_owned()),
            ..manifest.clone()
        },
        Manifest {
            isolated_state_dir: Some(process_state.clone()),
            ..manifest.clone()
        },
        Manifest {
            isolated_home: None,
            ..manifest.clone()
        },
    ] {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        receipt.manifest_sha256 = sha256(&bytes);
        assert!(read_manifest(&path).is_err());
        assert!(write_receipt(&directory, &receipt).is_err());
        assert!(home.join("grok/auth.json").exists());
    }
    // 监督 manifest 自身代次正确，也不能借用另一个宿主的 native 目录。
    let other = Uuid::new_v4();
    let (other_directory, _) = fixture(&process_state, other);
    manifest.generation = other;
    fs::write(
        other_directory.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(read_manifest(&other_directory.join("manifest.json")).is_err());
    assert!(home.join("grok/auth.json").exists());
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

#[cfg(unix)]
#[test]
fn atomic_environment_removes_dynamic_loader_injection() {
    for name in [
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "LD_AUDIT",
        "LD_DEBUG",
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "LD_PROFILE",
    ] {
        assert!(unsafe_dynamic_loader_environment(std::ffi::OsStr::new(
            name
        )));
    }
    assert!(!unsafe_dynamic_loader_environment(std::ffi::OsStr::new(
        "PATH"
    )));
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
    let install = [
        "--settings",
        "{\"autoUpdatesChannel\":\"latest\"}",
        "install",
        "2.1.280",
    ]
    .map(OsString::from);
    assert!(
        environment
            .validate_for_generation(state.path(), generation, &install)
            .is_ok()
    );
    assert!(
        environment
            .validate_for_generation(state.path(), Uuid::new_v4(), &install)
            .is_err()
    );
    for target in [
        "latest",
        "stable",
        "",
        "2.1",
        "2.1.280.0",
        "2.1.280-beta",
        "02.1.280",
        "--force",
    ] {
        let mut changed = install.clone();
        changed[3] = target.into();
        assert!(
            environment
                .validate_for_generation(state.path(), generation, &changed)
                .is_err()
        );
    }
    let mut extra = install.to_vec();
    extra.push("--force".into());
    assert!(
        environment
            .validate_for_generation(state.path(), generation, &extra)
            .is_err()
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn codex_snapshot_layout_requires_the_bound_standalone_update() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("codex");
    let releases = home.join("packages/standalone/releases");
    fs::create_dir_all(&releases).unwrap();
    let executable = releases.join("0.155.1-aarch64-apple-darwin/bin/codex");
    let state = root.join("state");
    let arguments = [OsString::from("update")];
    let mut environment = ManagedEnvironment {
        values: vec![
            ("CODEX_HOME".into(), home.into_os_string()),
            (
                "CODEX_INSTALL_DIR".into(),
                root.join("bin").into_os_string(),
            ),
            ("CODEX_RELEASE".into(), "0.156.1".into()),
        ],
        remove: vec![],
    };
    assert_eq!(
        environment
            .update_snapshot_root(&state, &executable, &arguments)
            .unwrap(),
        releases
    );
    assert!(
        environment
            .update_snapshot_root(&state, &root.join("other"), &arguments)
            .is_err()
    );
    assert!(
        environment
            .update_snapshot_root(&state, &executable, &["update".into(), "--force".into()])
            .is_err()
    );
    environment.values.pop();
    assert!(
        environment
            .update_snapshot_root(&state, &executable, &arguments)
            .is_err()
    );
    assert_eq!(
        ManagedEnvironment::default()
            .update_snapshot_root(&state, &root.join("claude"), &[])
            .unwrap(),
        state
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
