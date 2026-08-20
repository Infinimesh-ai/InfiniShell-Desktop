use std::io::{Read as _, Write as _};
use std::sync::Arc;
use std::time::Duration;

use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use interprocess::local_socket::LocalSocketStream;
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};
use warpui_core::r#async::executor::Background;

use super::{bind_listener, proxy};

#[test]
fn named_pipe_listener_binds_on_background_runtime() {
    let identity_key = format!("windows-listener-test-{}", uuid::Uuid::new_v4());
    let executor = Arc::new(Background::default());

    let listener = bind_listener(proxy::pipe_name(&identity_key), executor)
        .expect("named pipe listener should bind on the background runtime");
    drop(listener);
}

#[test]
fn named_pipe_multiple_round_trips_across_background_runtime() {
    let identity_key = format!("windows-round-trip-test-{}", uuid::Uuid::new_v4());
    let pipe_name = proxy::pipe_name(&identity_key);
    let executor = Arc::new(Background::default());
    let mut listener = bind_listener(pipe_name.clone(), executor.clone())
        .expect("named pipe listener should bind on the background runtime");
    let (server_result_tx, server_result_rx) = std::sync::mpsc::channel();

    let server_task = executor.spawn(async move {
        let result = async {
            let stream = listener.accept().await?;
            let (reader, writer) = tokio::io::split(stream);
            let mut reader = reader.compat();
            let mut writer = writer.compat_write();
            let mut first_request = [0; 4];
            reader.read_exact(&mut first_request).await?;
            assert_eq!(&first_request, b"ping");
            writer.write_all(b"pong").await?;
            writer.flush().await?;

            let mut second_request = [0; 4];
            reader.read_exact(&mut second_request).await?;
            assert_eq!(&second_request, b"next");
            writer.write_all(b"done").await?;
            writer.flush().await
        }
        .await;
        let _ = server_result_tx.send(result);
    });

    let (client_result_tx, client_result_rx) = std::sync::mpsc::channel();
    let client_thread = std::thread::spawn(move || {
        let result = (|| -> std::io::Result<([u8; 4], [u8; 4])> {
            let mut stream = LocalSocketStream::connect(pipe_name)?;
            stream.write_all(b"ping")?;
            stream.flush()?;
            let mut first_response = [0; 4];
            stream.read_exact(&mut first_response)?;

            stream.write_all(b"next")?;
            stream.flush()?;
            let mut second_response = [0; 4];
            stream.read_exact(&mut second_response)?;

            Ok((first_response, second_response))
        })();
        let _ = client_result_tx.send(result);
    });

    let (first_response, second_response) = client_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("named pipe client should receive a response")
        .expect("named pipe client round trip should succeed");
    assert_eq!(&first_response, b"pong");
    assert_eq!(&second_response, b"done");
    server_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("named pipe server should finish the response")
        .expect("named pipe server round trip should succeed");
    client_thread
        .join()
        .expect("named pipe client thread should finish");
    drop(server_task);
}

