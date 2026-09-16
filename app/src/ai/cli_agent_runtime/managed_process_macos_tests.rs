use super::super::{confirmed_exit, create_generation_directory, write_new_record};
use super::*;

struct Fixture {
    directory: PathBuf,
    manifest: Manifest,
    manifest_bytes: Vec<u8>,
    claim: Claim,
    native: NativeClaim,
    proof: CleanupProof,
    receipt: ExitReceipt,
}

impl Fixture {
    fn new(state: &Path) -> Self {
        let generation = Uuid::new_v4();
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
        let manifest_bytes = encode(&manifest).unwrap();
        // 仅构造持久化校验输入；这些编号不是本机清理目标，也不替代真实回执验收。
        let claim = Claim {
            version: 1,
            generation,
            manifest_sha256: sha256(&manifest_bytes),
            label: label(generation),
            boot_session: Uuid::new_v4().to_string(),
            wrapper: SavedIdentity {
                pid: 100,
                pid_version: 200,
                unique_id: 300,
                resource_cid: 400,
            },
        };
        let native = NativeClaim {
            generation,
            claim_sha256: String::new(),
            identity: SavedIdentity {
                pid: 101,
                pid_version: 201,
                unique_id: 301,
                resource_cid: 400,
            },
        };
        let proof = CleanupProof {
            version: 1,
            generation,
            claim_sha256: String::new(),
            native_sha256: None,
            job_removed: true,
            resource_cid_destroyed: true,
            native_wait_status: Some(37 << 8),
            execution_failed: false,
        };
        let receipt = ExitReceipt {
            version: 1,
            generation,
            cleanup_confirmed: true,
            containment: CONTAINMENT.to_owned(),
            exit_reason: ExitReason::NativeExit,
            exit_code: Some(37),
            manifest_sha256: sha256(&manifest_bytes),
        };
        Self {
            directory,
            manifest,
            manifest_bytes,
            claim,
            native,
            proof,
            receipt,
        }
    }

    fn save(mut self) -> Self {
        let claim = encode(&self.claim).unwrap();
        self.native.claim_sha256 = sha256(&claim);
        let native = encode(&self.native).unwrap();
        self.proof.claim_sha256 = sha256(&claim);
        self.proof.native_sha256 = Some(sha256(&native));
        for (file, bytes) in [
            ("manifest.json", self.manifest_bytes.clone()),
            (CLAIM_FILE, claim),
            (NATIVE_FILE, native),
            (PROOF_FILE, encode(&self.proof).unwrap()),
        ] {
            write_new_record(&self.directory.join(file), &bytes).unwrap();
        }
        write_receipt(&self.directory, &self.receipt).unwrap();
        self
    }
}

#[test]
fn persisted_complete_proof_survives_a_later_system_boot_without_touching_old_cid() {
    let state = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(state.path()).save();
    assert_ne!(fixture.claim.boot_session, macos_boot_session().unwrap());
    assert_eq!(
        confirmed_exit(state.path(), fixture.manifest.generation).unwrap(),
        Some(fixture.receipt)
    );
}

#[test]
fn missing_proof_cannot_upgrade_a_claim_after_system_boot_changed() {
    let state = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(state.path()).save();
    fs::remove_file(fixture.directory.join(PROOF_FILE)).unwrap();
    assert!(confirmed_exit(state.path(), fixture.manifest.generation).is_err());
}

#[test]
fn proved_cleanup_can_allow_resume_without_inventing_a_native_result() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(state.path());
    fixture.proof.native_wait_status = None;
    fixture.proof.execution_failed = true;
    fixture.receipt.exit_code = None;
    fixture.receipt.exit_reason = ExitReason::HostDisconnected;
    let fixture = fixture.save();
    assert_eq!(
        confirmed_exit(state.path(), fixture.manifest.generation).unwrap(),
        Some(fixture.receipt)
    );
}

#[test]
fn changed_first_cid_is_rejected_even_when_file_hashes_are_consistent() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(state.path());
    fixture.native.identity.resource_cid += 1;
    let fixture = fixture.save();
    assert!(confirmed_exit(state.path(), fixture.manifest.generation).is_err());
}

#[test]
fn incomplete_cleanup_never_unlocks_resume() {
    for (removed, destroyed, confirmed) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        let state = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new(state.path());
        fixture.proof.job_removed = removed;
        fixture.proof.resource_cid_destroyed = destroyed;
        fixture.receipt.cleanup_confirmed = confirmed;
        let fixture = fixture.save();
        assert!(confirmed_exit(state.path(), fixture.manifest.generation).is_err());
    }
}

#[test]
fn native_exit_code_must_match_the_direct_parent_wait_result() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(state.path());
    fixture.receipt.exit_code = Some(0);
    let fixture = fixture.save();
    assert!(confirmed_exit(state.path(), fixture.manifest.generation).is_err());
}

