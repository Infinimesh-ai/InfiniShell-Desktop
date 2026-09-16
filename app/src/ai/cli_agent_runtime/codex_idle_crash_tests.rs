//! 真实 Codex 就绪后的空闲崩溃验收；不提交模型输入，不替代运行中工具树验收。

use std::ffi::OsString;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::path::Path;
use std::process;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(target_os = "macos")]
use command::managed::{MacosProcessIdentity, macos_boot_session, macos_process_identity};
#[cfg(target_os = "macos")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use sha2::{Digest as _, Sha256};

use super::*;

#[path = "codex_idle_crash_identity_tests.rs"]
mod identity;

#[derive(Clone, Debug, PartialEq)]
struct ProcessIdentity {
    pid: u32,
    parent: u32,
    start_time: u64,
    executable: PathBuf,
    arguments: Vec<OsString>,
    #[cfg(target_os = "macos")]
    kernel: MacosProcessIdentity,
    #[cfg(target_os = "macos")]
    coalition: Option<CoalitionBinding>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Serialize)]
struct CoalitionBinding {
    generation: Uuid,
    manifest_sha256: String,
    claim_sha256: String,
    native_sha256: String,
    boot_session: String,
    resource_cid: u64,
}

fn own_child(
    system: &mut System,
    parent: u32,
    executable: &Path,
    arguments: &[OsString],
) -> ProcessIdentity {
    let candidates: Vec<_> = system
        .processes()
        .iter()
        .filter(|(_, child)| child.parent() == Some(Pid::from_u32(parent)))
        .map(|(pid, _)| *pid)
        .collect();
    // 只读取自己的下一层子进程参数，不收集或输出其他用户进程的命令行。
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let mut matches = candidates.iter().filter_map(|pid| {
        let child = system.process(*pid)?;
        let path = child.exe()?.canonicalize().ok()?;
        let command = child.cmd();
        if path != executable
            || child.parent() != Some(Pid::from_u32(parent))
            || command.len() != arguments.len() + 1
            || &command[1..] != arguments
        {
            return None;
        }
        Some(ProcessIdentity {
            pid: pid.as_u32(),
            parent,
            start_time: child.start_time(),
            executable: path,
            arguments: command[1..].to_vec(),
            #[cfg(target_os = "macos")]
            kernel: macos_process_identity(pid.as_u32() as i32).unwrap(),
            #[cfg(target_os = "macos")]
            coalition: None,
        })
    });
    let child = matches
        .next()
        .expect("自己的监督链必须存在一个精确匹配的子进程");
    assert!(
        matches.next().is_none(),
        "监督链有多个匹配进程，拒绝发送信号"
    );
    child
}

#[cfg(not(target_os = "macos"))]
fn own_chain(supervisor: &Path, executable: &Path, manifest: &Path) -> Vec<ProcessIdentity> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let arguments = [
        OsString::from("cli-agent-supervisor"),
        manifest.as_os_str().to_owned(),
    ];
    let worker = own_child(&mut system, process::id(), supervisor, &arguments);
    let mut chain = vec![worker];
    #[cfg(windows)]
    {
        // Windows 的 execute worker 不会 exec 替换自身；真实 CLI 是它的子进程。
        let arguments = [
            arguments[0].clone(),
            arguments[1].clone(),
            OsString::from("--execute"),
        ];
        let executor = own_child(
            &mut system,
            chain.last().unwrap().pid,
            supervisor,
            &arguments,
        );
        chain.push(executor);
    }
    let native = own_child(
        &mut system,
        chain.last().unwrap().pid,
        executable,
        &[OsString::from("app-server"), OsString::from("--stdio")],
    );
    chain.push(native);
    chain
}

#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct SavedIdentity {
    pid: i32,
    pid_version: u32,
    unique_id: u64,
    resource_cid: u64,
}

#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct CoalitionClaim {
    version: u32,
    generation: Uuid,
    manifest_sha256: String,
    label: String,
    boot_session: String,
    wrapper: SavedIdentity,
}

#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct NativeClaim {
    generation: Uuid,
    claim_sha256: String,
    identity: SavedIdentity,
}

#[cfg(target_os = "macos")]
fn private_record(path: &Path) -> Vec<u8> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.mode() & 0o077, 0);
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert!(metadata.len() <= 64 * 1024);
    fs::read(path).unwrap()
}

#[cfg(target_os = "macos")]
fn assert_saved_identity(current: MacosProcessIdentity, saved: &SavedIdentity, may_exec: bool) {
    assert_eq!(current.pid, saved.pid);
    assert_eq!(current.unique_id, saved.unique_id, "PID 已被另一进程复用");
    assert_eq!(current.resource_cid, saved.resource_cid);
    assert!(saved.pid > 0 && saved.unique_id != 0 && saved.resource_cid != 0);
    if !may_exec {
        assert_eq!(current.pid_version, saved.pid_version);
    }
}

