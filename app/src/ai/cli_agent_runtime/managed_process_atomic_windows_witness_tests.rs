use std::io::Write as _;

use super::*;

#[test]
fn temp_environment_binds_only_the_three_case_insensitive_keys() {
    let environment = [
        ("tmp".into(), "t".into()),
        ("TEMP".into(), "t".into()),
        ("UserProfile".into(), "h".into()),
        ("TMPDIR".into(), "ignored".into()),
    ];
    assert_eq!(
        expected_temp_environment(&environment),
        Some([vec![116], vec![116], vec![104]])
    );
}

#[test]
fn temp_environment_rejects_missing_duplicate_or_embedded_nul_values() {
    assert!(expected_temp_environment(&[]).is_none());
    let mut environment = vec![
        ("TMP".into(), "t".into()),
        ("TEMP".into(), "t".into()),
        ("USERPROFILE".into(), "h".into()),
    ];
    environment.push(("tmp".into(), "other".into()));
    assert!(expected_temp_environment(&environment).is_none());
    environment.pop();
    environment[0].1 = "t\0".into();
    assert!(expected_temp_environment(&environment).is_none());
}

#[test]
fn snapshots_are_due_once_at_each_fixed_boundary() {
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(14), false, false, true),
        SnapshotAction::Wait
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(15), false, false, true),
        SnapshotAction::Early
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(239), true, false, true),
        SnapshotAction::Wait
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(240), true, false, true),
        SnapshotAction::Late
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(241), true, true, true),
        SnapshotAction::Wait
    );
}

#[test]
fn cancellation_or_deadline_precedes_due_snapshot() {
    assert_eq!(
        snapshot_action(true, false, Duration::from_secs(15), false, false, true),
        SnapshotAction::Cancelled
    );
    assert_eq!(
        snapshot_action(true, true, Duration::from_secs(240), true, false, true),
        SnapshotAction::Cancelled
    );
    assert_eq!(
        snapshot_action(false, true, Duration::from_secs(240), true, false, true),
        SnapshotAction::Expired
    );
}

#[test]
fn snapshots_require_the_original_root_binding() {
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(15), false, false, false),
        SnapshotAction::Wait
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(240), true, false, false),
        SnapshotAction::Wait
    );
}

#[test]
fn witness_requires_exact_generation_and_authorized_mode() {
    let generation = Uuid::new_v4();
    let value = generation.to_string();
    assert_eq!(
        enabled(generation, "cmd", Some(&value), Some("cmd")),
        Some(WitnessMode::Cmd)
    );
    assert_eq!(
        enabled(generation, "powershell", Some(&value), Some("powershell")),
        Some(WitnessMode::PowerShell)
    );
    assert_eq!(
        enabled(generation, "powershell", Some(&value), Some("cmd")),
        None
    );
    assert_eq!(
        enabled(generation, "cmd", Some(&value), Some("powershell")),
        None
    );
    assert_eq!(
        enabled(generation, "pwsh", Some(&value), Some("pwsh")),
        None
    );
    assert_eq!(enabled(generation, "cmd", Some(&value), None), None);
    assert_eq!(enabled(generation, "cmd", None, Some("cmd")), None);
    assert_eq!(enabled(generation, "cmd", Some("1"), Some("cmd")), None);
    assert_eq!(
        enabled(
            generation,
            "cmd",
            Some(&Uuid::new_v4().to_string()),
            Some("cmd")
        ),
        None
    );
    assert_eq!(
        enabled(generation, "cmd", Some(&format!("{value} ")), Some("cmd")),
        None
    );
}

#[test]
fn powershell_does_not_bind_the_cmd_creation_witness() {
    let file = tempfile::tempfile().unwrap();
    let event = DEBUG_EVENT::default();
    let root = || VerifiedImage {
        file: &file,
        base: 0,
        size: 0,
        sha256: "",
    };
    assert!(
        WitnessMode::PowerShell
            .bind_creation(&event, root(), 0, 0)
            .unwrap()
            .is_none()
    );
    assert!(
        WitnessMode::Cmd
            .bind_creation(&event, root(), 0, 0)
            .is_err()
    );
}

#[test]
fn powershell_module_overflow_is_partial_but_cmd_still_rejects() {
    assert!(WitnessMode::PowerShell.retain_module(63, false).unwrap());
    assert!(!WitnessMode::PowerShell.retain_module(64, false).unwrap());
    assert!(!WitnessMode::PowerShell.retain_module(65, false).unwrap());
    assert!(WitnessMode::Cmd.retain_module(63, false).unwrap());
    assert!(WitnessMode::Cmd.retain_module(64, false).is_err());
}

