//! 独立终端输出线路确认；不生成 CLI hook、不发送用户输入，也不自动重放。

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;
use warpui::r#async::Timer;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::{BlockId, SessionId, ShellType, TerminalView};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::remote_server::proto::{TerminalBindingBound, TerminalBindingScope};
use crate::terminal::model::ansi::TerminalBindingChallenge;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) session: SessionId,
    pub(super) block: BlockId,
    pub(super) cwd: Option<String>,
}

enum Phase {
    Waiting { written: bool, nonce: Option<Uuid> },
    Acknowledging,
    Bound(TerminalBindingBound),
}

pub(super) struct TerminalBinding {
    snapshot: Snapshot,
    client: Arc<RemoteServerClient>,
    scope: TerminalBindingScope,
    attempt: Uuid,
    phase: Phase,
}

impl Drop for TerminalBinding {
    fn drop(&mut self) {
        self.client
            .cancel_terminal_binding(self.scope.clone(), self.attempt);
    }
}

impl TerminalBinding {
    fn accept_challenge(&mut self, challenge: &TerminalBindingChallenge) -> bool {
        if challenge.session_id != self.snapshot.session || challenge.attempt != self.attempt {
            return false;
        }
        let Phase::Waiting { nonce, .. } = &mut self.phase else {
            return false;
        };
        if nonce.is_some() {
            return false;
        }
        *nonce = Some(challenge.nonce);
        true
    }

    fn take_ack(&mut self) -> Option<Uuid> {
        let Phase::Waiting {
            written: true,
            nonce: Some(nonce),
        } = self.phase
        else {
            return None;
        };
        self.phase = Phase::Acknowledging;
        Some(nonce)
    }
}

impl TerminalView {
    pub(super) fn terminal_binding_snapshot(
        &self,
        ctx: &AppContext,
    ) -> Option<(Snapshot, Arc<RemoteServerClient>)> {
        // 此入口只能在 UI 线程、未持 TerminalModel 锁时调用。
        let model = self.model.lock();
        if !model.block_list().is_bootstrapped()
            || model.shared_session_status().is_sharer_or_viewer()
            || model.is_conversation_transcript_viewer()
        {
            return None;
        }
        let block = model.block_list().active_block();
        let session_id = block.session_id()?;
        let session = self.sessions.as_ref(ctx).get(session_id)?;
        if session.is_local() && !session.is_subshell_or_ssh()
            || !matches!(
                session.shell().shell_type(),
                ShellType::Bash | ShellType::Zsh
            )
        {
            return None;
        }
        let snapshot = Snapshot {
            session: session_id,
            block: block.id().clone(),
            cwd: block
                .metadata()
                .current_working_directory()
                .map(str::to_owned),
        };
        drop(model);
        let client = RemoteServerManager::as_ref(ctx)
            .client_for_session(session_id)?
            .clone();
        client
            .terminal_binding_available()
            .then_some((snapshot, client))
    }

    fn terminal_binding_is_current(&self, ctx: &AppContext) -> bool {
        let Some(binding) = &self.remote_terminal_binding else {
            return false;
        };
        self.terminal_binding_snapshot(ctx)
            .is_some_and(|(snapshot, client)| {
                snapshot == binding.snapshot
                    && Arc::ptr_eq(&client, &binding.client)
                    && client.terminal_binding_scope_is_current(&binding.scope)
            })
    }

    pub(super) fn invalidate_remote_terminal_binding(&mut self, ctx: &AppContext) {
        if self.remote_terminal_binding.is_some() && !self.terminal_binding_is_current(ctx) {
            self.remote_terminal_binding = None;
        }
    }

