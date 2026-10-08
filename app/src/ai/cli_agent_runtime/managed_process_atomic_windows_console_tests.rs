use std::sync::mpsc;

use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Storage::FileSystem::{
    FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::PROCESS_SYNCHRONIZE;

use super::*;

#[test]
fn npm_debug_trace_bounds_recent_events_without_losing_totals() {
    let generation = Uuid::new_v4();
    let mut trace = NpmDebugTrace::new(generation, 71);
    // 跨越环形记录上限；总计不能随最旧事件一起丢弃。
    for process_id in 101..=118 {
        trace.received(
            &DEBUG_EVENT {
                dwDebugEventCode: CREATE_THREAD_DEBUG_EVENT,
                dwProcessId: process_id,
                dwThreadId: 29,
                ..Default::default()
            },
            7,
            71,
            false,
        );
        trace.validated(NpmDebugResult::from_io(Ok(()), Duration::ZERO, 71), "node");
        trace.continued(
            NpmDebugResult::from_io(Ok(()), Duration::ZERO, 71),
            DBG_CONTINUE.0,
        );
    }

    assert_eq!(trace.generation, generation);
    assert_eq!(
        (trace.received, trace.validated, trace.continued),
        (18, 18, 18)
    );
    assert_eq!(trace.dropped_events, 2);
    assert_eq!(trace.recent.len(), 16);
    assert_eq!(trace.recent.front().unwrap().sequence, 3);
    let last = trace.recent.back().unwrap();
    assert_eq!(last.sequence, 18);
    assert_eq!((last.process_id, last.thread_id), (118, 29));
    assert_eq!(last.code, CREATE_THREAD_DEBUG_EVENT.0);
    assert_eq!(last.received_on_thread_id, 71);
    assert_eq!(last.mode, "normal");
    assert_eq!(last.role, "node");
    assert_eq!(last.continue_status, Some(DBG_CONTINUE.0));
}

#[test]
fn npm_debug_trace_waits_preserve_hresult_without_inventing_events() {
    let mut trace = NpmDebugTrace::new(Uuid::new_v4(), 71);
    let timeout = HRESULT::from_win32(ERROR_SEM_TIMEOUT.0);
    trace.wait_finished(Duration::from_millis(100), 71, Some(timeout));
    trace.wait_finished(Duration::from_millis(90), 72, Some(timeout));
    let denied = HRESULT::from_win32(5);
    trace.wait_finished(Duration::from_millis(3), 72, Some(denied));

    assert_eq!(trace.wait_calls, 3);
    assert_eq!(trace.wait_timeouts, 2);
    assert_eq!(trace.wait_elapsed_ms, 193);
    assert_eq!(trace.longest_wait_ms, 100);
    assert_eq!(trace.last_wait_hresult, Some(denied.0));
    assert_eq!(trace.last_boundary, "wait_failed");
    assert_eq!(trace.last_call_thread_id, 72);
    assert_eq!(trace.spawn_thread_id, 71);
    assert_eq!(
        (trace.received, trace.validated, trace.continued),
        (0, 0, 0)
    );
    assert!(trace.recent.is_empty());
}

#[test]
fn npm_debug_summary_binds_generation_and_omits_sensitive_failure_text() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let mut session = lease.prepare_image_debug_session().unwrap();
    assert!(session.package_debug_summary(Ok(0)).is_none());
    session.npm_diagnostics = Some(NpmProcessDiagnostics::new());
    let generation = Uuid::new_v4();
    session.bind_package_diagnostics(generation);
    session.bind_package_diagnostics(Uuid::new_v4());
    session.record_package_spawn_result(Ok(()), Duration::from_millis(9));
    let event = DEBUG_EVENT {
        dwDebugEventCode: LOAD_DLL_DEBUG_EVENT,
        dwProcessId: 31,
        dwThreadId: 32,
        ..Default::default()
    };
    session.pending_event = Some((31, 32, LOAD_DLL_DEBUG_EVENT));
    session
        .npm_diagnostics
        .as_mut()
        .unwrap()
        .trace
        .as_mut()
        .unwrap()
        .received(&event, 12, 71, false);
    let denied = HRESULT::from_win32(5);
    let api_failure = io::Error::other(WindowsError::from_hresult(denied));
    session.record_event_validation(Err(&api_failure), Duration::from_millis(4));
    let failure = io::Error::other(r"C:\private\credential-and-environment-sentinel");

    let fields = session.package_debug_summary(Err(&failure)).unwrap();
    assert_eq!(fields["generation"], generation.to_string());
    assert_eq!(fields["spawn_result"]["ok"], true);
    assert_eq!(fields["spawn_result"]["elapsed_ms"], 9);
    assert_eq!(fields["spawn_thread_id"], fields["spawn_return_thread_id"]);
    assert_eq!(fields["last_call_thread_id"], fields["spawn_thread_id"]);
    assert_eq!(
        fields["pending_event"],
        serde_json::json!([31, 32, LOAD_DLL_DEBUG_EVENT.0])
    );
    assert_eq!(fields["last_boundary"], "validation_failed");
    assert_eq!(fields["validated"], 0);
    assert_eq!(fields["continued"], 0);
    assert_eq!(fields["recent"][0]["sequence"], 1);
    assert_eq!(fields["recent"][0]["validation"]["ok"], false);
    assert_eq!(fields["recent"][0]["validation"]["hresult"], denied.0);
    assert_eq!(fields["recent"][0]["validation"]["elapsed_ms"], 4);
    assert!(fields["recent"][0]["continuation"].is_null());
    assert!(
        !fields
            .to_string()
            .contains("credential-and-environment-sentinel")
    );
    assert!(
        !fields
            .to_string()
            .contains(&fixture.directory.path().display().to_string())
    );
    let raw_failure = io::Error::from_raw_os_error(5);
    let raw = session.package_debug_summary(Err(&raw_failure)).unwrap();
    assert_eq!(raw["failure_codes"]["win32_error"], 5);
    assert!(raw["failure_codes"]["hresult"].is_null());
    // 原生非零退出并非 io::Error，仍须保留失败事件，不能标记成成功。
    let native_failure = session.package_debug_summary(Ok(17)).unwrap();
    assert_eq!(native_failure["native_exit_code"], 17);
    assert_eq!(native_failure["failed"], true);
    assert!(native_failure["recent"].is_array());
    // 无失败时不输出逐事件详情，避免正常消费者更新产生大段日志。
    assert!(
        session
            .package_debug_summary(Ok(0))
            .unwrap()
            .get("recent")
            .is_none()
    );
}

#[test]
fn npm_event_roles_distinguish_remaining_console_and_reused_process_identity() {
    let mut diagnostics = NpmProcessDiagnostics::new();
    for (process_id, role) in [(11, NpmProcessRole::Root), (12, NpmProcessRole::Console)] {
        diagnostics.received(process_id, CREATE_PROCESS_DEBUG_EVENT, None);
        diagnostics.roles.insert(process_id, role);
        let event = diagnostics
            .continued(process_id, CREATE_PROCESS_DEBUG_EVENT)
            .unwrap();
        assert_eq!(event.phase, "create_continued");
        assert_eq!(event.mode, "normal");
        assert_eq!(event.native_exit_code, None);
    }

    diagnostics.received(11, EXIT_PROCESS_DEBUG_EVENT, Some(5));
    let event = diagnostics.continued(11, EXIT_PROCESS_DEBUG_EVENT).unwrap();
    assert_eq!(event.role, "root");
    assert_eq!(event.native_exit_code, Some(5));
    assert_eq!(event.remaining, [0, 0, 0, 1, 0, 0]);

    // 后续复用同一数值身份也不能继承已退出根进程的角色。
    diagnostics.received(11, CREATE_PROCESS_DEBUG_EVENT, None);
    let event = diagnostics
        .continued(11, CREATE_PROCESS_DEBUG_EVENT)
        .unwrap();
    assert_eq!(event.role, "unknown");
    assert_eq!(event.remaining, [0, 0, 0, 1, 0, 1]);
}

#[test]
fn npm_cleanup_events_preserve_observed_cancellation_without_inventing_it() {
    for cancelled in [false, true] {
        let mut diagnostics = NpmProcessDiagnostics::new();
        let started = diagnostics.started;
        diagnostics.begin_cleanup(cancelled);
        // 清理禁用取消后再进入收尾，不能抹掉此前已观察的取消。
        diagnostics.begin_cleanup(false);
        diagnostics.received(1, CREATE_PROCESS_DEBUG_EVENT, None);
        let event = diagnostics
            .continued(1, CREATE_PROCESS_DEBUG_EVENT)
            .unwrap();
        assert_eq!(event.mode, "cleanup");
        assert_eq!(event.role, "unknown");
        assert_eq!(event.cancel_observed, cancelled);
        assert_eq!(diagnostics.started, started);
    }
}

#[test]
fn npm_creation_timing_does_not_invent_missing_or_negative_ages() {
    let mut diagnostics = NpmProcessDiagnostics::new();
    assert_eq!(diagnostics.creation_timing(None), (None, None));
    assert_eq!(diagnostics.creation_timing(Some(10_000)), (None, None));

    diagnostics.termination_requested_at = Some(50_001);
    assert_eq!(diagnostics.creation_timing(None), (None, None));
    assert_eq!(
        diagnostics.creation_timing(Some(50_002)),
        (Some(false), None)
    );
    assert_eq!(
        diagnostics.creation_timing(Some(50_001)),
        (Some(true), Some(0))
    );
    assert_eq!(
        diagnostics.creation_timing(Some(10_000)),
        (Some(true), Some(4))
    );
}

