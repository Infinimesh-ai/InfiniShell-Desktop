//! 用真实内存协议流控制旧 EOF、握手和退出回调的先后次序。

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use futures::io::AsyncWriteExt;
use tokio::io::DuplexStream;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uuid::Uuid;
use warp_core::SessionId;
use warp_core::channel::ChannelState;
use warpui_core::r#async::{FutureExt, executor};
use warpui_core::{App, ModelHandle};

use super::{
    HostRequestError, RemoteServerExitStatus, RemoteServerManager, RemoteServerManagerEvent,
    RemoteSessionState, SshRouteNodeState,
};
use crate::HostId;
use crate::auth::RemoteServerAuthContext;
use crate::client::RemoteServerClient;
use crate::proto::{
    ClientMessage, InitializeResponse, ListDirectory, ListDirectoryResponse, ListDirectorySuccess,
    RemoteAgentContextSnapshot, ServerMessage, client_message, host_scoped_request,
    list_directory_response, server_message, session_scoped_request,
};
use crate::protocol;
use crate::setup::{PreinstallCheckResult, RemotePlatform};
use crate::transport::{
    Connection, ControlPath, Error, InstallOutcome, RemoteTransport, TransportConnection,
};

fn session() -> SessionId {
    SessionId::from(31)
}

fn child() -> SessionId {
    SessionId::from(32)
}

#[derive(Debug)]
struct Resource {
    dropped: Arc<AtomicBool>,
    waiting: Option<oneshot::Sender<()>>,
    exit: oneshot::Receiver<()>,
}

impl Drop for Resource {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl TransportConnection for Resource {
    fn terminate(&mut self) {}

    fn try_exit_status(&mut self) -> Option<RemoteServerExitStatus> {
        None
    }

    fn wait_for_exit(
        mut self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Option<RemoteServerExitStatus>> + Send>> {
        Box::pin(async move {
            if let Some(waiting) = self.waiting.take() {
                let _ = waiting.send(());
            }
            let _ = (&mut self.exit).await;
            None
        })
    }

    fn stderr_tail(&self) -> Option<String> {
        None
    }
}

#[derive(Debug)]
struct PendingConnection {
    stream: DuplexStream,
    gate: oneshot::Receiver<()>,
    resource: Resource,
}

#[derive(Debug)]
struct MemoryTransport(Mutex<Option<PendingConnection>>);

impl RemoteTransport for MemoryTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, Error>> + Send>> {
        panic!("连接回归不执行安装流程")
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, Error>> + Send>> {
        panic!("连接回归不执行安装流程")
    }

    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send>> {
        panic!("连接回归不执行安装流程")
    }

    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> {
        panic!("连接回归不执行安装流程")
    }

    fn install_binary(&self) -> Pin<Box<dyn Future<Output = InstallOutcome> + Send>> {
        panic!("连接回归不执行安装流程")
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        panic!("夹具版本必须匹配，不能删除远端程序")
    }

    fn is_reconnectable(&self, _: Option<&RemoteServerExitStatus>) -> bool {
        false
    }

    fn connect(
        &self,
        executor: Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Connection>> + Send>> {
        let pending = self.0.lock().unwrap().take().expect("每个夹具只连接一次");
        Box::pin(async move {
            let PendingConnection {
                stream,
                gate,
                resource,
            } = pending;
            gate.await.expect("测试必须显式放行连接");
            let (reader, writer) = tokio::io::split(stream);
            let (client, event_rx, failure_rx, host_response_rx) =
                RemoteServerClient::new(reader.compat(), writer.compat_write(), &executor);
            Ok(Connection {
                client,
                event_rx,
                failure_rx,
                host_response_rx,
                resource: Box::new(resource),
                control_path: ControlPath::None,
            })
        })
    }
}

struct Peer {
    stream: Option<Compat<DuplexStream>>,
    gate: Option<oneshot::Sender<()>>,
    dropped: Arc<AtomicBool>,
    waiting: Option<oneshot::Receiver<()>>,
    exit: Option<oneshot::Sender<()>>,
}

impl Peer {
    fn new() -> (MemoryTransport, Self) {
        let (client, server) = tokio::io::duplex(8192);
        let (gate_tx, gate) = oneshot::channel();
        let (waiting_tx, waiting) = oneshot::channel();
        let (exit_tx, exit) = oneshot::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let transport = MemoryTransport(Mutex::new(Some(PendingConnection {
            stream: client,
            gate,
            resource: Resource {
                dropped: dropped.clone(),
                waiting: Some(waiting_tx),
                exit,
            },
        })));
        (
            transport,
            Self {
                stream: Some(server.compat()),
                gate: Some(gate_tx),
                dropped,
                waiting: Some(waiting),
                exit: Some(exit_tx),
            },
        )
    }

