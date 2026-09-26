use super::*;
use std::os::unix::fs::symlink;

fn manifest(directory: &Path) -> LaunchManifest {
    LaunchManifest {
        version: 1,
        launch_id: Uuid::from_u128(1),
        session_id: Uuid::from_u128(2),
        executable: PathBuf::from("/private/tmp/grok"),
        app_executable: PathBuf::from("/private/tmp/InfiniShell.app/Contents/MacOS/infinishell"),
        cwd: PathBuf::from("/private/tmp/中文 项目"),
        socket_path: directory.join("leader.sock"),
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
