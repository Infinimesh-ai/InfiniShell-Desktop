use serde_json::json;
use warpui::{App, SingletonEntity};

use super::*;
use crate::persistence::model::LocalCliTaskState;
use crate::terminal::CLIAgent;

fn task(config: Value) -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: Uuid::new_v4().to_string(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: "/project".into(),
        config_json: config.to_string(),
        native_session_id: Some(Uuid::new_v4().to_string()),
        generation: 1,
        revision: 2,
        state: LocalCliTaskState::Disconnected,
        result: None,
        terminal_evidence: None,
    }
}

#[test]
fn unrelated_tasks_do_not_discover_or_adopt_native_leaders() {
    assert!(
        restore_known_launches(&[
            task(json!({"runtime_generation":Uuid::new_v4()})),
            task(json!({"execution_kind":"terminal_hook"})),
        ])
        .is_empty()
    );
}

#[test]
fn missing_owned_manifest_is_a_recovery_failure_even_after_a_terminal_result() {
    for state in [
        LocalCliTaskState::Disconnected,
        LocalCliTaskState::Completed,
        LocalCliTaskState::Cancelled,
    ] {
        let mut saved = task(json!({"execution_kind":"grok_owned_terminal"}));
        saved.state = state;
        let recovered = restore_known_launches(&[saved]);
        assert_eq!(recovered.len(), 1);
        assert!(recovered[0].is_err());
    }
}

#[test]
fn pending_or_failed_recovery_keeps_grok_busy_without_a_pane() {
    let mut sessions = CLIAgentSessionsModel::new();
    assert!(!sessions.has_local_session(CLIAgent::Grok));
    sessions.grok_owned_recovery.pending = true;
    assert!(sessions.has_local_session(CLIAgent::Grok));
    assert!(!sessions.has_local_session(CLIAgent::Claude));
    sessions.grok_owned_recovery.pending = false;
    sessions.grok_owned_recovery.failed = true;
    assert!(sessions.has_local_session(CLIAgent::Grok));
}

#[test]
fn failed_recovery_callback_does_not_open_an_update_window() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let saved = task(json!({"execution_kind":"grok_owned_terminal"}));
        let (done, completed) = futures::channel::oneshot::channel();
        app.update(|ctx| {
            let mut done = Some(done);
            ctx.observe_model(&CLIAgentSessionsModel::handle(ctx), move |handle, ctx| {
                if !handle.as_ref(ctx).grok_owned_recovery.pending {
                    if let Some(done) = done.take() {
                        let _ = done.send(());
                    }
                }
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| {
                model.restore_grok_owned_launches(vec![saved], ctx);
                assert!(model.has_local_session(CLIAgent::Grok));
            })
        });
        completed.await.unwrap();
        app.update(|ctx| {
            let model = CLIAgentSessionsModel::as_ref(ctx);
            assert!(!model.grok_owned_recovery.pending);
            assert!(model.grok_owned_recovery.failed);
            assert!(model.has_local_session(CLIAgent::Grok));
            assert!(model.grok_owned_recovery.launches.is_empty());
        });
    });
}

#[test]
fn corrupted_grok_configuration_cannot_be_interpreted_as_no_owned_process() {
    let mut saved = task(json!({}));
    saved.config_json = "{invalid".into();
    let recovered = restore_known_launches(&[saved]);
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].is_err());
}
