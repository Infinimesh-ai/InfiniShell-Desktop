//! 当前 SSH/tmux pane 的显式 Grok 启动；保留一次性票据，重连只查询与关联。

use std::sync::Arc;

use uuid::Uuid;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::{BlockId, SessionId, ShellType, TerminalView};
use crate::remote_server::cli_image_grok_client::{self as remote, Journal, Launch};
use crate::remote_server::cli_image_grok_protocol::{Action, Reply};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::remote_server::proto::CliImageStagingScope;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, GrokPermissionEvidence};
use crate::terminal::model::session::command_executor::shell_quote_arg;

#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    session: SessionId,
    block: BlockId,
    cwd: String,
    shell: ShellType,
}

pub(super) struct RemoteOwned {
    pub(super) launch: Launch,
    pub(super) native_session: Option<Uuid>,
    pub(super) pending: Option<(Arc<RemoteServerClient>, CliImageStagingScope, u64)>,
    pub(super) sending: bool,
    pub(super) sending_generation: Option<Uuid>,
    snapshot: Snapshot,
    querying: bool,
}

impl Drop for RemoteOwned {
    fn drop(&mut self) {
        self.revoke();
    }
}

impl RemoteOwned {
    pub(super) fn revoke(&mut self) {
        if let Some((client, scope, revision)) = self.pending.take() {
            if let Ok(generation) = Uuid::parse_str(&scope.input_generation) {
                if let Ok(body) = remote::request_body(
                    &scope,
                    revision,
                    Action::Revoke {
                        ticket: self.launch.ticket.clone(),
                        generation,
                    },
                ) {
                    client.revoke_cli_grok_owned(scope, body);
                }
            }
        }
    }
}

impl TerminalView {
    pub(crate) fn is_remote_owned_grok_launch_available(&self, ctx: &AppContext) -> bool {
        self.remote_grok_snapshot(false, ctx).is_some()
    }
    fn remote_grok_snapshot(
        &self,
        active: bool,
        ctx: &AppContext,
    ) -> Option<(Snapshot, Arc<RemoteServerClient>)> {
        let model = self.model.lock();
        let block = model.block_list().active_block();
        if !model.block_list().is_bootstrapped()
            || block.is_active_and_long_running() != active
            || model.shared_session_status().is_sharer_or_viewer()
            || model.is_conversation_transcript_viewer()
        {
            return None;
        }
        let session_id = block.session_id()?;
        let session = self.sessions.as_ref(ctx).get(session_id)?;
        let shell = session.shell().shell_type();
        if session.is_local() && !session.is_subshell_or_ssh()
            || !matches!(shell, ShellType::Bash | ShellType::Zsh)
        {
            return None;
        }
        let cwd = block.metadata().current_working_directory()?.to_owned();
        let snapshot = Snapshot {
            session: session_id,
            block: block.id().clone(),
            cwd,
            shell,
        };
        drop(model);
        let client = RemoteServerManager::as_ref(ctx)
            .client_for_session(session_id)?
            .clone();
        client
            .cli_grok_owned_available()
            .then_some((snapshot, client))
    }