#[test]
fn npm_creation_timing_preserves_filetime_high_bits_without_overflow() {
    let created_at = filetime_ticks(FILETIME {
        dwLowDateTime: 0xffff_ffff,
        dwHighDateTime: 0x7fff_ffff,
    });
    let stopped_at = filetime_ticks(FILETIME {
        dwLowDateTime: 0x0000_270f,
        dwHighDateTime: 0x8000_0000,
    });
    assert_eq!(created_at, 0x7fff_ffff_ffff_ffff);
    assert_eq!(stopped_at, 0x8000_0000_0000_270f);
    let mut diagnostics = NpmProcessDiagnostics::new();
    diagnostics.termination_requested_at = Some(stopped_at);
    assert_eq!(
        diagnostics.creation_timing(Some(created_at)),
        (Some(true), Some(1))
    );

    diagnostics.termination_requested_at = Some(filetime_ticks(FILETIME {
        dwLowDateTime: u32::MAX,
        dwHighDateTime: u32::MAX,
    }));
    assert_eq!(diagnostics.termination_requested_at, Some(u64::MAX));
    assert_eq!(
        diagnostics.creation_timing(Some(0)),
        (Some(true), Some(1_844_674_407_370_955))
    );
}

#[test]
fn npm_termination_request_never_replaces_the_first_timestamp() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let mut session = lease.prepare_image_debug_session().unwrap();
    let mut diagnostics = NpmProcessDiagnostics::new();
    // 固定旧值排除两次实际时钟读取恰好相同而误通过的情况。
    diagnostics.termination_requested_at = Some(1);
    session.npm_diagnostics = Some(diagnostics);

    session.record_package_termination_request();
    session.record_package_termination_request();

    assert_eq!(
        session
            .npm_diagnostics
            .as_ref()
            .unwrap()
            .termination_requested_at,
        Some(1)
    );
}

#[test]
fn npm_creation_time_failure_does_not_invent_a_timestamp() {
    let fixture = Fixture::new();
    let file = File::open(&fixture.program).unwrap();

    assert_eq!(process_creation_time(HANDLE::default()), None);
    assert_eq!(process_creation_time(HANDLE(file.as_raw_handle())), None);
}

#[test]
fn npm_cleanup_image_role_uses_bound_identity_instead_of_names_or_bytes() {
    let fixture = Fixture::new();
    let node = fixture.bin.join("node.exe");
    let codex = fixture.bin.join("codex.exe");
    let other = fixture.bin.join("helper.exe");
    let unbound = fixture.install.join("node.exe");
    fs::copy(&fixture.program, &node).unwrap();
    fs::copy(&fixture.program, &codex).unwrap();
    fs::copy(&fixture.program, &other).unwrap();
    fs::copy(&node, &unbound).unwrap();
    let mut lease = prepare(&fixture.expected()).unwrap();
    lease
        .set_package_images(vec![
            ExpectedFileIdentity::capture(&node).unwrap(),
            ExpectedFileIdentity::capture(&codex).unwrap(),
            ExpectedFileIdentity::capture(&other).unwrap(),
        ])
        .unwrap();
    let session = lease.prepare_image_debug_session().unwrap();
    let node = File::open(node).unwrap();
    let unbound = File::open(unbound).unwrap();
    assert_ne!(
        inspect_handle(&node).unwrap().id,
        inspect_handle(&unbound).unwrap().id
    );
    assert_eq!(
        sha256_file(&mut node.try_clone().unwrap()).unwrap(),
        sha256_file(&mut unbound.try_clone().unwrap()).unwrap()
    );

    assert_eq!(session.cleanup_image_role(&node), NpmProcessRole::Node);
    assert_eq!(
        session.cleanup_image_role(&File::open(codex).unwrap()),
        NpmProcessRole::Codex
    );
    assert_eq!(
        session.cleanup_image_role(&File::open(other).unwrap()),
        NpmProcessRole::BoundOther
    );
    assert_eq!(
        session.cleanup_image_role(&unbound),
        NpmProcessRole::Unknown
    );
    assert!(session.processes.is_empty());
    assert!(session.pending_event.is_none());
}

#[test]
fn npm_bound_roles_never_expose_unknown_image_names() {
    for (path, role) in [
        (r"C:\private\NODE.EXE", "node"),
        (r"C:\private\codex.exe", "codex"),
        (r"C:\private\conhost.exe", "bound-other"),
        (r"C:\private\user-secret.exe", "bound-other"),
    ] {
        assert_eq!(NpmProcessRole::bound_image(Path::new(path)).as_str(), role);
    }
}

#[test]
#[ignore = "取消后必须在严格 Job 中收敛真实调试事件和 AppContainer"]
fn npm_cancel_before_initial_event_confirms_cleanup_without_running_child() {
    let name =
        "console_binding::npm_cancel_before_initial_event_confirms_cleanup_without_running_child";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::with_bound_child(true);
    fixture
        .debugger
        .bind_cancellation(Arc::new(AtomicBool::new(true)));
    fixture.process.resume().unwrap();

    let failure = fixture
        .debugger
        .verify_package_initial_image_in_container(&fixture.process)
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
    fixture.debugger.record_package_termination_request();
    assert!(
        fixture
            .debugger
            .npm_diagnostics
            .as_ref()
            .unwrap()
            .termination_requested_at
            .is_some()
    );
    fixture.finish();
    assert!(fixture.debugger.pending_event.is_none());
    assert!(fixture.debugger.processes.is_empty());
    assert!(fixture.debugger.held_package_processes.is_empty());
    record_debug_native_exit(name);
}

#[test]
#[ignore = "根进程已获准后取消仍须逐句柄确认退出并恢复 ACL"]
fn npm_cancel_after_initial_event_confirms_cleanup_without_running_child() {
    let name =
        "console_binding::npm_cancel_after_initial_event_confirms_cleanup_without_running_child";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::with_bound_child(true);
    let cancellation = Arc::new(AtomicBool::new(false));
    fixture.debugger.bind_cancellation(cancellation.clone());
    fixture.process.resume().unwrap();
    fixture
        .debugger
        .verify_package_initial_image_in_container(&fixture.process)
        .unwrap();
    cancellation.store(true, Ordering::Release);

    let failure = fixture
        .debugger
        .drain_package_in_container_until_exit(&fixture.process)
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
    fixture.debugger.record_package_termination_request();
    let requested_at = fixture
        .debugger
        .npm_diagnostics
        .as_ref()
        .unwrap()
        .termination_requested_at;
    assert!(requested_at.is_some());
    fixture.finish();
    assert!(fixture.debugger.pending_event.is_none());
    assert!(fixture.debugger.processes.is_empty());
    assert!(fixture.debugger.held_package_processes.is_empty());
    assert_eq!(
        fixture
            .debugger
            .npm_diagnostics
            .as_ref()
            .unwrap()
            .termination_requested_at,
        requested_at
    );
    record_debug_native_exit(name);
}

#[test]
fn npm_console_binding_requires_explicit_package_mode() {
    let fixture = Fixture::new();
    let mut lease = prepare(&fixture.expected()).unwrap();

    assert!(lease.enable_npm_console_host().is_err());
    assert!(lease.npm_console_host.is_none());
    lease.set_package_images(vec![fixture.expected()]).unwrap();
    assert!(lease.npm_console_host.is_none());
    lease.enable_npm_console_host().unwrap();
    assert!(lease.enable_npm_console_host().is_err());
}

#[test]
fn npm_console_binding_rejects_same_byte_private_copy_and_changed_digest() {
    let fixture = Fixture::new();
    let system = prepare_system_directory().unwrap();
    let console = prepare_npm_console_host(&system).unwrap();
    let original = File::open(final_path_from_handle(&console.program).unwrap()).unwrap();
    verify_npm_console_host(&console, &original, &system).unwrap();
    let private = fixture.bin.join("conhost.exe");
    fs::copy(final_path_from_handle(&console.program).unwrap(), &private).unwrap();

    assert!(verify_npm_console_host(&console, &File::open(private).unwrap(), &system).is_err());
    let mut changed = console.try_clone().unwrap();
    changed.sha256 = "0".repeat(64);
    assert!(verify_npm_console_host(&changed, &original, &system).is_err());
    assert!(
        OpenOptions::new()
            .write(true)
            .open(final_path_from_handle(&console.program).unwrap())
            .is_err()
    );
}

