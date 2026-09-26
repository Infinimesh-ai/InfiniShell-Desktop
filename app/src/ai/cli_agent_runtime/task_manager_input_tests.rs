use std::path::Path;

use serde_json::json;
use warpui::{AddSingletonModel, App};

use super::*;
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
use crate::ai::skills::SkillManager;
use crate::persistence::model::{LocalCliTask, LocalCliTaskState};
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};

fn connected_grok() -> ManagedTaskSnapshot {
    ManagedTaskSnapshot {
        task:LocalCliTask {
            version:1, task_id:"grok-skill-ui".into(), parent_task_id:None, parent_generation:None,
            harness:"grok".into(), working_directory:std::env::temp_dir().to_string_lossy().into_owned(),
            config_json:json!({
                "permission_policy":"Inherit","model":null,"selected_skills":[],"cli_version":"1.0.41",
                "runtime_generation":"11111111-1111-4111-8111-111111111111",
                "cli_version_runtime_generation":"11111111-1111-4111-8111-111111111111",
                "effective_permissions":{"requestedPolicy":"inherit","appCreationPolicyApplied":false,
                    "verifiedCapabilities":{"submit":true,"localTools":false},
                    "reportedMetadata":{"models":{"currentModelId":"grok-4.7"}}}
            }).to_string(),
            native_session_id:Some("native-grok-skill-session".into()),generation:2,revision:4,
            state:LocalCliTaskState::Completed,result:None,terminal_evidence:None,
        },
        ready:true,connected:true,active_turn_id:None,approvals:Vec::new(),output:String::new(),error:None,
    }
}

fn change_config(snapshot: &mut ManagedTaskSnapshot, change: impl FnOnce(&mut Value)) {
    let mut config = serde_json::from_str(&snapshot.task.config_json).unwrap();
    change(&mut config);
    snapshot.task.config_json = config.to_string();
}

#[test]
fn current_connected_grok_requires_the_native_model_and_matching_runtime_binding() {
    let mut snapshot = connected_grok();
    assert!(grok_current_skill_snapshot(&snapshot));
    assert!(grok_skill_refresh_idle(&snapshot));
    change_config(&mut snapshot, |config| {
        config["effective_permissions"]["reportedMetadata"]["models"]["currentModelId"] =
            json!("other-model")
    });
    assert!(!grok_current_skill_snapshot(&snapshot));
    let mut old_runtime = connected_grok();
    change_config(&mut old_runtime, |config| {
        config["cli_version_runtime_generation"] = json!("22222222-2222-4222-8222-222222222222")
    });
    assert!(!grok_current_skill_snapshot(&old_runtime));
}

#[test]
fn detected_future_version_cannot_reuse_the_fixed_skill_contract() {
    let mut snapshot = connected_grok();
    change_config(&mut snapshot, |config| {
        config["cli_version"] = json!("1.0.42")
    });
    assert!(!grok_current_skill_snapshot(&snapshot));
    change_config(&mut snapshot, |config| {
        config["cli_version"] = json!("1.0.30")
    });
    assert!(!grok_current_skill_snapshot(&snapshot));
}

#[test]
fn stopped_or_not_ready_runtime_cannot_enable_hot_skill_selection() {
    let mut disconnected = connected_grok();
    disconnected.connected = false;
    assert!(!grok_skill_refresh_idle(&disconnected));
    let mut opening = connected_grok();
    opening.ready = false;
    assert!(!grok_skill_refresh_idle(&opening));
}

#[test]
fn active_turn_or_persisted_unacknowledged_input_prevents_refresh() {
    let mut running = connected_grok();
    running.active_turn_id = Some("current-turn".into());
    assert!(grok_current_skill_snapshot(&running));
    assert!(!grok_skill_refresh_idle(&running));
    let mut waiting = connected_grok();
    change_config(&mut waiting, |config| {
        config["grok_pending_inputs"] = json!([{"message_id":"pending"}])
    });
    assert!(!grok_skill_refresh_idle(&waiting));
    change_config(&mut waiting, |config| {
        config["grok_pending_inputs"] = json!("malformed")
    });
    assert!(!grok_skill_refresh_idle(&waiting));
}

