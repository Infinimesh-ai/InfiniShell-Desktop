//! SSH 外层终端与现有 tmux 的原生绑定；CLI 会话授权由上层独立维护。

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use socket2::{Domain, SockAddr, Socket, Type};

#[path = "tmux_native_control.rs"]
mod control;
#[cfg(target_os = "macos")]
#[path = "tmux_native_macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "tmux_native_linux.rs"]
mod platform;

const MAX_PIDS: usize = 2048;
const MAX_FDS: usize = 1024;
const MAX_TMUX_PROCESSES: usize = 16;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const CHALLENGE_PREFIX: &[u8] = b"\x1b]9278;t;1;";

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

struct Budget {
    deadline: Instant,
}

impl Budget {
    fn new() -> Self {
        Self {
            deadline: Instant::now() + DISCOVERY_TIMEOUT,
        }
    }

    fn check(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "tmux 身份发现超过预算",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessSnapshot {
    pid: i32,
    parent: i32,
    uid: u32,
    group: i32,
    foreground_group: i32,
    session: i32,
    tty: u64,
    executable: PathBuf,
    executable_file: (u64, u64),
    cwd: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SocketEndpoint {
    descriptor: i32,
    socket: u64,
    protocol: u64,
    peer_socket: u64,
    peer_protocol: u64,
    listening: bool,
    local_path: Option<PathBuf>,
    peer_path: Option<PathBuf>,
}

impl SocketEndpoint {
    fn connects_to(&self, peer: &Self) -> bool {
        self.socket != 0
            && self.protocol != 0
            && self.peer_socket != 0
            && self.peer_protocol != 0
            && self.socket == peer.peer_socket
            && self.protocol == peer.peer_protocol
            && self.peer_socket == peer.socket
            && self.peer_protocol == peer.protocol
    }
}

struct OuterTerminalInner {
    process: platform::Process,
    terminal: File,
    path: PathBuf,
    file: (u64, u64, u64),
    session: i32,
}

#[derive(Clone)]
pub(crate) struct OuterTerminal {
    inner: Arc<OuterTerminalInner>,
}

impl OuterTerminal {
    /// PID/路径只是候选；上层收到同一 terminal model 的挑战 ACK 后才能 discover。
    pub(crate) fn capture(pid: u32, tty: &Path) -> io::Result<Self> {
        let pid = i32::try_from(pid).map_err(|_| invalid("外层进程 PID 越界"))?;
        if pid <= 1 || !tty.is_absolute() || fs::canonicalize(tty)? != tty {
            return Err(invalid("外层终端候选无效"));
        }
        let process = platform::Process::capture(pid)?;
        let before = process.snapshot()?;
        let terminal = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(tty)?;
        let metadata = terminal.metadata()?;
        if !metadata.file_type().is_char_device()
            || metadata.uid() != unsafe { libc::geteuid() }
            || before.tty != metadata.rdev()
            || before.session <= 1
            || before != process.snapshot()?
        {
            return Err(invalid("外层终端与内核进程不匹配"));
        }
        Ok(Self {
            inner: Arc::new(OuterTerminalInner {
                process,
                terminal,
                path: tty.to_owned(),
                file: (metadata.dev(), metadata.ino(), metadata.rdev()),
                session: before.session,
            }),
        })
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        self.snapshot().map(|_| ())
    }

    fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        let snapshot = self.inner.process.snapshot()?;
        let open = self.inner.terminal.metadata()?;
        let path = fs::symlink_metadata(&self.inner.path)?;
        if snapshot.session != self.inner.session
            || snapshot.tty != self.inner.file.2
            || (open.dev(), open.ino(), open.rdev()) != self.inner.file
            || (path.dev(), path.ino(), path.rdev()) != self.inner.file
            || path.file_type().is_symlink()
        {
            return Err(invalid("外层终端身份已改变"));
        }
        Ok(snapshot)
    }

    pub(crate) fn write_challenge(&self, frame: &[u8]) -> io::Result<()> {
        validate_challenge(frame)?;
        self.validate()?;
        let written = unsafe {
            libc::write(
                self.inner.terminal.as_raw_fd(),
                frame.as_ptr().cast(),
                frame.len(),
            )
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written as usize != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "终端挑战短写，不得补发",
            ));
        }
        self.validate()
    }
}