#[test]
#[ignore = "Windows npm 精确控制台依赖和真实 AppContainer 身份须显式运行"]
fn npm_console_host_runs_bound_child_and_confirms_cleanup() {
    let name = "console_binding::npm_console_host_runs_bound_child_and_confirms_cleanup";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::with_bound_child(true);
    let result = (|| -> io::Result<u32> {
        fixture.process.resume()?;
        fixture
            .debugger
            .verify_package_initial_image_in_container(&fixture.process)?;
        fixture
            .debugger
            .drain_package_in_container_until_exit(&fixture.process)?;
        fixture.process.exit_code()
    })();
    // 首次实机可能在宿主隔离核验处拒绝；先收敛 pending 事件和 ACL/profile，再断言原结果。
    let termination = if result.is_err() {
        fixture.process.terminate_job().and_then(|()| {
            fixture
                .debugger
                .drain_terminated_package_in_container(&fixture.process)
        })
    } else {
        Ok(())
    };
    let cleanup =
        termination.and_then(|()| fixture.process.write_cleanup_receipt(&fixture.receipt));
    cleanup.expect("正例无论运行成功或拒绝，都必须取得显式清理证明");

    assert_eq!(result.unwrap(), 0);
    assert!(
        fixture.marker.exists(),
        "已绑定子映像必须真正运行，而非提前拒绝控制台宿主"
    );
    assert_eq!(
        fs::read(&fixture.receipt).unwrap(),
        b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n"
    );
    record_debug_native_exit(name);
}

#[test]
#[ignore = "Windows npm 公共 shim 的 NUL 重定向须在真实 AppContainer 中与普通写入对照"]
fn npm_cmd_nul_redirection_runs_between_builtin_controls() {
    let name = "console_binding::npm_cmd_nul_redirection_runs_between_builtin_controls";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for relative in ["home", "config", "cache", "data", "tmp"] {
        fs::create_dir(root.join(relative)).unwrap();
    }
    let script = root.join("nul-control.cmd");
    // 同一 CMD、目录与 token 对照普通文件和 NUL 的 stderr 重定向；脚本不启动外部命令。
    // NUL 加扩展名仍是保留设备名，stdout 标记必须使用普通文件名。
    fs::write(
        &script,
        concat!(
            "@echo off\r\n",
            ">tmp\\plain.txt echo builtin-control\r\n",
            ">tmp\\plain-status.txt echo %errorlevel%\r\n",
            ">tmp\\regular.txt 2>tmp\\regular-stderr.txt echo builtin-control\r\n",
            ">tmp\\regular-status.txt echo %errorlevel%\r\n",
            ">tmp\\redirected.txt 2>NUL echo builtin-control\r\n",
            ">tmp\\nul-status.txt echo %errorlevel%\r\n",
            ">tmp\\after.txt echo builtin-control\r\n",
            "exit /b 0\r\n",
        ),
    )
    .unwrap();
    let script_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&script)
        .unwrap();
    let system = prepare_system_directory().unwrap();
    let program = final_path_from_handle(&system.file)
        .unwrap()
        .join("cmd.exe");
    let expected = ExpectedFileIdentity::capture(&program).unwrap();
    let mut lease = prepare(&expected).unwrap();
    lease.set_package_images(vec![expected]).unwrap();
    lease.enable_npm_console_host().unwrap();
    let cwd = prepare_directory(&AtomicDirectoryIdentity::capture(&root).unwrap()).unwrap();
    let execution_cwd = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_eq!(execution_cwd.canonicalize().unwrap(), root);
    let mut debugger = lease.prepare_image_debug_session().unwrap();
    // 仅启用已有核验后只读环境钩子；不改变 NUL 对照的派生、清理与成功断言。
    debugger.loader_trace = Some(loader_tests::LoaderTrace::default());
    let mut process = AppContainerProbe::spawn_package_suspended_with_execution_cwd(
        lease.execution_path(),
        r#"/d /v:off /s /c "nul-control.cmd""#.as_ref(),
        cwd.execution_path(),
        &execution_cwd,
        &super::super::super::version_probe::resolved_environment(&root).unwrap(),
        &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
        &[script],
    )
    .unwrap();
    let cancellation = Arc::new(AtomicBool::new(false));
    debugger.bind_cancellation(cancellation.clone());
    let (completed, completion) = mpsc::channel();
    // 对照本身挂起时仍给原调试线程留下清理时间，不直接依赖外层 Job 强杀。
    let watchdog = thread::spawn(move || {
        if completion.recv_timeout(Duration::from_secs(15)).is_err() {
            cancellation.store(true, Ordering::Release);
        }
    });
    let result = (|| -> io::Result<u32> {
        process.resume()?;
        debugger.verify_package_initial_image_in_container(&process)?;
        debugger.drain_package_in_container_until_exit(&process)?;
        process.exit_code()
    })();
    let _ = completed.send(());
    watchdog.join().unwrap();
    let termination = if result.is_err() {
        process
            .terminate_job()
            .and_then(|()| debugger.drain_terminated_package_in_container(&process))
    } else {
        Ok(())
    };
    drop(cwd);
    let receipt = root.join("appcontainer-cleanup-v1");
    let cleanup = termination.and_then(|()| process.write_cleanup_receipt(&receipt));
    drop(script_handle);
    cleanup.expect("NUL 对照无论成功或拒绝，都必须恢复 ACL、删除 profile 并清空 Job");
    assert_eq!(
        fs::read(&receipt).unwrap(),
        b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n"
    );
    assert_eq!(result.unwrap(), 0);
    let plain_status = fs::read_to_string(root.join("tmp/plain-status.txt")).unwrap();
    let regular_status = fs::read_to_string(root.join("tmp/regular-status.txt")).unwrap();
    let nul_status = fs::read_to_string(root.join("tmp/nul-status.txt")).unwrap();
    let plain = fs::read(root.join("tmp/plain.txt")).unwrap();
    let regular = fs::read(root.join("tmp/regular.txt")).unwrap();
    let regular_stderr = fs::read(root.join("tmp/regular-stderr.txt")).unwrap();
    let redirected = fs::read(root.join("tmp/redirected.txt")).unwrap();
    let after = fs::read(root.join("tmp/after.txt")).unwrap();
    let file_id = |name: &str| {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(root.join("tmp").join(name))
            .unwrap();
        inspect_handle(&file).unwrap().id
    };
    let distinct_regular_files = {
        let names = [
            "plain.txt",
            "regular.txt",
            "regular-stderr.txt",
            "redirected.txt",
        ];
        let ids = names.map(file_id);
        ids.iter()
            .enumerate()
            .all(|(index, id)| ids[..index].iter().all(|other| id != other))
    };
    let expected: &[u8] = b"builtin-control\r\n";
    eprintln!(
        "atomic_windows_nul_control={}",
        serde_json::json!({
            "plain_errorlevel": plain_status.trim().parse::<u32>().unwrap(),
            "regular_errorlevel": regular_status.trim().parse::<u32>().unwrap(),
            "nul_errorlevel": nul_status.trim().parse::<u32>().unwrap(),
            "plain_marker_matches": plain == expected,
            "regular_marker_matches": regular == expected,
            "regular_stderr_empty": regular_stderr.is_empty(),
            "distinct_regular_files": distinct_regular_files,
            "nul_marker_matches": redirected == expected,
            "after_marker_matches": after == expected,
            "cleanup_confirmed": true,
        })
    );
    assert_eq!(plain_status.trim(), "0");
    assert!(distinct_regular_files);
    assert_eq!(plain, expected);
    assert_eq!(regular_status.trim(), "0");
    assert_eq!(regular, expected);
    assert!(regular_stderr.is_empty());
    assert_eq!(after, expected);
    // 此断言失败只证明公共 shim 所需 NUL 重定向不可用，不把它当成 npm 挂起的唯一根因。
    assert_eq!(
        nul_status.trim(),
        "0",
        "同一 AppContainer 中的 NUL 重定向失败"
    );
    assert_eq!(redirected, expected);
    record_debug_native_exit(name);
}

const NUL_CREATEFILE_REPORT_ENV: &str = "INFINISHELL_WINDOWS_NUL_CREATEFILE_REPORT";
const NUL_CREATEFILE_ORDINARY_ENV: &str = "INFINISHELL_WINDOWS_NUL_CREATEFILE_ORDINARY";
const NUL_CREATEFILE_HELPER_ENV: &str = "INFINISHELL_WINDOWS_NUL_CREATEFILE_HELPER";
const NUL_CREATEFILE_HELPER_SHA256_ENV: &str = "INFINISHELL_WINDOWS_NUL_CREATEFILE_HELPER_SHA256";

fn parse_createfile_receipt(bytes: &[u8]) -> Option<serde_json::Value> {
    if bytes.len() != 120 {
        return None;
    }
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    if words[..6]
        != [
            0x4e55_4c31,
            1,
            GENERIC_WRITE.0,
            FILE_GENERIC_WRITE.0,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
            OPEN_EXISTING.0,
        ]
    {
        return None;
    }
    for result in words[6..].chunks_exact(4) {
        if result[0] > 1 || result[3] > 1 || result[0] == 0 && result[3] != 0 {
            return None;
        }
    }
    let result = |access_index, target_index| {
        let offset = 6 + (access_index * 3 + target_index) * 4;
        serde_json::json!({
            "opened": words[offset] == 1,
            "win32_error": words[offset + 1],
            "file_type": words[offset + 2],
            "close_confirmed": words[offset + 3] == 1,
        })
    };
    let compare = |access_index| {
        serde_json::json!({
            "desired_access": words[2 + access_index],
            "regular": result(access_index, 0),
            "nul": result(access_index, 1),
            "device_nul": result(access_index, 2),
        })
    };
    Some(serde_json::json!({
        "share_mode": words[4],
        "creation_disposition": words[5],
        "generic_write": compare(0usize),
        "file_generic_write": compare(1usize),
    }))
}