#[test]
fn completed_native_input_link_allows_hot_skills_without_erasing_recovery_identity() {
    let mut snapshot = connected_grok();
    change_config(&mut snapshot, |config| {
        config["grok_current_input"] = json!({
            "runtime_generation":config["runtime_generation"],
            "native_turn_id":"finished-turn"
        });
    });
    // 只有完成状态还不够，必须有同一会话和回合的结束收据。
    assert!(!grok_skill_refresh_idle(&snapshot));
    snapshot.task.terminal_evidence = Some(json!({
        "native_session_id":snapshot.task.native_session_id,
        "event":{"TurnFinished":{"turn_id":"finished-turn","outcome":"Completed","output":"result"}}
    }).to_string());
    assert!(grok_skill_refresh_idle(&snapshot));
    let saved = snapshot.task.config_json.clone();
    assert!(grok_skill_refresh_idle(&snapshot));
    assert_eq!(snapshot.task.config_json, saved);
    change_config(&mut snapshot, |config| {
        config["grok_pending_inputs"] = json!([{"message_id":"next"}]);
    });
    assert!(!grok_skill_refresh_idle(&snapshot));
}

#[test]
fn unrelated_terminal_receipt_cannot_unlock_hot_skills() {
    for (session, turn, runtime) in [
        (
            "other-session",
            "finished-turn",
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            "native-grok-skill-session",
            "other-turn",
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            "native-grok-skill-session",
            "finished-turn",
            "22222222-2222-4222-8222-222222222222",
        ),
    ] {
        let mut snapshot = connected_grok();
        change_config(&mut snapshot, |config| {
            config["grok_current_input"] = json!({
                "runtime_generation":runtime,"native_turn_id":"finished-turn"
            });
        });
        snapshot.task.terminal_evidence = Some(
            json!({
                "native_session_id":session,
                "event":{"TurnFinished":{"turn_id":turn,"outcome":"Completed","output":"result"}}
            })
            .to_string(),
        );
        assert!(!grok_skill_refresh_idle(&snapshot));
    }
}

#[test]
fn direct_tools_or_fixed_profiles_do_not_enable_default_grok_skills() {
    let mut sdk = connected_grok();
    change_config(&mut sdk, |config| {
        config["local_tools"] = json!({"allow_spawn":false,"allow_message":false})
    });
    assert!(!grok_current_skill_snapshot(&sdk));
    let mut fixed = connected_grok();
    change_config(&mut fixed, |config| {
        config["permission_policy"] = json!("GrokRestrictedFilesV1")
    });
    assert!(!grok_current_skill_snapshot(&fixed));
    let mut unknown_ceiling = connected_grok();
    change_config(&mut unknown_ceiling, |config| {
        config["permission_ceiling"] = json!({"unverified":"parent"})
    });
    assert!(!grok_current_skill_snapshot(&unknown_ceiling));
}

fn manager_view(app: &mut App) -> ViewHandle<LocalCLITaskManagerView> {
    crate::test_util::terminal::initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| crate::workspace::ToastStack);
    app.add_singleton_model(|_| CLIAgentInstallModel::empty_for_test());
    app.add_singleton_model(|_| LocalCLITaskCoordinator::with_restored_for_test(Vec::new()));
    app.add_window(
        warpui::platform::WindowStyle::NotStealFocus,
        LocalCLITaskManagerView::new,
    )
    .1
}

fn register_skill(app: &mut App, root: &Path, name: &str) -> SkillReference {
    let path = root.join(".grok/skills").join(name).join("SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!(
            "---\nname: {name}\ndescription: 本地技能\nuser-invocable: true\n---\n只读夹具。\n"
        ),
    )
    .unwrap();
    SkillManager::handle(app).update(app, |manager, _| {
        manager.handle_skills_added(vec![ai::skills::parse_skill(&path).unwrap()])
    });
    SkillReference::Path(LocalOrRemotePath::Local(path))
}

fn installation(app: &mut App, version: &str) {
    CLIAgentInstallModel::handle(app).update(app, |model, _| {
        *model = CLIAgentInstallModel::with_installation_for_test(
            CLIAgent::Grok,
            CLIAgentInstallation {
                executable: Some(std::env::temp_dir().join("fixture-grok")),
                version: CLIAgentVersionStatus::Detected(version.into()),
            },
        );
    });
}

