//! tmux 新窗格只经受核验的 daemon RPC 启动；现有 PTY 不接收启动命令。

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::terminal_binding::Snapshot;
use super::{DismissibleToast, TerminalView, ToastStack};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::proto::{
    TerminalBindingOwned, TerminalBindingOwnedAgent, TerminalBindingScope,
    TerminalBindingStartOwned,
};
use crate::remote_server::tmux_owned_client::{Journal, Launch};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::cli_agent_sessions::event::{CLIAgentEvent, CLIAgentEventType};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Binding,
    Requesting,
    Waiting,
    Active,
    Detached,
    Exited,
    Failed,
}

pub(super) struct TmuxOwned {
    operation: Uuid,
    snapshot: Snapshot,
    client: Arc<RemoteServerClient>,
    scope: TerminalBindingScope,
    launch: Option<Launch>,
    agent: CLIAgent,
    attempt: Option<Uuid>,
    pane_cwd: Option<String>,
    phase: Phase,
    restore: bool,
    explicit_restore: bool,
    launched: bool,
    native_session: Option<Uuid>,
    // 只保存已解析的真实 SessionStart；身份确认后再交给现有通知入口。
    starts: Vec<(Uuid, String)>,
}

/// 同一持久票据可以多次恢复；异步结果必须仍属于最初的线路确认和连接。
#[derive(Clone)]
pub(super) struct Instance {
    operation: Uuid,
    launch: Uuid,
    attempt: Uuid,
    binding: String,
    scope: TerminalBindingScope,
    snapshot: Snapshot,
    client: Arc<RemoteServerClient>,
}

impl PartialEq for Instance {
    fn eq(&self, other: &Self) -> bool {
        self.operation == other.operation
            && self.launch == other.launch
            && self.attempt == other.attempt
            && self.binding == other.binding
            && self.scope == other.scope
            && self.snapshot == other.snapshot
            && Arc::ptr_eq(&self.client, &other.client)
    }
}

fn protocol_agent(agent: CLIAgent) -> Option<TerminalBindingOwnedAgent> {
    match agent {
        CLIAgent::Claude => Some(TerminalBindingOwnedAgent::Claude),
        CLIAgent::Codex => Some(TerminalBindingOwnedAgent::Codex),
        CLIAgent::Grok => Some(TerminalBindingOwnedAgent::Grok),
        _ => None,
    }
}

fn cli_agent(agent: i32) -> Option<CLIAgent> {
    match TerminalBindingOwnedAgent::try_from(agent).ok()? {
        TerminalBindingOwnedAgent::Claude => Some(CLIAgent::Claude),
        TerminalBindingOwnedAgent::Codex => Some(CLIAgent::Codex),
        TerminalBindingOwnedAgent::Grok => Some(CLIAgent::Grok),
        TerminalBindingOwnedAgent::Unspecified => None,
    }
}

impl TerminalView {
    pub(crate) fn is_remote_tmux_owned_launch_available(
        &self,
        agent: CLIAgent,
        ctx: &AppContext,
    ) -> bool {
        if !super::FeatureFlag::CLIAgentRichInput.is_enabled() || !super::is_agent_supported(&agent)
        {
            return false;
        }
        let Some((_, client)) = self.terminal_binding_snapshot(ctx) else {
            return false;
        };
        if self.tmux_remote_owned.as_ref().is_some_and(|owned| {
            self.tmux_operation_is_current(owned.operation, ctx)
                && matches!(
                    owned.phase,
                    Phase::Preparing | Phase::Binding | Phase::Requesting | Phase::Waiting
                )
        }) {
            return false;
        }
        if !client.tmux_owned_available() {
            return false;
        }
        match agent {
            CLIAgent::Claude => client.cli_image_claude_read_available(),
            CLIAgent::Codex => client.cli_codex_owned_available(),
            CLIAgent::Grok => client.cli_grok_owned_available(),
            _ => false,
        }
    }

