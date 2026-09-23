use super::SuspendedChild;
use crate::blocking::Command;
use std::fs;
use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::process::Stdio;

const FIXTURE_ENV: &str = "INFINISHELL_COMMAND_SUSPENDED_FIXTURE";
const MARKER_ENV: &str = "INFINISHELL_COMMAND_SUSPENDED_MARKER";
const FIXTURE: &str = "windows::tests::suspended_fixture";

fn fixture_command(marker: &std::path::Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    // 测试进程保留测试运行器的 Job，不请求脱离宿主。
    command
        .inherit_managed_job()
        .args(["--ignored", "--exact", FIXTURE, "--nocapture"])
        .env(FIXTURE_ENV, "1")
        .env(MARKER_ENV, marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

#[test]
fn suspended_child_starts_only_after_native_resume() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("resumed");
    let suspended = fixture_command(&marker).spawn_suspended().unwrap();

    assert!(!marker.exists(), "恢复前子进程不得执行用户代码");
    let status = suspended.resume().unwrap().wait().unwrap();

    assert!(status.success());
    assert_eq!(fs::read(marker).unwrap(), b"resumed");
}

#[test]
fn dropping_suspended_child_terminates_and_waits() {
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("must-not-run");
    let suspended = fixture_command(&marker).spawn_suspended().unwrap();
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, suspended.id()) }.unwrap();
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };

    drop(suspended);

    let wait = unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 0) };
    assert_eq!(wait, WAIT_OBJECT_0, "Drop 返回前必须完成进程终止");
    assert!(!marker.exists(), "Drop 不得间接恢复子进程");
}

#[test]
fn thread_discovery_failure_returns_suspended_child_without_waiting() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("must-not-run");
    let mut suspended = fixture_command(&marker).spawn_suspended().unwrap();
    let child = suspended.child.take().unwrap();
    drop(suspended);

    let (failure, mut child) = SuspendedChild::from_child_with_primary_thread(
        child,
        Err(io::Error::other("注入线程发现失败")),
    )
    .unwrap_err();
    assert_eq!(failure.kind(), io::ErrorKind::Other);
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!marker.exists(), "线程发现失败后不得执行用户代码");
}

#[test]
fn spawn_suspended_failure_before_creation_has_no_child() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.exe");
    let (_, child) = Command::new(missing)
        .spawn_suspended_with_child_on_error()
        .unwrap_err();
    assert!(child.is_none());
}

#[test]
#[ignore = "只由 suspended spawn 测试派生"]
fn suspended_fixture() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let marker = std::env::var_os(MARKER_ENV).unwrap();
    fs::write(marker, b"resumed").unwrap();
}