    /// 入口只接显式 owned 启动意图；远端调用绝不采用本机扫描的 Grok 路径。
    pub(super) fn prepare_remote_owned_grok_pending(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some((snapshot, client)) = self.remote_grok_snapshot(false, ctx) else {
            return false;
        };
        let Some(intent) = &self.pending_specific_cli_agent_launch else {
            return true;
        };
        if intent.preparation.is_some() {
            return true;
        }
        let Some(host) = client.cli_image_reference_host() else {
            return true;
        };
        let editor_revision = intent.revision.clone();
        let token = Uuid::new_v4();
        self.pending_specific_cli_agent_launch
            .as_mut()
            .unwrap()
            .preparation = Some(token);
        let prepared = snapshot.clone();
        let remote_client = client.clone();
        ctx.spawn(
            async move {
                let (launch, fresh) = blocking::unblock(move || {
                    Journal::open()?.reserve(
                        host,
                        prepared.session,
                        prepared.block.to_string(),
                        prepared.cwd,
                    )
                })
                .await
                .map_err(|_| ())?;
                let (scope, revision) =
                    remote::scope(&remote_client, &launch, token).map_err(|_| ())?;
                let action = if fresh {
                    Action::Reserve {
                        ticket: launch.ticket.clone(),
                        cwd: launch.cwd.clone(),
                    }
                } else {
                    Action::Status {
                        ticket: launch.ticket.clone(),
                    }
                };
                let reply = remote::request(&remote_client, scope, revision, action)
                    .await
                    .map_err(|_| ())?;
                let argv = match reply {
                    Reply::Reserved { ticket, argv } if fresh && ticket == launch.ticket => {
                        Some(argv)
                    }
                    Reply::Launch { ticket, .. } if !fresh && ticket == launch.ticket => None,
                    Reply::Reserved { .. }
                    | Reply::Launch { .. }
                    | Reply::Input { .. }
                    | Reply::Revoked { .. }
                    | Reply::Failed { .. } => return Err(()),
                };
                if argv.is_some() {
                    let claim = launch.clone();
                    blocking::unblock(move || Journal::open()?.claim_launch(&claim))
                        .await
                        .map_err(|_| ())?;
                }
                Ok::<_, ()>((launch, argv))
            },
            move |view, result, ctx| {
                let current =
                    view.pending_specific_cli_agent_launch
                        .as_ref()
                        .is_some_and(|intent| {
                            intent.owned_grok
                                && intent.preparation == Some(token)
                                && intent.revision == editor_revision
                        })
                        && view.input.as_ref(ctx).has_pending_command()
                        && view.input.as_ref(ctx).buffer_text(ctx)
                            == CLIAgent::Grok.command_prefix()
                        && view
                            .input
                            .as_ref(ctx)
                            .editor()
                            .as_ref(ctx)
                            .buffer_revision(ctx)
                            == editor_revision
                        && view.remote_grok_snapshot(false, ctx).is_some_and(
                            |(now, current_client)| {
                                now == snapshot && Arc::ptr_eq(&current_client, &client)
                            },
                        );
                let Ok((launch, argv)) = result else {
                    if current {
                        view.pending_specific_cli_agent_launch = None;
                        view.input
                            .update(ctx, |input, _| input.cancel_owned_cli_pending_command());
                        view.show_error_toast(
                            crate::t!("cli-agent-grok-remote-launch-unavailable"),
                            ctx,
                        );
                    }
                    return;
                };
                if !current {
                    let cancelling = client.clone();
                    ctx.spawn(
                        async move {
                            if let Ok((scope, revision)) =
                                remote::scope(&cancelling, &launch, Uuid::new_v4())
                            {
                                let _ = remote::request(
                                    &cancelling,
                                    scope,
                                    revision,
                                    Action::Cancel {
                                        ticket: launch.ticket,
                                    },
                                )
                                .await;
                            }
                        },
                        |_, (), _| {},
                    );
                    return;
                }
                view.pending_specific_cli_agent_launch = None;
                let Some(argv) = argv else {
                    view.input
                        .update(ctx, |input, _| input.cancel_owned_cli_pending_command());
                    view.show_error_toast(crate::t!("cli-agent-grok-owned-input-claimed"), ctx);
                    return;
                };
                // 回调仅消费当前未编辑启动草稿；argv 逐项原生 shell 转义，不拼接用户命令。
                let command = argv
                    .iter()
                    .map(|arg| shell_quote_arg(arg, snapshot.shell))
                    .collect::<Vec<_>>()
                    .join(" ");
                view.grok_remote_owned = Some(RemoteOwned {
                    launch,
                    native_session: None,
                    pending: None,
                    sending: false,
                    sending_generation: None,
                    snapshot,
                    querying: false,
                });
                let sent = view.input.update(ctx, |input, ctx| {
                    input.execute_owned_cli_command_once(&command, ctx)
                });
                view.awaiting_pending_command_completion = sent;
                if !sent {
                    view.show_error_toast(
                        crate::t!("cli-agent-grok-remote-launch-unavailable"),
                        ctx,
                    );
                }
            },
        );
        true
    }