    pub(crate) fn can_restore_remote_tmux_owned(&self, ctx: &AppContext) -> bool {
        super::FeatureFlag::CLIAgentRichInput.is_enabled()
            && self
                .terminal_binding_snapshot(ctx)
                .is_some_and(|(_, client)| client.tmux_owned_available())
            && self.tmux_remote_owned.as_ref().is_none_or(|owned| {
                !self.tmux_operation_is_current(owned.operation, ctx)
                    || !matches!(
                        owned.phase,
                        Phase::Preparing | Phase::Binding | Phase::Requesting | Phase::Waiting
                    )
            })
    }

    fn tmux_operation_is_current(&self, operation: Uuid, ctx: &AppContext) -> bool {
        let Some(owned) = &self.tmux_remote_owned else {
            return false;
        };
        owned.operation == operation
            && owned.client.terminal_binding_scope_is_current(&owned.scope)
            && self
                .terminal_binding_snapshot(ctx)
                .is_some_and(|(snapshot, client)| {
                    snapshot == owned.snapshot && Arc::ptr_eq(&client, &owned.client)
                })
    }

    pub(super) fn invalidate_remote_tmux_owned(&mut self, ctx: &AppContext) {
        let Some(owned) = self.tmux_remote_owned.as_ref() else {
            return;
        };
        if matches!(owned.phase, Phase::Exited | Phase::Failed | Phase::Detached) {
            return;
        }
        if !self.tmux_operation_is_current(owned.operation, ctx)
            || owned.attempt.is_some()
                && self.current_remote_terminal_binding(ctx).is_none()
                && !matches!(owned.phase, Phase::Binding)
        {
            let owned = self.tmux_remote_owned.as_mut().unwrap();
            owned.phase = Phase::Detached;
            owned.native_session = None;
            owned.starts.clear();
            self.revoke_remote_owned_codex();
            self.revoke_remote_owned_grok();
        }
    }

    pub(crate) fn start_remote_tmux_owned(&mut self, agent: CLIAgent, ctx: &mut ViewContext<Self>) {
        if !self.is_remote_tmux_owned_launch_available(agent, ctx) {
            return;
        }
        let Some(protocol) = protocol_agent(agent) else {
            return;
        };
        let Some((snapshot, client)) = self.terminal_binding_snapshot(ctx) else {
            return;
        };
        let Some(scope) = client.terminal_binding_scope(snapshot.session) else {
            return;
        };
        let operation = Uuid::new_v4();
        self.cancel_remote_terminal_binding();
        self.tmux_remote_owned = Some(TmuxOwned {
            operation,
            snapshot: snapshot.clone(),
            client,
            scope: scope.clone(),
            launch: None,
            agent,
            attempt: None,
            pane_cwd: None,
            phase: Phase::Preparing,
            restore: false,
            explicit_restore: false,
            launched: false,
            native_session: None,
            starts: Vec::new(),
        });
        ctx.spawn(
            blocking::unblock(move || {
                Journal::open()?.reserve(
                    scope.host_id,
                    snapshot.session.as_u64(),
                    snapshot.block.to_string(),
                    protocol,
                )
            }),
            move |view, result, ctx| {
                if !view.tmux_operation_is_current(operation, ctx) {
                    return;
                }
                match result {
                    Ok(launch) => {
                        view.tmux_remote_owned.as_mut().unwrap().launch = Some(launch);
                        view.begin_tmux_owned_binding(ctx);
                    }
                    Err(_) => view.fail_tmux_owned(ctx),
                }
            },
        );
        ctx.notify();
    }

