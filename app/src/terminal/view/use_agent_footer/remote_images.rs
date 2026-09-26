//! 远程图片按 CLI 原生接口进入当前会话，消费回执与模型理解分别处理。

use std::io::Cursor;
use std::sync::Arc;

use base64::Engine;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::{CliInputSubmission, DismissibleToast, ImageContext, Rc, TerminalView, ToastStack};
use crate::remote_server::cli_image_submission::{
    self, NativeImageBinding, NativeImageConsumer, RemoteImageSubmission,
};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::util::image::MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT;

impl TerminalView {
    pub(super) fn register_remote_image_recovery(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            if event.terminal_view_id() == me.view_id
                && matches!(
                    event,
                    CLIAgentSessionsModelEvent::SessionUpdated {
                        agent: CLIAgent::Claude,
                        ..
                    } | CLIAgentSessionsModelEvent::StatusChanged {
                        agent: CLIAgent::Claude,
                        ..
                    }
                )
            {
                me.recover_current_claude_image_references(ctx);
            }
        });
        ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), |me, _, event, ctx| {
            if let RemoteServerManagerEvent::SessionConnected { session_id, .. }
            | RemoteServerManagerEvent::SessionReconnected { session_id, .. } = event
                && me.active_block_session_id() == Some(*session_id)
            {
                me.recover_current_remote_image_references(ctx);
                #[cfg(all(
                    feature = "local_fs",
                    feature = "local_tty",
                    any(
                        all(target_os = "macos", target_arch = "aarch64"),
                        all(target_os = "linux", target_arch = "x86_64"),
                        all(windows, target_arch = "x86_64")
                    )
                ))]
                me.observe_remote_owned_grok_start(ctx);
            }
        });
    }

    fn recover_current_claude_image_references(&self, ctx: &AppContext) {
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) else {
            return;
        };
        let Some(target) = &self.cli_agent_hook_input_target else {
            return;
        };
        if session.agent != CLIAgent::Claude
            || session.remote_host.is_none()
            || !session.received_rich_notification
            || session.listener.as_ref().map(|listener| listener.id()) != Some(target.listener_id)
            || session.session_context.session_id.as_ref() != Some(&target.native_session_id)
            || self.model_events_handle.id() != target.model_events_id
        {
            return;
        }
        let session_id = {
            let model = self.model.lock();
            let block = model.block_list().active_block();
            if block.id() != &target.block_id {
                return;
            }
            let Some(session_id) = block.session_id() else {
                return;
            };
            session_id
        };
        let Some(client) = RemoteServerManager::as_ref(ctx).client_for_session(session_id) else {
            return;
        };
        // 不读取当前输入代次，也不清草稿；旧代次仅通过账本中的精确回执完成回收。
        cli_image_submission::schedule_claude_reference_recovery(
            client.clone(),
            session_id,
            &target.native_session_id,
            ctx,
        );
    }

    fn recover_current_remote_image_references(&self, ctx: &AppContext) {
        let Some(session_id) = self.active_block_session_id() else {
            return;
        };
        let Some(client) = RemoteServerManager::as_ref(ctx).client_for_session(session_id) else {
            return;
        };
        cli_image_submission::schedule_reference_recovery(client.clone(), session_id, ctx);
    }

    fn remote_image_binding(
        &self,
        generation: Uuid,
        ctx: &AppContext,
    ) -> Option<(Arc<RemoteServerClient>, NativeImageBinding)> {
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        let target = self.cli_agent_hook_input_target.as_ref()?;
        let consumer = match session.agent {
            CLIAgent::Codex => {
                NativeImageConsumer::Codex(session.session_context.codex_process_evidence.clone()?)
            }
            CLIAgent::Claude => {
                let (process, transcript_path) =
                    session.session_context.claude_image_evidence.clone()?;
                NativeImageConsumer::Claude {
                    process,
                    transcript_path,
                }
            }
            CLIAgent::Gemini
            | CLIAgent::Grok
            | CLIAgent::Amp
            | CLIAgent::Droid
            | CLIAgent::OpenCode
            | CLIAgent::Copilot
            | CLIAgent::Pi
            | CLIAgent::OhMyPi
            | CLIAgent::Auggie
            | CLIAgent::CursorCli
            | CLIAgent::Goose
            | CLIAgent::DeepSeek
            | CLIAgent::Hermes
            | CLIAgent::Vibe
            | CLIAgent::Antigravity
            | CLIAgent::Omp
            | CLIAgent::WarpTui
            | CLIAgent::Unknown => return None,
        };
        let cwd = session.session_context.cwd.as_ref()?;
        if session.remote_host.is_none()
            || !session.received_rich_notification
            || session.listener.as_ref().map(|listener| listener.id()) != Some(target.listener_id)
            || session.session_context.session_id.as_ref() != Some(&target.native_session_id)
            || self.model_events_handle.id() != target.model_events_id
            || CLIAgentSessionsModel::as_ref(ctx).input_generation(self.view_id) != Some(generation)
        {
            return None;
        }
        let model = self.model.lock();
        let block = model.block_list().active_block();
        if !block.is_active_and_long_running() || block.id() != &target.block_id {
            return None;
        }
        let session_id = block.session_id()?;
        drop(model);
        let client = RemoteServerManager::as_ref(ctx)
            .client_for_session(session_id)?
            .clone();
        #[cfg(all(
            feature = "local_fs",
            feature = "local_tty",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64"),
                all(windows, target_arch = "x86_64")
            )
        ))]
        let codex_owned = self.remote_owned_codex_image_proof(ctx);
        #[cfg(not(all(
            feature = "local_fs",
            feature = "local_tty",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64"),
                all(windows, target_arch = "x86_64")
            )
        )))]
        let codex_owned = None;
        Some((
            client,
            NativeImageBinding {
                session_id,
                native_session_id: target.native_session_id.clone(),
                generation,
                listener_id: target.listener_id,
                model_events_id: target.model_events_id,
                block_id: target.block_id.to_string(),
                cwd: cwd.clone(),
                consumer,
                codex_owned,
            },
        ))
    }

    pub(super) fn is_remote_cli_image_input(&self, ctx: &AppContext) -> bool {
        let remote = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .is_some_and(|session| session.remote_host.is_some());
        if remote {
            self.recover_current_remote_image_references(ctx);
        }
        remote
    }

    fn remote_image_target_is_current(
        &self,
        submission: &RemoteImageSubmission,
        binding: &NativeImageBinding,
        ctx: &AppContext,
    ) -> bool {
        submission.is_live()
            && CLIAgentSessionsModel::as_ref(ctx)
                .is_input_submission_current(self.view_id, binding.generation)
            && self
                .remote_image_binding(binding.generation, ctx)
                .is_some_and(|(client, current)| {
                    Arc::ptr_eq(&client, &submission.client) && current == *binding
                })
    }

    pub(super) fn submit_remote_cli_images(
        &mut self,
        images: Vec<ImageContext>,
        text: Vec<u8>,
        generation: Uuid,
        snapshot: Option<Rc<CliInputSubmission>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some((client, binding)) = self
            .remote_image_binding(generation, ctx)
            .filter(|(client, binding)| binding.consumer.available(client))
        else {
            self.fail_cli_agent_text_submit(
                generation,
                crate::t!("cli-agent-input-remote-image-unavailable"),
                ctx,
            );
            return;
        };
        let Ok(submission) = RemoteImageSubmission::begin(self.view_id, client, binding.clone())
        else {
            self.fail_cli_agent_text_submit(
                generation,
                crate::t!("cli-agent-input-remote-image-unavailable"),
                ctx,
            );
            return;
        };
        let spawner = ctx.spawner();
        let worker = submission.clone();
        ctx.spawn(
            async move {
                let pngs = prepare_pngs(images, &text)?;
                if binding.consumer.is_claude()
                    && (pngs.iter().any(|image| image.len() > 20 * 1024 * 1024)
                        || pngs.iter().map(Vec::len).sum::<usize>() > 32 * 1024 * 1024)
                {
                    return Err(());
                }
                let text = String::from_utf8(text).map_err(|_| ())?;
                let mut digest = Sha256::new();
                digest.update((text.len() as u64).to_le_bytes());
                digest.update(text.as_bytes());
                for image in &pngs {
                    digest.update((image.len() as u64).to_le_bytes());
                    digest.update(image);
                }
                let subject: [u8; 32] = digest.finalize().into();
                worker.prepare(pngs).await?;
                let intent = worker.prepare_native_queue(subject)?;
                let claimed = {
                    let worker = worker.clone();
                    let binding = binding.clone();
                    spawner
                        .spawn(move |me, ctx| {
                            me.remote_image_target_is_current(&worker, &binding, ctx)
                                && worker.claim_native_write(subject)
                        })
                        .await
                };
                if !matches!(claimed, Ok(true)) {
                    return Err(());
                }
                // 原生输入只由远端类型化 RPC 派发一次；不写路径、Enter 或审批答复。
                let accepted = worker.submit_native_queue(text, subject, intent).await?;
                Ok((worker, binding, accepted))
            },
            move |me, result, ctx| match result {
                Ok((worker, binding, true))
                    if me.remote_image_target_is_current(&worker, &binding, ctx) =>
                {
                    me.complete_cli_agent_text_submit(generation, snapshot, ctx);
                    let message = if binding.consumer.is_claude() {
                        crate::t!("cli-agent-input-remote-claude-image-consumed")
                    } else {
                        crate::t!("cli-agent-input-remote-image-queued")
                    };
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |stack, ctx| {
                        stack.add_ephemeral_toast(
                            DismissibleToast::success(message),
                            window_id,
                            ctx,
                        );
                    });
                }
                Ok((_, _, true)) => {
                    // 回执已持久化，旧视图不清除新草稿，也不重新发送。
                }
                Ok((_, _, false)) | Err(()) => {
                    submission.revoke();
                    let message = if submission.native_write_was_claimed() {
                        crate::t!("cli-agent-input-remote-image-unconfirmed")
                    } else {
                        crate::t!("cli-agent-input-image-delivery-failed")
                    };
                    me.fail_cli_agent_text_submit(generation, message, ctx);
                }
            },
        );
    }

    /// 普通终端粘贴先形成图片卡片；用户提交后才上传，不占用本机剪贴板投递。
    pub(super) fn attach_remote_clipboard_images(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        let content = ctx.clipboard().read();
        self.input.update(ctx, |input, ctx| {
            input.attach_cli_clipboard_images(content, ctx)
        }) > 0
    }

    pub(super) fn attach_remote_dropped_images(
        &mut self,
        paths: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        if CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.view_id) {
            self.input.update(ctx, |input, ctx| {
                input.handle_pasted_or_dragdropped_image_filepaths(paths, ctx)
            });
        }
    }
}

fn prepare_pngs(images: Vec<ImageContext>, text: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
    // 图片附件不能被 ! 或 / 命令切换为 shell/内建命令；由用户在原生界面另行操作。
    if images.is_empty()
        || images.len() > 20
        || text
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| matches!(byte, b'!' | b'/'))
    {
        return Err(());
    }
    images
        .into_iter()
        .map(|image| {
            if image.data.len() > MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT.saturating_mul(4) / 3 + 4 {
                return Err(());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(image.data)
                .map_err(|_| ())?;
            if bytes.len() > MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT {
                return Err(());
            }
            let decoded = image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|_| ())?
                .decode()
                .map_err(|_| ())?;
            let mut png = Cursor::new(Vec::new());
            decoded
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|_| ())?;
            let png = png.into_inner();
            if png.len() > MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT {
                return Err(());
            }
            Ok(png)
        })
        .collect()
}