#[test]
fn partial_module_coverage_never_admits_a_duplicate_binding() {
    assert!(WitnessMode::PowerShell.retain_module(64, true).is_err());
    assert!(WitnessMode::PowerShell.retain_module(1, true).is_err());
    assert!(WitnessMode::Cmd.retain_module(1, true).is_err());
}

#[test]
fn mapped_size_uses_pe_optional_header_not_file_length() {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0xd0..0xd4].copy_from_slice(&0x5000u32.to_le_bytes());
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&bytes).unwrap();
    assert_eq!(
        mapped_image_size(&file, WitnessMode::Cmd, false).unwrap(),
        Some(0x5000)
    );
    assert_ne!(
        u64::from(
            mapped_image_size(&file, WitnessMode::Cmd, false)
                .unwrap()
                .unwrap()
        ),
        file.metadata().unwrap().len()
    );
    file.set_len(0xd2).unwrap();
    assert!(mapped_image_size(&file, WitnessMode::Cmd, false).is_err());
}

fn module_bytes() -> Vec<u8> {
    let mut bytes = vec![0; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
    bytes[0x96..0x98].copy_from_slice(&0x2002u16.to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0xa8..0xac].copy_from_slice(&0x1000u32.to_le_bytes());
    bytes[0xb8..0xbc].copy_from_slice(&0x1000u32.to_le_bytes());
    bytes[0xbc..0xc0].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0xd0..0xd4].copy_from_slice(&0x2000u32.to_le_bytes());
    bytes[0xd4..0xd8].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0xdc..0xde].copy_from_slice(&3u16.to_le_bytes());
    bytes[0x104..0x108].copy_from_slice(&16u32.to_le_bytes());
    bytes[0x188..0x18d].copy_from_slice(b".text");
    bytes[0x190..0x194].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x194..0x198].copy_from_slice(&0x1000u32.to_le_bytes());
    bytes[0x198..0x19c].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x19c..0x1a0].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x1ac..0x1b0].copy_from_slice(&0x6000_0020u32.to_le_bytes());
    bytes
}

fn pe32_module_bytes() -> Vec<u8> {
    let mut bytes = module_bytes();
    bytes[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&0x10bu16.to_le_bytes());
    bytes
}

fn module_file(bytes: &[u8]) -> (tempfile::TempDir, File) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("module.dll");
    std::fs::write(&path, bytes).unwrap();
    (directory, File::open(path).unwrap())
}

fn witness_session(file: &File, mode: WitnessMode) -> WindowsImageDebugSession {
    let identity = inspect_handle(file).unwrap();
    let mut diagnostics = NpmProcessDiagnostics::new();
    diagnostics.roles.insert(1, NpmProcessRole::Root);
    WindowsImageDebugSession {
        root_process_id: 1,
        expected_program_id: identity.id,
        expected_program_size: identity.size,
        expected_program_sha256: sha256_file(&mut file.try_clone().unwrap()).unwrap(),
        system_directory: prepare_system_directory().unwrap(),
        powershell: None,
        child_image: None,
        package_images: Some(Vec::new()),
        npm_console_host: None,
        child_images: HashMap::new(),
        component_images: HashMap::new(),
        processes: HashMap::new(),
        held_package_processes: Vec::new(),
        initial_breakpoints: HashSet::new(),
        pending_event: None,
        station_debugger: None,
        root_exit_observed: false,
        cancellation: None,
        npm_diagnostics: Some(diagnostics),
        native_witness: Some(NativeWitness {
            generation: Uuid::nil(),
            mode,
            modules: HashMap::new(),
            module_coverage_partial: false,
            skipped_module_events: 0,
            unsupported_module_events: 0,
            births: HashMap::new(),
            creation: None,
            completed_creation: None,
            creation_unavailable: None,
            snapshot: None,
            root_thread: None,
            clr: None,
            classification: None,
            expected_node: None,
            node_create: None,
            classification_entry: None,
            classification_return: None,
            pending_event: None,
            continuation_failed: false,
            expected_temp_environment: [vec![], vec![], vec![]],
            early_root_cpu: None,
            late_snapshot: None,
            failures: Vec::new(),
            cancelled: false,
            exit_confirmed: false,
        }),
        loader_trace: None,
    }
}

