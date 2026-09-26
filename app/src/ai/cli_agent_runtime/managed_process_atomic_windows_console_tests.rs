use windows::Win32::System::Threading::PROCESS_SYNCHRONIZE;

use super::*;

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