#[test]
fn stopped_or_foreign_generation_wait_status_is_not_a_native_exit() {
    let identity = SavedIdentity {
        pid: 1,
        pid_version: 2,
        unique_id: 3,
        resource_cid: 4,
    };
    let generation = Uuid::new_v4();
    assert!(
        validate_native_status(
            NativeStatus {
                generation,
                identity: identity.clone(),
                wait_status: (libc::SIGSTOP << 8) | 0x7f
            },
            generation,
            &identity
        )
        .is_err()
    );
    assert!(
        validate_native_status(
            NativeStatus {
                generation: Uuid::new_v4(),
                identity: identity.clone(),
                wait_status: 0
            },
            generation,
            &identity
        )
        .is_err()
    );
}

#[test]
fn exec_version_change_preserves_identity_but_reused_pid_does_not() {
    let initial = SavedIdentity {
        pid: 101,
        pid_version: 201,
        unique_id: 301,
        resource_cid: 401,
    };
    let mut current = MacosProcessIdentity::from(initial.clone());
    current.pid_version += 1;
    assert_eq!(validate_after_exec(current, &initial).unwrap(), current);
    current.unique_id += 1;
    assert!(validate_after_exec(current, &initial).is_err());
    current.unique_id = initial.unique_id;
    current.resource_cid += 1;
    assert!(validate_after_exec(current, &initial).is_err());
}

#[test]
fn raw_environment_is_preserved_without_accepting_internal_routes_or_nul() {
    let values = vec![(b"RAW".to_vec(), b"\xff value".to_vec())];
    let decoded = decode_environment(Environment {
        values: values.clone(),
    })
    .unwrap();
    assert_eq!(decoded[0].0.as_bytes(), values[0].0);
    assert_eq!(decoded[0].1.as_bytes(), values[0].1);
    for (key, value) in [
        (EXEC_CONTROL_ENV.as_bytes().to_vec(), b"route".to_vec()),
        (b"A=B".to_vec(), Vec::new()),
        (b"KEY".to_vec(), b"bad\0value".to_vec()),
    ] {
        assert!(
            decode_environment(Environment {
                values: vec![(key, value)]
            })
            .is_err()
        );
    }
}

#[test]
fn oversized_ipc_length_is_rejected_before_payload_allocation() {
    let (mut server, mut client) = UnixStream::pair().unwrap();
    client
        .write_all(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
        .unwrap();
    assert!(frame_read::<Environment>(&mut server).is_err());
}

#[test]
fn atomic_proof_publication_never_overwrites_existing_or_partial_records() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join(PROOF_FILE);
    persist_record(&path, b"partial old proof").unwrap();
    assert!(persist_record(&path, b"replacement proof").is_err());
    assert_eq!(read_record(&path).unwrap(), b"partial old proof");
    assert_eq!(fs::read_dir(state.path()).unwrap().count(), 1);
}

#[test]
fn stderr_forwarding_failure_is_reported_without_erasing_completed_cleanup() {
    let state = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(state.path()).save();
    let socket_directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(socket_directory.path().join("control")).unwrap();
    let (output, output_completed) = mpsc::channel();
    let (errors, errors_completed) = mpsc::channel();
    output.send(Ok(())).unwrap();
    errors
        .send(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "固定 stderr 转发故障",
        )))
        .unwrap();
    // 没有注册 launchd job 或领取内核域，只验证已经提交的清理证明与转发故障分开。
    let mut job = Job {
        directory: fixture.directory.clone(),
        service: label(fixture.manifest.generation),
        registered: false,
        bootstrap_pending: false,
        cleanup_attempted: true,
        coalition: None,
        output_completed: Some(output_completed),
        errors_completed: Some(errors_completed),
        _socket_directory: socket_directory,
        listener,
    };
    assert!(
        job.finish_output()
            .unwrap_err()
            .to_string()
            .contains("stderr 转发失败")
    );
    assert!(job.output_completed.is_none() && job.errors_completed.is_none());
    assert_eq!(
        confirmed_exit(state.path(), fixture.manifest.generation).unwrap(),
        Some(fixture.receipt)
    );
}

#[test]
fn failed_job_removal_still_cleans_the_domain_and_preserves_both_errors() {
    let calls = std::cell::RefCell::new(Vec::new());
    let error = cleanup_operations(
        || {
            calls.borrow_mut().push("remove");
            Err(io::Error::other("固定移除失败"))
        },
        || {
            calls.borrow_mut().push("domain");
            Err(io::Error::other("固定域清理失败"))
        },
    )
    .unwrap_err();
    assert_eq!(*calls.borrow(), ["remove", "domain"]);
    assert!(error.to_string().contains("固定移除失败"));
    assert!(error.to_string().contains("固定域清理失败"));
    let error = cleanup_operations(|| Err(io::Error::other("移除未确认")), || Ok(())).unwrap_err();
    assert_eq!(error.to_string(), "移除未确认");
}