    fn allow_connect(&mut self) {
        self.gate.take().unwrap().send(()).unwrap();
    }

    fn eof(&mut self) {
        // resource 的清理和 reader 观察 EOF 分开控制，重现生产中的迟到回调。
        drop(self.stream.take());
    }

    async fn read_initialize(&mut self) -> String {
        let message = bounded(protocol::read_client_message(self.stream.as_mut().unwrap()))
            .await
            .unwrap();
        assert!(matches!(
            message.message,
            Some(client_message::Message::SessionScoped(envelope))
                if matches!(envelope.message, Some(session_scoped_request::Message::Initialize(_)))
        ));
        message.request_id
    }

    async fn initialize(&mut self, request_id: String, host: &str) {
        self.reply(
            request_id,
            server_message::Message::InitializeResponse(InitializeResponse {
                server_version: ChannelState::app_version().unwrap_or_default().into(),
                host_id: host.into(),
                capabilities: vec![],
            }),
        )
        .await;
    }

    async fn reply(&mut self, request_id: String, message: server_message::Message) {
        let stream = self.stream.as_mut().unwrap();
        bounded(protocol::write_server_message(
            stream,
            &ServerMessage {
                request_id,
                message: Some(message),
            },
        ))
        .await
        .unwrap();
        bounded(stream.flush()).await.unwrap();
    }

    async fn roundtrip(&mut self, client: &RemoteServerClient) {
        let (response, ()) = bounded(async {
            futures::join!(client.list_directory("/fixture".into()), async {
                let ClientMessage { request_id, message } =
                    protocol::read_client_message(self.stream.as_mut().unwrap()).await.unwrap();
                assert!(matches!(message,
                    Some(client_message::Message::HostScoped(envelope))
                        if matches!(envelope.message, Some(host_scoped_request::Message::ListDirectory(_)))
                ));
                self.reply(request_id, server_message::Message::ListDirectoryResponse(
                    ListDirectoryResponse {
                        result: Some(list_directory_response::Result::Success(ListDirectorySuccess {
                            entries: vec![],
                            canonical_path: "/fixture".into(),
                        })),
                    },
                )).await;
            })
        }).await;
        assert!(matches!(response.unwrap().result,
            Some(list_directory_response::Result::Success(success))
                if success.canonical_path == "/fixture"
        ));
    }
}

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    future
        .with_timeout(Duration::from_secs(5))
        .await
        .expect("受控协议回调未完成")
}

fn connect(
    manager: &ModelHandle<RemoteServerManager>,
    app: &mut App,
    transport: MemoryTransport,
) -> (Uuid, oneshot::Receiver<()>) {
    manager.update(app, |manager, ctx| {
        manager.connect_session(
            session(),
            transport,
            Arc::new(RemoteServerAuthContext::new(
                || Box::pin(async { None }),
                || "connection-fixture".into(),
            )),
            None,
            ctx,
        );
        let attempt = manager.connection_attempts[&session()];
        (
            attempt,
            manager.connection_finished.remove(&attempt).unwrap(),
        )
    })
}

fn connected(
    manager: &ModelHandle<RemoteServerManager>,
    app: &mut App,
    attempt: Uuid,
) -> (Arc<RemoteServerClient>, oneshot::Receiver<()>) {
    manager.update(app, |manager, _| {
        assert_eq!(manager.connection_attempts[&session()], attempt);
        let client = manager.client_for_session(session()).unwrap().clone();
        assert!(Arc::ptr_eq(
            &manager
                .parent_connection_handle(session())
                .client()
                .unwrap(),
            &client
        ));
        (client, manager.connection_drained.remove(&attempt).unwrap())
    })
}

async fn finish(
    manager: &ModelHandle<RemoteServerManager>,
    app: &mut App,
    peer: &mut Peer,
    drained: oneshot::Receiver<()>,
) {
    peer.eof();
    bounded(drained).await.unwrap();
    assert!(peer.dropped.load(Ordering::SeqCst));
    manager.update(app, |manager, ctx| {
        assert!(matches!(
            manager.session(session()),
            Some(RemoteSessionState::Disconnected)
        ));
        assert!(
            manager
                .parent_connection_handle(session())
                .client()
                .is_none()
        );
        manager.deregister_session(session(), ctx);
        assert!(!manager.connection_attempts.contains_key(&session()));
    });
}

#[test]
fn late_old_eof_and_push_do_not_revoke_replacement_connection() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let events = Rc::new(RefCell::new(Vec::new()));
        app.update(|ctx| {
            let events = events.clone();
            ctx.subscribe_to_model(&manager, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            });
        });
        manager.update(&mut app, |manager, _| {
            manager
                .register_ssh_route(session(), None, 1, "parent".into(), None)
                .unwrap();
            manager
                .register_ssh_route(child(), Some(session()), 2, "child".into(), None)
                .unwrap();
        });
        let (transport, mut old) = Peer::new();
        let (old_attempt, done) = connect(&manager, &mut app, transport);
        old.allow_connect();
        let request = old.read_initialize().await;
        old.initialize(request, "old-host").await;
        bounded(done).await.unwrap();
        let (old_client, old_drained) = connected(&manager, &mut app, old_attempt);
        events.borrow_mut().clear();

