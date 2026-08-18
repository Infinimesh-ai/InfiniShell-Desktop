use super::{bind_listener, proxy};
use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use interprocess::local_socket::LocalSocketStream;
use std::io::{Read as _, Write as _};
use std::sync::Arc;
use std::time::Duration;
use warpui_core::r#async::executor::Background;

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
    let listener = bind_listener(pipe_name.clone(), executor.clone())
        .expect("named pipe listener should bind on the background runtime");
    let (server_result_tx, server_result_rx) = std::sync::mpsc::channel();

    let server_task = executor.spawn(async move {
        let result = async {
            let stream = listener.accept().await?;
            let (mut reader, mut writer) = stream.into_split();
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
