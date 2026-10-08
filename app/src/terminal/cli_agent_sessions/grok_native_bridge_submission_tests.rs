use super::super::listener::CLIAgentSessionListener;
use super::super::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionStatus,
};
use super::*;
use crate::ai::blocklist::{InputConfig, InputType};
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::ModelEventDispatcher;
use warpui::App;

fn open_session(native_session: &str) -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Grok,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            session_id: Some(native_session.to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Open {
            entrypoint: CLIAgentInputEntrypoint::CtrlG,
            previous_input_config: InputConfig {
                input_type: InputType::Shell,
                is_locked: false,
            },
            previous_was_lock_set_with_empty_buffer: true,
        },
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    }
}

#[test]
fn revoked_submission_cannot_write_or_be_reactivated() {
    let lease = SubmissionLease::new();
    let worker = lease.clone();
    lease.revoke();
    assert!(!worker.claim_write());
    assert!(!lease.claim_write());
    assert!(!worker.was_write_claimed());
}

#[test]
fn only_one_sender_claims_and_closing_does_not_undo_sent_input() {
    let lease = SubmissionLease::new();
    let observer = lease.clone();
    assert!(lease.claim_write());
    lease.revoke();
    assert!(observer.was_write_claimed());
    assert!(!lease.claim_write());
}

#[test]
fn failed_before_admission_preserves_zero_write_proof() {
    let lease = SubmissionLease::new();

    assert!(!lease.was_write_claimed());
    lease.revoke();
    assert!(!lease.was_write_claimed());
}

#[test]
fn closing_input_revokes_registered_write_and_duplicate_registration_is_rejected() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view = EntityId::new();
        model.update(&mut app, |model, ctx| {
            model.set_session(view, open_session("native-session"), ctx);
            let generation = model.input_generation(view).unwrap();
            assert!(model.begin_input_submission(view, generation));
            let lease = model
                .register_native_grok_submission(view, generation, "native-session".into())
                .unwrap();
            assert!(
                model
                    .register_native_grok_submission(view, generation, "native-session".into())
                    .is_none()
            );
            assert!(!model.begin_input_submission(view, generation));

            model.close_input(view, false, ctx);

            assert!(!lease.claim_write());
            assert!(!lease.was_write_claimed());
            assert!(!model.is_input_submission_current(view, generation));
            assert_ne!(model.input_generation(view), Some(generation));
        });
    });
}

#[test]
fn replacing_session_after_write_claim_invalidates_old_callback_without_revoking_new_send() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view = EntityId::new();
        model.update(&mut app, |model, ctx| {
            model.set_session(view, open_session("old-session"), ctx);
            let generation = model.input_generation(view).unwrap();
            assert!(model.begin_input_submission(view, generation));
            let old = model
                .register_native_grok_submission(view, generation, "old-session".into())
                .unwrap();
            assert!(old.claim_write());

            model.set_session(view, open_session("new-session"), ctx);
            let replacement_generation = model.input_generation(view).unwrap();
            assert_ne!(replacement_generation, generation);
            assert!(!model.is_input_submission_current(view, generation));
            assert!(old.was_write_claimed());
            assert!(!old.claim_write());
            assert!(model.begin_input_submission(view, replacement_generation));
            let replacement = model
                .register_native_grok_submission(view, replacement_generation, "new-session".into())
                .unwrap();

            model.finish_input_submission(view, generation);

            assert!(model.is_input_submission_current(view, replacement_generation));
            assert!(replacement.claim_write());
        });
    });
}

#[test]
fn replacing_listener_revokes_first_write_and_changes_callback_identity() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let view = EntityId::new();
        let (_sender, receiver) = async_channel::unbounded();
        let dispatcher = app.add_model(|ctx| ModelEventDispatcher::new(receiver, sessions, ctx));
        let listener = app
            .add_model(|ctx| CLIAgentSessionListener::new(view, CLIAgent::Grok, &dispatcher, ctx));
        let replacement = app
            .add_model(|ctx| CLIAgentSessionListener::new(view, CLIAgent::Grok, &dispatcher, ctx));
        model.update(&mut app, |model, ctx| {
            let mut session = open_session("native-session");
            session.listener = Some(listener.clone());
            model.set_session(view, session, ctx);
            let generation = model.input_generation(view).unwrap();
            assert!(model.begin_input_submission(view, generation));
            let lease = model
                .register_native_grok_submission(view, generation, "native-session".into())
                .unwrap();

            model.register_listener(
                view,
                CLIAgent::Grok,
                None,
                None,
                Some("native-session".into()),
                None,
                None,
                false,
                replacement.clone(),
                ctx,
            );

            assert!(!lease.claim_write());
            assert!(!lease.was_write_claimed());
            let current = model.session(view).unwrap().listener.as_ref().unwrap().id();
            assert_eq!(current, replacement.id());
            assert_ne!(current, listener.id());
        });
    });
}
