use std::sync::Arc;

use instant::Instant;
use remote_server::auth::RemoteServerAuthContext;
use remote_server::setup::{
    PreinstallCheckResult, PreinstallStatus, RemoteLibc, RemoteOs, RemotePlatform,
    UnsupportedReason,
};
use remote_server::transport::Error;
use settings::Setting;
use warp_core::SessionId;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle};

use super::pty_controller::{EventLoopSender, PtyController};
// Zap Wave 3-1:`ServerApiProvider` 不再被本文件使用 — `auth_client`
// 调用点随 AuthClient 一同物理删。auth context 在构造时基于
// `AuthState` 构建一次并缓存(BYOP 路径下无云端 token 刷新)。
use crate::auth::AuthStateProvider;
use crate::remote_server::auth_context::server_api_auth_context;
use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::remote_server::ssh_transport::SshTransport;
use crate::terminal::model::session::{
    ControlMasterOwnership, SessionInfo, SshSessionTransportDescriptor,
};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::shell::ShellType;
use crate::terminal::warpify::settings::{SshExtensionInstallMode, WarpifySettings};
use crate::{TelemetryEvent, send_telemetry_from_ctx};

/// Per-SSH-init state machine. Encoding the state as an enum makes invalid
/// transitions unrepresentable and ensures the `SessionInfo` stash cannot be
/// accessed after it has been consumed.
///
/// Every active state carries `setup_start` so that the total setup duration
/// can be measured when the flow reaches `SessionConnected`.
enum SshInitState {
    Idle,
    /// Stash held, `check_binary` in flight.
    AwaitingCheck {
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
    },
    /// Stash held, choice block showing.
    AwaitingUserChoice {
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
    },
    /// Stash held, `install_binary` in flight.
    /// `for_update` is `true` when reinstalling over an existing install
    /// (auto-update path) and `false` for a fresh install.
    AwaitingInstall {
        session_id: SessionId,
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
        #[allow(dead_code)]
        for_update: bool,
    },
    /// Stash held, `connect_session` in flight. Bootstrap is flushed only
    /// once `SessionConnected` arrives (or on connection failure).
    AwaitingConnect {
        session_id: SessionId,
        session_info: SessionInfo,
        setup_start: Instant,
    },
}

/// Per-pane orchestrator that defers the bootstrap script write for SSH sessions,
/// checks for the remote-server binary, and presents a two-option choice block when the binary is missing.
///
/// Uses a [`WeakModelHandle`] back to [`PtyController`] to avoid preventing
/// `PtyController` from being deallocated.
pub struct RemoteServerController<T: EventLoopSender> {
    pty_controller: WeakModelHandle<PtyController<T>>,
    model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
    auth_context: Arc<RemoteServerAuthContext>,
    state: SshInitState,
    /// Whether the binary was installed during this setup flow.
    did_install: bool,
    /// Detected remote platform from the binary check phase, used for telemetry.
    remote_platform: Option<RemotePlatform>,
    /// Outcome of the preinstall check from the binary check phase,
    /// used for telemetry on the supported path.
    preinstall_check: Option<PreinstallCheckResult>,
    /// 当前 SSH 初始化流程是否已经把 shell bootstrap 交给 PTY。
    bootstrap_flushed: bool,
}

impl<T: EventLoopSender> Entity for RemoteServerController<T> {
    type Event = ();
}