#[test]
#[ignore = "实际受限 AppContainer 令牌须直接打开 NUL 和同目录普通文件"]
fn npm_appcontainer_token_createfile_nul_vs_regular() {
    let name = "console_binding::npm_appcontainer_token_createfile_nul_vs_regular";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_LARGE_IMAGE_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for relative in ["home", "config", "cache", "data", "tmp"] {
        fs::create_dir(root.join(relative)).unwrap();
    }
    let source = PathBuf::from(std::env::var_os(NUL_CREATEFILE_HELPER_ENV).unwrap())
        .canonicalize()
        .unwrap();
    let expected_source = ExpectedFileIdentity::capture(&source).unwrap();
    assert_eq!(
        expected_source.sha256,
        std::env::var(NUL_CREATEFILE_HELPER_SHA256_ENV).unwrap(),
        "固定 Win32 helper 摘要不匹配"
    );
    let source_lease = prepare(&expected_source).unwrap();
    let mut source_file = source_lease.program.try_clone().unwrap();
    source_file.rewind().unwrap();
    let helper = root.join("createfile-helper.exe");
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&helper)
        .unwrap();
    assert_eq!(
        io::copy(&mut source_file, &mut destination).unwrap(),
        expected_source.size
    );
    destination.sync_all().unwrap();
    drop(destination);
    let expected_helper = ExpectedFileIdentity::capture(&helper).unwrap();
    assert_eq!(expected_helper.sha256, expected_source.sha256);
    assert_eq!(expected_helper.size, expected_source.size);
    let mut lease = prepare(&expected_helper).unwrap();
    lease
        .set_package_images(vec![expected_helper.clone()])
        .unwrap();
    let cwd = prepare_directory(&AtomicDirectoryIdentity::capture(&root).unwrap()).unwrap();
    let execution_cwd = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_eq!(execution_cwd.canonicalize().unwrap(), root);
    let report = root.join("tmp/createfile-actual.json");
    let ordinary = root.join("tmp/ordinary.txt");
    let mut environment = super::super::super::version_probe::resolved_environment(&root).unwrap();
    environment.push((
        NUL_CREATEFILE_REPORT_ENV.into(),
        report.clone().into_os_string(),
    ));
    environment.push((
        NUL_CREATEFILE_ORDINARY_ENV.into(),
        ordinary.clone().into_os_string(),
    ));
    let mut debugger = lease.prepare_image_debug_session().unwrap();
    let mut process = AppContainerProbe::spawn_package_suspended_with_execution_cwd(
        lease.execution_path(),
        "--createfile-control".as_ref(),
        cwd.execution_path(),
        &execution_cwd,
        &environment,
        &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
        &[helper],
    )
    .unwrap();
    // 授权后创建，令普通文件继承私有 tmp 的 AppContainer ACE；两个目标均以 OPEN_EXISTING 打开。
    fs::write(&ordinary, b"ordinary-control").unwrap();
    let cancellation = Arc::new(AtomicBool::new(false));
    debugger.bind_cancellation(cancellation.clone());
    let (completed, completion) = mpsc::channel();
    let watchdog = thread::spawn(move || {
        if completion.recv_timeout(Duration::from_secs(60)).is_err() {
            cancellation.store(true, Ordering::Release);
        }
    });
    let result = (|| -> io::Result<u32> {
        process.resume()?;
        debugger.verify_package_initial_image_in_container(&process)?;
        debugger.drain_package_in_container_until_exit(&process)?;
        process.exit_code()
    })();
    let _ = completed.send(());
    watchdog.join().unwrap();
    let termination = if result.is_err() {
        process
            .terminate_job()
            .and_then(|()| debugger.drain_terminated_package_in_container(&process))
    } else {
        Ok(())
    };
    drop(cwd);
    let receipt = root.join("appcontainer-cleanup-v1");
    let cleanup = termination.and_then(|()| process.write_cleanup_receipt(&receipt));
    let receipt_matches = fs::read(&receipt)
        .is_ok_and(|bytes| bytes == b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n");
    let report_bytes = fs::read(&report).ok();
    let observation = report_bytes.as_deref().and_then(parse_createfile_receipt);
    eprintln!(
        "atomic_windows_actual_createfile_control={}",
        serde_json::json!({
            "helper_sha256": expected_helper.sha256,
            "native_exit_code": result.as_ref().ok().copied(),
            "failure_kind": result.as_ref().err().map(|failure| format!("{:?}", failure.kind())),
            "os_code": result.as_ref().err().and_then(io::Error::raw_os_error),
            "helper_report_present": report_bytes.is_some(),
            "helper_receipt_valid": observation.is_some(),
            "observation": observation,
            "cleanup_confirmed": cleanup.is_ok(),
            "receipt_matches": receipt_matches,
        })
    );
    cleanup.expect("CreateFile 对照必须恢复 ACL、删除 profile 并清空 Job");
    assert!(receipt_matches);
    assert_eq!(result.unwrap(), 0);
    let observation = observation.expect("实际令牌未写入 CreateFile 收据");
    for (name, mask) in [
        ("generic_write", GENERIC_WRITE.0),
        ("file_generic_write", FILE_GENERIC_WRITE.0),
    ] {
        assert_eq!(observation[name]["desired_access"], mask);
        assert_eq!(observation[name]["regular"]["opened"], true);
        assert_eq!(observation[name]["regular"]["file_type"], FILE_TYPE_DISK.0);
        assert_eq!(observation[name]["regular"]["close_confirmed"], true);
        for target in ["nul", "device_nul"] {
            if observation[name][target]["opened"] == true {
                assert_eq!(observation[name][target]["close_confirmed"], true);
            }
        }
    }
    record_debug_native_exit(name);
}

#[test]
#[ignore = "Windows npm 的 TITLE 后续派生须在真实 AppContainer 中显式验证"]
fn npm_cmd_title_runs_bound_child_and_confirms_cleanup() {
    run_cmd_tail_control(
        "console_binding::npm_cmd_title_runs_bound_child_and_confirms_cleanup",
        false,
    );
}

#[test]
#[ignore = "Windows npm 的 GOTO/NUL/TITLE 后续派生须在真实 AppContainer 中显式验证"]
fn npm_cmd_goto_nul_title_runs_bound_child_and_confirms_cleanup() {
    run_cmd_tail_control(
        "console_binding::npm_cmd_goto_nul_title_runs_bound_child_and_confirms_cleanup",
        true,
    );
}

fn run_cmd_tail_control(name: &str, with_goto: bool) {
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for relative in ["home", "config", "cache", "data", "tmp"] {
        fs::create_dir(root.join(relative)).unwrap();
    }
    let system = prepare_system_directory().unwrap();
    let program = final_path_from_handle(&system.file)
        .unwrap()
        .join("cmd.exe");
    let child = root.join("bound-child.exe");
    fs::copy(&program, &child).unwrap();
    let script = root.join("tail-control.cmd");
    // 单独的诊断脚本只复现公共入口末行控制流；绑定子映像是 CMD，不替代真实 npm 验收。
    let tail = if with_goto {
        r#"endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%" /d /v:off /s /c "echo control-child>tmp\child.txt""#
    } else {
        r#"endLocal & title %COMSPEC% & >tmp\title-after.txt echo control-title& "%_prog%" /d /v:off /s /c "echo control-child>tmp\child.txt""#
    };
    let contents = format!(
        "@echo off\r\nsetlocal\r\nset \"_prog=.\\bound-child.exe\"\r\n>tmp\\before.txt echo control-before\r\n{tail}\r\n"
    );
    fs::write(&script, contents).unwrap();
    let script_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&script)
        .unwrap();
    let mut lease = prepare(&ExpectedFileIdentity::capture(&program).unwrap()).unwrap();
    lease
        .set_package_images(vec![ExpectedFileIdentity::capture(&child).unwrap()])
        .unwrap();
    lease.enable_npm_console_host().unwrap();
    let cwd = prepare_directory(&AtomicDirectoryIdentity::capture(&root).unwrap()).unwrap();
    let execution_cwd = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_eq!(execution_cwd.canonicalize().unwrap(), root);
    let mut environment = super::super::super::version_probe::resolved_environment(&root).unwrap();
    environment.push(("COMSPEC".into(), program.into_os_string()));
    let mut debugger = lease.prepare_image_debug_session().unwrap();
    let mut process = AppContainerProbe::spawn_package_suspended_with_execution_cwd(
        lease.execution_path(),
        r#"/d /v:off /s /c "tail-control.cmd""#.as_ref(),
        cwd.execution_path(),
        &execution_cwd,
        &environment,
        &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
        &[script, child],
    )
    .unwrap();
    let cancellation = Arc::new(AtomicBool::new(false));
    debugger.bind_cancellation(cancellation.clone());
    let (completed, completion) = mpsc::channel();
    // 看门狗只发取消；全部调试事件与清理仍由创建进程的同一线程处理。
    let watchdog = thread::spawn(move || {
        if completion.recv_timeout(Duration::from_secs(15)).is_err() {
            cancellation.store(true, Ordering::Release);
        }
    });
    let result = (|| -> io::Result<u32> {
        process.resume()?;
        debugger.verify_package_initial_image_in_container(&process)?;
        debugger.drain_package_in_container_until_exit(&process)?;
        process.exit_code()
    })();
    let _ = completed.send(());
    watchdog.join().unwrap();
    let termination = if result.is_err() {
        process
            .terminate_job()
            .and_then(|()| debugger.drain_terminated_package_in_container(&process))
    } else {
        Ok(())
    };
    drop(cwd);
    let receipt = root.join("appcontainer-cleanup-v1");
    let cleanup = termination.and_then(|()| process.write_cleanup_receipt(&receipt));
    drop(script_handle);
    cleanup.expect("CMD 末行对照无论成功或失败，都必须恢复 ACL、删除 profile 并清空 Job");
    assert_eq!(
        fs::read(&receipt).unwrap(),
        b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n"
    );
    // 先记录缺失标记或原生失败，再断言；NUL 对照已证明 errorlevel 可能保持为零。
    let before = fs::read(root.join("tmp/before.txt"));
    let child = fs::read(root.join("tmp/child.txt"));
    let before_matches = before
        .as_deref()
        .is_ok_and(|bytes| bytes == b"control-before\r\n");
    let child_matches = child
        .as_deref()
        .is_ok_and(|bytes| bytes == b"control-child\r\n");
    let title_matches = (!with_goto).then(|| {
        fs::read(root.join("tmp/title-after.txt")).is_ok_and(|bytes| bytes == b"control-title\r\n")
    });
    let native_exit_code = result.as_ref().ok().copied();
    let failure_kind = result
        .as_ref()
        .err()
        .map(|failure| format!("{:?}", failure.kind()));
    let os_code = result.as_ref().err().and_then(io::Error::raw_os_error);
    let case = if with_goto { "goto_nul_title" } else { "title" };
    eprintln!(
        "atomic_windows_cmd_tail_control={}",
        serde_json::json!({
            "case": case,
            "native_exit_code": native_exit_code,
            "failure_kind": failure_kind,
            "os_code": os_code,
            "before_marker_matches": before_matches,
            "title_marker_matches": title_matches,
            "child_marker_matches": child_matches,
            "cleanup_confirmed": true,
        })
    );
    assert_eq!(result.unwrap(), 0);
    assert_eq!(before.unwrap(), b"control-before\r\n");
    if let Some(title_matches) = title_matches {
        assert!(title_matches, "TITLE 返回后必须写入精确标记");
    }
    assert_eq!(child.unwrap(), b"control-child\r\n");
    record_debug_native_exit(name);
}

