use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;

use super::*;

struct Fixture {
    root: tempfile::TempDir,
    service: Arc<Service>,
    connection: Arc<Connection>,
    scope: Scope,
    requested: Receiver<(Scope, Ticket)>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let (cleanup, requested) = mpsc::channel();
        let service = Arc::new(Service {
            host: "grok-cleanup-test-host".into(),
            tickets: TicketStore::new(&root.path().canonicalize().unwrap()).unwrap(),
            cleanup,
        });
        let scope = Scope {
            host: service.host.clone(),
            terminal_session: 17,
            terminal_epoch: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        };
        let connection = connected(&scope);
        Self {
            root,
            service,
            connection,
            scope,
            requested,
        }
    }

    fn reserve_request(&self, scope: &Scope, ticket: &Ticket) -> Request {
        Request {
            version: 1,
            revision: 1,
            scope: scope.clone(),
            action: Action::Reserve {
                ticket: ticket.clone(),
                cwd: self
                    .root
                    .path()
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .into(),
            },
        }
    }

    fn reserve(&self, connection: &Arc<Connection>, scope: &Scope, ticket: &Ticket) {
        let reply = self.service.handle(
            connection,
            &encode(&self.reserve_request(scope, ticket)).unwrap(),
        );
        assert!(matches!(
            serde_json::from_slice::<Reply>(&reply).unwrap(),
            Reply::Reserved { ticket: reserved, .. } if reserved == *ticket
        ));
    }

    fn directory(&self, scope: &Scope, ticket: &Ticket) -> PathBuf {
        self.service
            .tickets
            .lock(scope, ticket)
            .unwrap()
            .directory
            .clone()
    }

    // 显式推进生产回收函数，使用真实私有票据，不等待轮询或启动原生进程。
    fn finish_queued_cleanup(&self, ticket: &Ticket) {
        let (scope, queued) = self.requested.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(queued.id, ticket.id);
        assert_eq!(queued.key, ticket.key);
        cancel_launch(&self.service.tickets, &scope, &queued).unwrap();
    }
}

fn connected(scope: &Scope) -> Arc<Connection> {
    let connection = Arc::new(Connection::new());
    connection.initialize();
    connection.bootstrap(scope.terminal_session, &scope.terminal_epoch.to_string(), 1);
    connection
}

fn ticket() -> Ticket {
    Ticket {
        id: Uuid::new_v4(),
        key: Uuid::new_v4(),
    }
}

