//! owned Grok 富输入仅使用已绑定的原生 socket；不写 PTY Enter，也不自动重投未知消息。

use std::path::PathBuf;
use std::rc::Rc;

use async_channel::Receiver;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use warpui::{SingletonEntity, ViewContext};

use super::{CliInputSubmission, TerminalView, file_attachments};
use crate::persistence::PersistenceWriter;
use crate::persistence::local_cli_tasks::checkpoint_task_with_message;
use crate::persistence::local_cli_tasks::grok_terminal::{INPUT_SUBJECT, RICH_INPUT_SUBJECT};
use crate::persistence::model::{LocalCliMessage, LocalCliMessageState, LocalCliTaskState};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::grok_owned_launch::GrokOwnedLaunch;
use crate::terminal::cli_agent_sessions::grok_owned_prompt;
use crate::terminal::cli_agent_sessions::grok_owned_worker::{
    GrokOwnedInputLease, GrokOwnedWorker, GrokOwnedWorkerEvent,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentRichInputCloseReason, CLIAgentSessionsModel,
    GrokPermissionEvidence,
};

enum PreparationFailure {
    Attachment,
    Persistence,
    Native,
}

impl TerminalView {
    pub(super) fn prepare_owned_grok_image_composer(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !self.owned_grok_identity_matches()
            || self
                .grok_owned_input
                .as_ref()
                .is_none_or(|owned| owned.invalidated)
            || !CLIAgentSessionsModel::as_ref(ctx)
                .session(self.view_id)
                .is_some_and(|session| {
                    session.agent == CLIAgent::Grok && session.remote_host.is_none()
                })
        {
            return false;
        }
        self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::AutoShow, ctx);
        self.has_active_cli_agent_input_session(ctx)
    }

    pub(in crate::terminal::view) fn submit_owned_grok_input(
        &mut self,
        text: String,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.owned_grok_identity_matches() {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-unavailable"), ctx);
            return;
        }
        let images = self
            .ai_context_model
            .as_ref(ctx)
            .pending_images()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let snapshot = Rc::new(CliInputSubmission {
            agent: CLIAgent::Grok,
            query: text.clone(),
            editor_revision: self
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx),
            attachments_revision: self
                .ai_context_model
                .as_ref(ctx)
                .pending_attachments_revision(),
        });
        let draft_files = self
            .ai_context_model
            .as_ref(ctx)
            .pending_files()
            .into_iter()
            .map(|file| file.file_path.clone())
            .collect::<Vec<_>>();
        let image_fingerprints = images
            .iter()
            .map(|image| {
                let mut digest = Sha256::new();
                digest.update(image.mime_type.as_bytes());
                digest.update([0]);
                digest.update(image.data.as_bytes());
                format!("{:x}", digest.finalize())
            })
            .collect::<Vec<_>>();
        let draft_identity = (text.clone(), draft_files, image_fingerprints);
        let Some(owned) = self.grok_owned_input.as_mut() else {
            return;
        };
        if owned.invalidated || owned.sending {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-unavailable"), ctx);
            return;
        }
        if owned
            .unconfirmed_drafts
            .values()
            .any(|draft| draft == &draft_identity)
            || owned.last_attempt.as_ref()
                == Some(&(
                    snapshot.editor_revision.clone(),
                    snapshot.attachments_revision,
                ))
        {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-claimed"), ctx);
            return;
        }
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) else {
            return;
        };
        let GrokPermissionEvidence::Observed(observation) =
            &session.session_context.grok_permission_evidence
        else {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-unavailable"), ctx);
            return;
        };
        if observation.mode != "default"
            || observation.session_id != owned.owner.session_id.to_string()
        {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-unavailable"), ctx);
            return;
        }
        let observation = observation.clone();
        let Some(database) = PersistenceWriter::as_ref(ctx).sender() else {
            self.show_error_toast(crate::t!("cli-agent-task-save-failed"), ctx);
            return;
        };
        owned.owner.input_revision = Uuid::new_v4();
        let owner = owned.owner.clone();
        let lease = GrokOwnedInputLease::new(
            owner.binding_id,
            owner.input_revision,
            owner.permission_revision,
        )
        .expect("本次启动已生成非空 UUID");
        if !CLIAgentSessionsModel::handle(ctx).update(ctx, |model, _| {
            model.register_owned_grok_input(self.view_id, observation.clone(), lease.clone())
        }) {
            return;
        }
        let files = self
            .ai_context_model
            .as_ref(ctx)
            .pending_files()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        // 后续提交不能覆盖此前回执仍未知的草稿。
        owned
            .unconfirmed_drafts
            .insert(owner.input_revision, draft_identity);
        owned.last_attempt = Some((
            snapshot.editor_revision.clone(),
            snapshot.attachments_revision,
        ));
        owned.sending = true;
        owned.lease = Some(lease.clone());
        let previous_task = owned.task.clone();
        let mut task = owned.task.clone();
        task.revision += 1;
        task.state = LocalCliTaskState::Running;
        let mut config: Value =
            serde_json::from_str(&task.config_json).expect("应用构造的任务配置");
        let path = PathBuf::from(config["launch_manifest"].as_str().unwrap());
        let sha = config["launch_sha256"].as_str().unwrap().to_owned();
        config["grok_terminal"] = json!(owner);
        task.config_json = config.to_string();
        owned.task = task.clone();
        let input_revision = owner.input_revision;
        ctx.spawn(
            async move {
                let body = if files.is_empty() {
                    text
                } else {
                    file_attachments::prepare_file_attachments(text, files)
                        .await
                        .map_err(|message| (PreparationFailure::Attachment, message))?
                };
                let (subject, body) = if images.is_empty() {
                    (INPUT_SUBJECT, body)
                } else {
                    let body = blocking::unblock(move || grok_owned_prompt::prepare(body, &images))
                        .await
                        .map_err(|message| (PreparationFailure::Attachment, message))?;
                    (RICH_INPUT_SUBJECT, body)
                };
                let message = LocalCliMessage {
                    version: 1,
                    message_id: Uuid::new_v4().to_string(),
                    sender_task_id: task.task_id.clone(),
                    recipient_task_id: task.task_id.clone(),
                    sender_generation: task.generation,
                    recipient_generation: task.generation,
                    subject: subject.into(),
                    body,
                    state: LocalCliMessageState::Queued,
                    receipt_kind: None,
                };
                checkpoint_task_with_message(&database, task.clone(), Some(task.generation), message.clone())
                    .map_err(|_| {
                        (
                            PreparationFailure::Persistence,
                            crate::t!("cli-agent-task-save-failed"),
                        )
                    })?
                    .await
                    .map_err(|_| {
                        (
                            PreparationFailure::Persistence,
                            crate::t!("cli-agent-task-save-failed"),
                        )
                    })?
                    .map_err(|_| {
                        (
                            PreparationFailure::Persistence,
                            crate::t!("cli-agent-task-save-failed"),
                        )
                    })?;
                let binding = blocking::unblock(move || {
                    GrokOwnedLaunch::restore(&path, &sha).and_then(|mut launch| {
                        launch
                            .bind(owner.binding_id, owner.permission_revision, &observation)
                            .map_err(|_| std::io::Error::other("原生 Grok 绑定未通过"))
                    })
                })
                .await
                .map_err(|_| {
                    (
                        PreparationFailure::Native,
                        crate::t!("cli-agent-grok-owned-input-unavailable"),
                    )
                })?;
                GrokOwnedWorker::start(binding, task, message, lease, database).map_err(|_| {
                    (
                        PreparationFailure::Native,
                        crate::t!("cli-agent-grok-owned-input-unavailable"),
                    )
                })
            },
            move |view, result, ctx| {
                let current = view.grok_owned_input.as_ref().is_some_and(|owned| {
                    owned.owner.input_revision == input_revision && !owned.invalidated
                }) && view.owned_grok_identity_matches();
                if !current {
                    return;
                }
                match result {
                    Ok((worker, events)) => {
                        view.grok_owned_input.as_mut().unwrap().worker = Some(worker);
                        view.receive_owned_grok_event(input_revision, snapshot, events, ctx);
                    }
                    Err((stage, message)) => {
                        let owned = view.grok_owned_input.as_mut().unwrap();
                        owned.sending = false;
                        match stage {
                            // 附件检查尚未创建消息，修正附件后可以重新提交。
                            PreparationFailure::Attachment => {
                                owned.task = previous_task;
                                owned.last_attempt = None;
                                owned.unconfirmed_drafts.remove(&input_revision);
                            }
                            // 无法确认事务是否落盘时，禁止继续递增本地 revision 覆盖数据库。
                            PreparationFailure::Persistence => owned.invalidated = true,
                            PreparationFailure::Native => {}
                        }
                        CLIAgentSessionsModel::handle(ctx)
                            .update(ctx, |model, _| model.revoke_owned_grok_input(view.view_id));
                        view.show_error_toast(message, ctx);
                    }
                }
            },
        );
    }

    fn receive_owned_grok_event(
        &mut self,
        input_revision: Uuid,
        snapshot: Rc<CliInputSubmission>,
        events: Receiver<GrokOwnedWorkerEvent>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.spawn(
            async move {
                let event = events.recv().await;
                (event, events)
            },
            move |view, (event, events), ctx| {
                if !view
                    .grok_owned_input
                    .as_ref()
                    .is_some_and(|owned| owned.owner.input_revision == input_revision)
                {
                    return;
                }
                match event {
                    Ok(GrokOwnedWorkerEvent::Claimed(_)) => {
                        view.receive_owned_grok_event(input_revision, snapshot, events, ctx)
                    }
                    Ok(GrokOwnedWorkerEvent::NativePermissionPending { .. }) => {
                        // 只让原生审批界面可见；输入桥没有允许、拒绝或发送回车的权限。
                        view.close_cli_agent_rich_input(
                            CLIAgentRichInputCloseReason::AutoToggle,
                            ctx,
                        );
                        view.receive_owned_grok_event(input_revision, snapshot, events, ctx);
                    }
                    Ok(GrokOwnedWorkerEvent::Finished(_)) => {
                        let owned = view.grok_owned_input.as_mut().unwrap();
                        owned.sending = false;
                        owned.worker = None;
                        owned.unconfirmed_drafts.remove(&input_revision);
                        CLIAgentSessionsModel::handle(ctx)
                            .update(ctx, |model, _| model.revoke_owned_grok_input(view.view_id));
                        if !view.owned_grok_identity_matches() {
                            return;
                        }
                        let attachments_unchanged = view
                            .ai_context_model
                            .as_ref(ctx)
                            .pending_attachments_revision()
                            == snapshot.attachments_revision;
                        let editor_unchanged = view.input.update(ctx, |input, ctx| {
                            input.acknowledge_cli_input_submission(&snapshot.editor_revision, ctx)
                        });
                        if attachments_unchanged {
                            view.ai_context_model
                                .update(ctx, |model, ctx| model.clear_pending_attachments(ctx));
                        }
                        let draft = view.input.as_ref(ctx).buffer_text(ctx);
                        CLIAgentSessionsModel::handle(ctx)
                            .update(ctx, |model, _| model.set_draft(view.view_id, draft));
                        // 原生 ACK 只确认这条消息；不合成 PromptSubmit 覆盖已经到达的 Stop。
                        if editor_unchanged && attachments_unchanged {
                            view.maybe_close_rich_input_after_submit(ctx);
                        }
                        ctx.notify();
                    }
                    Ok(GrokOwnedWorkerEvent::Stopped { .. }) | Err(_) => {
                        let owned = view.grok_owned_input.as_mut().unwrap();
                        owned.sending = false;
                        owned.worker = None;
                        CLIAgentSessionsModel::handle(ctx)
                            .update(ctx, |model, _| model.revoke_owned_grok_input(view.view_id));
                        view.show_error_toast(crate::t!("cli-agent-grok-owned-input-claimed"), ctx);
                    }
                }
            },
        );
    }
}
