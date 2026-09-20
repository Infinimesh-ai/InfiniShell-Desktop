//! 托管任务输入复用编辑器、附件 Chip 和技能发现，异步结果保留原草稿归属。

use std::collections::HashMap;
use std::path::PathBuf;

use crate::ui_components::blended_colors;
use ai::skills::{ParsedSkill, SkillReference, SkillScope};
use repo_metadata::RepoMetadataModel;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::ui::theme::color::internal_colors;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::{
    ChildView, CornerRadius, Flex, MouseStateHandle, ParentElement, Radius, Wrap,
};
use warpui::ui_components::chip::Chip;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity, ViewContext, ViewHandle};

use super::{LocalCLITaskManagerEvent, LocalCLITaskManagerView, TaskManagerAction};
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::PermissionPolicy;
use crate::ai::skills::SkillManager;
use crate::appearance::Appearance;
use crate::editor::{
    AttachedImage, EditorBufferRevision, Event as EditorEvent, ImageContextOptions,
};
use crate::terminal::cli_agent::CLIAgent;
use crate::terminal::input::skills::{is_user_invocable, selectable_cli_skill};
use crate::view_components::{Dropdown, DropdownItem};

#[derive(Clone, Default)]
pub(super) struct ComposerAttachments {
    pub images: Vec<ImageContext>,
    pub skills: Vec<SkillReference>,
    pub revision: Uuid,
}

#[derive(Clone)]
pub(super) struct ComposerSubmission {
    pub text: String,
    pub revision: EditorBufferRevision,
    pub attachments_revision: Uuid,
    pub input_generation: Uuid,
}

#[derive(Clone)]
pub(super) struct PreparedInputTarget {
    pub task_id: String,
    pub task_generation: i64,
    pub harness: Harness,
    pub turn_id: Option<String>,
}

pub(super) struct ManagedInputState {
    pub attachments: ComposerAttachments,
    pub saved: HashMap<String, ComposerAttachments>,
    pub saved_text_revisions: HashMap<String, EditorBufferRevision>,
    pub processing_images: bool,
    pub preparing: Option<Uuid>,
    pub skills: ViewHandle<Dropdown<TaskManagerAction>>,
    pub available_skills: bool,
    pub skill_index_token: Option<Uuid>,
    pub skill_index_directory: Option<(String, PathBuf)>,
}

impl ManagedInputState {
    pub fn new(ctx: &mut ViewContext<LocalCLITaskManagerView>) -> Self {
        let skills = ctx.add_typed_action_view(Dropdown::new);
        Self {
            attachments: Default::default(),
            saved: HashMap::new(),
            saved_text_revisions: HashMap::new(),
            processing_images: false,
            preparing: None,
            skills,
            available_skills: false,
            skill_index_token: None,
            skill_index_directory: None,
        }
    }
}

impl LocalCLITaskManagerView {
    pub(super) fn refresh_project_skill_index(&mut self, ctx: &mut ViewContext<Self>) {
        let directory = self.directory.as_ref(ctx).buffer_text(ctx);
        if directory.is_empty() || self.managed_input.skill_index_token.is_some() {
            return;
        }
        let generation = self.input_generation;
        let token = Uuid::new_v4();
        self.managed_input.skill_index_token = Some(token);
        let requested_directory = directory.clone();
        // 只在明确刷新或目录失焦时检查文件系统；逐字符编辑不创建项目索引。
        ctx.spawn(
            async move {
                let path = PathBuf::from(&requested_directory);
                if !path.is_absolute() || !path.is_dir() {
                    return Err(crate::t!("cli-task-manager-invalid-directory"));
                }
                StandardizedPath::from_local_canonicalized(&path)
                    .map_err(|_| crate::t!("cli-task-manager-invalid-directory"))
            },
            move |view, result, ctx| {
                if view.managed_input.skill_index_token != Some(token) {
                    return;
                }
                view.managed_input.skill_index_token = None;
                if view.input_generation != generation
                    || view.directory.as_ref(ctx).buffer_text(ctx) != directory
                {
                    return;
                }
                let result = result.and_then(|path| {
                    RepoMetadataModel::handle(ctx)
                        .update(ctx, |model, ctx| {
                            model.index_local_directory_path(&path, ctx)
                        })
                        .map(|()| {
                            // 索引中的技能使用真实路径；保留编辑器中的原始目录与启动语义。
                            view.managed_input.skill_index_directory =
                                Some((directory, path.to_local_path_lossy()));
                        })
                        .map_err(|_| crate::t!("cli-task-manager-skills-index-failed"))
                });
                if let Err(error) = result {
                    view.error = Some(error);
                }
                // 后续技能解析仍由现有 SkillWatcher 发布，列表订阅只读取当前目录。
                view.refresh_managed_input(ctx);
                ctx.notify();
            },
        );
    }

