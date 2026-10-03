//! 持久目标只标识原 server/pane；重连授权必须来自新的终端挑战与当前选择。

use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Budget, MAX_TMUX_PROCESSES, ProcessSnapshot, TmuxSnapshot, Token, invalid, platform};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessTarget {
    token: Token,
    uid: u32,
    executable: PathBuf,
    executable_file: (u64, u64),
}

impl ProcessTarget {
    fn same_birth(&self, other: &Self) -> io::Result<bool> {
        if self.token.pid != other.token.pid || self.token.boot != other.token.boot {
            return Ok(false);
        }
        #[cfg(target_os = "macos")]
        {
            Ok(self.token.macos_identity()?.unique_id == other.token.macos_identity()?.unique_id)
        }
        #[cfg(target_os = "linux")]
        {
            Ok(self
                .token
                .linux_identity()?
                .same_lifetime(other.token.linux_identity()?))
        }
    }

    fn capture(process: &platform::Process) -> io::Result<Self> {
        let before = process.snapshot()?;
        let token = process.token()?;
        let after = process.snapshot()?;
        if before.uid != after.uid
            || before.executable != after.executable
            || before.executable_file != after.executable_file
        {
            return Err(invalid("tmux 持久进程映像已改变"));
        }
        Ok(Self {
            token,
            uid: after.uid,
            executable: after.executable,
            executable_file: after.executable_file,
        })
    }

