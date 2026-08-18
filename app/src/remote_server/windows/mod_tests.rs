use super::{bind_listener, proxy};
use async_compat::CompatExt as _;
use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use interprocess::local_socket::LocalSocketStream;
use std::io::{Read as _, Write as _};
use std::sync::Arc;
use std::time::Duration;
use warpui_core::r#async::executor::Background;

#[test]
fn named_pipe_listener_binds_through_compat_runtime() {
    let identity_key = format!("windows-listener-test-{}", uuid::Uuid::new_v4());

    let listener = bind_listener(proxy::pipe_name(&identity_key))
        .expect("named pipe listener should bind through the compatibility runtime");
    drop(listener);
}

#[test]
fn named_pipe_round_trip_across_background_runtime() {
    let identity_key = format!("windows-round-trip-test-{}", uuid::Uuid::new_v4());
    let pipe_name = proxy::pipe_name(&identity_key);
    let listener = bind_listener(pipe_name.clone())
        .expect("named pipe listener should bind through the compatibility runtime");
    let executor = Arc::new(Background::default());
    let (server_result_tx, server_result_rx) = std::sync::mpsc::channel();

    let server_task = executor.spawn(async move {
        let result = async {
            let stream = listener.accept().compat().await?;
            let (mut reader, mut writer) = stream.into_split();
            let mut request = [0; 4];
            reader.read_exact(&mut request).await?;
            assert_eq!(&request, b"ping");
            writer.write_all(b"pong").await?;
            writer.flush().await
        }
        .await;
        let _ = server_result_tx.send(result);
    });

    let (client_result_tx, client_result_rx) = std::sync::mpsc::channel();
    let client_thread = std::thread::spawn(move || {
        let result = (|| -> std::io::Result<[u8; 4]> {
            let mut stream = LocalSocketStream::connect(pipe_name)?;
            stream.write_all(b"ping")?;
            stream.flush()?;
            let mut response = [0; 4];
            stream.read_exact(&mut response)?;
            Ok(response)
        })();
        let _ = client_result_tx.send(result);
    });

    let response = client_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("named pipe client should receive a response")
        .expect("named pipe client round trip should succeed");
    assert_eq!(&response, b"pong");
    server_result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("named pipe server should finish the response")
        .expect("named pipe server round trip should succeed");
    client_thread
        .join()
        .expect("named pipe client thread should finish");
    drop(server_task);
}
