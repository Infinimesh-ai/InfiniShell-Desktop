//! 只管理自己创建的无 pane 输出 control client；不复用用户客户端的输入。

use std::ffi::OsStr;
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

use command::blocking::Command;

use super::{Budget, ProcessSnapshot, SocketEndpoint, invalid, platform};

const MAX_FRAME: usize = 16 * 1024;
const MAX_REPLY: usize = 64 * 1024;

pub(super) struct Control {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: ChildStdout,
    process: platform::Process,
    endpoint: SocketEndpoint,
    buffer: Vec<u8>,
    last_number: u64,
    usable: bool,
}

impl Control {
    pub(super) fn start(
        image: &ProcessSnapshot,
        path: &Path,
        server: &platform::Process,
        budget: &Budget,
    ) -> io::Result<Self> {
        let mut child = Command::new(&image.executable)
            .args(["-C", "-N", "-S"])
            .arg(path)
            .args(["attach-session", "-E", "-f", "ignore-size,no-output"])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let setup = (|| {
            let process = platform::Process::capture(child.id() as i32)?;
            if !super::same_image(&process.snapshot()?, image) {
                return Err(invalid("tmux control client 映像不匹配"));
            }
            let input = child
                .stdin
                .take()
                .ok_or_else(|| invalid("tmux control 输入缺失"))?;
            let output = child
                .stdout
                .take()
                .ok_or_else(|| invalid("tmux control 输出缺失"))?;
            nonblocking(input.as_raw_fd())?;
            nonblocking(output.as_raw_fd())?;
            // 等待自己派生的 client 完成连接；此时尚未发送任何查询或 split。
            let endpoint = loop {
                budget.check()?;
                if child.try_wait()?.is_some() {
                    return Err(invalid("tmux control client 提前退出"));
                }
                let server_sockets = server.sockets(budget)?;
                let endpoints: Vec<_> = process
                    .sockets(budget)?
                    .into_iter()
                    .filter(|endpoint| server_sockets.iter().any(|peer| endpoint.connects_to(peer)))
                    .collect();
                if endpoints.len() == 1 {
                    break endpoints.into_iter().next().expect("唯一端点");
                }
                if endpoints.len() > 1 {
                    return Err(invalid("tmux control client 对端不唯一"));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            Ok((input, output, process, endpoint))
        })();
        let (input, output, process, endpoint) = match setup {
            Ok(value) => value,
            Err(error) => {
                // Child 尚未 wait，不可能把这个数字 PID 复用到用户进程。
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut control = Self {
            child: Some(child),
            input: Some(input),
            output,
            process,
            endpoint,
            buffer: Vec::new(),
            last_number: 0,
            usable: true,
        };
        control.reply(budget, 0)?;
        control.validate(server, budget)?;
        Ok(control)
    }

    pub(super) fn validate(
        &mut self,
        server: &platform::Process,
        budget: &Budget,
    ) -> io::Result<()> {
        if !self.usable
            || self
                .child
                .as_mut()
                .ok_or_else(|| invalid("tmux control 已关闭"))?
                .try_wait()?
                .is_some()
        {
            return Err(invalid("tmux control 生存期已结束"));
        }
        let client = self.process.sockets(budget)?;
        let server = server.sockets(budget)?;
        if !client.iter().any(|item| item == &self.endpoint)
            || !server.iter().any(|peer| self.endpoint.connects_to(peer))
        {
            self.usable = false;
            return Err(invalid("tmux control 的已验证连接已改变"));
        }
        Ok(())
    }

    pub(super) fn command(
        &mut self,
        arguments: &[&OsStr],
        budget: &Budget,
    ) -> io::Result<Vec<Vec<u8>>> {
        self.command_replies(arguments, budget, false)
    }

    pub(super) fn guarded_split(
        &mut self,
        arguments: &[&OsStr],
        budget: &Budget,
    ) -> io::Result<Vec<Vec<u8>>> {
        self.command_replies(arguments, budget, true)
    }

    fn command_replies(
        &mut self,
        arguments: &[&OsStr],
        budget: &Budget,
        guarded: bool,
    ) -> io::Result<Vec<Vec<u8>>> {
        let frame = encode(arguments)?;
        if !self.usable {
            return Err(invalid("tmux control 已失效"));
        }
        // 首次发送以后任何错误都会毒化本连接；调用方不得重放 split。
        self.usable = false;
        let input = self
            .input
            .as_ref()
            .ok_or_else(|| invalid("tmux control 已关闭"))?;
        let written = unsafe { libc::write(input.as_raw_fd(), frame.as_ptr().cast(), frame.len()) };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written as usize != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "tmux 命令短写，结果未知",
            ));
        }
        let mut reply = self.reply(budget, 1)?;
        if guarded {
            if !reply.is_empty() {
                return Err(invalid("tmux 条件命令包含意外输出"));
            }
            reply = self.reply(budget, 1)?;
        }
        self.usable = true;
        Ok(reply)
    }

    fn reply(&mut self, budget: &Budget, expected_flags: u64) -> io::Result<Vec<Vec<u8>>> {
        let mut consumed = 0;
        loop {
            let line = self.line(budget, &mut consumed)?;
            if !line.starts_with(b"%begin ") {
                validate_notification(&line)?;
                continue;
            }
            let header = guard(&line, b"%begin ")?;
            if header.1 <= self.last_number || header.2 > 1 {
                return Err(invalid("tmux control 命令编号无效"));
            }
            let mut reply = Vec::new();
            loop {
                let line = self.line(budget, &mut consumed)?;
                let end = line.starts_with(b"%end ");
                let error = line.starts_with(b"%error ");
                if end || error {
                    let prefix: &[u8] = if end { b"%end " } else { b"%error " };
                    if guard(&line, prefix)? != header {
                        return Err(invalid("tmux control 回执编号不匹配"));
                    }
                    self.last_number = header.1;
                    if header.2 == expected_flags {
                        if error {
                            return Err(io::Error::other("tmux 拒绝受限命令"));
                        }
                        return Ok(reply);
                    }
                    break;
                }
                if line.starts_with(b"%begin ") {
                    return Err(invalid("tmux control 回执交错"));
                }
                if header.2 == expected_flags {
                    reply.push(line);
                }
            }
        }
    }

    fn line(&mut self, budget: &Budget, consumed: &mut usize) -> io::Result<Vec<u8>> {
        loop {
            budget.check()?;
            if let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<_> = self.buffer.drain(..=index).collect();
                line.pop();
                *consumed += line.len() + 1;
                if *consumed > MAX_REPLY {
                    return Err(invalid("tmux control 回执超过预算"));
                }
                return Ok(line);
            }
            if self.buffer.len() > MAX_REPLY {
                return Err(invalid("tmux control 行超过预算"));
            }
            let mut bytes = [0u8; 4096];
            let size = unsafe {
                libc::read(
                    self.output.as_raw_fd(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                )
            };
            if size > 0 {
                self.buffer.extend_from_slice(&bytes[..size as usize]);
                continue;
            }
            if size == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "tmux control 已断开",
                ));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock
                && error.kind() != io::ErrorKind::Interrupted
            {
                return Err(error);
            }
            let mut poll = libc::pollfd {
                fd: self.output.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            unsafe {
                libc::poll(&mut poll, 1, 20);
            }
        }
    }

