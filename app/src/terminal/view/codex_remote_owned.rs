//! 每个 SSH/tmux pane 显式启动独立 Codex；当前原生进程绑定后才提供图片票据。

use super::tmux_remote_owned::Instance as TmuxInstance;
use super::{BlockId, PendingSpecificCLIAgentLaunch, SessionId, ShellType, TerminalView};
use crate::remote_server::cli_image_codex_owned_client::{self as remote, Journal, Launch};
use crate::remote_server::cli_image_codex_owned_protocol::{Action, Owner, ReadOnlySession, Reply};
use crate::remote_server::client::RemoteServerClient;
use crate::remote_server::manager::RemoteServerManager;
use crate::remote_server::proto::CliImageStagingScope;
use crate::remote_server::tmux_owned_client::Launch as TmuxLaunch;
use crate::settings::AISettings;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::cli_agent_sessions::listener::CLIAgentSessionListener;
use crate::terminal::model::session::command_executor::shell_quote_arg;
use std::sync::Arc;
use std::time::Duration;
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
    tmux_instance: Option<TmuxInstance>,
    launch: Launch,
    owner: Option<Owner>,
    observation: Option<Observation>,
    native_session_id: Option<Uuid>,
    /// 只读原生会话证明绑定原连接代次，不把它记成已收到 rich hook。
    read_only_scope: Option<CliImageStagingScope>,
    snapshot: Snapshot,
    client: Arc<RemoteServerClient>,
}

