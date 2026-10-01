//! 私有 Codex 票据与连接代次；启动和模型输入走各自的一次性领取。

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use uuid::Uuid;

use super::cli_image_codex_owned_launch::{OwnedImageLease, TicketStore, invalid};
use super::cli_image_codex_owned_protocol::{Action, Reply, Request, Scope, Ticket, encode};

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
    tmux_owned: Mutex<Option<Weak<super::tmux_owned::Service>>>,
}

impl Service {
    pub(super) fn set_tmux_owned(&self, service: Weak<super::tmux_owned::Service>) {
        *self.tmux_owned.lock().expect("tmux Codex 查询服务锁") = Some(service);
    }

    fn tmux_ticket_scope(
        &self,
        scope: &Scope,
        ticket: &Ticket,
        observe: bool,
    ) -> io::Result<(Scope, Option<super::tmux_owned::ImageGuard>)> {
        let service = self
            .tmux_owned
            .lock()
            .map_err(|_| invalid())?
            .as_ref()
            .map(|service| service.upgrade().ok_or_else(invalid))
            .transpose()?;
        let Some(service) = service else {
            return Ok((scope.clone(), None));
        };
        if !service.owns_ticket(ticket.id)? {
            return Ok((scope.clone(), None));
        }
        let current = super::proto::TerminalBindingScope {
            host_id: scope.host.clone(),
            terminal_session_id: scope.terminal_session,
            terminal_epoch: scope.terminal_epoch.to_string(),
        };
        let (guard, original) = if observe {
            let (guard, original) = service.image_guard(
                &current,
                ticket.id,
                super::proto::TerminalBindingOwnedAgent::Codex,
            )?;
            guard.validate_ticket(
                original.host(),
                original.terminal_session(),
                ticket.id,
                super::proto::TerminalBindingOwnedAgent::Codex,
            )?;
            (Some(guard), original)
        } else {
            // 已退出的票据仍可读终态或取消未进入的 wrapper，不能恢复输入与 sidecar。
            (
                None,
                service.ticket_terminal(
                    &current,
                    ticket.id,
                    ticket.key,
                    super::proto::TerminalBindingOwnedAgent::Codex,
                )?,
            )
        };
        let mut original_scope = scope.clone();
        original_scope.host = original.host().to_owned();
        original_scope.terminal_session = original.terminal_session();
        Ok((original_scope, guard))
    }

    /// 仅由已挑战的 tmux daemon 调用；票据仍绑定原 scope，不能改变旧票据归属。
    pub(super) fn reserve_tmux(
        &self,
        scope: &Scope,
        ticket: &Ticket,
        cwd: &str,
    ) -> io::Result<Reply> {
        if scope.host != self.host {
            return Err(invalid());
        }
        self.tickets.reserve(scope, ticket, cwd)
    }

    pub(super) fn status_tmux(
        &self,
        scope: &Scope,
        ticket: &Ticket,
        original: &super::tmux_owned::OriginalScope,
    ) -> io::Result<Reply> {
        if !original.matches(&scope.host, scope.terminal_session) {
            return Err(invalid());
        }
        self.tickets.lock(scope, ticket)?.status(ticket)
    }

    pub(super) fn cancel_tmux(&self, scope: &Scope, ticket: &Ticket) -> io::Result<Reply> {
        if scope.host != self.host {
            return Err(invalid());
        }
        self.tickets.lock(scope, ticket)?.cancel(ticket)
    }

    #[cfg(test)]
    pub(super) fn without_reaper_for_test(host: String, parent: &Path) -> io::Result<Self> {
        Ok(Self {
            host,
            tickets: Arc::new(TicketStore::new(parent)?),
            tmux_owned: Mutex::new(None),
        })
    }

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
        Ok(Self {
            host,
            tickets,
            tmux_owned: Mutex::new(None),
        })
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
                        let (scope, tmux) =
                            self.tmux_ticket_scope(&request.scope, &ticket, false)?;
                        let reply = self.tickets.lock(&scope, &ticket)?.status(&ticket)?;
                        if let Some(tmux) = tmux {
                            tmux.validate()?;
                        }
                        Ok(reply)
                    }
                    Action::Observe {
                        ticket,
                        expected_session,
                    } => {
                        let (scope, tmux) =
                            self.tmux_ticket_scope(&request.scope, &ticket, true)?;
                        let ticket_guard = self.tickets.lock(&scope, &ticket)?;
                        if let Some(tmux) = &tmux {
                            let Reply::Launch {
                                owner: Some(owner), ..
                            } = ticket_guard.status(&ticket)?
                            else {
                                return Err(invalid());
                            };
                            tmux.validate_process(owner.tui_pid, owner.tty_device)?;
                        }
                        let reply = ticket_guard.observe_session(&ticket, expected_session)?;
                        if !connection.current(&request.scope) {
                            return Err(invalid());
                        }
                        if let Some(tmux) = tmux {
                            let Reply::Observed { owner, .. } = &reply else {
                                return Err(invalid());
                            };
                            tmux.validate_process(owner.tui_pid, owner.tty_device)?;
                        }
                        Ok(reply)
                    }
                    Action::Cancel { ticket } => {
                        let (scope, _) = self.tmux_ticket_scope(&request.scope, &ticket, false)?;
                        self.tickets.lock(&scope, &ticket)?.cancel(&ticket)
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

    /// 仅 fresh tmux target 产生的能力可查询旧 daemon host 的原票据；普通入口仍核当前 host。
    pub(super) fn image_owner_tmux(
        &self,
        scope: &Scope,
        ticket_id: Uuid,
        manifest_sha256: &str,
        guard: &super::tmux_owned::ImageGuard,
    ) -> io::Result<OwnedImageLease> {
        guard.validate_ticket(
            &scope.host,
            scope.terminal_session,
            ticket_id,
            super::proto::TerminalBindingOwnedAgent::Codex,
        )?;
        let lease = self
            .tickets
            .image_owner(scope, ticket_id, manifest_sha256)?;
        guard.validate_process(lease.tui_pid(), lease.tty_device())?;
        Ok(lease)
    }
}
