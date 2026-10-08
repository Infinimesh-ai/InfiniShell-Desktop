//! 已验证 Windows Codex npm 探针在原认证连接上的协作取消与固定 Job 授权。

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) const COOPERATIVE_EXIT_TIMEOUT: Duration = Duration::from_secs(8);
const STOP_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_JOB_REQUEST: usize = 4096;
const JOB_ACK_FIRST: u8 = 3;
const JOB_ACK_SECOND: u8 = 4;

pub(super) fn request_stop(stream: &mut TcpStream) -> io::Result<()> {
    let result = stream
        .set_write_timeout(Some(STOP_WRITE_TIMEOUT))
        .and_then(|()| stream.write_all(&[2]));
    // 写入失败时也关闭发送方向，让仍可达的监听者按 EOF 取消；没有重连或新授权。
    let _ = stream.shutdown(Shutdown::Write);
    result
}

#[derive(Default)]
struct AuthorizationState {
    completed: u8,
    waiting: Option<u8>,
    acknowledged: bool,
}

impl AuthorizationState {
    fn begin(&mut self, stage: u8) -> io::Result<()> {
        if stage > 1 || self.completed != stage || self.waiting.is_some() {
            return Err(io::Error::other("managed_process.probe_job_stage_invalid"));
        }
        self.waiting = Some(stage);
        self.acknowledged = false;
        Ok(())
    }

    fn acknowledge(&mut self, byte: u8) -> io::Result<()> {
        let expected = match self.waiting {
            Some(0) => JOB_ACK_FIRST,
            Some(1) => JOB_ACK_SECOND,
            Some(_) | None => {
                return Err(io::Error::other("managed_process.probe_job_ack_unexpected"));
            }
        };
        if byte != expected || self.acknowledged {
            return Err(io::Error::other("managed_process.probe_job_ack_invalid"));
        }
        self.acknowledged = true;
        Ok(())
    }
}

pub(super) struct CancellationListener {
    cancelled: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    control: TcpStream,
    request: Mutex<()>,
    authorization: Arc<(Mutex<AuthorizationState>, Condvar)>,
}

impl CancellationListener {
    /// 调用方必须先核对原握手和完整 launch binding，不能接受其他来源的连接。
    pub(super) fn start(control: TcpStream) -> io::Result<Self> {
        // Windows 接收超时后的连接不能继续复用；改用非阻塞读取及可唤醒的短等待。
        control.set_read_timeout(None)?;
        control.set_nonblocking(true)?;
        let mut reader_control = control.try_clone()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader_cancelled = Arc::clone(&cancelled);
        let authorization = Arc::new((Mutex::new(AuthorizationState::default()), Condvar::new()));
        let reader_authorization = Arc::clone(&authorization);
        let reader = thread::Builder::new()
            .name("cli-npm-cancel".to_owned())
            .spawn(move || {
                let mut bytes = [0_u8; 16];
                while !reader_cancelled.load(Ordering::Acquire) {
                    match reader_control.read(&mut bytes) {
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::park_timeout(CANCELLATION_POLL_INTERVAL);
                        }
                        Ok(count) if count > 0 => {
                            let (state, ready) = &*reader_authorization;
                            let Ok(mut state) = state.lock() else {
                                reader_cancelled.store(true, Ordering::Release);
                                ready.notify_all();
                                break;
                            };
                            // 同批回复先完整检查；停止、额外 ACK 或非预期字节不能释放等待者。
                            if bytes[..count]
                                .iter()
                                .any(|byte| state.acknowledge(*byte).is_err())
                            {
                                reader_cancelled.store(true, Ordering::Release);
                            }
                            ready.notify_all();
                        }
                        // EOF 或错误仍是协作取消，绝不当作成功授权。
                        Ok(_) | Err(_) => {
                            reader_cancelled.store(true, Ordering::Release);
                            reader_authorization.1.notify_all();
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            cancelled,
            reader: Some(reader),
            control,
            request: Mutex::new(()),
            authorization,
        })
    }

    pub(super) fn token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    #[cfg(windows)]
    pub(super) fn authorize_process(
        &self,
        operation: command::managed::ProbeJobOperation,
        process: std::os::windows::io::BorrowedHandle<'_>,
    ) -> io::Result<()> {
        use command::managed::{ProbeJobOperation, ProbeJobRequest};
        let request = ProbeJobRequest::capture(operation, process)?;
        let bytes = serde_json::to_vec(&request).map_err(io::Error::other)?;
        let stage = match operation {
            ProbeJobOperation::ClaimBootstrap => 0,
            ProbeJobOperation::VerifyCandidate => 1,
        };
        self.authorize_payload(stage, &bytes)
    }

    fn authorize_payload(&self, stage: u8, bytes: &[u8]) -> io::Result<()> {
        let result = (|| {
            // 只允许一个同步请求，不排队；共两阶段，复用原握手期限而非延长探针期限。
            let _exclusive = self
                .request
                .try_lock()
                .map_err(|_| io::Error::other("managed_process.probe_job_request_concurrent"))?;
            let deadline = Instant::now() + super::HANDSHAKE_TIMEOUT;
            self.check_cancelled()?;
            if bytes.is_empty() || bytes.len() > MAX_JOB_REQUEST {
                return Err(io::Error::other("managed_process.probe_job_request_size"));
            }
            let (state, ready) = &*self.authorization;
            state
                .lock()
                .map_err(|_| io::Error::other("managed_process.probe_job_state_poisoned"))?
                .begin(stage)?;
            let mut frame = Vec::with_capacity(4 + bytes.len());
            frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            frame.extend_from_slice(bytes);
            let mut written = 0;
            let mut control = &self.control;
            while written < frame.len() {
                self.check_cancelled()?;
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "managed_process.probe_job_authorization_timeout",
                    ));
                }
                match control.write(&frame[written..]) {
                    Ok(0) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "managed_process.probe_job_control_closed",
                        ));
                    }
                    Ok(count) => written += count,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::park_timeout(CANCELLATION_POLL_INTERVAL)
                    }
                    Err(error) => return Err(error),
                }
            }
            let mut state = state
                .lock()
                .map_err(|_| io::Error::other("managed_process.probe_job_state_poisoned"))?;
            loop {
                self.check_cancelled()?;
                if state.acknowledged {
                    state.completed += 1;
                    state.waiting = None;
                    state.acknowledged = false;
                    return Ok(());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "managed_process.probe_job_authorization_timeout",
                    ));
                }
                state = ready
                    .wait_timeout(state, remaining.min(CANCELLATION_POLL_INTERVAL))
                    .map_err(|_| io::Error::other("managed_process.probe_job_state_poisoned"))?
                    .0;
            }
        })();
        if result.is_err() {
            self.cancelled.store(true, Ordering::Release);
        }
        result
    }

    fn check_cancelled(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "managed_process.probe_job_cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for CancellationListener {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.authorization.1.notify_all();
        if let Some(reader) = self.reader.take() {
            reader.thread().unpark();
            let _ = reader.join();
        }
    }
}

