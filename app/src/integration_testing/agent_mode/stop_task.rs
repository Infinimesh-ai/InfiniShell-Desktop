//! 为真实 PTY 停止测试建立本地会话，不访问模型提供方。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::Local;
use warpui::integration::{AssertionCallback, AssertionOutcome, TestStep};
use warpui::{App, SingletonEntity, WindowId, async_assert};

use crate::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentInput, UserQueryMode};
use crate::ai::blocklist::{RequestInput, ResponseStreamId};
use crate::ai::llms::LLMId;
use crate::integration_testing::view_getters::single_terminal_view_for_tab;
use crate::terminal::model::block::{BlockId, BlockState};
use crate::terminal::view::Event;

/// 记录同一个真实命令的生命周期，防止点击前已退出导致停止测试误报通过。
#[derive(Clone, Default)]
pub struct MonitoringCommandFixture {
    target: Rc<RefCell<Option<(BlockId, AIConversationId)>>>,
    interrupt_count: Rc<Cell<usize>>,
    command_finished: Rc<Cell<bool>>,
}

impl MonitoringCommandFixture {
    /// 保留真实 shell 命令，建立缺失有效子任务和响应流的监控会话。
    pub fn attach_to_running_command(&self) -> TestStep {
        let fixture = self.clone();
        TestStep::new("建立缺失响应流的长命令监控会话")
            .with_action(move |app, window_id, _| {
                let terminal = single_terminal_view_for_tab(app, window_id, 0);
                terminal.update(app, |view, ctx| {
                    let terminal_id = view.id();
                    let conversation_id =
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                            let conversation_id = history.start_new_conversation(
                                terminal_id,
                                false,
                                false,
                                false,
                                ctx,
                            );
                            let conversation = history
                                .conversation_mut(&conversation_id)
                                .expect("测试会话应存在");
                            let task_id = conversation.get_root_task_id().clone();
                            conversation
                                .update_for_new_request_input(
                                    RequestInput {
                                        conversation_id,
                                        input_messages: HashMap::from([(
                                            task_id,
                                            vec![AIAgentInput::UserQuery {
                                                query: "每 10 秒刷新一次运行状态".to_owned(),
                                                context: Default::default(),
                                                static_query_type: None,
                                                referenced_attachments: Default::default(),
                                                user_query_mode: UserQueryMode::Normal,
                                                running_command: None,
                                                intended_agent: None,
                                            }],
                                        )]),
                                        working_directory: None,
                                        model_id: LLMId::from("test-model"),
                                        coding_model_id: LLMId::from("test-model"),
                                        cli_agent_model_id: LLMId::from("test-model"),
                                        computer_use_model_id: LLMId::from("test-model"),
                                        shared_session_response_initiator: None,
                                        request_start_ts: Local::now(),
                                        supported_tools_override: None,
                                    },
                                    ResponseStreamId::new_local(),
                                    terminal_id,
                                    ctx,
                                )
                                .expect("测试请求应加入会话");
                            history.set_active_conversation_id(conversation_id, terminal_id, ctx);
                            conversation_id
                        });
                    {
                        let mut model = view.model.lock();
                        let block = model.block_list_mut().active_block_mut();
                        block.set_agent_interaction_mode_for_requested_command(
                            AIAgentActionId::from("monitor-command".to_owned()),
                            None,
                            conversation_id,
                        );
                        block
                            .set_agent_interaction_mode_for_agent_monitored_command(
                                &TaskId::new("missing-cli-task".to_owned()),
                                conversation_id,
                            )
                            .expect("运行中的命令应交由 Agent 控制");
                        *fixture.target.borrow_mut() = Some((block.id().clone(), conversation_id));
                    }
                    view.input().update(ctx, |input, ctx| {
                        // 预备步骤用用户输入启动真实 shell，编辑器会等命令结束才清空。
                        // 转成 Agent 命令后产品会保留用户草稿；先移除这份人工夹具残留，
                        // 对齐真实 Agent 执行命令时并未把命令键入用户编辑器的状态。
                        input.replace_buffer_content("", ctx);
                        input.set_input_mode_agent(false, ctx);
                    });
                    ctx.notify();
                });
                let observed = fixture.clone();
                app.update(|ctx| {
                    ctx.subscribe_to_view(&terminal, move |_, event, _| match event {
                        Event::WriteBytesToPty { bytes } if bytes.as_ref() == [0x03] => {
                            observed
                                .interrupt_count
                                .set(observed.interrupt_count.get() + 1);
                        }
                        Event::BlockCompleted { block, .. }
                            if observed
                                .target
                                .borrow()
                                .as_ref()
                                .is_some_and(|(id, _)| id == &block.id) =>
                        {
                            observed.command_finished.set(true);
                        }
                        _ => {}
                    });
                });
            })
            .add_named_assertion("同一监控命令仍执行且未被提前中断", self.running_assertion())
    }

    fn check_running(&self, app: &App, window_id: WindowId) -> Result<(), String> {
        let target = self.target.borrow();
        let (expected_block_id, conversation_id) = target.as_ref().ok_or("尚未建立监控夹具")?;
        let terminal = single_terminal_view_for_tab(app, window_id, 0);
        terminal.read(app, |view, ctx| {
            let status = BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id).map(|conversation| conversation.status());
            let model = view.model.lock();
            let block = model.block_list().active_block();
            if block.id() == expected_block_id && block.is_executing()
                && block.is_agent_in_control() && status == Some(&ConversationStatus::InProgress)
                && self.interrupt_count.get() == 0 && !self.command_finished.get()
            {
                Ok(())
            } else {
                Err(format!("点击前必须是原监控命令仍在执行：expected={expected_block_id:?}, active={:?}, state={:?}, control={:?}, status={status:?}, interrupts={}, finished={}",
                    block.id(), block.state(), block.long_running_control_state(), self.interrupt_count.get(), self.command_finished.get()))
            }
        })
    }

    pub fn running_assertion(&self) -> AssertionCallback {
        let fixture = self.clone();
        Box::new(
            move |app, window_id| match fixture.check_running(app, window_id) {
                Ok(()) => AssertionOutcome::Success,
                Err(message) => AssertionOutcome::immediate_failure(message),
            },
        )
    }

    /// 在鼠标事件生成的瞬间再检查一次，避免截图步骤与点击之间的提前退出。
    pub fn stop_button_position(&self, app: &App, window_id: WindowId) -> String {
        self.check_running(app, window_id)
            .expect("停止按钮点击前的命令状态错误");
        "agent_stop_task_button".to_owned()
    }

    pub fn stopped_assertion(&self) -> AssertionCallback {
        let fixture = self.clone();
        Box::new(move |app, window_id| {
            let target = fixture.target.borrow();
            let (block_id, conversation_id) = target.as_ref().expect("监控夹具应已建立");
            let terminal = single_terminal_view_for_tab(app, window_id, 0);
            terminal.read(app, |view, ctx| {
                let status = BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id).map(|conversation| conversation.status());
                let input = view.input().as_ref(ctx).buffer_text(ctx);
                let model = view.model.lock();
                let active_block = model.block_list().active_block();
                let stopped_block = model.block_list().block_with_id(block_id);
                async_assert!(
                    status == Some(&ConversationStatus::Cancelled)
                        && stopped_block.is_some_and(|block| matches!(block.state(), BlockState::DoneWithExecution))
                        && active_block.id() != block_id
                        && !active_block.started() && matches!(active_block.state(), BlockState::BeforeExecution)
                        && active_block.has_received_precmd()
                        && fixture.interrupt_count.get() == 1 && fixture.command_finished.get()
                        && input.is_empty(),
                    "停止后应中断原命令一次、保留Cancelled并返回空提示符：status={status:?}, active={:?}, state={:?}, interrupts={}, finished={}, input={input:?}",
                    active_block.id(), active_block.state(), fixture.interrupt_count.get(), fixture.command_finished.get()
                )
            })
        })
    }

    /// 捕获 SIGINT 的 REPL 继续存活时，键盘应直接送达原命令。
    pub fn stopped_running_assertion(
        &self,
        expected_output: Option<&'static str>,
    ) -> AssertionCallback {
        let fixture = self.clone();
        Box::new(move |app, window_id| {
            let target = fixture.target.borrow();
            let (block_id, conversation_id) = target.as_ref().expect("监控夹具应已建立");
            let terminal = single_terminal_view_for_tab(app, window_id, 0);
            terminal.read(app, |view, ctx| {
                let status = BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
                    .map(|conversation| conversation.status());
                let focused = ctx.focused_view_id(window_id);
                let model = view.model.lock();
                let block = model.block_list().active_block();
                let output = block.output_with_secrets_unobfuscated();
                async_assert!(
                    block.id() == block_id && block.is_executing()
                        && !block.is_agent_in_control()
                        && status == Some(&ConversationStatus::Cancelled)
                        && fixture.interrupt_count.get() == 1 && !fixture.command_finished.get()
                        && focused == Some(terminal.id())
                        && expected_output.is_none_or(|expected| output.contains(expected)),
                    "停止后 REPL 应保持原命令、接收键盘并产生预期输出：active={:?}, state={:?}, status={status:?}, focused={focused:?}, interrupts={}, output={output:?}",
                    block.id(), block.state(), fixture.interrupt_count.get()
                )
            })
        })
    }
}