#[cfg(target_os = "macos")]
fn own_chain(supervisor: &Path, executable: &Path, manifest: &Path) -> Vec<ProcessIdentity> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let arguments = [
        OsString::from("cli-agent-supervisor"),
        manifest.as_os_str().to_owned(),
    ];
    let worker = own_child(&mut system, process::id(), supervisor, &arguments);
    let directory = manifest.parent().unwrap();
    let generation: Uuid = directory
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let manifest_bytes = private_record(manifest);
    let manifest_value: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest_value["generation"], json!(generation));
    assert_eq!(manifest_value["version"], 1);
    assert_eq!(manifest_value["launch_allowed"], true);
    assert_eq!(manifest_value["executable"], json!(executable));
    assert_eq!(
        serde_json::from_value::<Vec<OsString>>(manifest_value["arguments"].clone()).unwrap(),
        [OsString::from("app-server"), OsString::from("--stdio")]
    );
    let claim_bytes = private_record(&directory.join("macos-coalition.json"));
    let native_bytes = private_record(&directory.join("macos-native.json"));
    let claim: CoalitionClaim = serde_json::from_slice(&claim_bytes).unwrap();
    let native: NativeClaim = serde_json::from_slice(&native_bytes).unwrap();
    let binding = CoalitionBinding {
        generation,
        manifest_sha256: format!("{:x}", Sha256::digest(&manifest_bytes)),
        claim_sha256: format!("{:x}", Sha256::digest(&claim_bytes)),
        native_sha256: format!("{:x}", Sha256::digest(&native_bytes)),
        boot_session: macos_boot_session().unwrap(),
        resource_cid: claim.wrapper.resource_cid,
    };
    assert_eq!(claim.version, 1);
    assert_eq!(claim.generation, generation);
    assert_eq!(native.generation, generation);
    assert_eq!(
        claim.label,
        format!("dev.infinishell.cli-agent.{generation}")
    );
    assert_eq!(claim.manifest_sha256, binding.manifest_sha256);
    assert_eq!(native.claim_sha256, binding.claim_sha256);
    assert_eq!(claim.boot_session, binding.boot_session);
    assert_ne!(worker.kernel.resource_cid, binding.resource_cid);

    // claim 指向的 wrapper 由 launchd 管理；只读取这个已认证对象，不按名称搜索用户 job。
    let wrapper_pid = Pid::from_u32(u32::try_from(claim.wrapper.pid).unwrap());
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[wrapper_pid]),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let wrapper_process = system
        .process(wrapper_pid)
        .expect("专属 job wrapper 已退出");
    let wrapper_arguments = [
        arguments[0].clone(),
        arguments[1].clone(),
        OsString::from("--execute"),
    ];
    assert_eq!(
        wrapper_process.exe().unwrap().canonicalize().unwrap(),
        supervisor
    );
    assert_eq!(wrapper_process.cmd().len(), wrapper_arguments.len() + 1);
    assert_eq!(&wrapper_process.cmd()[1..], wrapper_arguments);
    let wrapper = ProcessIdentity {
        pid: wrapper_pid.as_u32(),
        parent: wrapper_process.parent().unwrap().as_u32(),
        start_time: wrapper_process.start_time(),
        executable: supervisor.to_owned(),
        arguments: wrapper_arguments.to_vec(),
        kernel: macos_process_identity(claim.wrapper.pid).unwrap(),
        coalition: Some(binding.clone()),
    };
    assert_saved_identity(wrapper.kernel, &claim.wrapper, false);
    let mut native_process = own_child(
        &mut system,
        wrapper.pid,
        executable,
        &[OsString::from("app-server"), OsString::from("--stdio")],
    );
    // exec 改变 PID version，但内核 unique ID 与首次独占 CID 必须保持。
    assert_saved_identity(native_process.kernel, &native.identity, true);
    assert_eq!(native_process.kernel.resource_cid, binding.resource_cid);
    native_process.coalition = Some(binding);
    vec![worker, wrapper, native_process]
}

