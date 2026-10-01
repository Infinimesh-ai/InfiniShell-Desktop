//! 当前 SSH/tmux pane 的显式 Grok 启动；保留一次性票据，重连只查询与关联。

use std::sync::Arc;

use uuid::Uuid;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::tmux_remote_owned::Instance as TmuxInstance;
use super::{BlockId, SessionId, ShellType, TerminalView};
use crate::remote_server::cli_image_grok_client::{self as remote, Journal, Launch};
use crate::remote_server::cli_image_grok_protocol::{Action, Reply};
#[cfg(test)]
use crate::remote_server::cli_image_grok_protocol::Ticket;
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::remote_server::proto::CliImageStagingScope;
use crate::remote_server::tmux_owned_client::Launch as TmuxLaunch;
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
    tmux_instance: Option<TmuxInstance>,
    pub(super) launch: Launch,
    pub(super) native_session: Option<Uuid>,
    pub(super) pending: Option<(Arc<RemoteServerClient>, CliImageStagingScope, u64)>,
    pub(super) sending: bool,
    pub(super) sending_generation: Option<Uuid>,
    snapshot: Snapshot,
    querying: bool,
    readonly_identity: Option<ReadonlyIdentity>,
    readonly_query_epoch: Option<Uuid>,
}

/// 当前连接重新查询的专属原生身份，不是 hook 或运行状态。
#[derive(Clone, PartialEq, Eq)]
pub(super) struct ReadonlyIdentity {
    pub(super) native_session: Uuid,
    pub(super) cwd: String,
    manifest_sha256: String,
    epoch: Uuid,
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
    pub(super) fn accept_tmux_owned_grok(
        &mut self,
        intent: &TmuxLaunch,
        cwd: &str,
        bytes: &[u8],
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Ok(Reply::Launch {
            ticket,
            native_session,
            ..
        }) = serde_json::from_slice::<Reply>(bytes)
        else {
            return false;
        };
        if ticket.id != intent.id || ticket.key != intent.key {
            return false;
        }
        if let Some(id) = native_session {
            if !self.set_tmux_owned_native_session(CLIAgent::Grok, id) {
                return false;
            }
        }
        let Some(instance) = self.tmux_owned_instance(intent.id, ctx) else {
            return false;
        };
        let Some((snapshot, client)) = self.remote_grok_snapshot(true, ctx) else {
            return false;
        };
        if self.grok_remote_owned.as_ref().is_some_and(|owned| {
            owned.launch.ticket == ticket
                && owned.tmux_instance.as_ref() == Some(&instance)
                && owned.launch.tmux_terminal_session == Some(snapshot.session.as_u64())
                && owned.snapshot == snapshot
        }) {
            return true;
        }
        let launch = Launch {
            host: intent.host.clone(),
            terminal_session: intent.terminal_session,
            block_id: intent.block_id.clone(),
            cwd: cwd.into(),
            ticket,
            tmux_source: Some(intent.id),
            tmux_terminal_session: Some(snapshot.session.as_u64()),
        };
        let remembered = launch.clone();
        ctx.spawn(
            blocking::unblock(move || Journal::open()?.remember_tmux(&remembered)),
            move |view, result, ctx| {
                if !view.tmux_owned_instance_is_current(&instance, ctx)
                    || view.remote_grok_snapshot(true, ctx).is_none_or(
                        |(current, current_client)| {
                            current != snapshot || !Arc::ptr_eq(&current_client, &client)
                        },
                    )
                {
                    return;
                }
                if result.is_err() {
                    view.revoke_remote_owned_grok();
                    return;
                }
                if view.grok_remote_owned.as_ref().is_some_and(|owned| {
                    owned.launch.ticket == launch.ticket
                        && owned.tmux_instance.as_ref() == Some(&instance)
                        && owned.launch.tmux_terminal_session == launch.tmux_terminal_session
                        && owned.snapshot == snapshot
                }) {
                    return;
                }
                view.grok_remote_owned = Some(RemoteOwned {
                    tmux_instance: Some(instance),
                    launch,
                    native_session: None,
                    pending: None,
                    sending: false,
                    sending_generation: None,
                    snapshot,
                    querying: false,
                    readonly_identity: None,
                    readonly_query_epoch: None,
                });
                view.observe_remote_owned_grok_start(ctx);
            },
        );
        true
    }

