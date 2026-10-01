//! 输出挑战仅证明当前 SSH 终端线路；tmux 身份仍由独立内核与原生接口核验。

use super::proto::{
    SessionBootstrapped, TerminalBindingBound, TerminalBindingCancelled,
    TerminalBindingChallengeWritten, TerminalBindingFailed, TerminalBindingRequest,
    TerminalBindingResponse, TerminalBindingScope, terminal_binding_request,
    terminal_binding_response,
};
use super::tmux_native::{OuterTerminal, SplitOutcome, TmuxBinding, TmuxSnapshot};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const CHALLENGE_LIFETIME: Duration = Duration::from_secs(30);

struct Terminal {
    epoch: Uuid,
    revision: u64,
    candidate: Option<(u32, PathBuf)>,
    used: HashSet<Uuid>,
    current: Option<Arc<Attempt>>,
}

enum State {
    Writing,
    Ready(Arc<OuterTerminal>),
    Discovering,
    Bound(Arc<TmuxBinding>, TmuxSnapshot),
    Failed,
}

pub(super) struct Attempt {
    id: Uuid,
    nonce: Uuid,
    binding_id: Uuid,
    created: Instant,
    live: AtomicBool,
    launch: AtomicU8,
    state: Mutex<State>,
}

impl Attempt {
    fn pending(&self) -> bool {
        self.live.load(Ordering::Acquire) && self.created.elapsed() < CHALLENGE_LIFETIME
    }

    fn retire(&self) {
        let _ = self
            .launch
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
        self.live.store(false, Ordering::Release);
    }

    fn claim_launch(&self) -> bool {
        self.live.load(Ordering::Acquire)
            && self
                .launch
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            && self.live.load(Ordering::Acquire)
    }
}

/// 消费者只能持有可撤销授权，不能绕过连接生存期拿到原始 tmux Arc。
pub(super) struct Bound {
    attempt: Arc<Attempt>,
    binding: Arc<TmuxBinding>,
    source: TmuxSnapshot,
}

impl Bound {
    pub(super) fn cwd(&self) -> &Path {
        &self.source.cwd
    }

    fn live(&self) -> io::Result<()> {
        self.attempt
            .live
            .load(Ordering::Acquire)
            .then_some(())
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "终端绑定已撤销"))
    }

    pub(super) fn snapshot(&self) -> io::Result<TmuxSnapshot> {
        self.live()?;
        let snapshot = self.binding.snapshot()?;
        self.live()?;
        Ok(snapshot)
    }

    pub(super) fn validate_pane(&self, expected: &TmuxSnapshot) -> io::Result<()> {
        self.live()?;
        self.binding.validate_pane(expected)?;
        self.live()
    }

    pub(super) fn split_owned(
        &self,
        program: &Path,
        args: &[OsString],
    ) -> io::Result<SplitOutcome> {
        self.live()?;
        // 领取先赢则保留原请求结果；后续断开不会授权重放或另开一个 pane。
        self.binding
            .split_owned(&self.source, program, args, &self.source.cwd, || {
                self.attempt.claim_launch()
            })
    }
}

pub(super) struct Connection {
    host: String,
    live: AtomicBool,
    initialized: AtomicBool,
    terminals: Mutex<HashMap<u64, Terminal>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Work 不持有 Connection；模型整体析构也必须先封闭尚未执行的后台工作。
        // 不在这里 close/wait 原生资源，其控制客户端的析构自行后台回收。
        self.live.store(false, Ordering::Release);
        let terminals = self
            .terminals
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        for terminal in terminals.values() {
            if let Some(attempt) = &terminal.current {
                attempt.retire();
            }
        }
    }
}