/// 仅监督循环调用，不增加读线程；每轮最多读取一个有界片段、授权一个固定请求。
pub(super) struct SupervisorControl {
    stream: TcpStream,
    frame: Vec<u8>,
    expected: usize,
    started: Option<Instant>,
    acknowledged: u8,
    pending_ack: Option<u8>,
    closed: bool,
}

impl SupervisorControl {
    pub(super) fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_read_timeout(None)?;
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            frame: Vec::with_capacity(MAX_JOB_REQUEST + 4),
            expected: 4,
            started: None,
            acknowledged: 0,
            pending_ack: None,
            closed: false,
        })
    }

    pub(super) fn poll(
        &mut self,
        mut authorize: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        if self
            .started
            .is_some_and(|started| started.elapsed() >= super::HANDSHAKE_TIMEOUT)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "managed_process.probe_job_frame_timeout",
            ));
        }
        if let Some(byte) = self.pending_ack {
            match self.stream.write(&[byte]) {
                Ok(1) => {
                    self.pending_ack = None;
                    self.started = None;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "managed_process.probe_job_reply_closed",
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        let mut buffer = [0u8; MAX_JOB_REQUEST];
        let count = match self
            .stream
            .read(&mut buffer[..self.expected - self.frame.len()])
        {
            Ok(0) if self.frame.is_empty() => {
                self.closed = true;
                return Ok(());
            }
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "managed_process.probe_job_frame_incomplete",
                ));
            }
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        };
        if self.acknowledged >= 2 {
            return Err(io::Error::other("managed_process.probe_job_request_limit"));
        }
        self.started.get_or_insert_with(Instant::now);
        self.frame.extend_from_slice(&buffer[..count]);
        if self.frame.len() == 4 {
            let length = u32::from_le_bytes(self.frame[..4].try_into().unwrap()) as usize;
            if length == 0 || length > MAX_JOB_REQUEST {
                return Err(io::Error::other("managed_process.probe_job_request_size"));
            }
            self.expected = length + 4;
            return Ok(());
        }
        if self.frame.len() == self.expected {
            authorize(&self.frame[4..])?;
            self.pending_ack = Some(if self.acknowledged == 0 {
                JOB_ACK_FIRST
            } else {
                JOB_ACK_SECOND
            });
            self.acknowledged += 1;
            self.frame.clear();
            self.expected = 4;
        }
        Ok(())
    }

    pub(super) fn request_stop(&mut self) -> io::Result<()> {
        self.closed = true;
        self.pending_ack = None;
        request_stop(&mut self.stream)
    }
}

#[cfg(test)]
#[path = "managed_process_probe_control_tests.rs"]
mod tests;