    fn restore(&self) -> io::Result<platform::Process> {
        // PID 仅用于取得候选句柄；完整出生身份与映像均匹配后才允许使用。
        let process = platform::Process::capture(self.token.pid)?;
        if Self::capture(&process)? != *self {
            return Err(invalid("tmux 持久进程身份不匹配"));
        }
        Ok(process)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TmuxTarget {
    version: u32,
    server: ProcessTarget,
    pane: ProcessTarget,
    session_id: String,
    window_id: String,
    pane_id: String,
    pane_session: i32,
    tty: PathBuf,
    tty_file: (u64, u64, u64),
}

impl TmuxSnapshot {
    pub(crate) fn export_target(&self) -> io::Result<TmuxTarget> {
        let server = ProcessTarget::capture(&self.server)?;
        let pane = ProcessTarget::capture(&self.process)?;
        let tty_file = terminal_identity(&self.pane_tty)?;
        let snapshot = self.process.snapshot()?;
        validate_pane(
            &snapshot,
            &self.server.snapshot()?,
            self.pane_session,
            tty_file.2,
        )?;
        if self.server_pid != server.token.pid as u32
            || self.pane_pid != pane.token.pid as u32
            || ProcessTarget::capture(&self.server)? != server
            || ProcessTarget::capture(&self.process)? != pane
        {
            return Err(invalid("tmux 持久目标取样期间已改变"));
        }
        Ok(TmuxTarget {
            version: 1,
            server,
            pane,
            session_id: self.session_id.clone(),
            window_id: self.window_id.clone(),
            pane_id: self.pane_id.clone(),
            pane_session: self.pane_session,
            tty: self.pane_tty.clone(),
            tty_file,
        })
    }

    pub(crate) fn validate_target(&self, target: &TmuxTarget) -> io::Result<()> {
        if self.export_target()? != *target {
            return Err(invalid("tmux 当前选择不属于原持久目标"));
        }
        Ok(())
    }
}

impl TmuxTarget {
    /// 未知派发只约束原 server 下的消费者；不同 server 的稳定证据不受影响。
    pub(crate) fn matches_server_consumer(&self, pid: i32, tty: u64) -> io::Result<bool> {
        if self.version != 1 || pid <= 1 || tty == 0 {
            return Err(invalid("tmux server 匹配候选无效"));
        }
        let consumer = platform::Process::capture(pid)?;
        let before = consumer.snapshot()?;
        if before.tty != tty || before.session <= 1 || before.uid != unsafe { libc::geteuid() } {
            return Err(invalid("tmux server 匹配终端无效"));
        }
        let leader = platform::Process::capture(before.session)?;
        let session = leader.snapshot()?;
        if session.session != session.pid
            || session.uid != before.uid
            || session.tty != tty
            || consumer.snapshot()? != before
        {
            return Err(invalid("tmux 消费者 session 身份已改变"));
        }
        if leader.snapshot()? != session {
            return Err(invalid("tmux 消费者 session 取样期间已改变"));
        }
        if session.parent != self.server.token.pid {
            return Ok(false);
        }
        // 同号父进程仍需完整出生代次匹配，不能把 PID 复用当作原 server。
        let actual = platform::Process::capture(session.parent)?;
        let identity = ProcessTarget::capture(&actual)?;
        if leader.snapshot()? != session || consumer.snapshot()? != before {
            return Err(invalid("tmux server 匹配期间进程已改变"));
        }
        if !identity.same_birth(&self.server)? {
            return Ok(false);
        }
        if identity != self.server {
            return Err(invalid("tmux 原 server 的映像或身份已改变"));
        }
        Ok(true)
    }

    /// 精确注册表路径必须是该前台消费者当前持有的原生监听端点。
    pub(crate) fn validate_listener(&self, pid: i32, tty: u64, path: &Path) -> io::Result<()> {
        self.validate_consumer(pid, tty)?;
        let budget = Budget::new();
        let process = platform::Process::capture(pid)?;
        let file = super::private_socket(path)?;
        let endpoints: Vec<_> = process
            .sockets(&budget)?
            .into_iter()
            .filter(|endpoint| endpoint.listening && endpoint.local_path.as_deref() == Some(path))
            .collect();
        if endpoints.len() != 1 {
            return Err(invalid("tmux 消费者没有唯一匹配的监听端点"));
        }
        let current: Vec<_> = process
            .sockets(&budget)?
            .into_iter()
            .filter(|endpoint| endpoint.listening && endpoint.local_path.as_deref() == Some(path))
            .collect();
        if current != endpoints || super::private_socket(path)? != file {
            return Err(invalid("tmux 消费者监听端点代次已改变"));
        }
        self.validate_consumer(pid, tty)
    }

    /// 确定属于另一个内核 session/TTY 才返回 false；身份取证失败仍为错误。
    pub(crate) fn matches_consumer(&self, pid: i32, tty: u64) -> io::Result<bool> {
        if self.version != 1 || pid <= 1 || tty == 0 || self.pane_session != self.pane.token.pid {
            return Err(invalid("tmux 消费者匹配候选无效"));
        }
        let consumer = platform::Process::capture(pid)?;
        let before = consumer.snapshot()?;
        if before.tty != tty || consumer.snapshot()? != before {
            return Err(invalid("tmux 消费者匹配期间身份已改变"));
        }
        if before.tty != self.tty_file.2 || before.session != self.pane_session {
            return Ok(false);
        }
        let leader = platform::Process::capture(before.session)?;
        let session = leader.snapshot()?;
        let identity = ProcessTarget::capture(&leader)?;
        if session.session != session.pid
            || session.tty != tty
            || session.uid != before.uid
            || consumer.snapshot()? != before
            || leader.snapshot()? != session
        {
            return Err(invalid("tmux 消费者所属 session 身份已改变"));
        }
        if !identity.same_birth(&self.pane)? {
            return Ok(false);
        }
        self.validate_consumer(pid, tty)?;
        Ok(true)
    }

    /// 只返回目标前台终端的内核候选；调用方仍须核对 CLI 映像与精确注册表。
    pub(crate) fn consumer_candidates(&self, executable: &Path) -> io::Result<Vec<(i32, u64)>> {
        let budget = Budget::new();
        let server = self.server.restore()?;
        let pane = self.pane.restore()?;
        let state = pane.snapshot()?;
        let image = fs::metadata(executable)?;
        validate_pane(
            &state,
            &server.snapshot()?,
            self.pane_session,
            self.tty_file.2,
        )?;
        if self.version != 1
            || state.foreground_group <= 0
            || terminal_identity(&self.tty)? != self.tty_file
        {
            return Err(invalid("tmux 目标前台进程组不可用"));
        }
        let mut candidates = Vec::new();
        for pid in platform::pids(Some(state.foreground_group), &budget)? {
            budget.check()?;
            let Ok(process) = platform::Process::capture(pid) else {
                continue;
            };
            let now = process.snapshot()?;
            if now.uid == self.pane.uid
                && now.session == self.pane_session
                && now.tty == self.tty_file.2
                && now.group == state.foreground_group
                && now.foreground_group == now.group
                && now.executable == executable
                && now.executable_file == (image.dev(), image.ino())
            {
                if candidates.len() >= MAX_TMUX_PROCESSES {
                    return Err(invalid("tmux 前台消费者候选超过预算"));
                }
                self.validate_consumer(pid, now.tty)?;
                candidates.push((pid, now.tty));
            }
        }
        if pane.snapshot()?.foreground_group != state.foreground_group
            || ProcessTarget::capture(&server)? != self.server
            || ProcessTarget::capture(&pane)? != self.pane
        {
            return Err(invalid("tmux 候选取样期间目标已改变"));
        }
        Ok(candidates)
    }

    /// 仅在原 CLI 强绑定成功后调用；不替代 socket、原生会话或当前选择校验。
    pub(crate) fn validate_consumer(&self, pid: i32, tty: u64) -> io::Result<()> {
        if self.version != 1 || pid <= 1 || tty == 0 || tty != self.tty_file.2 {
            return Err(invalid("tmux 消费者目标无效"));
        }
        let server = self.server.restore()?;
        let pane = self.pane.restore()?;
        if terminal_identity(&self.tty)? != self.tty_file {
            return Err(invalid("tmux 持久终端身份已改变"));
        }
        validate_pane(
            &pane.snapshot()?,
            &server.snapshot()?,
            self.pane_session,
            tty,
        )?;
        let consumer = platform::Process::capture(pid)?;
        let before = consumer.snapshot()?;
        if before.uid != self.pane.uid
            || before.session != self.pane_session
            || before.tty != tty
            || before.group <= 0
            || before.foreground_group != before.group
        {
            return Err(invalid("tmux 原生消费者不属于目标前台终端"));
        }
        if consumer.snapshot()? != before
            || ProcessTarget::capture(&server)? != self.server
            || ProcessTarget::capture(&pane)? != self.pane
            || terminal_identity(&self.tty)? != self.tty_file
        {
            return Err(invalid("tmux 消费者校验期间身份已改变"));
        }
        Ok(())
    }
}

fn validate_pane(
    pane: &ProcessSnapshot,
    server: &ProcessSnapshot,
    session: i32,
    tty: u64,
) -> io::Result<()> {
    if pane.uid != server.uid
        || pane.session != pane.pid
        || pane.session != session
        || pane.parent != server.pid
        || pane.tty != tty
    {
        return Err(invalid("tmux 持久 pane 与原生 session/TTY 不匹配"));
    }
    Ok(())
}

fn terminal_identity(path: &Path) -> io::Result<(u64, u64, u64)> {
    if !path.is_absolute() || fs::canonicalize(path)? != path {
        return Err(invalid("tmux 终端路径不是规范绝对路径"));
    }
    let before = fs::symlink_metadata(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let opened = file.metadata()?;
    let after = fs::symlink_metadata(path)?;
    let identity = |metadata: &fs::Metadata| (metadata.dev(), metadata.ino(), metadata.rdev());
    if !opened.file_type().is_char_device()
        || opened.uid() != unsafe { libc::geteuid() }
        || identity(&before) != identity(&opened)
        || identity(&after) != identity(&opened)
    {
        return Err(invalid("tmux 终端文件身份已改变"));
    }
    Ok(identity(&opened))
}

#[cfg(test)]
#[path = "tmux_native_target_tests.rs"]
mod tests;
