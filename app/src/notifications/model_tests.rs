use warpui::{App, ModelHandle};

use super::*;
use crate::notifications::item::NotificationFilter;

fn initialize_notifications(app: &mut App) -> ModelHandle<NotificationsModel> {
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(NotificationsModel::new)
}

#[test]
fn cli_session_attention_uses_existing_request_notification_without_actions() {
    let _guard = FeatureFlag::HOANotifications.override_enabled(true);
    App::test((), |mut app| async move {
        let model = initialize_notifications(&mut app);
        let terminal_view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            let event = CLIAgentSessionsModelEvent::AttentionRequested {
                terminal_view_id,
                agent: CLIAgent::Grok,
            };
            model.handle_cli_agent_session_event(&event, ctx);
            model.handle_cli_agent_session_event(&event, ctx);

            assert_eq!(
                model.notifications.filtered_count(NotificationFilter::All),
                1
            );
            let item = model
                .notifications
                .items_filtered(NotificationFilter::All)
                .next()
                .unwrap();
            assert_eq!(item.category, NotificationCategory::Request);
            assert_eq!(
                item.origin,
                NotificationOrigin::CLISession(terminal_view_id)
            );
            assert_eq!(item.terminal_view_id, terminal_view_id);
            assert!(matches!(
                item.agent,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::Grok,
                    is_ambient: false,
                }
            ));
            assert_eq!(
                item.title,
                crate::t!(
                    "notifications-agent-needs-attention-title",
                    agent = CLIAgent::Grok.display_name()
                )
            );
            assert_eq!(item.message, crate::t!("notifications-waiting-for-input"));
            assert!(item.artifacts.is_empty());
        });
    });
}

#[test]
fn cli_session_update_clears_only_its_own_attention_notification() {
    let _guard = FeatureFlag::HOANotifications.override_enabled(true);
    App::test((), |mut app| async move {
        let model = initialize_notifications(&mut app);
        let terminal_view_id = EntityId::new();
        let other_terminal_view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            for id in [terminal_view_id, other_terminal_view_id] {
                model.handle_cli_agent_session_event(
                    &CLIAgentSessionsModelEvent::AttentionRequested {
                        terminal_view_id: id,
                        agent: CLIAgent::Grok,
                    },
                    ctx,
                );
            }
            model.handle_cli_agent_session_event(
                &CLIAgentSessionsModelEvent::SessionUpdated {
                    terminal_view_id,
                    agent: CLIAgent::Grok,
                },
                ctx,
            );

            assert_eq!(
                model.notifications.filtered_count(NotificationFilter::All),
                1
            );
            let item = model
                .notifications
                .items_filtered(NotificationFilter::All)
                .next()
                .unwrap();
            assert_eq!(
                item.origin,
                NotificationOrigin::CLISession(other_terminal_view_id)
            );
            assert!(
                !model
                    .session_attention_notifications
                    .contains_key(&terminal_view_id)
            );
        });
    });
}

#[test]
fn cli_session_update_preserves_status_notifications_that_replace_attention() {
    let _guard = FeatureFlag::HOANotifications.override_enabled(true);
    App::test((), |mut app| async move {
        let model = initialize_notifications(&mut app);
        let terminal_view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            for (status, category) in [
                (
                    CLIAgentSessionStatus::Success,
                    NotificationCategory::Complete,
                ),
                (
                    CLIAgentSessionStatus::Failed {
                        error_type: None,
                        message: None,
                    },
                    NotificationCategory::Error,
                ),
                (
                    CLIAgentSessionStatus::Cancelled,
                    NotificationCategory::Complete,
                ),
                (
                    CLIAgentSessionStatus::Unknown,
                    NotificationCategory::Request,
                ),
                (
                    CLIAgentSessionStatus::Disconnected,
                    NotificationCategory::Request,
                ),
            ] {
                model.handle_cli_agent_session_event(
                    &CLIAgentSessionsModelEvent::AttentionRequested {
                        terminal_view_id,
                        agent: CLIAgent::Grok,
                    },
                    ctx,
                );
                model.handle_cli_agent_session_event(
                    &CLIAgentSessionsModelEvent::StatusChanged {
                        terminal_view_id,
                        agent: CLIAgent::Grok,
                        status,
                        session_context: Default::default(),
                    },
                    ctx,
                );
                let status_id = model
                    .notifications
                    .items_filtered(NotificationFilter::All)
                    .next()
                    .unwrap()
                    .id;
                model.handle_cli_agent_session_event(
                    &CLIAgentSessionsModelEvent::SessionUpdated {
                        terminal_view_id,
                        agent: CLIAgent::Grok,
                    },
                    ctx,
                );

                assert_eq!(
                    model.notifications.filtered_count(NotificationFilter::All),
                    1
                );
                assert_eq!(
                    model.notifications.get_by_id(status_id).unwrap().category,
                    category
                );
                assert!(model.session_attention_notifications.is_empty());
            }
        });
    });
}