#[cfg(target_os = "macos")]
#[test]
fn persisted_native_identity_allows_exec_but_rejects_pid_reuse_and_another_cid() {
    let saved = SavedIdentity {
        pid: 10,
        pid_version: 20,
        unique_id: 30,
        resource_cid: 40,
    };
    let current = MacosProcessIdentity {
        pid: 10,
        pid_version: 21,
        unique_id: 30,
        resource_cid: 40,
    };
    assert_saved_identity(current, &saved, true);
    for changed in [
        MacosProcessIdentity {
            unique_id: 31,
            ..current
        },
        MacosProcessIdentity {
            resource_cid: 41,
            ..current
        },
    ] {
        assert!(std::panic::catch_unwind(|| assert_saved_identity(changed, &saved, true)).is_err());
    }
    assert!(std::panic::catch_unwind(|| assert_saved_identity(current, &saved, false)).is_err());
}

fn record(evidence: Value) {
    let path = PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_ARTIFACT").unwrap());
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    writeln!(file, "{evidence}").unwrap();
    file.sync_all().unwrap();
}

#[tokio::test]
#[ignore = "需要无凭据 idle-crash 运行器、固定 Codex 与同提交真实监督 worker"]
async fn live_codex_idle_crash_after_ready_disconnects_once() {
    let root = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_IDLE_CRASH_ROOT")
            .expect("必须通过 idle-crash 隔离运行器执行"),
    )
    .canonicalize()
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-idle-crash-probe")).unwrap(),
        "isolated unauthenticated idle-crash verification\n"
    );
    let codex_home = PathBuf::from(env::var_os("CODEX_HOME").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(codex_home, root.join("codex"));
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        "cli_auth_credentials_store = \"file\"\n"
    );
    assert!(!codex_home.join("auth.json").exists());
    assert!(env::vars_os().all(|(key, _)| {
        let key = key.to_string_lossy().to_ascii_uppercase();
        !["TOKEN", "API_KEY", "AUTH", "SECRET"]
            .iter()
            .any(|part| key.contains(*part))
    }));
    for name in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
        assert!(
            PathBuf::from(env::var_os(name).unwrap())
                .canonicalize()
                .unwrap()
                .starts_with(&root)
        );
    }
    let executable = PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE").unwrap())
        .canonicalize()
        .unwrap();
    let supervisor = PathBuf::from(env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE").unwrap())
        .canonicalize()
        .unwrap();
    assert_ne!(
        supervisor,
        env::current_exe().unwrap().canonicalize().unwrap()
    );
    let directory = tempfile::Builder::new()
        .prefix("idle-crash-")
        .tempdir_in(&root)
        .unwrap();
    let generation = Uuid::new_v4();
    let mut settings = options();
    settings.executable = executable.clone();
    settings.cwd = directory.path().canonicalize().unwrap();
    settings.state_dir = settings.cwd.join("state");
    fs::create_dir(&settings.state_dir).unwrap();
    settings.generation = generation;
    settings.permission_policy = PermissionPolicy::Inherit;
    let state_dir = settings.state_dir.clone();
    let generation_dir = state_dir
        .join("cli-agent-processes")
        .join(generation.to_string());
    let manifest = generation_dir.join("manifest.json");
    let RuntimeConnection {
        controller,
        mut events,
        task,
    } = connect(settings).unwrap();
    let mut task = Some(task);
    let mut worker_binding = None;
    let checked = AssertUnwindSafe(async {
        // 保留 controller，整个测试不发送 Submit、Steer、审批或 Shutdown。
        let ready = tokio::select! {
            result = task.as_mut().unwrap() => panic!("原生会话就绪前结束：{result:?}"),
            event = events.recv() => event.expect("原生会话就绪前事件通道关闭"),
            _ = tokio::time::sleep(Duration::from_secs(40)) => panic!("真实 Codex 会话就绪超时"),
        };
        assert_eq!(ready.generation, generation);
        assert!(matches!(ready.kind, RuntimeEventKind::SessionReady { .. }));
        let native_id = ready.native_session_id.clone().expect("SessionReady 必须带真实原生 ID");
        assert!(!native_id.is_empty());
        record(json!({"event": "idle_crash_native_ready", "generation": generation,
            "native_session_id": native_id, "model_commands_sent": 0}));
        let chain = own_chain(&supervisor, &executable, &manifest);
        worker_binding = Some(identity::BoundProcess::open(chain[0].pid, &supervisor));
        let native = chain.last().unwrap();
        let bound = identity::BoundProcess::open(native.pid, &executable);
        // 先持有平台身份，再复核整条私有监督链；此后终止不再通过裸 PID 寻址。
        assert_eq!(own_chain(&supervisor, &executable, &manifest), chain,
            "绑定句柄期间监督链变化，拒绝发送信号");
        bound.assert_current(&executable);
        record(json!({"event": "idle_crash_target_bound", "generation": generation,
            "native_pid": native.pid, "native_parent_pid": native.parent,
            "supervisor_pid": chain[0].pid, "termination_mechanism": identity::MECHANISM}));
        bound.terminate_and_wait();
        record(json!({"event": "idle_crash_native_root_exited", "generation": generation,
            "native_pid": native.pid, "native_root_exit_observed": true}));
        let result = tokio::time::timeout(Duration::from_secs(30), task.as_mut().unwrap()).await
            .expect("原生 CLI 崩溃后适配器与监督收尾超时");
        task.take();
        record(json!({"event": "idle_crash_adapter_stopped", "generation": generation,
            "runtime_result_error": result.as_ref().err().map(ToString::to_string)}));
        let error = result.expect_err("空闲 CLI 崩溃不能成为成功结果");
        let disconnected = events.recv().await.expect("必须收到连接中断");
        assert_eq!(disconnected.generation, generation);
        assert_eq!(disconnected.native_session_id.as_deref(), Some(native_id.as_str()));
        assert_eq!(disconnected.kind, RuntimeEventKind::Disconnected { reason: error.to_string() });
        assert!(events.recv().await.is_none(), "必须只有一次 Ready 和一次 Disconnected，不能生成成功回合");
        let receipt: managed_process::ExitReceipt = serde_json::from_slice(
            &fs::read(generation_dir.join("exit.json")).expect("必须保存原生退出诊断")).unwrap();
        record(json!({"event": "idle_crash_exit_receipt", "generation": generation,
            "exit_receipt": receipt}));
        assert_eq!(receipt.generation, generation);
        #[cfg(unix)]
        assert_eq!(receipt.exit_code, None, "必须保留原生 SIGKILL 退出，不能写成正常退出");
        #[cfg(windows)]
        assert_eq!(receipt.exit_code, Some(73), "必须保留被终止的真实 Codex 退出码");
        #[cfg(target_os = "macos")]
        {
            assert_eq!(receipt.containment, "macos_resource_coalition");
            assert!(receipt.cleanup_confirmed);
            assert_eq!(managed_process::confirmed_exit(&state_dir, generation).unwrap(), Some(receipt.clone()));
            assert!(matches!(error, RuntimeError::Protocol(ref text)
                if text == "app-server stdout closed; delivery may be uncertain"));
        }
        #[cfg(any(target_os = "linux", windows))]
        {
            #[cfg(target_os = "linux")]
            assert_eq!(receipt.containment, "linux_subtree");
            #[cfg(windows)]
            assert_eq!(receipt.containment, "windows_job");
            assert!(receipt.cleanup_confirmed);
            assert_eq!(managed_process::confirmed_exit(&state_dir, generation).unwrap(), Some(receipt.clone()));
            assert!(matches!(error, RuntimeError::Protocol(ref text)
                if text == "app-server stdout closed; delivery may be uncertain"));
        }
        assert!(!codex_home.join("auth.json").exists());
        let processes: Vec<_> = chain.iter().enumerate().map(|(index, child)| {
            let value = json!({
                "depth": index, "pid": child.pid, "parent_pid": child.parent, "start_time": child.start_time,
                "is_native_cli": child.pid == native.pid,
            });
            #[cfg(target_os = "macos")]
            let value = {
                let mut value = value;
                value["kernel_identity"] = json!({"pid": child.kernel.pid, "pid_version": child.kernel.pid_version,
                    "unique_id": child.kernel.unique_id, "resource_cid": child.kernel.resource_cid});
                value["coalition_binding"] = json!(child.coalition);
                value
            };
            value
        }).collect();
        let evidence = json!({
            "event": "idle_crash_probe_finished", "passed": true,
            "phase": "after_session_ready", "generation": generation, "native_session_id": native_id,
            "credentials_provided": false, "model_commands_sent": 0,
            "runtime_events": [ready, disconnected], "processes": processes,
            "termination_mechanism": identity::MECHANISM, "native_root_exit_observed": true,
            "idle_process_cleanup_confirmed": receipt.cleanup_confirmed,
            "unsafe_recovery_prevented": !receipt.cleanup_confirmed,
            "before_ready_crash_verified": false, "running_tool_tree_cleanup_verified": false,
            "app_restart_and_ui_verified": false, "exit_receipt": receipt,
        });
        #[cfg(target_os = "macos")]
        let evidence = {
            let mut evidence = evidence;
            evidence["macos_coalition_ownership_verified"] = json!(true);
            evidence["macos_cleanup_proof_verified"] = json!(true);
            evidence
        };
        record(evidence);
    }).catch_unwind().await;
    // 断言失败或超时也先关闭生产控制连接，让监督者完成自己的限时清理。
    drop(task);
    drop(controller);
    if let Err(failure) = checked {
        if let Some(worker) = worker_binding {
            worker.wait_for_exit(30000);
        }
        resume_unwind(failure);
    }
}