    pub(super) fn close(&mut self) -> io::Result<()> {
        self.usable = false;
        // control.c 的空行仅 detach 当前这个 control client。
        if let Some(input) = self.input.take() {
            unsafe {
                libc::write(input.as_raw_fd(), b"\n".as_ptr().cast(), 1);
            }
        }
        if let Some(child) = self.child.as_mut() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if child.try_wait()?.is_some() {
                    self.child.take();
                    return Ok(());
                }
                if std::time::Instant::now() >= deadline {
                    child.kill()?;
                    child.wait()?;
                    self.child.take();
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(())
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.input.take();
        if let Some(mut child) = self.child.take() {
            // 只回收本对象拥有且未 wait 的 Child；Drop 不阻塞模型锁。
            let _ = std::thread::Builder::new()
                .name("tmux-control-reap".into())
                .spawn(move || {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    while std::time::Instant::now() < deadline {
                        if child.try_wait().ok().flatten().is_some() {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                });
        }
    }
}

fn nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn guard(line: &[u8], prefix: &[u8]) -> io::Result<(u64, u64, u64)> {
    let body = line
        .strip_prefix(prefix)
        .ok_or_else(|| invalid("tmux control 边界无效"))?;
    let fields: Vec<_> = body.split(|byte| *byte == b' ').collect();
    if fields.len() != 3 {
        return Err(invalid("tmux control 边界字段无效"));
    }
    let number = |value: &[u8]| {
        if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
            return Err(invalid("tmux control 编号无效"));
        }
        std::str::from_utf8(value)
            .map_err(|_| invalid("tmux control 编码无效"))?
            .parse()
            .map_err(|_| invalid("tmux control 编号越界"))
    };
    Ok((number(fields[0])?, number(fields[1])?, number(fields[2])?))
}

fn validate_notification(line: &[u8]) -> io::Result<()> {
    if !line.starts_with(b"%")
        || line.starts_with(b"%output ")
        || line.starts_with(b"%extended-output ")
        || line.starts_with(b"%exit")
        || line.starts_with(b"%end ")
        || line.starts_with(b"%error ")
    {
        return Err(invalid("tmux control 收到未授权输出或状态"));
    }
    Ok(())
}

/// tmux cmd-parse.y 的单引号 token；不是交给 shell 执行的命令字符串。
pub(super) fn encode(arguments: &[&OsStr]) -> io::Result<Vec<u8>> {
    if arguments.is_empty() {
        return Err(invalid("tmux 命令为空"));
    }
    let mut frame = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if index != 0 {
            frame.push(b' ');
        }
        frame.push(b'\'');
        for byte in argument.as_bytes() {
            if matches!(*byte, 0 | b'\n' | b'\r') {
                return Err(invalid("tmux 参数含控制分隔符"));
            }
            if *byte == b'\'' {
                frame.extend_from_slice(b"'\\''");
            } else {
                frame.push(*byte);
            }
        }
        frame.push(b'\'');
        if frame.len() >= MAX_FRAME {
            return Err(invalid("tmux 命令超过预算"));
        }
    }
    frame.push(b'\n');
    Ok(frame)
}

#[cfg(test)]
#[path = "tmux_native_control_tests.rs"]
mod tests;