impl<T: EventLoopSender> RemoteServerController<T> {
    pub fn new(
        pty_controller: WeakModelHandle<PtyController<T>>,
        model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let auth_context = Arc::new(server_api_auth_context(
            AuthStateProvider::as_ref(ctx).get().clone(),
        ));
        ctx.subscribe_to_model(&model_event_dispatcher, |me, _, event, ctx| {
            if let ModelEvent::SshInitShell {
                pending_session_info,
            } = event
            {
                me.on_ssh_init_shell_requested(pending_session_info.as_ref().clone(), ctx);
            }
        });

        let mgr = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr, |me, _, event, ctx| match event {
            RemoteServerManagerEvent::BinaryCheckComplete {
                session_id,
                result,
                remote_platform,
                preinstall_check,
                has_old_binary,
            } => {
                me.remote_platform = remote_platform.clone();
                me.preinstall_check = preinstall_check.clone();
                me.on_binary_check_complete(
                    *session_id,
                    result.clone(),
                    preinstall_check.clone(),
                    *has_old_binary,
                    ctx,
                );
            }
            RemoteServerManagerEvent::BinaryInstallComplete {
                session_id,
                result,
                install_source: _,
            } => {
                me.on_binary_install_complete(*session_id, result.clone(), ctx);
            }
            RemoteServerManagerEvent::SessionConnected { session_id, .. } => {
                me.on_session_connected(*session_id, ctx);
            }
            RemoteServerManagerEvent::SessionConnectionFailed { session_id, .. } => {
                me.on_session_connection_failed(*session_id, ctx);
            }
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionDisconnected { .. }
            | RemoteServerManagerEvent::SessionReconnected { .. }
            | RemoteServerManagerEvent::SessionDeregistered { .. }
            | RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::HostDisconnected { .. }
            | RemoteServerManagerEvent::RemoteAgentContextSnapshot { .. }
            | RemoteServerManagerEvent::NavigatedToDirectory { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
            | RemoteServerManagerEvent::CodebaseIndexMutationFailed { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. }
            | RemoteServerManagerEvent::CommitChainResponse { .. }
            | RemoteServerManagerEvent::GitPushResponse { .. }
            | RemoteServerManagerEvent::CreatePrResponse { .. }
            | RemoteServerManagerEvent::GenerateCommitMessageResponse { .. }
            | RemoteServerManagerEvent::GetCommittedBranchFilesResponse { .. }
            | RemoteServerManagerEvent::GitStatusPushReceived { .. }
            | RemoteServerManagerEvent::GitHubPrInfoPushReceived { .. }
            | RemoteServerManagerEvent::GitHubRepositoryInfoPushReceived { .. } => {}
        });

        Self {
            pty_controller,
            model_event_dispatcher,
            auth_context,
            state: SshInitState::Idle,
            did_install: false,
            remote_platform: None,
            preinstall_check: None,
            bootstrap_flushed: false,
        }
    }

    /// Extracts the `SessionInfo` from the stash and writes the bootstrap
    /// script to the PTY via `PtyController::initialize_shell`.
    fn flush_stashed_bootstrap(&mut self, session_info: SessionInfo, ctx: &mut ModelContext<Self>) {
        if self.bootstrap_flushed {
            return;
        }
        self.bootstrap_flushed = true;
        match self.pty_controller.upgrade(ctx) {
            Some(pty) => {
                pty.update(ctx, |pty, ctx| {
                    pty.initialize_shell(&session_info, ctx);
                });
            }
            _ => {
                log::warn!("Remote server PtyController dropped before bootstrap could be flushed");
            }
        }
    }