    pub(super) fn remote_owned_grok_is_current(&self, ctx: &AppContext) -> bool {
        self.grok_remote_owned.as_ref().is_some_and(|owned| {
            self.remote_grok_snapshot(true, ctx)
                .is_some_and(|(snapshot, client)| {
                    snapshot == owned.snapshot
                        && client.cli_image_reference_host().as_deref() == Some(&owned.launch.host)
                })
        })
    }

    pub(super) fn revoke_remote_owned_grok(&mut self) {
        remote::revoke(self.view_id);
        if let Some(owned) = &mut self.grok_remote_owned {
            owned.revoke();
        }
    }

    pub(super) fn retire_remote_owned_grok_for_block(
        &mut self,
        block: &BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .grok_remote_owned
            .as_ref()
            .is_none_or(|owned| &owned.snapshot.block != block)
        {
            return;
        }
        self.revoke_remote_owned_grok();
        let Some(owned) = self.grok_remote_owned.take() else {
            return;
        };
        let Some(client) = RemoteServerManager::as_ref(ctx)
            .client_for_session(SessionId::from(owned.launch.terminal_session))
            .cloned()
        else {
            return;
        };
        let launch = owned.launch.clone();
        ctx.spawn(
            async move {
                if let Ok((scope, revision)) = remote::scope(&client, &launch, Uuid::new_v4()) {
                    let _ = remote::request(
                        &client,
                        scope,
                        revision,
                        Action::Cancel {
                            ticket: launch.ticket,
                        },
                    )
                    .await;
                }
            },
            |_, (), _| {},
        );
    }

