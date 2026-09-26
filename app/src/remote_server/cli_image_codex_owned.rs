//! 私有 Codex 票据与连接代次；启动和模型输入走各自的一次性领取。

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

use super::cli_image_codex_owned_launch::{OwnedImageLease, TicketStore, invalid};
use super::cli_image_codex_owned_protocol::{Action, Reply, Request, Scope, encode};

struct Terminal {
    epoch: Uuid,
    revision: u64,
    input_revision: u64,
    used: HashSet<Uuid>,
    lease: Option<(Uuid, Arc<AtomicU8>)>,
}

pub(super) struct Connection {
    live: AtomicBool,
    initialized: AtomicBool,
    terminals: Mutex<HashMap<u64, Terminal>>,
}

impl Connection {
    pub(super) fn new() -> Self {
        Self {
            live: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            terminals: Mutex::new(HashMap::new()),
        }
    }
    pub(super) fn initialize(&self) {
        self.initialized.store(true, Ordering::Release);
    }
    pub(super) fn bootstrap(&self, session: u64, epoch: &str, revision: u64) {
        let Ok(epoch) = Uuid::parse_str(epoch) else {
            return;
        };
        if epoch.is_nil() || revision == 0 {
            return;
        }
        let mut terminals = self.terminals.lock().expect("远端 Codex 终端锁");
        if terminals
            .get(&session)
            .is_some_and(|current| current.revision >= revision)
        {
            return;
        }
        if let Some(previous) = terminals.insert(
            session,
            Terminal {
                epoch,
                revision,
                input_revision: 0,
                used: HashSet::new(),
                lease: None,
            },
        ) {
            if let Some((_, lease)) = previous.lease {
                let _ = lease.compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
    }
    pub(super) fn disconnect(&self) {
        self.live.store(false, Ordering::Release);
        for terminal in self.terminals.lock().expect("远端 Codex 终端锁").values() {
            if let Some((_, lease)) = &terminal.lease {
                let _ = lease.compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
    }
    fn current(&self, scope: &Scope) -> bool {
        self.live.load(Ordering::Acquire)
            && self.initialized.load(Ordering::Acquire)
            && self
                .terminals
                .lock()
                .expect("远端 Codex 终端锁")
                .get(&scope.terminal_session)
                .is_some_and(|terminal| terminal.epoch == scope.terminal_epoch)
    }
    fn permit(&self, scope: &Scope, revision: u64) -> io::Result<Arc<AtomicU8>> {
        if !self.current(scope) {
            return Err(invalid());
        }
        let mut terminals = self.terminals.lock().expect("远端 Codex 终端锁");
        let terminal = terminals
            .get_mut(&scope.terminal_session)
            .ok_or_else(invalid)?;
        if terminal.epoch != scope.terminal_epoch
            || revision <= terminal.input_revision
            || !terminal.used.insert(scope.generation)
        {
            return Err(invalid());
        }
        terminal.input_revision = revision;
        if let Some((_, previous)) = terminal.lease.take() {
            let _ = previous.compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
        }
        let lease = Arc::new(AtomicU8::new(0));
        terminal.lease = Some((scope.generation, lease.clone()));
        Ok(lease)
    }
    /// 通知先同步撤销，不能排在磁盘/原生连接后面；已领取的请求继续保存自己的 ACK。
    pub(super) fn revoke_request(&self, bytes: &[u8]) {
        if let Ok(Request {
            scope,
            revision,
            action: Action::Revoke { generation, .. },
            ..
        }) = Request::decode(bytes)
        {
            if !self.current(&scope) {
                return;
            }
            let mut terminals = self.terminals.lock().expect("远端 Codex 终端锁");
            if let Some(terminal) = terminals.get_mut(&scope.terminal_session) {
                terminal.used.insert(generation);
                terminal.input_revision = terminal.input_revision.max(revision);
                if let Some((current, lease)) = &terminal.lease {
                    if *current == generation {
                        let _ = lease.compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
                    }
                }
            }
        }
    }
}

pub(super) struct Service {
    host: String,
    tickets: Arc<TicketStore>,
}

impl Service {
    pub(super) fn new(host: String, parent: &Path) -> io::Result<Self> {
        let tickets = Arc::new(TicketStore::new(parent)?);
        let weak = Arc::downgrade(&tickets);
        std::thread::Builder::new()
            .name("codex-owned-reaper".into())
            .spawn(move || {
                loop {
                    let Some(tickets) = weak.upgrade() else {
                        break;
                    };
                    tickets.reap();
                    drop(tickets);
                    std::thread::sleep(Duration::from_secs(1));
                }
            })?;
        Ok(Self { host, tickets })
    }

    pub(super) fn handle(self: &Arc<Self>, connection: &Arc<Connection>, bytes: &[u8]) -> Vec<u8> {
        let result = Request::decode(bytes)
            .map_err(|_| invalid())
            .and_then(|request| {
                if request.scope.host != self.host || !connection.current(&request.scope) {
                    return Err(invalid());
                }
                match request.action {
                    Action::Reserve { ticket, cwd } => {
                        let lease = connection.permit(&request.scope, request.revision)?;
                        lease
                            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                            .map_err(|_| invalid())?;
                        self.tickets.reserve(&request.scope, &ticket, &cwd)
                    }
                    Action::Status { ticket } => {
                        self.tickets.lock(&request.scope, &ticket)?.status(&ticket)
                    }
                    Action::Cancel { ticket } => {
                        self.tickets.lock(&request.scope, &ticket)?.cancel(&ticket)
                    }
                    Action::Revoke { ticket, .. } => {
                        connection.revoke_request(bytes);
                        Ok(Reply::Revoked { ticket })
                    }
                }
            });
        encode(&result.unwrap_or_else(|_| Reply::Failed {
            code: "remote_codex_owned_unavailable".into(),
        }))
        .unwrap_or_default()
    }

    /// scope 仍须由外层当前 SSH 连接校验；磁盘票据另外限制同一 host 与 terminal session。
    pub(super) fn image_owner(
        &self,
        scope: &Scope,
        ticket_id: Uuid,
        manifest_sha256: &str,
    ) -> io::Result<OwnedImageLease> {
        if scope.host != self.host || scope.terminal_epoch.is_nil() || scope.generation.is_nil() {
            return Err(invalid());
        }
        self.tickets.image_owner(scope, ticket_id, manifest_sha256)
    }
}
