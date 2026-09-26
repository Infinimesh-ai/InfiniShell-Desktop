use super::*;
use warpui::App;

#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
use crate::terminal::cli_agent::init_cli_agent_updates;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::test_util::settings::initialize_settings_for_tests;

fn available_entry() -> Entry {
    let mut entry = Entry::new(Instant::now());
    entry.status.phase = CliAgentUpdatePhase::Available;
    entry.status.installed_version = Some("2.1.273".to_owned());
    entry.status.latest_version = Some("2.1.278".to_owned());
    entry.plan = Some(sources::plan_for_test());
    entry
}

#[test]
fn update_waits_for_first_session_sync_and_for_existing_session_to_exit() {
    let mut entry = available_entry();
    assert!(!entry.ready_to_update());
    entry.status.busy = false;
    assert!(entry.ready_to_update());
    entry.active = true;
    assert!(!entry.ready_to_update());
}

#[test]
fn failed_target_is_not_automatically_retried_but_new_release_is() {
    let mut entry = available_entry();
    entry.status.busy = false;
    entry.failed_target = Some("2.1.278".to_owned());
    assert!(!entry.ready_to_update());
    entry.status.latest_version = Some("2.1.279".to_owned());
    assert!(entry.ready_to_update());
}

#[test]
fn manual_update_still_obeys_busy_and_single_operation_guards() {
    let mut entry = available_entry();
    entry.status.auto_update = false;
    entry.manual_update = true;
    assert!(!entry.ready_to_update());
    entry.status.busy = false;
    assert!(entry.ready_to_update());
    entry.active = true;
    assert!(!entry.ready_to_update());
}

#[test]
fn only_network_checks_have_short_bounded_retry_schedule() {
    let mut entry = Entry::new(Instant::now());
    let now = Instant::now();
    for expected in [60, 300, 21600, 21600] {
        entry.check_failed(CliAgentUpdateError::Network, now);
        assert_eq!(entry.next_check.duration_since(now).as_secs(), expected);
    }
    entry.check_failed(CliAgentUpdateError::CommandFailed, now);
    assert_eq!(entry.next_check.duration_since(now), CHECK_INTERVAL);
    assert!(entry.plan.is_none());
}

#[test]
fn configure_invalidates_old_channel_before_idle_can_dispatch() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                model.entries.insert(CLIAgent::Claude, available_entry());
                model.configure(
                    CLIAgent::Claude,
                    false,
                    false,
                    CliAgentUpdateChannel::Stable,
                    ctx,
                );
                let entry = &model.entries[&CLIAgent::Claude];
                assert!(entry.plan.is_none());
                assert!(!entry.status.auto_update);
                assert!(!entry.status.busy);
                assert_eq!(entry.status.channel, CliAgentUpdateChannel::Stable);
                assert!(!entry.active);
                assert!(!model.operations_enabled);
            });
        });
    });
}

#[test]
fn stale_check_cannot_release_current_operation() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Grok).unwrap();
                entry.operation = 7;
                entry.active = true;
                entry.status.phase = CliAgentUpdatePhase::Updating;
                model.checked(
                    CLIAgent::Grok,
                    6,
                    CliAgentUpdateChannel::FollowInstallation,
                    Err(CliAgentUpdateError::Network),
                    ctx,
                );
                assert!(model.entries[&CLIAgent::Grok].active);
                assert_eq!(
                    model.status(CLIAgent::Grok).unwrap().phase,
                    CliAgentUpdatePhase::Updating
                );
            });
        });
    });
}

#[test]
fn active_update_enters_verifying_before_completion() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Claude).unwrap();
                entry.operation = 7;
                entry.active = true;
                entry.status.phase = CliAgentUpdatePhase::Updating;

                model.mark_verifying(CLIAgent::Claude, 7, ctx);

                assert_eq!(
                    model.status(CLIAgent::Claude).unwrap().phase,
                    CliAgentUpdatePhase::Verifying
                );
            });
        });
    });
}