    pub(super) fn refresh_managed_input(&mut self, ctx: &mut ViewContext<Self>) {
        let options = ImageContextOptions::Enabled {
            unsupported_model: !matches!(self.harness, Harness::Codex | Harness::Claude),
            is_processing_attached_images: self.managed_input.processing_images,
            num_images_attached: self.managed_input.attachments.images.len(),
            num_images_in_conversation: 0,
        };
        self.prompt.update(ctx, |editor, ctx| {
            editor.update_image_context_options(options, ctx)
        });
        let agent = match self.harness {
            Harness::Codex => Some(CLIAgent::Codex),
            Harness::Claude => Some(CLIAgent::Claude),
            Harness::Grok if self.permission == PermissionPolicy::Inherit => Some(CLIAgent::Grok),
            Harness::Grok
            | Harness::Oz
            | Harness::OpenCode
            | Harness::Gemini
            | Harness::Unknown => None,
        };
        let directory_text = self.directory.as_ref(ctx).buffer_text(ctx);
        let directory = LocalOrRemotePath::Local(
            self.managed_input
                .skill_index_directory
                .as_ref()
                .filter(|(requested, _)| requested == &directory_text)
                .map(|(_, resolved)| resolved.clone())
                .unwrap_or_else(|| PathBuf::from(&directory_text)),
        );
        let manager = SkillManager::as_ref(ctx);
        let items = manager
            .get_skills_for_working_directory(Some(&directory), ctx)
            .into_iter()
            .filter(|skill| skill.scope != SkillScope::Bundled)
            .filter_map(|skill| {
                let agent = agent?;
                if agent == CLIAgent::Grok {
                    selectable_cli_skill(skill, Some(agent), manager)
                } else {
                    manager
                        .skill_exists_for_any_provider(
                            &skill,
                            agent.supported_skill_providers_for_scope(skill.scope),
                        )
                        .then_some(skill)
                }
            })
            .filter(|skill| {
                manager
                    .active_skill_by_reference(&skill.reference, ctx)
                    .is_some()
                    && self.claude_skill_was_registered(&skill.reference, ctx)
            })
            .map(|skill| {
                DropdownItem::new(
                    skill.name,
                    TaskManagerAction::SelectSkill {
                        reference: skill.reference,
                        input_generation: self.input_generation,
                    },
                )
            })
            .collect::<Vec<_>>();
        self.managed_input.available_skills = !items.is_empty();
        self.managed_input.skills.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
        });
    }

    pub(super) fn restore_attachment_draft(
        &mut self,
        previous_key: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.managed_input
            .saved
            .insert(previous_key, self.managed_input.attachments.clone());
        self.managed_input.attachments = self
            .managed_input
            .saved
            .get(&self.draft_key())
            .cloned()
            .unwrap_or_default();
        self.managed_input.preparing = None;
        self.prompt.update(ctx, |editor, ctx| {
            editor.set_external_image_input_generation(self.input_generation, ctx)
        });
        self.refresh_managed_input(ctx);
    }

    pub(super) fn handle_managed_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::ImagesProcessed { generation, images }
                if *generation == self.input_generation =>
            {
                self.managed_input
                    .attachments
                    .images
                    .extend(images.0.iter().cloned());
                self.managed_input.attachments.revision = Uuid::new_v4();
            }
            EditorEvent::FilePathsSelected {
                generation,
                file_paths,
            } if *generation == self.input_generation => {
                // 沿用终端文件上下文入口的完整路径文本，不额外发明文件协议。
                let text = file_paths.join("\n");
                if let Err(error) = self.append_context(&self.draft_key(), *generation, text, ctx) {
                    self.error = Some(error);
                }
            }
            EditorEvent::ProcessingAttachedImages(processing) => {
                self.managed_input.processing_images = *processing
            }
            EditorEvent::DroppedImageFiles(paths) => self.prompt.update(ctx, |editor, ctx| {
                editor.read_and_process_images_async(paths.len(), paths.clone(), ctx)
            }),
            EditorEvent::Paste => {
                let content = ctx.clipboard().read();
                let images = content
                    .images
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(index, image)| AttachedImage {
                        data: image.data,
                        mime_type: image.mime_type,
                        file_name: image
                            .filename
                            .unwrap_or_else(|| format!("clipboard-{}.png", index + 1)),
                    })
                    .collect::<Vec<_>>();
                if !images.is_empty() {
                    self.prompt.update(ctx, |editor, ctx| {
                        editor.process_and_attach_images_as_ai_context(images.len(), images, ctx)
                    });
                }
            }
            _ => return,
        }
        self.refresh_managed_input(ctx);
        self.refresh_buttons(ctx);
        ctx.notify();
    }

    fn claude_skill_was_registered(&self, reference: &SkillReference, ctx: &AppContext) -> bool {
        if self.permission == PermissionPolicy::ClaudeRestrictedFilesV1 {
            return false;
        }
        if self.harness != Harness::Claude || self.selected_task.is_none() {
            return true;
        }
        self.selected_record(ctx)
            .and_then(|task| {
                serde_json::from_str::<super::SavedLaunchOptions>(&task.config_json).ok()
            })
            .is_some_and(|options| {
                options.selected_skills.iter().any(|skill| {
                    *reference == SkillReference::Path(LocalOrRemotePath::Local(skill.path.clone()))
                })
            })
    }

    pub(super) fn parsed_composer_skills(
        &self,
        ctx: &AppContext,
    ) -> Result<Vec<ParsedSkill>, String> {
        if self.permission == PermissionPolicy::ClaudeRestrictedFilesV1
            && !self.managed_input.attachments.skills.is_empty()
        {
            return Err(crate::t!("cli-task-manager-permission-claude-files-skills"));
        }
        if self.harness == Harness::Grok && !self.managed_input.attachments.skills.is_empty() {
            if self.permission != PermissionPolicy::Inherit {
                return Err(crate::t!("cli-agent-grok-skill-policy-required"));
            }
            if self.managed_input.attachments.skills.len() > 1 {
                return Err(crate::t!("cli-agent-task-skill-one-per-turn"));
            }
        }
        let manager = SkillManager::as_ref(ctx);
        self.managed_input
            .attachments
            .skills
            .iter()
            .map(|reference| {
                manager
                    .active_skill_by_reference(reference, ctx)
                    .filter(|skill| {
                        self.harness != Harness::Grok
                            || is_user_invocable(&skill.user_invocable(), CLIAgent::Grok)
                    })
                    .cloned()
                    .ok_or_else(|| {
                        crate::t!(
                            "cli-agent-task-skill-unavailable",
                            skill = reference.display_label()
                        )
                    })
            })
            .collect()
    }

    pub(super) fn select_composer_skill(
        &mut self,
        reference: &SkillReference,
        generation: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if generation != self.input_generation {
            return Err(crate::t!("cli-agent-input-target-changed"));
        }
        if self.permission == PermissionPolicy::ClaudeRestrictedFilesV1 {
            return Err(crate::t!("cli-task-manager-permission-claude-files-skills"));
        }
        if self.harness == Harness::Grok {
            if self.permission != PermissionPolicy::Inherit {
                return Err(crate::t!("cli-agent-grok-skill-policy-required"));
            }
            if !self.managed_input.attachments.skills.is_empty()
                && !self.managed_input.attachments.skills.contains(reference)
            {
                return Err(crate::t!("cli-agent-task-skill-one-per-turn"));
            }
        }
        if !self.claude_skill_was_registered(reference, ctx) {
            return Err(crate::t!("cli-task-manager-skills-session-fixed"));
        }
        let skill = SkillManager::as_ref(ctx)
            .active_skill_by_reference(reference, ctx)
            .ok_or_else(|| {
                crate::t!(
                    "cli-agent-task-skill-unavailable",
                    skill = reference.display_label()
                )
            })?;
        if !skill.path.is_local()
            || skill.is_bundled()
            || (self.harness == Harness::Grok
                && !is_user_invocable(&skill.user_invocable(), CLIAgent::Grok))
        {
            return Err(crate::t!(
                "cli-agent-task-skill-unavailable",
                skill = skill.name.clone()
            ));
        }
        if !self.managed_input.attachments.skills.contains(reference) {
            self.managed_input
                .attachments
                .skills
                .push(reference.clone());
            self.managed_input.attachments.revision = Uuid::new_v4();
        }
        Ok(())
    }

    pub(super) fn import_current_review(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(LocalCLITaskManagerEvent::ImportReview {
            draft_key: self.draft_key(),
            input_generation: self.input_generation,
        });
    }

    fn render_composer_chip(
        &self,
        label: String,
        action: TaskManagerAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let close = appearance
            .ui_builder()
            .close_button(
                appearance.monospace_font_size(),
                MouseStateHandle::default(),
            )
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish();
        Chip::new(
            label,
            UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(appearance.monospace_font_size()),
                font_color: Some(blended_colors::text_main(
                    appearance.theme(),
                    appearance.theme().background(),
                )),
                border_width: Some(1.),
                border_color: Some(internal_colors::neutral_4(appearance.theme()).into()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
                ..Default::default()
            },
        )
        .with_close_button(close)
        .build()
        .finish()
    }

    pub(super) fn render_managed_attachments(
        &self,
        ctx: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut body = Flex::column();
        body.add_child(self.row(&["attach-files", "import-review"]));
        body.add_child(self.text(crate::t!("cli-task-manager-skills"), appearance));
        if self.permission == PermissionPolicy::ClaudeRestrictedFilesV1 {
            body.add_child(self.text(
                crate::t!("cli-task-manager-permission-claude-files-skills"),
                appearance,
            ));
        } else if self.managed_input.available_skills {
            body.add_child(ChildView::new(&self.managed_input.skills).finish());
        } else {
            body.add_child(self.text(crate::t!("cli-task-manager-no-skills"), appearance));
        }
        if !self.managed_input.attachments.images.is_empty()
            || !self.managed_input.attachments.skills.is_empty()
        {
            body.add_child(self.text(crate::t!("cli-task-manager-attachments"), appearance));
        }
        let mut chips = Wrap::row().with_run_spacing(4.);
        for (index, image) in self.managed_input.attachments.images.iter().enumerate() {
            chips = chips.with_child(self.render_composer_chip(
                image.file_name.clone(),
                TaskManagerAction::RemoveImage {
                    index,
                    input_generation: self.input_generation,
                    revision: self.managed_input.attachments.revision,
                },
                appearance,
            ));
        }
        let manager = SkillManager::as_ref(ctx);
        for (index, reference) in self.managed_input.attachments.skills.iter().enumerate() {
            let label = manager
                .active_skill_by_reference(reference, ctx)
                .map(|skill| skill.name.clone())
                .unwrap_or_else(|| reference.display_label());
            chips = chips.with_child(self.render_composer_chip(
                label,
                TaskManagerAction::RemoveSkill {
                    index,
                    input_generation: self.input_generation,
                    revision: self.managed_input.attachments.revision,
                },
                appearance,
            ));
        }
        body.add_child(chips.finish());
        if self.harness == Harness::Claude && self.selected_task.is_some() {
            body.add_child(self.text(
                crate::t!("cli-task-manager-skills-session-fixed"),
                appearance,
            ));
        }
        if self.harness == Harness::Claude {
            body.add_child(self.text(crate::t!("cli-agent-claude-png-only"), appearance));
        }
        if self.managed_input.preparing.is_some() || self.managed_input.processing_images {
            body.add_child(self.text(crate::t!("cli-task-manager-preparing-input"), appearance));
        }
        body.finish()
    }
}

