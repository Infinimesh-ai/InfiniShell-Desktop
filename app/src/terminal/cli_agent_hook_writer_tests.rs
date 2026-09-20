use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader};
use std::process::{Child, Stdio};

use command::blocking::Command;
use serde_json::{Value, json};

use super::*;

fn payload() -> Value {
    json!({"v":1,"agent":"grok","event":"cancelled","session_id":"session-1",
        "prompt_id":"prompt-1","event_id":format!("grok:{}", "a".repeat(64)),
        "plugin_version":"0.1.3"})
}

fn parsed(value: &Value) -> Notification {
    parse_notification(&serde_json::to_vec(value).unwrap()).unwrap()
}

#[test]
fn accepts_only_the_declared_grok_events_and_identity() {
    for event in [
        "session_start",
        "prompt_submit",
        "tool_complete",
        "stop",
        "stop_failure",
        "cancelled",
        "notification",
    ] {
        let mut value = payload();
        value["event"] = json!(event);
        assert!(parse_notification(&serde_json::to_vec(&value).unwrap()).is_ok());
    }
    for (key, value) in [
        ("v", json!(2)),
        ("agent", json!("claude")),
        ("event", json!("cancel")),
        ("event", json!("done")),
        ("event_id", json!(format!("grok:{}", "A".repeat(64)))),
        ("event_id", json!(format!("grok:{}", "a".repeat(63)))),
        ("plugin_version", json!("0.1.3-alpha")),
        ("terminal_unverified", json!("true")),
    ] {
        let mut candidate = payload();
        candidate[key] = value;
        assert_eq!(
            parse_notification(&serde_json::to_vec(&candidate).unwrap()).unwrap_err(),
            HookWriteError::InvalidPayload
        );
    }
}