#[test]
fn stale_verification_progress_cannot_replace_current_operation() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Claude).unwrap();
                entry.operation = 7;
                entry.active = true;
                entry.status.phase = CliAgentUpdatePhase::Updating;

                model.mark_verifying(CLIAgent::Claude, 6, ctx);

                assert_eq!(
                    model.status(CLIAgent::Claude).unwrap().phase,
                    CliAgentUpdatePhase::Updating
                );
            });
        });
    });
}

#[test]
fn successful_installation_keeps_unverified_managed_version_closed() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Claude).unwrap();
                entry.operation = 7;
                entry.active = true;
                entry.status.phase = CliAgentUpdatePhase::Verifying;
                entry.status.installed_version = Some("2.1.280".to_owned());
                entry.status.latest_version = Some("2.1.267".to_owned());

                model.updated(CLIAgent::Claude, 7, Ok("2.1.267".to_owned()), ctx);

                let entry = &model.entries[&CLIAgent::Claude];
                assert!(!entry.active);
                assert_eq!(entry.status.phase, CliAgentUpdatePhase::UpToDate);
                assert_eq!(entry.status.installed_version.as_deref(), Some("2.1.267"));
                assert!(entry.status.error.is_none());
                assert!(entry.failed_target.is_none());
                assert!(!crate::ai::cli_agent_runtime::claude::supported_version(
                    "2.1.267"
                ));
            });
        });
    });
}

#[test]
fn failed_version_probe_never_becomes_up_to_date() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Claude).unwrap();
                entry.operation = 7;
                entry.active = true;
                entry.status.phase = CliAgentUpdatePhase::Verifying;
                entry.status.installed_version = Some("2.1.267".to_owned());
                entry.status.latest_version = Some("2.1.278".to_owned());

                model.updated(
                    CLIAgent::Claude,
                    7,
                    Err(CliAgentUpdateError::ProbeFailed),
                    ctx,
                );

                let entry = &model.entries[&CLIAgent::Claude];
                assert!(!entry.active);
                assert_eq!(entry.status.phase, CliAgentUpdatePhase::Failed);
                assert_eq!(entry.status.installed_version.as_deref(), Some("2.1.267"));
                assert_eq!(entry.status.error, Some(CliAgentUpdateError::ProbeFailed));
                assert_eq!(entry.failed_target.as_deref(), Some("2.1.278"));
            });
        });
    });
}

#[test]
fn unresolved_recovery_keeps_launch_guard_after_failed_read_only_recheck() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Grok).unwrap();
                entry.operation = 1;
                entry.active = true;
                entry.status.phase = CliAgentUpdatePhase::Checking;
                entry.status.error = Some(CliAgentUpdateError::RecoveryRequired);
                model.checked(
                    CLIAgent::Grok,
                    1,
                    CliAgentUpdateChannel::FollowInstallation,
                    Err(CliAgentUpdateError::Network),
                    ctx,
                );
                assert!(model.is_updating(CLIAgent::Grok));
                assert_eq!(
                    model.status(CLIAgent::Grok).unwrap().error,
                    Some(CliAgentUpdateError::RecoveryRequired)
                );
            });
        });
    });
}

#[test]
fn launch_reservation_survives_root_idle_sync_and_blocks_until_real_release() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let view = EntityId::new();
                model.configure(
                    CLIAgent::Grok,
                    true,
                    false,
                    CliAgentUpdateChannel::Stable,
                    ctx,
                );
                assert!(model.reserve_launch(CLIAgent::Grok, view, ctx));
                model.configure(
                    CLIAgent::Grok,
                    true,
                    false,
                    CliAgentUpdateChannel::Stable,
                    ctx,
                );
                assert!(model.status(CLIAgent::Grok).unwrap().busy);
                model.release_launch(view, ctx);
                assert!(!model.status(CLIAgent::Grok).unwrap().busy);
                model.set_phase_for_test(CLIAgent::Grok, CliAgentUpdatePhase::Updating);
                assert!(!model.reserve_launch(CLIAgent::Grok, EntityId::new(), ctx));
            });
        });
    });
}

