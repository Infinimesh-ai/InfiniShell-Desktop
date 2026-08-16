//! Unix-specific implementation of the remote server daemon and proxy.
//!
//! - `run_proxy()`: entry point for the `remote-server-proxy` subcommand.
//!   Uses a ControlMaster-like pattern (flock + fork + exec) to daemonize
//!   the server and bridge the SSH stdio channel to its Unix socket.
//!
//! - `run_daemon()`: entry point for the `remote-server-daemon` subcommand.
//!   Binds a Unix domain socket, accepts multiple concurrent proxy connections,
//!   and exits after a grace period with no connections.
//!
//! All platform-specific code is contained here so that the parent `mod.rs`
//! is a thin dispatcher with no Unix assumptions.

pub(super) mod proxy;

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;

use warp_errors::report_error;
use warpui::SingletonEntity;
use warpui::r#async::executor;

use super::server_model::{ConnectionId, ServerModel};
use crate::{TelemetryEvent, send_telemetry_from_app_ctx};

/// Run the `remote-server-daemon` subcommand.
///
/// Delegates to `run_internal` with `LaunchMode::RemoteServerDaemon`.
/// All initialization (feature flags, profiling, logging, resource limits,
/// TLS, `initialize_app`, crash reporting) is handled by `run_internal`.
/// The daemon-specific socket binding and `ServerModel` registration
/// happen in [`launch_daemon`], called from `launch()`.
pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    let result = crate::run_internal(crate::LaunchMode::RemoteServerDaemon {
        identity_key: identity_key.clone(),
    });

    // Clean up socket and PID files after the event loop exits.
    let socket_path = proxy::socket_path(&identity_key);
    let pid_path = proxy::pid_path(&identity_key);
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
    log::info!("Daemon exiting");
    result
}

/// Called from `launch()` inside the headless AppBuilder callback.
/// Binds the Unix domain socket, writes the PID file, spawns the
/// accept loop, and registers the `ServerModel` singleton.
///
/// socket_path: ~/.warp[-channel]/remote-server/{identity_key}/server.sock
///   The Unix domain socket the daemon binds on.  Proxy processes connect
///   to it and bridge their SSH stdio channel through it.
///
/// pid_path:    ~/.warp[-channel]/remote-server/{identity_key}/server.pid
///   Contains the daemon's PID.  Proxy processes read it and use
///   kill(pid, 0) to detect whether the daemon is still alive before
///   deciding whether to start a new one.
pub(crate) fn launch_daemon(identity_key: &str, ctx: &mut warpui::AppContext) {
    let socket_path = proxy::socket_path(identity_key);
    let pid_path = proxy::pid_path(identity_key);

    if let Some(parent) = socket_path.parent()
        && let Err(e) = proxy::ensure_private_daemon_dir(parent)
    {
        report_error!(e.context("Failed to create daemon directory"));
        return;
    }
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    // Bind with std (no async runtime needed yet); converted to
    // async_io::Async inside the closure where the executor is active.
    let listener = match std::os::unix::net::UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            report_error!(anyhow::Error::new(e).context("Daemon: failed to bind socket"));
            return;
        }
    };
    let _ = std::fs::set_permissions(&socket_path, Permissions::from_mode(0o600));
    // async_io::Async::new() requires non-blocking mode.
    listener.set_nonblocking(true).ok();
    log::info!("Daemon bound to {}", socket_path.display());

    // Flush the accumulated IntervalTimer data as telemetry now that the
    // daemon is ready to accept connections. The timer was created in
    // `run_internal` and carries intervals from the full startup path
    // (logging, SQLite, singleton models, etc.).
    //
    // All telemetry dependencies are ready at this point:
    // `AppTelemetryContextProvider` and `AuthStateProvider` are
    // registered during `initialize_app` (before `launch` calls us),
    // and `TelemetryCollector` is already running its periodic flush.
    // The flush sends directly to Rudderstack using a baked-in write
    // key — no user auth token is required.
    let timing_data =
        warp_core::interval_timer::IntervalTimer::handle(ctx).update(ctx, |timer, _| {
            timer.mark_interval_end("DAEMON_SOCKET_BOUND");
            timer.compute_stats()
        });
    send_telemetry_from_app_ctx!(
        TelemetryEvent::RemoteServerDaemonStartup { timing_data },
        ctx
    );

    let _ = std::fs::write(&pid_path, std::process::id().to_string());

    ctx.add_singleton_model(move |ctx| {
        let spawner = ctx.spawner();
        let exec = ctx.background_executor();
        let spawner_loop = spawner.clone();
        let background_executor = exec.clone();

        exec.spawn(async move {
            let listener = match async_io::Async::new(listener) {
                Ok(l) => l,
                Err(e) => {
                    report_error!(anyhow::Error::new(e).context("Daemon: async listener error"));
                    return;
                }
            };
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let conn_id = uuid::Uuid::new_v4();
                        log::info!("Daemon: accepted connection {conn_id}");
                        let spawner = spawner_loop.clone();
                        background_executor
                            .spawn(handle_daemon_connection(
                                conn_id,
                                stream,
                                spawner,
                                background_executor.clone(),
                            ))
                            .detach();
                    }
                    Err(e) => report_error!(anyhow::Error::new(e).context("Daemon: accept error")),
                }
            }
        })
        .detach();

        ServerModel::new(ctx)
    });
}

/// Handles a single Unix socket connection from a proxy process.
///
/// Spawns a dedicated **reader task** that owns the read half of the socket
/// and runs a tight `read_client_message` loop, forwarding each decoded
/// message to `ServerModel` via the spawner.  The reader is never cancelled
/// mid-read, which avoids the framing desynchronisation that would occur if
/// `read_client_message` were polled inside a `select!` branch.
///
/// The calling task becomes the **writer loop**: it drains the per-connection
/// outbound channel (`conn_rx`) and writes each `ServerMessage` to the socket.
/// When the reader exits (EOF / error) it calls `deregister_connection`, which
/// drops `conn_tx` from `ServerModel` and causes `conn_rx` to close, naturally
/// terminating the writer loop.
pub(super) async fn handle_daemon_connection(
    conn_id: ConnectionId,
    stream: async_io::Async<std::os::unix::net::UnixStream>,
    spawner: warpui::ModelSpawner<ServerModel>,
    exec: std::sync::Arc<executor::Background>,
) {
    use futures::AsyncReadExt as _;

    let (read_half, write_half) = stream.split();
    super::daemon::handle_daemon_connection(conn_id, read_half, write_half, spawner, exec).await;
}
