//! 每个 SSH/tmux pane 显式启动独立 Codex；当前原生进程绑定后才提供图片票据。

use super::{BlockId, PendingSpecificCLIAgentLaunch, SessionId, ShellType, TerminalView};
use crate::remote_server::cli_image_codex_owned_client::{self as remote, Journal, Launch};
use crate::remote_server::cli_image_codex_owned_protocol::{Action, Owner, Reply};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::model::session::command_executor::shell_quote_arg;
use std::sync::Arc;
use uuid::Uuid;
use warp_core::cli_agent_protocol::CodexProcessEvidence;
use warpui::{AppContext, EntityId, SingletonEntity, ViewContext};

#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    session: SessionId,
    block: BlockId,
    cwd: String,
    shell: ShellType,
}
#[derive(Clone, PartialEq, Eq)]
struct Observation {
    native_session: String,
    listener: EntityId,
    process: CodexProcessEvidence,
}

pub(super) struct RemoteOwned {
    launch: Launch,
    owner: Option<Owner>,
    observation: Option<Observation>,
    snapshot: Snapshot,
    client: Arc<RemoteServerClient>,
}

impl TerminalView {
    pub(crate) fn queue_remote_owned_codex(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_remote_owned_codex_launch_available(ctx) {
            self.show_error_toast(crate::t!("cli-agent-codex-remote-launch-unavailable"), ctx);
            return;
        }
        self.pending_specific_cli_agent_launch = None;
        self.set_pending_command(CLIAgent::Codex.command_prefix(), ctx);
        self.pending_specific_cli_agent_launch = Some(PendingSpecificCLIAgentLaunch {
            agent: CLIAgent::Codex,
            executable: None,
            owned_grok: false,
            owned_codex: true,
            preparation: None,
            revision: self
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx),
        });
        self.execute_pending_command((), ctx);
    }
    pub(crate) fn is_remote_owned_codex_launch_available(&self, ctx: &AppContext) -> bool {
        self.remote_codex_snapshot(false, ctx).is_some()
    }
    fn remote_codex_snapshot(
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
            .cli_codex_owned_available()
            .then_some((snapshot, client))
    }

    /// 入口只接显式 owned 启动意图；远端调用绝不采用本机扫描的 Codex 路径。
    pub(super) fn prepare_remote_owned_codex_pending(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some((snapshot, client)) = self.remote_codex_snapshot(false, ctx) else {
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
                            intent.owned_codex
                                && intent.preparation == Some(token)
                                && intent.revision == editor_revision
                        })
                        && view.input.as_ref(ctx).has_pending_command()
                        && view.input.as_ref(ctx).buffer_text(ctx)
                            == CLIAgent::Codex.command_prefix()
                        && view
                            .input
                            .as_ref(ctx)
                            .editor()
                            .as_ref(ctx)
                            .buffer_revision(ctx)
                            == editor_revision
                        && view.remote_codex_snapshot(false, ctx).is_some_and(
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
                            crate::t!("cli-agent-codex-remote-launch-unavailable"),
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
                    view.show_error_toast(crate::t!("cli-agent-codex-owned-launch-claimed"), ctx);
                    return;
                };
                // 回调仅消费当前未编辑启动草稿；argv 逐项原生 shell 转义，不拼接用户命令。
                let command = argv
                    .iter()
                    .map(|arg| shell_quote_arg(arg, snapshot.shell))
                    .collect::<Vec<_>>()
                    .join(" ");
                view.codex_remote_owned = Some(RemoteOwned {
                    launch,
                    owner: None,
                    observation: None,
                    snapshot,
                    client: client.clone(),
                });
                let sent = view.input.update(ctx, |input, ctx| {
                    input.execute_owned_cli_command_once(&command, ctx)
                });
                view.awaiting_pending_command_completion = sent;
                if !sent {
                    view.show_error_toast(
                        crate::t!("cli-agent-codex-remote-launch-unavailable"),
                        ctx,
                    );
                }
            },
        );
        true
    }

    fn remote_codex_observation(&self, ctx: &AppContext) -> Option<Observation> {
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        if session.agent != CLIAgent::Codex
            || session.remote_host.is_none()
            || !session.received_rich_notification
        {
            return None;
        }
        let native_session = session.session_context.session_id.clone()?;
        if Uuid::parse_str(&native_session).ok()?.is_nil() {
            return None;
        }
        Some(Observation {
            native_session,
            listener: session.listener.as_ref()?.id(),
            process: session.session_context.codex_process_evidence.clone()?,
        })
    }

    pub(super) fn remote_owned_codex_image_proof(
        &self,
        ctx: &AppContext,
    ) -> Option<(String, String)> {
        let owned = self.codex_remote_owned.as_ref()?;
        let observation = self.remote_codex_observation(ctx)?;
        let (snapshot, client) = self.remote_codex_snapshot(true, ctx)?;
        let owner = owned.owner.as_ref()?;
        if owned.snapshot != snapshot
            || !Arc::ptr_eq(&owned.client, &client)
            || owned.observation.as_ref() != Some(&observation)
            || owner.server_pid as u32 != observation.process.daemon_pid_candidate
            || owner.ticket_id != owned.launch.ticket.id
        {
            return None;
        }
        Some((owner.ticket_id.to_string(), owner.manifest_sha256.clone()))
    }

    pub(super) fn revoke_remote_owned_codex(&mut self) {
        self.codex_remote_restore_generation = None;
        if let Some(owned) = &mut self.codex_remote_owned {
            owned.owner = None;
            owned.observation = None;
        }
    }

    pub(super) fn retire_remote_owned_codex_for_block(
        &mut self,
        block: &BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .codex_remote_owned
            .as_ref()
            .is_none_or(|owned| &owned.snapshot.block != block)
        {
            return;
        }
        self.revoke_remote_owned_codex();
        let Some(owned) = self.codex_remote_owned.take() else {
            return;
        };
        ctx.spawn(
            async move {
                if let Ok((scope, revision)) =
                    remote::scope(&owned.client, &owned.launch, Uuid::new_v4())
                {
                    let _ = remote::request(
                        &owned.client,
                        scope,
                        revision,
                        Action::Cancel {
                            ticket: owned.launch.ticket,
                        },
                    )
                    .await;
                }
            },
            |_, (), _| {},
        );
    }

    /// 首次通知、后续 listener 更新和 SSH 重连共用查询；查询失败不能变成启动重放。
    pub(super) fn observe_remote_owned_codex_start(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(observation) = self.remote_codex_observation(ctx) else {
            self.revoke_remote_owned_codex();
            return;
        };
        let Some((snapshot, client)) = self.remote_codex_snapshot(true, ctx) else {
            self.revoke_remote_owned_codex();
            return;
        };
        if self.remote_owned_codex_image_proof(ctx).is_some()
            || self.codex_remote_restore_generation.is_some()
        {
            return;
        }
        let Some(host) = client.cli_image_reference_host() else {
            return;
        };
        let generation = Uuid::new_v4();
        self.codex_remote_restore_generation = Some(generation);
        let query_snapshot = snapshot.clone();
        let expected_client = client.clone();
        let expected_process = observation.process.daemon_pid_candidate;
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
                        phase,
                        owner: Some(owner),
                    } = reply
                    {
                        if ticket == launch.ticket
                            && owner.ticket_id == ticket.id
                            && phase == "active"
                            && owner.server_pid as u32 == expected_process
                            && owner.manifest_sha256.len() == 64
                        {
                            if matched.is_some() {
                                return None;
                            }
                            matched = Some((launch, owner));
                        }
                    }
                }
                matched
            },
            move |view, result, ctx| {
                if view.codex_remote_restore_generation != Some(generation) {
                    return;
                }
                view.codex_remote_restore_generation = None;
                if view.remote_codex_observation(ctx).as_ref() != Some(&observation)
                    || view
                        .remote_codex_snapshot(true, ctx)
                        .is_none_or(|(now, client)| {
                            now != snapshot || !Arc::ptr_eq(&client, &expected_client)
                        })
                {
                    return;
                }
                if let Some((launch, owner)) = result {
                    view.codex_remote_owned = Some(RemoteOwned {
                        launch,
                        owner: Some(owner),
                        observation: Some(observation),
                        snapshot,
                        client: expected_client,
                    });
                    view.bind_cli_agent_hook_input_target(ctx);
                    ctx.notify();
                }
            },
        );
    }
}
