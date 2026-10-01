use super::*;

use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixListener;
use std::process::{Child, Stdio};

use command::blocking::Command;

fn response(inode: u32, cookie: u64, state: u8) -> Vec<u8> {
    let mut bytes = vec![0; NETLINK_HEADER + DIAG_HEADER];
    bytes[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    bytes[8..12].copy_from_slice(&17_u32.to_ne_bytes());
    bytes[12..16].copy_from_slice(&29_u32.to_ne_bytes());
    bytes[16] = libc::AF_UNIX as u8;
    bytes[17] = libc::SOCK_STREAM as u8;
    bytes[18] = state;
    bytes[20..24].copy_from_slice(&inode.to_ne_bytes());
    bytes[24..28].copy_from_slice(&(cookie as u32).to_ne_bytes());
    bytes[28..32].copy_from_slice(&((cookie >> 32) as u32).to_ne_bytes());
    attribute(&mut bytes, 7, &unsafe { libc::geteuid() }.to_ne_bytes());
    bytes
}

fn attribute(bytes: &mut Vec<u8>, kind: u16, value: &[u8]) {
    bytes.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
    bytes.extend_from_slice(&kind.to_ne_bytes());
    bytes.extend_from_slice(value);
    bytes.resize((bytes.len() + 3) & !3, 0);
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
}

#[test]
fn diag_rejects_truncation_sequence_cookie_and_duplicate_attributes() {
    let bytes = response(41, 53, TCP_ESTABLISHED);
    for length in 0..bytes.len() {
        assert!(parse_response(&bytes[..length], 41, Some(53), 17, 29).is_err());
    }
    assert!(parse_response(&bytes, 41, Some(54), 17, 29).is_err());
    assert!(parse_response(&bytes, 42, Some(53), 17, 29).is_err());
    assert!(parse_response(&bytes, 41, Some(53), 18, 29).is_err());
    assert!(parse_response(&bytes, 41, Some(53), 17, 30).is_err());
    let mut duplicate = bytes.clone();
    attribute(&mut duplicate, 7, &0_u32.to_ne_bytes());
    assert!(parse_response(&duplicate, 41, Some(53), 17, 29).is_err());
    let mut malformed = bytes;
    malformed[32..34].copy_from_slice(&3_u16.to_ne_bytes());
    assert!(parse_response(&malformed, 41, Some(53), 17, 29).is_err());
}

#[test]
fn diag_preserves_anonymous_peer_and_rejects_missing_uid() {
    let mut bytes = response(41, 53, TCP_ESTABLISHED);
    attribute(&mut bytes, 2, &61_u32.to_ne_bytes());
    let parsed = parse_response(&bytes, 41, None, 17, 29).unwrap().unwrap();
    assert_eq!(parsed.peer, Some(61));
    assert!(parsed.path.is_none());
    bytes.truncate(NETLINK_HEADER + DIAG_HEADER);
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    assert!(parse_response(&bytes, 41, None, 17, 29).is_err());
}

#[test]
fn diag_path_checks_distinguish_abstract_and_filesystem_names() {
    let mut abstract_name = response(41, 53, TCP_ESTABLISHED);
    attribute(&mut abstract_name, 0, b"\0private\0name");
    assert!(
        parse_response(&abstract_name, 41, None, 17, 29)
            .unwrap()
            .unwrap()
            .path
            .is_none()
    );
    for name in [b"relative\0".as_slice(), b"/tmp/a\0b\0", b""] {
        let mut bytes = response(41, 53, TCP_LISTEN);
        attribute(&mut bytes, 0, name);
        assert!(parse_response(&bytes, 41, None, 17, 29).is_err());
    }
    let mut bytes = response(41, 53, TCP_LISTEN);
    attribute(&mut bytes, 0, b"/tmp/tmux-1/default\0");
    assert_eq!(
        parse_response(&bytes, 41, None, 17, 29)
            .unwrap()
            .unwrap()
            .path,
        Some(PathBuf::from("/tmp/tmux-1/default"))
    );
}

#[test]
fn diag_pair_requires_same_user_exact_reverse_inode_and_distinct_cookie() {
    let mut left = response(41, 53, TCP_ESTABLISHED);
    let mut right = response(61, 71, TCP_ESTABLISHED);
    attribute(&mut left, 2, &61_u32.to_ne_bytes());
    attribute(&mut right, 2, &41_u32.to_ne_bytes());
    let left = parse_response(&left, 41, None, 17, 29).unwrap().unwrap();
    let mut right = parse_response(&right, 61, None, 17, 29).unwrap().unwrap();
    validate_pair(&left, &right).unwrap();
    right.uid ^= 1;
    assert!(validate_pair(&left, &right).is_err());
    right.uid = left.uid;
    right.peer = Some(42);
    assert!(validate_pair(&left, &right).is_err());
    right.peer = Some(41);
    right.cookie = left.cookie;
    assert!(validate_pair(&left, &right).is_err());
}

#[test]
fn socket_inode_rejects_partial_or_overflow_identity() {
    assert_eq!(socket_inode(b"socket:[42]").unwrap(), Some(42));
    assert_eq!(socket_inode(b"/dev/pts/3").unwrap(), None);
    for value in [
        b"socket:[0]".as_slice(),
        b"socket:[042]",
        b"socket:[4294967296]",
        b"socket:[42]extra",
    ] {
        assert!(socket_inode(value).is_err());
    }
}

#[test]
fn real_anonymous_socket_pair_has_exact_reverse_cookies() {
    let (left, right) = UnixStream::pair().unwrap();
    let process = Process::capture(std::process::id() as i32).unwrap();
    let sockets = process.sockets(&Budget::new()).unwrap();
    let left = sockets
        .iter()
        .find(|socket| socket.descriptor == left.as_raw_fd())
        .unwrap();
    let right = sockets
        .iter()
        .find(|socket| socket.descriptor == right.as_raw_fd())
        .unwrap();
    assert!(left.connects_to(right));
    assert!(right.connects_to(left));
    assert!(left.local_path.is_none());
    assert!(right.local_path.is_none());
}

#[test]
fn real_listener_vfs_rejects_replaced_name_and_retains_original_cookie() {
    let directory = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = directory.path().join("tmux");
    let listener = UnixListener::bind(&path).unwrap();
    let process = Process::capture(std::process::id() as i32).unwrap();
    let sockets = process.sockets(&Budget::new()).unwrap();
    let socket = sockets
        .iter()
        .find(|socket| socket.descriptor == listener.as_raw_fd())
        .unwrap();
    assert!(socket.listening);
    assert_eq!(socket.local_path.as_ref(), Some(&path));
    let inode = u32::try_from(socket.socket).unwrap();
    let cookie = socket.protocol;
    fs::remove_file(&path).unwrap();
    let replacement = UnixListener::bind(&path).unwrap();
    let mut diagnostics = Diagnostics::open().unwrap();
    let original = diagnostics
        .query(inode, Some(cookie), &Budget::new())
        .unwrap()
        .unwrap();
    assert!(validate_listener(&original).is_err());
    assert!(process.sockets(&Budget::new()).is_err());
    drop(replacement);
}

#[test]
fn real_stream_peer_retains_pidfd_or_reports_unsupported_kernel() {
    let (stream, other) = UnixStream::pair().unwrap();
    match peer(&stream) {
        Ok(process) => {
            assert_eq!(process.snapshot().unwrap().pid, std::process::id() as i32);
            let second = peer(&other).unwrap();
            assert!(process.same_identity(&second));
        }
        Err(error) => {
            // 独立 peer API 不能按裸 PID 降级；已有 server 的校验走 validate_peer。
            assert_eq!(error.raw_os_error(), Some(libc::ENOPROTOOPT));
        }
    }
}

#[test]
fn expired_budget_rejects_query_before_any_netlink_wait() {
    let budget = Budget {
        deadline: Instant::now(),
    };
    let mut diagnostics = Diagnostics::open().unwrap();
    assert_eq!(
        diagnostics.query(42, None, &budget).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
}

const PEER_FIXTURE_ROOT: &str = "INFINISHELL_TMUX_LINUX_PEER_FIXTURE_ROOT";

struct PeerChild {
    child: Child,
    root: tempfile::TempDir,
}

impl PeerChild {
    fn start() -> Self {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let module = module_path!().split_once("::").unwrap().1;
        let name = format!("{module}::peer_validation_child_fixture");
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", &name, "--nocapture"])
            .env(PEER_FIXTURE_ROOT, root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut fixture = Self { child, root };
        fixture.wait_file("ready");
        fixture
    }

    fn socket_path(&self) -> PathBuf {
        self.root.path().join("socket")
    }

    fn process(&self) -> Process {
        Process::capture(self.child.id() as i32).unwrap()
    }

    fn wait_file(&mut self, name: &str) {
        let budget = Budget::new();
        while !self.root.path().join(name).is_file() {
            budget.check().unwrap();
            assert!(self.child.try_wait().unwrap().is_none());
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn accept(&mut self) {
        self.child.stdin.as_mut().unwrap().write_all(b"a").unwrap();
        self.wait_file("accepted");
    }

    fn stop(&mut self) {
        self.child.stdin.take();
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for PeerChild {
    fn drop(&mut self) {
        self.child.stdin.take();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "仅由持有私有目录和管道的父测试启动"]
fn peer_validation_child_fixture() {
    let Some(root) = std::env::var_os(PEER_FIXTURE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    assert!(root.is_absolute());
    assert_eq!(root, fs::canonicalize(&root).unwrap());
    assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o700);
    let listener = UnixListener::bind(root.join("socket")).unwrap();
    let marker = |name| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join(name))
            .unwrap();
    };
    marker("ready");
    let mut input = std::io::stdin().lock();
    let mut command = [0];
    if input.read_exact(&mut command).is_err() {
        return;
    }
    assert_eq!(command, [b'a']);
    let (stream, _) = listener.accept().unwrap();
    marker("accepted");
    assert_eq!(input.read(&mut command).unwrap(), 0);
    drop(stream);
}

#[test]
fn known_server_peer_validation_uses_original_pidfd_and_real_new_connection() {
    let mut server = PeerChild::start();
    let expected = server.process();
    let stream = UnixStream::connect(server.socket_path()).unwrap();
    server.accept();
    let (inode, cookie) = stream_identity(&stream).unwrap();
    let mut diagnostics = Diagnostics::open().unwrap();
    let local = diagnostics
        .query(inode, Some(cookie), &Budget::new())
        .unwrap()
        .unwrap();
    assert_eq!(local.cookie, cookie);
    validate_peer(&stream, &expected, &Budget::new()).unwrap();
    // 相同 UID 的其他活进程也不能替代本次 stream 的实际 server。
    let wrong = Process::capture(std::process::id() as i32).unwrap();
    assert!(validate_peer(&stream, &wrong, &Budget::new()).is_err());
}

#[test]
fn unaccepted_peer_is_not_authorized_and_can_be_rechecked_after_accept() {
    let mut server = PeerChild::start();
    let expected = server.process();
    let stream = UnixStream::connect(server.socket_path()).unwrap();
    let (inode, cookie) = stream_identity(&stream).unwrap();
    let local = Diagnostics::open()
        .unwrap()
        .query(inode, Some(cookie), &Budget::new())
        .unwrap()
        .unwrap();
    assert!(local.peer.is_none());
    let budget = Budget {
        deadline: Instant::now() + Duration::from_millis(30),
    };
    assert_eq!(
        validate_peer(&stream, &expected, &budget)
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    server.accept();
    validate_peer(&stream, &expected, &Budget::new()).unwrap();
}

#[test]
fn original_pidfd_exit_cannot_authorize_a_replacement_server_connection() {
    let mut original = PeerChild::start();
    let expected = original.process();
    let stream = UnixStream::connect(original.socket_path()).unwrap();
    original.accept();
    validate_peer(&stream, &expected, &Budget::new()).unwrap();
    original.stop();
    assert!(expected.snapshot().is_err());
    let mut replacement = PeerChild::start();
    let new_stream = UnixStream::connect(replacement.socket_path()).unwrap();
    replacement.accept();
    assert!(validate_peer(&new_stream, &expected, &Budget::new()).is_err());
    validate_peer(&new_stream, &replacement.process(), &Budget::new()).unwrap();
}

#[test]
fn restored_listener_path_does_not_authorize_connection_to_another_server() {
    let mut original = PeerChild::start();
    let expected = original.process();
    let original_path = original.socket_path();
    let old_identity = fs::symlink_metadata(&original_path).unwrap().ino();
    let mut replacement = PeerChild::start();
    let replacement_path = replacement.socket_path();
    let saved = original.root.path().join("saved");
    fs::rename(&original_path, &saved).unwrap();
    fs::rename(&replacement_path, &original_path).unwrap();
    let stream = UnixStream::connect(&original_path).unwrap();
    replacement.accept();
    fs::rename(&original_path, &replacement_path).unwrap();
    fs::rename(&saved, &original_path).unwrap();
    assert_eq!(
        fs::symlink_metadata(&original_path).unwrap().ino(),
        old_identity
    );
    assert!(expected.snapshot().is_ok());
    assert!(validate_peer(&stream, &expected, &Budget::new()).is_err());
    validate_peer(&stream, &replacement.process(), &Budget::new()).unwrap();
    // 旧路径仍可建立正确连接，拒绝只针对连接实际落入的另一个 server。
    let real_stream = UnixStream::connect(&original_path).unwrap();
    original.accept();
    validate_peer(&real_stream, &expected, &Budget::new()).unwrap();
}

#[test]
fn old_connection_and_reused_inode_with_wrong_cookie_cannot_match_new_peer() {
    let (stream, peer_stream) = UnixStream::pair().unwrap();
    let (old_stream, old_peer) = UnixStream::pair().unwrap();
    let process = Process::capture(std::process::id() as i32).unwrap();
    let sockets = process.sockets(&Budget::new()).unwrap();
    let (inode, cookie) = stream_identity(&stream).unwrap();
    let mut diagnostics = Diagnostics::open().unwrap();
    let local = diagnostics
        .query(inode, Some(cookie), &Budget::new())
        .unwrap()
        .unwrap();
    let peer = diagnostics
        .query(local.peer.unwrap(), None, &Budget::new())
        .unwrap()
        .unwrap();
    let old: Vec<_> = sockets
        .iter()
        .filter(|socket| {
            socket.descriptor == old_stream.as_raw_fd() || socket.descriptor == old_peer.as_raw_fd()
        })
        .cloned()
        .collect();
    assert_eq!(old.len(), 2);
    // 即使所有端点属于同一个活进程，旧连接也不能证明本次新连接归属。
    assert!(reverse_endpoint(&local, &peer, &old).unwrap().is_none());
    let correct = sockets
        .iter()
        .find(|socket| socket.descriptor == peer_stream.as_raw_fd())
        .unwrap()
        .clone();
    assert!(
        reverse_endpoint(&local, &peer, std::slice::from_ref(&correct))
            .unwrap()
            .is_some()
    );
    let mut changed = correct.clone();
    changed.protocol ^= 1;
    assert!(
        reverse_endpoint(&local, &peer, &[changed])
            .unwrap()
            .is_none()
    );
    assert!(reverse_endpoint(&local, &peer, &[correct.clone(), correct]).is_err());
}
