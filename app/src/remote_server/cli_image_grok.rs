//! SSH 连接内的 Grok 专属 leader 输入；仅一次派发，权限请求始终留在原生 TUI。

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::cli_image_grok_launch::{
    TicketStore, digest, invalid, optional_json, read_json, write_new,
};
use super::cli_image_grok_protocol::{
    Action, Input, Observation, Reply, Request, Scope, Ticket, encode,
};
use crate::ai::agent::ImageContext;
use crate::terminal::cli_agent_sessions::GrokPermissionObservation;
use crate::terminal::cli_agent_sessions::grok_leader_input::{
    GrokLeaderDelivery, GrokLeaderDeliveryStatus, GrokLeaderInput, GrokLeaderInputError,
    GrokLeaderInputEvent, GrokLeaderOutcome, GrokLeaderPrompt,
};

struct Terminal {
    epoch: Uuid,
    revision: u64,
    input_revision: u64,
    used: HashSet<Uuid>,
    lease: Option<(Uuid, Arc<AtomicU8>)>,
}

fn terminal_scope(scope: &Scope) -> super::proto::TerminalBindingScope {
    super::proto::TerminalBindingScope {
        host_id: scope.host.clone(),
        terminal_session_id: scope.terminal_session,
        terminal_epoch: scope.terminal_epoch.to_string(),
    }
}

struct RegisteredLaunch {
    scope: Scope,
    ticket: Ticket,
    cleanup: Sender<(Scope, Ticket)>,
}

impl RegisteredLaunch {
    fn request_cleanup(self) {
        let _ = self.cleanup.send((self.scope, self.ticket));
    }
}

