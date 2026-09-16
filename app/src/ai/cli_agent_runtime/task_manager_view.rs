//! 本机托管任务面板；视图关闭不关闭任务，输入只在原生确认后清除。

#[path = "task_manager_history.rs"]
mod history;
#[path = "task_manager_input.rs"]
mod input;

use ai::skills::SkillReference;
use history::ResultHistory;
use input::{ComposerSubmission, ManagedInputState, PreparedInputTarget};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, Flex,
    ParentElement, ScrollbarWidth,
};
use warpui::keymap::FixedBinding;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::coordinator::{
    LocalCLITaskCoordinator, LocalCLITaskCoordinatorEvent, ManagedTaskSnapshot,
};
use super::local_skills::SelectedLocalSkill;
use super::local_tools::LocalToolPermissions;
use super::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeEventKind,
    SessionOptions, SessionTarget,
};
use crate::appearance::Appearance;
use crate::editor::{
    EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent, InteractionState,
    SingleLineEditorOptions,
};
use crate::persistence::local_cli_tasks::load_task_messages;
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliReceiptKind, LocalCliTask, LocalCliTaskState,
};
use crate::terminal::cli_agent::{CLIAgent, CLIAgentInstallModel, CLIAgentVersionStatus};
use crate::view_components::action_button::{
    ActionButton, DangerPrimaryTheme, NakedTheme, PrimaryTheme, SecondaryTheme,
};

pub(crate) fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        TaskManagerAction::Close,
        id!(LocalCLITaskManagerView::ui_name()),
    )]);
}

pub(crate) enum LocalCLITaskManagerEvent {
    Close,
    ImportReview {
        draft_key: String,
        input_generation: Uuid,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TaskManagerAction {
    New,
    Start,
    Send,
    Resume,
    Refresh,
    Close,
    AttachFiles,
    ImportReview,
    SelectSkill {
        reference: SkillReference,
        input_generation: Uuid,
    },
    RemoveImage {
        index: usize,
        input_generation: Uuid,
        revision: Uuid,
    },
    RemoveSkill {
        index: usize,
        input_generation: Uuid,
        revision: Uuid,
    },
    DiscardDraft,
    CopyResult,
    SelectResultGeneration {
        task_id: String,
        active_generation: i64,
        result_generation: i64,
        token: Uuid,
    },
    CopyHistoricalResult {
        task_id: String,
        active_generation: i64,
        result_generation: i64,
        token: Uuid,
    },
    SelectTask(String),
    SelectHarness(Harness),
    SelectPermission(PermissionPolicy),
    ToggleSpawn,
    ToggleMessages,
    Approve {
        task_id: String,
        task_generation: i64,
        approval_id: String,
        decision: ApprovalDecision,
    },
    Control {
        task_id: String,
        task_generation: i64,
        key: String,
        action: RuntimeAction,
    },
    CopyInput {
        task_id: String,
        task_generation: i64,
        message_id: String,
    },
    RestoreInput {
        task_id: String,
        task_generation: i64,
        message_id: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedLaunchOptions {
    permission_policy: PermissionPolicy,
    #[serde(default)]
    permission_ceiling: Option<super::permissions::ParentPermissionCeiling>,
    model: Option<String>,
    #[serde(default)]
    local_tools: Option<LocalToolPermissions>,
    #[serde(default)]
    selected_skills: Vec<SelectedLocalSkill>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputPhase {
    WaitingReady,
    WaitingAck,
    Uncertain,
}

struct PendingInput {
    message_id: Uuid,
    task_generation: i64,
    text: String,
    phase: InputPhase,
    composer: Option<ComposerSubmission>,
}

struct ApprovalButtons {
    task_id: String,
    task_generation: i64,
    approval_id: String,
    allow: ViewHandle<ActionButton>,
    deny: ViewHandle<ActionButton>,
}

struct MessageButtons {
    copy: ViewHandle<ActionButton>,
    restore: ViewHandle<ActionButton>,
}

pub(crate) struct LocalCLITaskManagerView {
    harness: Harness,
    permission: PermissionPolicy,
    local_tools: LocalToolPermissions,
    selected_task: Option<String>,
    input_generation: Uuid,
    managed_input: ManagedInputState,
    directory: ViewHandle<EditorView>,
    prompt: ViewHandle<EditorView>,
    drafts: HashMap<String, String>,
    pending_inputs: HashMap<String, PendingInput>,
    pending_controls: HashMap<String, Uuid>,
    error: Option<String>,
    buttons: HashMap<&'static str, ViewHandle<ActionButton>>,
    harness_buttons: Vec<(Harness, ViewHandle<ActionButton>)>,
    permission_buttons: Vec<(PermissionPolicy, ViewHandle<ActionButton>)>,
    task_buttons: BTreeMap<String, ViewHandle<ActionButton>>,
    approval_buttons: Vec<ApprovalButtons>,
    saved_messages: Vec<LocalCliMessage>,
    message_buttons: Vec<MessageButtons>,
    message_scope: Option<(String, i64)>,
    message_load_token: Uuid,
    message_loading: bool,
    message_reload_pending: bool,
    observed_committed_task: Option<LocalCliTask>,
    result_history: ResultHistory,
    body_scroll: ClippedScrollStateHandle,
    prompt_scroll: ClippedScrollStateHandle,
}

impl LocalCLITaskManagerView {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        let directory = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    soft_wrap: true,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(crate::t!("cli-task-manager-directory-placeholder"), ctx);
            editor
        });
        let prompt = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::new(
                EditorOptions {
                    soft_wrap: true,
                    autogrow: true,
                    single_line: false,
                    supports_vim_mode: false,
                    enter_settings: EnterSettings {
                        enter: EnterAction::InsertNewLineIfMultiLine,
                        shift_enter: EnterAction::InsertNewLineIfMultiLine,
                        alt_enter: EnterAction::InsertNewLineIfMultiLine,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(crate::t!("cli-task-manager-prompt-placeholder"), ctx);
            editor
        });
        ctx.subscribe_to_view(&prompt, |view, _, event, ctx| {
            view.handle_managed_editor_event(event, ctx)
        });
        ctx.subscribe_to_view(&directory, |view, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_) | EditorEvent::BufferReplaced) {
                view.input_generation = Uuid::new_v4();
                view.managed_input.preparing = None;
                view.managed_input.skill_index_token = None;
                view.managed_input.skill_index_directory = None;
                view.prompt.update(ctx, |editor, ctx| {
                    editor.set_external_image_input_generation(view.input_generation, ctx)
                });
                view.refresh_managed_input(ctx);
            } else if matches!(event, EditorEvent::Blurred) {
                view.refresh_project_skill_index(ctx);
            }
        });
        for editor in [&directory, &prompt] {
            ctx.subscribe_to_view(editor, |view, _, event, ctx| match event {
                EditorEvent::Escape => ctx.emit(LocalCLITaskManagerEvent::Close),
                EditorEvent::Edited(_) => {
                    view.refresh_buttons(ctx);
                    ctx.notify();
                }
                _ => {}
            });
        }
        let mut buttons = HashMap::new();
        for (key, label, action) in [
            (
                "attach-files",
                crate::t!("cli-task-manager-attach-files"),
                TaskManagerAction::AttachFiles,
            ),
            (
                "import-review",
                crate::t!("cli-task-manager-import-review"),
                TaskManagerAction::ImportReview,
            ),
            (
                "allow-spawn",
                crate::t!("cli-task-manager-tools-spawn"),
                TaskManagerAction::ToggleSpawn,
            ),
            (
                "allow-messages",
                crate::t!("cli-task-manager-tools-messages"),
                TaskManagerAction::ToggleMessages,
            ),
            (
                "copy-result",
                crate::t!("cli-task-manager-copy-result"),
                TaskManagerAction::CopyResult,
            ),
            (
                "new",
                crate::t!("cli-task-manager-new"),
                TaskManagerAction::New,
            ),
            (
                "start",
                crate::t!("cli-task-manager-start"),
                TaskManagerAction::Start,
            ),
            (
                "send",
                crate::t!("cli-task-manager-send"),
                TaskManagerAction::Send,
            ),
            (
                "resume",
                crate::t!("cli-task-manager-resume"),
                TaskManagerAction::Resume,
            ),
            (
                "refresh",
                crate::t!("common-refresh"),
                TaskManagerAction::Refresh,
            ),
            ("close", crate::t!("common-close"), TaskManagerAction::Close),
            (
                "discard-draft",
                crate::t!("cli-task-manager-discard-draft"),
                TaskManagerAction::DiscardDraft,
            ),
        ] {
            buttons.insert(
                key,
                ctx.add_typed_action_view(move |_| {
                    let button = if key == "start" || key == "send" {
                        ActionButton::new(label, PrimaryTheme)
                    } else {
                        ActionButton::new(label, SecondaryTheme)
                    };
                    button.on_click(move |ctx| ctx.dispatch_typed_action(action.clone()))
                }),
            );
        }
        for (key, label) in [
            ("cancel", crate::t!("cli-task-manager-cancel")),
            ("disconnect", crate::t!("cli-task-manager-disconnect")),
        ] {
            buttons.insert(
                key,
                ctx.add_typed_action_view(move |_| ActionButton::new(label, DangerPrimaryTheme)),
            );
        }
        let harness_buttons = [Harness::Codex, Harness::Claude, Harness::Grok]
            .into_iter()
            .map(|harness| {
                let button = ctx.add_typed_action_view(move |_| {
                    ActionButton::new(harness_name(harness), SecondaryTheme).on_click(move |ctx| {
                        ctx.dispatch_typed_action(TaskManagerAction::SelectHarness(harness))
                    })
                });
                (harness, button)
            })
            .collect();
        let permission_buttons = [
            PermissionPolicy::Inherit,
            PermissionPolicy::ReadOnly,
            PermissionPolicy::WorkspaceWrite,
        ]
        .into_iter()
        .map(|permission| {
            let button = ctx.add_typed_action_view(move |_| {
                ActionButton::new(permission_name(permission), SecondaryTheme).on_click(
                    move |ctx| {
                        ctx.dispatch_typed_action(TaskManagerAction::SelectPermission(permission))
                    },
                )
            });
            (permission, button)
        })
        .collect();
        ctx.subscribe_to_model(
            &LocalCLITaskCoordinator::handle(ctx),
            |view, _, event, ctx| {
                view.handle_coordinator_event(event, ctx);
            },
        );
        ctx.subscribe_to_model(&CLIAgentInstallModel::handle(ctx), |view, _, _, ctx| {
            view.refresh_buttons(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(
            &crate::ai::skills::SkillManager::handle(ctx),
            |view, _, _, ctx| {
                view.refresh_managed_input(ctx);
                ctx.notify();
            },
        );
        let mut view = Self {
            harness: Harness::Codex,
            permission: PermissionPolicy::Inherit,
            local_tools: Default::default(),
            selected_task: None,
            input_generation: Uuid::new_v4(),
            managed_input: ManagedInputState::new(ctx),
            directory,
            prompt,
            drafts: HashMap::new(),
            pending_inputs: HashMap::new(),
            pending_controls: HashMap::new(),
            error: None,
            buttons,
            harness_buttons,
            permission_buttons,
            task_buttons: BTreeMap::new(),
            approval_buttons: Vec::new(),
            saved_messages: Vec::new(),
            message_buttons: Vec::new(),
            message_scope: None,
            message_load_token: Uuid::new_v4(),
            message_loading: false,
            message_reload_pending: false,
            observed_committed_task: None,
            result_history: ResultHistory::new(ctx),
            body_scroll: Default::default(),
            prompt_scroll: Default::default(),
        };
        view.prompt.update(ctx, |editor, ctx| {
            editor.set_external_image_input_generation(view.input_generation, ctx)
        });
        view.refresh_managed_input(ctx);
        view.refresh_buttons(ctx);
        view
    }

    pub(crate) fn on_open(&mut self, cwd: Option<PathBuf>, ctx: &mut ViewContext<Self>) {
        if self.selected_task.is_none()
            && self.directory.as_ref(ctx).buffer_text(ctx).is_empty()
            && let Some(cwd) = cwd.filter(|cwd| cwd.is_absolute())
        {
            self.directory.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&cwd.to_string_lossy(), ctx)
            });
        }
        LocalCLITaskCoordinator::handle(ctx).update(ctx, |model, ctx| model.refresh_records(ctx));
        CLIAgentInstallModel::handle(ctx).update(ctx, |model, ctx| model.refresh(ctx));
        self.refresh_buttons(ctx);
        self.refresh_messages(ctx);
        self.refresh_result_history(ctx);
        ctx.focus(&self.prompt);
    }