#[test]
fn rejects_missing_duplicate_unknown_and_multiple_payloads() {
    for key in [
        "v",
        "agent",
        "event",
        "session_id",
        "event_id",
        "plugin_version",
    ] {
        let mut value = payload();
        value.as_object_mut().unwrap().remove(key);
        assert!(parse_notification(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for key in ["sequence", "turn_id", "tool_input", "unexpected"] {
        let mut value = payload();
        value[key] = json!(1);
        assert!(parse_notification(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let text = serde_json::to_string(&payload()).unwrap();
    let duplicate = text.replacen('{', "{\"v\":1,", 1);
    assert!(parse_notification(duplicate.as_bytes()).is_err());
    assert!(parse_notification(format!("{text}\n{text}").as_bytes()).is_err());
}

#[test]
fn identity_budget_never_truncates_or_admits_terminal_syntax() {
    for key in ["session_id", "prompt_id"] {
        let mut value = payload();
        value[key] = json!("a".repeat(MAX_ID_BYTES));
        assert_eq!(
            parsed(&value).session_id.len(),
            if key == "session_id" { MAX_ID_BYTES } else { 9 }
        );
        for invalid in [
            "".to_owned(),
            "a".repeat(MAX_ID_BYTES + 1),
            "会话".into(),
            "a/b".into(),
            "a;z".into(),
            "a\x1b]".into(),
        ] {
            value[key] = json!(invalid);
            assert_eq!(
                parse_notification(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
                HookWriteError::InvalidPayload
            );
        }
    }
}

#[derive(Default)]
struct ParsedOsc {
    values: Vec<Value>,
    invalid: usize,
    printed: String,
}

impl vte::Perform for ParsedOsc {
    fn print(&mut self, character: char) {
        self.printed.push(character);
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        if !bell_terminated
            || params.len() != 4
            || params[0] != b"777"
            || params[1] != b"notify"
            || params[2] != CLI_AGENT_NOTIFICATION_SENTINEL.as_bytes()
        {
            self.invalid += 1;
            return;
        }
        match serde_json::from_slice(params[3]) {
            Ok(value) => self.values.push(value),
            Err(_) => self.invalid += 1,
        }
    }
}

fn parse_osc(bytes: &[u8]) -> ParsedOsc {
    let mut parser = vte::Parser::new();
    let mut performer = ParsedOsc::default();
    for byte in bytes {
        parser.advance(&mut performer, *byte);
    }
    performer
}

#[test]
fn real_vte_preserves_utf8_and_many_semicolons_without_control_injection() {
    let mut value = payload();
    value["response"] = json!(format!(
        "中文 English\n\x1b]777;evil\x07\u{7f}\u{85}\u{9b}{}",
        ";".repeat(64)
    ));
    let frame = encode_frame(&parsed(&value), false).unwrap();
    let result = parse_osc(&frame);
    assert_eq!(result.values, vec![value]);
    assert_eq!(result.invalid, 0);
    assert!(result.printed.is_empty());
}

#[test]
fn final_frame_budget_includes_utf8_escapes_and_tmux_wrapper() {
    for tmux in [false, true] {
        let mut value = payload();
        value["summary"] = json!("");
        let base = encode_frame(&parsed(&value), tmux).unwrap().len();
        value["summary"] = json!("a".repeat(MAX_FRAME_BYTES - base));
        assert_eq!(
            encode_frame(&parsed(&value), tmux).unwrap().len(),
            MAX_FRAME_BYTES
        );
        value["summary"] = json!("a".repeat(MAX_FRAME_BYTES - base + 1));
        assert_eq!(
            encode_frame(&parsed(&value), tmux).unwrap_err(),
            HookWriteError::FrameTooLarge
        );
    }
    let mut value = payload();
    value["summary"] = json!(";\u{9b}".repeat(400));
    assert!(serde_json::to_vec(&value).unwrap().len() < MAX_FRAME_BYTES);
    assert_eq!(
        encode_frame(&parsed(&value), false).unwrap_err(),
        HookWriteError::FrameTooLarge
    );
    let notification = parsed(&payload());
    let plain = encode_frame(&notification, false).unwrap();
    let wrapped = encode_frame(&notification, true).unwrap();
    assert_eq!(wrapped.len(), plain.len() + 10);
    assert!(wrapped.starts_with(b"\x1bPtmux;\x1b\x1b]"));
    assert!(wrapped.ends_with(b"\x07\x1b\\"));
}

#[test]
fn stdin_budget_rejects_the_first_excess_byte() {
    assert_eq!(
        read_input(&vec![b' '; MAX_FRAME_BYTES][..]).unwrap().len(),
        MAX_FRAME_BYTES
    );
    assert_eq!(
        read_input(&vec![b' '; MAX_FRAME_BYTES + 1][..]).unwrap_err(),
        HookWriteError::FrameTooLarge
    );
    assert_eq!(
        parse_notification(&vec![b' '; MAX_FRAME_BYTES + 1]).unwrap_err(),
        HookWriteError::FrameTooLarge
    );
}

struct ShortWriter<W> {
    inner: W,
    limit: usize,
    errors: VecDeque<io::ErrorKind>,
}

impl<W: io::Write> io::Write for ShortWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.errors.pop_front() {
            return Err(kind.into());
        }
        self.inner.write(&bytes[..bytes.len().min(self.limit)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[test]
fn short_writes_and_transient_errors_preserve_every_byte_once() {
    let frame = encode_frame(&parsed(&payload()), false).unwrap();
    let mut writer = ShortWriter {
        inner: Vec::new(),
        limit: 17,
        errors: VecDeque::from([io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock]),
    };
    write_frame(&mut writer, &frame, IO_TIMEOUT).unwrap();
    assert_eq!(writer.inner, frame);
}

#[test]
fn zero_writes_hard_errors_and_expired_budget_are_failures() {
    let mut zero = ShortWriter {
        inner: Vec::new(),
        limit: 0,
        errors: VecDeque::new(),
    };
    assert_eq!(
        write_frame(&mut zero, b"abc", IO_TIMEOUT).unwrap_err(),
        HookWriteError::WriteFailed
    );
    let mut broken = ShortWriter {
        inner: Vec::new(),
        limit: 1,
        errors: VecDeque::from([io::ErrorKind::BrokenPipe]),
    };
    assert_eq!(
        write_frame(&mut broken, b"abc", IO_TIMEOUT).unwrap_err(),
        HookWriteError::WriteFailed
    );
    assert_eq!(
        write_frame(&mut Vec::new(), b"abc", Duration::ZERO).unwrap_err(),
        HookWriteError::WriteTimeout
    );
}

fn lock_directory(root: &tempfile::TempDir) -> PathBuf {
    root.path().canonicalize().unwrap().join("notify")
}

#[test]
fn failed_write_releases_os_lock_without_replacing_lock_file() {
    let root = tempfile::tempdir().unwrap();
    let directory = lock_directory(&root);
    let mut broken = ShortWriter {
        inner: Vec::new(),
        limit: 0,
        errors: VecDeque::new(),
    };
    assert_eq!(
        send_frame(&directory, &mut broken, b"frame").unwrap_err(),
        HookWriteError::WriteFailed
    );
    let before = fs::metadata(directory.join("transport.lock")).unwrap();
    let lock = open_lock(&directory).unwrap();
    acquire_lock(&lock, Duration::ZERO).unwrap();
    let contender = open_lock(&directory).unwrap();
    assert_eq!(
        acquire_lock(&contender, Duration::ZERO).unwrap_err(),
        HookWriteError::LockTimeout
    );
    drop(lock);
    acquire_lock(&contender, IO_TIMEOUT).unwrap();
    let after = fs::metadata(directory.join("transport.lock")).unwrap();
    assert_eq!(before.len(), 0);
    assert_eq!(after.len(), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    }
}

#[test]
fn nonempty_lock_is_rejected_without_deleting_its_contents() {
    let root = tempfile::tempdir().unwrap();
    let directory = lock_directory(&root);
    drop(open_lock(&directory).unwrap());
    let path = directory.join("transport.lock");
    fs::write(&path, b"foreign state").unwrap();
    assert_eq!(
        open_lock(&directory).unwrap_err(),
        HookWriteError::LockUnavailable
    );
    assert_eq!(fs::read(path).unwrap(), b"foreign state");
}

#[cfg(unix)]
#[test]
fn unix_lock_rejects_shared_permissions_symlinks_and_hardlinks() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let root = tempfile::tempdir().unwrap();
    let directory = lock_directory(&root);
    drop(open_lock(&directory).unwrap());
    let path = directory.join("transport.lock");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(open_lock(&directory).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(open_lock(&directory).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let alias = root.path().canonicalize().unwrap().join("alias");
    fs::hard_link(&path, &alias).unwrap();
    assert!(open_lock(&directory).is_err());
    fs::remove_file(&alias).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&alias, b"").unwrap();
    symlink(&alias, &path).unwrap();
    assert!(open_lock(&directory).is_err());
    fs::remove_file(&path).unwrap();
    fs::remove_dir(&directory).unwrap();
    let real = root.path().canonicalize().unwrap().join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, &directory).unwrap();
    assert!(open_lock(&directory).is_err());
    assert!(fs::read_dir(real).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn unix_terminal_rejects_non_tty_and_outside_device_paths() {
    let root = tempfile::tempdir().unwrap();
    let ordinary = root.path().join("not-a-tty");
    fs::write(&ordinary, b"unchanged").unwrap();
    for path in [
        ordinary.as_path(),
        Path::new("/dev/null"),
        Path::new("relative-tty"),
        Path::new("/dev/../dev/tty"),
    ] {
        assert!(open_unix_terminal(path).is_err());
    }
    assert_eq!(fs::read(ordinary).unwrap(), b"unchanged");
}

const CHILD_LOCK_ENV: &str = "INFINISHELL_HOOK_WRITER_TEST_LOCK";
const CHILD_TTY_ENV: &str = "INFINISHELL_HOOK_WRITER_TEST_TTY";
const CHILD_FILTER: &str = "terminal::cli_agent_hook_writer::tests::lock_holder_subprocess";

// 子测试仅在父测试的私有目录上运行，不能作为产品 worker 的可选模式。
#[test]
#[ignore = "只供持锁和 kill 回归通过精确过滤器调用"]
fn lock_holder_subprocess() {
    let Some(directory) = std::env::var_os(CHILD_LOCK_ENV) else {
        return;
    };
    let lock = open_lock(Path::new(&directory)).unwrap();
    acquire_lock(&lock, IO_TIMEOUT).unwrap();
    #[cfg(unix)]
    if let Some(tty) = std::env::var_os(CHILD_TTY_ENV) {
        let mut terminal = open_unix_terminal(Path::new(&tty)).unwrap();
        let frame = encode_frame(&parsed(&payload()), false).unwrap();
        write_frame(&mut terminal, &frame[..31], IO_TIMEOUT).unwrap();
    }
    println!("HOOK_LOCK_READY");
    io::stdout().flush().unwrap();
    let mut byte = [0];
    let _ = io::stdin().read(&mut byte);
    drop(lock);
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_lock_holder(directory: &Path, tty: Option<&Path>) -> ChildGuard {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", CHILD_FILTER, "--nocapture"])
        .env(CHILD_LOCK_ENV, directory)
        .env_remove(CHILD_TTY_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0);
    if let Some(tty) = tty {
        command.env(CHILD_TTY_ENV, tty);
    }
    let mut child = ChildGuard(command.spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut ready = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if line.contains("HOOK_LOCK_READY") {
                ready = true;
                break;
            }
        }
        let _ = sender.send(ready);
    });
    assert!(receiver.recv_timeout(Duration::from_secs(10)).unwrap());
    child
}

#[test]
fn killed_lock_owner_releases_same_persistent_os_lock() {
    let root = tempfile::tempdir().unwrap();
    let directory = lock_directory(&root);
    let mut child = start_lock_holder(&directory, None);
    let contender = open_lock(&directory).unwrap();
    assert_eq!(
        acquire_lock(&contender, Duration::ZERO).unwrap_err(),
        HookWriteError::LockTimeout
    );
    #[cfg(unix)]
    let before = {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = contender.metadata().unwrap();
        (metadata.dev(), metadata.ino())
    };
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    acquire_lock(&contender, IO_TIMEOUT).unwrap();
    // Windows 的独占字节锁会拒绝第二个句柄读取；用持锁句柄确认原文件仍为空。
    assert_eq!(contender.metadata().unwrap().len(), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = fs::metadata(directory.join("transport.lock")).unwrap();
        assert_eq!(before, (metadata.dev(), metadata.ino()));
    }
}

#[cfg(unix)]
fn private_pty() -> (File, File, PathBuf) {
    use std::ffi::CStr;
    use std::os::fd::{AsRawFd as _, FromRawFd as _};
    let mut master = -1;
    let mut slave = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    let mut name = [0; 256];
    assert_eq!(
        unsafe { libc::ttyname_r(slave.as_raw_fd(), name.as_mut_ptr(), name.len()) },
        0
    );
    let name = unsafe { CStr::from_ptr(name.as_ptr()) }.to_str().unwrap();
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    (master, slave, PathBuf::from(name))
}

#[cfg(unix)]
fn read_pty(mut master: File, length: usize) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    while bytes.len() < length && Instant::now() < deadline {
        match master.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("私有 PTY 读取失败: {error}"),
        }
    }
    bytes
}

#[cfg(unix)]
#[test]
fn concurrent_maximum_short_writes_arrive_as_complete_real_pty_frames() {
    use std::sync::{Arc, Barrier};
    let root = tempfile::tempdir().unwrap();
    let directory = lock_directory(&root);
    let (master, slave, path) = private_pty();
    let mut value = payload();
    value["summary"] = json!("");
    let base = encode_frame(&parsed(&value), false).unwrap().len();
    value["summary"] = json!("x".repeat(MAX_FRAME_BYTES - base));
    let first = encode_frame(&parsed(&value), false).unwrap();
    value["prompt_id"] = json!("prompt-2");
    let second = encode_frame(&parsed(&value), false).unwrap();
    let total = first.len() + second.len();
    let reader = thread::spawn(move || read_pty(master, total));
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = [first.clone(), second.clone()]
        .into_iter()
        .map(|frame| {
            let directory = directory.clone();
            let terminal = open_unix_terminal(&path).unwrap();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut writer = ShortWriter {
                    inner: terminal,
                    limit: 17,
                    errors: VecDeque::new(),
                };
                barrier.wait();
                send_frame(&directory, &mut writer, &frame)
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap().unwrap();
    }
    let bytes = reader.join().unwrap();
    assert!(bytes == [first.clone(), second.clone()].concat() || bytes == [second, first].concat());
    let result = parse_osc(&bytes);
    assert_eq!(result.values.len(), 2);
    assert_eq!(result.invalid, 0);
    assert!(result.printed.is_empty());
    drop(slave);
}

#[cfg(unix)]
#[test]
fn killed_partial_pty_frame_resynchronizes_at_the_next_complete_notification() {
    let root = tempfile::tempdir().unwrap();
    let directory = lock_directory(&root);
    let (master, slave, path) = private_pty();
    let mut child = start_lock_holder(&directory, Some(&path));
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let mut value = payload();
    value["prompt_id"] = json!("prompt-2");
    let frame = encode_frame(&parsed(&value), false).unwrap();
    let expected = 31 + frame.len();
    let mut terminal = open_unix_terminal(&path).unwrap();
    send_frame(&directory, &mut terminal, &frame).unwrap();
    let bytes = read_pty(master, expected);
    assert_eq!(bytes.len(), expected);
    let result = parse_osc(&bytes);
    assert_eq!(result.values, vec![value]);
    assert_eq!(result.invalid, 1);
    assert!(result.printed.is_empty());
    drop(slave);
}
