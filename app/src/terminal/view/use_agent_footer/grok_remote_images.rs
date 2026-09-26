//! 远端专属 Grok 富输入走类型化侧车；未知回执保留草稿，原生审批仍由用户处理。

use std::io::Cursor;
use std::rc::Rc;
use std::sync::Arc;

use base64::Engine;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warpui::{AppContext, EntityId, SingletonEntity, ViewContext};

use super::{CliInputSubmission, TerminalView};
use crate::remote_server::cli_image_grok_client::{self as remote, Journal, Launch};
use crate::remote_server::cli_image_grok_protocol::{Action, Input, Observation, Png, Reply};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::remote_server::proto::CliImageStagingScope;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::{
    CLIAgentRichInputCloseReason, CLIAgentSessionsModel, GrokPermissionEvidence,
    GrokPermissionObservation,
};

#[derive(Clone)]
struct Binding {
    launch: Launch,
    observed: GrokPermissionObservation,
    listener: EntityId,
    events: EntityId,
    generation: Uuid,
    attempt: Uuid,
}

impl TerminalView {
    fn remote_grok_input_binding(
        &self,
        generation: Uuid,
        ctx: &AppContext,
    ) -> Option<(Arc<RemoteServerClient>, Binding)> {
        if !self.remote_owned_grok_is_current(ctx) {
            return None;
        }
        let owned = self.grok_remote_owned.as_ref()?;
        let native = owned.native_session?;
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        let GrokPermissionEvidence::Observed(observed) =
            &session.session_context.grok_permission_evidence
        else {
            return None;
        };
        let target = self.cli_agent_hook_input_target.as_ref()?;
        if session.agent != CLIAgent::Grok
            || session.remote_host.is_none()
            || !session.received_rich_notification
            || observed.session_id != native.to_string()
            || observed.cwd != owned.launch.cwd
            || observed.mode != "default"
            || session.listener.as_ref().map(|listener| listener.id()) != Some(target.listener_id)
            || target.native_session_id != observed.session_id
            || target.model_events_id != self.model_events_handle.id()
            || CLIAgentSessionsModel::as_ref(ctx).input_generation(self.view_id) != Some(generation)
        {
            return None;
        }
        let client = RemoteServerManager::as_ref(ctx)
            .client_for_session(warp_core::SessionId::from(owned.launch.terminal_session))?
            .clone();
        Some((
            client,
            Binding {
                launch: owned.launch.clone(),
                observed: observed.clone(),
                listener: target.listener_id,
                events: target.model_events_id,
                generation,
                attempt: Uuid::new_v4(),
            },
        ))
    }

    fn remote_grok_binding_matches(&self, binding: &Binding, ctx: &AppContext) -> bool {
        self.remote_grok_input_binding(binding.generation, ctx)
            .is_some_and(|(_, current)| {
                current.launch.ticket == binding.launch.ticket
                    && current.observed == binding.observed
                    && current.listener == binding.listener
                    && current.events == binding.events
            })
    }