    fn tasks(&self, ctx: &AppContext) -> Vec<LocalCliTask> {
        let coordinator = LocalCLITaskCoordinator::as_ref(ctx);
        merge_tasks(
            coordinator.restored_tasks(),
            coordinator
                .snapshots()
                .map(|snapshot| snapshot.task.clone()),
        )
    }

    fn selected_snapshot(&self, ctx: &AppContext) -> Option<ManagedTaskSnapshot> {
        LocalCLITaskCoordinator::as_ref(ctx)
            .snapshot(self.selected_task.as_deref()?)
            .cloned()
    }

    fn selected_record(&self, ctx: &AppContext) -> Option<LocalCliTask> {
        let selected = self.selected_task.as_deref()?;
        self.tasks(ctx)
            .into_iter()
            .find(|task| task.task_id == selected)
    }

    /// 外部上下文只能进入发起请求时的草稿，不清除评审或选区源内容。
    pub(crate) fn append_context(
        &mut self,
        draft_key: &str,
        generation: Uuid,
        text: String,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if self.draft_key() != draft_key || self.input_generation != generation {
            return Err(crate::t!("cli-agent-input-target-changed"));
        }
        let previous = self.prompt.as_ref(ctx).buffer_text(ctx);
        let combined = if previous.is_empty() {
            text
        } else {
            format!("{previous}\n\n{text}")
        };
        self.prompt
            .update(ctx, |editor, ctx| editor.set_buffer_text(&combined, ctx));
        self.refresh_buttons(ctx);
        ctx.notify();
        Ok(())
    }

    pub(crate) fn show_input_error(&mut self, message: String, ctx: &mut ViewContext<Self>) {
        self.error = Some(message);
        ctx.notify();
    }

    fn draft_key(&self) -> String {
        self.selected_task.clone().unwrap_or_default()
    }