    /// 仅构造已有内存终端的 owned 绑定；测试不经过菜单、持久账本或启动 RPC。
    #[cfg(test)]
    pub(super) fn bind_remote_owned_grok_for_test(
        &mut self,
        ticket: Ticket,
        native_session: Uuid,
        ctx: &AppContext,
    ) {
        let (snapshot, client) = self.remote_grok_snapshot(true, ctx).unwrap();
        self.grok_remote_owned = Some(RemoteOwned {
            tmux_instance: None,
            launch: Launch {
                tmux_source: None,
                tmux_terminal_session: None,
                host: client.cli_image_reference_host().unwrap(),
                terminal_session: snapshot.session.as_u64(),
                block_id: snapshot.block.to_string(),
                cwd: snapshot.cwd.clone(),
                ticket,
            },
            native_session: Some(native_session),
            pending: None,
            sending: false,
            sending_generation: None,
            snapshot,
            querying: false,
            readonly_identity: None,
            readonly_query_epoch: None,
        });
    }

    #[cfg(test)]
    pub(super) fn bind_tmux_owned_grok_for_test(
        &mut self,
        ticket: Ticket,
        native: Uuid,
        ctx: &AppContext,
    ) {
        let instance = self.tmux_owned_instance(ticket.id, ctx).unwrap();
        self.bind_remote_owned_grok_for_test(ticket, native, ctx);
        let owned = self.grok_remote_owned.as_mut().unwrap();
        owned.launch.tmux_source = Some(owned.launch.ticket.id);
        owned.launch.tmux_terminal_session = Some(owned.snapshot.session.as_u64());
        owned.tmux_instance = Some(instance);
    }

