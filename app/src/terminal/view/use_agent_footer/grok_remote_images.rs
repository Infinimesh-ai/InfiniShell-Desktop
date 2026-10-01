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
use crate::remote_server::cli_image_grok_protocol::{
    Action, Input, Observation, Png, Reply, Ticket,
};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::remote_server::proto::CliImageStagingScope;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::{
    CLIAgentRichInputCloseReason, CLIAgentSessionsModel, GrokPermissionEvidence,
    GrokPermissionObservation,
};
use crate::terminal::view::grok_remote_owned::ReadonlyIdentity;

#[derive(Clone)]
struct Binding {
    client: Arc<RemoteServerClient>,
    launch: Launch,
    identity: InputIdentity,
    listener: EntityId,
    events: EntityId,
    generation: Uuid,
    attempt: Uuid,
}

#[derive(Clone, PartialEq, Eq)]
enum InputIdentity {
    Hook(GrokPermissionObservation),
    Owned(ReadonlyIdentity),
}

fn same_remote_grok_attempt<T>(
    ticket: &Ticket,
    attempt: Uuid,
    client: &Arc<T>,
    active_ticket: &Ticket,
    sending_generation: Option<Uuid>,
    pending: Option<(&Arc<T>, &str)>,
) -> bool {
    ticket == active_ticket
        && sending_generation == Some(attempt)
        && pending.is_some_and(|(pending_client, generation)| {
            Arc::ptr_eq(client, pending_client) && generation == attempt.to_string()
        })
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
        let identity = if let Some(identity) = self.remote_owned_grok_readonly_identity(ctx) {
            InputIdentity::Owned(identity.clone())
        } else {
            let GrokPermissionEvidence::Observed(observed) =
                &session.session_context.grok_permission_evidence
            else {
                return None;
            };
            if !session.received_rich_notification || observed.mode != "default" {
                return None;
            }
            InputIdentity::Hook(observed.clone())
        };
        let (session_id, cwd) = match &identity {
            InputIdentity::Hook(observed) => (observed.session_id.clone(), observed.cwd.as_str()),
            InputIdentity::Owned(identity) => {
                (identity.native_session.to_string(), identity.cwd.as_str())
            }
        };
        let target = self.cli_agent_hook_input_target.as_ref()?;
        if session.agent != CLIAgent::Grok
            || session.remote_host.is_none()
            || session_id != native.to_string()
            || cwd != owned.launch.cwd
            || session.listener.as_ref().map(|listener| listener.id()) != Some(target.listener_id)
            || target.native_session_id != session_id
            || target.model_events_id != self.model_events_handle.id()
            || CLIAgentSessionsModel::as_ref(ctx).input_generation(self.view_id) != Some(generation)
        {
            return None;
        }
        let client = RemoteServerManager::as_ref(ctx)
            .client_for_session(warp_core::SessionId::from(
                owned
                    .launch
                    .tmux_terminal_session
                    .unwrap_or(owned.launch.terminal_session),
            ))?
            .clone();
        Some((
            client.clone(),
            Binding {
                client,
                launch: owned.launch.clone(),
                identity,
                listener: target.listener_id,
                events: target.model_events_id,
                generation,
                attempt: Uuid::new_v4(),
            },
        ))
    }

    fn remote_grok_attempt_matches(&self, binding: &Binding) -> bool {
        self.grok_remote_owned.as_ref().is_some_and(|owned| {
            same_remote_grok_attempt(
                &binding.launch.ticket,
                binding.attempt,
                &binding.client,
                &owned.launch.ticket,
                owned.sending_generation,
                owned
                    .pending
                    .as_ref()
                    .map(|(client, scope, _)| (client, scope.input_generation.as_str())),
            )
        })
    }

    fn remote_grok_binding_matches(&self, binding: &Binding, ctx: &AppContext) -> bool {
        if !self.remote_grok_attempt_matches(binding)
            || !self.grok_remote_owned.as_ref().is_some_and(|owned| {
                owned
                    .pending
                    .as_ref()
                    .is_some_and(|(_, scope, _)| binding.client.cli_image_scope_is_current(scope))
            })
        {
            return false;
        }
        self.remote_grok_input_binding(binding.generation, ctx)
            .is_some_and(|(client, current)| {
                Arc::ptr_eq(&client, &binding.client)
                    && current.launch.ticket == binding.launch.ticket
                    && current.identity == binding.identity
                    && current.listener == binding.listener
                    && current.events == binding.events
            })
    }

    fn remote_grok_consumption_matches(&self, binding: &Binding, ctx: &AppContext) -> bool {
        let sessions = CLIAgentSessionsModel::as_ref(ctx);
        if sessions
            .remote_image_consumption_revision(self.view_id, binding.attempt)
            .is_none()
        {
            return false;
        }
        let Some(generation) = sessions.input_generation(self.view_id) else {
            return false;
        };
        // 仅清稿收据可跟随自动收起/恢复；原发送绑定与写入租约仍保持旧代际。
        let mut current = binding.clone();
        current.generation = generation;
        self.remote_grok_binding_matches(&current, ctx)
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
        if !CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
            sessions.register_remote_image_submission_owner(
                self.view_id,
                generation,
                binding.attempt,
                Some(snapshot.editor_revision.clone()),
            )
        }) {
            self.fail_cli_agent_text_submit(
                generation,
                crate::t!("cli-agent-grok-owned-input-unavailable"),
                ctx,
            );
            return;
        }
        let owned = self.grok_remote_owned.as_mut().unwrap();
        owned.sending = true;
        owned.sending_generation = Some(binding.attempt);
        owned.pending = Some((client.clone(), scope.clone(), revision));
        match &binding.identity {
            InputIdentity::Hook(observed) => remote::register(
                self.view_id,
                client.clone(),
                scope.clone(),
                revision,
                binding.launch.ticket.clone(),
                observed.clone(),
            ),
            InputIdentity::Owned(identity) => remote::register_owned(
                self.view_id,
                client.clone(),
                scope.clone(),
                revision,
                binding.launch.ticket.clone(),
                identity.native_session,
                identity.cwd.clone(),
            ),
        }
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
                    let launch = binding.launch.clone();
                    let message_id = input.message_id;
                    ctx.spawn(
                        blocking::unblock(move || {
                            Journal::open()?.record_not_dispatched(&launch, message_id, &subject)
                        }),
                        move |view, result, ctx| {
                            // 此分支尚未创建 Submit future；不把真正派发后的错误改成可重试。
                            if result.is_err() {
                                view.fail_remote_grok_worker(&binding, ctx);
                                return;
                            }
                            let current = view.remote_grok_binding_matches(&binding, ctx);
                            view.finish_remote_grok_worker(&binding, ctx);
                            if current {
                                view.show_error_toast(
                                    crate::t!("cli-agent-grok-owned-input-not-dispatched"),
                                    ctx,
                                );
                            }
                        },
                    );
                    return;
                }
                // 持久账本已领取唯一派发；先保存原稿收据，最终精确 ACK 才能确认消费。
                // 此记录不恢复已撤销的写权限，也不允许查询或重连重新派发。
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                    sessions.register_remote_image_consumption(
                        view.view_id,
                        binding.generation,
                        binding.attempt,
                    );
                });
                let worker_binding = binding.clone();
                let worker_scope = scope.clone();
                let worker_client = client.clone();
                ctx.spawn(
                    async move {
                        let message_id = input.message_id;
                        let action = match &worker_binding.identity {
                            InputIdentity::Hook(observed) => Action::Submit {
                                ticket: worker_binding.launch.ticket.clone(),
                                observation: Observation {
                                    native_session: Uuid::parse_str(&observed.session_id)
                                        .map_err(|_| ())?,
                                    cwd: observed.cwd.clone(),
                                    event_id: observed.session_start_event_id.clone(),
                                    permission_mode: observed.mode.clone(),
                                    permission_revision: Uuid::new_v4(),
                                    binding_id: Uuid::new_v4(),
                                },
                                input,
                            },
                            InputIdentity::Owned(_) => Action::SubmitOwned {
                                ticket: worker_binding.launch.ticket.clone(),
                                binding_id: Uuid::new_v4(),
                                input,
                            },
                        };
                        let reply = remote::request(&worker_client, worker_scope, revision, action)
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
        let current = self.remote_grok_binding_matches(binding, ctx);
        self.finish_remote_grok_worker(binding, ctx);
        if current {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-input-claimed"), ctx);
        }
    }

    /// 只结束本次传输占用与提交租约；未知草稿仍由持久账本拒绝再次派发。
    fn finish_remote_grok_worker(&mut self, binding: &Binding, ctx: &mut ViewContext<Self>) {
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
            sessions.finish_remote_image_consumption(self.view_id, binding.attempt);
            sessions.finish_remote_image_submission(
                self.view_id,
                binding.generation,
                binding.attempt,
            );
        });
        if !self.remote_grok_attempt_matches(binding) {
            return;
        }
        if let Some(owned) = &mut self.grok_remote_owned {
            owned.sending = false;
            owned.sending_generation = None;
            owned.revoke();
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
            let current = self.remote_grok_binding_matches(&binding, ctx);
            self.finish_remote_grok_worker(&binding, ctx);
            if current {
                self.show_error_toast(crate::t!("cli-agent-grok-owned-input-claimed"), ctx);
            }
            return;
        };
        if ticket != &binding.launch.ticket || *message_id != message || *subject_sha256 != subject
        {
            return;
        }
        if state == "rejected_before_enqueue"
            && native_prompt_id.is_none()
            && native_ack_sha256.as_ref().is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            let current = self.remote_grok_binding_matches(&binding, ctx);
            self.finish_remote_grok_worker(&binding, ctx);
            if current {
                self.show_error_toast(crate::t!("cli-agent-grok-owned-input-unavailable"), ctx);
            }
            return;
        }
        if matches!(state.as_str(), "finished" | "cancelled")
            && native_prompt_id.is_some()
            && native_ack_sha256.is_some()
        {
            let consumed =
                state == "finished" && self.remote_grok_consumption_matches(&binding, ctx);
            // 精确 ACK 已独立持久化；取消、编辑、新附件或新会话都不能清空当前草稿。
            if self.remote_grok_attempt_matches(&binding) {
                if let Some(owned) = &mut self.grok_remote_owned {
                    owned.sending = false;
                    owned.sending_generation = None;
                    owned.pending = None;
                }
            }
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                sessions.finish_remote_image_submission(
                    self.view_id,
                    binding.generation,
                    binding.attempt,
                );
            });
            if consumed {
                // 原生最终 ACK 不合成 PromptSubmit，避免覆盖已经到达的 Stop。
                if self.complete_remote_cli_image_consumption(binding.attempt, &snapshot, ctx) {
                    self.maybe_close_rich_input_after_submit(ctx);
                }
                ctx.notify();
            } else {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                    sessions.finish_remote_image_consumption(self.view_id, binding.attempt);
                });
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
                    let current = view.remote_grok_binding_matches(&binding, ctx);
                    view.finish_remote_grok_worker(&binding, ctx);
                    if current {
                        view.show_error_toast(crate::t!("cli-agent-grok-owned-input-claimed"), ctx);
                    }
                }
            },
        );
    }
}

#[cfg(test)]
#[path = "grok_remote_images_tests.rs"]
mod tests;
