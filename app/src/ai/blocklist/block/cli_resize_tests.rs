use std::cell::RefCell;
use std::rc::Rc;

use warpui::{App, SingletonEntity};

use super::{CLISubagentAction, CLISubagentController, CLISubagentView};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::{BlocklistAIActionModel, BlocklistAIHistoryModel};
use crate::terminal::resizable_data::{ModalSizes, ResizableData};
use crate::test_util::ai_agent_tasks::{create_api_task, create_message};
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, initialize_app_for_terminal_view,
};

#[test]
fn cli_subagent_resize_action_is_remembered_by_new_views() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);
        let (terminal_model, active_session, event_dispatcher, controller) =
            terminal.read(&app, |view, _| {
                (
                    view.model.clone(),
                    view.active_session().clone(),
                    view.model_event_dispatcher().clone(),
                    view.ai_controller().clone(),
                )
            });
        ResizableData::handle(&app).update(&mut app, |data, _| {
            data.insert(window_id, ModalSizes::default());
        });
        let action_model = app.add_model(|ctx| {
            BlocklistAIActionModel::new(
                terminal_model.clone(),
                active_session,
                &event_dispatcher,
                terminal.id(),
                ctx,
            )
        });
        let subagent_controller = app.add_model(|ctx| {
            CLISubagentController::new(
                &controller,
                &action_model,
                None,
                terminal_model.clone(),
                &event_dispatcher,
                terminal.id(),
                ctx,
            )
        });
        let conversation_id = AIConversationId::new();
        let conversation = AIConversation::new_restored(
            conversation_id,
            vec![create_api_task(
                "resize-task",
                vec![create_message("resize-output", "resize-task")],
            )],
            None,
        )
        .expect("浮窗测试对话应恢复成功");
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            history.restore_conversations(terminal.id(), vec![conversation], ctx);
        });
        let block_id = terminal_model.lock().active_block_id().clone();
        let view = app.add_typed_action_view(window_id, |ctx| {
            CLISubagentView::new_restored(
                block_id.clone(),
                action_model.clone(),
                subagent_controller.clone(),
                terminal_model.clone(),
                conversation_id,
                TaskId::new("resize-task".to_string()),
                None,
                None,
                ctx,
            )
        });
        view.update(&mut app, |view, _| {
            view.resizable_width
                .lock()
                .expect("宽度应可访问")
                .set_size(640.);
            view.resizable_height
                .lock()
                .expect("高度应可访问")
                .set_size(480.);
        });

        let requested_save_sizes = Rc::new(RefCell::new(Vec::new()));
        let observed_save_sizes = requested_save_sizes.clone();
        app.add_global_action::<_, (), _>("workspace:save_app", move |_, app| {
            let sizes = ResizableData::as_ref(app)
                .get_all_handles(window_id)
                .expect("保存请求应读取到窗口记忆尺寸");
            observed_save_sizes
                .borrow_mut()
                .push((sizes.cli_subagent_width, sizes.cli_subagent_height));
        });

        // 经实际 typed action 注册与路由执行，不能由直接调用记忆值 setter 替代。
        app.dispatch_typed_action(window_id, &[view.id()], &CLISubagentAction::Resize);

        assert_eq!(*requested_save_sizes.borrow(), vec![(640., 480.)]);
        ResizableData::handle(&app).read(&app, |data, _| {
            let sizes = data.get_all_handles(window_id).expect("窗口记忆尺寸应存在");
            assert_eq!(sizes.cli_subagent_width, 640.);
            assert_eq!(sizes.cli_subagent_height, 480.);
        });
        let next_view = app.add_typed_action_view(window_id, |ctx| {
            CLISubagentView::new_restored(
                block_id,
                action_model,
                subagent_controller,
                terminal_model,
                conversation_id,
                TaskId::new("resize-task".to_string()),
                None,
                None,
                ctx,
            )
        });
        next_view.read(&app, |view, _| {
            assert_eq!(
                view.resizable_width.lock().expect("宽度应可访问").size(),
                640.
            );
            assert_eq!(
                view.resizable_height.lock().expect("高度应可访问").size(),
                480.
            );
        });

        // 新浮窗读取记忆尺寸，但旧浮窗后续的拖拽状态仍保持独立。
        view.update(&mut app, |view, _| {
            view.resizable_width
                .lock()
                .expect("宽度应可访问")
                .set_size(720.);
        });
        next_view.read(&app, |view, _| {
            assert_eq!(
                view.resizable_width.lock().expect("宽度应可访问").size(),
                640.
            );
        });
    });
}
