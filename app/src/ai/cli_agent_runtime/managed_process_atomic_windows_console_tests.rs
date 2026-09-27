use std::sync::mpsc;

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
    // 同一 CMD、目录与 token 只改变 stderr 的 NUL 重定向；脚本不启动外部命令。
    fs::write(
        &script,
        concat!(
            "@echo off\r\n",
            ">tmp\\plain.txt echo builtin-control\r\n",
            ">tmp\\plain-status.txt echo %errorlevel%\r\n",
            ">tmp\\nul.txt 2>NUL echo builtin-control\r\n",
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
    let nul_status = fs::read_to_string(root.join("tmp/nul-status.txt")).unwrap();
    let plain = fs::read(root.join("tmp/plain.txt")).unwrap();
    let redirected = fs::read(root.join("tmp/nul.txt")).unwrap();
    let after = fs::read(root.join("tmp/after.txt")).unwrap();
    let expected: &[u8] = b"builtin-control\r\n";
    eprintln!(
        "atomic_windows_nul_control={}",
        serde_json::json!({
            "plain_errorlevel": plain_status.trim().parse::<u32>().unwrap(),
            "nul_errorlevel": nul_status.trim().parse::<u32>().unwrap(),
            "plain_marker_matches": plain == expected,
            "nul_marker_matches": redirected == expected,
            "after_marker_matches": after == expected,
            "cleanup_confirmed": true,
        })
    );
    assert_eq!(plain_status.trim(), "0");
    assert_eq!(plain, expected);
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
