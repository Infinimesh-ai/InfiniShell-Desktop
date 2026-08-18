//! Windows remote-server daemon 和 named-pipe 代理。

pub(super) mod proxy;

use async_compat::CompatExt as _;
use interprocess::local_socket::tokio::LocalSocketListener;
use warp_errors::report_error;
use warpui::SingletonEntity;

use super::server_model::ServerModel;
use crate::{TelemetryEvent, send_telemetry_from_app_ctx};

pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    let result = crate::run_internal(crate::LaunchMode::RemoteServerDaemon {
        identity_key: identity_key.clone(),
    });
    log::info!("Windows remote-server daemon exiting");
    result
}

fn bind_listener(pipe_name: String) -> std::io::Result<LocalSocketListener> {
    warpui_core::r#async::block_on(async move { LocalSocketListener::bind(pipe_name) }.compat())
}

pub(crate) fn launch_daemon(identity_key: &str, ctx: &mut warpui::AppContext) {
    let pipe_name = proxy::pipe_name(identity_key);
    let listener = match bind_listener(pipe_name) {
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
                match listener.accept().compat().await {
                    Ok(stream) => {
                        let conn_id = uuid::Uuid::new_v4();
                        log::info!("Windows daemon accepted connection {conn_id}");
                        let (read_half, write_half) = stream.into_split();
                        let spawner = spawner_loop.clone();
                        background_executor
                            .spawn(super::daemon::handle_daemon_connection(
                                conn_id,
                                read_half,
                                write_half,
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
