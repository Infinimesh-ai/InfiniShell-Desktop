use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::AIConversationAutoexecuteMode;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn new_request_conversation_preserves_full_access_mode() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let terminal_view_id = terminal.id();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history.set_default_autoexecute_mode(
                    terminal_view_id,
                    AIConversationAutoexecuteMode::FullAccess,
                );
            });

            let controller = terminal.ai_controller();
            let context_model = controller.as_ref(ctx).context_model.clone();
            context_model.update(ctx, |context_model, ctx| {
                context_model
                    .set_pending_query_state_for_new_conversation(AgentViewEntryOrigin::Cli, ctx);
            });

            let mode = controller.update(ctx, |controller, ctx| {
                controller
                    .start_new_conversation_for_request(ctx)
                    .autoexecute_override()
            });
            assert_eq!(mode, AIConversationAutoexecuteMode::FullAccess);
        });
    });
}