    fn select_task(&mut self, task_id: Option<String>, ctx: &mut ViewContext<Self>) {
        let previous_key = self.draft_key();
        self.managed_input.saved_text_revisions.insert(
            previous_key.clone(),
            self.prompt.as_ref(ctx).buffer_revision(ctx),
        );
        self.drafts
            .insert(self.draft_key(), self.prompt.as_ref(ctx).buffer_text(ctx));
        self.selected_task = task_id;
        self.input_generation = Uuid::new_v4();
        if self.selected_task.is_none() {
            self.local_tools = Default::default();
        }
        self.error = None;
        if let Some(task) = self.selected_record(ctx) {
            self.harness =
                Harness::parse_orchestration_harness(&task.harness).unwrap_or(Harness::Unknown);
            let saved = serde_json::from_str::<SavedLaunchOptions>(&task.config_json).ok();
            self.permission = saved
                .as_ref()
                .map(|options| options.permission_policy)
                .unwrap_or(PermissionPolicy::Inherit);
            self.local_tools = saved
                .and_then(|options| options.local_tools)
                .unwrap_or_default();
            self.directory.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&task.working_directory, ctx)
            });
        }
        let draft = self
            .drafts
            .get(&self.draft_key())
            .cloned()
            .unwrap_or_default();
        self.prompt
            .update(ctx, |editor, ctx| editor.set_buffer_text(&draft, ctx));
        self.restore_attachment_draft(previous_key, ctx);
        self.refresh_buttons(ctx);
        self.refresh_messages(ctx);
        self.refresh_result_history(ctx);
        ctx.notify();
    }

    fn refresh_messages(&mut self, ctx: &mut ViewContext<Self>) {
        let scope = self
            .selected_record(ctx)
            .map(|task| (task.task_id, task.generation));
        if self.message_loading && self.message_scope == scope {
            self.message_reload_pending = true;
            return;
        }
        self.message_loading = false;
        self.message_reload_pending = false;
        if self.message_scope != scope {
            self.saved_messages.clear();
            self.message_buttons.clear();
        }
        self.message_scope = scope.clone();
        self.message_load_token = Uuid::new_v4();
        let token = self.message_load_token;
        let Some((task_id, generation)) = scope else {
            return;
        };
        let Some(sender) = LocalCLITaskCoordinator::as_ref(ctx).sender() else {
            return;
        };
        let requested_task = task_id.clone();
        self.message_loading = true;
        ctx.spawn(
            async move {
                load_task_messages(&sender, requested_task)?
                    .await
                    .map_err(|_| crate::t!("cli-task-manager-message-read-failed"))?
            },
            move |view, result, ctx| {
                let selected = view
                    .selected_record(ctx)
                    .map(|task| (task.task_id, task.generation));
                if !message_read_is_current(
                    token,
                    view.message_load_token,
                    (&task_id, generation),
                    selected
                        .as_ref()
                        .map(|(id, generation)| (id.as_str(), *generation)),
                ) {
                    return;
                }
                view.message_loading = false;
                let reload_pending = std::mem::take(&mut view.message_reload_pending);
                match result {
                    Ok(messages) => {
                        view.saved_messages = messages;
                        view.message_buttons = view
                            .saved_messages
                            .iter()
                            .map(|message| {
                                let copy_task = task_id.clone();
                                let restore_task = task_id.clone();
                                let copy_id = message.message_id.clone();
                                let restore_id = message.message_id.clone();
                                let can_restore = recoverable_parts(message).is_some();
                                MessageButtons {
                                    copy: ctx.add_typed_action_view(move |_| {
                                        ActionButton::new(
                                            crate::t!("cli-task-manager-copy-input"),
                                            SecondaryTheme,
                                        )
                                        .on_click(
                                            move |ctx| {
                                                ctx.dispatch_typed_action(
                                                    TaskManagerAction::CopyInput {
                                                        task_id: copy_task.clone(),
                                                        task_generation: generation,
                                                        message_id: copy_id.clone(),
                                                    },
                                                )
                                            },
                                        )
                                    }),
                                    restore: ctx.add_typed_action_view(move |ctx| {
                                        let mut button = ActionButton::new(
                                            crate::t!("cli-task-manager-restore-input"),
                                            SecondaryTheme,
                                        )
                                        .on_click(move |ctx| {
                                            ctx.dispatch_typed_action(
                                                TaskManagerAction::RestoreInput {
                                                    task_id: restore_task.clone(),
                                                    task_generation: generation,
                                                    message_id: restore_id.clone(),
                                                },
                                            )
                                        });
                                        button.set_disabled(!can_restore, ctx);
                                        button
                                    }),
                                }
                            })
                            .collect();
                    }
                    Err(error) => view.error = Some(error),
                }
                if reload_pending {
                    view.refresh_messages(ctx);
                }
                ctx.notify();
            },
        );
    }

    fn handle_saved_input(
        &mut self,
        task_id: &str,
        generation: i64,
        message_id: &str,
        restore: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if !self
            .selected_record(ctx)
            .is_some_and(|task| task.task_id == task_id && task.generation == generation)
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let message = self
            .saved_messages
            .iter()
            .find(|message| message.message_id == message_id)
            .ok_or_else(|| crate::t!("cli-task-manager-message-read-failed"))?;
        if !restore {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(message_body_text(message)));
            return Ok(());
        }
        let input = recoverable_parts(message)
            .ok_or_else(|| crate::t!("cli-task-manager-input-not-recoverable"))?;
        let id = Uuid::parse_str(&message.message_id)
            .map_err(|_| crate::t!("cli-task-manager-input-not-recoverable"))?;
        let recipient_generation = message.recipient_generation;
        self.restore_saved_composer(task_id, recipient_generation, id, input, ctx)
    }

    fn executable(&self, harness: Harness, ctx: &AppContext) -> Option<PathBuf> {
        let agent = match harness {
            Harness::Codex => CLIAgent::Codex,
            Harness::Claude => CLIAgent::Claude,
            Harness::Grok
            | Harness::Oz
            | Harness::Gemini
            | Harness::OpenCode
            | Harness::Unknown => return None,
        };
        let installation = CLIAgentInstallModel::as_ref(ctx).installation(agent)?;
        if !verified_version(harness, &installation.version) {
            return None;
        }
        installation
            .executable
            .clone()
            .filter(|path| path.is_absolute())
    }

    fn start_prepared(
        &mut self,
        resume: bool,
        input: Vec<InputContent>,
        composer: Option<ComposerSubmission>,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if !FeatureFlag::LocalCLIManagedTasks.is_enabled() {
            return Err(crate::t!("cli-agent-managed-version-unavailable"));
        }
        if !resume && self.selected_task.is_some() {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let executable = self
            .executable(self.harness, ctx)
            .ok_or_else(|| crate::t!("cli-agent-managed-version-unavailable"))?;
        let prompt = composer
            .as_ref()
            .map(|snapshot| snapshot.text.clone())
            .unwrap_or_default();
        if !resume && input.is_empty() {
            return Err(crate::t!("cli-task-manager-empty-prompt"));
        }
        let (task, target, expected_generation, saved) = if resume {
            if self
                .selected_snapshot(ctx)
                .is_some_and(|snapshot| snapshot.connected)
            {
                return Err(crate::t!("cli-agent-task-already-running"));
            }
            let previous = self
                .selected_record(ctx)
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            let (task, target) = resumed_task(&previous)?;
            let saved = serde_json::from_str::<SavedLaunchOptions>(&previous.config_json)
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            (task, target, Some(previous.generation), saved)
        } else {
            let cwd = PathBuf::from(self.directory.as_ref(ctx).buffer_text(ctx));
            if !cwd.is_absolute() || !cwd.is_dir() {
                return Err(crate::t!("cli-task-manager-invalid-directory"));
            }
            let saved = SavedLaunchOptions {
                permission_policy: self.permission,
                permission_ceiling: None,
                model: None,
                selected_skills: input
                    .iter()
                    .filter_map(|part| match part {
                        InputContent::Skill { name, path } => Some(SelectedLocalSkill {
                            name: name.clone(),
                            path: path.clone(),
                        }),
                        InputContent::Text(_) | InputContent::LocalImage(_) => None,
                    })
                    .collect(),
                local_tools: (self.local_tools.allow_spawn || self.local_tools.allow_message)
                    .then_some(self.local_tools),
            };
            let task = LocalCliTask {
                version: 1,
                task_id: Uuid::new_v4().to_string(),
                parent_task_id: None,
                parent_generation: None,
                harness: self.harness.config_name().into(),
                working_directory: cwd.to_string_lossy().to_string(),
                config_json: serde_json::to_string(&saved)
                    .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?,
                native_session_id: None,
                generation: 1,
                revision: 0,
                state: LocalCliTaskState::Queued,
                result: None,
                terminal_evidence: None,
            };
            (task, SessionTarget::New, None, saved)
        };
        let task_id = task.task_id.clone();
        let task_generation = task.generation;
        let options = SessionOptions {
            state_dir: super::current_state_dir(),
            executable,
            cwd: PathBuf::from(&task.working_directory),
            target,
            generation: Uuid::new_v4(),
            permission_policy: saved.permission_policy,
            permission_ceiling: saved.permission_ceiling,
            model: saved.model,
            local_tools: saved.local_tools,
            selected_skills: saved.selected_skills,
        };
        let initial_message_id = Uuid::new_v4();
        LocalCLITaskCoordinator::handle(ctx).update(ctx, |model, ctx| {
            if resume {
                model.start(task, options, expected_generation, ctx)
            } else {
                model.start_with_input(
                    task,
                    options,
                    expected_generation,
                    initial_message_id,
                    input,
                    ctx,
                )
            }
        })?;
        if !resume {
            self.drafts.remove("");
            self.managed_input.saved.remove("");
        }
        self.selected_task = Some(task_id.clone());
        if !resume {
            self.pending_inputs.insert(
                task_id,
                PendingInput {
                    message_id: initial_message_id,
                    task_generation,
                    text: prompt,
                    phase: InputPhase::WaitingReady,
                    composer,
                },
            );
        }
        self.error = None;
        self.refresh_buttons(ctx);
        Ok(())
    }

    fn send_prepared(
        &mut self,
        input: Vec<InputContent>,
        composer: ComposerSubmission,
        target: PreparedInputTarget,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let task_id = self
            .selected_task
            .clone()
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        let Some(snapshot) = self
            .selected_snapshot(ctx)
            .filter(|snapshot| snapshot.ready && snapshot.connected)
        else {
            return Err(crate::t!("cli-agent-status-disconnected"));
        };
        if self.pending_inputs.contains_key(&task_id) {
            return Err(crate::t!("cli-task-manager-input-pending"));
        }
        let text = composer.text.clone();
        if input.is_empty() {
            return Err(crate::t!("cli-task-manager-empty-prompt"));
        }
        let harness = Harness::parse_orchestration_harness(&snapshot.task.harness)
            .ok_or_else(|| crate::t!("cli-agent-managed-version-unavailable"))?;
        if target.task_id != task_id
            || target.task_generation != snapshot.task.generation
            || target.harness != harness
        {
            return Err(crate::t!("cli-agent-input-target-changed"));
        }
        let action = input_action_with_content(harness, target.turn_id.as_deref(), input)?;
        let task_generation = snapshot.task.generation;
        let message_id = Uuid::new_v4();
        let receiver = LocalCLITaskCoordinator::handle(ctx).update(ctx, |model, _| {
            model.request_for_generation(&task_id, task_generation, message_id, action)
        })?;
        self.pending_inputs.insert(
            task_id.clone(),
            PendingInput {
                message_id,
                task_generation,
                text,
                phase: InputPhase::WaitingAck,
                composer: Some(composer),
            },
        );
        ctx.spawn(async move { receiver.await }, move |view, result, ctx| {
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(_) => Some(crate::t!("cli-task-manager-input-uncertain")),
            };
            if let Some(error) = error {
                view.mark_uncertain(&task_id, message_id);
                if view.selected_record(ctx).is_some_and(|task| {
                    task.task_id == task_id && task.generation == task_generation
                }) {
                    view.error = Some(error);
                }
            }
            view.refresh_buttons(ctx);
            ctx.notify();
        });
        Ok(())
    }

    fn mark_uncertain(&mut self, task_id: &str, message_id: Uuid) {
        if let Some(pending) = self.pending_inputs.get_mut(task_id)
            && pending.message_id == message_id
        {
            pending.phase = InputPhase::Uncertain;
        }
    }

    fn send_control(
        &mut self,
        task_id: String,
        task_generation: i64,
        key: String,
        action: RuntimeAction,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let key = format!("{task_id}:{task_generation}:{key}");
        if self.pending_controls.contains_key(&key) {
            return Err(crate::t!("cli-task-manager-input-pending"));
        }
        let message_id = Uuid::new_v4();
        let receiver = LocalCLITaskCoordinator::handle(ctx).update(ctx, |model, _| {
            model.request_for_generation(&task_id, task_generation, message_id, action)
        })?;
        self.pending_controls.insert(key.clone(), message_id);
        ctx.spawn(async move { receiver.await }, move |view, result, ctx| {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if view.selected_record(ctx).is_some_and(|task| {
                        task.task_id == task_id && task.generation == task_generation
                    }) {
                        view.error = Some(error);
                    }
                }
                Err(_) => {
                    if view.selected_record(ctx).is_some_and(|task| {
                        task.task_id == task_id && task.generation == task_generation
                    }) {
                        view.error = Some(crate::t!("cli-task-manager-input-uncertain"));
                    }
                }
            }
            view.refresh_buttons(ctx);
            ctx.notify();
        });
        Ok(())
    }

    fn handle_coordinator_event(
        &mut self,
        event: &LocalCLITaskCoordinatorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let LocalCLITaskCoordinatorEvent::Runtime { task, event } = event {
            match &event.kind {
                RuntimeEventKind::CommandDispatched { message_id, .. } => {
                    self.pending_controls.retain(|_, id| id != message_id);
                }
                RuntimeEventKind::MessageAccepted { message_id, .. } => {
                    self.pending_controls.retain(|_, id| id != message_id);
                    if self
                        .pending_inputs
                        .get(&task.task_id)
                        .is_some_and(|pending| pending.message_id == *message_id)
                    {
                        let pending = self
                            .pending_inputs
                            .remove(&task.task_id)
                            .expect("matching pending input");
                        if let Some(composer) = &pending.composer {
                            self.acknowledge_composer(&task.task_id, composer, ctx);
                        }
                    }
                }
                RuntimeEventKind::RequestFailed {
                    message_id,
                    message,
                } => {
                    self.pending_controls.retain(|_, id| id != message_id);
                    if self
                        .pending_inputs
                        .get(&task.task_id)
                        .is_some_and(|pending| pending.message_id == *message_id)
                    {
                        self.pending_inputs.remove(&task.task_id);
                    }
                    if self.selected_task.as_ref() == Some(&task.task_id) {
                        self.error = Some(message.clone());
                    }
                }
                RuntimeEventKind::Disconnected { .. } => {
                    if let Some(pending) = self.pending_inputs.get_mut(&task.task_id) {
                        pending.phase = InputPhase::Uncertain;
                    }
                }
                RuntimeEventKind::ApprovalResolved { approval_id, .. }
                | RuntimeEventKind::ApprovalCancelled { approval_id } => {
                    self.pending_controls.remove(&format!(
                        "{}:{}:{approval_id}",
                        task.task_id, task.generation
                    ));
                }
                RuntimeEventKind::SessionReady { .. }
                | RuntimeEventKind::TurnStarted { .. }
                | RuntimeEventKind::TextDelta { .. }
                | RuntimeEventKind::Progress { .. }
                | RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::ApprovalRequested { .. }
                | RuntimeEventKind::TurnFinished { .. } => {}
            }
        }
        let coordinator = LocalCLITaskCoordinator::as_ref(ctx);
        for (task_id, pending) in &mut self.pending_inputs {
            if let Some(snapshot) = coordinator.snapshot(task_id) {
                if !snapshot.connected
                    || (pending.phase == InputPhase::WaitingReady
                        && snapshot.task.generation != pending.task_generation)
                {
                    pending.phase = InputPhase::Uncertain;
                } else if snapshot.ready && pending.phase == InputPhase::WaitingReady {
                    pending.phase = InputPhase::WaitingAck;
                }
            }
        }
        self.pending_controls.retain(|key, _| {
            coordinator.snapshots().any(|snapshot| {
                snapshot.connected
                    && key.starts_with(&format!(
                        "{}:{}:",
                        snapshot.task.task_id, snapshot.task.generation
                    ))
            })
        });
        let selected_record = self.selected_record(ctx);
        // TextDelta 只更新瞬态输出；仅已提交的任务记录改变才触发同代重新读取。
        let committed_changed = self.observed_committed_task != selected_record;
        self.observed_committed_task = selected_record.clone();
        let selected_scope = selected_record
            .as_ref()
            .map(|task| (task.task_id.clone(), task.generation));
        let message_changed = match event {
            LocalCLITaskCoordinatorEvent::MessagesChanged {
                sender_task_id,
                recipient_task_id,
            } => {
                self.selected_task.as_ref() == Some(sender_task_id)
                    || self.selected_task.as_ref() == Some(recipient_task_id)
            }
            LocalCLITaskCoordinatorEvent::ResultReady { message, .. }
            | LocalCLITaskCoordinatorEvent::ParentMessageReady { message, .. } => {
                self.selected_task.as_ref() == Some(&message.sender_task_id)
                    || self.selected_task.as_ref() == Some(&message.recipient_task_id)
            }
            LocalCLITaskCoordinatorEvent::Runtime { task, event } => {
                let related_runtime_changed = selected_record.as_ref().is_some_and(|selected| {
                    selected.task_id == task.task_id
                        || selected.parent_task_id.as_ref() == Some(&task.task_id)
                        || task.parent_task_id.as_ref() == Some(&selected.task_id)
                }) && matches!(
                    event.kind,
                    RuntimeEventKind::MessageAccepted { .. }
                        | RuntimeEventKind::CommandDispatched { .. }
                        | RuntimeEventKind::RequestFailed { .. }
                        | RuntimeEventKind::TurnFinished { .. }
                        | RuntimeEventKind::Disconnected { .. }
                        | RuntimeEventKind::LocalToolCancelled { .. }
                );
                let selected_runtime_changed = self.selected_task.as_ref() == Some(&task.task_id)
                    && matches!(
                        event.kind,
                        RuntimeEventKind::MessageAccepted { .. }
                            | RuntimeEventKind::CommandDispatched { .. }
                            | RuntimeEventKind::RequestFailed { .. }
                            | RuntimeEventKind::SessionReady { .. }
                            | RuntimeEventKind::TurnStarted { .. }
                            | RuntimeEventKind::Disconnected { .. }
                    );
                // 接收方的回执也会改变发送方的消息行；控制交付仅触发读库，不视为 ACK。
                let recorded_message_changed = matches!(&event.kind,
                    RuntimeEventKind::MessageAccepted { message_id, .. }
                    | RuntimeEventKind::CommandDispatched { message_id, .. }
                    | RuntimeEventKind::RequestFailed { message_id, .. }
                    if self.saved_messages.iter().any(|message| message.message_id == message_id.to_string()));
                related_runtime_changed || selected_runtime_changed || recorded_message_changed
            }
            LocalCLITaskCoordinatorEvent::Changed => false,
        };
        if self.message_scope != selected_scope || message_changed || committed_changed {
            self.refresh_messages(ctx);
        }
        let history_changed = match event {
            LocalCLITaskCoordinatorEvent::Runtime { task, event } => {
                self.selected_task.as_ref() == Some(&task.task_id)
                    && matches!(
                        event.kind,
                        RuntimeEventKind::TurnFinished { .. }
                            | RuntimeEventKind::TurnStarted { .. }
                            | RuntimeEventKind::Disconnected { .. }
                    )
            }
            LocalCLITaskCoordinatorEvent::ResultReady { task, .. } => {
                self.selected_task.as_ref() == Some(&task.task_id)
            }
            LocalCLITaskCoordinatorEvent::Changed
            | LocalCLITaskCoordinatorEvent::MessagesChanged { .. }
            | LocalCLITaskCoordinatorEvent::ParentMessageReady { .. } => false,
        };
        if self.result_history.scope != selected_scope || history_changed || committed_changed {
            self.refresh_result_history(ctx);
        }
        self.refresh_buttons(ctx);
        ctx.notify();
    }

    fn refresh_buttons(&mut self, ctx: &mut ViewContext<Self>) {
        let tasks = self.tasks(ctx);
        self.task_buttons
            .retain(|id, _| tasks.iter().any(|task| &task.task_id == id));
        for task in tasks {
            let id = task.task_id.clone();
            let active = self.selected_task.as_ref() == Some(&id);
            let label = crate::t!(
                "cli-task-manager-task-row",
                cli = harness_name(
                    Harness::parse_orchestration_harness(&task.harness).unwrap_or(Harness::Unknown)
                ),
                state = task_state_name(task.state),
                task = task.task_id.chars().take(8).collect::<String>()
            );
            let button = self.task_buttons.entry(task.task_id).or_insert_with(|| {
                ctx.add_typed_action_view(move |_| {
                    ActionButton::new("", NakedTheme)
                        .with_full_width(true)
                        .on_click(move |ctx| {
                            ctx.dispatch_typed_action(TaskManagerAction::SelectTask(id.clone()))
                        })
                })
            });
            button.update(ctx, |button, ctx| {
                button.set_label(label, ctx);
                button.set_active(active, ctx);
            });
        }
        self.directory.update(ctx, |editor, ctx| {
            editor.set_interaction_state(
                if self.selected_task.is_some() {
                    InteractionState::Selectable
                } else {
                    InteractionState::Editable
                },
                ctx,
            )
        });
        let snapshot = self.selected_snapshot(ctx);
        let connected = snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.connected && snapshot.ready);
        let running = snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.active_turn_id.is_some());
        let pending = self
            .selected_task
            .as_ref()
            .is_some_and(|id| self.pending_inputs.contains_key(id));
        let text_empty = self.prompt.as_ref(ctx).buffer_text(ctx).trim().is_empty()
            && self.managed_input.attachments.images.is_empty()
            && self.managed_input.attachments.skills.is_empty();
        let preparing =
            self.managed_input.preparing.is_some() || self.managed_input.processing_images;
        let enabled = FeatureFlag::LocalCLIManagedTasks.is_enabled();
        for (key, disabled) in [
            (
                "start",
                !enabled
                    || self.selected_task.is_some()
                    || text_empty
                    || preparing
                    || self.executable(self.harness, ctx).is_none(),
            ),
            (
                "send",
                !enabled || !connected || pending || text_empty || preparing,
            ),
            (
                "resume",
                !enabled
                    || snapshot.as_ref().is_some_and(|snapshot| snapshot.connected)
                    || self.selected_record(ctx).is_none_or(|task| {
                        task.native_session_id.is_none()
                            || task.state.is_active()
                            || task.state == LocalCliTaskState::Unknown
                    }),
            ),
            ("cancel", !connected || !running),
            (
                "disconnect",
                !snapshot.as_ref().is_some_and(|snapshot| snapshot.connected),
            ),
        ] {
            self.buttons[key].update(ctx, |button, ctx| button.set_disabled(disabled, ctx));
        }
        if let Some(snapshot) = &snapshot {
            for (key, action) in [
                (
                    "cancel",
                    snapshot
                        .active_turn_id
                        .as_ref()
                        .map(|turn_id| RuntimeAction::Interrupt {
                            turn_id: turn_id.clone(),
                        }),
                ),
                ("disconnect", Some(RuntimeAction::Shutdown)),
            ] {
                if let Some(action) = action {
                    let control = TaskManagerAction::Control {
                        task_id: snapshot.task.task_id.clone(),
                        task_generation: snapshot.task.generation,
                        key: key.to_string(),
                        action,
                    };
                    self.buttons[key].update(ctx, |button, ctx| {
                        button.set_on_click(
                            move |ctx| ctx.dispatch_typed_action(control.clone()),
                            ctx,
                        )
                    });
                }
            }
        }
        for (harness, button) in &self.harness_buttons {
            button.update(ctx, |button, ctx| {
                button.set_active(*harness == self.harness, ctx);
                button.set_disabled(self.selected_task.is_some(), ctx);
            });
        }
        for (key, active) in [
            ("allow-spawn", self.local_tools.allow_spawn),
            ("allow-messages", self.local_tools.allow_message),
        ] {
            self.buttons[key].update(ctx, |button, ctx| {
                button.set_active(active, ctx);
                button.set_disabled(
                    self.selected_task.is_some() || self.harness == Harness::Grok,
                    ctx,
                );
            });
        }
        for (permission, button) in &self.permission_buttons {
            button.update(ctx, |button, ctx| {
                button.set_active(*permission == self.permission, ctx);
                button.set_disabled(
                    self.selected_task.is_some()
                        || (self.harness != Harness::Codex
                            && *permission != PermissionPolicy::Inherit),
                    ctx,
                );
            });
        }
        let approvals = snapshot
            .as_ref()
            .map(|snapshot| snapshot.approvals.as_slice())
            .unwrap_or_default();
        if self.approval_buttons.iter().any(|buttons| {
            snapshot.as_ref().is_none_or(|snapshot| {
                buttons.task_id != snapshot.task.task_id
                    || buttons.task_generation != snapshot.task.generation
            })
        }) || self
            .approval_buttons
            .iter()
            .map(|buttons| &buttons.approval_id)
            .ne(approvals.iter().map(|approval| &approval.approval_id))
        {
            self.approval_buttons = approvals
                .iter()
                .map(|approval| {
                    let snapshot = snapshot.as_ref().expect("approvals have a task snapshot");
                    let task_id = snapshot.task.task_id.clone();
                    let task_generation = snapshot.task.generation;
                    let allow_task = task_id.clone();
                    let deny_task = task_id.clone();
                    let id = approval.approval_id.clone();
                    let allow_id = id.clone();
                    let deny_id = id.clone();
                    ApprovalButtons {
                        task_id,
                        task_generation,
                        approval_id: id,
                        allow: ctx.add_typed_action_view(move |_| {
                            ActionButton::new(
                                crate::t!("cli-task-manager-allow-once"),
                                PrimaryTheme,
                            )
                            .on_click(move |ctx| {
                                ctx.dispatch_typed_action(TaskManagerAction::Approve {
                                    task_id: allow_task.clone(),
                                    task_generation,
                                    approval_id: allow_id.clone(),
                                    decision: ApprovalDecision::AllowOnce,
                                })
                            })
                        }),
                        deny: ctx.add_typed_action_view(move |_| {
                            ActionButton::new(
                                crate::t!("cli-task-manager-deny-once"),
                                SecondaryTheme,
                            )
                            .on_click(move |ctx| {
                                ctx.dispatch_typed_action(TaskManagerAction::Approve {
                                    task_id: deny_task.clone(),
                                    task_generation,
                                    approval_id: deny_id.clone(),
                                    decision: ApprovalDecision::DenyOnce,
                                })
                            })
                        }),
                    }
                })
                .collect();
        }
        for buttons in &self.approval_buttons {
            let disabled = self.pending_controls.contains_key(&format!(
                "{}:{}:{}",
                buttons.task_id, buttons.task_generation, buttons.approval_id
            ));
            buttons
                .allow
                .update(ctx, |button, ctx| button.set_disabled(disabled, ctx));
            buttons
                .deny
                .update(ctx, |button, ctx| button.set_disabled(disabled, ctx));
        }
    }

    fn text(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .span(text)
                .with_soft_wrap()
                .build()
                .finish(),
        )
        .with_margin_bottom(8.)
        .finish()
    }

    fn row(&self, keys: &[&str]) -> Box<dyn Element> {
        let mut row = Flex::row();
        for key in keys {
            row.add_child(
                Container::new(ChildView::new(&self.buttons[key]).finish())
                    .with_margin_right(8.)
                    .finish(),
            );
        }
        Container::new(row.finish()).with_margin_bottom(8.).finish()
    }

    fn render_installation(&self, ctx: &AppContext, appearance: &Appearance) -> Box<dyn Element> {
        let mut column = Flex::column();
        for (harness, agent) in [
            (Harness::Codex, CLIAgent::Codex),
            (Harness::Claude, CLIAgent::Claude),
            (Harness::Grok, CLIAgent::Grok),
        ] {
            let installation = CLIAgentInstallModel::as_ref(ctx).installation(agent);
            let version = match installation.map(|installation| &installation.version) {
                Some(CLIAgentVersionStatus::Detected(version)) => version.clone(),
                Some(CLIAgentVersionStatus::NotInstalled) => {
                    crate::t!("cli-task-manager-not-installed")
                }
                Some(CLIAgentVersionStatus::Unknown) => {
                    crate::t!("cli-task-manager-version-unknown")
                }
                Some(CLIAgentVersionStatus::NotProbed) | None => {
                    crate::t!("cli-task-manager-scanning")
                }
            };
            let path = installation
                .and_then(|installation| installation.executable.as_ref())
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
            column.add_child(self.text(
                crate::t!(
                    "cli-task-manager-installation",
                    cli = harness_name(harness),
                    version = version,
                    path = path
                ),
                appearance,
            ));
        }
        column.finish()
    }
}