    /// Idle -> AwaitingCheck
    fn on_ssh_init_shell_requested(&mut self, info: SessionInfo, ctx: &mut ModelContext<Self>) {
        let Some(descriptor) = info.is_ssh_wrapper_session.transport() else {
            return;
        };
        let session_id = info.session_id;
        let remote_os = if info.shell.shell_type() == ShellType::PowerShell {
            RemoteOs::Windows
        } else {
            // Linux 与 macOS 的 setup 命令均为 POSIX；精确平台随后在同一
            // transport 上探测。这里的 Linux 仅作为 shell dialect hint。
            RemoteOs::Linux
        };
        let transport = match descriptor {
            SshSessionTransportDescriptor::ControlMaster {
                socket_path,
                ownership,
            } => SshTransport::new_control_master(
                socket_path.clone(),
                self.auth_context.clone(),
                *ownership == ControlMasterOwnership::WarpManaged,
                remote_os,
            ),
            SshSessionTransportDescriptor::RustBroker {
                endpoint,
                capability,
            } => SshTransport::new_rust_broker(
                endpoint.clone(),
                capability.clone(),
                self.auth_context.clone(),
                remote_os,
            ),
            SshSessionTransportDescriptor::Unavailable => return,
        };
        let early_bootstrap =
            (info.shell.shell_type() == ShellType::PowerShell).then(|| info.clone());
        debug_assert!(matches!(self.state, SshInitState::Idle));
        match std::mem::replace(&mut self.state, SshInitState::Idle) {
            SshInitState::Idle => {}
            SshInitState::AwaitingCheck {
                session_info: old_info,
                ..
            }
            | SshInitState::AwaitingUserChoice {
                session_info: old_info,
                ..
            }
            | SshInitState::AwaitingInstall {
                session_info: old_info,
                ..
            }
            | SshInitState::AwaitingConnect {
                session_info: old_info,
                ..
            } => {
                self.flush_stashed_bootstrap(old_info, ctx);
            }
        }
        self.bootstrap_flushed = false;
        self.did_install = false;
        self.remote_platform = None;
        self.preinstall_check = None;
        self.state = SshInitState::AwaitingCheck {
            session_info: info,
            transport: transport.clone(),
            setup_start: Instant::now(),
        };
        if let Some(session_info) = early_bootstrap {
            self.flush_stashed_bootstrap(session_info, ctx);
        }
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.check_binary(session_id, transport, ctx);
        });
    }

    fn on_binary_check_complete(
        &mut self,
        session_id: SessionId,
        result: Result<bool, Arc<Error>>,
        preinstall_check: Option<PreinstallCheckResult>,
        has_old_binary: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingCheck {
            ref session_info, ..
        } = self.state
        else {
            return;
        };
        if session_info.session_id != session_id {
            return;
        }

        let SshInitState::AwaitingCheck {
            session_info,
            transport,
            setup_start,
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingCheck above");
        };

        // Preinstall gate. Runs **before** any user-visible install
        // affordance: if the script positively classified the host as
        // unsupported, skip the install/prompt entirely and fall back to
        // the legacy ControlMaster-backed SSH flow.
        if let Some(PreinstallCheckResult {
            status: PreinstallStatus::Unsupported { reason },
            libc,
            ..
        }) = preinstall_check.as_ref()
        {
            log::info!(
                "Preinstall check classified {session_id:?} as unsupported \
                 ({reason:?}); falling back to legacy SSH"
            );
            send_unsupported_telemetry(self.remote_platform.as_ref(), reason, Some(libc), ctx);
            // 这里不再回调 `RemoteServerManager::mark_setup_unsupported`:
            // manager 的 `emit_unsupported_preinstall_check` 已经在发出
            // `BinaryCheckComplete` 之前先发了
            // `SetupStateChanged { state: Unsupported { reason } }`,
            // 再调一次只会重复广播同一个状态。
            self.flush_stashed_bootstrap(session_info, ctx);
            return;
        }

        match result {
            Ok(true) => {
                let connection_label = connection_label_for_session_info(&session_info);
                self.state = SshInitState::AwaitingConnect {
                    session_id,
                    session_info,
                    setup_start,
                };
                self.connect_session_for_current_identity(
                    session_id,
                    transport,
                    connection_label,
                    ctx,
                );
            }
            Ok(false) if has_old_binary => {
                // Auto-update: a prior install exists, so skip the modal
                // and reinstall.
                self.did_install = true;
                self.state = SshInitState::AwaitingInstall {
                    session_id,
                    session_info,
                    transport: transport.clone(),
                    setup_start,
                    for_update: true,
                };
                RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                    mgr.install_binary(session_id, transport, true, ctx);
                });
            }
            Ok(false) => {
                let install_mode = *WarpifySettings::as_ref(ctx)
                    .ssh_extension_install_mode
                    .value();
                match install_mode {
                    SshExtensionInstallMode::AlwaysAsk => {
                        self.state = SshInitState::AwaitingUserChoice {
                            session_info,
                            transport,
                            setup_start,
                        };
                        self.model_event_dispatcher.update(ctx, |d, ctx| {
                            d.request_remote_server_block(session_id, ctx);
                        });
                    }
                    SshExtensionInstallMode::AlwaysInstall => {
                        self.did_install = true;
                        self.state = SshInitState::AwaitingInstall {
                            session_id,
                            session_info,
                            transport: transport.clone(),
                            setup_start,
                            for_update: false,
                        };
                        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                            mgr.install_binary(session_id, transport, false, ctx);
                        });
                    }
                    SshExtensionInstallMode::NeverInstall => {
                        self.flush_stashed_bootstrap(session_info, ctx);
                    }
                }
            }
            Err(err) => {
                log::warn!("Remote server binary check failed: session={session_id:?} error={err}");
                self.flush_stashed_bootstrap(session_info, ctx);
            }
        }
    }

    pub fn handle_ssh_remote_server_install(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingUserChoice { .. } = self.state else {
            log::warn!(
                "Remote server install requested in unexpected state: session={session_id:?}"
            );
            return;
        };

        let SshInitState::AwaitingUserChoice {
            session_info,
            transport,
            setup_start,
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingUserChoice above");
        };

        // Reaching this path implies the user explicitly confirmed a
        // fresh install from the modal. Auto-update flows (with an old
        // binary detected) skip the modal entirely and go through
        // `on_binary_check_complete` with `is_update: true`.
        self.did_install = true;
        self.state = SshInitState::AwaitingInstall {
            session_id,
            session_info,
            transport: transport.clone(),
            setup_start,
            for_update: false,
        };
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.install_binary(session_id, transport, false, ctx);
        });
    }

    /// Called when the remote server session is connected. Flushes the
    /// stashed bootstrap (so the session initializes with a live client)
    /// and emits the `RemoteServerSetupDuration` telemetry event.
    fn on_session_connected(&mut self, session_id: SessionId, ctx: &mut ModelContext<Self>) {
        let SshInitState::AwaitingConnect {
            session_id: expected,
            ..
        } = &self.state
        else {
            return;
        };
        if *expected != session_id {
            return;
        }

        let SshInitState::AwaitingConnect {
            session_info,
            setup_start,
            ..
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingConnect above");
        };

        let duration_ms = Instant::now()
            .duration_since(setup_start)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let (remote_os, remote_arch) = self
            .remote_platform
            .as_ref()
            .map(|p| {
                (
                    Some(p.os.as_str().to_owned()),
                    Some(p.arch.as_str().to_owned()),
                )
            })
            .unwrap_or((None, None));
        let remote_libc = self
            .preinstall_check
            .as_ref()
            .map(|check| describe_libc(&check.libc));
        send_telemetry_from_ctx!(
            TelemetryEvent::RemoteServerSetupDuration {
                duration_ms,
                installed_binary: self.did_install,
                remote_os,
                remote_arch,
                remote_libc,
            },
            ctx
        );

        // 会话随后初始化时 `client_for_session` 会返回 `Some`，从而选择
        // `RemoteServerCommandExecutor`。
        self.flush_stashed_bootstrap(session_info, ctx);
    }

    /// Called when the remote server connection failed. Flushes the stashed
    /// bootstrap so the SSH session is not permanently blocked.
    fn on_session_connection_failed(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingConnect {
            session_id: expected,
            ..
        } = &self.state
        else {
            return;
        };
        if *expected != session_id {
            return;
        }

        let SshInitState::AwaitingConnect { session_info, .. } =
            std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingConnect above");
        };
        log::warn!("Remote server connection failed: session={session_id:?}");
        self.flush_stashed_bootstrap(session_info, ctx);
    }

    pub fn handle_ssh_remote_server_skip(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingUserChoice { session_info, .. } =
            std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            log::warn!("Remote server skip requested in unexpected state: session={session_id:?}");
            return;
        };
        self.flush_stashed_bootstrap(session_info, ctx);
    }

    fn on_binary_install_complete(
        &mut self,
        session_id: SessionId,
        result: Result<(), Arc<Error>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let expected = match &self.state {
            SshInitState::AwaitingInstall { session_id, .. } => *session_id,
            _ => return,
        };
        if expected != session_id {
            return;
        }

        let (session_info, transport, setup_start) =
            match std::mem::replace(&mut self.state, SshInitState::Idle) {
                SshInitState::AwaitingInstall {
                    session_info,
                    transport,
                    setup_start,
                    ..
                } => (session_info, transport, setup_start),
                _ => unreachable!("just matched AwaitingInstall above"),
            };
        match result {
            Ok(()) => {
                let connection_label = connection_label_for_session_info(&session_info);
                self.state = SshInitState::AwaitingConnect {
                    session_id,
                    session_info,
                    setup_start,
                };
                self.connect_session_for_current_identity(
                    session_id,
                    transport,
                    connection_label,
                    ctx,
                );
            }
            Err(err) => {
                log::warn!(
                    "Remote server binary install failed: session={session_id:?} error={err}"
                );
                self.flush_stashed_bootstrap(session_info, ctx);
            }
        }
    }

    fn connect_session_for_current_identity(
        &mut self,
        session_id: SessionId,
        transport: SshTransport,
        connection_label: String,
        ctx: &mut ModelContext<Self>,
    ) {
        // Zap:auth context 为构造时缓存的本地 BYOP context(无 auth_client、
        // 无 crash-reporting 上报),不随连接重建。
        let auth_context = self.auth_context.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.connect_session(
                session_id,
                transport,
                auth_context,
                Some(connection_label),
                ctx,
            );
        });
    }
}

