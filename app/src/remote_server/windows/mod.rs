//! Windows remote-server daemon 和 named-pipe 代理。

pub(super) mod proxy;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};
use warp_errors::report_error;
use warpui::SingletonEntity;
use warpui::r#async::executor;

use super::server_model::ServerModel;
use crate::{TelemetryEvent, send_telemetry_from_app_ctx};

pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    let result = crate::run_internal(crate::LaunchMode::RemoteServerDaemon {
        identity_key: identity_key.clone(),
    });
    log::info!("Windows remote-server daemon exiting");
    result
}

struct NamedPipeListener {
    path: String,
    server: NamedPipeServer,
}

impl NamedPipeListener {
    fn bind(pipe_name: String) -> std::io::Result<Self> {
        let path = format!(r"\\.\pipe\{pipe_name}");
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)?;
        Ok(Self { path, server })
    }

    async fn accept(&mut self) -> std::io::Result<NamedPipeServer> {
        self.server.connect().await?;
        let next = ServerOptions::new().create(&self.path)?;
        Ok(std::mem::replace(&mut self.server, next))
    }
}

fn bind_listener(
    pipe_name: String,
    background_executor: std::sync::Arc<executor::Background>,
) -> std::io::Result<NamedPipeListener> {
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    background_executor
        .spawn(async move {
            let _ = result_tx.send(NamedPipeListener::bind(pipe_name));
        })
        .detach();
    result_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error))?
}

pub(crate) fn launch_daemon(identity_key: &str, ctx: &mut warpui::AppContext) {
    let pipe_name = proxy::pipe_name(identity_key);
    let mut listener = match bind_listener(pipe_name, ctx.background_executor().clone()) {
        Ok(listener) => listener,
        Err(error) => {
            report_error!(anyhow::Error::new(error).context("Daemon: failed to bind named pipe"));
            return;
        }
    };
    log::info!("Windows daemon bound to named pipe");

    let timing_data =
        warp_core::interval_timer::IntervalTimer::handle(ctx).update(ctx, |timer, _| {
            timer.mark_interval_end("DAEMON_SOCKET_BOUND");
            timer.compute_stats()
        });
    send_telemetry_from_app_ctx!(
        TelemetryEvent::RemoteServerDaemonStartup { timing_data },
        ctx
    );

    ctx.add_singleton_model(move |ctx| {
        let spawner = ctx.spawner();
        let exec = ctx.background_executor();
        let spawner_loop = spawner.clone();
        let background_executor = exec.clone();

        exec.spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(stream) => {
                        let conn_id = uuid::Uuid::new_v4();
                        log::info!("Windows daemon accepted connection {conn_id}");
                        let (read_half, write_half) = tokio::io::split(stream);
                        let spawner = spawner_loop.clone();
                        background_executor
                            .spawn(super::daemon::handle_daemon_connection(
                                conn_id,
                                read_half.compat(),
                                write_half.compat_write(),
                                spawner,
                                background_executor.clone(),
                            ))
                            .detach();
                    }
                    Err(error) => {
                        report_error!(
                            anyhow::Error::new(error).context("Daemon: named-pipe accept error")
                        );
                        break;
                    }
                }
            }
        })
        .detach();

        ServerModel::new(ctx)
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