    pub(crate) fn cancel_remote_terminal_binding(&mut self) {
        if self.remote_terminal_binding.take().is_some() {
            #[cfg(all(
                feature = "local_fs",
                feature = "local_tty",
                any(
                    all(target_os = "macos", target_arch = "aarch64"),
                    all(target_os = "linux", target_arch = "x86_64"),
                    all(windows, target_arch = "x86_64")
                )
            ))]
            {
                self.revoke_remote_owned_codex();
                self.revoke_remote_owned_grok();
            }
        }
    }

    /// 由显式产品入口调用；先登记 attempt，避免 TTY 输出先于 Begin 响应到达。
    pub(crate) fn begin_remote_terminal_binding(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Uuid> {
        self.cancel_remote_terminal_binding();
        let (snapshot, client) = self.terminal_binding_snapshot(ctx)?;
        let scope = client.terminal_binding_scope(snapshot.session)?;
        let attempt = Uuid::new_v4();
        self.remote_terminal_binding = Some(TerminalBinding {
            snapshot,
            client: client.clone(),
            scope: scope.clone(),
            attempt,
            phase: Phase::Waiting {
                written: false,
                nonce: None,
            },
        });
        ctx.spawn(
            async move { client.begin_terminal_binding(scope, attempt).await },
            move |view, reply, ctx| {
                view.invalidate_remote_terminal_binding(ctx);
                let Some(binding) = view
                    .remote_terminal_binding
                    .as_mut()
                    .filter(|binding| binding.attempt == attempt)
                else {
                    return;
                };
                if reply.is_err() {
                    view.cancel_remote_terminal_binding();
                    #[cfg(all(
                        feature = "local_fs",
                        feature = "local_tty",
                        any(
                            all(target_os = "macos", target_arch = "aarch64"),
                            all(target_os = "linux", target_arch = "x86_64"),
                            all(windows, target_arch = "x86_64")
                        )
                    ))]
                    view.continue_tmux_owned_after_binding(attempt, ctx);
                    ctx.notify();
                    return;
                }
                if let Phase::Waiting { written, .. } = &mut binding.phase {
                    *written = true;
                    view.maybe_ack_remote_terminal_binding(ctx);
                    ctx.notify();
                }
            },
        );
        // 只有本次未完成的 attempt 超时；旧计时器不得撤销新绑定。
        ctx.spawn(
            Timer::after(Duration::from_secs(30)),
            move |view, _, ctx| {
                if view
                    .remote_terminal_binding
                    .as_ref()
                    .is_some_and(|binding| {
                        binding.attempt == attempt && !matches!(binding.phase, Phase::Bound(_))
                    })
                {
                    view.cancel_remote_terminal_binding();
                    #[cfg(all(
                        feature = "local_fs",
                        feature = "local_tty",
                        any(
                            all(target_os = "macos", target_arch = "aarch64"),
                            all(target_os = "linux", target_arch = "x86_64"),
                            all(windows, target_arch = "x86_64")
                        )
                    ))]
                    view.continue_tmux_owned_after_binding(attempt, ctx);
                    ctx.notify();
                }
            },
        );
        Some(attempt)
    }

    pub(super) fn handle_terminal_binding_challenge(
        &mut self,
        challenge: &TerminalBindingChallenge,
        ctx: &mut ViewContext<Self>,
    ) {
        self.invalidate_remote_terminal_binding(ctx);
        let Some(binding) = &mut self.remote_terminal_binding else {
            return;
        };
        if binding.accept_challenge(challenge) {
            self.maybe_ack_remote_terminal_binding(ctx);
            ctx.notify();
        }
    }

    fn maybe_ack_remote_terminal_binding(&mut self, ctx: &mut ViewContext<Self>) {
        self.invalidate_remote_terminal_binding(ctx);
        let Some(binding) = &mut self.remote_terminal_binding else {
            return;
        };
        let Some(nonce) = binding.take_ack() else {
            return;
        };
        let attempt = binding.attempt;
        let scope = binding.scope.clone();
        let client = binding.client.clone();
        ctx.spawn(
            async move { client.ack_terminal_binding(scope, attempt, nonce).await },
            move |view, reply, ctx| {
                view.invalidate_remote_terminal_binding(ctx);
                let Some(binding) = view
                    .remote_terminal_binding
                    .as_mut()
                    .filter(|binding| binding.attempt == attempt)
                else {
                    return;
                };
                if !matches!(binding.phase, Phase::Acknowledging) {
                    return;
                }
                match reply {
                    Ok(id) => binding.phase = Phase::Bound(id),
                    Err(_) => view.cancel_remote_terminal_binding(),
                }
                #[cfg(all(
                    feature = "local_fs",
                    feature = "local_tty",
                    any(
                        all(target_os = "macos", target_arch = "aarch64"),
                        all(target_os = "linux", target_arch = "x86_64"),
                        all(windows, target_arch = "x86_64")
                    )
                ))]
                view.continue_tmux_owned_after_binding(attempt, ctx);
                ctx.notify();
            },
        );
    }

    pub(super) fn current_remote_terminal_pane_cwd(&self, ctx: &AppContext) -> Option<String> {
        if !self.terminal_binding_is_current(ctx) {
            return None;
        }
        let Phase::Bound(bound) = &self.remote_terminal_binding.as_ref()?.phase else {
            return None;
        };
        Some(bound.pane_cwd.clone())
    }

    /// 消费方每次使用前仍须检查原 view/block/client/epoch，不能保存跨重连的 ID。
    pub(crate) fn current_remote_terminal_binding(
        &self,
        ctx: &AppContext,
    ) -> Option<(Arc<RemoteServerClient>, TerminalBindingScope, Uuid, String)> {
        if !self.terminal_binding_is_current(ctx) {
            return None;
        }
        let binding = self.remote_terminal_binding.as_ref()?;
        let Phase::Bound(id) = &binding.phase else {
            return None;
        };
        Some((
            binding.client.clone(),
            binding.scope.clone(),
            binding.attempt,
            id.opaque_binding_id.clone(),
        ))
    }
}

#[cfg(test)]
#[path = "terminal_binding_tests.rs"]
pub(super) mod tests;