impl Entity for LocalCLITaskManagerView {
    type Event = LocalCLITaskManagerEvent;
}

impl View for LocalCLITaskManagerView {
    fn ui_name() -> &'static str {
        "LocalCLITaskManagerView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let mut body = Flex::column();
        if !FeatureFlag::LocalCLIManagedTasks.is_enabled() {
            body.add_child(self.text(
                crate::t!("cli-agent-managed-version-unavailable"),
                appearance,
            ));
            body.add_child(self.row(&["close"]));
            return body.finish();
        }
        body.add_child(self.text(crate::t!("cli-task-manager-local-only"), appearance));
        body.add_child(self.row(&["new", "refresh", "close"]));
        body.add_child(self.render_installation(ctx, appearance));
        body.add_child(self.text(crate::t!("cli-task-manager-grok-unavailable"), appearance));
        body.add_child(self.text(
            crate::t!("cli-task-manager-claude-verification"),
            appearance,
        ));
        if self.task_buttons.is_empty() {
            body.add_child(self.text(crate::t!("cli-task-manager-empty"), appearance));
        }
        for button in self.task_buttons.values() {
            body.add_child(ChildView::new(button).finish());
        }
        let mut harnesses = Flex::row();
        for (_, button) in &self.harness_buttons {
            harnesses.add_child(
                Container::new(ChildView::new(button).finish())
                    .with_margin_right(8.)
                    .finish(),
            );
        }
        body.add_child(
            Container::new(harnesses.finish())
                .with_vertical_padding(8.)
                .finish(),
        );
        body.add_child(self.text(crate::t!("cli-task-manager-directory"), appearance));
        body.add_child(
            Container::new(ChildView::new(&self.directory).finish())
                .with_uniform_padding(8.)
                .with_margin_bottom(8.)
                .finish(),
        );
        body.add_child(self.text(crate::t!("cli-task-manager-permission"), appearance));
        let mut permissions = Flex::row();
        for (_, button) in &self.permission_buttons {
            permissions.add_child(
                Container::new(ChildView::new(button).finish())
                    .with_margin_right(8.)
                    .finish(),
            );
        }
        body.add_child(permissions.finish());
        body.add_child(self.text(
            crate::t!("cli-task-manager-permission-inherit-help"),
            appearance,
        ));
        body.add_child(self.text(crate::t!("cli-task-manager-tools-title"), appearance));
        body.add_child(self.row(&["allow-spawn", "allow-messages"]));
        body.add_child(self.text(crate::t!("cli-task-manager-tools-help"), appearance));
        if let Some(task) = self.selected_record(ctx) {
            if task.state == LocalCliTaskState::Unconfirmed {
                body.add_child(
                    self.text(crate::t!("cli-agent-task-outcome-unconfirmed"), appearance),
                );
            }
            if let Ok(config) = serde_json::from_str::<serde_json::Value>(&task.config_json)
                && let Some(permissions) = config
                    .get("effective_permissions")
                    .filter(|value| !value.is_null())
            {
                body.add_child(self.text(
                    crate::t!("cli-task-manager-effective-permissions"),
                    appearance,
                ));
                body.add_child(
                    self.text(
                        serde_json::to_string_pretty(permissions)
                            .unwrap_or_default()
                            .chars()
                            .take(8192)
                            .collect(),
                        appearance,
                    ),
                );
            }
            body.add_child(self.text(
                crate::t!(
                        "cli-task-manager-task-details",
                        task = task.task_id,
                        state = task_state_name(task.state),
                        session = task
                            .native_session_id
                            .unwrap_or_else(|| crate::t!("cli-task-manager-session-pending"))
                    ),
                appearance,
            ));
        }
        if !self.saved_messages.is_empty() {
            body.add_child(self.text(crate::t!("cli-task-manager-saved-inputs"), appearance));
            body.add_child(self.text(crate::t!("cli-task-manager-saved-inputs-help"), appearance));
            for (message, buttons) in self.saved_messages.iter().zip(&self.message_buttons) {
                body.add_child(self.text(
                    crate::t!(
                        "cli-task-manager-message-details",
                        message = message.message_id.clone(),
                        generation = message.recipient_generation,
                        state = message_state_name(message.state)
                    ),
                    appearance,
                ));
                body.add_child(self.text(
                    crate::t!(
                        "cli-task-manager-message-route",
                        subject = message.subject.clone(),
                        sender = message.sender_task_id.clone(),
                        sender_generation = message.sender_generation,
                        recipient = message.recipient_task_id.clone(),
                        recipient_generation = message.recipient_generation
                    ),
                    appearance,
                ));
                body.add_child(self.text(message_receipt_name(message), appearance));
                let text = message_body_text(message);
                let preview = text.chars().take(4096).collect::<String>();
                body.add_child(self.text(preview.clone(), appearance));
                if preview.len() < text.len() {
                    body.add_child(
                        self.text(crate::t!("cli-task-manager-input-truncated"), appearance),
                    );
                }
                let mut actions = Flex::row().with_child(ChildView::new(&buttons.copy).finish());
                if message.subject == "user_input" {
                    actions.add_child(ChildView::new(&buttons.restore).finish());
                }
                body.add_child(actions.finish());
            }
        }
        let snapshot = self.selected_snapshot(ctx);
        if let Some(snapshot) = &snapshot {
            for approval in &snapshot.approvals {
                body.add_child(self.text(
                    crate::t!(
                        "cli-task-manager-approval",
                        method = approval.method.clone()
                    ),
                    appearance,
                ));
                body.add_child(self.text(
                    serde_json::to_string_pretty(&approval.details).unwrap_or_default(),
                    appearance,
                ));
                if let Some(buttons) = self
                    .approval_buttons
                    .iter()
                    .find(|buttons| buttons.approval_id == approval.approval_id)
                {
                    body.add_child(
                        Flex::row()
                            .with_child(ChildView::new(&buttons.allow).finish())
                            .with_child(ChildView::new(&buttons.deny).finish())
                            .finish(),
                    );
                }
            }
        }
        body.add_child(self.render_managed_attachments(ctx, appearance));
        body.add_child(self.text(crate::t!("cli-task-manager-prompt"), appearance));
        body.add_child(
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.prompt_scroll.clone(),
                    Container::new(ChildView::new(&self.prompt).finish())
                        .with_uniform_padding(8.)
                        .finish(),
                    ScrollbarWidth::Auto,
                    appearance.theme().nonactive_ui_detail().into(),
                    appearance.theme().active_ui_detail().into(),
                    warpui::elements::Fill::None,
                )
                .finish(),
            )
            .with_min_height(80.)
            .with_max_height(180.)
            .finish(),
        );
        if let Some(pending) = self
            .selected_task
            .as_ref()
            .and_then(|id| self.pending_inputs.get(id))
        {
            body.add_child(self.text(
                match pending.phase {
                    InputPhase::WaitingReady => crate::t!("cli-task-manager-input-waiting-ready"),
                    InputPhase::WaitingAck => crate::t!("cli-task-manager-input-pending"),
                    InputPhase::Uncertain => crate::t!("cli-task-manager-input-uncertain"),
                },
                appearance,
            ));
            if pending.phase == InputPhase::Uncertain {
                body.add_child(self.row(&["discard-draft"]));
            }
        }
        body.add_child(self.row(&["start", "send", "resume"]));
        body.add_child(self.row(&["cancel", "disconnect"]));
        let output = snapshot
            .as_ref()
            .map(|snapshot| snapshot.output.clone())
            .or_else(|| self.selected_record(ctx).and_then(|task| task.result))
            .unwrap_or_default();
        if !output.is_empty() {
            body.add_child(self.text(crate::t!("cli-task-manager-output"), appearance));
            let preview = output.chars().take(32768).collect::<String>();
            if preview.len() < output.len() {
                body.add_child(
                    self.text(crate::t!("cli-task-manager-output-truncated"), appearance),
                );
            }
            body.add_child(self.text(preview, appearance));
            body.add_child(self.row(&["copy-result"]));
        }
        body.add_child(self.render_result_history(appearance));
        let error = self
            .error
            .as_deref()
            .or_else(|| {
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.error.as_deref())
            })
            .or_else(|| LocalCLITaskCoordinator::as_ref(ctx).recovery_error());
        if let Some(error) = error {
            body.add_child(self.text(
                crate::t!("cli-task-manager-error", error = error),
                appearance,
            ));
        }
        ConstrainedBox::new(
            ClippedScrollable::vertical(
                self.body_scroll.clone(),
                body.finish(),
                ScrollbarWidth::Auto,
                appearance.theme().nonactive_ui_detail().into(),
                appearance.theme().active_ui_detail().into(),
                warpui::elements::Fill::None,
            )
            .finish(),
        )
        .with_max_height(650.)
        .finish()
    }
}