fn validate_challenge(frame: &[u8]) -> io::Result<()> {
    if frame.len() > 256 || !frame.starts_with(CHALLENGE_PREFIX) || frame.last() != Some(&7) {
        return Err(invalid("终端挑战帧无效"));
    }
    let body = std::str::from_utf8(&frame[CHALLENGE_PREFIX.len()..frame.len() - 1])
        .map_err(|_| invalid("终端挑战编码无效"))?;
    let fields: Vec<_> = body.split(';').collect();
    if fields.len() != 3
        || fields[0].is_empty()
        || fields[0].len() > 20
        || !fields[0].bytes().all(|byte| byte.is_ascii_digit())
        || fields[0].parse::<u64>().is_err()
    {
        return Err(invalid("终端挑战字段无效"));
    }
    for value in &fields[1..] {
        let parsed = uuid::Uuid::parse_str(value).map_err(|_| invalid("终端挑战 UUID 无效"))?;
        if parsed.is_nil() || parsed.hyphenated().to_string() != *value {
            return Err(invalid("终端挑战 UUID 不是规范形式"));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct TmuxSnapshot {
    pub(crate) client_pid: u32,
    pub(crate) server_pid: u32,
    pub(crate) session_id: String,
    pub(crate) window_id: String,
    pub(crate) pane_id: String,
    pub(crate) pane_pid: u32,
    pub(crate) pane_tty: PathBuf,
    pub(crate) cwd: PathBuf,
    pub(crate) pane_session: i32,
    process: Arc<platform::Process>,
    client: Arc<platform::Process>,
    server: Arc<platform::Process>,
}

pub(crate) enum SplitOutcome {
    Started(TmuxSnapshot),
    SelectionChanged,
    OutcomeUnknown,
}

struct BindingState {
    control: control::Control,
    session_id: Option<String>,
    split_attempted: bool,
}

pub(crate) struct TmuxBinding {
    outer: Arc<OuterTerminal>,
    client: Arc<platform::Process>,
    server: Arc<platform::Process>,
    client_image: ProcessSnapshot,
    client_endpoint: SocketEndpoint,
    server_endpoint: SocketEndpoint,
    state: Mutex<BindingState>,
}

impl TmuxBinding {
    pub(crate) fn discover(outer: Arc<OuterTerminal>) -> io::Result<Self> {
        let budget = Budget::new();
        let terminal = outer.snapshot()?;
        if terminal.foreground_group <= 1 {
            return Err(invalid("外层终端没有有效前台进程组"));
        }
        let mut clients = Vec::new();
        for pid in platform::pids(Some(terminal.foreground_group), &budget)? {
            budget.check()?;
            let Ok(process) = platform::Process::capture(pid) else {
                continue;
            };
            let snapshot = process.snapshot()?;
            if snapshot.uid == terminal.uid
                && snapshot.session == terminal.session
                && snapshot.tty == terminal.tty
                && snapshot.group == terminal.foreground_group
                && snapshot.executable.file_name() == Some(OsStr::new("tmux"))
            {
                clients.push((process, snapshot));
            }
        }
        if clients.len() != 1 {
            return Err(invalid("外层终端的 tmux client 不唯一"));
        }
        let (client, client_image) = clients.pop().expect("唯一 client");
        let endpoints: Vec<_> = client
            .sockets(&budget)?
            .into_iter()
            .filter(|endpoint| !endpoint.listening && endpoint.peer_socket != 0)
            .collect();
        if endpoints.is_empty() || endpoints.len() > 8 {
            return Err(invalid("tmux client 端点数量无效"));
        }
        let mut matching = Vec::new();
        let mut candidates = 0;
        for pid in platform::pids(None, &budget)? {
            budget.check()?;
            if pid == client_image.pid {
                continue;
            }
            let Ok(process) = platform::Process::capture(pid) else {
                continue;
            };
            let Ok(snapshot) = process.snapshot() else {
                continue;
            };
            if !same_image(&client_image, &snapshot) {
                continue;
            }
            candidates += 1;
            if candidates > MAX_TMUX_PROCESSES {
                return Err(invalid("tmux 同映像候选超出预算"));
            }
            for peer in process.sockets(&budget)? {
                for endpoint in &endpoints {
                    if endpoint.connects_to(&peer) {
                        matching.push((pid, endpoint.clone(), peer.clone()));
                    }
                }
            }
        }
        if matching.len() != 1 {
            return Err(invalid("tmux socket 的反向归属不唯一"));
        }
        let (pid, client_endpoint, server_endpoint) = matching.pop().expect("唯一 server");
        let server = platform::Process::capture(pid)?;
        if !same_image(&client_image, &server.snapshot()?) {
            return Err(invalid("tmux server 映像改变"));
        }
        let sockets = server.sockets(&budget)?;
        if !sockets.contains(&server_endpoint)
            || !client.sockets(&budget)?.contains(&client_endpoint)
        {
            return Err(invalid("tmux 端点在发现期间改变"));
        }
        let listeners: Vec<_> = sockets
            .iter()
            .filter(|endpoint| endpoint.listening && endpoint.local_path.is_some())
            .collect();
        if listeners.len() != 1 {
            return Err(invalid("tmux server 监听端点不唯一"));
        }
        let path = listeners[0].local_path.as_ref().expect("监听路径");
        let file = private_socket(path)?;
        // 路径只是候选；连接的内核对端生存期才授予后续建立 control 通道的资格。
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
        socket.connect_timeout(
            &SockAddr::unix(path)?,
            budget.deadline.saturating_duration_since(Instant::now()),
        )?;
        let stream: UnixStream = socket.into();
        platform::validate_peer(&stream, &server, &budget)?;
        if private_socket(path)? != file
            || !server.sockets(&budget)?.contains(&server_endpoint)
        {
            return Err(invalid("tmux 监听路径与已绑定 server 不一致"));
        }
        let control = control::Control::start(&client_image, path, &server, &budget)?;
        let binding = Self {
            outer,
            client: Arc::new(client),
            server: Arc::new(server),
            client_image,
            client_endpoint,
            server_endpoint,
            state: Mutex::new(BindingState {
                control,
                session_id: None,
                split_attempted: false,
            }),
        };
        {
            let mut state = binding
                .state
                .lock()
                .map_err(|_| invalid("tmux 绑定锁已失效"))?;
            let snapshot = binding.snapshot_locked(&mut state, &budget)?;
            state.session_id = Some(snapshot.session_id);
        }
        Ok(binding)
    }

    fn validate_processes(&self, budget: &Budget) -> io::Result<()> {
        let terminal = self.outer.snapshot()?;
        let client = self.client.snapshot()?;
        if client.uid != terminal.uid
            || client.session != terminal.session
            || client.tty != terminal.tty
            || client.group != terminal.foreground_group
            || !same_image(&client, &self.client_image)
            || !same_image(&self.server.snapshot()?, &self.client_image)
            || !self.client.sockets(budget)?.contains(&self.client_endpoint)
            || !self.server.sockets(budget)?.contains(&self.server_endpoint)
        {
            return Err(invalid("tmux 外层线路或原 client 生存期已改变"));
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        self.snapshot().map(|_| ())
    }

    pub(crate) fn snapshot(&self) -> io::Result<TmuxSnapshot> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("tmux 绑定锁已失效"))?;
        self.snapshot_locked(&mut state, &Budget::new())
    }

    fn snapshot_locked(
        &self,
        state: &mut BindingState,
        budget: &Budget,
    ) -> io::Result<TmuxSnapshot> {
        self.validate_processes(budget)?;
        state.control.validate(&self.server, budget)?;
        let filter = format!("#{{==:#{{client_pid}},{}}}", self.client_image.pid);
        let format = "#{client_pid}\t#{client_tty}\t#{client_flags}\t#{session_id}\t#{window_id}\t#{pane_id}\t#{pane_pid}\t#{pane_tty}";
        let rows = state.control.command(
            &[
                OsStr::new("list-clients"),
                OsStr::new("-f"),
                OsStr::new(&filter),
                OsStr::new("-F"),
                OsStr::new(format),
            ],
            budget,
        )?;
        let row = parse_client(&rows)?;
        if row.client_pid != self.client_image.pid as u32
            || row.client_tty != self.outer.inner.path
            || state
                .session_id
                .as_ref()
                .is_some_and(|session| session != &row.session_id)
        {
            return Err(invalid("tmux 原 client 会话或终端改变"));
        }
        let process = Arc::new(platform::Process::capture(row.pane_pid as i32)?);
        let pane = process.snapshot()?;
        let metadata = fs::symlink_metadata(&row.pane_tty)?;
        if !metadata.file_type().is_char_device()
            || metadata.uid() != pane.uid
            || fs::canonicalize(&row.pane_tty)? != row.pane_tty
            || metadata.rdev() != pane.tty
            || pane.uid != self.client_image.uid
            || pane.session != pane.pid
            || pane.parent != self.server.snapshot()?.pid
        {
            return Err(invalid("tmux pane 与内核 session/TTY/父进程不一致"));
        }
        self.validate_processes(budget)?;
        state.control.validate(&self.server, budget)?;
        Ok(TmuxSnapshot {
            client_pid: row.client_pid,
            server_pid: self.server.snapshot()?.pid as u32,
            session_id: row.session_id,
            window_id: row.window_id,
            pane_id: row.pane_id,
            pane_pid: row.pane_pid,
            pane_tty: row.pane_tty,
            cwd: pane.cwd,
            pane_session: pane.session,
            process,
            client: Arc::clone(&self.client),
            server: Arc::clone(&self.server),
        })
    }

    pub(crate) fn validate_pane(&self, expected: &TmuxSnapshot) -> io::Result<()> {
        let current = self.snapshot()?;
        if !current.same_pane(expected) {
            return Err(invalid("tmux 当前 pane 代次已改变"));
        }
        Ok(())
    }

    pub(crate) fn split_owned(
        &self,
        expected: &TmuxSnapshot,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        claim: impl FnOnce() -> bool,
    ) -> io::Result<SplitOutcome> {
        if !program.is_absolute()
            || !cwd.is_absolute()
            || fs::canonicalize(program)? != program
            || fs::canonicalize(cwd)? != cwd
            || !fs::metadata(program)?.is_file()
            || !fs::metadata(cwd)?.is_dir()
        {
            return Err(invalid("owned pane 启动参数不是规范绝对路径"));
        }
        // macOS 的 /bin/sh 是会再次 exec 的选择器；直接使用系统实现，避免发布中间身份。
        #[cfg(target_os = "macos")]
        let shell = fs::canonicalize("/bin/bash")?;
        #[cfg(target_os = "linux")]
        let shell = fs::canonicalize("/bin/sh")?;
        let shell_metadata = fs::metadata(&shell)?;
        let shell_file = (shell_metadata.dev(), shell_metadata.ino());
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("tmux 绑定锁已失效"))?;
        if state.split_attempted {
            return Err(invalid("同一 tmux 绑定已启动过 pane，不得重放"));
        }
        let budget = Budget::new();
        let before = self.snapshot_locked(&mut state, &budget)?;
        if !before.same_pane(expected) || before.cwd != expected.cwd || cwd != expected.cwd {
            return Err(invalid("tmux pane 或点击时的工作目录已改变"));
        }
        let target = format!(
            "{}:{}.{}",
            before.session_id, before.window_id, before.pane_id
        );
        let mut arguments: Vec<OsString> = [
            "split-window",
            "-P",
            "-F",
            "#{pane_id}\t#{pane_pid}\t#{pane_tty}",
            "-t",
            &target,
            "-c",
            "/",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        arguments.push(shell.clone().into_os_string());
        // -p 禁止读取 BASH_ENV/ENV 和继承 shell 函数；不让用户配置改变固定包装器。
        #[cfg(target_os = "macos")]
        arguments.extend(["--noprofile", "--norc", "-p"].map(OsString::from));
        arguments.extend([
            "-c",
            "cd -P \"$1\" || exit 125; shift; \"$@\"; result=$?; exit \"$result\"",
            "infinishell-tmux-owned",
        ].map(OsString::from));
        arguments.push(cwd.as_os_str().to_owned());
        arguments.push(program.as_os_str().to_owned());
        arguments.extend_from_slice(args);
        let references: Vec<_> = arguments.iter().map(OsString::as_os_str).collect();
        let mut split = control::encode(&references)?;
        split.pop();
        let predicate = split_predicate(before.client_pid);
        let guarded = [
            OsStr::new("if-shell"),
            OsStr::new("-F"),
            OsStr::new("-t"),
            OsStr::new(&target),
            OsStr::new(&predicate),
            OsStr::from_bytes(&split),
            OsStr::new("display-message -p infinishell-selection-changed"),
        ];
        control::encode(&guarded)?;
        // 在所有只读准备之后领取；撤销先赢时不写控制通道，也不创建 pane。
        if !claim() {
            return Err(invalid("tmux pane 启动授权已经撤销或领取"));
        }
        state.split_attempted = true;
        let result = state.control.guarded_split(&guarded, &budget);
        // 一旦请求可能进入 tmux 队列，失败只能保留未知状态；不再次创建 pane。
        let Ok(rows) = result else {
            return Ok(SplitOutcome::OutcomeUnknown);
        };
        if rows == [b"infinishell-selection-changed".to_vec()] {
            return Ok(SplitOutcome::SelectionChanged);
        }
        let Ok((pane_id, pane_pid, pane_tty)) = parse_pane(&rows) else {
            return Ok(SplitOutcome::OutcomeUnknown);
        };
        loop {
            // split 回执可能先于 forkpty 子进程完成 setsid 和 exec；只等待这一个已知新 pane。
            // 启动 cwd 由固定位置参数和上层 wrapper claim 保证，不要求 CLI 的实时 cwd 不变。
            if budget.check().is_err()
                || self.validate_processes(&budget).is_err()
                || state.control.validate(&self.server, &budget).is_err()
            {
                return Ok(SplitOutcome::OutcomeUnknown);
            }
            if let Ok(current) = self.snapshot_locked(&mut state, &budget) {
                if current.pane_id != pane_id
                    || current.pane_pid != pane_pid
                    || current.pane_tty != pane_tty
                {
                    return Ok(SplitOutcome::OutcomeUnknown);
                }
                // exec 会改变 macOS 的 pid_version；必须绑定已进入固定 shell 的身份，
                // 不能把尚在 tmux 子映像的临时生存期交给后续图片消费者。
                if current.process.snapshot().is_ok_and(|process| {
                    process.executable == shell && process.executable_file == shell_file
                }) {
                    return Ok(SplitOutcome::Started(current));
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn close(&self) -> io::Result<()> {
        self.state
            .lock()
            .map_err(|_| invalid("tmux 绑定锁已失效"))?
            .control
            .close()
    }
}

impl TmuxSnapshot {
    pub(crate) fn same_pane(&self, other: &Self) -> bool {
        self.client_pid == other.client_pid
            && self.server_pid == other.server_pid
            && self.session_id == other.session_id
            && self.window_id == other.window_id
            && self.pane_id == other.pane_id
            && self.pane_tty == other.pane_tty
            && self.client.same_identity(&other.client)
            && self.server.same_identity(&other.server)
            && self.process.same_identity(&other.process)
    }
}

fn same_image(first: &ProcessSnapshot, second: &ProcessSnapshot) -> bool {
    first.uid == second.uid
        && first.executable == second.executable
        && first.executable_file == second.executable_file
}

fn split_predicate(client_pid: u32) -> String {
    // L: 保留 -t 的 session/window/pane 上下文，但 client_session/client_flags 来自循环 client。
    // if-shell -F 插入的 split 在同一次 cmdq_next 内执行，不把选择核验交给另一条 socket。
    format!(
        concat!(
            "#{{&&:#{{window_active}},#{{pane_active}},",
            "#{{==:#{{L:#{{?#{{==:#{{client_pid}},{client_pid}}},",
            "#{{&&:#{{==:#{{client_session}},#{{session_name}}}},",
            "#{{m:*attached*,#{{client_flags}}}},#{{!:#{{m:*active-pane*,#{{client_flags}}}}}},",
            "#{{!:#{{m:*suspended*,#{{client_flags}}}}}},#{{!:#{{client_readonly}}}}}},}}}},1}}}}"
        ),
        client_pid = client_pid
    )
}

fn private_socket(path: &Path) -> io::Result<(u64, u64)> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("tmux socket 没有父目录"))?;
    let directory = fs::symlink_metadata(parent)?;
    let socket = fs::symlink_metadata(path)?;
    let canonical = fs::metadata(fs::canonicalize(parent)?)?;
    if (canonical.dev(), canonical.ino()) != (directory.dev(), directory.ino())
        || !directory.is_dir()
        || directory.uid() != unsafe { libc::geteuid() }
        || directory.mode() & 0o077 != 0
        || !socket.file_type().is_socket()
        || socket.uid() != directory.uid()
        || socket.nlink() != 1
    {
        return Err(invalid("tmux socket 或私有父目录无效"));
    }
    Ok((socket.dev(), socket.ino()))
}

struct ClientRow {
    client_pid: u32,
    client_tty: PathBuf,
    session_id: String,
    window_id: String,
    pane_id: String,
    pane_pid: u32,
    pane_tty: PathBuf,
}

fn parse_client(rows: &[Vec<u8>]) -> io::Result<ClientRow> {
    let fields = fields(rows, 8)?;
    let flags = fields[2].split(',').collect::<Vec<_>>();
    if !flags.contains(&"attached")
        || flags.contains(&"control-mode")
        || flags.contains(&"suspended")
        || flags.contains(&"read-only")
    {
        return Err(invalid("tmux 原 client 不是活动交互客户端"));
    }
    if flags.contains(&"active-pane") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "tmux 独立 client pane 尚无准确读取合同",
        ));
    }
    Ok(ClientRow {
        client_pid: positive_pid(fields[0])?,
        client_tty: tty_path(fields[1])?,
        session_id: identifier(fields[3], b'$')?,
        window_id: identifier(fields[4], b'@')?,
        pane_id: identifier(fields[5], b'%')?,
        pane_pid: positive_pid(fields[6])?,
        pane_tty: tty_path(fields[7])?,
    })
}

fn parse_pane(rows: &[Vec<u8>]) -> io::Result<(String, u32, PathBuf)> {
    let fields = fields(rows, 3)?;
    Ok((
        identifier(fields[0], b'%')?,
        positive_pid(fields[1])?,
        tty_path(fields[2])?,
    ))
}

fn fields(rows: &[Vec<u8>], count: usize) -> io::Result<Vec<&str>> {
    if rows.len() != 1 {
        return Err(invalid("tmux 回执对象不唯一"));
    }
    let text = std::str::from_utf8(&rows[0]).map_err(|_| invalid("tmux 回执编码无效"))?;
    let fields: Vec<_> = text.split('\t').collect();
    if fields.len() != count || fields.iter().any(|field| field.is_empty()) {
        return Err(invalid("tmux 回执字段数无效"));
    }
    Ok(fields)
}

fn identifier(value: &str, prefix: u8) -> io::Result<String> {
    let bytes = value.as_bytes();
    if bytes.first() != Some(&prefix)
        || bytes.len() <= 1
        || bytes.len() > 11
        || !bytes[1..].iter().all(u8::is_ascii_digit)
        || value[1..].parse::<u32>().is_err()
    {
        return Err(invalid("tmux 对象编号无效"));
    }
    Ok(value.to_owned())
}

fn positive_pid(value: &str) -> io::Result<u32> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("tmux PID 无效"));
    }
    let pid: u32 = value.parse().map_err(|_| invalid("tmux PID 越界"))?;
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(invalid("tmux PID 越界"));
    }
    Ok(pid)
}

fn tty_path(value: &str) -> io::Result<PathBuf> {
    if !value.starts_with("/dev/")
        || value.len() > 256
        || value.bytes().any(|byte| byte < 32 || byte == 127)
    {
        return Err(invalid("tmux TTY 路径无效"));
    }
    Ok(PathBuf::from(value))
}

#[cfg(test)]
#[path = "tmux_native_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "macos"))]
#[path = "tmux_native_live_tests.rs"]
mod live_tests;
