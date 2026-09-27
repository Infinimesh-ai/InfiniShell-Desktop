use windows::Win32::System::Threading::PROCESS_SYNCHRONIZE;

use super::*;

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
    fixture.finish();
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
    fixture.finish();
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
