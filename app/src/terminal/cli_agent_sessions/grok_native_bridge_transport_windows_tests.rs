use super::*;
use std::cell::Cell;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

fn manifest(instance: Uuid) -> serde_json::Value {
    serde_json::json!({
        "protocol_version": 1,
        "native_pid": 17,
        "instance_id": instance.to_string(),
        "socket": pipe_name(instance),
        "token": "a".repeat(64),
    })
}

fn pair() -> (Runtime, NamedPipeClient, NamedPipeServer) {
    let runtime = runtime().unwrap();
    let name = pipe_name(Uuid::new_v4());
    let server = {
        let _entered = runtime.enter();
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .unwrap()
    };
    let client = connect(&runtime, &name).unwrap();
    runtime.block_on(server.connect()).unwrap();
    (runtime, client, server)
}

#[test]
fn native_bridge_transport_windows_rejects_wrong_pipe_instance_and_manifest_fields() {
    let root = tempfile::tempdir().unwrap();
    let instance = Uuid::new_v4();
    let directory = root.path().join(format!("t-{}", instance.simple()));
    let value = manifest(instance);
    decode_manifest(&serde_json::to_vec(&value).unwrap(), &directory).unwrap();
    let mut remote = value.clone();
    remote["socket"] = r"\\server\pipe\infinishell-grok-terminal-bridge-v1-test".into();
    assert!(decode_manifest(&serde_json::to_vec(&remote).unwrap(), &directory).is_err());
    let mut other = value.clone();
    other["socket"] = serde_json::to_value(pipe_name(Uuid::new_v4())).unwrap();
    assert!(decode_manifest(&serde_json::to_vec(&other).unwrap(), &directory).is_err());
    let mut extra = value;
    extra["ready"] = true.into();
    assert!(decode_manifest(&serde_json::to_vec(&extra).unwrap(), &directory).is_err());
}

#[test]
fn native_bridge_transport_windows_locator_holds_private_manifest_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let root_file = files::create_directory(&root).unwrap();
    let root_identity = files::file_identity(&root_file).unwrap();
    let instance = Uuid::new_v4();
    let directory = root.join(format!("t-{}", instance.simple()));
    drop(files::create_directory(&directory).unwrap());
    let path = directory.join("manifest.json");
    let bytes = serde_json::to_vec(&manifest(instance)).unwrap();
    files::write_new(&path, &bytes).unwrap();
    let locator = Locator::read(&root, root_identity, directory.clone()).unwrap();

    assert!(fs::write(&path, b"replacement").is_err());
    assert!(fs::remove_file(&path).is_err());
    assert!(fs::rename(&directory, root.join("renamed")).is_err());
    locator.validate().unwrap();
    drop(locator);
    fs::remove_file(path).unwrap();
    drop(root_file);
}

#[test]
fn native_bridge_transport_windows_manifest_hardlink_is_not_a_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let root_file = files::create_directory(&root).unwrap();
    let root_identity = files::file_identity(&root_file).unwrap();
    let instance = Uuid::new_v4();
    let directory = root.join(format!("t-{}", instance.simple()));
    drop(files::create_directory(&directory).unwrap());
    let path = directory.join("manifest.json");
    files::write_new(&path, &serde_json::to_vec(&manifest(instance)).unwrap()).unwrap();
    fs::hard_link(&path, directory.join("alias")).unwrap();

    assert!(Locator::read(&root, root_identity, directory).is_err());
    drop(root_file);
}

#[test]
fn native_bridge_transport_windows_refused_admission_writes_zero_bytes() {
    let (runtime, mut client, mut server) = pair();
    let calls = Cell::new(0);
    let result = exchange_frame(
        &runtime,
        &mut client,
        b"request",
        Instant::now() + IO_TIMEOUT,
        || {
            calls.set(calls.get() + 1);
            false
        },
    );
    assert!(result.is_err());
    assert_eq!(calls.get(), 1);
    drop(client);
    let mut bytes = [0u8; 1];
    let read = runtime.block_on(server.read(&mut bytes));
    assert!(
        matches!(read, Ok(0))
            || matches!(read, Err(error) if error.kind() == io::ErrorKind::BrokenPipe)
    );
}

#[test]
fn native_bridge_transport_windows_expired_lease_does_not_claim_or_write() {
    let (runtime, mut client, mut server) = pair();
    let calls = Cell::new(0);
    let result = exchange_frame(&runtime, &mut client, b"request", Instant::now(), || {
        calls.set(calls.get() + 1);
        true
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert_eq!(calls.get(), 0);
    drop(client);
    let mut bytes = [0u8; 1];
    let read = runtime.block_on(server.read(&mut bytes));
    assert!(
        matches!(read, Ok(0))
            || matches!(read, Err(error) if error.kind() == io::ErrorKind::BrokenPipe)
    );
}

#[test]
fn native_bridge_transport_windows_reads_split_response_before_peer_closes() {
    let (runtime, mut client, mut server) = pair();
    let peer = WindowsProcessLease::from_named_pipe_peer(&client).unwrap();
    let responder = runtime.spawn(async move {
        let mut request = [0u8; 11];
        server.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"\0\0\0\x07request");
        server.write_all(&[0, 0]).await.unwrap();
        tokio::task::yield_now().await;
        server.write_all(&[0, 5]).await.unwrap();
        server.write_all(b"reply").await.unwrap();
        let mut read_complete = [0u8; 1];
        server.read_exact(&mut read_complete).await.unwrap();
        assert_eq!(read_complete, [1]);
        server
    });

    let response = exchange_frame(
        &runtime,
        &mut client,
        b"request",
        Instant::now() + IO_TIMEOUT,
        || true,
    )
    .unwrap();
    let server = runtime.block_on(responder).unwrap();
    assert_eq!(response, b"reply");
    drop(server);
    peer.validate().unwrap();
}

#[test]
fn native_bridge_transport_windows_truncated_response_is_unknown() {
    let (runtime, mut client, mut server) = pair();
    let responder = runtime.spawn(async move {
        let mut request = [0u8; 11];
        server.read_exact(&mut request).await.unwrap();
        server.write_all(b"\0\0\0\x05no").await.unwrap();
    });

    assert!(
        exchange_frame(
            &runtime,
            &mut client,
            b"request",
            Instant::now() + IO_TIMEOUT,
            || true
        )
        .is_err()
    );
    runtime.block_on(responder).unwrap();
}

#[test]
fn native_bridge_transport_windows_rejects_nested_runtime_without_panicking() {
    let runtime = runtime().unwrap();
    let _entered = runtime.enter();
    assert_eq!(
        require_blocking_thread().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}