#[test]
fn installation_rescan_discards_old_check_before_it_can_produce_a_plan() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let mut entry = available_entry();
                entry.active = true;
                entry.operation = 4;
                entry.recheck = true;
                entry.status.busy = false;
                model.entries.insert(CLIAgent::Claude, entry);
                let report = sources::CheckReport {
                    failed_target: None,
                    installed_version: "2.1.273".to_owned(),
                    latest_version: "2.1.278".to_owned(),
                    source: CliAgentUpdateSource::Native,
                    effective_channel: CliAgentUpdateChannel::Latest,
                    up_to_date: false,
                    error: None,
                    plan: Some(sources::plan_for_test()),
                };
                model.checked(
                    CLIAgent::Claude,
                    4,
                    CliAgentUpdateChannel::FollowInstallation,
                    Ok(report),
                    ctx,
                );
                assert!(model.entries[&CLIAgent::Claude].plan.is_none());
                assert!(!model.entries[&CLIAgent::Claude].active);
            });
        });
    });
}

#[test]
fn isolation_unavailable_check_removes_the_previous_plan_and_blocks_dispatch() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let (sender, receiver) = mpsc::sync_channel(1);
                model.recorded_updates = Some(sender);
                model.operations_enabled = true;
                let mut entry = available_entry();
                entry.status.busy = false;
                entry.manual_update = true;
                assert!(entry.ready_to_update());
                entry.active = true;
                model.entries.insert(CLIAgent::Codex, entry);
                model.checked(
                    CLIAgent::Codex,
                    0,
                    CliAgentUpdateChannel::FollowInstallation,
                    Ok(sources::CheckReport {
                        failed_target: None,
                        installed_version: "0.155.0".to_owned(),
                        latest_version: "0.156.1".to_owned(),
                        source: CliAgentUpdateSource::Npm,
                        effective_channel: CliAgentUpdateChannel::Latest,
                        up_to_date: false,
                        error: Some(CliAgentUpdateError::IsolationUnavailable),
                        plan: None,
                    }),
                    ctx,
                );
                let entry = &model.entries[&CLIAgent::Codex];
                assert_eq!(entry.status.phase, CliAgentUpdatePhase::Unsupported);
                assert_eq!(
                    entry.status.error,
                    Some(CliAgentUpdateError::IsolationUnavailable)
                );
                assert!(entry.plan.is_none());
                assert!(!entry.active);
                assert!(!entry.manual_update);
                assert!(!entry.ready_to_update());
                assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
            });
        });
    });
}

#[test]
fn channel_mismatch_is_never_rendered_as_success_even_if_versions_match() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                model.entries.get_mut(&CLIAgent::Claude).unwrap().active = true;
                let report = sources::CheckReport {
                    failed_target: None,
                    installed_version: "2.1.278".to_owned(),
                    latest_version: "2.1.278".to_owned(),
                    source: CliAgentUpdateSource::Homebrew,
                    effective_channel: CliAgentUpdateChannel::Stable,
                    up_to_date: true,
                    error: Some(CliAgentUpdateError::ChannelMismatch),
                    plan: None,
                };
                model.checked(
                    CLIAgent::Claude,
                    0,
                    CliAgentUpdateChannel::FollowInstallation,
                    Ok(report),
                    ctx,
                );
                assert_eq!(
                    model.status(CLIAgent::Claude).unwrap().phase,
                    CliAgentUpdatePhase::Unsupported
                );
            });
        });
    });
}

