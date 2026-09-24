use warpui::App;

use super::*;
use crate::terminal::cli_agent::{CLIAgentInstallModel, init_cli_agent_updates};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::cli_agent_updates::{CliAgentUpdatePhase, CliAgentUpdatesModel};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn launch_fixture(harness: &str) -> (LocalCliTask, SessionOptions, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let task = LocalCliTask {
        version: 1,
        task_id: "update-launch-fixture".into(),
        parent_task_id: None,
        parent_generation: None,
        harness: harness.into(),
        working_directory: directory.path().to_string_lossy().into(),
        config_json: "{}".into(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let options = SessionOptions {
        executable: directory.path().join("not-a-cli"),
        cwd: directory.path().to_owned(),
        state_dir: directory.path().to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    (task, options, directory)
}

#[test]
fn updating_cli_refuses_managed_task_before_creating_connection() {
    let _flag = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::Updating);
        });
        let coordinator = app.add_model(|_| LocalCLITaskCoordinator::new(None));
        let (task, options, _directory) = launch_fixture("codex");

        coordinator.update(&mut app, |coordinator, ctx| {
            let result = coordinator.start(task, options, None, ctx);

            assert_eq!(
                result,
                Err(crate::t!(
                    "settings-cli-updates-launch-blocked",
                    agent = "Codex"
                ))
            );
            assert!(coordinator.entries.is_empty());
            assert!(coordinator.restored.is_empty());
        });
    });
}

#[test]
fn verifying_cli_refuses_prepared_child_before_creating_connection() {
    let _flag = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Claude, CliAgentUpdatePhase::Verifying);
        });
        let coordinator = app.add_model(|_| LocalCLITaskCoordinator::new(None));
        let (mut task, options, _directory) = launch_fixture("claude");
        task.parent_task_id = Some("parent".into());
        task.parent_generation = Some(1);

        coordinator.update(&mut app, |coordinator, ctx| {
            let result = coordinator.start_prepared(task, options, ctx);

            assert_eq!(
                result,
                Err(crate::t!(
                    "settings-cli-updates-launch-blocked",
                    agent = "Claude Code"
                ))
            );
            assert!(coordinator.entries.is_empty());
        });
    });
}

#[test]
fn updating_cli_refuses_history_resume_with_new_input() {
    let _flag = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Grok, CliAgentUpdatePhase::Updating);
        });
        let coordinator = app.add_model(|_| LocalCLITaskCoordinator::new(None));
        let (mut task, mut options, _directory) = launch_fixture("grok");
        task.native_session_id = Some("native-history".into());
        options.target = SessionTarget::Resume {
            native_session_id: "native-history".into(),
        };

        coordinator.update(&mut app, |coordinator, ctx| {
            let result = coordinator.start_with_input(
                task,
                options,
                Some(1),
                Uuid::new_v4(),
                vec![InputContent::Text("恢复后追加".into())],
                ctx,
            );

            assert_eq!(
                result,
                Err(crate::t!(
                    "settings-cli-updates-launch-blocked",
                    agent = "Grok Build"
                ))
            );
            assert!(coordinator.entries.is_empty());
        });
    });
}

#[test]
fn checking_cli_does_not_replace_normal_managed_launch_validation() {
    let _flag = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Grok, CliAgentUpdatePhase::Checking);
        });
        let coordinator = app.add_model(|_| LocalCLITaskCoordinator::new(None));
        let (task, options, _directory) = launch_fixture("grok");

        coordinator.update(&mut app, |coordinator, ctx| {
            let result = coordinator.start(task, options, None, ctx);

            // 无数据库 writer 时走原保存错误，不能触发 CLI 或被检查状态误拦。
            assert_eq!(result, Err(crate::t!("cli-agent-task-save-failed")));
            assert!(coordinator.entries.is_empty());
        });
    });
}

