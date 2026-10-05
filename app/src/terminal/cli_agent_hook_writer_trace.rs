//! 仅测试构建的 Linux 单次通知阶段记录；缺少记录不等于对应阶段未执行。

use std::env;
use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Mutex, OnceLock};

const AUTHORIZATION: &str = "INFINISHELL_TEST_NOTIFY_TRACE";
const RECORD_SIZE: usize = 48;
const MAX_RECORDS: u32 = 32;
static TRACE: OnceLock<Mutex<Trace>> = OnceLock::new();

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Stage {
    Main = 1,
    FluentBegin,
    FluentEnd,
    LeaseBegin,
    LeaseEnd,
    Worker,
    InputBegin,
    InputReceived,
    Prepare,
    Terminal,
    Cache,
    LockOpen,
    LockWait,
    FrameWrite,
    FrameWritten,
    ReplyBegin,
    ReplyEnd,
    MainOk,
    MainError,
}

struct Trace {
    descriptor: OwnedFd,
    nonce: [u8; 16],
    sequence: u32,
    active: bool,
}

fn authorization(value: &str) -> Option<(u64, u64, [u8; 16])> {
    let mut parts = value.split(':');
    if parts.next()? != "v1" {
        return None;
    }
    let device = parts.next()?.parse().ok()?;
    let inode = parts.next()?.parse().ok()?;
    let hex = parts.next()?;
    if parts.next().is_some() || hex.len() != 32 {
        return None;
    }
    let mut nonce = [0; 16];
    for (byte, pair) in nonce.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let digit = |value| match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            _ => None,
        };
        *byte = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some((device, inode, nonce))
}

fn validate_pipe(descriptor: RawFd, value: &str) -> Option<[u8; 16]> {
    let (device, inode, nonce) = authorization(value)?;
    let mut info = MaybeUninit::<libc::stat>::uninit();
    // 只查询已继承的专用描述符；不打开路径，不更改标准流或信号策略。
    let info = unsafe {
        if libc::fstat(descriptor, info.as_mut_ptr()) != 0 {
            return None;
        }
        info.assume_init()
    };
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    let capacity = unsafe { libc::fcntl(descriptor, libc::F_GETPIPE_SZ) };
    if info.st_mode & libc::S_IFMT != libc::S_IFIFO
        || info.st_uid != unsafe { libc::geteuid() }
        || info.st_dev != device
        || info.st_ino != inode
        || flags < 0
        || flags & libc::O_ACCMODE != libc::O_WRONLY
        || flags & libc::O_NONBLOCK == 0
        || capacity < (RECORD_SIZE * MAX_RECORDS as usize) as i32
    {
        return None;
    }
    Some(nonce)
}

fn authorized_pipe(
    mut arguments: impl Iterator<Item = OsString>,
    value: &str,
    descriptor: RawFd,
) -> Option<[u8; 16]> {
    if arguments.next().as_deref() != Some("cli-agent-notify".as_ref())
        || arguments.next().as_deref() != Some("--require-protocol".as_ref())
        || arguments.next().as_deref() != Some("1".as_ref())
        || arguments.next().is_some()
    {
        return None;
    }
    let nonce = validate_pipe(descriptor, value)?;
    let mut disposition = MaybeUninit::<libc::sigaction>::uninit();
    // 不修正信号处理器；若运行时未忽略 SIGPIPE，就不开启可能写入断管的诊断。
    unsafe {
        if libc::sigaction(libc::SIGPIPE, std::ptr::null(), disposition.as_mut_ptr()) != 0
            || disposition.assume_init().sa_sigaction != libc::SIG_IGN
        {
            return None;
        }
    }
    Some(nonce)
}

pub fn initialize() {
    if TRACE.get().is_some() {
        return;
    }
    let Ok(value) = env::var(AUTHORIZATION) else {
        return;
    };
    let Some(nonce) = authorized_pipe(env::args_os().skip(1), &value, 3) else {
        return;
    };
    // 只在完整授权后接管 FD3；进程退出关闭它，子进程不能继承它。
    let descriptor = unsafe { OwnedFd::from_raw_fd(3) };
    unsafe {
        let flags = libc::fcntl(3, libc::F_GETFD);
        if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags | libc::FD_CLOEXEC) != 0 {
            return;
        }
        let _ = TRACE.set(Mutex::new(Trace {
            descriptor,
            nonce,
            sequence: 0,
            active: true,
        }));
    }
    emit(Stage::Main);
}

impl Trace {
    fn emit(&mut self, stage: Stage) {
        if !self.active || self.sequence >= MAX_RECORDS {
            return;
        }
        let mut now = MaybeUninit::<libc::timespec>::uninit();
        let now = unsafe {
            if libc::clock_gettime(libc::CLOCK_MONOTONIC, now.as_mut_ptr()) != 0 {
                return;
            }
            now.assume_init()
        };
        let Some(timestamp) = u64::try_from(now.tv_sec).ok().and_then(|seconds| {
            seconds
                .checked_mul(1_000_000_000)?
                .checked_add(u64::try_from(now.tv_nsec).ok()?)
        }) else {
            return;
        };
        let mut record = [0; RECORD_SIZE];
        record[..4].copy_from_slice(b"INW1");
        record[4] = stage as u8;
        record[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        record[12..16].copy_from_slice(&std::process::id().to_le_bytes());
        record[16..24].copy_from_slice(&timestamp.to_le_bytes());
        record[24..40].copy_from_slice(&self.nonce);
        // 一次非阻塞写；不重试、不刷新、不等待，失败后停止记录，不影响原业务返回。
        let written = unsafe {
            libc::write(
                self.descriptor.as_raw_fd(),
                record.as_ptr().cast(),
                record.len(),
            )
        };
        self.active = written == RECORD_SIZE as isize;
        self.sequence += 1;
    }
}

pub fn emit(stage: Stage) {
    if let Some(trace) = TRACE.get() {
        // 阶段来自多个原线程，争用时允许缺项，绝不让诊断锁阻塞原发送。
        if let Ok(mut trace) = trace.try_lock() {
            trace.emit(stage);
        }
    }
}

#[cfg(test)]
#[path = "cli_agent_hook_writer_trace_tests.rs"]
mod tests;