    /// 用户显式恢复时只读原意图；即使之前没有收到 Start 响应，也不重新启动。
    pub(crate) fn restore_remote_tmux_owned(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.can_restore_remote_tmux_owned(ctx) {
            return;
        }
        let Some((snapshot, client)) = self.terminal_binding_snapshot(ctx) else {
            return;
        };
        let Some(scope) = client.terminal_binding_scope(snapshot.session) else {
            return;
        };
        let retained = self
            .tmux_remote_owned
            .as_ref()
            .and_then(|owned| owned.launch.clone());
        let operation = Uuid::new_v4();
        self.cancel_remote_terminal_binding();
        self.tmux_remote_owned = Some(TmuxOwned {
            operation,
            snapshot: snapshot.clone(),
            client,
            scope: scope.clone(),
            launch: None,
            agent: CLIAgent::Unknown,
            attempt: None,
            pane_cwd: None,
            phase: Phase::Preparing,
            restore: true,
            explicit_restore: true,
            launched: false,
            native_session: None,
            starts: Vec::new(),
        });
        ctx.spawn(
            blocking::unblock(move || {
                let journal = Journal::open()?;
                let launch = match retained {
                    Some(launch) => {
                        let persisted = journal.read(launch.id)?;
                        if persisted != launch {
                            return Err(std::io::Error::other("tmux 意图已改变"));
                        }
                        persisted
                    }
                    None => journal.unique_for_block(
                        None,
                        snapshot.session.as_u64(),
                        &snapshot.block.to_string(),
                    )?,
                };
                Ok(launch)
            }),
            move |view, result, ctx| {
                if !view.tmux_operation_is_current(operation, ctx) {
                    return;
                }
                match result {
                    Ok(launch) => {
                        let Some(agent) = cli_agent(launch.agent) else {
                            view.fail_tmux_owned(ctx);
                            return;
                        };
                        let owned = view.tmux_remote_owned.as_mut().unwrap();
                        owned.agent = agent;
                        owned.launch = Some(launch);
                        view.begin_tmux_owned_binding(ctx);
                    }
                    Err(_) => view.fail_tmux_owned(ctx),
                }
            },
        );
    }