impl TypedActionView for LocalCLITaskManagerView {
    type Action = TaskManagerAction;
    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if self.selected_task.is_none()
            && matches!(
                action,
                TaskManagerAction::SelectPermission(_)
                    | TaskManagerAction::ToggleSpawn
                    | TaskManagerAction::ToggleMessages
            )
        {
            self.input_generation = Uuid::new_v4();
            self.managed_input.preparing = None;
            self.prompt.update(ctx, |editor, ctx| {
                editor.set_external_image_input_generation(self.input_generation, ctx)
            });
        }
        let result = match action {
            TaskManagerAction::Close => {
                ctx.emit(LocalCLITaskManagerEvent::Close);
                Ok(())
            }
            TaskManagerAction::New => {
                self.select_task(None, ctx);
                Ok(())
            }
            TaskManagerAction::SelectTask(id) => {
                self.select_task(Some(id.clone()), ctx);
                Ok(())
            }
            TaskManagerAction::SelectHarness(harness) => {
                if self.selected_task.is_none() {
                    self.harness = *harness;
                    self.permission = PermissionPolicy::Inherit;
                    self.input_generation = Uuid::new_v4();
                    self.managed_input.preparing = None;
                    self.prompt.update(ctx, |editor, ctx| {
                        editor.set_external_image_input_generation(self.input_generation, ctx)
                    });
                }
                Ok(())
            }
            TaskManagerAction::SelectPermission(permission) => {
                if self.selected_task.is_none()
                    && (self.harness == Harness::Codex || *permission == PermissionPolicy::Inherit)
                {
                    self.permission = *permission;
                }
                Ok(())
            }
            TaskManagerAction::ToggleSpawn => {
                if self.selected_task.is_none() && self.harness != Harness::Grok {
                    self.local_tools.allow_spawn = !self.local_tools.allow_spawn;
                }
                Ok(())
            }
            TaskManagerAction::ToggleMessages => {
                if self.selected_task.is_none() && self.harness != Harness::Grok {
                    self.local_tools.allow_message = !self.local_tools.allow_message;
                }
                Ok(())
            }
            TaskManagerAction::AttachFiles => {
                self.prompt
                    .update(ctx, |editor, ctx| editor.attach_files(ctx));
                Ok(())
            }
            TaskManagerAction::ImportReview => {
                self.import_current_review(ctx);
                Ok(())
            }
            TaskManagerAction::SelectSkill {
                reference,
                input_generation,
            } => self.select_composer_skill(reference, *input_generation, ctx),
            TaskManagerAction::RemoveImage {
                index,
                input_generation,
                revision,
            } => {
                if self.input_generation == *input_generation
                    && self.managed_input.attachments.revision == *revision
                    && *index < self.managed_input.attachments.images.len()
                {
                    self.managed_input.attachments.images.remove(*index);
                    self.managed_input.attachments.revision = Uuid::new_v4();
                }
                Ok(())
            }
            TaskManagerAction::RemoveSkill {
                index,
                input_generation,
                revision,
            } => {
                if self.input_generation == *input_generation
                    && self.managed_input.attachments.revision == *revision
                    && *index < self.managed_input.attachments.skills.len()
                {
                    self.managed_input.attachments.skills.remove(*index);
                    self.managed_input.attachments.revision = Uuid::new_v4();
                }
                Ok(())
            }
            TaskManagerAction::Start => self.start(false, ctx),
            TaskManagerAction::Resume => self.start(true, ctx),
            TaskManagerAction::Send => self.send_input(ctx),
            TaskManagerAction::Control {
                task_id,
                task_generation,
                key,
                action,
            } => self.send_control(
                task_id.clone(),
                *task_generation,
                key.clone(),
                action.clone(),
                ctx,
            ),
            TaskManagerAction::SelectResultGeneration {
                task_id,
                active_generation,
                result_generation,
                token,
            } => self.select_result_generation(
                task_id,
                *active_generation,
                *result_generation,
                *token,
                ctx,
            ),
            TaskManagerAction::CopyHistoricalResult {
                task_id,
                active_generation,
                result_generation,
                token,
            } => self.copy_historical_result(
                task_id,
                *active_generation,
                *result_generation,
                *token,
                ctx,
            ),
            TaskManagerAction::CopyResult => {
                let output = self
                    .selected_snapshot(ctx)
                    .map(|snapshot| snapshot.output)
                    .or_else(|| self.selected_record(ctx).and_then(|task| task.result))
                    .unwrap_or_default();
                ctx.clipboard().write(ClipboardContent::plain_text(output));
                Ok(())
            }
            TaskManagerAction::DiscardDraft => {
                if let Some(task_id) = self.selected_task.clone()
                    && self
                        .pending_inputs
                        .get(&task_id)
                        .is_some_and(|pending| pending.phase == InputPhase::Uncertain)
                {
                    let pending = self
                        .pending_inputs
                        .remove(&task_id)
                        .expect("uncertain draft exists");
                    if let Some(composer) = pending.composer {
                        self.acknowledge_composer(&task_id, &composer, ctx);
                    }
                    // 解除疑似重发锁只影响该条记录，不清除后来编辑的新草稿。
                    self.input_generation = Uuid::new_v4();
                    self.managed_input.preparing = None;
                    self.prompt.update(ctx, |editor, ctx| {
                        editor.set_external_image_input_generation(self.input_generation, ctx)
                    });
                }
                Ok(())
            }
            TaskManagerAction::Approve {
                task_id,
                task_generation,
                approval_id,
                decision,
            } => self.send_control(
                task_id.clone(),
                *task_generation,
                approval_id.clone(),
                RuntimeAction::RespondApproval {
                    approval_id: approval_id.clone(),
                    decision: *decision,
                },
                ctx,
            ),
            TaskManagerAction::CopyInput {
                task_id,
                task_generation,
                message_id,
            } => self.handle_saved_input(task_id, *task_generation, message_id, false, ctx),
            TaskManagerAction::RestoreInput {
                task_id,
                task_generation,
                message_id,
            } => self.handle_saved_input(task_id, *task_generation, message_id, true, ctx),
            TaskManagerAction::Refresh => {
                LocalCLITaskCoordinator::handle(ctx)
                    .update(ctx, |model, ctx| model.refresh_records(ctx));
                CLIAgentInstallModel::handle(ctx).update(ctx, |model, ctx| model.refresh(ctx));
                self.refresh_messages(ctx);
                self.refresh_result_history(ctx);
                self.refresh_project_skill_index(ctx);
                Ok(())
            }
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
        self.refresh_managed_input(ctx);
        self.refresh_buttons(ctx);
        ctx.notify();
    }
}

