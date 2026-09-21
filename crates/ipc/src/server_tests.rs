use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use futures::io::{AsyncWriteExt as _, BufReader};
use warpui_core::r#async::executor::Background;

use super::*;
use crate::platform::client::connect_client;
use crate::{Client, ClientError, service_caller};

struct EchoService;

impl Service for EchoService {
    type Request = u8;
    type Response = u8;
}

#[derive(Clone)]
struct EchoServiceImpl;

#[async_trait]
impl ServiceImpl for EchoServiceImpl {
    type Service = EchoService;

    async fn handle_request(&self, request: u8) -> u8 {
        request
    }
}

struct BlobService;

impl Service for BlobService {
    type Request = Vec<u8>;
    type Response = Vec<u8>;
}

#[derive(Clone)]
struct BlobServiceImpl;

#[async_trait]
impl ServiceImpl for BlobServiceImpl {
    type Service = BlobService;

    async fn handle_request(&self, request: Vec<u8>) -> Vec<u8> {
        request
    }
}

struct OversizedResponseService;

impl Service for OversizedResponseService {
    type Request = ();
    type Response = Vec<u8>;
}

#[derive(Clone)]
struct OversizedResponseServiceImpl;

#[async_trait]
impl ServiceImpl for OversizedResponseServiceImpl {
    type Service = OversizedResponseService;

    async fn handle_request(&self, request: ()) -> Vec<u8> {
        let () = request;
        vec![0; 300]
    }
}

#[test]
fn server_accepts_a_new_client_after_rejecting_an_oversized_frame() {
    let executor = Arc::new(Background::new(2, |_| {
        "ipc-server-frame-limit-test".to_owned()
    }));
    let (server, address) = ServerBuilder::default()
        .with_max_frame_bytes(256)
        .with_service(EchoServiceImpl)
        .with_service(BlobServiceImpl)
        .with_service(OversizedResponseServiceImpl)
        .build_and_run(executor.clone())
        .unwrap();
    #[cfg(unix)]
    let endpoint = address.0.clone();

    block_on(async {
        let (_, mut malicious_writer) = connect_client(address.clone()).await.unwrap();
        malicious_writer
            .write_all(&257_usize.to_be_bytes())
            .await
            .unwrap();
        malicious_writer.flush().await.unwrap();
        drop(malicious_writer);

        let (malformed_reader, mut malformed_writer) =
            connect_client(address.clone()).await.unwrap();
        send_message(
            &mut malformed_writer,
            Request::new(service_id::<EchoService>(), Vec::new()),
            Some(256),
        )
        .await
        .unwrap();
        let mut malformed_reader = BufReader::new(malformed_reader);
        let response: Response = receive_message(&mut malformed_reader, Some(256))
            .await
            .unwrap();
        assert!(matches!(response, Response::Failure { .. }));
        drop((malformed_reader, malformed_writer));

        let client = Arc::new(
            Client::connect_with_max_frame_bytes(address, executor, 256)
                .await
                .unwrap(),
        );
        let caller = service_caller::<EchoService>(client.clone());
        for value in 0..=u8::MAX {
            assert_eq!(caller.call(value).await.unwrap(), value);
        }
        let blob = service_caller::<BlobService>(client.clone());
        assert!(matches!(
            blob.call(vec![0; 300]).await.unwrap_err(),
            ClientError::InternalProtocol(ProtocolError::FrameTooLarge { .. })
        ));
        let oversized_response = service_caller::<OversizedResponseService>(client);
        assert!(matches!(
            oversized_response.call(()).await.unwrap_err(),
            ClientError::InternalProtocol(ProtocolError::Other(_))
        ));
    });

    drop(server);
    #[cfg(unix)]
    let _ = std::fs::remove_file(endpoint);
}
