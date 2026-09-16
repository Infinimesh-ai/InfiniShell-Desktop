//! 无模型的真实监督进程验收；夹具进程限时退出，不能替代三款 CLI 的生命周期验收。

use std::process::Child as BlockingChild;

use command::blocking::Command as BlockingCommand;
use futures::executor::block_on;

use super::*;

const FIXTURE_ENV: &str = "INFINISHELL_MANAGED_PROCESS_FIXTURE";
const GENERATION_ENV: &str = "INFINISHELL_MANAGED_PROCESS_GENERATION";
const DETACH_ENV: &str = "INFINISHELL_MANAGED_PROCESS_DETACHED";
const HOST_TEST: &str = "ai::cli_agent_runtime::managed_process::live_tests::fixture_host";
const NATIVE_TEST: &str = "ai::cli_agent_runtime::managed_process::live_tests::fixture_native";
const NATIVE_EXIT_HOST_TEST: &str =
    "ai::cli_agent_runtime::managed_process::live_tests::fixture_native_exit_host";
const NATIVE_EXIT_TEST: &str =
    "ai::cli_agent_runtime::managed_process::live_tests::fixture_native_exit";
const DESCENDANT_TEST: &str =
    "ai::cli_agent_runtime::managed_process::live_tests::fixture_descendant";

struct Host(BlockingChild);