fn message_read_is_current(
    token: Uuid,
    current_token: Uuid,
    requested: (&str, i64),
    selected: Option<(&str, i64)>,
) -> bool {
    token == current_token && selected == Some(requested)
}

fn draft_can_be_restored(
    current: &str,
    restored: &str,
    pending: Option<Uuid>,
    message_id: Uuid,
) -> bool {
    (current.is_empty() || current == restored)
        && pending.is_none_or(|pending| pending == message_id)
}

fn message_body_text(message: &LocalCliMessage) -> String {
    if message.subject == "user_input" {
        input_text(message).unwrap_or_else(|| message.body.clone())
    } else {
        message.body.clone()
    }
}

fn message_receipt_name(message: &LocalCliMessage) -> String {
    if message.state != LocalCliMessageState::Acknowledged {
        return crate::t!("cli-task-manager-message-no-receipt");
    }
    match message.receipt_kind {
        Some(LocalCliReceiptKind::NativeProtocol) => {
            crate::t!("cli-task-manager-message-native-receipt")
        }
        Some(LocalCliReceiptKind::ApplicationHistory) => {
            crate::t!("cli-task-manager-message-history-receipt")
        }
        Some(LocalCliReceiptKind::Unknown) | None => {
            crate::t!("cli-task-manager-message-unverified-receipt")
        }
    }
}

