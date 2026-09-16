//! 历史结果只改变查看位置，不改变执行目标；复制使用完整持久记录。

use uuid::Uuid;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{ChildView, Flex, ParentElement};
use warpui::{AppContext, Element, SingletonEntity, ViewContext, ViewHandle};

use super::{LocalCLITaskManagerView, TaskManagerAction, task_state_name};
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
use crate::appearance::Appearance;
use crate::persistence::local_cli_tasks::load_task_generations;
use crate::persistence::model::LocalCliTask;
use crate::view_components::action_button::{ActionButton, SecondaryTheme};
use crate::view_components::{Dropdown, DropdownItem};

pub(super) struct ResultHistory {
    pub scope: Option<(String, i64)>,
    pub token: Uuid,
    pub records: Vec<LocalCliTask>,
    pub selected: Option<i64>,
    pub loading: bool,
    reload_pending: bool,
    selector: ViewHandle<Dropdown<TaskManagerAction>>,
    copy: ViewHandle<ActionButton>,
}

impl ResultHistory {
    pub fn new(ctx: &mut ViewContext<LocalCLITaskManagerView>) -> Self {
        Self {
            scope: None,
            token: Uuid::new_v4(),
            records: Vec::new(),
            selected: None,
            loading: false,
            reload_pending: false,
            selector: ctx.add_typed_action_view(Dropdown::new),
            copy: ctx.add_typed_action_view(|_| {
                ActionButton::new(
                    crate::t!("cli-task-manager-copy-history-result"),
                    SecondaryTheme,
                )
            }),
        }
    }

    fn selected_record(&self) -> Option<&LocalCliTask> {
        self.records
            .iter()
            .find(|record| Some(record.generation) == self.selected)
    }
}

impl LocalCLITaskManagerView {
    pub(super) fn refresh_result_history(&mut self, ctx: &mut ViewContext<Self>) {
        let scope = self
            .selected_record(ctx)
            .map(|task| (task.task_id, task.generation));
        if self.result_history.loading && self.result_history.scope == scope {
            self.result_history.reload_pending = true;
            return;
        }
        self.result_history.reload_pending = false;
        if self.result_history.scope.as_ref().map(|(task, _)| task)
            != scope.as_ref().map(|(task, _)| task)
        {
            self.result_history.records.clear();
            self.result_history.selected = None;
        }
        self.result_history.scope = scope.clone();
        self.result_history.token = Uuid::new_v4();
        self.result_history.loading = false;
        let Some((task_id, active_generation)) = scope else {
            return;
        };
        let Some(sender) = LocalCLITaskCoordinator::as_ref(ctx).sender() else {
            return;
        };
        let token = self.result_history.token;
        let requested_task = task_id.clone();
        self.result_history.loading = true;
        self.result_history
            .copy
            .update(ctx, |button, ctx| button.set_disabled(true, ctx));
        ctx.spawn(
            async move {
                load_task_generations(&sender, requested_task)?
                    .await
                    .map_err(|_| crate::t!("cli-task-manager-history-read-failed"))?
            },
            move |view, result, ctx| {
                if !view.result_history_action_is_current(&task_id, active_generation, token, ctx) {
                    return;
                }
                view.result_history.loading = false;
                let reload_pending = std::mem::take(&mut view.result_history.reload_pending);
                match result {
                    Ok(records) => view.set_result_history_records(records, ctx),
                    Err(error) => view.error = Some(error),
                }
                if reload_pending {
                    view.refresh_result_history(ctx);
                }
                ctx.notify();
            },
        );
    }

