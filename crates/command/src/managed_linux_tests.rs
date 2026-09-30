use super::*;
use crate::blocking::Command;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::process::{Child, Stdio};

const EXEC_FIXTURE_ENV: &str = "INFINISHELL_COMMAND_NATIVE_BRIDGE_EXEC_FIXTURE";
const EXEC_FIXTURE: &str = "managed::linux::tests::terminal_bridge_exec_fixture";

// 夹具只等待 stdin；失败路径也先关闭管道，随后回收自己的子进程，不发送信号。
struct WaitingChild(Child);

impl Drop for WaitingChild {
    fn drop(&mut self) {
        self.0.stdin.take();
        let _ = self.0.wait();
    }
}

#[test]
fn terminal_bridge_retains_original_pidfd_and_actual_executable() {
    let original = LinuxProcessHandle::capture(std::process::id() as i32).unwrap();
    let retained = original.try_clone().unwrap();
    let mut executable = retained.executable_file().unwrap();
    let expected = original.identity();
    drop(original);

    assert_eq!(retained.snapshot().unwrap().identity, expected);
    let image = executable.metadata().unwrap();
    assert_eq!(image.dev(), expected.executable_device);
    assert_eq!(image.ino(), expected.executable_inode);
    let mut magic = [0u8; 4];
    executable.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"\x7fELF");
}

#[test]
fn terminal_bridge_cannot_open_an_executable_from_a_forged_snapshot() {
    let mut handle = LinuxProcessHandle::capture(std::process::id() as i32).unwrap();
    let original = handle.identity;
    handle.identity.executable_inode = original.executable_inode.wrapping_add(1);
    assert!(handle.executable_file().is_err());
    assert!(handle.try_clone().is_err());

    handle.identity = original;
    handle.identity.start_time_ticks = original.start_time_ticks.wrapping_add(1);
    assert!(handle.executable_file().is_err());

    handle.identity = original;
    handle.identity.proc_inode = original.proc_inode.wrapping_add(1);
    assert!(handle.executable_file().is_err());
}

#[test]
fn terminal_bridge_connected_pidfd_survives_socket_close_without_pid_fallback() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let peer = linux_peer_handle(&client).unwrap();
    let expected = LinuxProcessHandle::capture(std::process::id() as i32)
        .unwrap()
        .identity();
    server.write_all(b"response").unwrap();
    drop(server);
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    drop(client);

    assert_eq!(response, b"response");
    assert_eq!(peer.snapshot().unwrap().identity, expected);
    assert_eq!(
        peer.executable_file().unwrap().metadata().unwrap().ino(),
        expected.executable_inode
    );
}

#[test]
fn terminal_bridge_exited_process_cannot_reauthorize_its_retained_image() {
    let mut child = WaitingChild(
        Command::new("sh")
            .args(["-c", "printf x; IFS= read -r line || :"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
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
    let handle = LinuxProcessHandle::capture(child.0.id() as i32).unwrap();
    let image = handle.executable_file().unwrap();
    child.0.stdin.take();
    assert!(child.0.wait().unwrap().success());

    assert!(image.metadata().unwrap().is_file());
    assert!(handle.exited().unwrap());
    assert!(handle.executable_file().is_err());
    assert!(handle.try_clone().is_err());
}

#[test]
fn terminal_bridge_exec_rejects_old_image_even_with_the_same_process_lifetime() {
    let mut child = WaitingChild(
        Command::new("sh")
            .args([
                "-c",
                "printf 'before\\n'; IFS= read -r ready; exec \"$1\" --ignored --exact \"$2\" --nocapture",
                "native-bridge-fixture",
            ])
            .arg(std::env::current_exe().unwrap())
            .arg(EXEC_FIXTURE)
            .env(EXEC_FIXTURE_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(line, "before\n");
    let handle = LinuxProcessHandle::capture(child.0.id() as i32).unwrap();
    let image = handle.executable_file().unwrap();
    child.0.stdin.as_mut().unwrap().write_all(b"\n").unwrap();
    loop {
        line.clear();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "exec 夹具未确认新映像"
        );
        if line == "after\n" {
            break;
        }
    }
    let current = LinuxProcessHandle::capture(child.0.id() as i32).unwrap();

    assert!(handle.identity().same_lifetime(current.identity()));
    assert_ne!(
        current.identity().executable_inode,
        image.metadata().unwrap().ino()
    );
    assert!(handle.snapshot().is_err());
    assert!(handle.executable_file().is_err());
    assert!(handle.try_clone().is_err());
    child.0.stdin.take();
    assert!(child.0.wait().unwrap().success());
}

#[test]
#[ignore = "只由 exec 身份回归派生；不修改测试宿主进程"]
fn terminal_bridge_exec_fixture() {
    assert_eq!(std::env::var(EXEC_FIXTURE_ENV).unwrap(), "1");
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(b"\nafter\n").unwrap();
    stdout.flush().unwrap();
    drop(stdout);
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
}