    fn begin_tmux_owned_binding(&mut self, ctx: &mut ViewContext<Self>) {
        // 新目标必须先撤销旧 listener/输入权限，不能把兄弟 pane 的会话接到新操作。
        self.revoke_remote_owned_codex();
        self.revoke_remote_owned_grok();
        self.codex_remote_owned = None;
        self.grok_remote_owned = None;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.remove_session(self.view_id, ctx)
        });
        let attempt = self.begin_remote_terminal_binding(ctx);
        let owned = self.tmux_remote_owned.as_mut().unwrap();
        owned.attempt = attempt;
        owned.phase = if attempt.is_some() {
            Phase::Binding
        } else {
            Phase::Failed
        };
        if attempt.is_none() {
            self.fail_tmux_owned(ctx);
        }
        ctx.notify();
    }

    pub(super) fn continue_tmux_owned_after_binding(
        &mut self,
        attempt: Uuid,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(owned) = self.tmux_remote_owned.as_ref() else {
            return;
        };
        if owned.attempt != Some(attempt) || owned.phase != Phase::Binding {
            return;
        }
        if !self.tmux_operation_is_current(owned.operation, ctx) {
            self.invalidate_remote_tmux_owned(ctx);
            return;
        }
        let Some(cwd) = self.current_remote_terminal_pane_cwd(ctx) else {
            self.fail_tmux_owned(ctx);
            return;
        };
        self.tmux_remote_owned.as_mut().unwrap().pane_cwd = Some(cwd);
        self.request_tmux_owned(0, ctx);
    }

    fn request_tmux_owned(&mut self, poll: u32, ctx: &mut ViewContext<Self>) {
        let Some((client, scope, attempt, binding_id)) = self.current_remote_terminal_binding(ctx)
        else {
            return;
        };
        let Some(owned) = self.tmux_remote_owned.as_mut() else {
            return;
        };
        if owned.attempt != Some(attempt)
            || matches!(
                owned.phase,
                Phase::Requesting | Phase::Failed | Phase::Exited | Phase::Detached
            )
        {
            return;
        }
        let Some(launch) = owned.launch.clone() else {
            return;
        };
        let operation = owned.operation;
        let status = owned.restore || poll > 0;
        // 此后即使本地写记录失败，恢复也只能查状态，不能再进入 Start。
        owned.restore = true;
        owned.phase = Phase::Requesting;
        let request = TerminalBindingStartOwned {
            opaque_binding_id: binding_id,
            launch_id: launch.id.to_string(),
            launch_key: launch.key.to_string(),
            agent: launch.agent,
        };
        ctx.spawn(
            async move {
                if poll > 0 {
                    warpui::r#async::Timer::after(Duration::from_millis(
                        250 * (1_u64 << poll.min(4)),
                    ))
                    .await;
                }
                if !status {
                    let claim = launch.clone();
                    if blocking::unblock(move || Journal::open()?.claim_start(&claim))
                        .await
                        .is_err()
                    {
                        return None;
                    }
                    client
                        .start_terminal_owned(scope, attempt, request)
                        .await
                        .ok()
                } else {
                    client
                        .status_terminal_owned(scope, attempt, request)
                        .await
                        .ok()
                }
            },
            move |view, reply, ctx| {
                if !view.tmux_operation_is_current(operation, ctx)
                    || view
                        .current_remote_terminal_binding(ctx)
                        .is_none_or(|(_, _, current, _)| current != attempt)
                {
                    view.invalidate_remote_tmux_owned(ctx);
                    return;
                }
                let Some(owned) = view.tmux_remote_owned.as_mut() else {
                    return;
                };
                owned.phase = Phase::Waiting;
                // Start 回应只用于确认已派发；绑定消费者前再查询原启动的当前状态。
                if !status {
                    view.request_tmux_owned(1, ctx);
                    return;
                }
                match reply {
                    Some(reply) if matches!(reply.phase.as_str(), "started" | "running") => {
                        view.accept_tmux_owned_reply(reply, ctx);
                        if view.tmux_remote_owned.as_ref().is_some_and(|owned| {
                            owned.native_session.is_none() && owned.phase != Phase::Failed
                        }) {
                            view.request_tmux_owned(poll.saturating_add(1), ctx);
                        }
                    }
                    Some(reply) if reply.phase == "exited" => view.finish_tmux_owned(ctx),
                    Some(reply) if reply.phase == "failed" => view.fail_tmux_owned(ctx),
                    Some(_) | None => {
                        if poll < 7 {
                            view.request_tmux_owned(poll + 1, ctx);
                        } else {
                            let owned = view.tmux_remote_owned.as_mut().unwrap();
                            owned.phase = Phase::Detached;
                            owned.native_session = None;
                            owned.starts.clear();
                            view.cancel_remote_terminal_binding();
                            view.show_error_toast(crate::t!("cli-agent-tmux-owned-unknown"), ctx);
                        }
                    }
                }
                ctx.notify();
            },
        );
    }

    fn accept_tmux_owned_reply(
        &mut self,
        reply: TerminalBindingOwned,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(owned) = self.tmux_remote_owned.as_ref() else {
            return;
        };
        let Some(launch) = owned.launch.clone() else {
            return;
        };
        if reply.launch_id != launch.id.to_string() || reply.agent != launch.agent {
            self.fail_tmux_owned(ctx);
            return;
        }
        let native = reply
            .native_session_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .filter(|id| !id.is_nil());
        if owned.native_session.is_some() && native.is_some() && owned.native_session != native {
            self.fail_tmux_owned(ctx);
            return;
        }
        let Some(cwd) = owned.pane_cwd.clone() else {
            self.fail_tmux_owned(ctx);
            return;
        };
        let agent = owned.agent;
        self.tmux_remote_owned.as_mut().unwrap().phase = Phase::Active;
        self.tmux_remote_owned.as_mut().unwrap().launched = true;
        let accepted = match agent {
            CLIAgent::Claude => reply.owned_reply_json.is_empty(),
            CLIAgent::Codex => {
                self.accept_tmux_owned_codex(&launch, &cwd, &reply.owned_reply_json, ctx)
            }
            CLIAgent::Grok => {
                self.accept_tmux_owned_grok(&launch, &cwd, &reply.owned_reply_json, ctx)
            }
            _ => false,
        };
        if !accepted {
            self.fail_tmux_owned(ctx);
            return;
        }
        if let Some(native) = native {
            self.tmux_remote_owned.as_mut().unwrap().native_session = Some(native);
        }
        self.replay_tmux_session_start(ctx);
    }

    pub(super) fn set_tmux_owned_native_session(&mut self, agent: CLIAgent, id: Uuid) -> bool {
        let Some(owned) = self.tmux_remote_owned.as_mut() else {
            return true;
        };
        if owned.agent != agent || id.is_nil() || owned.native_session.is_some_and(|old| old != id)
        {
            return false;
        }
        owned.native_session = Some(id);
        true
    }

    pub(super) fn replay_tmux_session_start(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(owned) = self.tmux_remote_owned.as_mut() else {
            return;
        };
        let Some(native) = owned.native_session else {
            return;
        };
        let body = owned
            .starts
            .iter()
            .position(|(id, _)| *id == native)
            .map(|index| owned.starts.swap_remove(index).1);
        owned.starts.clear();
        if let Some(body) = body {
            let sentinel = Some(super::CLI_AGENT_NOTIFICATION_SENTINEL);
            if CLIAgentSessionsModel::as_ref(ctx)
                .session(self.view_id)
                .is_some_and(|session| session.listener.is_some())
            {
                if let Some(event) = super::parse_event(sentinel, &body) {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                        sessions.update_from_event(self.view_id, &event, ctx)
                    });
                    self.bind_cli_agent_hook_input_target(ctx);
                    self.observe_remote_owned_codex_start(ctx);
                    self.observe_remote_owned_grok_start(ctx);
                }
            } else {
                self.handle_cli_agent_notification(sentinel, &body, ctx);
            }
        }
    }

    pub(super) fn accept_tmux_cli_notification(
        &mut self,
        event: &CLIAgentEvent,
        body: &str,
        ctx: &AppContext,
    ) -> bool {
        let Some(owned) = self.tmux_remote_owned.as_ref() else {
            return true;
        };
        if self
            .terminal_binding_snapshot(ctx)
            .is_some_and(|(snapshot, _)| snapshot.block != owned.snapshot.block)
        {
            return true;
        }
        if !self.tmux_operation_is_current(owned.operation, ctx) || owned.agent != event.agent {
            return false;
        }
        let Some(id) = event
            .session_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|id| !id.is_nil())
        else {
            return false;
        };
        let owned = self.tmux_remote_owned.as_mut().unwrap();
        if owned.native_session == Some(id)
            && owned.launched
            && !matches!(owned.phase, Phase::Detached | Phase::Exited | Phase::Failed)
        {
            return true;
        }
        if owned.native_session.is_none()
            && event.event == CLIAgentEventType::SessionStart
            && body.len() <= 64 * 1024
            && owned.starts.len() < 8
            && !owned.starts.iter().any(|(previous, _)| *previous == id)
        {
            owned.starts.push((id, body.into()));
        }
        false
    }

    pub(super) fn tmux_owned_launch_is_current(&self, id: Uuid, ctx: &AppContext) -> bool {
        self.tmux_remote_owned.as_ref().is_some_and(|owned| {
            owned.launched
                && !matches!(owned.phase, Phase::Detached | Phase::Exited | Phase::Failed)
                && owned.launch.as_ref().is_some_and(|launch| launch.id == id)
                && self.tmux_operation_is_current(owned.operation, ctx)
                && self
                    .current_remote_terminal_binding(ctx)
                    .is_some_and(|(_, _, attempt, _)| owned.attempt == Some(attempt))
        })
    }

    pub(super) fn tmux_owned_instance(&self, id: Uuid, ctx: &AppContext) -> Option<Instance> {
        if !self.tmux_owned_launch_is_current(id, ctx) {
            return None;
        }
        let owned = self.tmux_remote_owned.as_ref()?;
        let (client, scope, attempt, binding) = self.current_remote_terminal_binding(ctx)?;
        Some(Instance {
            operation: owned.operation,
            launch: id,
            attempt,
            binding,
            scope,
            snapshot: owned.snapshot.clone(),
            client,
        })
    }

    pub(super) fn tmux_owned_instance_is_current(
        &self,
        instance: &Instance,
        ctx: &AppContext,
    ) -> bool {
        self.tmux_owned_instance(instance.launch, ctx).as_ref() == Some(instance)
    }

    pub(super) fn tmux_owned_cwd(
        &self,
        agent: CLIAgent,
        session: super::SessionId,
        block: &super::BlockId,
    ) -> Option<&str> {
        let owned = self.tmux_remote_owned.as_ref()?;
        (owned.agent == agent
            && owned.launched
            && !matches!(owned.phase, Phase::Detached | Phase::Exited | Phase::Failed)
            && owned.snapshot.session == session
            && &owned.snapshot.block == block)
            .then_some(owned.pane_cwd.as_deref())
            .flatten()
    }

    pub(super) fn tmux_owned_image_reference(
        &self,
        agent: CLIAgent,
        native: &str,
        ctx: &AppContext,
    ) -> Result<Option<(String, String, String)>, ()> {
        let Some(owned) = self.tmux_remote_owned.as_ref() else {
            return Ok(None);
        };
        if self
            .terminal_binding_snapshot(ctx)
            .is_some_and(|(snapshot, _)| snapshot.block != owned.snapshot.block)
        {
            return Ok(None);
        }
        if !owned.launched
            || matches!(owned.phase, Phase::Detached | Phase::Exited | Phase::Failed)
            || owned.agent != agent
            || !self.tmux_operation_is_current(owned.operation, ctx)
            || owned.native_session.map(|id| id.to_string()).as_deref() != Some(native)
        {
            return Err(());
        }
        let (_, _, attempt, binding) = self.current_remote_terminal_binding(ctx).ok_or(())?;
        if owned.attempt != Some(attempt) {
            return Err(());
        }
        let launch = owned.launch.as_ref().ok_or(())?;
        Ok(Some((
            binding,
            launch.id.to_string(),
            launch.key.to_string(),
        )))
    }

    fn finish_tmux_owned(&mut self, ctx: &mut ViewContext<Self>) {
        let explicit_restore = self.tmux_remote_owned.as_mut().is_some_and(|owned| {
            owned.phase = Phase::Exited;
            owned.launched = false;
            owned.native_session = None;
            owned.starts.clear();
            owned.explicit_restore
        });
        self.cancel_remote_terminal_binding();
        self.revoke_remote_owned_codex();
        self.revoke_remote_owned_grok();
        if explicit_restore {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |stack, ctx| {
                stack.add_ephemeral_toast(
                    DismissibleToast::default(crate::t!("cli-agent-tmux-owned-exited")),
                    window_id,
                    ctx,
                );
            });
        }
        ctx.notify();
    }

    fn fail_tmux_owned(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(owned) = self.tmux_remote_owned.as_mut() {
            owned.phase = Phase::Failed;
            owned.native_session = None;
            owned.starts.clear();
        }
        self.cancel_remote_terminal_binding();
        self.revoke_remote_owned_codex();
        self.revoke_remote_owned_grok();
        self.show_error_toast(crate::t!("cli-agent-tmux-owned-unavailable"), ctx);
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "tmux_remote_owned_tests.rs"]
mod tests;