impl Drop for Host {
    fn drop(&mut self) {
        // 仅处理测试自己持有的宿主句柄；真正 CLI 的清理由独立监督者完成。
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture_command(name: &str, directory: &Path) -> BlockingCommand {
    let mut command = BlockingCommand::new(std::env::current_exe().unwrap());
    let fixture_name = name.rsplit("::").next().unwrap();
    let errors = fs::File::create(directory.join(format!("{fixture_name}.stderr"))).unwrap();
    command
        .args(["--ignored", "--exact", name, "--nocapture"])
        .current_dir(directory)
        .env(FIXTURE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(errors));
    command
}

fn wait_until(message: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !predicate() {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn start_host(directory: &Path, generation: Uuid, detached: bool) -> Host {
    supervisor_executable().expect("先构建主程序/TUI，并设置监督 worker 的绝对路径");
    let mut command = fixture_command(HOST_TEST, directory);
    command.env(GENERATION_ENV, generation.to_string());
    if detached {
        command.env(DETACH_ENV, "1");
    }
    let mut host = Host(command.spawn().unwrap());
    wait_until("监督宿主与根进程没有就绪", || {
        if let Some(status) = host.0.try_wait().unwrap() {
            let errors = fs::read_to_string(directory.join("fixture_host.stderr")).unwrap();
            panic!("监督宿主在就绪前退出（{status}）: {errors}");
        }
        directory.join("host-ready").exists() && directory.join("root-heartbeat").exists()
    });
    host
}

fn wait_diagnostic_receipt(directory: &Path, generation: Uuid) -> ExitReceipt {
    let path = generation_directory(directory, generation).join("exit.json");
    wait_until("未收到受管理进程退出诊断", || path.is_file());
    // 这里只读取隔离夹具的原始诊断，绝不能把文件存在当作允许恢复的确认。
    let receipt: ExitReceipt = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(receipt.generation, generation);
    receipt
}

fn assert_abnormal_exit_receipt_boundary(
    directory: &Path,
    generation: Uuid,
    receipt: &ExitReceipt,
) {
    assert_platform_containment(receipt);
    #[cfg(target_os = "macos")]
    {
        assert_ne!(receipt.exit_code, Some(0));
        assert!(!receipt.cleanup_confirmed);
        assert!(confirmed_exit(directory, generation).is_err());
    }
    #[cfg(any(target_os = "linux", windows))]
    {
        assert!(receipt.cleanup_confirmed);
        assert_eq!(
            confirmed_exit(directory, generation).unwrap(),
            Some(receipt.clone())
        );
    }
}

fn assert_stopped(path: &Path) {
    let before = fs::read(path).unwrap();
    thread::sleep(Duration::from_millis(250));
    assert_eq!(before, fs::read(path).unwrap(), "退出回执之后仍在写入");
}

fn assert_platform_containment(receipt: &ExitReceipt) {
    #[cfg(target_os = "linux")]
    assert_eq!(receipt.containment, "linux_subtree");
    #[cfg(target_os = "macos")]
    assert_eq!(receipt.containment, "unix_process_group");
    #[cfg(windows)]
    assert_eq!(receipt.containment, "windows_job");
}

#[test]
#[ignore = "需要已构建的真实监督 worker，不访问模型或用户配置"]
fn supervised_native_nonzero_exit_code_is_preserved() {
    supervisor_executable().expect("先构建主程序/TUI，并设置监督 worker 的绝对路径");
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let mut command = fixture_command(NATIVE_EXIT_HOST_TEST, directory.path());
    command.env(GENERATION_ENV, generation.to_string());
    let mut host = Host(command.spawn().unwrap());
    wait_until("非零退出宿主夹具未结束", || {
        host.0.try_wait().unwrap().is_some()
    });
    let errors =
        fs::read_to_string(directory.path().join("fixture_native_exit_host.stderr")).unwrap();
    assert!(
        host.0.wait().unwrap().success(),
        "非零退出验证失败: {errors}"
    );
    let receipt = wait_diagnostic_receipt(directory.path(), generation);
    assert_abnormal_exit_receipt_boundary(directory.path(), generation, &receipt);
    assert_eq!(receipt.exit_code, Some(37));
}

#[test]
#[ignore = "内部非零退出宿主夹具，只由 supervised 测试派生"]
fn fixture_native_exit_host() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let directory = std::env::current_dir().unwrap();
    let generation = std::env::var(GENERATION_ENV).unwrap().parse().unwrap();
    let arguments = ["--ignored", "--exact", NATIVE_EXIT_TEST, "--nocapture"].map(OsString::from);
    let child = block_on(spawn(
        &directory,
        generation,
        &std::env::current_exe().unwrap(),
        &arguments,
        &directory,
    ))
    .unwrap();
    // 先等原生退出诊断，不能由 finish 的停止请求把 37 改为强制终止码。
    let receipt = wait_diagnostic_receipt(&directory, generation);
    assert_eq!(receipt.exit_code, Some(37));
    assert_abnormal_exit_receipt_boundary(&directory, generation, &receipt);
    #[cfg(target_os = "macos")]
    assert!(block_on(child.finish()).is_err());
    #[cfg(any(target_os = "linux", windows))]
    assert_eq!(block_on(child.finish()).unwrap(), receipt);
}

#[test]
#[ignore = "内部原生非零退出夹具，只由监督 worker 派生"]
fn fixture_native_exit() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    std::process::exit(37);
}

#[test]
#[ignore = "需要已构建的真实监督 worker，不访问模型或用户配置"]
fn supervised_finish_stops_root_and_descendants_and_missing_receipt_blocks_resume() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let mut host = start_host(directory.path(), generation, false);
    wait_until("后代没有就绪", || {
        directory.path().join("child-heartbeat").exists()
    });
    fs::write(directory.path().join("finish-request"), b"finish").unwrap();
    wait_until("宿主 finish 没有返回", || {
        directory.path().join("host-finished").exists()
    });
    assert!(host.0.wait().unwrap().success());
    let receipt = wait_diagnostic_receipt(directory.path(), generation);
    assert_abnormal_exit_receipt_boundary(directory.path(), generation, &receipt);
    assert_stopped(&directory.path().join("root-heartbeat"));
    assert_stopped(&directory.path().join("child-heartbeat"));
    assert!(
        confirmed_exit(directory.path(), Uuid::new_v4())
            .unwrap()
            .is_none()
    );

    // 删除回执模拟掉盘/丢失；已有代次账本不能被未启动证明覆盖。
    fs::remove_file(generation_directory(directory.path(), generation).join("exit.json")).unwrap();
    assert!(
        confirmed_exit(directory.path(), generation)
            .unwrap()
            .is_none()
    );
    assert!(
        record_not_started(
            directory.path(),
            generation,
            &std::env::current_exe().unwrap(),
            &[],
            directory.path(),
        )
        .is_err()
    );
}

#[test]
#[ignore = "需要已构建的真实监督 worker，不访问模型或用户配置"]
fn supervised_host_sigkill_stops_root_and_descendants_before_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let mut host = start_host(directory.path(), generation, false);
    wait_until("后代没有就绪", || {
        directory.path().join("child-heartbeat").exists()
    });
    host.0.kill().unwrap();
    assert!(!host.0.wait().unwrap().success());
    let receipt = wait_diagnostic_receipt(directory.path(), generation);
    assert_abnormal_exit_receipt_boundary(directory.path(), generation, &receipt);
    assert!(matches!(
        receipt.exit_reason,
        ExitReason::HostDisconnected | ExitReason::StdioClosed
    ));
    assert_stopped(&directory.path().join("root-heartbeat"));
    assert_stopped(&directory.path().join("child-heartbeat"));
}