#[test]
#[ignore = "完整 npm CMD 选路须在真实 AppContainer 和 stdin EOF 下显式验证"]
fn npm_cmd_full_shim_explicit_node_with_stdin_eof() {
    run_cmd_full_shim_control(
        "console_binding::npm_cmd_full_shim_explicit_node_with_stdin_eof",
        true,
        false,
    );
}

#[test]
#[ignore = "完整 npm CMD 选路须在真实 AppContainer 和开放 stdin 下显式验证"]
fn npm_cmd_full_shim_explicit_node_with_stdin_open() {
    run_cmd_full_shim_control(
        "console_binding::npm_cmd_full_shim_explicit_node_with_stdin_open",
        true,
        true,
    );
}

#[test]
#[ignore = "完整 npm CMD 的 PATH 选路须在真实 AppContainer 和 stdin EOF 下显式验证"]
fn npm_cmd_full_shim_path_node_with_stdin_eof() {
    run_cmd_full_shim_control(
        "console_binding::npm_cmd_full_shim_path_node_with_stdin_eof",
        false,
        false,
    );
}

#[test]
#[ignore = "完整 npm CMD 的 PATH 选路须在真实 AppContainer 和开放 stdin 下显式验证"]
fn npm_cmd_full_shim_path_node_with_stdin_open() {
    run_cmd_full_shim_control(
        "console_binding::npm_cmd_full_shim_path_node_with_stdin_open",
        false,
        true,
    );
}

fn run_cmd_full_shim_control(name: &str, explicit_node: bool, keep_stdin_open: bool) {
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job_with_stdin(name, DEBUG_DRIVER_TIMEOUT, keep_stdin_open);
        return;
    }
    await_debug_driver_authorization();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for relative in ["home", "config", "cache", "data", "tmp", "runtime"] {
        fs::create_dir(root.join(relative)).unwrap();
    }
    let system = prepare_system_directory().unwrap();
    let program = final_path_from_handle(&system.file)
        .unwrap()
        .join("cmd.exe");
    let runtime = root.join("runtime");
    let child = runtime.join("node.exe");
    fs::copy(&program, &child).unwrap();
    let script = root.join("full-shim-control.cmd");
    let lookup = if explicit_node {
        r"%dp0%\runtime\node.exe"
    } else {
        r"%dp0%\node.exe"
    };
    // 只复现完整 shim 的选路控制流；显式分支指向独立 runtime 中的同一绑定 CMD。
    // 子参数故意使用 CMD 写标记，区别于官方 codex.js，不能视为真实 Node/npm 验收。
    let contents = format!(
        concat!(
            "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n",
            ":start\r\nSETLOCAL\r\nCALL :find_dp0\r\n>tmp\\before.txt echo routing-before\r\n",
            "IF EXIST \"{lookup}\" (\r\n  SET \"_prog={lookup}\"\r\n",
            "  >tmp\\branch.txt echo explicit\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n",
            "  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n  >tmp\\branch.txt echo path\r\n)\r\n",
            "endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\" ",
            "/d /v:off /s /c \"echo routing-child>tmp\\child.txt\"\r\n"
        ),
        lookup = lookup,
    );
    fs::write(&script, contents).unwrap();
    let script_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&script)
        .unwrap();
    let mut lease = prepare(&ExpectedFileIdentity::capture(&program).unwrap()).unwrap();
    lease
        .set_package_images(vec![ExpectedFileIdentity::capture(&child).unwrap()])
        .unwrap();
    lease.enable_npm_console_host().unwrap();
    let cwd = prepare_directory(&AtomicDirectoryIdentity::capture(&root).unwrap()).unwrap();
    let execution_cwd = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_eq!(execution_cwd.canonicalize().unwrap(), root);
    let mut environment = super::super::super::version_probe::resolved_environment(&root).unwrap();
    let system_path = environment
        .iter_mut()
        .find(|(name, _)| name == "PATH")
        .unwrap();
    system_path.1 = std::env::join_paths(
        std::iter::once(execution_cwd.join("runtime")).chain(std::env::split_paths(&system_path.1)),
    )
    .unwrap();
    environment.push(("CODEX_HOME".into(), root.join("config").into_os_string()));
    environment.push(("NODE_DISABLE_COMPILE_CACHE".into(), "1".into()));
    environment.push(("COMSPEC".into(), program.into_os_string()));
    let mut debugger = lease.prepare_image_debug_session().unwrap();
    let mut process = AppContainerProbe::spawn_package_suspended_with_execution_cwd(
        lease.execution_path(),
        r#"/d /v:off /s /c "full-shim-control.cmd""#.as_ref(),
        cwd.execution_path(),
        &execution_cwd,
        &environment,
        &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
        &[runtime, script, child],
    )
    .unwrap();
    let cancellation = Arc::new(AtomicBool::new(false));
    debugger.bind_cancellation(cancellation.clone());
    let (completed, completion) = mpsc::channel();
    let watchdog = thread::spawn(move || {
        if completion.recv_timeout(Duration::from_secs(15)).is_err() {
            cancellation.store(true, Ordering::Release);
        }
    });
    let result = (|| -> io::Result<u32> {
        process.resume()?;
        debugger.verify_package_initial_image_in_container(&process)?;
        debugger.drain_package_in_container_until_exit(&process)?;
        process.exit_code()
    })();
    let _ = completed.send(());
    let watchdog_completed = watchdog.join().is_ok();
    let termination = if result.is_err() {
        debugger.record_package_termination_request();
        process
            .terminate_job()
            .and_then(|()| debugger.drain_terminated_package_in_container(&process))
    } else {
        Ok(())
    };
    drop(cwd);
    let receipt = root.join("appcontainer-cleanup-v1");
    let cleanup = termination.and_then(|()| process.write_cleanup_receipt(&receipt));
    drop(script_handle);
    let before = fs::read(root.join("tmp/before.txt"));
    let branch = fs::read(root.join("tmp/branch.txt"));
    let child = fs::read(root.join("tmp/child.txt"));
    let expected_branch: &[u8] = if explicit_node {
        b"explicit\r\n"
    } else {
        b"path\r\n"
    };
    let before_matches = before
        .as_deref()
        .is_ok_and(|bytes| bytes == b"routing-before\r\n");
    let branch_matches = branch
        .as_deref()
        .is_ok_and(|bytes| bytes == expected_branch);
    let child_matches = child
        .as_deref()
        .is_ok_and(|bytes| bytes == b"routing-child\r\n");
    let receipt_matches = fs::read(&receipt)
        .is_ok_and(|bytes| bytes == b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n");
    let case = match (explicit_node, keep_stdin_open) {
        (true, false) => "explicit_stdin_eof",
        (true, true) => "explicit_stdin_open",
        (false, false) => "path_stdin_eof",
        (false, true) => "path_stdin_open",
    };
    // 原生失败和首次清理结果均先落日志；断言不能遮蔽入口停在哪个阶段。
    eprintln!(
        "atomic_windows_cmd_full_shim_control={}",
        serde_json::json!({
            "case": case,
            "child_image_kind": "bound_cmd_copy",
            "native_exit_code": result.as_ref().ok().copied(),
            "failure_kind": result.as_ref().err().map(|failure| format!("{:?}", failure.kind())),
            "os_code": result.as_ref().err().and_then(io::Error::raw_os_error),
            "before_marker_matches": before_matches,
            "branch_marker_matches": branch_matches,
            "child_marker_matches": child_matches,
            "cleanup_confirmed": cleanup.is_ok(),
            "cleanup_failure_kind": cleanup.as_ref().err().map(|failure| format!("{:?}", failure.kind())),
            "cleanup_os_code": cleanup.as_ref().err().and_then(io::Error::raw_os_error),
            "receipt_matches": receipt_matches,
            "watchdog_completed": watchdog_completed,
        })
    );
    cleanup.expect("完整 CMD 选路对照必须恢复 ACL、删除 profile 并清空 Job");
    assert!(watchdog_completed);
    assert_eq!(result.unwrap(), 0);
    assert_eq!(before.unwrap(), b"routing-before\r\n");
    assert_eq!(branch.unwrap(), expected_branch);
    assert_eq!(child.unwrap(), b"routing-child\r\n");
    assert!(receipt_matches);
    record_debug_native_exit(name);
}