fn input_text(message: &LocalCliMessage) -> Option<String> {
    let action: RuntimeAction = serde_json::from_str(&message.body).ok()?;
    let input = match action {
        RuntimeAction::Submit { input } | RuntimeAction::Steer { input, .. } => input,
        RuntimeAction::Interrupt { .. }
        | RuntimeAction::RespondApproval { .. }
        | RuntimeAction::RespondLocalTool { .. }
        | RuntimeAction::Shutdown => return None,
    };
    input
        .into_iter()
        .map(|part| match part {
            InputContent::Text(text) => Some(text),
            InputContent::LocalImage(_) | InputContent::Skill { .. } => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("\n\n"))
}

fn recoverable_parts(message: &LocalCliMessage) -> Option<Vec<InputContent>> {
    if message.version != 1
        || message.subject != "user_input"
        || !matches!(
            message.state,
            LocalCliMessageState::Queued | LocalCliMessageState::Sent
        )
    {
        return None;
    }
    Uuid::parse_str(&message.message_id).ok()?;
    match serde_json::from_str(&message.body).ok()? {
        RuntimeAction::Submit { input } | RuntimeAction::Steer { input, .. }
            if !input.is_empty() =>
        {
            Some(input)
        }
        RuntimeAction::Submit { .. }
        | RuntimeAction::Steer { .. }
        | RuntimeAction::Interrupt { .. }
        | RuntimeAction::RespondApproval { .. }
        | RuntimeAction::RespondLocalTool { .. }
        | RuntimeAction::Shutdown => None,
    }
}

#[cfg(test)]
fn recoverable_input(message: &LocalCliMessage) -> Option<String> {
    recoverable_parts(message)?;
    input_text(message).filter(|text| !text.is_empty())
}

fn message_state_name(state: LocalCliMessageState) -> String {
    match state {
        LocalCliMessageState::Queued => crate::t!("cli-task-manager-message-queued"),
        LocalCliMessageState::Sent => crate::t!("cli-task-manager-message-sent"),
        LocalCliMessageState::Acknowledged => crate::t!("cli-task-manager-message-acknowledged"),
        LocalCliMessageState::Failed => crate::t!("cli-task-manager-message-failed"),
        LocalCliMessageState::Cancelled => crate::t!("cli-task-manager-message-cancelled"),
        LocalCliMessageState::Unknown => crate::t!("cli-task-manager-message-unknown"),
    }
}

fn merge_tasks(
    restored: &[LocalCliTask],
    live: impl IntoIterator<Item = LocalCliTask>,
) -> Vec<LocalCliTask> {
    let mut tasks = restored
        .iter()
        .cloned()
        .map(|task| (task.task_id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    for task in live {
        tasks.insert(task.task_id.clone(), task);
    }
    tasks.into_values().collect()
}

fn resumed_task(previous: &LocalCliTask) -> Result<(LocalCliTask, SessionTarget), String> {
    // 没有已连接的协调器不代表旧 PTY 已结束；未确认的运行仍占用当前代。
    match previous.state {
        LocalCliTaskState::Unconfirmed => {
            return Err(crate::t!("cli-agent-task-outcome-unconfirmed"));
        }
        LocalCliTaskState::Queued
        | LocalCliTaskState::Running
        | LocalCliTaskState::WaitingForUser => {
            return Err(crate::t!("cli-agent-task-already-running"));
        }
        LocalCliTaskState::Unknown => {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        LocalCliTaskState::Completed
        | LocalCliTaskState::Failed
        | LocalCliTaskState::Cancelled
        | LocalCliTaskState::Disconnected => {}
    }
    let native_session_id = previous
        .native_session_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| crate::t!("cli-task-manager-resume-unavailable"))?;
    let mut task = previous.clone();
    task.generation = task
        .generation
        .checked_add(1)
        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
    task.revision = 0;
    task.state = LocalCliTaskState::Queued;
    task.result = None;
    task.terminal_evidence = None;
    Ok((task, SessionTarget::Resume { native_session_id }))
}

#[cfg(test)]
fn input_action(
    harness: Harness,
    active_turn: Option<&str>,
    text: &str,
) -> Result<RuntimeAction, String> {
    input_action_with_content(
        harness,
        active_turn,
        vec![InputContent::Text(text.to_string())],
    )
}

fn input_action_with_content(
    harness: Harness,
    active_turn: Option<&str>,
    input: Vec<InputContent>,
) -> Result<RuntimeAction, String> {
    match harness {
        Harness::Codex => Ok(match active_turn {
            Some(turn_id) => RuntimeAction::Steer {
                expected_turn_id: turn_id.to_string(),
                input,
            },
            None => RuntimeAction::Submit { input },
        }),
        Harness::Claude if active_turn.is_some() => {
            Err(crate::t!("cli-agent-claude-queue-unverified"))
        }
        Harness::Claude => Ok(RuntimeAction::Submit { input }),
        Harness::Grok | Harness::Oz | Harness::Gemini | Harness::OpenCode | Harness::Unknown => {
            Err(crate::t!("cli-agent-managed-version-unavailable"))
        }
    }
}

fn verified_version(harness: Harness, version: &CLIAgentVersionStatus) -> bool {
    match (harness, version) {
        (Harness::Codex, CLIAgentVersionStatus::Detected(version)) => version == "0.147.0",
        (Harness::Claude, CLIAgentVersionStatus::Detected(version)) => version == "2.1.273",
        _ => false,
    }
}

fn harness_name(harness: Harness) -> String {
    match harness {
        Harness::Codex => "Codex CLI".into(),
        Harness::Claude => "Claude Code".into(),
        Harness::Grok => "Grok Build".into(),
        Harness::Oz | Harness::Gemini | Harness::OpenCode | Harness::Unknown => {
            crate::t!("cli-task-manager-unknown-cli")
        }
    }
}

fn permission_name(permission: PermissionPolicy) -> String {
    match permission {
        PermissionPolicy::Inherit => crate::t!("cli-task-manager-permission-inherit"),
        PermissionPolicy::ReadOnly => crate::t!("cli-task-manager-permission-readonly"),
        PermissionPolicy::WorkspaceWrite => crate::t!("cli-task-manager-permission-workspace"),
    }
}

fn task_state_name(state: LocalCliTaskState) -> String {
    match state {
        LocalCliTaskState::Queued => crate::t!("cli-task-manager-state-queued"),
        LocalCliTaskState::Running => crate::t!("cli-task-manager-state-running"),
        LocalCliTaskState::WaitingForUser => crate::t!("cli-task-manager-state-waiting"),
        LocalCliTaskState::Unconfirmed => crate::t!("cli-task-manager-state-unconfirmed"),
        LocalCliTaskState::Completed => crate::t!("cli-task-manager-state-completed"),
        LocalCliTaskState::Failed => crate::t!("cli-task-manager-state-failed"),
        LocalCliTaskState::Cancelled => crate::t!("cli-task-manager-state-cancelled"),
        LocalCliTaskState::Disconnected => crate::t!("cli-task-manager-state-disconnected"),
        LocalCliTaskState::Unknown => crate::t!("cli-task-manager-state-unknown"),
    }
}

#[cfg(test)]
#[path = "task_manager_view_tests.rs"]
mod tests;