        let (transport, mut new) = Peer::new();
        let (new_attempt, done) = connect(&manager, &mut app, transport);
        assert_ne!(old_attempt, new_attempt);
        assert!(old.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, _| {
            assert!(
                manager
                    .parent_connection_handle(session())
                    .client()
                    .is_none()
            );
            assert!(
                manager
                    .client_for_host(&HostId::new("old-host".into()))
                    .is_none()
            );
            assert!(manager.ssh_route(session()).unwrap().host_id.is_none());
            assert_eq!(
                manager.ssh_route(child()).unwrap().state,
                SshRouteNodeState::BlockedByParent
            );
        });
        let observed = events.borrow();
        let disconnected = observed
            .iter()
            .position(|event| matches!(event, RemoteServerManagerEvent::SessionDisconnected { .. }))
            .unwrap();
        let setup = observed
            .iter()
            .position(|event| matches!(event, RemoteServerManagerEvent::SetupStateChanged { .. }))
            .unwrap();
        let connecting = observed
            .iter()
            .position(|event| matches!(event, RemoteServerManagerEvent::SessionConnecting { .. }))
            .unwrap();
        assert!(disconnected < setup && disconnected < connecting);
        drop(observed);
        new.allow_connect();
        let request = new.read_initialize().await;
        new.initialize(request, "new-host").await;
        bounded(done).await.unwrap();
        let (new_client, new_drained) = connected(&manager, &mut app, new_attempt);

        old.reply(
            String::new(),
            server_message::Message::RemoteAgentContextSnapshot(RemoteAgentContextSnapshot {
                revision: 99,
                home_dir: String::new(),
                skills: vec![],
                global_rules: vec![],
            }),
        )
        .await;
        old.eof();
        bounded(old_drained).await.unwrap();
        assert!(old_client.is_disconnected());
        assert!(!new.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, _| {
            assert_eq!(manager.connection_attempts[&session()], new_attempt);
            assert!(Arc::ptr_eq(
                manager.client_for_session(session()).unwrap(),
                &new_client
            ));
            assert!(Arc::ptr_eq(
                &manager
                    .parent_connection_handle(session())
                    .client()
                    .unwrap(),
                &new_client
            ));
            assert!(
                manager
                    .client_for_host(&HostId::new("old-host".into()))
                    .is_none()
            );
            assert!(
                !manager
                    .remote_agent_context_snapshot_revisions
                    .contains_key(&HostId::new("new-host".into()))
            );
        });
        assert!(!events.borrow().iter().any(|event| matches!(event,
            RemoteServerManagerEvent::RemoteAgentContextSnapshot { snapshot, .. } if snapshot.revision == 99)));
        new.roundtrip(&new_client).await;
        finish(&manager, &mut app, &mut new, new_drained).await;
    });
}

#[test]
fn late_old_initialize_reply_does_not_replace_new_connected_client() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let (transport, mut old) = Peer::new();
        let (old_attempt, old_done) = connect(&manager, &mut app, transport);
        old.allow_connect();
        let old_request = old.read_initialize().await;
        let (transport, mut new) = Peer::new();
        let (new_attempt, done) = connect(&manager, &mut app, transport);
        new.allow_connect();
        let request = new.read_initialize().await;
        new.initialize(request, "new-host").await;
        bounded(done).await.unwrap();
        let (new_client, new_drained) = connected(&manager, &mut app, new_attempt);
        old.initialize(old_request, "old-host").await;
        bounded(old_done).await.unwrap();
        assert!(old.dropped.load(Ordering::SeqCst));
        assert!(!new.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, _| {
            assert!(!manager.connection_drained.contains_key(&old_attempt));
            assert_eq!(manager.connection_attempts[&session()], new_attempt);
            assert!(Arc::ptr_eq(
                manager.client_for_session(session()).unwrap(),
                &new_client
            ));
        });
        old.eof();
        new.roundtrip(&new_client).await;
        finish(&manager, &mut app, &mut new, new_drained).await;
    });
}