fn classification_package(
    root: &Path,
    relative: &Path,
) -> (WindowsDirectoryLease, SystemHelperLease) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = module_bytes();
    bytes[0x96..0x98].copy_from_slice(&0x0002u16.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    (
        prepare_directory(&AtomicDirectoryIdentity::capture(root).unwrap()).unwrap(),
        prepare_package_image(&ExpectedFileIdentity::capture(&path).unwrap(), false).unwrap(),
    )
}

#[test]
fn classification_requires_one_original_node_lease() {
    let directory = tempfile::tempdir().unwrap();
    let (root, mut node) = classification_package(directory.path(), Path::new(r"runtime\node.exe"));
    let mapped = Path::new(r"Z:\");
    assert!(classification_node_path(&[], &root, mapped).is_err());
    let duplicate = node.try_clone().unwrap();
    assert!(
        classification_node_path(
            &[(false, node.try_clone().unwrap()), (false, duplicate)],
            &root,
            mapped
        )
        .is_err()
    );
    node.npm_role = NpmProcessRole::BoundOther;
    assert!(classification_node_path(&[(false, node)], &root, mapped).is_err());
}

#[test]
fn classification_maps_only_the_original_node_under_the_bound_package_root() {
    for relative in [r"runtime\node.exe", r"install\node.exe"] {
        let directory = tempfile::tempdir().unwrap();
        let (root, node) = classification_package(directory.path(), Path::new(relative));
        let original_identity = node.identity;
        let images = [(false, node)];
        for drive in [r"D:\", r"Z:\"] {
            let expected: Vec<_> = format!("{drive}{relative}").encode_utf16().collect();
            assert_eq!(
                classification_node_path(&images, &root, Path::new(drive)).unwrap(),
                expected
            );
            assert_eq!(
                inspect_handle(&images[0].1.program).unwrap(),
                original_identity
            );
        }
    }
}

#[test]
fn classification_rejects_non_root_or_alternate_mapped_root_spelling() {
    let directory = tempfile::tempdir().unwrap();
    let (root, node) = classification_package(directory.path(), Path::new(r"runtime\node.exe"));
    let images = [(false, node)];
    for mapped in [
        r"C:\",
        r"z:\",
        r"Z:",
        r"Z:\child",
        r"\\?\Z:\",
        r"\\server\share",
        "Z:\\\0",
    ] {
        assert!(classification_node_path(&images, &root, Path::new(mapped)).is_err());
    }
}

#[test]
fn classification_rejects_same_name_nodes_outside_the_original_root_or_layout() {
    let directory = tempfile::tempdir().unwrap();
    let (root, _node) = classification_package(
        &directory.path().join("package"),
        Path::new(r"runtime\node.exe"),
    );
    let (_other_root, other_node) = classification_package(
        &directory.path().join("package-neighbor"),
        Path::new(r"runtime\node.exe"),
    );
    assert!(classification_node_path(&[(false, other_node)], &root, Path::new(r"Z:\")).is_err());
    for relative in [
        r"elsewhere\node.exe",
        r"runtime\nested\node.exe",
        r"runtime\Node.exe",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (root, node) = classification_package(directory.path(), Path::new(relative));
        assert!(classification_node_path(&[(false, node)], &root, Path::new(r"Z:\")).is_err());
    }
}

#[test]
fn classification_rechecks_original_root_and_node_identity_before_mapping() {
    let directory = tempfile::tempdir().unwrap();
    let (mut root, mut node) =
        classification_package(directory.path(), Path::new(r"runtime\node.exe"));
    let original_root = root.identity;
    root.identity.index ^= 1;
    assert!(
        classification_node_path(
            &[(false, node.try_clone().unwrap())],
            &root,
            Path::new(r"Z:\")
        )
        .is_err()
    );
    root.identity = original_root;
    node.sha256 = "0".repeat(64);
    assert!(classification_node_path(&[(false, node)], &root, Path::new(r"Z:\")).is_err());
}

#[test]
fn witness_continuation_rejects_a_different_original_event() {
    let (_directory, file) = module_file(&module_bytes());
    let mut session = witness_session(&file, WitnessMode::PowerShell);
    let event = DEBUG_EVENT {
        dwDebugEventCode: EXIT_THREAD_DEBUG_EVENT,
        dwProcessId: 1,
        dwThreadId: 2,
        ..Default::default()
    };
    session.native_witness_received(&event);
    assert!(
        session
            .native_witness_continued(1, 3, EXIT_THREAD_DEBUG_EVENT)
            .is_err()
    );
}

#[test]
fn witness_cleanup_drains_but_cannot_pass_after_invalid_continuation() {
    let (_directory, file) = module_file(&module_bytes());
    let mut session = witness_session(&file, WitnessMode::PowerShell);
    session
        .npm_diagnostics
        .as_mut()
        .unwrap()
        .begin_cleanup(false);
    let event = DEBUG_EVENT {
        dwDebugEventCode: EXIT_THREAD_DEBUG_EVENT,
        dwProcessId: 1,
        dwThreadId: 2,
        ..Default::default()
    };
    session.native_witness_received(&event);
    session
        .native_witness_continued(1, 3, EXIT_THREAD_DEBUG_EVENT)
        .unwrap();
    session.root_exit_observed = true;
    assert!(session.native_witness_confirm_exit().is_err());
    assert!(!session.native_witness.as_ref().unwrap().exit_confirmed);
}

#[test]
fn classification_error_evidence_does_not_include_arbitrary_error_text() {
    let evidence = classification_failure(
        "classification_thread",
        &io::Error::other("C:\\private\\input-token"),
    );
    assert!(evidence["register_readback"].is_null());
    assert!(!evidence.to_string().contains("input-token"));
}

#[test]
fn powershell_pe32_dll_is_partial_without_becoming_an_x64_module() {
    let (_directory, file) = module_file(&pe32_module_bytes());
    let mut session = witness_session(&file, WitnessMode::PowerShell);
    // 来源授权租约已存在；此用例不借诊断格式分类生成生产授权。
    let lease =
        lease_mapped_image(&file, &final_path_from_handle(&file).unwrap(), true, true).unwrap();
    session.component_images.insert(1, vec![lease]);

    session
        .native_witness_module(1, &file, 0x1000, true)
        .unwrap();

    let witness = session.native_witness.as_ref().unwrap();
    assert!(witness.module_coverage_partial);
    assert_eq!(witness.unsupported_module_events, 1);
    assert!(witness.modules[&1].is_empty());
    assert!(witness.failures.is_empty());
}

#[test]
fn pe32_dll_never_bypasses_missing_production_authorization() {
    let (_directory, file) = module_file(&pe32_module_bytes());
    let mut session = witness_session(&file, WitnessMode::PowerShell);

    assert!(
        session
            .native_witness_module(1, &file, 0x1000, true)
            .is_err()
    );
    let witness = session.native_witness.as_ref().unwrap();
    assert!(!witness.module_coverage_partial);
    assert_eq!(witness.unsupported_module_events, 0);
}

#[test]
fn cmd_and_root_still_reject_pe32_native_stack_bindings() {
    let (_directory, file) = module_file(&pe32_module_bytes());

    assert!(mapped_image_size(&file, WitnessMode::Cmd, true).is_err());
    assert!(mapped_image_size(&file, WitnessMode::Cmd, false).is_err());
    assert!(mapped_image_size(&file, WitnessMode::PowerShell, false).is_err());
}

#[test]
fn malformed_or_truncated_pe32_is_not_partial_coverage() {
    let mut bytes = pe32_module_bytes();
    bytes[0] = 0;
    let (_bad_directory, bad) = module_file(&bytes);
    assert!(mapped_image_size(&bad, WitnessMode::PowerShell, true).is_err());

    let (_short_directory, short) = module_file(&pe32_module_bytes()[..0xd2]);
    assert!(mapped_image_size(&short, WitnessMode::PowerShell, true).is_err());

    let mut mismatched = pe32_module_bytes();
    mismatched[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    let (_mismatch_directory, mismatch) = module_file(&mismatched);
    assert!(mapped_image_size(&mismatch, WitnessMode::PowerShell, true).is_err());

    let mut zero_size = pe32_module_bytes();
    zero_size[0xd0..0xd4].fill(0);
    let (_zero_directory, zero) = module_file(&zero_size);
    assert!(mapped_image_size(&zero, WitnessMode::PowerShell, true).is_err());
}

#[test]
fn unreadable_module_is_not_partial_coverage() {
    let (directory, file) = module_file(&pe32_module_bytes());
    drop(file);
    let write_only = OpenOptions::new()
        .write(true)
        .open(directory.path().join("module.dll"))
        .unwrap();

    assert!(mapped_image_size(&write_only, WitnessMode::PowerShell, true).is_err());
}

#[test]
fn powershell_bad_unwind_table_does_not_block_root_capture_preparation() {
    let mut bytes = module_bytes();
    bytes[0x120..0x124].copy_from_slice(&0x1000u32.to_le_bytes());
    let (_directory, file) = module_file(&bytes);
    let sha = sha256_file(&mut file.try_clone().unwrap()).unwrap();
    let modules = [VerifiedModule {
        file: &file,
        base: 0x10000,
        size: 0x2000,
        sha256: &sha,
    }];

    let (_, evidence, partial) = WitnessMode::PowerShell
        .prepare_root_modules(&modules)
        .unwrap();

    assert!(partial);
    let evidence = evidence.unwrap();
    assert_eq!(
        evidence["omitted"][0]["failure"]["reason"],
        "exception_directory_zero_mismatch"
    );
    assert_eq!(evidence["exception_bytes_attempted"], 0);
    // 同一准备入口返回 Ok，原 root 身份复核和 capture 不再被坏表前置短路；未调用原生线程 API。
    assert!(WitnessMode::Cmd.prepare_root_modules(&modules).is_err());
}

#[test]
fn powershell_root_capture_preparation_still_rejects_invalid_binding() {
    let (_directory, file) = module_file(&module_bytes());
    let modules = [VerifiedModule {
        file: &file,
        base: 0x10000,
        size: 0x2000,
        sha256: "invalid",
    }];

    assert!(
        WitnessMode::PowerShell
            .prepare_root_modules(&modules)
            .is_err()
    );
}

#[test]
fn authorized_component_bound_imports_keep_the_original_lease() {
    let mut bytes = module_bytes();
    bytes[0x160..0x164].copy_from_slice(&0x1100u32.to_le_bytes());
    bytes[0x164..0x168].copy_from_slice(&8u32.to_le_bytes());
    let (_directory, file) = module_file(&bytes);
    let path = final_path_from_handle(&file).unwrap();
    let lease = lease_mapped_image(&file, &path, true, true).unwrap();
    assert!(lease_mapped_image(&file, &path, true, false).is_err());
    let mut session = witness_session(&file, WitnessMode::PowerShell);
    session.component_images.insert(1, vec![lease]);

    session
        .native_witness_module(1, &file, 0x1000, true)
        .unwrap();

    let module = &session.native_witness.as_ref().unwrap().modules[&1][0];
    assert_eq!(module.lease.identity, inspect_handle(&file).unwrap());
    assert_eq!(module.size, 0x2000);
    assert!(OpenOptions::new().write(true).open(path).is_err());
}

#[test]
fn authorized_module_clone_rejects_a_changed_identity() {
    let (_directory, file) = module_file(&module_bytes());
    let path = final_path_from_handle(&file).unwrap();
    let mut lease = lease_mapped_image(&file, &path, true, true).unwrap();
    lease.identity.size += 1;
    let mut session = witness_session(&file, WitnessMode::PowerShell);
    session.component_images.insert(1, vec![lease]);

    assert!(
        session
            .native_witness_module(1, &file, 0x1000, true)
            .is_err()
    );
    assert!(
        !session
            .native_witness
            .as_ref()
            .unwrap()
            .module_coverage_partial
    );
}

#[test]
fn authorized_module_clone_rejects_different_file_identity_with_same_bytes() {
    let (_directory, file) = module_file(&module_bytes());
    let (_other_directory, other) = module_file(&module_bytes());
    let lease =
        lease_mapped_image(&file, &final_path_from_handle(&file).unwrap(), true, true).unwrap();
    let mut session = witness_session(&other, WitnessMode::PowerShell);
    session.package_images = Some(vec![(true, lease)]);

    assert!(
        session
            .native_witness_module(1, &other, 0x1000, true)
            .is_err()
    );
    assert!(
        !session
            .native_witness
            .as_ref()
            .unwrap()
            .module_coverage_partial
    );
}

#[test]
fn safe_failure_preserves_wrapped_native_error_without_error_text() {
    let hresult = HRESULT::from_win32(5);
    let failure = io::Error::other(WindowsError::from_hresult(hresult));
    let observed = safe_failure("thread_open", &failure);
    assert_eq!(observed["hresult"], hresult.0);
    assert_eq!(observed["win32_error"], 5);
    let direct = safe_failure("thread_open", &io::Error::from_raw_os_error(6));
    assert_eq!(direct["win32_error"], 6);
    let private = safe_failure(
        "module_binding",
        &io::Error::other("不得记录此路径或控制材料"),
    );
    assert!(!private.to_string().contains("不得记录"));
}