    pub(super) fn observe_remote_owned_grok_start(&mut self, ctx: &mut ViewContext<Self>) {
        if self.grok_remote_owned.is_none() {
            self.restore_remote_owned_grok(ctx);
            return;
        }
        if !self.remote_owned_grok_is_current(ctx) {
            self.revoke_remote_owned_grok();
            return;
        }
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) else {
            return;
        };
        let GrokPermissionEvidence::Observed(observed) =
            &session.session_context.grok_permission_evidence
        else {
            self.revoke_remote_owned_grok();
            return;
        };
        if session.agent != CLIAgent::Grok
            || session.remote_host.is_none()
            || observed.mode != "default"
        {
            self.revoke_remote_owned_grok();
            return;
        }
        let observed = observed.clone();
        let Some((snapshot, client)) = self.remote_grok_snapshot(true, ctx) else {
            return;
        };
        let Some(owned) = &mut self.grok_remote_owned else {
            return;
        };
        if owned.querying || owned.native_session.is_some() {
            return;
        }
        let launch = owned.launch.clone();
        let expected_ticket = launch.ticket.clone();
        let Ok((scope, revision)) = remote::scope(&client, &launch, Uuid::new_v4()) else {
            return;
        };
        owned.querying = true;
        ctx.spawn(
            async move {
                remote::request(
                    &client,
                    scope,
                    revision,
                    Action::Status {
                        ticket: launch.ticket.clone(),
                    },
                )
                .await
            },
            move |view, reply, ctx| {
                let Some(owned) = &mut view.grok_remote_owned else {
                    return;
                };
                if owned.launch.ticket != expected_ticket || owned.snapshot != snapshot {
                    return;
                }
                owned.querying = false;
                if !view.remote_owned_grok_is_current(ctx) {
                    return;
                }
                let Some(current) = CLIAgentSessionsModel::as_ref(ctx).session(view.view_id) else {
                    return;
                };
                if current.session_context.grok_permission_evidence
                    != GrokPermissionEvidence::Observed(observed.clone())
                {
                    return;
                }
                let Some(owned) = &mut view.grok_remote_owned else {
                    return;
                };
                if owned.snapshot != snapshot {
                    return;
                }
                if let Ok(Reply::Launch {
                    ticket,
                    native_session: Some(native),
                    manifest_sha256: Some(hash),
                    phase,
                }) = reply
                {
                    if ticket == owned.launch.ticket
                        && native.to_string() == observed.session_id
                        && hash.len() == 64
                        && phase == "dispatched_unknown"
                    {
                        owned.native_session = Some(native);
                        view.bind_cli_agent_hook_input_target(ctx);
                        ctx.notify();
                    }
                }
            },
        );
    }

    fn restore_remote_owned_grok(&mut self, ctx: &mut ViewContext<Self>) {
        if self.grok_remote_restore_generation.is_some() {
            return;
        }
        let Some((snapshot, client)) = self.remote_grok_snapshot(true, ctx) else {
            return;
        };
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) else {
            return;
        };
        let GrokPermissionEvidence::Observed(observed) =
            &session.session_context.grok_permission_evidence
        else {
            return;
        };
        if session.agent != CLIAgent::Grok
            || session.remote_host.is_none()
            || observed.mode != "default"
        {
            return;
        }
        let observed = observed.clone();
        let Some(host) = client.cli_image_reference_host() else {
            return;
        };
        let query_snapshot = snapshot.clone();
        let expected = observed.session_id.clone();
        let generation = Uuid::new_v4();
        self.grok_remote_restore_generation = Some(generation);
        let expected_client = client.clone();
        ctx.spawn(
            async move {
                let candidates = blocking::unblock(move || {
                    Journal::open()?.known_launches(
                        &host,
                        query_snapshot.session,
                        &query_snapshot.cwd,
                    )
                })
                .await
                .ok()?;
                let mut matched = None;
                for launch in candidates {
                    let (scope, revision) = remote::scope(&client, &launch, Uuid::new_v4()).ok()?;
                    let reply = remote::request(
                        &client,
                        scope,
                        revision,
                        Action::Status {
                            ticket: launch.ticket.clone(),
                        },
                    )
                    .await
                    .ok()?;
                    if let Reply::Launch {
                        ticket,
                        native_session: Some(native),
                        manifest_sha256: Some(hash),
                        phase,
                    } = reply
                    {
                        if ticket == launch.ticket
                            && native.to_string() == expected
                            && hash.len() == 64
                            && phase == "dispatched_unknown"
                        {
                            if matched.is_some() {
                                return None;
                            }
                            let _ = remote::recover_replies(&client, &launch).await;
                            matched = Some((launch, native));
                        }
                    }
                }
                matched
            },
            move |view, result, ctx| {
                if view.grok_remote_restore_generation != Some(generation) {
                    return;
                }
                view.grok_remote_restore_generation = None;
                if view.grok_remote_owned.is_some()
                    || view
                        .remote_grok_snapshot(true, ctx)
                        .is_none_or(|(current, client)| {
                            current != snapshot || !Arc::ptr_eq(&client, &expected_client)
                        })
                {
                    return;
                }
                let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(view.view_id) else {
                    return;
                };
                if session.session_context.grok_permission_evidence
                    != GrokPermissionEvidence::Observed(observed.clone())
                {
                    return;
                }
                if let Some((launch, native)) = result {
                    view.grok_remote_owned = Some(RemoteOwned {
                        launch,
                        native_session: Some(native),
                        pending: None,
                        sending: false,
                        sending_generation: None,
                        snapshot,
                        querying: false,
                    });
                    view.bind_cli_agent_hook_input_target(ctx);
                    ctx.notify();
                }
            },
        );
    }
}