#[test]
#[ignore = "需要已构建的真实监督 worker；macOS 明确检验主动脱组的覆盖限制"]
fn supervised_detachment_obeys_platform_containment_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let mut host = start_host(directory.path(), generation, true);
    #[cfg(unix)]
    wait_until("脱组后代没有就绪", || {
        directory.path().join("child-heartbeat").exists()
    });
    #[cfg(windows)]
    wait_until("未记录严格 Job 拒绝 breakaway", || {
        directory.path().join("breakaway-rejected").exists()
    });
    host.0.kill().unwrap();
    let _ = host.0.wait().unwrap();
    let receipt = wait_diagnostic_receipt(directory.path(), generation);
    assert_abnormal_exit_receipt_boundary(directory.path(), generation, &receipt);
    assert_stopped(&directory.path().join("root-heartbeat"));
    #[cfg(target_os = "linux")]
    assert_stopped(&directory.path().join("child-heartbeat"));
    #[cfg(target_os = "macos")]
    {
        // 诊断不能确认整个任务退出；主动 setsid 后代仍写入，因此必须拒绝恢复。
        let path = directory.path().join("child-heartbeat");
        let before = fs::read(&path).unwrap();
        thread::sleep(Duration::from_millis(200));
        assert_ne!(before, fs::read(&path).unwrap());
        fs::write(directory.path().join("detached-stop"), b"stop").unwrap();
        wait_until("主动脱组的负向夹具未退出", || {
            directory.path().join("child-finished").exists()
        });
    }
    #[cfg(windows)]
    assert!(!directory.path().join("child-heartbeat").exists());
}

#[test]
#[ignore = "内部隔离宿主夹具，只由 supervised 测试派生"]
fn fixture_host() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let directory = std::env::current_dir().unwrap();
    let generation = std::env::var(GENERATION_ENV).unwrap().parse().unwrap();
    let executable = std::env::current_exe().unwrap();
    let arguments = ["--ignored", "--exact", NATIVE_TEST, "--nocapture"].map(OsString::from);
    let child = block_on(spawn(
        &directory,
        generation,
        &executable,
        &arguments,
        &directory,
    ))
    .unwrap();
    fs::write(directory.join("host-ready"), b"ready").unwrap();
    wait_until("宿主夹具没有收到正常结束指令", || {
        directory.join("finish-request").exists()
    });
    let finished = block_on(child.finish());
    let receipt = wait_diagnostic_receipt(&directory, generation);
    assert_abnormal_exit_receipt_boundary(&directory, generation, &receipt);
    #[cfg(target_os = "macos")]
    assert!(finished.is_err());
    #[cfg(any(target_os = "linux", windows))]
    assert_eq!(finished.unwrap(), receipt);
    fs::write(directory.join("host-finished"), b"finished").unwrap();
}

#[test]
#[ignore = "内部原生 CLI 替身，只由监督 worker 派生"]
fn fixture_native() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let directory = std::env::current_dir().unwrap();
    let detached = std::env::var_os(DETACH_ENV).is_some();
    let mut command = fixture_command(DESCENDANT_TEST, &directory);
    #[cfg(windows)]
    if !detached {
        command.inherit_managed_job();
    }
    match command.spawn() {
        Ok(child) => {
            #[cfg(windows)]
            assert!(!detached, "严格 Job 不得允许 CREATE_BREAKAWAY_FROM_JOB");
            drop(child);
        }
        Err(error) => {
            #[cfg(windows)]
            if detached {
                fs::write(directory.join("breakaway-rejected"), b"rejected").unwrap();
            } else {
                panic!("受管理后代启动失败: {error}");
            }
            #[cfg(unix)]
            panic!("后代启动失败: {error}");
        }
    }
    let _ = detached;
    heartbeat(&directory.join("root-heartbeat"), None);
}

#[test]
#[ignore = "内部后代夹具，只由原生 CLI 替身派生"]
fn fixture_descendant() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    #[cfg(unix)]
    if std::env::var_os(DETACH_ENV).is_some() {
        assert!(unsafe { libc::setsid() } >= 0, "负向夹具主动 setsid 失败");
    }
    let directory = std::env::current_dir().unwrap();
    heartbeat(
        &directory.join("child-heartbeat"),
        Some(&directory.join("detached-stop")),
    );
    fs::write(directory.join("child-finished"), b"finished").unwrap();
}

fn heartbeat(path: &Path, stop: Option<&Path>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    while Instant::now() < deadline && !stop.is_some_and(Path::exists) {
        file.write_all(b"x").unwrap();
        file.flush().unwrap();
        thread::sleep(Duration::from_millis(20));
    }
}