#[test]
fn disconnect_requests_cleanup_before_any_hook_or_input() {
    let fixture = Fixture::new();
    let ticket = ticket();
    fixture.reserve(&fixture.connection, &fixture.scope, &ticket);
    let directory = fixture.directory(&fixture.scope, &ticket);
    write_new(&directory.join("entered.json"), &true).unwrap();

    fixture.connection.disconnect();
    fixture.finish_queued_cleanup(&ticket);

    assert!(directory.join("cleanup-requested.json").exists());
    assert!(!directory.join("released.json").exists());
    assert!(!directory.join("cancelled.json").exists());
    fixture.connection.disconnect();
    assert!(matches!(
        fixture.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
}

#[test]
fn reserve_finishing_after_disconnect_cannot_escape_cleanup() {
    let fixture = Fixture::new();
    let ticket = ticket();
    let request = fixture.reserve_request(&fixture.scope, &ticket);
    // 固定 handle 的 current 检查成功、Reserve 尚未完成时发生 EOF 的顺序。
    assert!(fixture.connection.current(&request.scope));
    fixture.connection.disconnect();
    let reply = fixture
        .service
        .execute(&fixture.connection, request)
        .unwrap();
    assert!(matches!(reply, Reply::Reserved { .. }));

    fixture.finish_queued_cleanup(&ticket);

    let directory = fixture.directory(&fixture.scope, &ticket);
    assert!(directory.join("cancelled.json").exists());
    assert!(!directory.join("entered.json").exists());
    assert!(matches!(
        fixture.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
}

#[test]
fn old_disconnect_cannot_retire_reconnected_ticket_or_input() {
    let fixture = Fixture::new();
    let old_ticket = ticket();
    fixture.reserve(&fixture.connection, &fixture.scope, &old_ticket);
    let new_scope = Scope {
        terminal_epoch: Uuid::new_v4(),
        generation: Uuid::new_v4(),
        ..fixture.scope.clone()
    };
    let new_connection = connected(&new_scope);
    let new_ticket = ticket();
    fixture.reserve(&new_connection, &new_scope, &new_ticket);
    let new_lease = new_connection.permit(&new_scope, 2).unwrap();
    let new_directory = fixture.directory(&new_scope, &new_ticket);
    write_new(&new_directory.join("entered.json"), &true).unwrap();

    fixture.connection.disconnect();
    fixture.finish_queued_cleanup(&old_ticket);

    assert!(!new_directory.join("cleanup-requested.json").exists());
    assert!(!new_directory.join("cancelled.json").exists());
    assert!(!new_directory.join("released.json").exists());
    assert!(new_connection.current(&new_scope));
    assert_eq!(new_lease.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fixture.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
}

#[test]
fn failed_duplicate_reserve_does_not_adopt_another_connections_ticket() {
    let fixture = Fixture::new();
    let ticket = ticket();
    fixture.reserve(&fixture.connection, &fixture.scope, &ticket);
    let other = connected(&fixture.scope);
    let reply = fixture.service.handle(
        &other,
        &encode(&fixture.reserve_request(&fixture.scope, &ticket)).unwrap(),
    );
    assert!(matches!(
        serde_json::from_slice::<Reply>(&reply).unwrap(),
        Reply::Failed { .. }
    ));

    other.disconnect();

    assert!(matches!(
        fixture.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
    assert!(
        !fixture
            .directory(&fixture.scope, &ticket)
            .join("cancelled.json")
            .exists()
    );
    fixture.connection.disconnect();
    fixture.finish_queued_cleanup(&ticket);
}

#[test]
fn input_revoke_does_not_request_launch_cleanup() {
    let fixture = Fixture::new();
    let ticket = ticket();
    fixture.reserve(&fixture.connection, &fixture.scope, &ticket);
    let lease = fixture.connection.permit(&fixture.scope, 1).unwrap();
    let request = Request {
        version: 1,
        revision: 2,
        scope: fixture.scope.clone(),
        action: Action::Revoke {
            ticket: ticket.clone(),
            generation: fixture.scope.generation,
        },
    };

    fixture
        .connection
        .revoke_request(&encode(&request).unwrap());

    assert_eq!(lease.load(Ordering::SeqCst), 2);
    assert!(matches!(
        fixture.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
    assert!(
        !fixture
            .directory(&fixture.scope, &ticket)
            .join("cancelled.json")
            .exists()
    );
    fixture.connection.disconnect();
    fixture.finish_queued_cleanup(&ticket);
}

#[test]
fn rejected_scope_does_not_register_cleanup_or_create_ticket() {
    let fixture = Fixture::new();
    let ticket = ticket();
    let wrong_scope = Scope {
        terminal_epoch: Uuid::new_v4(),
        ..fixture.scope.clone()
    };
    let reply = fixture.service.handle(
        &fixture.connection,
        &encode(&fixture.reserve_request(&wrong_scope, &ticket)).unwrap(),
    );
    assert!(matches!(
        serde_json::from_slice::<Reply>(&reply).unwrap(),
        Reply::Failed { .. }
    ));

    fixture.connection.disconnect();

    assert!(matches!(
        fixture.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
    assert!(
        fixture
            .service
            .tickets
            .lock(&fixture.scope, &ticket)
            .is_err()
    );
}

#[test]
fn service_drop_before_connection_still_drains_launch_cleanup() {
    let fixture = Fixture::new();
    let ticket = ticket();
    fixture.reserve(&fixture.connection, &fixture.scope, &ticket);
    let directory = fixture.directory(&fixture.scope, &ticket);
    write_new(&directory.join("entered.json"), &true).unwrap();
    let tickets = fixture.service.tickets.clone();
    let Fixture {
        root,
        service,
        connection,
        scope,
        requested,
    } = fixture;
    let worker = std::thread::spawn(move || cleanup_worker(tickets, requested));

    drop(service);
    drop(connection);
    worker.join().unwrap();

    assert!(directory.join("cleanup-requested.json").exists());
    assert!(!directory.join("released.json").exists());
    drop((scope, root));
}
