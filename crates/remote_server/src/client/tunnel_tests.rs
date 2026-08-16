use std::sync::atomic::{AtomicU64, AtomicUsize};

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{TunnelMultiplexer, reserve_inbound};
use crate::proto::{
    ServerMessage, TunnelChannel, TunnelData, TunnelServerMessage, TunnelWindowUpdate,
    client_message, server_message, tunnel_client_message, tunnel_server_message,
};

fn tunnel_message(stream_id: &str, message: tunnel_server_message::Message) -> ServerMessage {
    ServerMessage {
        request_id: String::new(),
        message: Some(server_message::Message::Tunnel(TunnelServerMessage {
            stream_id: stream_id.to_string(),
            message: Some(message),
        })),
    }
}

#[test]
fn inbound_window_requires_contiguous_offsets_and_available_credit() {
    let credit = AtomicUsize::new(8);
    let offset = AtomicU64::new(0);
    assert!(reserve_inbound(&credit, &offset, 0, 4));
    assert_eq!(credit.load(std::sync::atomic::Ordering::Acquire), 4);
    assert_eq!(offset.load(std::sync::atomic::Ordering::Acquire), 4);
    assert!(!reserve_inbound(&credit, &offset, 3, 1));
    assert!(!reserve_inbound(&credit, &offset, 4, 5));
}

#[tokio::test]
async fn stdin_window_limits_frames_and_refills_after_acknowledgement() {
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let multiplexer = TunnelMultiplexer::new(outbound_tx);
    let mut stream = multiplexer.register("stream-1".to_string());
    multiplexer.activate("stream-1", 4).unwrap();

    assert_eq!(stream.write(b"abcdef").await.unwrap(), 4);
    let first = outbound_rx.next().await.unwrap();
    let Some(client_message::Message::Tunnel(tunnel)) = first.message else {
        panic!("期望隧道消息");
    };
    let Some(tunnel_client_message::Message::Data(data)) = tunnel.message else {
        panic!("期望隧道数据");
    };
    assert_eq!(data.offset, 0);
    assert_eq!(data.data, b"abcd");

    multiplexer
        .handle_server_message(tunnel_message(
            "stream-1",
            tunnel_server_message::Message::WindowUpdate(TunnelWindowUpdate {
                channel: TunnelChannel::Stdin.into(),
                consumed_bytes: 4,
            }),
        ))
        .await;
    assert_eq!(stream.write(b"ef").await.unwrap(), 2);
    let second = outbound_rx.next().await.unwrap();
    let Some(client_message::Message::Tunnel(tunnel)) = second.message else {
        panic!("期望隧道消息");
    };
    let Some(tunnel_client_message::Message::Data(data)) = tunnel.message else {
        panic!("期望隧道数据");
    };
    assert_eq!(data.offset, 4);
    assert_eq!(data.data, b"ef");
}

#[tokio::test]
async fn invalid_stdout_offset_resets_only_the_affected_stream() {
    let (outbound_tx, _outbound_rx) = mpsc::channel(8);
    let multiplexer = TunnelMultiplexer::new(outbound_tx);
    let mut bad_stream = multiplexer.register("bad".to_string());
    let mut good_stream = multiplexer.register("good".to_string());

    multiplexer
        .handle_server_message(tunnel_message(
            "bad",
            tunnel_server_message::Message::Data(TunnelData {
                channel: TunnelChannel::Stdout.into(),
                offset: 1,
                data: b"bad".to_vec(),
            }),
        ))
        .await;
    let error = bad_stream.read(&mut [0; 3]).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);

    multiplexer
        .handle_server_message(tunnel_message(
            "good",
            tunnel_server_message::Message::Data(TunnelData {
                channel: TunnelChannel::Stdout.into(),
                offset: 0,
                data: b"ok".to_vec(),
            }),
        ))
        .await;
    let mut output = [0; 2];
    good_stream.read_exact(&mut output).await.unwrap();
    assert_eq!(&output, b"ok");
}
