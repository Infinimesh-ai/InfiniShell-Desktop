use warpui::App;

use super::*;
use crate::terminal::cli_agent_updates::{CliAgentUpdatePhase, CliAgentUpdatesModel};
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
