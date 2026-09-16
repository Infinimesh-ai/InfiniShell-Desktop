//! 将本地 CLI 会话变更串行提交到现有 SQLite writer，避免异步完成顺序覆盖终态。

use std::sync::mpsc::SyncSender;

use async_channel::Sender;
use uuid::Uuid;
use warpui::{EntityId, ModelContext};

use super::{
    CLIAgentSessionContext, CLIAgentSessionStatus, CLIAgentSessionsModel,
    CLIAgentSessionsModelEvent,
};
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::{checkpoint_task, load_tasks};
use crate::persistence::model::{LocalCliTask, LocalCliTaskState};

pub(super) struct TaskBinding {
    token: Uuid,
    sender: Sender<TaskUpdate>,
}

struct TaskUpdate {
    status: CLIAgentSessionStatus,
    context: CLIAgentSessionContext,
    evidence: Option<String>,
}

impl CLIAgentSessionsModel {
    /// 调用方必须先确认初始任务已提交，再绑定新建 pane；此方法不会启动进程。
    pub(crate) fn bind_local_task(
        &mut self,
        terminal_view_id: EntityId,
        task: LocalCliTask,
        sender: SyncSender<ModelEvent>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        if self.local_task_bindings.contains_key(&terminal_view_id) {
            return Err(crate::t!("cli-agent-task-already-bound"));
        }
        let (updates, receiver) = async_channel::bounded(256);
        let token = Uuid::new_v4();
        self.local_task_bindings.insert(
            terminal_view_id,
            TaskBinding {
                token,
                sender: updates,
            },
        );
        ctx.spawn(
            async move {
                let mut task = task;
                while let Ok(update) = receiver.recv().await {
                    apply_update(&sender, &mut task, update).await?;
                }
                Ok::<(), String>(())
            },
            move |model, result, ctx| {
                if result.is_err()
                    && model
                        .local_task_bindings
                        .get(&terminal_view_id)
                        .is_some_and(|binding| binding.token == token)
                {
                    model.local_task_bindings.remove(&terminal_view_id);
                    model.report_local_task_persistence_failure(terminal_view_id, ctx);
                }
            },
        );
        Ok(())
    }

    pub(super) fn persist_local_task_status(
        &mut self,
        terminal_view_id: EntityId,
        status: CLIAgentSessionStatus,
        context: CLIAgentSessionContext,
        evidence: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(binding) = self.local_task_bindings.get(&terminal_view_id) else {
            return;
        };
        if binding
            .sender
            .try_send(TaskUpdate {
                status,
                context,
                evidence,
            })
            .is_err()
        {
            self.local_task_bindings.remove(&terminal_view_id);
            self.report_local_task_persistence_failure(terminal_view_id, ctx);
        }
    }

    fn report_local_task_persistence_failure(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        session.status = CLIAgentSessionStatus::Unknown;
        session.session_context.summary = Some(crate::t!("cli-agent-task-save-failed"));
        ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
            terminal_view_id,
            agent: session.agent,
            status: session.status.clone(),
            session_context: Box::new(session.session_context.clone()),
        });
    }

    /// 只恢复本地记录，不依据 PID 重连，也不重发历史输入。
    pub(crate) fn restore_local_tasks(
        &mut self,
        sender: SyncSender<ModelEvent>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(
            async move {
                load_tasks(&sender, true)?
                    .await
                    .map_err(|_| "任务恢复确认通道已关闭".to_owned())?
            },
            |model, result, ctx| {
                match result {
                    Ok(tasks) => {
                        model.restored_local_tasks = tasks;
                        #[cfg(not(target_family = "wasm"))]
                        {
                            use warpui::SingletonEntity;
                            crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator::handle(ctx)
                                .update(ctx, |coordinator, ctx| coordinator.refresh_records(ctx));
                        }
                    }
                    Err(_) => model.local_task_recovery_failed = true,
                }
                ctx.notify();
            },
        );
    }

    pub(crate) fn restored_local_tasks(&self) -> &[LocalCliTask] {
        &self.restored_local_tasks
    }

    pub(crate) fn local_task_recovery_failed(&self) -> bool {
        self.local_task_recovery_failed
    }
}

async fn apply_update(
    sender: &SyncSender<ModelEvent>,
    task: &mut LocalCliTask,
    update: TaskUpdate,
) -> Result<(), String> {
    let next_state = match update.status {
        CLIAgentSessionStatus::InProgress => LocalCliTaskState::Running,
        CLIAgentSessionStatus::Success
            if update.evidence.is_some() && update.context.session_id.is_some() =>
        {
            LocalCliTaskState::Completed
        }
        CLIAgentSessionStatus::Success | CLIAgentSessionStatus::Unknown => {
            LocalCliTaskState::Unconfirmed
        }
        CLIAgentSessionStatus::Failed { .. } => LocalCliTaskState::Failed,
        CLIAgentSessionStatus::Blocked { .. } => LocalCliTaskState::WaitingForUser,
        CLIAgentSessionStatus::Cancelled => LocalCliTaskState::Cancelled,
        CLIAgentSessionStatus::Disconnected => LocalCliTaskState::Disconnected,
    };
    if task.state.is_terminal() {
        if next_state != LocalCliTaskState::Running {
            return Ok(());
        }
        let previous_generation = task.generation;
        task.generation += 1;
        task.revision = 0;
        task.state = LocalCliTaskState::Queued;
        task.result = None;
        task.terminal_evidence = None;
        commit(sender, task.clone(), previous_generation).await?;
    }
    let native_session_id = update.context.session_id.or(task.native_session_id.clone());
    let result = update.context.response.or(task.result.clone());
    let evidence = if next_state.is_terminal() {
        update.evidence.or(task.terminal_evidence.clone())
    } else {
        task.terminal_evidence.clone()
    };
    if next_state == task.state
        && native_session_id == task.native_session_id
        && result == task.result
        && evidence == task.terminal_evidence
    {
        return Ok(());
    }
    task.revision += 1;
    task.state = next_state;
    task.native_session_id = native_session_id;
    task.result = result;
    task.terminal_evidence = evidence;
    commit(sender, task.clone(), task.generation).await
}

async fn commit(
    sender: &SyncSender<ModelEvent>,
    task: LocalCliTask,
    previous_generation: i64,
) -> Result<(), String> {
    checkpoint_task(sender, task, Some(previous_generation))?
        .await
        .map_err(|_| "任务提交确认通道已关闭".to_owned())?
}

#[cfg(test)]
#[path = "local_tasks_tests.rs"]
mod tests;