    pub(in crate::terminal::view) fn submit_remote_owned_grok_input(
        &mut self,
        text: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(generation) = CLIAgentSessionsModel::as_ref(ctx).input_generation(self.view_id)
        else {
            return;
        };
        let Some((client, binding)) = self.remote_grok_input_binding(generation, ctx) else {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-unavailable"), ctx);
            return;
        };
        if self
            .grok_remote_owned
            .as_ref()
            .is_none_or(|owned| owned.sending)
        {
            return;
        }
        if !self.ai_context_model.as_ref(ctx).pending_files().is_empty() {
            self.show_error_toast(crate::t!("cli-agent-input-remote-file-unavailable"), ctx);
            return;
        }
        let images = self
            .ai_context_model
            .as_ref(ctx)
            .pending_images()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if text.trim().is_empty() && images.is_empty() {
            return;
        }
        if !CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
            sessions.begin_input_submission(self.view_id, generation)
        }) {
            self.show_error_toast(crate::t!("cli-agent-input-still-sending"), ctx);
            return;
        }
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
        let Ok((scope, revision)) = remote::scope(&client, &binding.launch, binding.attempt) else {
            self.fail_cli_agent_text_submit(
                generation,
                crate::t!("cli-agent-grok-owned-input-unavailable"),
                ctx,
            );
            return;
        };
        let owned = self.grok_remote_owned.as_mut().unwrap();
        owned.sending = true;
        owned.sending_generation = Some(binding.attempt);
        owned.pending = Some((client.clone(), scope.clone(), revision));
        remote::register(
            self.view_id,
            client.clone(),
            scope.clone(),
            revision,
            binding.launch.ticket.clone(),
            binding.observed.clone(),
        );
        let worker_binding = binding.clone();
        let worker_client = client.clone();
        ctx.spawn(
            async move {
                let input = blocking::unblock(move || {
                    let mut pngs = Vec::new();
                    for image in images {
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(&image.data)
                            .map_err(|_| ())?;
                        if bytes.len() > crate::util::image::MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT {
                            return Err(());
                        }
                        let decoded = image::ImageReader::new(Cursor::new(bytes))
                            .with_guessed_format()
                            .map_err(|_| ())?
                            .decode()
                            .map_err(|_| ())?;
                        let mut output = Cursor::new(Vec::new());
                        decoded
                            .write_to(&mut output, image::ImageFormat::Png)
                            .map_err(|_| ())?;
                        let bytes = output.into_inner();
                        pngs.push(Png {
                            sha256: format!("{:x}", Sha256::digest(&bytes)),
                            data: base64::engine::general_purpose::STANDARD.encode(bytes),
                        });
                    }
                    let input = Input {
                        message_id: Uuid::new_v4(),
                        text,
                        images: pngs,
                    };
                    crate::remote_server::cli_image_grok_protocol::encode(&input)?;
                    Ok::<_, ()>(input)
                })
                .await?;
                // 先回收原票据已有 ACK；查询失败不会重发任何历史输入。
                let _ = remote::recover_replies(&worker_client, &worker_binding.launch).await;
                let saved = input.clone();
                let launch = worker_binding.launch.clone();
                let subject =
                    blocking::unblock(move || Journal::open()?.claim_input(&launch, &saved))
                        .await
                        .map_err(|_| ())?;
                Ok::<_, ()>((input, subject))
            },
            move |view, result, ctx| {
                let Ok((input, subject)) = result else {
                    view.fail_remote_grok_worker(&binding, ctx);
                    return;
                };
                // 编辑器版本含线程内 Rc；派发前在 UI 线程核对，不把版本快照移入后台任务。
                if !view.remote_grok_binding_matches(&binding, ctx)
                    || view
                        .input
                        .as_ref(ctx)
                        .editor()
                        .as_ref(ctx)
                        .buffer_revision(ctx)
                        != snapshot.editor_revision
                    || view
                        .ai_context_model
                        .as_ref(ctx)
                        .pending_attachments_revision()
                        != snapshot.attachments_revision
                {
                    view.fail_remote_grok_worker(&binding, ctx);
                    return;
                }
                let worker_binding = binding.clone();
                let worker_scope = scope.clone();
                let worker_client = client.clone();
                ctx.spawn(
                    async move {
                        let message_id = input.message_id;
                        let observation = Observation {
                            native_session: Uuid::parse_str(&worker_binding.observed.session_id)
                                .map_err(|_| ())?,
                            cwd: worker_binding.observed.cwd.clone(),
                            event_id: worker_binding.observed.session_start_event_id.clone(),
                            permission_mode: worker_binding.observed.mode.clone(),
                            permission_revision: Uuid::new_v4(),
                            binding_id: Uuid::new_v4(),
                        };
                        let reply = remote::request(
                            &worker_client,
                            worker_scope,
                            revision,
                            Action::Submit {
                                ticket: worker_binding.launch.ticket.clone(),
                                observation,
                                input,
                            },
                        )
                        .await
                        .map_err(|_| ())?;
                        Journal::open()
                            .and_then(|journal| {
                                journal.record_reply(&worker_binding.launch, &reply)
                            })
                            .map_err(|_| ())?;
                        Ok::<_, ()>((message_id, subject, reply))
                    },
                    move |view, result, ctx| match result {
                        Ok((message, subject, reply)) => view.receive_remote_grok_reply(
                            client, scope, revision, binding, snapshot, message, subject, reply,
                            ctx,
                        ),
                        Err(()) => view.fail_remote_grok_worker(&binding, ctx),
                    },
                );
            },
        );
    }

    fn fail_remote_grok_worker(&mut self, binding: &Binding, ctx: &mut ViewContext<Self>) {
        self.finish_remote_grok_worker(binding);
        if self.remote_grok_binding_matches(binding, ctx) {
            if let Some(owned) = &mut self.grok_remote_owned {
                owned.sending = false;
                owned.revoke();
            }
            self.fail_cli_agent_text_submit(
                binding.generation,
                crate::t!("cli-agent-grok-owned-input-claimed"),
                ctx,
            );
        }
    }

    /// 只结束本次传输占用；未知草稿仍由持久账本拒绝再次派发。
    fn finish_remote_grok_worker(&mut self, binding: &Binding) {
        if let Some(owned) = &mut self.grok_remote_owned {
            if owned.launch.ticket == binding.launch.ticket
                && owned.sending_generation == Some(binding.attempt)
            {
                owned.sending = false;
                owned.sending_generation = None;
                owned.revoke();
            }
        }
    }

    fn receive_remote_grok_reply(
        &mut self,
        client: Arc<RemoteServerClient>,
        scope: CliImageStagingScope,
        revision: u64,
        binding: Binding,
        snapshot: Rc<CliInputSubmission>,
        message: Uuid,
        subject: String,
        reply: Reply,
        ctx: &mut ViewContext<Self>,
    ) {
        let Reply::Input {
            ticket,
            message_id,
            subject_sha256,
            state,
            native_prompt_id,
            native_ack_sha256,
        } = &reply
        else {
            self.finish_remote_grok_worker(&binding);
            if self.remote_grok_binding_matches(&binding, ctx) {
                if let Some(owned) = &mut self.grok_remote_owned {
                    owned.sending = false;
                }
                self.fail_cli_agent_text_submit(
                    binding.generation,
                    crate::t!("cli-agent-grok-owned-input-claimed"),
                    ctx,
                );
            }
            return;
        };
        if ticket != &binding.launch.ticket || *message_id != message || *subject_sha256 != subject
        {
            return;
        }
        if matches!(state.as_str(), "finished" | "cancelled")
            && native_prompt_id.is_some()
            && native_ack_sha256.is_some()
        {
            // 精确 ACK 已独立持久化；新 listener/会话/输入代际绝不被旧回调清空。
            if let Some(owned) = &mut self.grok_remote_owned {
                if owned.launch.ticket == binding.launch.ticket
                    && owned.sending_generation == Some(binding.attempt)
                {
                    owned.sending = false;
                    owned.sending_generation = None;
                    owned.pending = None;
                }
            }
            if self.remote_grok_binding_matches(&binding, ctx) {
                self.release_cli_agent_input_submission(binding.generation, ctx);
                let attachments_unchanged = self
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_attachments_revision()
                    == snapshot.attachments_revision;
                let editor_unchanged = self.input.update(ctx, |input, ctx| {
                    input.acknowledge_cli_input_submission(&snapshot.editor_revision, ctx)
                });
                if attachments_unchanged {
                    self.ai_context_model
                        .update(ctx, |model, ctx| model.clear_pending_attachments(ctx));
                }
                let draft = self.input.as_ref(ctx).buffer_text(ctx);
                CLIAgentSessionsModel::handle(ctx)
                    .update(ctx, |sessions, _| sessions.set_draft(self.view_id, draft));
                // 原生最终 ACK 不合成 PromptSubmit，避免覆盖已经到达的 Stop。
                if editor_unchanged && attachments_unchanged {
                    self.maybe_close_rich_input_after_submit(ctx);
                }
                ctx.notify();
            }
            return;
        }
        if !matches!(state.as_str(), "unknown" | "permission_pending") {
            return;
        }
        if state == "permission_pending" && self.remote_grok_binding_matches(&binding, ctx) {
            self.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::AutoToggle, ctx);
        }
        // 查询只取回已知 message ID；即使关闭输入框或代际变化也不再次发送内容。
        let query_client = client.clone();
        let query_scope = scope.clone();
        let launch = binding.launch.clone();
        ctx.spawn(
            async move {
                warpui::r#async::Timer::after(std::time::Duration::from_millis(800)).await;
                let reply = remote::request(
                    &query_client,
                    query_scope,
                    revision,
                    Action::InputStatus {
                        ticket: launch.ticket.clone(),
                        message_id: message,
                    },
                )
                .await
                .map_err(|_| ())?;
                Journal::open()
                    .and_then(|journal| journal.record_reply(&launch, &reply))
                    .map_err(|_| ())?;
                Ok::<_, ()>(reply)
            },
            move |view, result, ctx| {
                if let Ok(reply) = result {
                    view.receive_remote_grok_reply(
                        client, scope, revision, binding, snapshot, message, subject, reply, ctx,
                    );
                } else {
                    view.finish_remote_grok_worker(&binding);
                    if view.remote_grok_binding_matches(&binding, ctx) {
                        view.fail_cli_agent_text_submit(
                            binding.generation,
                            crate::t!("cli-agent-grok-owned-input-claimed"),
                            ctx,
                        );
                    }
                }
            },
        );
    }
}
