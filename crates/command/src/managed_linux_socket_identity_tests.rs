use super::super::{LinuxProcessHandle, linux_receive_peer_handle, open_pidfd};
use super::WaitingChild;
use crate::blocking::Command;
use std::fs::File;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::mem;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Stdio;
use std::ptr;
use std::time::{Duration, Instant};

const SOCKET_FIXTURE_ENV: &str = "INFINISHELL_COMMAND_NATIVE_BRIDGE_SOCKET_FIXTURE";
const SOCKET_FIXTURE: &str =
    "managed::linux::tests::socket_identity::terminal_bridge_socket_fixture";

fn pair(pass_credentials: bool) -> (UnixStream, UnixStream) {
    let (client, server) = UnixStream::pair().unwrap();
    set_pass_credentials(&client, pass_credentials);
    (client, server)
}

fn set_pass_credentials(stream: &UnixStream, pass_credentials: bool) {
    let enabled = i32::from(pass_credentials);
    assert_eq!(
        unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const i32).cast(),
                mem::size_of::<i32>() as libc::socklen_t,
            )
        },
        0
    );
}

fn send_identity(stream: &UnixStream, marker: u8, descriptors: &[RawFd]) {
    let credentials = libc::ucred {
        pid: std::process::id() as i32,
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
    };
    let rights_bytes = mem::size_of_val(descriptors);
    let rights_space = if descriptors.is_empty() {
        0
    } else {
        (unsafe { libc::CMSG_SPACE(rights_bytes as u32) }) as usize
    };
    let credential_space =
        unsafe { libc::CMSG_SPACE(mem::size_of::<libc::ucred>() as u32) } as usize;
    let length = rights_space + credential_space;
    let mut control = vec![0usize; length.div_ceil(mem::size_of::<usize>())];
    let bytes = control.as_mut_ptr().cast::<u8>();
    unsafe {
        if !descriptors.is_empty() {
            let header = bytes.cast::<libc::cmsghdr>();
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(rights_bytes as u32) as usize;
            ptr::copy_nonoverlapping(
                descriptors.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(header),
                rights_bytes,
            );
        }
        let header = bytes.add(rights_space).cast::<libc::cmsghdr>();
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_CREDENTIALS;
        (*header).cmsg_len = libc::CMSG_LEN(mem::size_of::<libc::ucred>() as u32) as usize;
        ptr::write_unaligned(libc::CMSG_DATA(header).cast::<libc::ucred>(), credentials);
    }
    let mut vector = libc::iovec {
        iov_base: (&marker as *const u8).cast_mut().cast(),
        iov_len: 1,
    };
    let mut message = unsafe { mem::zeroed::<libc::msghdr>() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = bytes.cast();
    message.msg_controllen = length;
    assert_eq!(
        unsafe { libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL) },
        1,
        "身份夹具发送失败: {}",
        io::Error::last_os_error()
    );
}

fn receive(stream: &UnixStream) -> io::Result<LinuxProcessHandle> {
    linux_receive_peer_handle(stream, Instant::now() + Duration::from_secs(2))
}

#[test]
fn terminal_bridge_received_pidfd_survives_socket_close_without_pid_fallback() {
    let (mut client, mut server) = pair(true);
    let original = open_pidfd(std::process::id() as i32).unwrap();
    send_identity(&server, 1, &[original.as_raw_fd()]);
    server.write_all(b"response").unwrap();
    drop(original);
    drop(server);

    let peer = receive(&client).unwrap();
    let expected = LinuxProcessHandle::capture(std::process::id() as i32)
        .unwrap()
        .identity();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    drop(client);

    assert_eq!(response, b"response");
    assert_eq!(peer.snapshot().unwrap().identity, expected);
    assert_eq!(
        peer.executable_file().unwrap().metadata().unwrap().ino(),
        expected.executable_inode
    );
    let flags = unsafe { libc::fcntl(peer.descriptor.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0);
    assert_ne!(flags & libc::FD_CLOEXEC, 0);
}

#[test]
fn terminal_bridge_received_child_pidfd_expires_after_its_natural_exit() {
    let root = tempfile::Builder::new().prefix("pidfd-").tempdir().unwrap();
    let path = root.path().join("s");
    let mut child = WaitingChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", SOCKET_FIXTURE, "--nocapture"])
            .env(SOCKET_FIXTURE_ENV, &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "原生身份夹具未启动"
        );
        if line == "ready\n" {
            break;
        }
    }
    let stream = UnixStream::connect(path).unwrap();
    set_pass_credentials(&stream, true);
    child.0.stdin.as_mut().unwrap().write_all(b"\n").unwrap();
    let handle = receive(&stream).unwrap();
    assert_eq!(handle.identity().pid as u32, child.0.id());
    assert_eq!(handle.snapshot().unwrap().identity, handle.identity());
    let image = handle.executable_file().unwrap();

    child.0.stdin.take();
    assert!(child.0.wait().unwrap().success());
    assert!(image.metadata().unwrap().is_file());
    assert!(handle.exited().unwrap());
    assert!(handle.snapshot().is_err());
    assert!(handle.executable_file().is_err());
    assert!(handle.try_clone().is_err());
}

