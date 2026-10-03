//! 固定 Mac 环境的真实产品 API 验收；不启动 CLI、模型或用户 shell 配置。

use super::{Budget, OuterTerminal, SplitOutcome, TmuxBinding, platform};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::fs::{
    FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use command::Stdio;
use command::blocking::Command;
use command::managed::{MacosProcessIdentity, macos_process_identity};
use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use warp_core::channel::ChannelState;
use warp_core::cli_agent_protocol::{WARP_CLI_AGENT_PROTOCOL_VERSION_ENV, WARP_CLIENT_VERSION_ENV};

use crate::terminal::cli_agent_sessions::event::current_protocol_version;

const FIXTURE_TEST: &str = "remote_server::tmux_native::live_tests::owned_pane_fixture";
const ROOT_ENV: &str = "INFINISHELL_TMUX_LIVE_ROOT";
const ROLE_ENV: &str = "INFINISHELL_TMUX_LIVE_ROLE";
const LITERAL_ENV: &str = "INFINISHELL_TMUX_LIVE_LITERAL";
const LITERAL: &str = "空格 ' \" ; $(touch forbidden) #{pane_id} \\";
const CHALLENGE: &[u8] = b"\x1b]9278;t;1;42;11111111-1111-4111-8111-111111111111;22222222-2222-4222-8222-222222222222\x07";
const WAIT: Duration = Duration::from_secs(15);

fn wait_for(mut condition: impl FnMut() -> io::Result<bool>) -> io::Result<()> {
    let deadline = Instant::now() + WAIT;
    loop {
        if condition()? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "自有 tmux 夹具未在期限内就绪或退出",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn record(path: &Path, value: &Value) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()
}

fn identity_value(identity: MacosProcessIdentity) -> Value {
    json!({"pid": identity.pid, "pid_version": identity.pid_version,
        "unique_id": identity.unique_id, "resource_cid": identity.resource_cid})
}

fn exited(identity: MacosProcessIdentity) -> io::Result<bool> {
    match macos_process_identity(identity.pid) {
        // 同一生存期内 exec 会改变 pid_version；不能据此冒称进程已退出。
        Ok(current) => Ok(current.unique_id != identity.unique_id),
        Err(_) if !platform::pids(None, &Budget::new())?.contains(&identity.pid) => Ok(true),
        Err(error) => Err(error),
    }
}

fn run(mut command: Command, expected_error: Option<&str>) -> io::Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let status = wait_for(|| Ok(child.try_wait()?.is_some()));
    if let Err(error) = status {
        // 这里只回收本次仍持有 Child 的元数据命令，不向 tmux server 或进程组发信号。
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let status = child.wait()?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .expect("标准输出管道")
        .take(8193)
        .read_to_end(&mut stdout)?;
    child
        .stderr
        .take()
        .expect("标准错误管道")
        .take(8193)
        .read_to_end(&mut stderr)?;
    let expected_status = match expected_error {
        None => status.success(),
        Some(message) => {
            status.code() == Some(1)
                && stdout.is_empty()
                && stderr == format!("{message}\n").as_bytes()
        }
    };
    if !expected_status || stdout.len() > 8192 || stderr.len() > 8192 {
        return Err(io::Error::other(format!(
            "自有 tmux 元数据命令失败：{status}，stderr={}",
            String::from_utf8_lossy(&stderr)
        )));
    }
    Ok(stdout)
}

struct LiveTmux {
    root: PathBuf,
    root_file: (u64, u64),
    cwd: PathBuf,
    tmux: PathBuf,
    socket: PathBuf,
    tty: PathBuf,
    outer: Child,
    stop_reader: Arc<AtomicBool>,
    challenges: Arc<AtomicUsize>,
    reader: Option<JoinHandle<io::Result<()>>>,
    identities: Vec<MacosProcessIdentity>,
    receipts: Vec<Value>,
    cleaned: bool,
}

impl LiveTmux {
    fn new() -> io::Result<Self> {
        let tmux = fs::canonicalize(
            std::env::var_os("INFINISHELL_TMUX_LIVE_EXECUTABLE")
                .ok_or_else(|| io::Error::other("真实验收必须提供固定 tmux 绝对路径"))?,
        )?;
        let expected_sha =
            std::env::var("INFINISHELL_TMUX_LIVE_SHA256").map_err(io::Error::other)?;
        let version = std::env::var("INFINISHELL_TMUX_LIVE_VERSION").map_err(io::Error::other)?;
        assert_eq!(hex::encode(Sha256::digest(fs::read(&tmux)?)), expected_sha);
        let mut command = Command::new(&tmux);
        command.arg("-V");
        assert_eq!(
            String::from_utf8(run(command, None)?)
                .map_err(io::Error::other)?
                .trim(),
            version
        );

        let temp = fs::canonicalize(
            std::env::var_os("TMPDIR")
                .ok_or_else(|| io::Error::other("真实验收必须使用外层门禁的短 TMPDIR"))?,
        )?;
        let root = tempfile::Builder::new()
            .prefix("tx-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(temp)?
            .keep();
        let metadata = fs::symlink_metadata(&root)?;
        let cwd = root.join("work 空格 '#{pane_id}");
        fs::create_dir(&cwd)?;
        let socket = root.join("socket");
        let config = root.join("config");
        fs::write(
            &config,
            b"set -g default-shell /bin/sh\nset -g status off\nset -s exit-empty on\n",
        )?;
        let pair = nix::pty::openpty(
            Some(&libc::winsize {
                ws_row: 40,
                ws_col: 120,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )?;
        let (master, slave) = unsafe {
            (
                File::from_raw_fd(pair.master),
                File::from_raw_fd(pair.slave),
            )
        };
        for file in [&master, &slave] {
            fcntl(file.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        }
        fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
        let tty = fs::canonicalize(nix::unistd::ttyname(slave.as_raw_fd())?)?;
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "\"$@\"; result=$?; exit \"$result\"",
                "infinishell-tmux-live",
            ])
            .arg(&tmux)
            .args(["-S"])
            .arg(&socket)
            .arg("-f")
            .arg(&config)
            .args(["new-session", "-s", "infinishell-live", "-c", "/"])
            .args([
                "/bin/sh",
                "-c",
                "cd -P \"$1\" || exit 125; shift; \"$@\"; result=$?; exit \"$result\"",
                "infinishell-live-pane",
            ])
            .arg(&cwd)
            .args(Self::pane_arguments(&root, "original")?)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            // 模拟从非宿主终端创建的旧 server；不依赖运行测试时的环境碰巧缺失。
            .env_remove(WARP_CLI_AGENT_PROTOCOL_VERSION_ENV)
            .env_remove(WARP_CLIENT_VERSION_ENV)
            .env_remove("ENV")
            .env_remove("BASH_ENV")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // 仅 fork 后安全系统调用；外层 shell 保留为 session leader，tmux 子进程共享其前台组。
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0
                    || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0
                    || libc::tcsetpgrp(0, libc::getpgrp()) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let outer = command.spawn()?;
        let stop_reader = Arc::new(AtomicBool::new(false));
        let challenges = Arc::new(AtomicUsize::new(0));
        let reader_stop = Arc::clone(&stop_reader);
        let reader_challenges = Arc::clone(&challenges);
        let reader = thread::spawn(move || drain(master, &reader_stop, &reader_challenges));
        Ok(Self {
            root,
            root_file: (metadata.dev(), metadata.ino()),
            cwd,
            tmux,
            socket,
            tty,
            outer,
            stop_reader,
            challenges,
            reader: Some(reader),
            identities: Vec::new(),
            receipts: Vec::new(),
            cleaned: false,
        })
    }

    fn pane_arguments(root: &Path, role: &str) -> io::Result<Vec<OsString>> {
        Ok(vec![
            fs::canonicalize("/usr/bin/env")?.into_os_string(),
            OsString::from(format!("{ROOT_ENV}={}", root.display())),
            OsString::from(format!("{ROLE_ENV}={role}")),
            OsString::from(format!("{LITERAL_ENV}={LITERAL}")),
            fs::canonicalize(std::env::current_exe()?)?.into_os_string(),
            OsString::from("--ignored"),
            OsString::from("--exact"),
            OsString::from(FIXTURE_TEST),
            OsString::from("--nocapture"),
        ])
    }

    fn command(&self, args: &[&OsStr]) -> io::Result<Vec<u8>> {
        let mut command = Command::new(&self.tmux);
        command.arg("-N").arg("-S").arg(&self.socket).args(args);
        run(command, None)
    }

    fn host_environment_absent(&self) -> io::Result<()> {
        // 每次只读两个公开能力键；不枚举或归档 server/session 的其他环境变量。
        for name in [WARP_CLI_AGENT_PROTOCOL_VERSION_ENV, WARP_CLIENT_VERSION_ENV] {
            for selector in [vec!["-g"], vec!["-t", "infinishell-live"]] {
                let mut command = Command::new(&self.tmux);
                command
                    .args(["-N", "-S"])
                    .arg(&self.socket)
                    .arg("show-environment")
                    .args(selector)
                    .arg(name);
                run(command, Some(&format!("unknown variable: {name}")))?;
            }
        }
        Ok(())
    }

    fn receipt(&mut self, role: &str) -> io::Result<Value> {
        let path = self.root.join(format!("{role}.json"));
        wait_for(|| Ok(path.exists()))?;
        let value: Value = serde_json::from_slice(&fs::read(path)?)?;
        self.receipts.push(safe_receipt(role, &value, &self.cwd));
        for field in ["pid", "parent", "server"] {
            let pid = i32::try_from(
                value[field]
                    .as_u64()
                    .ok_or_else(|| io::Error::other("夹具 PID 缺失"))?,
            )
            .map_err(io::Error::other)?;
            self.identities.push(macos_process_identity(pid)?);
        }
        if role == "original" {
            self.identities
                .push(macos_process_identity(self.outer.id() as i32)?);
            let outer = platform::Process::capture(self.outer.id() as i32)?.snapshot()?;
            for pid in platform::pids(Some(outer.foreground_group), &Budget::new())? {
                let process = platform::Process::capture(pid)?.snapshot()?;
                if process.executable == self.tmux && process.tty == outer.tty {
                    self.identities.push(macos_process_identity(pid)?);
                }
            }
        }
        Ok(value)
    }

    fn release(&mut self) -> io::Result<()> {
        fs::write(self.root.join("release"), [])?;
        wait_for(|| Ok(self.outer.try_wait()?.is_some()))?;
        if !self.outer.wait()?.success() {
            return Err(io::Error::other("自有外层 shell 非零退出，保留现场"));
        }
        for identity in &self.identities {
            wait_for(|| exited(*identity))?;
        }
        self.stop_reader.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| io::Error::other("PTY 排空线程失败"))??;
        }
        Ok(())
    }

    fn cleanup(&mut self) -> io::Result<()> {
        self.release()?;
        let current = fs::symlink_metadata(&self.root)?;
        if (current.dev(), current.ino()) != self.root_file
            || !current.is_dir()
            || current.file_type().is_symlink()
        {
            return Err(io::Error::other("夹具目录身份已改变，保留现场"));
        }
        // 只移除本测试固定创建的文件；未知条目使 remove_dir 失败并保留现场。
        for name in [
            "config",
            "socket",
            "original.json",
            "alternate.json",
            "owned.json",
            "release",
            "failure.safe.json",
        ] {
            let path = self.root.join(name);
            match fs::symlink_metadata(&path) {
                Ok(metadata)
                    if (metadata.is_file() || metadata.file_type().is_socket())
                        && metadata.uid() == unsafe { libc::geteuid() }
                        && metadata.nlink() == 1 =>
                {
                    fs::remove_file(path)?
                }
                Ok(_) => return Err(io::Error::other("夹具文件类型或归属改变，保留现场")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        fs::remove_dir(&self.cwd)?;
        fs::remove_dir(&self.root)?;
        self.cleaned = true;
        Ok(())
    }

    fn failure_receipts(&self) -> Vec<Value> {
        let mut receipts = self.receipts.clone();
        let Ok(root) = fs::symlink_metadata(&self.root) else {
            return receipts;
        };
        if !root.is_dir() || (root.dev(), root.ino()) != self.root_file {
            return receipts;
        }
        for name in [
            "original.json",
            "alternate.json",
            "owned.json",
            "original.pending",
            "alternate.pending",
            "owned.pending",
        ] {
            let path = self.root.join(name);
            let read = (|| -> io::Result<Value> {
                let mut file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&path)?;
                let metadata = file.metadata()?;
                if !metadata.is_file()
                    || metadata.len() > 8192
                    || metadata.nlink() != 1
                    || metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.mode() & 0o077 != 0
                {
                    return Err(io::Error::other("夹具回执身份或大小无效"));
                }
                let mut bytes = Vec::new();
                (&mut file).take(8193).read_to_end(&mut bytes)?;
                let current = fs::symlink_metadata(&path)?;
                if !current.is_file()
                    || bytes.len() > 8192
                    || (current.dev(), current.ino()) != (metadata.dev(), metadata.ino())
                {
                    return Err(io::Error::other("夹具回执在读取期间改变"));
                }
                let parsed = serde_json::from_slice(&bytes)
                    .ok()
                    .map(|value| safe_receipt(name, &value, &self.cwd));
                Ok(
                    json!({"file": name, "bytes": bytes.len(), "sha256": hex::encode(Sha256::digest(&bytes)), "receipt": parsed}),
                )
            })();
            match read {
                Ok(value) => receipts.push(value),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => receipts
                    .push(json!({"file": name, "read_error": format!("{:?}", error.kind())})),
            }
        }
        receipts
    }
}

fn safe_receipt(role: &str, value: &Value, cwd: &Path) -> Value {
    let mut fields = serde_json::Map::new();
    for key in [
        "pid",
        "parent",
        "server",
        "session",
        "group",
        "foreground_group",
        "parent_session",
        "parent_group",
        "parent_tty",
        "tty",
    ] {
        fields.insert(key.into(), json!(value[key].as_u64()));
    }
    for key in ["host_protocol", "host_version"] {
        fields.insert(key.into(), json!(value[key].as_str()));
    }
    fields.insert("role".into(), json!(role));
    fields.insert(
        "cwd_matches".into(),
        json!(value["cwd"].as_str() == cwd.to_str()),
    );
    fields.insert(
        "literal_matches".into(),
        json!(value["literal"].as_str() == Some(LITERAL)),
    );
    fields.insert(
        "devices".into(),
        json!(
            value["devices"]
                .as_array()
                .filter(|values| values.len() == 3)
                .map(|values| values.iter().map(Value::as_u64).collect::<Vec<_>>())
        ),
    );
    Value::Object(fields)
}

impl Drop for LiveTmux {
    fn drop(&mut self) {
        if !self.cleaned {
            // 失败只释放自有夹具，保留小型回执和目录给外层审计，不销毁失败的唯一证据。
            let released = self.release();
            let identities: Vec<_> = self
                .identities
                .iter()
                .copied()
                .map(identity_value)
                .collect();
            let audit = json!({
                    "cleanup_ready": false, "known_processes_released": released.is_ok(),
                    "root": self.root, "outer_pid": self.outer.id(), "identities": identities,
                "root_device": self.root_file.0, "root_inode": self.root_file.1,
                "error": released.err().map(|error| error.to_string()),
                "fixture_receipts": self.failure_receipts()
            });
            // 外层门禁可能随后清理短目录；先写 stderr 保留唯一小证据，绝不输出原始 PTY。
            let _ = writeln!(io::stderr().lock(), "tmux-native-live-failure {audit}");
            let _ = record(&self.root.join("failure.safe.json"), &audit);
            self.stop_reader.store(true, Ordering::Release);
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }
}

fn drain(mut master: File, stop: &AtomicBool, challenges: &AtomicUsize) -> io::Result<()> {
    let mut tail = Vec::new();
    let mut bytes = [0; 4096];
    while !stop.load(Ordering::Acquire) {
        match master.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                tail.extend_from_slice(&bytes[..count]);
                challenges.fetch_add(
                    tail.windows(CHALLENGE.len())
                        .filter(|frame| *frame == CHALLENGE)
                        .count(),
                    Ordering::AcqRel,
                );
                let keep = tail.len().saturating_sub(CHALLENGE.len() - 1);
                tail.drain(..keep);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) if error.raw_os_error() == Some(libc::EIO) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[test]
#[ignore = "仅由固定 Mac tmux 产品 API 验收 re-exec；独立运行不计能力通过"]
fn owned_pane_fixture() {
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).expect("夹具缺少专属根目录"));
    let role = std::env::var(ROLE_ENV).expect("夹具缺少角色");
    assert!(["original", "alternate", "owned"].contains(&role.as_str()));
    let process = platform::Process::capture(std::process::id() as i32)
        .unwrap()
        .snapshot()
        .unwrap();
    let parent = platform::Process::capture(process.parent)
        .unwrap()
        .snapshot()
        .unwrap();
    let mut devices = Vec::new();
    for descriptor in [0, 1, 2] {
        assert_eq!(unsafe { libc::isatty(descriptor) }, 1);
        let path = nix::unistd::ttyname(descriptor).unwrap();
        devices.push(fs::metadata(path).unwrap().rdev());
    }
    let pending = root.join(format!("{role}.pending"));
    record(&pending, &json!({"pid": process.pid, "parent": process.parent, "server": parent.parent, "session": process.session,
        "group": process.group, "foreground_group": process.foreground_group, "parent_session": parent.session,
        "parent_group": parent.group, "parent_tty": parent.tty, "tty": process.tty, "devices": devices,
        "cwd": process.cwd, "literal": std::env::var(LITERAL_ENV).unwrap(),
        "host_protocol": std::env::var(WARP_CLI_AGENT_PROTOCOL_VERSION_ENV).ok(),
        "host_version": std::env::var(WARP_CLIENT_VERSION_ENV).ok()})).unwrap();
    fs::rename(pending, root.join(format!("{role}.json"))).unwrap();
    // 正常路径由父验收释放；故障时也只让本夹具自行退出，不遗留永久 pane。
    let deadline = Instant::now() + Duration::from_secs(90);
    while !root.join("release").exists() {
        assert!(Instant::now() < deadline, "自有 pane 未收到释放标记");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "需要固定 Mac tmux 路径/SHA/版本及短 TMPDIR；必须显式执行，不把缺环境记为通过"]
fn real_tmux_product_api_preserves_pane_and_process_identity() {
    let mut fixture = LiveTmux::new().unwrap();
    let original_receipt = fixture.receipt("original").unwrap();
    assert_eq!(original_receipt["host_protocol"], Value::Null);
    assert_eq!(original_receipt["host_version"], Value::Null);
    fixture.host_environment_absent().unwrap();
    let outer = Arc::new(OuterTerminal::capture(fixture.outer.id(), &fixture.tty).unwrap());
    outer.write_challenge(CHALLENGE).unwrap();
    wait_for(|| Ok(fixture.challenges.load(Ordering::Acquire) == 1)).unwrap();
    let binding = TmuxBinding::discover(Arc::clone(&outer)).unwrap();
    let original = binding.snapshot().unwrap();
    assert_eq!(original.cwd, fixture.cwd);
    fixture.identities.extend([
        macos_process_identity(original.client_pid as i32).unwrap(),
        macos_process_identity(original.server_pid as i32).unwrap(),
    ]);

    let alternate = LiveTmux::pane_arguments(&fixture.root, "alternate").unwrap();
    let mut args = vec![
        OsString::from("split-window"),
        OsString::from("-t"),
        OsString::from(&original.pane_id),
        OsString::from("-c"),
        OsString::from("/"),
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from("cd -P \"$1\" || exit 125; shift; \"$@\"; result=$?; exit \"$result\""),
        OsString::from("infinishell-live-pane"),
        fixture.cwd.clone().into_os_string(),
    ];
    args.extend(alternate);
    fixture
        .command(&args.iter().map(OsString::as_os_str).collect::<Vec<_>>())
        .unwrap();
    let alternate_receipt = fixture.receipt("alternate").unwrap();
    assert_eq!(alternate_receipt["host_protocol"], Value::Null);
    assert_eq!(alternate_receipt["host_version"], Value::Null);
    let before = fixture
        .command(&[
            OsStr::new("list-panes"),
            OsStr::new("-a"),
            OsStr::new("-F"),
            OsStr::new("#{pane_id}"),
        ])
        .unwrap();
    let owned = LiveTmux::pane_arguments(&fixture.root, "owned").unwrap();
    let program = PathBuf::from(&owned[0]);
    assert!(binding.validate_pane(&original).is_err());
    assert!(
        binding
            .split_owned(&original, &program, &owned[1..], &fixture.cwd, || true)
            .is_err()
    );
    assert!(!fixture.root.join("owned.json").exists());
    assert_eq!(
        fixture
            .command(&[
                OsStr::new("list-panes"),
                OsStr::new("-a"),
                OsStr::new("-F"),
                OsStr::new("#{pane_id}")
            ])
            .unwrap(),
        before
    );

    let expected = binding.snapshot().unwrap();
    assert!(
        binding
            .split_owned(&expected, &program, &owned[1..], &fixture.cwd, || false)
            .is_err()
    );
    assert!(!fixture.root.join("owned.json").exists());
    assert_eq!(
        fixture
            .command(&[
                OsStr::new("list-panes"),
                OsStr::new("-a"),
                OsStr::new("-F"),
                OsStr::new("#{pane_id}")
            ])
            .unwrap(),
        before
    );
    let current = match binding
        .split_owned(&expected, &program, &owned[1..], &fixture.cwd, || true)
        .unwrap()
    {
        SplitOutcome::Started(snapshot) => snapshot,
        SplitOutcome::SelectionChanged => panic!("未改变选择却被拒绝"),
        SplitOutcome::OutcomeUnknown => panic!("真实 split 未取得确定回执，不能重投"),
    };
    let receipt = fixture.receipt("owned").unwrap();
    assert_eq!(
        receipt["host_protocol"],
        current_protocol_version().to_string()
    );
    assert_eq!(
        receipt["host_version"],
        ChannelState::app_version().unwrap_or("local")
    );
    fixture.host_environment_absent().unwrap();
    assert_eq!(receipt["literal"], LITERAL);
    assert_eq!(receipt["cwd"], fixture.cwd.to_str().unwrap());
    assert_eq!(receipt["parent"], current.pane_pid);
    assert_eq!(receipt["session"], current.pane_pid);
    assert_eq!(receipt["parent_session"], current.pane_pid);
    assert_eq!(receipt["group"], receipt["foreground_group"]);
    assert_eq!(receipt["group"], receipt["parent_group"]);
    assert_eq!(receipt["tty"], receipt["parent_tty"]);
    assert_eq!(
        receipt["devices"],
        json!([receipt["tty"], receipt["tty"], receipt["tty"]])
    );
    let observed = binding.snapshot().unwrap();
    assert!(
        observed.same_pane(&current),
        "启动回执与当前选择不一致：{}",
        json!({
            "selection_at_start": [current.session_id, current.window_id, current.pane_id],
            "selection_now": [observed.session_id, observed.window_id, observed.pane_id],
            "same_tty": current.pane_tty == observed.pane_tty,
            "same_client": current.client.same_identity(&observed.client),
            "same_server": current.server.same_identity(&observed.server),
            "pane_at_start": identity_value(current.process.identity_for_test()),
            "pane_now": identity_value(observed.process.identity_for_test()),
        })
    );
    let target = current.export_target().unwrap();
    let encoded_target = serde_json::to_vec(&target).unwrap();
    let restored_target: super::TmuxTarget = serde_json::from_slice(&encoded_target).unwrap();
    binding
        .snapshot()
        .unwrap()
        .validate_target(&restored_target)
        .unwrap();
    assert!(expected.validate_target(&restored_target).is_err());
    restored_target
        .validate_consumer(
            receipt["pid"].as_i64().unwrap() as i32,
            receipt["tty"].as_u64().unwrap(),
        )
        .unwrap();
    assert!(
        restored_target
            .matches_consumer(
                receipt["pid"].as_i64().unwrap() as i32,
                receipt["tty"].as_u64().unwrap()
            )
            .unwrap()
    );
    let after = fixture
        .command(&[
            OsStr::new("list-panes"),
            OsStr::new("-a"),
            OsStr::new("-F"),
            OsStr::new("#{pane_id}"),
        ])
        .unwrap();
    assert!(
        binding
            .split_owned(&current, &program, &owned[1..], &fixture.cwd, || true)
            .is_err()
    );
    assert_eq!(
        fixture
            .command(&[
                OsStr::new("list-panes"),
                OsStr::new("-a"),
                OsStr::new("-F"),
                OsStr::new("#{pane_id}")
            ])
            .unwrap(),
        after
    );

    binding.close().unwrap();
    let clients = fixture
        .command(&[
            OsStr::new("list-clients"),
            OsStr::new("-F"),
            OsStr::new("#{client_pid}"),
        ])
        .unwrap();
    assert_eq!(
        String::from_utf8(clients).unwrap().trim(),
        original.client_pid.to_string()
    );
    platform::Process::capture(receipt["pid"].as_i64().unwrap() as i32)
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(fixture.challenges.load(Ordering::Acquire), 1);
    fixture.cleanup().unwrap();
    // 外层 nextest 显式使用 --success-output immediate，将小型白名单证据归入门禁日志。
    eprintln!(
        "tmux-native-live {}",
        json!({
            "tmux": fixture.tmux, "original": original_receipt, "alternate": alternate_receipt,
            "owned": receipt, "identities": fixture.identities.iter().copied().map(identity_value).collect::<Vec<_>>(),
            "challenge_frames": 1, "old_pane_rejected": true, "cancelled_claim_rejected": true, "split_count": 1,
            "owned_host_environment_verified": true, "original_server_session_environment_unchanged": true,
            "only_own_control_closed": true, "native_release_completed": true, "cleanup_ready": true
        })
    );
}
