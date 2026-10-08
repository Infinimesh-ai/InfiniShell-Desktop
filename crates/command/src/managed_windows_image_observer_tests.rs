use super::{Observer, image_file, observe_process_image, readonly_image, verify_event_process};
use crate::blocking::Command;
use crate::managed::WindowsProcessLease;
use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Debug::CREATE_PROCESS_DEBUG_INFO;
use windows::Win32::System::Threading::WaitForSingleObject;

const FIXTURE: &str = "managed_windows_image_observer::tests::image_observer_fixture";
const FIXTURE_ROLE: &str = "INFINISHELL_IMAGE_OBSERVER_FIXTURE_ROLE";
const FIXTURE_READY: &str = "INFINISHELL_IMAGE_OBSERVER_FIXTURE_READY";
const FIXTURE_TARGET: &str = "INFINISHELL_IMAGE_OBSERVER_FIXTURE_TARGET";

fn fixture_command(role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .inherit_managed_job()
        .args(["--ignored", "--exact", FIXTURE, "--nocapture"])
        .env(FIXTURE_ROLE, role)
        .env_remove(FIXTURE_READY)
        .env_remove(FIXTURE_TARGET)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

struct Target(Child);

impl Target {
    fn start(directory: &Path) -> Self {
        let ready = directory.join("ready");
        let mut command = fixture_command("target");
        command.env(FIXTURE_READY, &ready);
        let target = Self(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "测试目标未就绪");
            thread::sleep(Duration::from_millis(10));
        }
        target
    }

    fn lease(&self) -> WindowsProcessLease {
        WindowsProcessLease::from_process_handle(&self.0).unwrap()
    }

    fn finish(mut self) {
        drop(self.0.stdin.take());
        let wait = unsafe { WaitForSingleObject(HANDLE(self.0.as_raw_handle()), 5000) };
        assert_eq!(wait, WAIT_OBJECT_0, "目标应在 EOF 后恢复执行并退出");
        assert!(self.0.wait().unwrap().success());
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        // 仅回收本测试创建的目标；生产观察器没有对应的终止操作。
        let _ = self.0.kill();
        let _ = unsafe { WaitForSingleObject(HANDLE(self.0.as_raw_handle()), 5000) };
    }
}

#[test]
fn actual_image_is_returned_and_target_remains_running() {
    let directory = tempfile::tempdir().unwrap();
    let target = Target::start(directory.path());
    let held = target.lease();

    let mut image = observe_process_image(&held, fixture_command("bootstrap")).unwrap();
    image.seek(SeekFrom::Start(0)).unwrap();
    let mut actual = Vec::new();
    image.read_to_end(&mut actual).unwrap();

    assert_eq!(actual, fs::read(std::env::current_exe().unwrap()).unwrap());
    assert!(!held.exited().unwrap(), "观察成功不能结束用户目标");
    target.finish();
}

#[test]
fn failed_bootstrap_never_attaches_or_terminates_target() {
    let directory = tempfile::tempdir().unwrap();
    let target = Target::start(directory.path());
    let held = target.lease();
    let missing = Command::new(directory.path().join("missing-bootstrap.exe"));

    assert!(observe_process_image(&held, missing).is_err());
    assert!(!held.exited().unwrap());
    target.finish();
}

#[test]
fn observer_process_exit_after_attach_does_not_kill_target() {
    let directory = tempfile::tempdir().unwrap();
    let target = Target::start(directory.path());
    let held = target.lease();
    let mut command = fixture_command("observer-exit");
    command.env(FIXTURE_TARGET, target.0.id().to_string());
    let mut observer = Target(command.spawn().unwrap());

    let wait = unsafe { WaitForSingleObject(HANDLE(observer.0.as_raw_handle()), 15000) };
    assert_eq!(wait, WAIT_OBJECT_0);
    assert_eq!(observer.0.wait().unwrap().code(), Some(87));
    assert!(
        !held.exited().unwrap(),
        "未执行析构的观察器退出不得杀死目标"
    );
    target.finish();
}

#[test]
fn different_process_event_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let target = Target::start(directory.path());
    let held = target.lease();
    let current = WindowsProcessLease::capture(std::process::id()).unwrap();
    let event = CREATE_PROCESS_DEBUG_INFO {
        hProcess: HANDLE(current.as_raw_handle()),
        ..CREATE_PROCESS_DEBUG_INFO::default()
    };

    assert!(verify_event_process(&event, &held).is_err());
    assert!(!held.exited().unwrap());
    target.finish();
}

#[test]
fn absent_image_or_process_handle_is_not_a_pathname_fallback() {
    let current = WindowsProcessLease::capture(std::process::id()).unwrap();

    assert!(image_file(HANDLE::default()).is_none());
    assert!(image_file(HANDLE(-1isize as *mut std::ffi::c_void)).is_none());
    assert!(verify_event_process(&CREATE_PROCESS_DEBUG_INFO::default(), &current).is_err());
}

#[test]
fn returned_handle_keeps_the_file_object_without_write_access() {
    let mut original = tempfile::tempfile().unwrap();
    original.write_all(b"image").unwrap();
    let mut observed = readonly_image(original.try_clone().unwrap()).unwrap();
    original.seek(SeekFrom::Start(0)).unwrap();
    original.write_all(b"IMAGE").unwrap();

    observed.seek(SeekFrom::Start(0)).unwrap();
    let mut actual = String::new();
    observed.read_to_string(&mut actual).unwrap();

    assert_eq!(actual, "IMAGE");
    assert!(observed.write_all(b"forbidden").is_err());
}

#[test]
#[ignore = "仅由映像观察测试派生，模拟私有 helper 与普通 shell 目标"]
fn image_observer_fixture() {
    let Ok(role) = std::env::var(FIXTURE_ROLE) else {
        return;
    };
    match role.as_str() {
        "bootstrap" | "target" => {
            if let Some(ready) = std::env::var_os(FIXTURE_READY) {
                fs::write(ready, b"ready").unwrap();
            }
            std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink()).unwrap();
        }
        "observer-exit" => {
            let pid = std::env::var(FIXTURE_TARGET).unwrap().parse().unwrap();
            let target = WindowsProcessLease::capture(pid).unwrap();
            // 刻意跳过 Observer::drop，验证 OS 的 FALSE 策略，而非正常 detach 路径。
            let mut observer = Observer::start(fixture_command("bootstrap")).unwrap();
            observer.attach(target).unwrap();
            std::process::exit(87);
        }
        unexpected => panic!("未知映像观察夹具模式：{unexpected}"),
    }
}