    pub(super) fn set_result_history_records(
        &mut self,
        records: Vec<LocalCliTask>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some((task_id, active_generation)) = self.result_history.scope.clone() else {
            return;
        };
        // 存储返回若混入其他任务，不允许其结果变成当前任务的历史。
        let mut records = records
            .into_iter()
            .filter(|record| record.task_id == task_id)
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.generation);
        records.dedup_by_key(|record| record.generation);
        if !records
            .iter()
            .any(|record| Some(record.generation) == self.result_history.selected)
        {
            self.result_history.selected = records
                .iter()
                .rev()
                .find(|record| record.generation < active_generation)
                .or_else(|| records.last())
                .map(|record| record.generation);
        }
        self.result_history.records = records;
        let token = self.result_history.token;
        let items = self
            .result_history
            .records
            .iter()
            .rev()
            .map(|record| {
                DropdownItem::new(
                    crate::t!(
                        "cli-task-manager-history-generation",
                        generation = record.generation,
                        state = task_state_name(record.state)
                    ),
                    TaskManagerAction::SelectResultGeneration {
                        task_id: task_id.clone(),
                        active_generation,
                        result_generation: record.generation,
                        token,
                    },
                )
            })
            .collect();
        self.result_history
            .selector
            .update(ctx, |selector, ctx| selector.set_items(items, ctx));
        self.refresh_history_copy_button(ctx);
    }

    fn result_history_action_is_current(
        &self,
        task_id: &str,
        active_generation: i64,
        token: Uuid,
        ctx: &AppContext,
    ) -> bool {
        self.result_history.token == token
            && self
                .result_history
                .scope
                .as_ref()
                .is_some_and(|(id, generation)| id == task_id && *generation == active_generation)
            && self
                .selected_record(ctx)
                .is_some_and(|task| task.task_id == task_id && task.generation == active_generation)
    }

    fn refresh_history_copy_button(&mut self, ctx: &mut ViewContext<Self>) {
        let result = self
            .result_history
            .selected_record()
            .map(|record| (record.generation, record.result.is_some()));
        let Some((task_id, active_generation)) = self.result_history.scope.clone() else {
            return;
        };
        let token = self.result_history.token;
        if let Some((result_generation, _)) = result {
            self.result_history.selector.update(ctx, |selector, ctx| {
                selector.set_selected_by_action(
                    TaskManagerAction::SelectResultGeneration {
                        task_id: task_id.clone(),
                        active_generation,
                        result_generation,
                        token,
                    },
                    ctx,
                )
            });
        }
        self.result_history.copy.update(ctx, |button, ctx| {
            button.set_disabled(
                self.result_history.loading || !result.is_some_and(|(_, present)| present),
                ctx,
            );
            if let Some((result_generation, _)) = result {
                button.set_on_click(
                    move |ctx| {
                        ctx.dispatch_typed_action(TaskManagerAction::CopyHistoricalResult {
                            task_id: task_id.clone(),
                            active_generation,
                            result_generation,
                            token,
                        })
                    },
                    ctx,
                );
            }
        });
    }

    pub(super) fn select_result_generation(
        &mut self,
        task_id: &str,
        active_generation: i64,
        result_generation: i64,
        token: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if !self.result_history_action_is_current(task_id, active_generation, token, ctx)
            || !self
                .result_history
                .records
                .iter()
                .any(|record| record.generation == result_generation)
        {
            return Err(crate::t!("cli-task-manager-history-selection-expired"));
        }
        self.result_history.selected = Some(result_generation);
        self.refresh_history_copy_button(ctx);
        Ok(())
    }

    pub(super) fn copy_historical_result(
        &self,
        task_id: &str,
        active_generation: i64,
        result_generation: i64,
        token: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if !self.result_history_action_is_current(task_id, active_generation, token, ctx) {
            return Err(crate::t!("cli-task-manager-history-selection-expired"));
        }
        let result = self
            .result_history
            .records
            .iter()
            .find(|record| record.generation == result_generation)
            .and_then(|record| record.result.as_ref())
            .ok_or_else(|| crate::t!("cli-task-manager-history-no-result"))?;
        ctx.clipboard()
            .write(ClipboardContent::plain_text(result.clone()));
        Ok(())
    }

    pub(super) fn render_result_history(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut body = Flex::column();
        if self.result_history.scope.is_none() {
            return body.finish();
        }
        body.add_child(self.text(crate::t!("cli-task-manager-history-title"), appearance));
        if self.result_history.loading {
            body.add_child(self.text(crate::t!("cli-task-manager-history-loading"), appearance));
        }
        if self.result_history.records.is_empty() {
            body.add_child(self.text(crate::t!("cli-task-manager-history-empty"), appearance));
            return body.finish();
        }
        body.add_child(ChildView::new(&self.result_history.selector).finish());
        if let Some(record) = self.result_history.selected_record() {
            let active_generation = self
                .result_history
                .scope
                .as_ref()
                .map(|(_, generation)| *generation)
                .unwrap_or(record.generation);
            body.add_child(self.text(
                crate::t!(
                    "cli-task-manager-history-read-only",
                    generation = record.generation,
                    active_generation = active_generation
                ),
                appearance,
            ));
            body.add_child(self.text(
                crate::t!(
                        "cli-task-manager-history-details",
                        state = task_state_name(record.state),
                        session = record
                            .native_session_id
                            .clone()
                            .unwrap_or_else(|| crate::t!("cli-task-manager-session-pending"))
                    ),
                appearance,
            ));
            match &record.result {
                Some(result) => {
                    let preview = result.chars().take(32768).collect::<String>();
                    if preview.len() < result.len() {
                        body.add_child(
                            self.text(crate::t!("cli-task-manager-output-truncated"), appearance),
                        );
                    }
                    body.add_child(self.text(preview, appearance));
                    body.add_child(ChildView::new(&self.result_history.copy).finish());
                }
                None => {
                    body.add_child(
                        self.text(crate::t!("cli-task-manager-history-no-result"), appearance),
                    );
                }
            }
        }
        body.finish()
    }
}
