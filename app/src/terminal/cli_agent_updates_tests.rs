use super::*;
use warpui::App;

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