    #[cfg(test)]
    pub(super) fn remote_grok_identity_query_pending_for_test(&self) -> bool {
        self.grok_remote_owned
            .as_ref()
            .is_some_and(|owned| owned.querying)
    }

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
        let metadata = block.metadata();
        let cwd = self
            .tmux_owned_cwd(CLIAgent::Grok, session_id, block.id())
            .or_else(|| metadata.current_working_directory())?
            .to_owned();
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
                    | Reply::OwnedIdentity { .. }
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
                    tmux_instance: None,
                    launch,
                    native_session: None,
                    pending: None,
                    sending: false,
                    sending_generation: None,
                    snapshot,
                    querying: false,
                    readonly_identity: None,
                    readonly_query_epoch: None,
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
            (owned.launch.tmux_source.is_none()
                || owned
                    .tmux_instance
                    .as_ref()
                    .is_some_and(|instance| self.tmux_owned_instance_is_current(instance, ctx)))
                && self
                    .remote_grok_snapshot(true, ctx)
                    .is_some_and(|(snapshot, client)| {
                        snapshot == owned.snapshot
                            && (client.cli_image_reference_host().as_deref()
                                == Some(&owned.launch.host)
                                || owned.launch.tmux_source == Some(owned.launch.ticket.id)
                                    && owned.launch.tmux_terminal_session.is_some())
                    })
        })
    }

    pub(super) fn revoke_remote_owned_grok(&mut self) {
        remote::revoke(self.view_id);
        if let Some(owned) = &mut self.grok_remote_owned {
            owned.launch.tmux_terminal_session = None;
            owned.tmux_instance = None;
            owned.readonly_identity = None;
            owned.readonly_query_epoch = None;
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
        if self
            .tmux_restored_native_session(CLIAgent::Grok, ctx)
            .is_some()
        {
            if CLIAgentSessionsModel::as_ref(ctx)
                .session(self.view_id)
                .is_some_and(|session| session.session_context.grok_owned_identity_epoch.is_some())
            {
                self.observe_remote_owned_grok_identity(ctx);
                return;
            }
            if let Some(owned) = self.grok_remote_owned.as_mut() {
                let had_identity = owned.readonly_identity.take().is_some();
                let had_query = owned.readonly_query_epoch.take().is_some();
                if had_identity || had_query {
                    owned.native_session = None;
                    owned.querying = false;
                }
            }
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
        let instance = owned.tmux_instance.clone();
        let expected_ticket = launch.ticket.clone();
        let Ok((scope, revision)) = remote::scope(&client, &launch, Uuid::new_v4()) else {
            return;
        };
        owned.querying = true;
        let expected_client = client.clone();
        let expected_scope = scope.clone();
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
                // 先复核原请求所属的连接与恢复操作；旧回复不能清除新对象的查询状态。
                if !expected_client.cli_image_scope_is_current(&expected_scope)
                    || instance
                        .as_ref()
                        .is_some_and(|instance| !view.tmux_owned_instance_is_current(instance, ctx))
                    || view
                        .remote_grok_snapshot(true, ctx)
                        .is_none_or(|(current, client)| {
                            current != snapshot || !Arc::ptr_eq(&client, &expected_client)
                        })
                    || !view.remote_owned_grok_is_current(ctx)
                {
                    return;
                }
                let Some(owned) = &mut view.grok_remote_owned else {
                    return;
                };
                if owned.launch.ticket != expected_ticket
                    || owned.snapshot != snapshot
                    || owned.tmux_instance != instance
                {
                    return;
                }
                owned.querying = false;
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

    pub(in crate::terminal::view) fn remote_owned_grok_readonly_identity(
        &self,
        ctx: &AppContext,
    ) -> Option<&ReadonlyIdentity> {
        if !self.remote_owned_grok_is_current(ctx) {
            return None;
        }
        let native = self.tmux_restored_native_session(CLIAgent::Grok, ctx)?;
        let owned = self.grok_remote_owned.as_ref()?;
        let identity = owned.readonly_identity.as_ref()?;
        (identity.native_session == native
            && owned.native_session == Some(native)
            && CLIAgentSessionsModel::as_ref(ctx)
                .session(self.view_id)
                .is_some_and(|session| {
                    session.session_context.grok_owned_identity_epoch == Some(identity.epoch)
                })
            && identity.cwd == owned.launch.cwd)
            .then_some(identity)
    }

    fn observe_remote_owned_grok_identity(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(native) = self.tmux_restored_native_session(CLIAgent::Grok, ctx) else {
            return;
        };
        let Some((snapshot, client)) = self.remote_grok_snapshot(true, ctx) else {
            return;
        };
        let Some(epoch) = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .and_then(|session| session.session_context.grok_owned_identity_epoch)
        else {
            return;
        };
        let Some(owned) = self.grok_remote_owned.as_mut() else {
            return;
        };
        if owned.querying || owned.readonly_identity.is_some() {
            return;
        }
        let Some(instance) = owned.tmux_instance.clone() else {
            return;
        };
        let ticket = owned.launch.ticket.clone();
        let Ok((scope, revision)) = remote::scope(&client, &owned.launch, Uuid::new_v4()) else {
            return;
        };
        owned.querying = true;
        owned.readonly_query_epoch = Some(epoch);
        let expected_client = client.clone();
        let expected_scope = scope.clone();
        let requested_ticket = ticket.clone();
        ctx.spawn(
            async move {
                remote::request(
                    &client,
                    scope,
                    revision,
                    Action::Observe {
                        ticket: requested_ticket,
                    },
                )
                .await
            },
            move |view, reply, ctx| {
                if !expected_client.cli_image_scope_is_current(&expected_scope)
                    || !view.tmux_owned_instance_is_current(&instance, ctx)
                    || view.tmux_restored_native_session(CLIAgent::Grok, ctx) != Some(native)
                    || view
                        .remote_grok_snapshot(true, ctx)
                        .is_none_or(|(current, client)| {
                            current != snapshot || !Arc::ptr_eq(&client, &expected_client)
                        })
                {
                    return;
                }
                if CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .is_none_or(|session| {
                        session.session_context.grok_owned_identity_epoch != Some(epoch)
                    })
                {
                    // 当前线路的查询已被真实事件撤销；完成时刷新界面，但不恢复旧证明。
                    ctx.notify();
                    return;
                }
                let Some(owned) = view.grok_remote_owned.as_mut() else {
                    return;
                };
                if owned.launch.ticket != ticket
                    || owned.tmux_instance.as_ref() != Some(&instance)
                    || owned.snapshot != snapshot
                    || owned.readonly_query_epoch != Some(epoch)
                {
                    return;
                }
                owned.querying = false;
                owned.readonly_query_epoch = None;
                if let Ok(Reply::OwnedIdentity {
                    ticket: response_ticket,
                    native_session,
                    cwd,
                    manifest_sha256,
                    permission_mode,
                }) = reply
                {
                    if response_ticket == ticket
                        && native_session == native
                        && cwd == owned.launch.cwd
                        && permission_mode == "default"
                        && manifest_sha256.len() == 64
                        && manifest_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        owned.native_session = Some(native);
                        owned.readonly_identity = Some(ReadonlyIdentity {
                            native_session,
                            cwd,
                            manifest_sha256,
                            epoch,
                        });
                        view.bind_cli_agent_hook_input_target(ctx);
                    }
                }
                ctx.notify();
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
                        tmux_instance: None,
                        launch,
                        native_session: Some(native),
                        pending: None,
                        sending: false,
                        sending_generation: None,
                        snapshot,
                        querying: false,
                        readonly_identity: None,
                        readonly_query_epoch: None,
                    });
                    view.bind_cli_agent_hook_input_target(ctx);
                    ctx.notify();
                }
            },
        );
    }
}