fn connection_label_for_session_info(session_info: &SessionInfo) -> String {
    let ssh_host = session_info
        .subshell_info
        .as_ref()
        .and_then(|info| info.ssh_connection_info.as_ref())
        .and_then(|ssh| ssh.host.as_deref());

    connection_label_from_session_hosts(&session_info.user, &session_info.hostname, ssh_host)
}

fn connection_label_from_session_hosts(
    user: &str,
    hostname: &str,
    ssh_host: Option<&str>,
) -> String {
    let host = ssh_host
        .filter(|host| !host.is_empty())
        .map(connection_label_from_ssh_host)
        .or_else(|| (!hostname.is_empty()).then(|| hostname.to_string()));

    connection_label_from_user_and_host(user, host.as_deref())
}

fn connection_label_from_user_and_host(user: &str, host: Option<&str>) -> String {
    match (user.is_empty(), host.filter(|host| !host.is_empty())) {
        (false, Some(host)) => format!("{user}@{host}"),
        (false, None) => user.to_string(),
        (true, Some(host)) => host.to_string(),
        (true, None) => "Remote host".to_string(),
    }
}

fn connection_label_from_ssh_host(host: &str) -> String {
    host.rsplit_once('@')
        .map_or(host, |(_user, host)| host)
        .to_string()
}

