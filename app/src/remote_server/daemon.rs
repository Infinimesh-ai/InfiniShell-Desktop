//! remote-server daemon 的跨平台连接处理。

use std::sync::Arc;

use futures::FutureExt as _;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use warp_errors::report_error;
use warpui::r#async::executor;

use super::server_model::{ConnectionId, ServerModel};

/// 处理一个来自平台代理（Unix socket 或 Windows named pipe）的连接。
pub(super) async fn handle_daemon_connection<R, W>(
    conn_id: ConnectionId,
    read_half: R,
    write_half: W,
    spawner: warpui::ModelSpawner<ServerModel>,
    exec: std::sync::Arc<executor::Background>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (conn_tx, conn_rx) = async_channel::bounded::<remote_server::proto::ServerMessage>(256);
    let (tunnel_tx, tunnel_rx) = async_channel::bounded::<remote_server::proto::ServerMessage>(128);
    let tunnel_broker =
        super::tunnel::TunnelBroker::new(conn_tx.clone(), tunnel_tx, Arc::clone(&exec));

    let _ = spawner
        .spawn({
            let conn_tx_reg = conn_tx.clone();
            move |me, ctx| {
                me.register_connection(conn_id, conn_tx_reg, ctx);
            }
        })
        .await;

    let mut writer = BufWriter::new(write_half);
    let spawner_reader = spawner.clone();
    let tunnel_broker_reader = tunnel_broker.clone();
    exec.spawn(async move {
        let mut reader = BufReader::new(read_half);
        loop {
            match remote_server::protocol::read_client_message(&mut reader).await {
                Ok(msg) => {
                    if super::tunnel::TunnelBroker::is_tunnel_message(&msg) {
                        tunnel_broker_reader.handle_message(msg).await;
                        continue;
                    }
                    let result = spawner_reader
                        .spawn(move |me, ctx| {
                            me.handle_message(conn_id, msg, ctx);
                        })
                        .await;
                    if result.is_err() {
                        log::warn!("Daemon: ServerModel dropped, closing conn {conn_id}");
                        break;
                    }
                }
                Err(remote_server::protocol::ProtocolError::UnexpectedEof) => {
                    log::info!("Daemon: proxy {conn_id} disconnected (EOF)");
                    break;
                }
                Err(e) if e.is_read_recoverable() => {
                    log::warn!("Daemon: skipping malformed message from conn {conn_id}: {e}");
                }
                Err(e) => {
                    if is_disconnect_error(&e) {
                        log::warn!(
                            "Daemon: read error from conn {conn_id} (client disconnected): {e}"
                        );
                    } else {
                        report_error!(
                            anyhow::Error::new(e).context("Daemon: fatal read error from conn"),
                            extra: { "conn_id" => %conn_id }
                        );
                    }
                    break;
                }
            }
        }
        let _ = spawner_reader
            .spawn(move |me, ctx| {
                me.deregister_connection(conn_id, ctx);
            })
            .await;
    })
    .detach();
    drop(tunnel_broker);

    loop {
        let control_message = conn_rx.recv();
        let tunnel_message = tunnel_rx.recv();
        futures::pin_mut!(control_message, tunnel_message);
        let msg = futures::select_biased! {
            msg = control_message.fuse() => msg.ok(),
            msg = tunnel_message.fuse() => msg.ok(),
        };
        let Some(msg) = msg else {
            break;
        };
        if let Err(e) = remote_server::protocol::write_server_message(&mut writer, &msg).await {
            if !e.is_write_recoverable() {
                if is_disconnect_error(&e) {
                    log::warn!("Daemon: write error on conn {conn_id} (client disconnected): {e}");
                } else {
                    report_error!(
                        anyhow::Error::new(e).context("Daemon: write error on conn"),
                        extra: { "conn_id" => %conn_id }
                    );
                }
                break;
            }
            log::warn!("Daemon: skipping undeliverable message on conn {conn_id}: {e}");

            if msg.request_id.is_empty() {
                continue;
            }
            let error_msg = remote_server::proto::ServerMessage {
                request_id: msg.request_id.clone(),
                message: Some(remote_server::proto::server_message::Message::Error(
                    remote_server::proto::ErrorResponse {
                        code: remote_server::proto::ErrorCode::Internal.into(),
                        message: format!("Response could not be delivered: {e}"),
                    },
                )),
            };
            if let Err(e2) =
                remote_server::protocol::write_server_message(&mut writer, &error_msg).await
            {
                if !e2.is_write_recoverable() {
                    report_error!(
                        anyhow::Error::new(e2)
                            .context("Daemon: failed to send error response on conn"),
                        extra: { "conn_id" => %conn_id }
                    );
                    break;
                }
                log::warn!("Daemon: failed to send error response on conn {conn_id}: {e2}");
                continue;
            }
        }
        if let Err(e) = writer.flush().await {
            if is_disconnect_io_error(&e) {
                log::warn!("Daemon: flush error on conn {conn_id} (client disconnected): {e}");
            } else {
                report_error!(
                    anyhow::Error::new(e).context("Daemon: flush error on conn"),
                    extra: { "conn_id" => %conn_id }
                );
            }
            break;
        }
    }

    let _ = writer.flush().await;
    let _ = spawner
        .spawn(move |me, ctx| {
            me.deregister_connection(conn_id, ctx);
        })
        .await;
}

fn is_disconnect_io_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
    )
}

fn is_disconnect_error(error: &remote_server::protocol::ProtocolError) -> bool {
    match error {
        remote_server::protocol::ProtocolError::Io(io_error) => is_disconnect_io_error(io_error),
        remote_server::protocol::ProtocolError::Decode(_, _)
        | remote_server::protocol::ProtocolError::MessageTooLarge { .. }
        | remote_server::protocol::ProtocolError::UnexpectedEof => false,
    }
}