#[test]
fn managed_runtime_events_defer_update_until_completed_connection_exits() {
    for (agent, version) in [
        (CLIAgent::Codex, "0.156.1"),
        (CLIAgent::Claude, "2.1.280"),
        (CLIAgent::Grok, "1.0.41"),
    ] {
        App::test((), move |mut app| async move {
            initialize_settings_for_tests(&mut app);
            app.add_singleton_model(|_| CLIAgentSessionsModel::new());
            app.add_singleton_model(|_| CLIAgentInstallModel::empty_for_test());
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(None));
            app.update(init_cli_agent_updates);
            let (task, options, _directory) = launch_fixture(agent.command_prefix());
            let token = options.generation;
            let mut snapshot = ManagedTaskSnapshot {
                task,
                output: String::new(),
                ready: false,
                connected: true,
                active_turn_id: None,
                approvals: Vec::new(),
                error: None,
            };
            // 仅替代原生连接；运行事件消费、协调器发布及升级订阅均使用产品路径。
            let (commands, _requests) = mpsc::channel(1);
            app.update(|ctx| {
                LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, _| {
                    coordinator.entries.insert(
                        snapshot.task.task_id.clone(),
                        ManagedTaskEntry {
                            token,
                            snapshot: snapshot.clone(),
                            commands,
                            pending_tool_calls: HashSet::new(),
                        },
                    );
                });
            });
            let mut dispatched = None;
            for kind in [
                RuntimeEventKind::SessionReady {
                    verified_cli_version: Some(version.into()),
                    effective_permissions: json!({}),
                },
                RuntimeEventKind::TurnStarted {
                    turn_id: "update-busy-turn".into(),
                },
                RuntimeEventKind::ApprovalRequested {
                    approval_id: "update-busy-approval".into(),
                    turn_id: "update-busy-turn".into(),
                    method: "synthetic-file-tool".into(),
                    details: json!({}),
                },
                RuntimeEventKind::ApprovalCancelled {
                    approval_id: "update-busy-approval".into(),
                },
                RuntimeEventKind::TurnFinished {
                    turn_id: "update-busy-turn".into(),
                    outcome: TurnOutcome::Completed,
                    output: "synthetic-result".into(),
                },
            ] {
                let event = RuntimeEvent {
                    generation: token,
                    native_session_id: Some("update-busy-session".into()),
                    kind,
                };
                apply_runtime_event(&mut snapshot, &event).unwrap();
                app.update(|ctx| {
                    LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                        coordinator.publish(token, snapshot.clone(), Some(event), ctx);
                    });
                });
                if dispatched.is_none() {
                    dispatched = Some(app.update(|ctx| {
                        CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
                            assert!(updates.status(agent).unwrap().busy);
                            updates.stage_recorded_update_for_test(agent, ctx)
                        })
                    }));
                }
                app.update(|ctx| {
                    let status = CliAgentUpdatesModel::as_ref(ctx).status(agent).unwrap();
                    assert!(status.busy);
                    assert_eq!(status.phase, CliAgentUpdatePhase::WaitingForIdle);
                    assert_eq!(
                        dispatched.as_ref().unwrap().try_recv(),
                        Err(std::sync::mpsc::TryRecvError::Empty)
                    );
                });
            }
            assert_eq!(snapshot.task.state, LocalCliTaskState::Completed);
            assert!(snapshot.connected && snapshot.active_turn_id.is_none());
            let event = RuntimeEvent {
                generation: token,
                native_session_id: Some("update-busy-session".into()),
                kind: RuntimeEventKind::Disconnected {
                    reason: "synthetic-normal-exit".into(),
                },
            };
            apply_runtime_event(&mut snapshot, &event).unwrap();
            app.update(|ctx| {
                LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.publish(token, snapshot, Some(event), ctx);
                });
            });
            app.update(|ctx| {
                let updates = CliAgentUpdatesModel::as_ref(ctx);
                assert!(!updates.status(agent).unwrap().busy);
                assert!(updates.is_updating(agent));
                let dispatched = dispatched.as_ref().unwrap();
                assert_eq!(dispatched.try_recv(), Ok(agent));
                assert_eq!(
                    dispatched.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Empty)
                );
            });
        });
    }
}