#[test]
#[ignore = "CMD 基线须在 npm 验收运行器的管道与磁盘 stderr 布置中显式验证"]
fn npm_cmd_full_shim_bound_cmd_with_worker_stdio() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_cmd_full_shim_bound_cmd_with_worker_stdio",
        WorkerStdioControl::BoundCmd,
    );
}

#[test]
#[ignore = "固定 Node 20.9.0 须在 npm 验收标准流与 AppContainer 边界中显式验证"]
fn npm_cmd_full_shim_fixed_node_with_worker_stdio() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_cmd_full_shim_fixed_node_with_worker_stdio",
        WorkerStdioControl::NodeChild,
    );
}

#[test]
#[ignore = "固定 Node 根进程须与 CMD 子进程保持相同验收标准流和 AppContainer 边界"]
fn npm_fixed_node_root_with_worker_stdio() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_fixed_node_root_with_worker_stdio",
        WorkerStdioControl::NodeRoot,
    );
}

#[test]
#[ignore = "隐藏独立控制台的 CMD 基线须保持相同 Job 与 AppContainer 核验"]
fn npm_cmd_full_shim_bound_cmd_with_hidden_console() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_cmd_full_shim_bound_cmd_with_hidden_console",
        WorkerStdioControl::HiddenBoundCmd,
    );
}

#[test]
#[ignore = "隐藏独立控制台的 Node 子进程须显式核验版本、映像及完整清理"]
fn npm_cmd_full_shim_fixed_node_with_hidden_console() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_cmd_full_shim_fixed_node_with_hidden_console",
        WorkerStdioControl::HiddenNodeChild,
    );
}

#[test]
#[ignore = "隐藏独立控制台的 Node 根进程须与原控制台模式独立对照"]
fn npm_fixed_node_root_with_hidden_console() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_fixed_node_root_with_hidden_console",
        WorkerStdioControl::HiddenNodeRoot,
    );
}

#[test]
#[ignore = "私有窗口站和桌面仅供固定 Node 根进程诊断，必须核验新对象并确认完整清理"]
fn npm_fixed_node_root_with_private_desktop() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_fixed_node_root_with_private_desktop",
        WorkerStdioControl::NodeRootPrivateDesktop,
    );
}

#[test]
#[ignore = "现有非交互窗口站内的私有桌面须由固定 Node 根进程和完整清理原生对照"]
fn npm_fixed_node_root_with_existing_station_desktop() {
    run_cmd_worker_stdio_control(
        "console_binding::npm_fixed_node_root_with_existing_station_desktop",
        WorkerStdioControl::NodeRootExistingStationDesktop,
    );
}

#[derive(Clone, Copy)]
enum WorkerStdioControl {
    BoundCmd,
    NodeChild,
    NodeRoot,
    HiddenBoundCmd,
    HiddenNodeChild,
    HiddenNodeRoot,
    NodeRootPrivateDesktop,
    NodeRootExistingStationDesktop,
}

// 只回显固定 API 阶段和数值；不输出错误正文、对象名称、SID 或路径。
fn private_desktop_failure_summary(failure: &io::Error) -> serde_json::Value {
    let description = failure.to_string();
    let api = description
        .strip_prefix("私有桌面 ")
        .and_then(|text| text.split_once(" 失败：HRESULT=0x"))
        .and_then(|(stage, code)| {
            let stage = [
                "sid_string",
                "open_token",
                "token_user",
                "build_sd",
                "read_sd",
                "read_sd_control",
                "read_object_flags",
                "original_station",
                "original_desktop",
                "existing_station",
                "read_existing_station_name",
                "read_existing_station_flags",
                "read_existing_desktop_name",
                "read_current_desktop",
                "verify_existing_station",
                "create_station",
                "select_station",
                "create_desktop",
                "create_existing_station_desktop",
                "restore_station",
                "restore_desktop",
                "verify_restored_station",
                "verify_restored_desktop",
                "close_desktop",
                "close_station",
            ]
            .into_iter()
            .find(|allowed| *allowed == stage)?;
            let code = code.split('；').next()?;
            (code.len() == 8 && code.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(code, 16).ok())
                .flatten()
                .map(|code| (stage, code))
        });
    // 只认创建站错误后的第一个固定字段；清理正文或任意 JSON 字段都不能进入摘要。
    let administrator_membership = api
        .filter(|(stage, _)| *stage == "create_station")
        .and_then(|_| description.split('；').nth(1))
        .and_then(|field| field.strip_prefix("administrator_membership="))
        .filter(|json| json.len() <= 128)
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .filter(|value| {
            value.as_object().is_some_and(|fields| {
                fields.len() == 3
                    && fields
                        .keys()
                        .all(|name| matches!(name.as_str(), "status" | "member" | "hresult"))
                    && ((value["status"] == "ok"
                        && value["member"].is_boolean()
                        && value["hresult"].is_null())
                        || (value["status"] == "query_error"
                            && value["member"].is_null()
                            && value["hresult"]
                                .as_u64()
                                .is_some_and(|code| code <= u32::MAX as u64)))
            })
        });
    serde_json::json!({
        "kind": format!("{:?}", failure.kind()),
        "administrator_membership": administrator_membership,
        "os_code": failure.raw_os_error(),
        "private_object_api_stage": api.map(|(stage, _)| stage),
        "hresult": api.map(|(_, code)| code).or_else(|| {
            failure.get_ref()
                .and_then(|error| error.downcast_ref::<windows::core::Error>())
                .map(|error| error.code().0 as u32)
        }),
    })
}

#[test]
fn private_desktop_failure_keeps_only_fixed_stage_and_numeric_hresult() {
    let failure = io::Error::other("私有桌面 create_station 失败：HRESULT=0x80070005");
    assert_eq!(
        private_desktop_failure_summary(&failure),
        serde_json::json!({
            "kind": "Other",
            "administrator_membership": null,
            "os_code": null,
            "private_object_api_stage": "create_station",
            "hresult": 2147942405_u32,
        })
    );

    let native_failure = io::Error::other(windows::core::Error::from_hresult(
        windows::core::HRESULT(-2147024891),
    ));
    assert_eq!(
        private_desktop_failure_summary(&native_failure),
        serde_json::json!({
            "kind": "Other",
            "administrator_membership": null,
            "os_code": null,
            "private_object_api_stage": null,
            "hresult": 2147942405_u32,
        })
    );

    let existing_station =
        io::Error::other("私有桌面 create_existing_station_desktop 失败：HRESULT=0x80070005");
    assert_eq!(
        private_desktop_failure_summary(&existing_station)["private_object_api_stage"],
        "create_existing_station_desktop"
    );
}

#[test]
fn private_desktop_failure_does_not_echo_sensitive_or_malformed_details() {
    // 均为虚构输入；非白名单阶段或混入正文的数值不能进入安全证据。
    let unknown_stage = io::Error::other(
        r"私有桌面 S-1-15-2-777 C:\private-fixture\manifest 失败：HRESULT=0x80070005",
    );
    assert_eq!(
        private_desktop_failure_summary(&unknown_stage),
        serde_json::json!({
            "kind": "Other",
            "administrator_membership": null,
            "os_code": null,
            "private_object_api_stage": null,
            "hresult": null,
        })
    );
    let malformed = io::Error::other(
        r"私有桌面 create_station 失败：HRESULT=0x80070005 C:\private-fixture\S-1-15-2-777",
    );
    assert_eq!(
        private_desktop_failure_summary(&malformed),
        serde_json::json!({
            "kind": "Other",
            "administrator_membership": null,
            "os_code": null,
            "private_object_api_stage": null,
            "hresult": null,
        })
    );
}

#[test]
fn private_desktop_failure_excludes_appended_cleanup_body() {
    let failure = io::Error::other(
        r#"私有桌面 create_station 失败：HRESULT=0x80070005；私有对象清理失败：S-1-15-2-777 C:\private-fixture\manifest；administrator_membership={"status":"ok","member":true,"hresult":null}"#,
    );
    assert_eq!(
        private_desktop_failure_summary(&failure),
        serde_json::json!({
            "kind": "Other",
            "administrator_membership": null,
            "os_code": null,
            "private_object_api_stage": "create_station",
            "hresult": 2147942405_u32,
        })
    );
}