#[test]
#[ignore = "只由实际子进程身份回归派生；仅监听私有测试路径并等待 stdin"]
fn terminal_bridge_socket_fixture() {
    let path = std::env::var_os(SOCKET_FIXTURE_ENV).expect("缺少私有身份夹具路径");
    let listener = UnixListener::bind(path).unwrap();
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(b"\nready\n").unwrap();
    stdout.flush().unwrap();
    drop(stdout);
    let mut ready = [0u8; 1];
    std::io::stdin().read_exact(&mut ready).unwrap();
    assert_eq!(ready, [b'\n']);
    // 父先完成 connect 再授权；失败时 stdin EOF 能使夹具退出，不会留在 accept 中。
    let (stream, _address) = listener.accept().unwrap();
    let original = open_pidfd(std::process::id() as i32).unwrap();
    send_identity(&stream, 1, &[original.as_raw_fd()]);
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn terminal_bridge_another_live_process_pidfd_cannot_impersonate_socket_peer() {
    let mut child = WaitingChild(
        Command::new("sh")
            .args(["-c", "printf x; IFS= read -r line || :"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut ready = [0u8; 1];
    child
        .0
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .unwrap();
    assert_eq!(ready, [b'x']);
    let other = open_pidfd(child.0.id() as i32).unwrap();
    let (client, server) = pair(true);
    send_identity(&server, 1, &[other.as_raw_fd()]);

    assert!(receive(&client).is_err());
    assert!(child.0.try_wait().unwrap().is_none());
    child.0.stdin.take();
    assert!(child.0.wait().unwrap().success());
}

#[test]
fn terminal_bridge_regular_file_is_not_a_pidfd() {
    let file = tempfile::tempfile().unwrap();
    let (client, server) = pair(true);
    send_identity(&server, 1, &[file.as_raw_fd()]);

    assert!(receive(&client).is_err());
    assert!(file.metadata().unwrap().is_file());
}

#[test]
fn terminal_bridge_missing_pidfd_or_received_credentials_is_rejected() {
    let (client, server) = pair(true);
    send_identity(&server, 1, &[]);
    assert!(receive(&client).is_err());

    // 未开启 SO_PASSCRED 的连接收不到 ucred，不能因附带了合法 pidfd 就接受。
    let original = open_pidfd(std::process::id() as i32).unwrap();
    let (client, server) = pair(false);
    send_identity(&server, 1, &[original.as_raw_fd()]);
    assert!(receive(&client).is_err());
}

fn assert_extra_descriptors_closed(count: usize) {
    let mut pipe = [-1; 2];
    assert_eq!(
        unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) },
        0
    );
    let mut reader = unsafe { File::from_raw_fd(pipe[0]) };
    let writer = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
    let original = open_pidfd(std::process::id() as i32).unwrap();
    let mut descriptors = vec![writer.as_raw_fd(); count];
    descriptors.push(original.as_raw_fd());
    let (client, server) = pair(true);
    send_identity(&server, 1, &descriptors);
    drop(writer);

    assert!(receive(&client).is_err());
    // 接收方关闭全部已安装的副本，内核也关闭截断的副本，否则这里会返回 WouldBlock。
    assert_eq!(reader.read(&mut [0u8; 1]).unwrap(), 0);
}

#[test]
fn terminal_bridge_extra_descriptors_are_rejected_and_closed() {
    assert_extra_descriptors_closed(1);
}

#[test]
fn terminal_bridge_truncated_control_closes_every_received_descriptor() {
    assert_extra_descriptors_closed(96);
}

#[test]
fn terminal_bridge_wrong_marker_cannot_authorize_a_valid_pidfd() {
    let original = open_pidfd(std::process::id() as i32).unwrap();
    let (client, server) = pair(true);
    send_identity(&server, 2, &[original.as_raw_fd()]);

    assert!(receive(&client).is_err());
}

#[test]
fn terminal_bridge_eof_and_absolute_deadline_do_not_return_identity() {
    let (client, server) = pair(true);
    drop(server);
    assert!(receive(&client).is_err());

    let (client, _server) = pair(true);
    assert!(matches!(
        linux_receive_peer_handle(&client, Instant::now() + Duration::from_millis(30)),
        Err(error) if error.kind() == io::ErrorKind::TimedOut
    ));
    assert!(matches!(
        linux_receive_peer_handle(&client, Instant::now()),
        Err(error) if error.kind() == io::ErrorKind::TimedOut
    ));
}