impl LocalCLITaskManagerView {
    pub(super) fn start(
        &mut self,
        resume: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if resume {
            return self.start_prepared(true, Vec::new(), None, ctx);
        }
        self.prepare_composer(None, ctx)
    }

    pub(super) fn send_input(&mut self, ctx: &mut ViewContext<Self>) -> Result<(), String> {
        let snapshot = self
            .selected_snapshot(ctx)
            .filter(|snapshot| snapshot.ready && snapshot.connected)
            .ok_or_else(|| crate::t!("cli-agent-status-disconnected"))?;
        let harness = Harness::parse_orchestration_harness(&snapshot.task.harness)
            .ok_or_else(|| crate::t!("cli-agent-managed-version-unavailable"))?;
        self.prepare_composer(
            Some(PreparedInputTarget {
                task_id: snapshot.task.task_id,
                task_generation: snapshot.task.generation,
                harness,
                turn_id: snapshot.active_turn_id,
            }),
            ctx,
        )
    }

    fn prepare_composer(
        &mut self,
        target: Option<PreparedInputTarget>,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if self.managed_input.preparing.is_some() || self.managed_input.processing_images {
            return Err(crate::t!("cli-task-manager-preparing-input"));
        }
        if self
            .selected_task
            .as_ref()
            .is_some_and(|task_id| self.pending_inputs.contains_key(task_id))
        {
            return Err(crate::t!("cli-task-manager-input-pending"));
        }
        if target.is_none() && self.selected_task.is_some() {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let token = Uuid::new_v4();
        let draft_key = self.draft_key();
        let generation = self.input_generation;
        let harness = target
            .as_ref()
            .map(|target| target.harness)
            .unwrap_or(self.harness);
        let text = self.prompt.as_ref(ctx).buffer_text(ctx);
        let skills = self.parsed_composer_skills(ctx)?;
        let images = self.managed_input.attachments.images.clone();
        let snapshot = ComposerSubmission {
            text: text.clone(),
            revision: self.prompt.as_ref(ctx).buffer_revision(ctx),
            attachments_revision: self.managed_input.attachments.revision,
            input_generation: generation,
        };
        let directory = super::super::current_state_dir().join("local-cli-attachments");
        self.managed_input.preparing = Some(token);
        ctx.spawn(
            async move {
                super::super::managed_input::prepare_managed_input(
                    harness, text, &images, skills, &directory,
                )
            },
            move |view, result, ctx| {
                if view.managed_input.preparing != Some(token) {
                    return;
                }
                view.managed_input.preparing = None;
                if view.input_generation != generation || view.draft_key() != draft_key {
                    return;
                }
                let result = result.and_then(|input| match target {
                    Some(target) => view.send_prepared(input, snapshot, target, ctx),
                    None => view.start_prepared(false, input, Some(snapshot), ctx),
                });
                if let Err(error) = result {
                    view.error = Some(error);
                }
                view.refresh_buttons(ctx);
                ctx.notify();
            },
        );
        self.refresh_buttons(ctx);
        Ok(())
    }

    pub(super) fn restore_saved_composer(
        &mut self,
        task_id: &str,
        task_generation: i64,
        message_id: Uuid,
        input: Vec<super::InputContent>,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        let mut text_parts = Vec::new();
        let mut paths = Vec::new();
        let mut skills = Vec::new();
        for part in input {
            match part {
                super::InputContent::Text(text) => text_parts.push(text),
                super::InputContent::LocalImage(path) => paths.push(path),
                super::InputContent::Skill { path, .. } => {
                    skills.push(SkillReference::Path(LocalOrRemotePath::Local(path)));
                }
            }
        }
        let text = text_parts.join("\n\n");
        if !super::draft_can_be_restored(
            &self.prompt.as_ref(ctx).buffer_text(ctx),
            &text,
            self.pending_inputs
                .get(task_id)
                .map(|input| input.message_id),
            message_id,
        ) || !self.managed_input.attachments.images.is_empty()
            || !self.managed_input.attachments.skills.is_empty()
            || self.managed_input.preparing.is_some()
            || self.managed_input.processing_images
        {
            return Err(crate::t!("cli-task-manager-draft-in-use"));
        }
        let task_id = task_id.to_string();
        let selected_generation = self.selected_record(ctx).map(|task| task.generation);
        let token = Uuid::new_v4();
        let generation = self.input_generation;
        let revision = self.prompt.as_ref(ctx).buffer_revision(ctx);
        let attachments_revision = self.managed_input.attachments.revision;
        let directory = super::super::current_state_dir().join("local-cli-attachments");
        self.managed_input.preparing = Some(token);
        ctx.spawn(
            async move { super::super::managed_input::restore_managed_images(paths, &directory) },
            move |view, result, ctx| {
                if view.managed_input.preparing != Some(token) {
                    return;
                }
                view.managed_input.preparing = None;
                if view.selected_task.as_deref() != Some(task_id.as_str())
                    || view.selected_record(ctx).map(|task| task.generation) != selected_generation
                    || view.input_generation != generation
                    || view.prompt.as_ref(ctx).buffer_revision(ctx) != revision
                    || view.managed_input.attachments.revision != attachments_revision
                {
                    return;
                }
                match result {
                    Ok(images) => {
                        view.managed_input.attachments = ComposerAttachments {
                            images,
                            skills,
                            revision: Uuid::new_v4(),
                        };
                        view.prompt
                            .update(ctx, |editor, ctx| editor.set_buffer_text(&text, ctx));
                        view.drafts.insert(task_id.clone(), text.clone());
                        let composer = ComposerSubmission {
                            text: text.clone(),
                            revision: view.prompt.as_ref(ctx).buffer_revision(ctx),
                            attachments_revision: view.managed_input.attachments.revision,
                            input_generation: generation,
                        };
                        view.pending_inputs.insert(
                            task_id,
                            super::PendingInput {
                                message_id,
                                task_generation,
                                text,
                                phase: super::InputPhase::Uncertain,
                                composer: Some(composer),
                            },
                        );
                        ctx.focus(&view.prompt);
                    }
                    Err(error) => view.error = Some(error),
                }
                view.refresh_managed_input(ctx);
                view.refresh_buttons(ctx);
                ctx.notify();
            },
        );
        self.refresh_buttons(ctx);
        Ok(())
    }

    pub(super) fn composer_matches(
        &self,
        submitted: &ComposerSubmission,
        ctx: &AppContext,
    ) -> bool {
        self.input_generation == submitted.input_generation
            && self.prompt.as_ref(ctx).buffer_revision(ctx) == submitted.revision
    }

    pub(super) fn acknowledge_composer(
        &mut self,
        task_id: &str,
        submitted: &ComposerSubmission,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.managed_input.saved_text_revisions.get(task_id) == Some(&submitted.revision) {
            self.drafts.remove(task_id);
            self.managed_input.saved_text_revisions.remove(task_id);
        }
        if self.selected_task.as_deref() == Some(task_id) && self.composer_matches(submitted, ctx) {
            self.prompt.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx)
            });
        }
        if self
            .managed_input
            .saved
            .get(task_id)
            .is_some_and(|attachments| attachments.revision == submitted.attachments_revision)
        {
            self.managed_input.saved.remove(task_id);
        }
        if self.selected_task.as_deref() == Some(task_id)
            && self.input_generation == submitted.input_generation
            && self.managed_input.attachments.revision == submitted.attachments_revision
        {
            self.managed_input.attachments = Default::default();
        }
        self.refresh_managed_input(ctx);
    }
}