impl TerminalView {
    pub(super) fn accept_tmux_owned_codex(
        &mut self,
        intent: &TmuxLaunch,
        cwd: &str,
        bytes: &[u8],
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Ok(Reply::Launch { ticket, .. }) = serde_json::from_slice::<Reply>(bytes) else {
            return false;
        };
        if ticket.id != intent.id || ticket.key != intent.key {
            return false;
        }
        let Some(instance) = self.tmux_owned_instance(intent.id, ctx) else {
            return false;
        };
        let Some((snapshot, client)) = self.remote_codex_snapshot(true, ctx) else {
            return false;
        };
        if self.codex_remote_owned.as_ref().is_some_and(|owned| {
            owned.launch.ticket == ticket
                && owned.tmux_instance.as_ref() == Some(&instance)
                && owned.launch.tmux_terminal_session == Some(snapshot.session.as_u64())
                && owned.snapshot == snapshot
                && Arc::ptr_eq(&owned.client, &client)
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
                if !view.tmux_owned_instance_is_current(&instance, ctx) {
                    return;
                }
                if result.is_err() {
                    view.revoke_remote_owned_codex();
                    return;
                }
                if view.codex_remote_owned.as_ref().is_some_and(|owned| {
                    owned.launch.ticket == launch.ticket
                        && owned.tmux_instance.as_ref() == Some(&instance)
                        && owned.launch.tmux_terminal_session == launch.tmux_terminal_session
                        && owned.snapshot == snapshot
                        && Arc::ptr_eq(&owned.client, &client)
                }) {
                    return;
                }
                view.codex_remote_owned = Some(RemoteOwned {
                    tmux_instance: Some(instance),
                    launch,
                    owner: None,
                    observation: None,
                    native_session_id: None,
                    read_only_scope: None,
                    snapshot,
                    client,
                });
                view.observe_remote_owned_codex_bootstrap(0, ctx);
            },
        );
        true
    }

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
        let metadata = block.metadata();
        let cwd = self
            .tmux_owned_cwd(CLIAgent::Codex, session_id, block.id())
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
                    | Reply::Observed { .. }
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
                    tmux_instance: None,
                    launch,
                    owner: None,
                    observation: None,
                    native_session_id: None,
                    read_only_scope: None,
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
                } else {
                    view.observe_remote_owned_codex_bootstrap(0, ctx);
                }
            },
        );
        true
    }

    fn remote_codex_observation(&self, ctx: &AppContext) -> Option<Observation> {
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        if session.agent != CLIAgent::Codex || session.remote_host.is_none() {
            return None;
        }
        let native_session = session.session_context.session_id.clone()?;
        let native_id = Uuid::parse_str(&native_session).ok()?;
        if native_id.is_nil()
            || self
                .codex_remote_owned
                .as_ref()
                .and_then(|owned| owned.native_session_id)
                .is_some_and(|pinned| pinned != native_id)
        {
            return None;
        }
        let listener = session.listener.as_ref()?.id();
        let process = if session.received_rich_notification {
            session.session_context.codex_process_evidence.clone()?
        } else {
            let owned = self.codex_remote_owned.as_ref()?;
            let scope = owned.read_only_scope.as_ref()?;
            let observed = owned.observation.as_ref()?;
            if !owned.client.cli_image_scope_is_current(scope)
                || observed.native_session != native_session
                || observed.listener != listener
            {
                return None;
            }
            observed.process.clone()
        };
        Some(Observation {
            native_session,
            listener,
            process,
        })
    }

    pub(super) fn remote_owned_codex_image_proof(
        &self,
        ctx: &AppContext,
    ) -> Option<(String, String)> {
        let owned = self.codex_remote_owned.as_ref()?;
        if owned.launch.tmux_source.is_some()
            && owned.tmux_instance.as_ref().is_none_or(|instance| {
                !self.tmux_owned_instance_is_current(instance, ctx)
            })
        {
            return None;
        }
        let observation = self.remote_codex_observation(ctx)?;
        let (snapshot, client) = self.remote_codex_snapshot(true, ctx)?;
        let owner = owned.owner.as_ref()?;
        if owned.snapshot != snapshot
            || !Arc::ptr_eq(&owned.client, &client)
            || owned.observation.as_ref() != Some(&observation)
            || owner.server_pid as u32 != observation.process.daemon_pid_candidate
            || owner.ticket_id != owned.launch.ticket.id
            || !owner_session_matches(owner, &observation.native_session)
            || owned
                .read_only_scope
                .as_ref()
                .is_some_and(|scope| !client.cli_image_scope_is_current(scope))
        {
            return None;
        }
        Some((owner.ticket_id.to_string(), owner.manifest_sha256.clone()))
    }

    pub(super) fn remote_owned_codex_process(
        &self,
        ctx: &AppContext,
    ) -> Option<CodexProcessEvidence> {
        self.remote_owned_codex_image_proof(ctx)?;
        Some(self.remote_codex_observation(ctx)?.process)
    }

    /// 第一次图像不依赖先发文字来触发 SessionStart；此循环只查询菜单创建的原票据。
    fn observe_remote_owned_codex_bootstrap(&mut self, attempt: u32, ctx: &mut ViewContext<Self>) {
        let Some(owned) = self.codex_remote_owned.as_ref() else {
            return;
        };
        if owned.observation.is_some() || self.codex_remote_restore_generation.is_some() {
            return;
        }
        let launch = owned.launch.clone();
        let snapshot = owned.snapshot.clone();
        let client = owned.client.clone();
        let generation = Uuid::new_v4();
        let Ok((scope, revision)) = remote::scope(&client, &launch, generation) else {
            return;
        };
        self.codex_remote_restore_generation = Some(generation);
        let query_client = client.clone();
        let query_scope = scope.clone();
        let ticket = launch.ticket.clone();
        ctx.spawn(
            async move {
                // 首次信任和慢启动不设总时限；每轮仅查询原票据，UI 回调决定是否继续。
                let delay = Duration::from_millis(250 * (1_u64 << attempt.min(4)));
                warpui::r#async::Timer::after(delay).await;
                if !query_client.cli_image_scope_is_current(&query_scope) {
                    return None;
                }
                let reply = remote::request(
                    &query_client,
                    query_scope,
                    revision,
                    Action::Observe {
                        ticket: ticket.clone(),
                        expected_session: None,
                    },
                )
                .await
                .ok()?;
                if let Reply::Observed {
                    ticket: returned,
                    owner,
                    session,
                } = reply
                {
                    return (returned == ticket).then_some((owner, session));
                }
                None
            },
            move |view, result, ctx| {
                if view.codex_remote_restore_generation != Some(generation) {
                    return;
                }
                view.codex_remote_restore_generation = None;
                if !client.cli_image_scope_is_current(&scope)
                    || view.codex_remote_owned.as_ref().is_none_or(|owned| {
                        owned.launch.ticket != launch.ticket
                            || owned.snapshot != snapshot
                            || !Arc::ptr_eq(&owned.client, &client)
                    })
                    || view
                        .remote_codex_snapshot(true, ctx)
                        .or_else(|| view.remote_codex_snapshot(false, ctx))
                        .is_none_or(|(now, current)| {
                            now != snapshot || !Arc::ptr_eq(&current, &client)
                        })
                {
                    return;
                }
                if let Some((owner, session)) = result {
                    if view.remote_codex_snapshot(true, ctx).is_none() {
                        view.observe_remote_owned_codex_bootstrap(attempt.saturating_add(1), ctx);
                        return;
                    }
                    view.accept_remote_owned_codex_session(
                        &launch, &snapshot, &client, scope, owner, session, ctx,
                    );
                    // 同一时期若已有真实 hook，它沿原路径升级，不能被只读结果覆盖。
                    view.observe_remote_owned_codex_start(ctx);
                } else {
                    view.observe_remote_owned_codex_bootstrap(attempt.saturating_add(1), ctx);
                }
            },
        );
    }

    fn accept_remote_owned_codex_session(
        &mut self,
        launch: &Launch,
        snapshot: &Snapshot,
        client: &Arc<RemoteServerClient>,
        scope: CliImageStagingScope,
        owner: Owner,
        session: ReadOnlySession,
        ctx: &mut ViewContext<Self>,
    ) {
        if !client.cli_image_scope_is_current(&scope)
            || self
                .remote_codex_snapshot(true, ctx)
                .is_none_or(|(now, current)| &now != snapshot || !Arc::ptr_eq(&current, client))
            || self.codex_remote_owned.as_ref().is_none_or(|owned| {
                owned.launch.ticket != launch.ticket
                    || owned.snapshot != *snapshot
                    || !Arc::ptr_eq(&owned.client, client)
                    || owned.observation.is_some()
                    || owned
                        .native_session_id
                        .is_some_and(|pinned| pinned != session.native_session_id)
            })
            || session.native_session_id.is_nil()
            || owner.ticket_id != launch.ticket.id
            || owner.pinned_native_session_id != Some(session.native_session_id)
            || owner.manifest_sha256.len() != 64
            || owner.server_pid as u32 != session.process.daemon_pid_candidate
            || CLIAgentSessionsModel::as_ref(ctx)
                .session(self.view_id)
                .is_some_and(|current| {
                    current.agent != CLIAgent::Codex
                        || current.received_rich_notification
                        || current
                            .session_context
                            .session_id
                            .as_ref()
                            .is_some_and(|id| id != &session.native_session_id.to_string())
                })
        {
            return;
        }
        let view_id = self.view_id;
        if launch.tmux_source.is_some()
            && !self.set_tmux_owned_native_session(CLIAgent::Codex, session.native_session_id)
        {
            return;
        }
        let events = self.model_events_handle.clone();
        let listener = ctx
            .add_model(|ctx| CLIAgentSessionListener::new(view_id, CLIAgent::Codex, &events, ctx));
        let observation = Observation {
            native_session: session.native_session_id.to_string(),
            listener: listener.id(),
            process: session.process,
        };
        let remote_host = self.active_session_remote_host(ctx);
        let auto_open = *AISettings::as_ref(ctx).auto_open_rich_input_on_cli_agent_start;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.register_listener(
                view_id,
                CLIAgent::Codex,
                Some(snapshot.cwd.clone()),
                None,
                Some(observation.native_session.clone()),
                None,
                remote_host,
                auto_open,
                listener,
                ctx,
            );
        });
        let owned = self.codex_remote_owned.as_mut().unwrap();
        owned.owner = Some(owner);
        owned.native_session_id = Some(session.native_session_id);
        owned.observation = Some(observation);
        owned.read_only_scope = Some(scope);
        self.bind_cli_agent_hook_input_target(ctx);
        self.maybe_show_use_agent_footer_in_blocklist(ctx);
        self.maybe_auto_open_cli_agent_rich_input(ctx);
        self.replay_tmux_session_start(ctx);
        ctx.notify();
    }

    pub(super) fn revoke_remote_owned_codex(&mut self) {
        self.codex_remote_restore_generation = None;
        if let Some(owned) = &mut self.codex_remote_owned {
            owned.launch.tmux_terminal_session = None;
            owned.tmux_instance = None;
            owned.owner = None;
            owned.observation = None;
            owned.read_only_scope = None;
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
            // 初始观察尚未返回时，无 rich 上下文的中间事件不撤销该只读任务。
            // 任务回调仍以原票据、block、client 和 epoch 判定继续或停止。
            if self.codex_remote_restore_generation.is_some()
                && self.codex_remote_owned.as_ref().is_some_and(|owned| {
                    owned.owner.is_none()
                        && owned.observation.is_none()
                        && owned.native_session_id.is_none()
                })
            {
                return;
            }
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
        let expected_session = observation.native_session.clone();
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
                            && owner_session_matches(&owner, &expected_session)
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
                        tmux_instance: None,
                        launch,
                        owner: Some(owner),
                        native_session_id: Uuid::parse_str(&observation.native_session).ok(),
                        observation: Some(observation),
                        read_only_scope: None,
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

fn owner_session_matches(owner: &Owner, native_session: &str) -> bool {
    // 无记录仅保留真实 hook 恢复兼容；零轮入口另行要求精确 Some(SID)。
    owner
        .pinned_native_session_id
        .is_none_or(|pinned| pinned.to_string() == native_session)
}

#[cfg(test)]
#[path = "codex_remote_owned_tests.rs"]
mod tests;