impl Connection {
    pub(super) fn new(host: String) -> Self {
        Self {
            host,
            live: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            terminals: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn initialize(&self) {
        self.initialized.store(true, Ordering::Release);
    }

    /// 调用方须在后台释放返回的对象，控制客户端退出不能阻塞模型线程。
    pub(super) fn bootstrap(&self, message: &SessionBootstrapped) -> Option<Arc<Attempt>> {
        let epoch = Uuid::parse_str(&message.image_staging_epoch).ok()?;
        if epoch.is_nil() || message.image_staging_epoch_revision == 0 {
            return None;
        }
        let mut terminals = self.terminals.lock().expect("终端绑定锁");
        if terminals
            .get(&message.session_id)
            .is_some_and(|terminal| terminal.revision >= message.image_staging_epoch_revision)
        {
            return None;
        }
        let candidate = message
            .shell_pid
            .zip(message.shell_tty.as_ref())
            .and_then(|(pid, tty)| {
                let path = PathBuf::from(tty);
                (pid != 0 && path.is_absolute() && tty.len() <= 4096 && !tty.contains('\0'))
                    .then_some((pid, path))
            });
        let previous = terminals
            .insert(
                message.session_id,
                Terminal {
                    epoch,
                    revision: message.image_staging_epoch_revision,
                    candidate,
                    used: HashSet::new(),
                    current: None,
                },
            )
            .and_then(|terminal| terminal.current);
        if let Some(previous) = &previous {
            previous.retire();
        }
        previous
    }

    pub(super) fn disconnect(&self) -> Vec<Arc<Attempt>> {
        self.live.store(false, Ordering::Release);
        let mut terminals = self.terminals.lock().expect("终端绑定锁");
        let retired: Vec<_> = terminals
            .values_mut()
            .filter_map(|terminal| terminal.current.take())
            .collect();
        for attempt in &retired {
            attempt.retire();
        }
        terminals.clear();
        retired
    }

    fn scope_current(&self, scope: &TerminalBindingScope, terminal: &Terminal) -> bool {
        self.live.load(Ordering::Acquire)
            && self.initialized.load(Ordering::Acquire)
            && scope.host_id == self.host
            && Uuid::parse_str(&scope.terminal_epoch).ok() == Some(terminal.epoch)
    }

    /// 按线上的请求顺序同步领取；后台操作不能让较旧的 Begin 晚到后复活。
    pub(super) fn prepare(&self, request: TerminalBindingRequest) -> Work {
        let mut work = Work {
            request,
            operation: Operation::Failed,
            retired: None,
        };
        let Some(scope) = work.request.scope.as_ref() else {
            return work;
        };
        let Ok(id) = Uuid::parse_str(&work.request.attempt_id) else {
            return work;
        };
        if id.is_nil() {
            return work;
        }
        let mut terminals = self.terminals.lock().expect("终端绑定锁");
        let Some(terminal) = terminals.get_mut(&scope.terminal_session_id) else {
            return work;
        };
        if !self.scope_current(scope, terminal) {
            return work;
        }
        match work.request.action.as_ref() {
            Some(terminal_binding_request::Action::Begin(_)) => {
                let Some(candidate) = terminal.candidate.clone() else {
                    return work;
                };
                if !terminal.used.insert(id) {
                    return work;
                }
                let attempt = Arc::new(Attempt {
                    id,
                    nonce: Uuid::new_v4(),
                    binding_id: Uuid::new_v4(),
                    created: Instant::now(),
                    live: AtomicBool::new(true),
                    launch: AtomicU8::new(0),
                    state: Mutex::new(State::Writing),
                });
                work.retired = terminal.current.replace(attempt.clone());
                if let Some(previous) = &work.retired {
                    previous.retire();
                }
                work.operation = Operation::Begin { attempt, candidate };
            }
            Some(terminal_binding_request::Action::Ack(ack)) => {
                let Some(attempt) = terminal.current.as_ref().filter(|attempt| attempt.id == id)
                else {
                    return work;
                };
                let Ok(nonce) = Uuid::parse_str(&ack.nonce) else {
                    return work;
                };
                if !attempt.pending() || nonce != attempt.nonce {
                    return work;
                }
                let mut state = attempt.state.lock().expect("终端挑战状态锁");
                if matches!(&*state, State::Ready(_)) {
                    let State::Ready(outer) = mem::replace(&mut *state, State::Discovering) else {
                        unreachable!()
                    };
                    work.operation = Operation::Ack {
                        attempt: attempt.clone(),
                        outer,
                    };
                }
            }
            Some(terminal_binding_request::Action::Cancel(_)) => {
                if terminal
                    .current
                    .as_ref()
                    .is_some_and(|attempt| attempt.id == id)
                {
                    work.retired = terminal.current.take();
                    work.retired.as_ref().expect("已检查当前挑战").retire();
                    work.operation = Operation::Cancel;
                }
            }
            None => {}
        }
        work
    }

    /// 仅用于内部 Work 已完成的回执，在模型线程发送前重新核对所属代次。
    /// 原生成功由 Work 验证，此处不执行原生 I/O；Bound 的实际消费还须经过 bound()。
    pub(super) fn completed_response_is_current(&self, response: &TerminalBindingResponse) -> bool {
        let binding_id = match response.result.as_ref() {
            Some(terminal_binding_response::Result::ChallengeWritten(_)) => None,
            Some(terminal_binding_response::Result::Bound(bound)) => Some(&bound.opaque_binding_id),
            Some(terminal_binding_response::Result::Failed(_))
            | Some(terminal_binding_response::Result::Cancelled(_)) => return true,
            None => return false,
        };
        let Some(scope) = response.scope.as_ref() else {
            return false;
        };
        let terminals = self.terminals.lock().expect("终端绑定锁");
        let Some(terminal) = terminals.get(&scope.terminal_session_id) else {
            return false;
        };
        if !self.scope_current(scope, terminal) {
            return false;
        }
        let Some(attempt) = terminal.current.as_ref() else {
            return false;
        };
        if !attempt.live.load(Ordering::Acquire) || response.attempt_id != attempt.id.to_string() {
            return false;
        }
        match binding_id {
            Some(id) => *id == attempt.binding_id.to_string(),
            None => attempt.pending(),
        }
    }

    pub(super) fn bound(&self, scope: &TerminalBindingScope, binding_id: &str) -> Option<Bound> {
        let id = Uuid::parse_str(binding_id).ok()?;
        let terminals = self.terminals.lock().expect("终端绑定锁");
        let terminal = terminals.get(&scope.terminal_session_id)?;
        if !self.scope_current(scope, terminal) {
            return None;
        }
        let attempt = terminal.current.as_ref()?;
        if !attempt.live.load(Ordering::Acquire) || attempt.binding_id != id {
            return None;
        }
        match &*attempt.state.lock().expect("终端挑战状态锁") {
            State::Bound(binding, source) => Some(Bound {
                attempt: attempt.clone(),
                binding: binding.clone(),
                source: source.clone(),
            }),
            State::Writing | State::Ready(_) | State::Discovering | State::Failed => None,
        }
    }
}

enum Operation {
    Begin {
        attempt: Arc<Attempt>,
        candidate: (u32, PathBuf),
    },
    Ack {
        attempt: Arc<Attempt>,
        outer: Arc<OuterTerminal>,
    },
    Cancel,
    Failed,
}

pub(super) struct Work {
    request: TerminalBindingRequest,
    operation: Operation,
    retired: Option<Arc<Attempt>>,
}

impl Work {
    pub(super) fn execute(self) -> TerminalBindingResponse {
        // 旧控制客户端在后台结束；这里从不持有连接或 TerminalModel 锁。
        drop(self.retired);
        let result = match self.operation {
            Operation::Begin {
                attempt,
                candidate: (pid, tty),
            } => {
                let scope = self.request.scope.as_ref().expect("准备阶段已验证 scope");
                let created = (|| {
                    if !attempt.pending() {
                        return None;
                    }
                    let outer = Arc::new(OuterTerminal::capture(pid, &tty).ok()?);
                    outer.validate().ok()?;
                    if !attempt.pending() {
                        return None;
                    }
                    let session = scope.terminal_session_id;
                    let id = attempt.id;
                    let nonce = attempt.nonce;
                    let frame = format!("\x1b]9278;t;1;{session};{id};{nonce}\x07");
                    outer.write_challenge(frame.as_bytes()).ok()?;
                    attempt.pending().then_some(outer)
                })();
                let mut state = attempt.state.lock().expect("终端挑战状态锁");
                if let Some(outer) = created.filter(|_| attempt.pending()) {
                    *state = State::Ready(outer);
                    terminal_binding_response::Result::ChallengeWritten(
                        TerminalBindingChallengeWritten {},
                    )
                } else {
                    *state = State::Failed;
                    failed()
                }
            }
            Operation::Ack { attempt, outer } => {
                let binding = attempt
                    .pending()
                    .then(|| TmuxBinding::discover(outer))
                    .and_then(Result::ok)
                    .and_then(|binding| binding.snapshot().ok().map(|source| (binding, source)));
                let mut state = attempt.state.lock().expect("终端挑战状态锁");
                if let Some((binding, source)) = binding.filter(|_| attempt.pending()) {
                    // source 留在 daemon；后续点击到启动期间切换 pane 不得改选新目标。
                    *state = State::Bound(Arc::new(binding), source);
                    terminal_binding_response::Result::Bound(TerminalBindingBound {
                        opaque_binding_id: attempt.binding_id.to_string(),
                    })
                } else {
                    *state = State::Failed;
                    failed()
                }
            }
            Operation::Cancel => {
                terminal_binding_response::Result::Cancelled(TerminalBindingCancelled {})
            }
            Operation::Failed => failed(),
        };
        TerminalBindingResponse {
            scope: self.request.scope,
            attempt_id: self.request.attempt_id,
            result: Some(result),
        }
    }
}

fn failed() -> terminal_binding_response::Result {
    terminal_binding_response::Result::Failed(TerminalBindingFailed {
        code: "unavailable".into(),
    })
}

#[cfg(test)]
#[path = "terminal_binding_service_tests.rs"]
mod tests;