pub(super) struct Connection {
    live: AtomicBool,
    initialized: AtomicBool,
    terminals: Mutex<HashMap<u64, Terminal>>,
    // 启动生命周期独立于输入租约；None 表示断连已封闭登记。
    launches: Mutex<Option<Vec<RegisteredLaunch>>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl Connection {
    pub(super) fn new() -> Self {
        Self {
            live: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            terminals: Mutex::new(HashMap::new()),
            launches: Mutex::new(Some(Vec::new())),
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
        let mut terminals = self.terminals.lock().expect("远端 Grok 终端锁");
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
        let launches = self.launches.lock().expect("远端 Grok 启动锁").take();
        if let Some(launches) = launches {
            for launch in launches {
                launch.request_cleanup();
            }
        }
        for terminal in self.terminals.lock().expect("远端 Grok 终端锁").values() {
            if let Some((_, lease)) = &terminal.lease {
                let _ = lease.compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
    }

    fn register_launch(&self, scope: Scope, ticket: Ticket, cleanup: Sender<(Scope, Ticket)>) {
        let launch = RegisteredLaunch {
            scope,
            ticket,
            cleanup,
        };
        let mut launches = self.launches.lock().expect("远端 Grok 启动锁");
        if let Some(launches) = launches.as_mut() {
            launches.push(launch);
            return;
        }
        drop(launches);
        // Reserve 的磁盘写入可晚于 EOF 完成；此时不能把成功票据遗留在已关闭连接。
        launch.request_cleanup();
    }
    fn current(&self, scope: &Scope) -> bool {
        self.live.load(Ordering::Acquire)
            && self.initialized.load(Ordering::Acquire)
            && self
                .terminals
                .lock()
                .expect("远端 Grok 终端锁")
                .get(&scope.terminal_session)
                .is_some_and(|terminal| terminal.epoch == scope.terminal_epoch)
    }
    fn permit(&self, scope: &Scope, revision: u64) -> io::Result<Arc<AtomicU8>> {
        if !self.current(scope) {
            return Err(invalid());
        }
        let mut terminals = self.terminals.lock().expect("远端 Grok 终端锁");
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
            let mut terminals = self.terminals.lock().expect("远端 Grok 终端锁");
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
    tickets: TicketStore,
    cleanup: Sender<(Scope, Ticket)>,
    tmux_owned: Mutex<Option<Weak<super::tmux_owned::Service>>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    subject: String,
    delivery: GrokLeaderDelivery,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finished {
    claim: Claim,
    native_ack_sha256: String,
}

impl Service {
    #[cfg(test)]
    pub(super) fn without_reaper_for_test(host: String, parent: &Path) -> io::Result<Self> {
        let (cleanup, _) = mpsc::channel();
        Ok(Self {
            host,
            tickets: TicketStore::new(parent)?,
            cleanup,
            tmux_owned: Mutex::new(None),
        })
    }

    pub(super) fn set_tmux_owned(&self, service: Weak<super::tmux_owned::Service>) {
        *self.tmux_owned.lock().expect("tmux 图片服务锁") = Some(service);
    }

    fn tmux_service(&self) -> io::Result<Option<Arc<super::tmux_owned::Service>>> {
        self.tmux_owned
            .lock()
            .map_err(|_| invalid())?
            .as_ref()
            .map(|service| service.upgrade().ok_or_else(invalid))
            .transpose()
    }

    fn ticket_scope(&self, scope: &Scope, ticket: &Ticket) -> io::Result<Scope> {
        let mut original = scope.clone();
        if let Some(service) = self.tmux_service()? {
            if service.owns_ticket(ticket.id)? {
                let stored = service.ticket_terminal(
                    &terminal_scope(scope),
                    ticket.id,
                    ticket.key,
                    super::proto::TerminalBindingOwnedAgent::Grok,
                )?;
                original.host = stored.host().to_owned();
                original.terminal_session = stored.terminal_session();
            }
        }
        Ok(original)
    }

    /// tmux pane 可跨 SSH 连接存活，不登记到会在连接 EOF 时取消的普通启动列表。
    pub(super) fn reserve_tmux(
        &self,
        scope: &Scope,
        ticket: &Ticket,
        cwd: &str,
    ) -> io::Result<Reply> {
        if scope.host != self.host {
            return Err(invalid());
        }
        let reply = self.tickets.reserve(scope, ticket, cwd)?;
        let guard = self.tickets.lock(scope, ticket)?;
        // 仅登记退出后的回收，不写 cancelled；pane 仍可在 SSH 断开后继续运行。
        write_new(&guard.directory.join("cleanup-requested.json"), &true)?;
        Ok(reply)
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
        cancel_launch(&self.tickets, scope, ticket)
    }

    pub(super) fn new(host: String, parent: &Path) -> io::Result<Self> {
        let tickets = TicketStore::new(parent)?;
        let cleanup_tickets = tickets.clone();
        let (cleanup, requested) = mpsc::channel();
        std::thread::Builder::new()
            .name("grok-owned-cleanup".into())
            .spawn(move || cleanup_worker(cleanup_tickets, requested))?;
        Ok(Self {
            host,
            tickets,
            cleanup,
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
                self.execute(connection, request)
            });
        encode(&result.unwrap_or(Reply::Failed {
            code: "unavailable".into(),
        }))
        .unwrap_or_default()
    }

    fn execute(
        self: &Arc<Self>,
        connection: &Arc<Connection>,
        request: Request,
    ) -> io::Result<Reply> {
        let scope = request.scope;
        match request.action {
            Action::Reserve { ticket, cwd } => {
                let reply = self.tickets.reserve(&scope, &ticket, &cwd)?;
                connection.register_launch(scope, ticket, self.cleanup.clone());
                Ok(reply)
            }
            Action::Status { ticket } => self
                .tickets
                .lock(&self.ticket_scope(&scope, &ticket)?, &ticket)?
                .status(&ticket),
            Action::Cancel { ticket } => {
                cancel_launch(&self.tickets, &self.ticket_scope(&scope, &ticket)?, &ticket)
            }
            Action::Revoke { ticket, .. } => {
                self.tickets
                    .lock(&self.ticket_scope(&scope, &ticket)?, &ticket)?;
                Ok(Reply::Revoked { ticket })
            }
            Action::InputStatus { ticket, message_id } => {
                let guard = self
                    .tickets
                    .lock(&self.ticket_scope(&scope, &ticket)?, &ticket)?;
                delivery_status(&guard.directory, &ticket, message_id)
            }
            Action::Submit {
                ticket,
                observation,
                input,
            } => self.submit(
                connection,
                scope,
                request.revision,
                ticket,
                observation,
                input,
            ),
        }
    }

    fn submit(
        self: &Arc<Self>,
        connection: &Arc<Connection>,
        scope: Scope,
        revision: u64,
        ticket: Ticket,
        observation: Observation,
        input: Input,
    ) -> io::Result<Reply> {
        if input.message_id.is_nil()
            || observation.native_session.is_nil()
            || observation.binding_id.is_nil()
            || observation.permission_revision.is_nil()
            || observation.event_id.is_empty()
            || observation.event_id.len() > 512
            || observation.permission_mode != "default"
        {
            return Err(invalid());
        }
        let prompt = prompt(&input)?;
        let subject = input_subject(&input)?;
        let ticket_scope = self.ticket_scope(&scope, &ticket)?;
        let guard = self.tickets.lock(&ticket_scope, &ticket)?;
        let claim_path = guard
            .directory
            .join(format!("{}-claim.json", input.message_id));
        if let Some(previous) = optional_json::<Claim>(&claim_path)? {
            if previous.subject != subject {
                return Err(invalid());
            }
            return delivery_status(&guard.directory, &ticket, input.message_id);
        }
        // 同一内容的未知请求不能改 message UUID 再次发送。
        for entry in std::fs::read_dir(&guard.directory)? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().ends_with("-claim.json") {
                continue;
            }
            let previous: Claim = read_json(&entry.path())?;
            if previous.subject == subject
                && !guard
                    .directory
                    .join(format!("{}-finished.json", previous.delivery.message_id))
                    .exists()
            {
                return Err(invalid());
            }
        }
        let tmux = match self.tmux_service()? {
            Some(service) if service.owns_ticket(ticket.id)? => Some(
                service
                    .image_guard(
                        &terminal_scope(&scope),
                        ticket.id,
                        super::proto::TerminalBindingOwnedAgent::Grok,
                    )?
                    .0,
            ),
            Some(_) | None => None,
        };
        if let Some(tmux) = &tmux {
            tmux.validate_ticket(
                &ticket_scope.host,
                ticket_scope.terminal_session,
                ticket.id,
                super::proto::TerminalBindingOwnedAgent::Grok,
            )?;
        }
        let mut launch = guard.launch_for_input()?;
        if launch.session_id() != observation.native_session
            || launch.working_directory().to_string_lossy() != observation.cwd
        {
            return Err(invalid());
        }
        let observed = GrokPermissionObservation {
            session_id: observation.native_session.to_string(),
            cwd: observation.cwd,
            session_start_event_id: observation.event_id,
            mode: observation.permission_mode,
        };
        let binding = launch
            .bind(
                observation.binding_id,
                observation.permission_revision,
                &observed,
            )
            .map_err(|_| invalid())?;
        let lease = connection.permit(&scope, revision)?;
        let target = binding.into_target();
        let terminal = match &tmux {
            Some(tmux) => {
                let terminal = target.terminal_identity().map_err(|_| invalid())?;
                tmux.validate_process(terminal.0, terminal.1)?;
                Some(terminal)
            }
            None => None,
        };
        let mut sidecar = GrokLeaderInput::connect(target, None).map_err(|_| invalid())?;
        let body_path = guard
            .directory
            .join(format!("{}-input.json", input.message_id));
        write_new(&body_path, &input)?;
        let claim_subject = subject.clone();
        // 原图的持久内容先写，原生 RPC ID/Unknown 后写，最后领取当前连接/输入代际。
        let submitted = sidecar.submit_prompt_once_checked(
            observation.binding_id,
            input.message_id,
            &prompt,
            |delivery| {
                write_new(
                    &claim_path,
                    &Claim {
                        subject: claim_subject,
                        delivery: delivery.clone(),
                    },
                )
            },
            || {
                if let (Some(tmux), Some(terminal)) = (&tmux, terminal) {
                    tmux.validate_process(terminal.0, terminal.1)
                        .map_err(|_| GrokLeaderInputError::StaleBinding)?;
                }
                if !connection.current(&scope) {
                    return Err(GrokLeaderInputError::StaleBinding);
                }
                lease
                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .map(|_| ())
                    .map_err(|_| GrokLeaderInputError::StaleBinding)
            },
        );
        if submitted.is_err() {
            return delivery_status(&guard.directory, &ticket, input.message_id);
        }
        let service = self.clone();
        let directory = guard.directory.clone();
        let message = input.message_id;
        let binding_id = observation.binding_id;
        let reply_ticket = ticket.clone();
        drop(guard);
        // 断连后仅继续收取已领取请求的精确 ACK；本线程没有恢复、重投或答复审批入口。
        std::thread::Builder::new()
            .name("grok-remote-receipt".into())
            .spawn(move || {
                loop {
                    match sidecar.poll(binding_id) {
                        Ok(Some(GrokLeaderInputEvent::Delivery(delivery))) => {
                            if !matches!(delivery.status, GrokLeaderDeliveryStatus::Finished { .. })
                            {
                                continue;
                            }
                            let Some(raw) = sidecar.take_final_response() else {
                                break;
                            };
                            let Ok(bytes) = serde_json::to_vec(&raw) else {
                                break;
                            };
                            let Ok(guard) = service.tickets.lock(&ticket_scope, &ticket) else {
                                break;
                            };
                            let Ok(claim) = read_json::<Claim>(
                                &guard.directory.join(format!("{message}-claim.json")),
                            ) else {
                                break;
                            };
                            if claim.delivery.rpc_id != delivery.rpc_id
                                || claim.delivery.session_id != delivery.session_id
                            {
                                break;
                            }
                            let record = Finished {
                                claim: Claim {
                                    subject: claim.subject,
                                    delivery,
                                },
                                native_ack_sha256: digest(&bytes),
                            };
                            if write_new(
                                &guard.directory.join(format!("{message}-finished.json")),
                                &record,
                            )
                            .is_ok()
                            {
                                // 只删除本次自有、摘要仍相同的图片副本；替换对象和用户原图均保留。
                                let _ = remove_payload(&body_path, &record.claim.subject);
                            }
                            break;
                        }
                        Ok(Some(GrokLeaderInputEvent::NativePermissionPending { .. })) => {
                            let _ = write_new(
                                &directory.join(format!("{message}-permission.json")),
                                &true,
                            );
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Reply::Input {
            ticket: reply_ticket,
            message_id: message,
            subject_sha256: subject,
            state: "unknown".into(),
            native_prompt_id: None,
            native_ack_sha256: None,
        })
    }
}

fn cleanup_worker(tickets: TicketStore, requested: Receiver<(Scope, Ticket)>) {
    loop {
        if let Ok(directories) = tickets.reap_requested() {
            for directory in directories {
                let _ = cleanup_payloads(&directory);
            }
        }
        match requested.recv_timeout(Duration::from_secs(5)) {
            Ok((scope, ticket)) => {
                let _ = cancel_launch(&tickets, &scope, &ticket);
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Service 与登记的连接均释放发送端后，排空已关闭连接的回收请求才退出。
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn cancel_launch(tickets: &TicketStore, scope: &Scope, ticket: &Ticket) -> io::Result<Reply> {
    let guard = tickets.lock(scope, ticket)?;
    let reply = guard.cancel(ticket)?;
    if matches!(&reply, Reply::Launch { phase, .. } if phase == "released") {
        cleanup_payloads(&guard.directory)?;
    }
    Ok(reply)
}

fn prompt(input: &Input) -> io::Result<GrokLeaderPrompt> {
    if input.images.is_empty() {
        return GrokLeaderPrompt::text(&input.text).map_err(|_| invalid());
    }
    let images = input
        .images
        .iter()
        .map(|image| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .map_err(|_| invalid())?;
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || digest(&bytes) != image.sha256 {
                return Err(invalid());
            }
            Ok(ImageContext {
                data: image.data.clone(),
                mime_type: "image/png".into(),
                file_name: "image.png".into(),
                is_figma: false,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    GrokLeaderPrompt::with_png(Some(input.text.clone()), images).map_err(|_| invalid())
}

fn delivery_status(directory: &Path, ticket: &Ticket, message_id: Uuid) -> io::Result<Reply> {
    if message_id.is_nil() {
        return Err(invalid());
    }
    if let Some(record) =
        optional_json::<Finished>(&directory.join(format!("{message_id}-finished.json")))?
    {
        let GrokLeaderDeliveryStatus::Finished {
            native_prompt_id,
            outcome,
        } = record.claim.delivery.status
        else {
            return Err(invalid());
        };
        return Ok(Reply::Input {
            ticket: ticket.clone(),
            message_id,
            subject_sha256: record.claim.subject,
            state: match outcome {
                GrokLeaderOutcome::EndTurn => "finished",
                GrokLeaderOutcome::Cancelled => "cancelled",
            }
            .into(),
            native_prompt_id: Some(native_prompt_id),
            native_ack_sha256: Some(record.native_ack_sha256),
        });
    }
    let record: Claim = read_json(&directory.join(format!("{message_id}-claim.json")))?;
    Ok(Reply::Input {
        ticket: ticket.clone(),
        message_id,
        subject_sha256: record.subject,
        state: if directory
            .join(format!("{message_id}-permission.json"))
            .exists()
        {
            "permission_pending"
        } else {
            "unknown"
        }
        .into(),
        native_prompt_id: None,
        native_ack_sha256: None,
    })
}

fn cleanup_payloads(directory: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().ends_with("-claim.json") {
            continue;
        }
        let claim: Claim = read_json(&entry.path())?;
        let path = directory.join(format!("{}-input.json", claim.delivery.message_id));
        if path.exists() {
            remove_payload(&path, &claim.subject)?;
        }
    }
    Ok(())
}

fn remove_payload(path: &Path, expected: &str) -> io::Result<()> {
    let before = super::cli_image_grok_launch::private_metadata(path, false)?;
    if input_subject(&read_json::<Input>(path)?)? != expected {
        return Err(invalid());
    }
    let after = super::cli_image_grok_launch::private_metadata(path, false)?;
    use std::os::unix::fs::MetadataExt;
    if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err(invalid());
    }
    let quarantine = path
        .parent()
        .ok_or_else(invalid)?
        .join(format!("payload-reap-{}", Uuid::new_v4()));
    std::fs::rename(path, &quarantine)?;
    let renamed = super::cli_image_grok_launch::private_metadata(&quarantine, false)?;
    if (renamed.dev(), renamed.ino()) != (before.dev(), before.ino())
        || input_subject(&read_json::<Input>(&quarantine)?)? != expected
    {
        // 路径被替换时保存隔离对象，不把新的文件当作本次图片删除。
        return Err(invalid());
    }
    std::fs::remove_file(quarantine)
}

pub(crate) fn input_subject(input: &Input) -> io::Result<String> {
    Ok(digest(
        &encode(&(&input.text, &input.images)).map_err(|_| invalid())?,
    ))
}

#[cfg(test)]
#[path = "cli_image_grok_tests.rs"]
mod tests;