#[tokio::test(flavor = "current_thread")]
async fn named_pipe_async_client_can_write_while_read_is_pending() {
    let identity_key = format!("windows-pending-read-test-{}", uuid::Uuid::new_v4());
    let pipe_name = proxy::pipe_name(&identity_key);
    let executor = Arc::new(Background::default());
    let mut listener = bind_listener(pipe_name.clone(), executor.clone())
        .expect("named pipe listener should bind on the background runtime");
    let (server_result_tx, server_result_rx) = std::sync::mpsc::channel();

    let server_task = executor.spawn(async move {
        let result = async {
            let stream = listener.accept().await?;
            let (reader, writer) = tokio::io::split(stream);
            let mut reader = reader.compat();
            let mut writer = writer.compat_write();
            for (request, response) in [(b"ping", b"pong"), (b"next", b"done")] {
                let mut received = [0; 4];
                reader.read_exact(&mut received).await?;
                assert_eq!(&received, request);
                writer.write_all(response).await?;
                writer.flush().await?;
            }
            Ok::<_, std::io::Error>(())
        }
        .await;
        let _ = server_result_tx.send(result);
    });

    let stream = proxy::connect_pipe(&pipe_name)
        .expect("pending-read async client should connect to named pipe");
    let (mut read_stream, mut write_stream) = tokio::io::split(stream);
    tokio::io::AsyncWriteExt::write_all(&mut write_stream, b"ping")
        .await
        .expect("first request should write");
    tokio::io::AsyncWriteExt::flush(&mut write_stream)
        .await
        .expect("first request should flush");
    let mut first_response = [0; 4];
    tokio::io::AsyncReadExt::read_exact(&mut read_stream, &mut first_response)
        .await
        .expect("first response should arrive");
    assert_eq!(&first_response, b"pong");

    let (read_started_tx, read_started_rx) = async_channel::bounded(1);
    let reader_task = tokio::spawn(async move {
        let _ = read_started_tx.send(()).await;
        let mut response = [0; 4];
        tokio::io::AsyncReadExt::read_exact(&mut read_stream, &mut response)
            .await
            .map(|_| response)
    });
    read_started_rx
        .recv()
        .await
        .expect("second response read should start");
    tokio::task::yield_now().await;

    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::io::AsyncWriteExt::write_all(&mut write_stream, b"next"),
    )
    .await
    .expect("second write must not block behind the pending read")
    .expect("second write should succeed");
    tokio::io::AsyncWriteExt::flush(&mut write_stream)
        .await
        .expect("second request should flush");
    let second_response = reader_task
        .await
        .expect("second response reader should finish")
        .expect("second response read should succeed");
    assert_eq!(&second_response, b"done");

    server_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("pending-read server should finish")
        .expect("pending-read server round trips should succeed");
    drop(server_task);
}

#[test]
fn named_pipe_large_write_completes_while_reader_is_pending() {
    let identity_key = format!("windows-large-write-test-{}", uuid::Uuid::new_v4());
    let pipe_name = proxy::pipe_name(&identity_key);
    let executor = Arc::new(Background::default());
    let mut listener = bind_listener(pipe_name.clone(), executor.clone())
        .expect("named pipe listener should bind on the background runtime");
    let (server_result_tx, server_result_rx) = std::sync::mpsc::channel();

    let server_executor = executor.clone();
    let server_task = executor.spawn(async move {
        let result = async {
            let stream = listener.accept().await?;
            let (reader, writer) = tokio::io::split(stream);
            let mut reader = reader.compat();
            let mut writer = writer.compat_write();
            let (request_tx, request_rx) = async_channel::bounded(1);
            server_executor
                .spawn(async move {
                    let mut request = [0; 4];
                    let result = reader.read_exact(&mut request).await.map(|_| request);
                    let _ = request_tx.send(result).await;

                    let mut pending = [0; 1];
                    let _ = reader.read_exact(&mut pending).await;
                })
                .detach();

            let request = request_rx.recv().await.map_err(std::io::Error::other)??;
            assert_eq!(&request, b"ping");
            let response = vec![42_u8; 1024 * 1024];
            writer.write_all(&response).await?;
            writer.flush().await
        }
        .await;
        let _ = server_result_tx.send(result);
    });

    let (client_result_tx, client_result_rx) = std::sync::mpsc::channel();
    let client_thread = std::thread::spawn(move || {
        let result = (|| -> std::io::Result<Vec<u8>> {
            let mut stream = LocalSocketStream::connect(pipe_name)?;
            stream.write_all(b"ping")?;
            stream.flush()?;
            let mut response = vec![0; 1024 * 1024];
            stream.read_exact(&mut response)?;
            Ok(response)
        })();
        let _ = client_result_tx.send(result);
    });

    let response = client_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("named pipe client should receive the large response")
        .expect("named pipe client large read should succeed");
    assert!(response.iter().all(|byte| *byte == 42));
    server_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("named pipe server should finish the large response")
        .expect("named pipe server large write should succeed");
    client_thread
        .join()
        .expect("named pipe client thread should finish");
    drop(server_task);
}