#[test]
fn late_old_connect_result_drops_only_its_own_resource() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let (transport, mut old) = Peer::new();
        let (old_attempt, old_done) = connect(&manager, &mut app, transport);
        let (transport, mut new) = Peer::new();
        let (new_attempt, done) = connect(&manager, &mut app, transport);
        new.allow_connect();
        let request = new.read_initialize().await;
        new.initialize(request, "new-host").await;
        bounded(done).await.unwrap();
        let (new_client, new_drained) = connected(&manager, &mut app, new_attempt);
        old.allow_connect();
        bounded(old_done).await.unwrap();
        assert!(old.dropped.load(Ordering::SeqCst));
        assert!(!new.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, _| {
            assert!(!manager.connection_drained.contains_key(&old_attempt));
            assert_eq!(manager.connection_attempts[&session()], new_attempt);
            assert!(Arc::ptr_eq(
                manager.client_for_session(session()).unwrap(),
                &new_client
            ));
        });
        old.eof();
        new.roundtrip(&new_client).await;
        finish(&manager, &mut app, &mut new, new_drained).await;
    });
}

#[test]
fn late_old_handshake_failure_keeps_new_initializing_client() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let (transport, mut old) = Peer::new();
        let (_, old_done) = connect(&manager, &mut app, transport);
        old.allow_connect();
        let _ = old.read_initialize().await;
        let (transport, mut new) = Peer::new();
        let (new_attempt, done) = connect(&manager, &mut app, transport);
        new.allow_connect();
        let request = new.read_initialize().await;
        let initializing = manager.update(&mut app, |manager, _| {
            let Some(RemoteSessionState::Initializing { client, .. }) = manager.session(session())
            else {
                panic!("新握手尚未完成")
            };
            client.clone()
        });
        old.eof();
        bounded(old_done).await.unwrap();
        assert!(old.dropped.load(Ordering::SeqCst));
        assert!(!new.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, _| {
            assert_eq!(manager.connection_attempts[&session()], new_attempt);
            assert!(matches!(manager.session(session()),
                Some(RemoteSessionState::Initializing { client, .. }) if Arc::ptr_eq(client, &initializing)));
        });
        new.initialize(request, "new-host").await;
        bounded(done).await.unwrap();
        let (new_client, new_drained) = connected(&manager, &mut app, new_attempt);
        new.roundtrip(&new_client).await;
        finish(&manager, &mut app, &mut new, new_drained).await;
    });
}

#[test]
fn late_old_exit_status_does_not_publish_failure_for_replacement() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let events = Rc::new(RefCell::new(Vec::new()));
        app.update(|ctx| {
            let events = events.clone();
            ctx.subscribe_to_model(&manager, move |_, event, _| {
                events.borrow_mut().push(event.clone())
            });
        });
        let (transport, mut old) = Peer::new();
        let (_, old_done) = connect(&manager, &mut app, transport);
        old.allow_connect();
        let _ = old.read_initialize().await;
        old.eof();
        bounded(old.waiting.take().unwrap()).await.unwrap();
        manager.update(&mut app, |manager, _| {
            assert!(matches!(
                manager.session(session()),
                Some(RemoteSessionState::AwaitingExitStatus { .. })
            ));
        });
        let (transport, mut new) = Peer::new();
        let (new_attempt, done) = connect(&manager, &mut app, transport);
        // 释放实际退出等待，再等旧后台任务完成，不能用延时猜测回调已到达。
        old.exit.take().unwrap().send(()).unwrap();
        bounded(old_done).await.unwrap();
        assert!(old.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, _| {
            assert_eq!(manager.connection_attempts[&session()], new_attempt);
            assert!(matches!(
                manager.session(session()),
                Some(RemoteSessionState::Connecting)
            ));
        });
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            RemoteServerManagerEvent::SessionConnectionFailed { .. }
        )));
        new.allow_connect();
        let request = new.read_initialize().await;
        new.initialize(request, "new-host").await;
        bounded(done).await.unwrap();
        let (new_client, new_drained) = connected(&manager, &mut app, new_attempt);
        new.roundtrip(&new_client).await;
        finish(&manager, &mut app, &mut new, new_drained).await;
    });
}