/// Describes a [`RemoteLibc`] as a short string for telemetry.
fn describe_libc(libc: &RemoteLibc) -> String {
    match libc {
        RemoteLibc::Glibc(version) => format!("glibc {version}"),
        RemoteLibc::NonGlibc { name } => name.clone(),
        RemoteLibc::Unknown => "unknown".to_string(),
    }
}

fn send_unsupported_telemetry<T: EventLoopSender>(
    remote_platform: Option<&RemotePlatform>,
    unsupported_reason: &UnsupportedReason,
    detected_libc: Option<&RemoteLibc>,
    ctx: &mut ModelContext<RemoteServerController<T>>,
) {
    let (remote_os, remote_arch) = remote_platform
        .map(|p| {
            (
                Some(p.os.as_str().to_owned()),
                Some(p.arch.as_str().to_owned()),
            )
        })
        .unwrap_or((None, None));
    let detected_libc = detected_libc
        .map(describe_libc)
        .unwrap_or_else(|| "unknown".to_string());
    send_telemetry_from_ctx!(
        TelemetryEvent::RemoteServerHostUnsupported {
            remote_os,
            remote_arch,
            unsupported_reason: unsupported_reason.clone(),
            detected_libc,
        },
        ctx
    );
}

#[cfg(test)]
#[path = "remote_server_controller_tests.rs"]
mod tests;