#[test]
fn private_desktop_failure_preserves_administrator_membership_tristate() {
    let member = io::Error::other(
        r#"私有桌面 create_station 失败：HRESULT=0x80070005；administrator_membership={"status":"ok","member":true,"hresult":null}"#,
    );
    assert_eq!(
        private_desktop_failure_summary(&member)["administrator_membership"],
        serde_json::json!({"status": "ok", "member": true, "hresult": null})
    );
    let not_member = io::Error::other(
        r#"私有桌面 create_station 失败：HRESULT=0x80070005；administrator_membership={"status":"ok","member":false,"hresult":null}"#,
    );
    assert_eq!(
        private_desktop_failure_summary(&not_member)["administrator_membership"],
        serde_json::json!({"status": "ok", "member": false, "hresult": null})
    );
    let query_failed = io::Error::other(
        r#"私有桌面 create_station 失败：HRESULT=0x80070005；administrator_membership={"status":"query_error","member":null,"hresult":2147942487}"#,
    );
    assert_eq!(
        private_desktop_failure_summary(&query_failed)["administrator_membership"],
        serde_json::json!({"status": "query_error", "member": null, "hresult": 2147942487_u32})
    );
    let untrusted = io::Error::other(
        r#"私有桌面 create_station 失败：HRESULT=0x80070005；administrator_membership={"status":"ok","member":true,"path":"C:\\private-fixture\\S-1-15-2-777"}"#,
    );
    assert_eq!(
        private_desktop_failure_summary(&untrusted),
        serde_json::json!({
            "kind": "Other",
            "administrator_membership": null,
            "os_code": null,
            "private_object_api_stage": "create_station",
            "hresult": 2147942405_u32,
        })
    );
}