#[test]
fn unresolved_completed_host_and_recovery_scan_defer_update_until_exit_is_proven() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        app.add_singleton_model(|_| CLIAgentInstallModel::empty_for_test());
        app.add_singleton_model(|_| LocalCLITaskCoordinator::new(None));
        app.update(init_cli_agent_updates);
        let (mut task, mut options, directory) = launch_fixture("codex");
        let state_dir = directory.path().canonicalize().unwrap();
        options.state_dir = state_dir.clone();
        options.cwd = state_dir.clone();
        task.native_session_id = Some("completed-native-session".into());
        task.config_json = json!({"runtime_generation": options.generation}).to_string();
        let writer = crate::persistence::start_test_writer(&state_dir.join("busy.sqlite")).unwrap();
        checkpoint_task(&writer.sender, task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        task.revision += 1;
        task.state = LocalCliTaskState::Running;
        checkpoint_task(&writer.sender, task.clone(), Some(task.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        task.revision += 1;
        task.state = LocalCliTaskState::Completed;
        task.result = Some("保留完成结果".into());
        task.terminal_evidence = Some("保留终态证据".into());
        checkpoint_task(&writer.sender, task.clone(), Some(task.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        // 真实恢复扫描缺少宿主退出证据，不能把已完成的回合当作 CLI 空闲。
        let batch = recover_runtime_hosts_in_state_dir(
            &writer.sender,
            vec![task.clone()],
            &HashMap::new(),
            &state_dir,
        )
        .await
        .unwrap();
        assert_eq!(batch.records, vec![task.clone()]);
        app.update(|ctx| {
            LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                coordinator.restored = batch.records;
                coordinator.unconfirmed_hosts = batch.unconfirmed_hosts;
                ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
            });
        });
        let dispatched = app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
                assert!(updates.status(CLIAgent::Codex).unwrap().busy);
                updates.stage_recorded_update_for_test(CLIAgent::Codex, ctx)
            })
        });
        super::super::runtime_host::write_manifest_failure_receipt_for_test(
            task.task_id.clone(),
            task.generation,
            Harness::Codex,
            options,
        )
        .unwrap();
        let batch = recover_runtime_hosts_in_state_dir(
            &writer.sender,
            vec![task.clone()],
            &HashMap::new(),
            &state_dir,
        )
        .await
        .unwrap();
        assert!(batch.unconfirmed_hosts.is_empty());
        assert_eq!(batch.records, vec![task.clone()]);
        app.update(|ctx| {
            LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, _| {
                coordinator.unconfirmed_hosts.clear();
            });
        });
        // 扫描进行中及扫描失败也不能暂时清空 Busy。
        for failed in [false, true] {
            app.update(|ctx| {
                LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.recovery_owner = (!failed).then(Uuid::new_v4);
                    coordinator.recovery_error = failed.then(|| "恢复证据不可读".into());
                    ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                });
            });
            app.update(|ctx| {
                let status = CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Codex)
                    .unwrap();
                assert!(status.busy);
                assert_eq!(status.phase, CliAgentUpdatePhase::WaitingForIdle);
                assert_eq!(
                    dispatched.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Empty)
                );
            });
        }
        app.update(|ctx| {
            LocalCLITaskCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                coordinator.restored = batch.records;
                coordinator.unconfirmed_hosts = batch.unconfirmed_hosts;
                coordinator.recovery_owner = None;
                coordinator.recovery_complete = true;
                coordinator.recovery_error = None;
                ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
            });
        });
        app.update(|ctx| {
            let updates = CliAgentUpdatesModel::as_ref(ctx);
            assert!(!updates.status(CLIAgent::Codex).unwrap().busy);
            assert!(updates.is_updating(CLIAgent::Codex));
            assert_eq!(dispatched.try_recv(), Ok(CLIAgent::Codex));
            assert_eq!(
                dispatched.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            );
            assert_eq!(
                LocalCLITaskCoordinator::as_ref(ctx).restored_tasks(),
                &[task]
            );
        });
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}
