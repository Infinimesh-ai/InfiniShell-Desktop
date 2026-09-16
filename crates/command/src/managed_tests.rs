use super::*;
use crate::blocking::Command;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::Stdio;

const FIXTURE_ENV: &str = "INFINISHELL_COMMAND_MANAGED_FIXTURE";
const DIRECTORY_ENV: &str = "INFINISHELL_COMMAND_MANAGED_DIRECTORY";
const EMPTY_ROOT_ENV: &str = "INFINISHELL_COMMAND_MANAGED_EMPTY_ROOT";
const DRIVER: &str = "managed::tests::isolated_driver";
const ROOT: &str = "managed::tests::fixture_root";
const DESCENDANT: &str = "managed::tests::fixture_descendant";

fn command(name: &str, directory: &Path, managed: bool) -> Command {
    let executable = std::env::current_exe().unwrap();
    let mut command = if managed {
        Command::new_with_managed_process_group(executable)
    } else {
        Command::new(executable)
    };
    command
        .args(["--ignored", "--exact", name, "--nocapture"])
        .env(FIXTURE_ENV, "1")
        .env(DIRECTORY_ENV, directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

fn wait_until(predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut predicate = predicate;
    while !predicate() {
        assert!(Instant::now() < deadline, "进程夹具未及时就绪");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn managed_tree_reaps_descendants_and_never_signals_after_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = command(DRIVER, directory.path(), false).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "隔离监督验收失败: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("隔离监督验收超时");
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(directory.path().join("verified").exists());
}

#[test]
#[ignore = "只由受管理进程测试派生，隔离 Linux subreaper 的作用范围"]
fn isolated_driver() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let directory = std::path::PathBuf::from(std::env::var_os(DIRECTORY_ENV).unwrap());
    prepare_supervisor().unwrap();
    let mut root_command = command(ROOT, &directory, true);
    root_command.stdin(Stdio::piped());
    let mut tree = ManagedTree::claim(root_command.spawn().unwrap()).unwrap();
    tree.child_mut()
        .stdin
        .take()
        .unwrap()
        .write_all(b"1")
        .unwrap();
    wait_until(|| directory.join("heartbeat").exists());
    wait_until(|| tree.root_exited().unwrap());
    let status = tree.terminate_and_confirm(Duration::from_secs(5)).unwrap();
    assert!(status.success(), "根夹具应先自然退出");
    let before = fs::read(directory.join("heartbeat")).unwrap();
    thread::sleep(Duration::from_millis(150));
    assert_eq!(before, fs::read(directory.join("heartbeat")).unwrap());

    // 模拟回收后身份已指向新进程；第二次确认必须只读保存的状态，不再向 PID/组发信号。
    // 这里没有强制操作系统复用 PID，不把这个用例描述成真实 PID 分配器验收。
    let other = command(DESCENDANT, &directory, false).spawn().unwrap();
    let previous = std::mem::replace(&mut tree.child, other);
    drop(previous);
    assert_eq!(tree.terminate_and_confirm(Duration::ZERO).unwrap(), status);
    assert!(tree.child_mut().try_wait().unwrap().is_none());
    tree.child_mut().kill().unwrap();
    tree.child_mut().wait().unwrap();

    // 原生会话空闲结束时可能只剩未回收的根进程，不能靠 killpg(僵尸组)判断退出。
    let mut empty_command = command(ROOT, &directory, true);
    empty_command.env(EMPTY_ROOT_ENV, "1").stdin(Stdio::piped());
    let mut empty = ManagedTree::claim(empty_command.spawn().unwrap()).unwrap();
    empty
        .child_mut()
        .stdin
        .take()
        .unwrap()
        .write_all(b"1")
        .unwrap();
    wait_until(|| empty.root_exited().unwrap());
    assert!(
        empty
            .terminate_and_confirm(Duration::from_secs(5))
            .unwrap()
            .success()
    );
    fs::write(directory.join("verified"), b"verified").unwrap();
}

#[test]
#[ignore = "内部受管理根进程夹具"]
fn fixture_root() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let mut authorization = [0];
    std::io::stdin().read_exact(&mut authorization).unwrap();
    assert_eq!(authorization, [b'1']);
    if std::env::var_os(EMPTY_ROOT_ENV).is_some() {
        return;
    }
    let directory = std::path::PathBuf::from(std::env::var_os(DIRECTORY_ENV).unwrap());
    let mut command = command(DESCENDANT, &directory, false);
    #[cfg(windows)]
    command.inherit_managed_job();
    drop(command.spawn().unwrap());
    wait_until(|| directory.join("heartbeat").exists());
}

#[test]
#[ignore = "内部限时后代夹具"]
fn fixture_descendant() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let directory = std::path::PathBuf::from(std::env::var_os(DIRECTORY_ENV).unwrap());
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("heartbeat"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        file.write_all(b"x").unwrap();
        file.flush().unwrap();
        thread::sleep(Duration::from_millis(20));
    }
}