#[test]
fn fixed_new_draft_can_stage_two_skills_for_native_validation() {
    App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        let alpha = register_skill(&mut app, project.path(), "alpha");
        let beta = register_skill(&mut app, project.path(), "beta");
        installation(&mut app, "1.0.41");
        manager.update(&mut app, |view, ctx| {
            view.harness = Harness::Grok;
            view.permission = PermissionPolicy::Inherit;
            view.select_composer_skill(&alpha, view.input_generation, ctx)
                .unwrap();
            view.select_composer_skill(&beta, view.input_generation, ctx)
                .unwrap();
            assert_eq!(view.parsed_composer_skills(ctx).unwrap().len(), 2);
            assert_eq!(view.managed_input.attachments.skills, vec![alpha, beta]);
        });
    });
}

#[test]
fn legacy_grok_draft_keeps_the_single_skill_limit() {
    App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        let alpha = register_skill(&mut app, project.path(), "alpha");
        let beta = register_skill(&mut app, project.path(), "beta");
        installation(&mut app, "1.0.30");
        manager.update(&mut app, |view, ctx| {
            view.harness = Harness::Grok;
            view.permission = PermissionPolicy::Inherit;
            view.select_composer_skill(&alpha, view.input_generation, ctx)
                .unwrap();
            assert_eq!(
                view.select_composer_skill(&beta, view.input_generation, ctx)
                    .unwrap_err(),
                crate::t!("cli-agent-task-skill-one-per-turn")
            );
            assert_eq!(view.managed_input.attachments.skills, vec![alpha]);
        });
    });
}

#[test]
fn image_draft_cannot_mix_with_a_selected_grok_skill() {
    App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        let alpha = register_skill(&mut app, project.path(), "alpha");
        installation(&mut app, "1.0.41");
        manager.update(&mut app, |view, ctx| {
            view.harness = Harness::Grok;
            view.permission = PermissionPolicy::Inherit;
            view.managed_input.attachments.images.push(ImageContext {
                data: String::new(),
                mime_type: "image/png".into(),
                file_name: "fixture.png".into(),
                is_figma: false,
            });
            assert!(
                view.select_composer_skill(&alpha, view.input_generation, ctx)
                    .is_err()
            );
            view.managed_input.attachments.skills.push(alpha);
            assert!(view.parsed_composer_skills(ctx).is_err());
        });
    });
}

#[test]
fn skill_count_limit_is_checked_before_mutating_the_draft() {
    App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        let extra = register_skill(&mut app, project.path(), "extra");
        installation(&mut app, "1.0.41");
        manager.update(&mut app, |view, ctx| {
            view.harness = Harness::Grok;
            view.permission = PermissionPolicy::Inherit;
            view.managed_input.attachments.skills = (0..32)
                .map(|index| {
                    SkillReference::Path(LocalOrRemotePath::Local(
                        project.path().join(format!("selected-{index}/SKILL.md")),
                    ))
                })
                .collect();
            assert_eq!(
                view.select_composer_skill(&extra, view.input_generation, ctx)
                    .unwrap_err(),
                crate::t!("cli-agent-grok-skill-selection-invalid")
            );
            assert_eq!(view.managed_input.attachments.skills.len(), 32);
        });
    });
}

#[test]
fn future_installation_cannot_stage_a_second_skill() {
    App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        let alpha = register_skill(&mut app, project.path(), "alpha");
        let beta = register_skill(&mut app, project.path(), "beta");
        installation(&mut app, "1.0.42");
        manager.update(&mut app, |view, ctx| {
            view.harness = Harness::Grok;
            view.permission = PermissionPolicy::Inherit;
            view.select_composer_skill(&alpha, view.input_generation, ctx)
                .unwrap();
            assert!(
                view.select_composer_skill(&beta, view.input_generation, ctx)
                    .is_err()
            );
            assert_eq!(view.managed_input.attachments.skills, vec![alpha]);
        });
    });
}

#[test]
fn permission_change_is_rechecked_when_parsing_an_existing_draft() {
    App::test((), |mut app| async move {
        let project = tempfile::tempdir().unwrap();
        let manager = manager_view(&mut app);
        let alpha = register_skill(&mut app, project.path(), "alpha");
        installation(&mut app, "1.0.41");
        manager.update(&mut app, |view, ctx| {
            view.harness = Harness::Grok;
            view.permission = PermissionPolicy::Inherit;
            view.select_composer_skill(&alpha, view.input_generation, ctx)
                .unwrap();
            view.permission = PermissionPolicy::GrokRestrictedFilesV1;
            assert!(view.parsed_composer_skills(ctx).is_err());
            view.permission = PermissionPolicy::Inherit;
            view.local_tools.allow_message = true;
            assert!(view.parsed_composer_skills(ctx).is_err());
        });
    });
}
