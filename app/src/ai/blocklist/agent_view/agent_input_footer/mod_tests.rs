use super::*;
use crate::terminal::cli_agent_sessions::event::{CLI_AGENT_NOTIFICATION_SENTINEL, parse_event};
use crate::terminal::cli_agent_sessions::{
    CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use warp_core::settings::Setting as _;
use warpui::App;

fn remote_codex_footer(
    app: &mut App,
    plugin_version: Option<&str>,
) -> ViewHandle<AgentInputFooter> {
    initialize_app_for_terminal_view(app);
    FeatureFlag::HOANotifications.set_enabled(true);
    FeatureFlag::CodexNotifications.set_enabled(true);
    FeatureFlag::CodexPlugin.set_enabled(true);
    assert!(plugin_manager_for(CLIAgent::Codex).is_some());
    AISettings::handle(app).update(app, |settings, ctx| {
        settings
            .show_agent_notifications
            .set_value(true, ctx)
            .unwrap();
    });
    let terminal = add_window_with_terminal(app, None);
    let terminal_view_id = terminal.id();
    CLIAgentSessionsModel::handle(app).update(app, |sessions, ctx| {
        sessions.set_session(terminal_view_id, CLIAgentSession {
            agent: CLIAgent::Codex,
            status: CLIAgentSessionStatus::Unknown,
            session_context: CLIAgentSessionContext::default(),
            input_state: CLIAgentInputState::Closed,
            should_auto_toggle_input: false,
            listener: None,
            remote_host: Some("isolated-host".to_owned()),
            plugin_version: plugin_version.map(str::to_owned),
            draft_text: None,
            custom_command_prefix: None,
            received_rich_notification: false,
        }, ctx);
        // 重连首先收到回合事件，没有重新执行 SessionStart，也没有版本字段。
        let event = parse_event(
            Some(CLI_AGENT_NOTIFICATION_SENTINEL),
            r#"{"v":1,"agent":"codex","event":"prompt_submit","session_id":"same-native-session","turn_id":"second-turn"}"#,
        ).unwrap();
        sessions.update_from_event(terminal_view_id, &event, ctx);
    });
    terminal.read(app, |view, ctx| {
        view.input().as_ref(ctx).agent_input_footer().clone()
    })
}

#[test]
fn tmux_reconnect_without_session_start_keeps_plugin_version_unknown() {
    App::test((), |mut app| async move {
        let footer = remote_codex_footer(&mut app, None);
        footer.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.terminal_view_id)
                .unwrap();
            assert!(session.received_rich_notification);
            assert_eq!(session.plugin_version, None);
            assert_eq!(view.plugin_chip_kind(ctx), None);
        });
    });
}

#[test]
fn known_old_remote_plugin_still_shows_update_after_reconnect() {
    App::test((), |mut app| async move {
        let footer = remote_codex_footer(&mut app, Some("0.1.0"));
        footer.read(&app, |view, ctx| {
            assert_eq!(view.plugin_chip_kind(ctx), Some(PluginChipKind::Update));
        });
    });
}