fn run_cmd_worker_stdio_control(name: &str, mode: WorkerStdioControl) {
    let real_node = !matches!(
        mode,
        WorkerStdioControl::BoundCmd | WorkerStdioControl::HiddenBoundCmd
    );
    let shim_executed = !matches!(
        mode,
        WorkerStdioControl::NodeRoot
            | WorkerStdioControl::HiddenNodeRoot
            | WorkerStdioControl::NodeRootPrivateDesktop
            | WorkerStdioControl::NodeRootExistingStationDesktop
    );
    let private_desktop = matches!(
        mode,
        WorkerStdioControl::NodeRootPrivateDesktop
            | WorkerStdioControl::NodeRootExistingStationDesktop
    );
    let existing_station_desktop =
        matches!(mode, WorkerStdioControl::NodeRootExistingStationDesktop);
    let hidden_console = matches!(
        mode,
        WorkerStdioControl::HiddenBoundCmd
            | WorkerStdioControl::HiddenNodeChild
            | WorkerStdioControl::HiddenNodeRoot
    );
    let (case, expected_line, child_arguments) = match mode {
        WorkerStdioControl::BoundCmd => (
            "bound_cmd",
            "routing-child",
            r#"/d /v:off /s /c "echo routing-child""#,
        ),
        WorkerStdioControl::NodeChild => ("node_20_9_0", "v20.9.0", "--version"),
        WorkerStdioControl::NodeRoot => ("node_20_9_0_root", "v20.9.0", "--version"),
        WorkerStdioControl::HiddenBoundCmd => (
            "bound_cmd_hidden_console",
            "routing-child",
            r#"/d /v:off /s /c "echo routing-child""#,
        ),
        WorkerStdioControl::HiddenNodeChild => {
            ("node_20_9_0_hidden_console", "v20.9.0", "--version")
        }
        WorkerStdioControl::HiddenNodeRoot => {
            ("node_20_9_0_root_hidden_console", "v20.9.0", "--version")
        }
        WorkerStdioControl::NodeRootPrivateDesktop => {
            ("node_20_9_0_root_private_desktop", "v20.9.0", "--version")
        }
        WorkerStdioControl::NodeRootExistingStationDesktop => (
            "node_20_9_0_root_existing_station_desktop",
            "v20.9.0",
            "--version",
        ),
    };
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_stdio_fixture_in_strict_job(name, case, expected_line);
        return;
    }
    await_debug_driver_authorization();
    // 此处对齐 cfg(test) 验收运行器；非测试产品的 stderr 仍是 Stdio::null()。
    let [stdin_kind, stdout_kind, stderr_kind] =
        [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|stream| unsafe { GetStdHandle(stream) }.map_or("unavailable", debug_stdio_kind));
    eprintln!(
        "atomic_windows_cmd_stdio_handles={}",
        serde_json::json!({
            "case": case,
            "console_mode": if hidden_console { "hidden_new_console" } else { "no_window" },
            "stdio_scope": "npm_acceptance_fixture",
            "stdin_kind": stdin_kind,
            "stdout_kind": stdout_kind,
            "stderr_kind": stderr_kind,
        })
    );
    assert_eq!(
        (stdin_kind, stdout_kind, stderr_kind),
        ("pipe", "pipe", "disk")
    );
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for relative in ["home", "config", "cache", "data", "tmp", "runtime"] {
        fs::create_dir(root.join(relative)).unwrap();
    }
    let system = prepare_system_directory().unwrap();
    let program = final_path_from_handle(&system.file)
        .unwrap()
        .join("cmd.exe");
    let source = if real_node {
        PathBuf::from(std::env::var_os(NODE_CONTROL_ENV).expect("必须提供固定 Node 输入"))
            .canonicalize()
            .unwrap()
    } else {
        program.clone()
    };
    let expected_source = ExpectedFileIdentity::capture(&source).unwrap();
    if real_node {
        let digest = std::env::var(NODE_CONTROL_SHA256_ENV)
            .expect("必须提供固定 Node 的 SHA256")
            .to_ascii_lowercase();
        assert!(digest.len() == 64 && digest.bytes().all(|value| value.is_ascii_hexdigit()));
        assert_eq!(expected_source.sha256, digest, "固定 Node 输入摘要不匹配");
    }
    // 原始 Node 始终持读租约；只从该句柄复制并核对摘要，执行仅使用私有副本。
    let source_lease = prepare(&expected_source).unwrap();
    let mut source_file = source_lease.program.try_clone().unwrap();
    source_file.rewind().unwrap();
    let runtime = root.join("runtime");
    let child = runtime.join("node.exe");
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&child)
        .unwrap();
    assert_eq!(
        io::copy(&mut source_file, &mut destination).unwrap(),
        expected_source.size
    );
    destination.sync_all().unwrap();
    drop(destination);
    let expected_child = ExpectedFileIdentity::capture(&child).unwrap();
    assert_eq!(expected_child.sha256, expected_source.sha256);
    assert_eq!(expected_child.size, expected_source.size);
    let script = root.join("worker-stdio-control.cmd");
    // 诊断脚本保留官方选路控制流；参数为 CMD 标记或 Node --version，不运行 codex.js。
    let contents = format!(
        concat!(
            "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n",
            ":start\r\nSETLOCAL\r\nCALL :find_dp0\r\n>tmp\\before.txt echo routing-before\r\n",
            "IF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n",
            "  >tmp\\branch.txt echo explicit\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n",
            "  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n  >tmp\\branch.txt echo path\r\n)\r\n",
            "endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\" {child_arguments}\r\n"
        ),
        child_arguments = child_arguments,
    );
    fs::write(&script, contents).unwrap();
    let script_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&script)
        .unwrap();
    // 根 Node 对照保留相同脚本、目录和只读授权，只改变启动拓扑；脚本本身不执行。
    let expected_program = if shim_executed {
        ExpectedFileIdentity::capture(&program).unwrap()
    } else {
        expected_child.clone()
    };
    let mut lease = prepare(&expected_program).unwrap();
    lease.set_package_images(vec![expected_child]).unwrap();
    lease.enable_npm_console_host().unwrap();
    let cwd = prepare_directory(&AtomicDirectoryIdentity::capture(&root).unwrap()).unwrap();
    let execution_cwd = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_eq!(execution_cwd.canonicalize().unwrap(), root);
    let mut environment = super::super::super::version_probe::resolved_environment(&root).unwrap();
    let system_path = environment
        .iter_mut()
        .find(|(name, _)| name == "PATH")
        .unwrap();
    system_path.1 = std::env::join_paths(
        std::iter::once(execution_cwd.join("runtime")).chain(std::env::split_paths(&system_path.1)),
    )
    .unwrap();
    environment.push(("CODEX_HOME".into(), root.join("config").into_os_string()));
    environment.push(("NODE_DISABLE_COMPILE_CACHE".into(), "1".into()));
    environment.push(("COMSPEC".into(), program.into_os_string()));
    let mut debugger = lease.prepare_image_debug_session().unwrap();
    debugger.loader_trace = Some(loader_tests::LoaderTrace::default());
    let arguments = if shim_executed {
        r#"/d /v:off /s /c "worker-stdio-control.cmd""#
    } else {
        "--version"
    };
    // 私有桌面只在独立 driver 中创建；此入口固定 --version 与 NoWindow。
    let mut process = if private_desktop {
        let spawn = if existing_station_desktop {
            AppContainerProbe::spawn_package_suspended_with_existing_station_desktop
        } else {
            AppContainerProbe::spawn_package_suspended_with_private_desktop
        };
        match spawn(
            lease.execution_path(),
            cwd.execution_path(),
            &execution_cwd,
            &environment,
            &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
            &[runtime, script, child],
        ) {
            Ok(process) => process,
            Err(failure) => {
                // 没有返回可核验的进程，Drop 的尽力清理不能替代完整清理证明。
                eprintln!(
                    "atomic_windows_private_desktop_control={}",
                    serde_json::json!({
                        "case": case,
                        "phase": if existing_station_desktop { "spawn_existing_station_desktop_failed" } else { "spawn_private_desktop_failed" },
                        "environment_control_established": false,
                        "private_objects_verified": false,
                        "private_objects_closed": null,
                        "cleanup_confirmed": null,
                        "native_exit_code": null,
                        "failure": private_desktop_failure_summary(&failure),
                    })
                );
                panic!("私有桌面对照环境未建立，创建失败证据已保留");
            }
        }
    } else {
        // 新模式仅由本对照显式选择；正式 npm 调用及其他来源继续使用原默认入口。
        let spawn = if hidden_console {
            AppContainerProbe::spawn_package_suspended_with_hidden_console
        } else {
            AppContainerProbe::spawn_package_suspended_with_execution_cwd
        };
        spawn(
            lease.execution_path(),
            arguments.as_ref(),
            cwd.execution_path(),
            &execution_cwd,
            &environment,
            &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
            &[runtime, script, child],
        )
        .unwrap()
    };
    let mut private_objects_verified = false;
    let mut private_desktop_phase = "resume";
    let cancellation = Arc::new(AtomicBool::new(false));
    debugger.bind_cancellation(cancellation.clone());
    let (completed, completion) = mpsc::channel();
    let watchdog = thread::spawn(move || {
        if completion.recv_timeout(Duration::from_secs(30)).is_err() {
            cancellation.store(true, Ordering::Release);
        }
    });
    let result = (|| -> io::Result<u32> {
        process.resume()?;
        if private_desktop {
            private_desktop_phase = "root_initial_image_verification";
        }
        debugger.verify_package_initial_image_in_container(&process)?;
        if private_desktop {
            private_desktop_phase = "private_object_verification";
            process.verify_private_desktop()?;
            private_objects_verified = true;
            private_desktop_phase = "node_execution";
        }
        debugger.drain_package_in_container_until_exit(&process)?;
        process.exit_code()
    })();
    let _ = completed.send(());
    let watchdog_completed = watchdog.join().is_ok();
    let termination = if result.is_err() {
        debugger.record_package_termination_request();
        process
            .terminate_job()
            .and_then(|()| debugger.drain_terminated_package_in_container(&process))
    } else {
        Ok(())
    };
    drop(cwd);
    let receipt = root.join("appcontainer-cleanup-v1");
    let cleanup = termination.and_then(|()| process.write_cleanup_receipt(&receipt));
    drop(script_handle);
    let before = fs::read(root.join("tmp/before.txt"));
    let branch = fs::read(root.join("tmp/branch.txt"));
    let before_matches = shim_executed.then(|| {
        before
            .as_ref()
            .is_ok_and(|bytes| bytes.as_slice() == b"routing-before\r\n")
    });
    let branch_matches = shim_executed.then(|| {
        branch
            .as_ref()
            .is_ok_and(|bytes| bytes.as_slice() == b"path\r\n")
    });
    // 未执行的 shim 标记记为 null，并单独验证文件不存在，不能把省略记成匹配成功。
    let shim_markers_absent = (!shim_executed).then(|| {
        before
            .as_ref()
            .is_err_and(|failure| failure.kind() == io::ErrorKind::NotFound)
            && branch
                .as_ref()
                .is_err_and(|failure| failure.kind() == io::ErrorKind::NotFound)
    });
    let bound_console_exit_events = debugger
        .loader_trace
        .as_ref()
        .unwrap()
        .bound_console_exit_events;
    let receipt_matches = fs::read(&receipt)
        .is_ok_and(|bytes| bytes == b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n");
    eprintln!(
        "atomic_windows_cmd_stdio_control={}",
        serde_json::json!({
            "case": case,
            "real_node": real_node,
            "console_mode": if hidden_console { "hidden_new_console" } else { "no_window" },
            "shim_executed": shim_executed,
            "source_sha256": expected_source.sha256,
            "native_exit_code": result.as_ref().ok().copied(),
            "failure_kind": result.as_ref().err().map(|failure| format!("{:?}", failure.kind())),
            "os_code": result.as_ref().err().and_then(io::Error::raw_os_error),
            "before_marker_matches": before_matches,
            "branch_marker_matches": branch_matches,
            "shim_markers_absent": shim_markers_absent,
            "bound_console_exit_events": bound_console_exit_events,
            "cleanup_confirmed": cleanup.is_ok(),
            "cleanup_failure_kind": cleanup.as_ref().err().map(|failure| format!("{:?}", failure.kind())),
            "cleanup_os_code": cleanup.as_ref().err().and_then(io::Error::raw_os_error),
            "receipt_matches": receipt_matches,
            "watchdog_completed": watchdog_completed,
        })
    );
    if private_desktop {
        // 此核验针对持有的新对象；不声称直接观察了子线程实际 desktop 或最低权限合同。
        eprintln!(
            "atomic_windows_private_desktop_control={}",
            serde_json::json!({
                "case": case,
                "phase": private_desktop_phase,
                "desktop_mode": if existing_station_desktop { "existing_noninteractive_station" } else { "new_private_station" },
                "console_mode": "no_window",
                "shim_executed": false,
                "environment_control_established": private_objects_verified,
                "private_objects_verified": private_objects_verified,
                "object_verification_scope": if existing_station_desktop { "retained_private_desktop_and_borrowed_station_identity" } else { "retained_private_object_handles" },
                "private_objects_closed_scope": if existing_station_desktop { "owned_private_desktop_only" } else { "owned_handles" },
                "private_objects_closed": cleanup.is_ok().then_some(true),
                "cleanup_confirmed": cleanup.is_ok(),
                "receipt_matches": receipt_matches,
                "native_exit_code": result.as_ref().ok().copied(),
                "failure": result.as_ref().err().map(private_desktop_failure_summary),
                "cleanup_failure": cleanup.as_ref().err().map(private_desktop_failure_summary),
            })
        );
        // 在常规断言前保存失败证据；不能把环境建立失败归因为 Node 自然初始化失败。
        assert!(cleanup.is_ok(), "私有桌面对照未确认完整清理，证据已保留");
        assert!(
            private_objects_verified,
            "私有桌面对照环境未建立，证据已保留"
        );
        assert!(result.is_ok(), "私有桌面对照运行失败，证据已保留");
    }
    cleanup.expect("标准流对照必须恢复 ACL、删除 profile 并清空 Job");
    assert_eq!(result.unwrap(), 0);
    if hidden_console {
        assert!(
            bound_console_exit_events > 0,
            "隐藏控制台必须取得已核验 conhost 的退出事件"
        );
    }
    if shim_executed {
        assert_eq!(before_matches, Some(true));
        assert_eq!(branch_matches, Some(true));
    } else {
        assert_eq!(shim_markers_absent, Some(true));
    }
    assert!(receipt_matches && watchdog_completed);
    record_debug_native_exit(name);
}

#[test]
#[ignore = "Windows npm 缺少进程隔离核验时必须在首事件前停止"]
fn npm_console_host_rejects_missing_container_guard() {
    let name = "console_binding::npm_console_host_rejects_missing_container_guard";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::new();
    fixture.process.resume().unwrap();

    let failure = fixture
        .debugger
        .verify_package_initial_image(fixture.process.id())
        .unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_console_container_missing"
    );
    assert!(fixture.debugger.pending_event.is_some());
    assert!(!fixture.marker.exists());
    assert!(!fixture.receipt.exists());
    fixture.finish();
    record_debug_native_exit(name);
}

#[test]
fn package_exit_timeout_preserves_handles_across_repeated_cleanup() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let mut session = lease.prepare_image_debug_session().unwrap();
    // EXIT 事件已收齐也不能代表内核句柄已 signaled；当前活进程提供真实未退出句柄。
    let original = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            GetCurrentProcessId(),
        )
    }
    .unwrap();
    let original = unsafe { OwnedHandle::from_raw_handle(original.0) };
    session.held_package_processes.push(original);
    session.root_exit_observed = true;
    assert!(session.processes.is_empty());

    let first = session
        .wait_for_package_processes_exit(Instant::now())
        .unwrap_err();
    let second = session
        .wait_for_package_processes_exit(Instant::now())
        .unwrap_err();

    assert_eq!(
        first.to_string(),
        "managed_process.atomic_windows_cleanup_process_not_signaled"
    );
    assert_eq!(
        second.to_string(),
        "managed_process.atomic_windows_cleanup_process_not_signaled"
    );
    assert_eq!(session.held_package_processes.len(), 1);
    assert_eq!(
        unsafe { GetProcessId(HANDLE(session.held_package_processes[0].as_raw_handle())) },
        unsafe { GetCurrentProcessId() }
    );
}