#[test]
fn native_channel_sync_at_same_version_waits_for_idle_and_obeys_auto_toggle() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        app.update(|ctx| {
            CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
                let entry = model.entries.get_mut(&CLIAgent::Claude).unwrap();
                entry.active = true;
                entry.status.channel = CliAgentUpdateChannel::Latest;
                let report = sources::CheckReport {
                    failed_target: None,
                    installed_version: "2.1.278".to_owned(),
                    latest_version: "2.1.278".to_owned(),
                    source: CliAgentUpdateSource::Native,
                    effective_channel: CliAgentUpdateChannel::Latest,
                    // 版本已一致，但原生渠道尚未同步，不能呈现为完成。
                    up_to_date: false,
                    error: None,
                    plan: Some(sources::plan_for_test()),
                };
                model.checked(
                    CLIAgent::Claude,
                    0,
                    CliAgentUpdateChannel::Latest,
                    Ok(report),
                    ctx,
                );
                let entry = &model.entries[&CLIAgent::Claude];
                assert_eq!(entry.status.phase, CliAgentUpdatePhase::WaitingForIdle);
                assert!(entry.plan.is_some());
                assert!(!entry.ready_to_update());
                model.set_auto_update(CLIAgent::Claude, false, ctx);
                model.set_busy(CLIAgent::Claude, false, ctx);
                let entry = &model.entries[&CLIAgent::Claude];
                assert_eq!(entry.status.phase, CliAgentUpdatePhase::Available);
                assert!(entry.plan.is_some());
                assert!(!entry.ready_to_update());
                model.set_auto_update(CLIAgent::Claude, true, ctx);
                assert!(model.entries[&CLIAgent::Claude].ready_to_update());
            });
        });
    });
}

#[test]
fn pty_session_events_defer_update_until_last_local_session_exits() {
    for agent in AGENTS {
        App::test((), move |mut app| async move {
            initialize_settings_for_tests(&mut app);
            app.add_singleton_model(|_| CLIAgentSessionsModel::new());
            app.add_singleton_model(|_| CLIAgentInstallModel::empty_for_test());
            #[cfg(feature = "local_fs")]
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(None));
            app.update(init_cli_agent_updates);
            let views = [EntityId::new(), EntityId::new()];
            for (view, status) in views.into_iter().zip([
                CLIAgentSessionStatus::Blocked { message: None },
                CLIAgentSessionStatus::Success,
            ]) {
                app.update(|ctx| {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                        sessions.set_session(
                            view,
                            CLIAgentSession {
                                agent,
                                status,
                                session_context: CLIAgentSessionContext::default(),
                                input_state: CLIAgentInputState::Closed,
                                should_auto_toggle_input: false,
                                listener: None,
                                plugin_version: None,
                                remote_host: None,
                                draft_text: None,
                                custom_command_prefix: None,
                                received_rich_notification: false,
                            },
                            ctx,
                        );
                    });
                });
            }
            let dispatched = app.update(|ctx| {
                CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
                    assert!(updates.status(agent).unwrap().busy);
                    updates.stage_recorded_update_for_test(agent, ctx)
                })
            });
            assert_eq!(dispatched.try_recv(), Err(mpsc::TryRecvError::Empty));
            app.update(|ctx| {
                assert_eq!(
                    CliAgentUpdatesModel::as_ref(ctx)
                        .status(agent)
                        .unwrap()
                        .phase,
                    CliAgentUpdatePhase::WaitingForIdle
                );
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.remove_session(views[0], ctx);
                });
            });
            app.update(|ctx| {
                let status = CliAgentUpdatesModel::as_ref(ctx).status(agent).unwrap();
                assert!(status.busy);
                assert_eq!(status.phase, CliAgentUpdatePhase::WaitingForIdle);
                assert_eq!(dispatched.try_recv(), Err(mpsc::TryRecvError::Empty));
                // 回合已完成仍占用本地 CLI；最后一个真实 Ended 事件才释放更新。
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.remove_session(views[1], ctx);
                });
            });
            app.update(|ctx| {
                let updates = CliAgentUpdatesModel::as_ref(ctx);
                assert!(!updates.status(agent).unwrap().busy);
                assert!(updates.is_updating(agent));
                assert_eq!(dispatched.try_recv(), Ok(agent));
                assert_eq!(dispatched.try_recv(), Err(mpsc::TryRecvError::Empty));
            });
        });
    }
}