#[test]
fn late_old_host_decode_failure_resolves_exact_pending_request() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let (transport, mut old) = Peer::new();
        let (old_attempt, done) = connect(&manager, &mut app, transport);
        old.allow_connect();
        let request = old.read_initialize().await;
        old.initialize(request, "old-host").await;
        bounded(done).await.unwrap();
        let (old_client, old_drained) = connected(&manager, &mut app, old_attempt);
        let request_id = protocol::RequestId::new().to_string();
        let pending = manager.update(&mut app, |manager, _| {
            manager.send_host_request(
                &HostId::new("old-host".into()),
                ClientMessage::host_scoped(
                    request_id.clone(),
                    host_scoped_request::Message::ListDirectory(ListDirectory {
                        path: "/fixture".into(),
                    }),
                ),
            )
        });
        let issued = bounded(protocol::read_client_message(old.stream.as_mut().unwrap()))
            .await
            .unwrap();
        assert_eq!(issued.request_id, request_id);

        // 同主机仍有另一个活连接，所以旧请求按原合同继续等待，不随会话替换失败。
        let (transport, mut sibling) = Peer::new();
        let (sibling_attempt, done) = manager.update(&mut app, |manager, ctx| {
            manager.connect_session(
                child(),
                transport,
                Arc::new(RemoteServerAuthContext::new(
                    || Box::pin(async { None }),
                    || "connection-fixture".into(),
                )),
                None,
                ctx,
            );
            let attempt = manager.connection_attempts[&child()];
            (
                attempt,
                manager.connection_finished.remove(&attempt).unwrap(),
            )
        });
        sibling.allow_connect();
        let request = sibling.read_initialize().await;
        sibling.initialize(request, "old-host").await;
        bounded(done).await.unwrap();
        let (sibling_client, sibling_drained) = manager.update(&mut app, |manager, _| {
            (
                manager.client_for_session(child()).unwrap().clone(),
                manager.connection_drained.remove(&sibling_attempt).unwrap(),
            )
        });
        let (transport, mut new) = Peer::new();
        let (new_attempt, done) = connect(&manager, &mut app, transport);
        new.allow_connect();
        let request = new.read_initialize().await;
        new.initialize(request, "new-host").await;
        bounded(done).await.unwrap();
        let (new_client, new_drained) = connected(&manager, &mut app, new_attempt);
        manager.update(&mut app, |manager, _| {
            assert!(
                manager
                    .pending_host_requests
                    .contains_key(&protocol::RequestId::from(request_id.clone()))
            );
        });

        // request_id 字段完整，后续 protobuf 字段故意截断；真实 reader 应发精确请求失败。
        let mut payload = vec![0x0a, request_id.len() as u8];
        payload.extend_from_slice(request_id.as_bytes());
        payload.extend_from_slice(&[0x12, 0xff]);
        let stream = old.stream.as_mut().unwrap();
        bounded(stream.write_all(&(payload.len() as u32).to_le_bytes()))
            .await
            .unwrap();
        bounded(stream.write_all(&payload)).await.unwrap();
        bounded(stream.flush()).await.unwrap();
        assert!(matches!(
            bounded(pending).await.unwrap(),
            Err(HostRequestError::UnexpectedResponse)
        ));
        old.eof();
        bounded(old_drained).await.unwrap();
        assert!(old_client.is_disconnected());
        manager.update(&mut app, |manager, _| {
            assert_eq!(manager.connection_attempts[&session()], new_attempt);
            assert!(Arc::ptr_eq(
                manager.client_for_session(session()).unwrap(),
                &new_client
            ));
            assert!(Arc::ptr_eq(
                manager
                    .client_for_host(&HostId::new("old-host".into()))
                    .unwrap(),
                &sibling_client
            ));
        });
        new.roundtrip(&new_client).await;
        sibling.roundtrip(&sibling_client).await;
        finish(&manager, &mut app, &mut new, new_drained).await;
        sibling.eof();
        bounded(sibling_drained).await.unwrap();
        assert!(sibling.dropped.load(Ordering::SeqCst));
        manager.update(&mut app, |manager, ctx| {
            manager.deregister_session(child(), ctx)
        });
    });
}
