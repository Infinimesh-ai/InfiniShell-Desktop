#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::{Read as _, Write as _};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd as _, FromRawFd as _};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::{Arc, mpsc};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use command::blocking::Command;
#[cfg(unix)]
use nix::fcntl::{FcntlArg, OFlag, fcntl};
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, poll};
#[cfg(unix)]
use tempfile::tempdir;

use super::*;

#[test]
fn test_program_invalid_bash() {
    // This test assumes there is no bash binary at /some/weird/path/bash.
    let shell_path = "/some/weird/path/bash".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_invalid_zsh() {
    // This test assumes there is no bash zsh at /some/weird/path/bash.
    let shell_path = "/some/weird/path/zsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_program_unknown_shell() {
    let shell_path = "/some/weird/path/wtfsh".to_owned();
    assert!(supported_shell_path_and_type(&shell_path).is_none());
}

#[test]
fn test_trim_wsl_err_from_output() {
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n".to_vec()),
        b"/bin/bash\n".to_vec()
    );
    assert_eq!(
        take_until_utf16_crlf(b"/bin/bash\n\r\0\n\0W\0A\0R\0N\0I\0N\0G\0".to_vec()),
        b"/bin/bash\n".to_vec()
    );
}

#[test]
fn bash_spawn_command_pins_bash_variable_to_resolved_path() {
    let args = arguments_for_session_spawning_command(
        "/opt/homebrew/bin/bash",
        ShellType::Bash,
        123.into(),
    );
    let command = args[1].to_string_lossy();

    // rcfile 和变量赋值各有一层单引号，保留 $0 为 bash。
    assert!(
        command.contains(r#"BASH='"'"'/opt/homebrew/bin/bash'"'"';"#),
        "{command}"
    );
    assert!(command.starts_with("exec -a bash '/opt/homebrew/bin/bash' "));
}

#[cfg(unix)]
fn bash_variable_for_spawned_shell(shell_path: &Path) -> String {
    let args = arguments_for_session_spawning_command(
        &shell_path.to_string_lossy(),
        ShellType::Bash,
        123.into(),
    );
    let pty = nix::pty::openpty(None, None).unwrap();
    // SAFETY: openpty 返回的描述符尚未被其他对象持有。
    let (mut master, slave) =
        unsafe { (File::from_raw_fd(pty.master), File::from_raw_fd(pty.slave)) };
    fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
    let mut child = Command::new(shell_path)
        .args(args)
        .env("PATH", "/usr/bin:/bin")
        .env("HISTFILE", "/dev/null")
        .env("INPUTRC", "/dev/null")
        .env_remove("BASH_ENV")
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave)
        .spawn()
        .unwrap();
    // 八进制标记不会与 PTY 回显的命令文本混淆。
    let written = master.write_all(b"printf '\\074\\074%s\\076\\076' \"$BASH\"; exit\n");
    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let reader_cancelled = cancelled.clone();
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; 4096];
        while !reader_cancelled.load(Ordering::Relaxed) {
            let mut fds = [PollFd::new(master.as_raw_fd(), PollFlags::POLLIN)];
            if poll(&mut fds, 100).is_err() {
                break;
            }
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                // Linux 在 slave 关闭后返回 EIO，已读内容仍有效。
                Err(_) => break,
            }
        }
        let _ = tx.send(output);
    });
    let output = rx.recv_timeout(Duration::from_secs(30));
    cancelled.store(true, Ordering::Relaxed);
    let _ = child.kill();
    let _ = child.wait();
    reader.join().unwrap();
    written.expect("应能向 shell 写入探测命令");
    let output = output.expect("等待 shell 退出超时");
    let output = String::from_utf8_lossy(&output);
    let start = output.find("<<").expect("shell 未输出 BASH 路径") + 2;
    let end = output[start..].find(">>").unwrap() + start;
    output[start..end].to_owned()
}

#[cfg(unix)]
#[test]
fn spawned_bash_reports_the_executed_binary_with_spaces_and_quotes() {
    let system_bash = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .map(Path::new)
        .find(|path| path.exists())
        .expect("Unix shell 启动回归需要系统 Bash");
    let root = tempdir().unwrap();
    for dir_name in ["custom bash", "it's bash"] {
        let dir = root.path().join(dir_name);
        std::fs::create_dir(&dir).unwrap();
        let shell_path = dir.join("bash");
        std::os::unix::fs::symlink(system_bash, &shell_path).unwrap();

        assert_eq!(
            bash_variable_for_spawned_shell(&shell_path),
            shell_path.to_string_lossy()
        );
    }
}
