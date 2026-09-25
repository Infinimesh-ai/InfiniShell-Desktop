//! 固定 Grok 的零模型监督清理验收；不替代完整适配器、权限策略或用户取消流程。

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::executor::block_on;
use futures::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::write_message;
use crate::ai::cli_agent_runtime::managed_process;

// 只复用稳定进程身份和退出观察；本验收不调用该模块的故障注入终止方法。
#[path = "codex_idle_crash_identity_tests.rs"]
mod identity;

const MANIFEST_ENV: &str = "INFINISHELL_GROK_SUPERVISED_EXIT_MANIFEST";
const MARKER: &str = ".infinishell-grok-supervised-exit";
const MARKER_BYTES: &[u8] = b"isolated Grok supervised exit verification v1\n";
const MISSING_SESSION: &str = "00000000-0000-4000-8000-000000000000";
const MAX_BYTES: u64 = 1024 * 1024;
const SETUP_PHASES: [&str; 7] = [
    "auth",
    "resolve_workspace",
    "folder_trust",
    "plugin_registry",
    "mcp_merge",
    "persistence_init",
    "spawn_session_actor",
];
const SCENARIOS: [(&str, bool, bool); 4] = [
    ("private_leader_finish", true, false),
    ("private_leader_stdin_eof", true, true),
    ("no_leader_finish", false, false),
    ("no_leader_stdin_eof", false, true),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    path: PathBuf,
    sha256: String,
    bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    schema: u32,
    root: PathBuf,
    source_commit: String,
    source_tree: String,
    test_binary: Binary,
    supervisor: Binary,
    grok: Binary,
    cli_version: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_digest(path: &Path) -> String {
    let mut file = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            return format!("{:x}", hash.finalize());
        }
        hash.update(&buffer[..count]);
    }
}

fn verify_binary(binary: &Binary) -> PathBuf {
    assert!(binary.path.is_absolute());
    let metadata = fs::symlink_metadata(&binary.path).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.len(), binary.bytes);
    assert_eq!(file_digest(&binary.path), binary.sha256);
    binary.path.canonicalize().unwrap()
}

fn private_directory(path: &Path) {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path).unwrap();
}

fn record(file: &mut File, value: Value) {
    serde_json::to_writer(&mut *file, &value).unwrap();
    writeln!(file).unwrap();
    file.flush().unwrap();
}

fn own_child(system: &mut System, parent: u32, executable: &Path, arguments: &[OsString]) -> u32 {
    let candidates: Vec<_> = system
        .processes()
        .iter()
        .filter(|(_, child)| child.parent() == Some(Pid::from_u32(parent)))
        .map(|(pid, _)| *pid)
        .collect();
    // 只读取自有链路下一层的参数，不采集其他应用的命令行。
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let matches: Vec<_> = candidates
        .iter()
        .filter_map(|pid| {
            let child = system.process(*pid)?;
            (child.exe()?.canonicalize().ok()? == executable
                && child.parent() == Some(Pid::from_u32(parent))
                && child.cmd().len() == arguments.len() + 1
                && child.cmd()[1..] == *arguments)
                .then_some(pid.as_u32())
        })
        .collect();
    assert_eq!(matches.len(), 1, "自有监督链必须精确匹配一个进程");
    matches[0]
}

fn native_processes(
    supervisor: &Path,
    executable: &Path,
    manifest: &Path,
    arguments: &[OsString],
    generation: Uuid,
    leader: bool,
) -> Vec<identity::BoundProcess> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let worker_args = [
        OsString::from("cli-agent-supervisor"),
        manifest.as_os_str().to_owned(),
    ];
    let worker = own_child(&mut system, std::process::id(), supervisor, &worker_args);
    #[cfg(not(target_os = "macos"))]
    let native_root = {
        #[cfg(windows)]
        let parent = own_child(
            &mut system,
            worker,
            supervisor,
            &[
                worker_args[0].clone(),
                worker_args[1].clone(),
                OsString::from("--execute"),
            ],
        );
        #[cfg(target_os = "linux")]
        let parent = worker;
        own_child(&mut system, parent, executable, arguments)
    };
    #[cfg(target_os = "macos")]
    let (native_root, resource_cid) = {
        use command::managed::{macos_boot_session, macos_process_identity};
        let directory = manifest.parent().unwrap();
        let claim_bytes = fs::read(directory.join("macos-coalition.json")).unwrap();
        let claim: Value = serde_json::from_slice(&claim_bytes).unwrap();
        let native: Value =
            serde_json::from_slice(&fs::read(directory.join("macos-native.json")).unwrap())
                .unwrap();
        assert_eq!(claim["generation"], json!(generation));
        assert_eq!(native["generation"], json!(generation));
        assert_eq!(claim["manifest_sha256"], file_digest(manifest));
        assert_eq!(native["claim_sha256"], digest(&claim_bytes));
        assert_eq!(claim["boot_session"], macos_boot_session().unwrap());
        let pid = u32::try_from(native["identity"]["pid"].as_u64().unwrap()).unwrap();
        let current = macos_process_identity(pid as i32).unwrap();
        assert_eq!(native["identity"]["unique_id"], current.unique_id);
        let cid = claim["wrapper"]["resource_cid"].as_u64().unwrap();
        assert_ne!(cid, 0);
        assert_eq!(current.resource_cid, cid);
        assert_ne!(
            macos_process_identity(worker as i32).unwrap().resource_cid,
            cid
        );
        (pid, cid)
    };
    assert_eq!(
        manifest
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap(),
        generation.to_string()
    );
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(native_root)]),
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    assert_eq!(
        &system.process(Pid::from_u32(native_root)).unwrap().cmd()[1..],
        arguments
    );
    let mut candidates = BTreeSet::from([Pid::from_u32(native_root)]);
    #[cfg(not(target_os = "macos"))]
    {
        let mut owned = BTreeSet::from([Pid::from_u32(worker)]);
        loop {
            let previous = owned.len();
            for (pid, process) in system.processes() {
                if process
                    .parent()
                    .is_some_and(|parent| owned.contains(&parent))
                {
                    owned.insert(*pid);
                }
            }
            if owned.len() == previous {
                break;
            }
        }
        candidates.extend(owned);
    }
    #[cfg(target_os = "macos")]
    {
        // 脱离父 PID 的 leader 仍必须属于本次已认证的 macOS 资源域。
        for pid in system.processes().keys() {
            if command::managed::macos_process_identity(pid.as_u32() as i32)
                .is_ok_and(|identity| identity.resource_cid == resource_cid)
            {
                candidates.insert(*pid);
            }
        }
    }
    let candidates: Vec<_> = candidates.into_iter().collect();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let native: Vec<_> = candidates
        .iter()
        .filter(|pid| {
            system
                .process(**pid)
                .and_then(|process| process.exe())
                .and_then(|path| path.canonicalize().ok())
                .is_some_and(|path| path == executable)
        })
        .map(|pid| pid.as_u32())
        .collect();
    assert!(native.contains(&native_root));
    assert!(
        native.len() >= if leader { 2 } else { 1 },
        "未观察到本次真实 Grok 根及私有 leader"
    );
    native
        .into_iter()
        .map(|pid| identity::BoundProcess::open(pid, executable))
        .collect()
}

async fn read_frame(stdout: &mut BufReader<command::r#async::ChildStdout>) -> Value {
    let mut bytes = Vec::new();
    let count = stdout
        .take(MAX_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .await
        .unwrap();
    assert!(count > 0 && count as u64 <= MAX_BYTES && bytes.last() == Some(&b'\n'));
    let frame: Value = serde_json::from_slice(&bytes).expect("原生 ACP 行必须是 JSON");
    assert_eq!(frame["jsonrpc"], "2.0");
    frame
}

async fn boundaries(
    stdin: &mut command::r#async::ChildStdin,
    stdout: &mut BufReader<command::r#async::ChildStdout>,
    project: &Path,
    leader: bool,
) -> Vec<String> {
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}});
    let new = json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":project,"mcpServers":[]}});
    let load = json!({"jsonrpc":"2.0","id":3,"method":"session/load","params":{"cwd":project,"mcpServers":[],"sessionId":MISSING_SESSION}});
    let resume = json!({"jsonrpc":"2.0","id":4,"method":"session/resume","params":{"cwd":project,"mcpServers":[],"sessionId":MISSING_SESSION}});
    let mut setup = Vec::new();
    let mut setup_session = None;
    // 自动私有 leader 在无认证时止于前五步；直连仍发送两条会话创建阶段通知。
    let expected_setup = &SETUP_PHASES[..if leader { 5 } else { 7 }];
    for request in [initialize, new, load, resume] {
        write_message(stdin, &request).await.unwrap();
        let response = async {
            let mut response = None;
            for _ in 0..128 {
                let frame = read_frame(stdout).await;
                if frame.get("id").is_some() {
                    assert!(response.is_none(), "原生请求不得出现重复响应");
                    assert_eq!(frame["id"], request["id"], "响应重复、过时或不属于当前请求");
                    assert!(frame.get("method").is_none(), "不得接受反向工具请求");
                    response = Some(frame);
                } else if frame["method"] == "_x.ai/session/setup" {
                    assert_eq!(request["id"], 2);
                    assert_eq!(frame.as_object().unwrap().len(), 3);
                    assert_eq!(frame["params"].as_object().unwrap().len(), 3);
                    assert_eq!(frame["params"]["method"], "session/new");
                    let phase = frame["params"]["phase"].as_str().unwrap().to_owned();
                    assert_eq!(expected_setup.get(setup.len()).copied(), Some(phase.as_str()), "setup 阶段缺失、重复、乱序或不属于当前拓扑");
                    if setup.len() < 5 {
                        assert!(frame["params"]["sessionId"].is_null());
                    } else {
                        let session = frame["params"]["sessionId"].as_str().unwrap();
                        Uuid::parse_str(session).unwrap();
                        if let Some(previous) = &setup_session { assert_eq!(previous, session); }
                        setup_session = Some(session.to_owned());
                    }
                    setup.push(phase);
                } else {
                    assert_eq!(frame, json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}));
                }
                // 直连的最后一条 setup 可晚于认证拒绝响应；两者都齐全后再继续。
                if response.is_some() && (request["id"] != 2 || setup.len() == expected_setup.len()) {
                    return response.unwrap();
                }
            }
            panic!("原生 ACP 帧数超限")
        }.with_timeout(Duration::from_secs(20)).await.expect("原生 ACP 响应超时");
        match request["id"].as_u64().unwrap() {
            1 => {
                assert!(response.get("error").is_none());
                let result = &response["result"];
                assert_eq!(result["protocolVersion"], 1);
                assert_eq!(result["_meta"]["agentVersion"], "1.0.41");
                assert_eq!(result["_meta"]["mcpServers"], json!([]));
                assert!(result["_meta"]["defaultAuthMethodId"].is_null());
                let methods: Vec<_> = result["authMethods"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["id"].as_str().unwrap())
                    .collect();
                assert_eq!(methods, ["grok.com"]);
            }
            2 => {
                assert!(response.get("result").is_none());
                assert_eq!(response["error"]["code"], -32000);
                assert_eq!(response["error"]["message"], "Authentication required");
                write_message(stdin, &json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":MISSING_SESSION}})).await.unwrap();
            }
            3 | 4 => {
                assert!(response.get("result").is_none());
                assert_eq!(response["error"]["code"], -32603);
                assert_eq!(response["error"]["data"]["code"], "FS_NOT_FOUND");
            }
            other => panic!("未授权请求编号 {other}"),
        }
    }
    assert_eq!(setup, expected_setup);
    setup
}

async fn scenario(
    inputs: &Inputs,
    file: &mut File,
    input_digest: &str,
    name: &str,
    leader: bool,
    eof: bool,
) {
    record(file, json!({"event":"case_started","scenario":name}));
    record(
        file,
        json!({"event":"case_stage","scenario":name,"stage":"startup"}),
    );
    let root = inputs.root.join(name);
    assert!(!root.exists());
    private_directory(&root);
    let generation = Uuid::new_v4();
    let state = root.join("state");
    private_directory(&state);
    let state = state.canonicalize().unwrap();
    // 遵循生产隔离状态域，避免 worker 在原生 CLI 启动前拒绝夹具目录。
    let home = state.join("grok-managed").join(generation.to_string());
    for relative in [
        "home/.grok",
        "home/.claude",
        "home/.codex",
        "home/AppData/Roaming",
        "home/AppData/Local",
        "home/.config",
        "home/.local/share",
        "home/.cache",
        "grok",
        "tmp",
    ] {
        private_directory(&home.join(relative));
    }
    let home = home.canonicalize().unwrap();
    let environment = managed_process::isolated_environment(&home);
    for (name, relative) in [("HOME", "home"), ("GROK_HOME", "grok")] {
        assert!(
            environment.contains(&(OsString::from(name), home.join(relative).into_os_string()))
        );
    }
    assert!(!home.join("grok/auth.json").exists());
    assert!(!home.join("home/.grok/auth.json").exists());
    let settings = home.join("grok/config.toml");
    fs::write(
        &settings,
        b"[cli]\nuse_leader = true\nauto_update = false\n",
    )
    .unwrap();
    let project = root.join("project");
    private_directory(&project);
    let project = project.canonicalize().unwrap();
    // 保持 Unix socket 短路径；目录仍为本测试独占且随本场景生命周期保留。
    let mut builder = tempfile::Builder::new();
    builder.prefix("isg-exit-");
    #[cfg(unix)]
    let socket = builder.tempdir_in("/tmp").unwrap();
    #[cfg(windows)]
    let socket = builder.tempdir_in(home.join("tmp")).unwrap();
    let arguments = if leader {
        vec![
            OsString::from("agent"),
            OsString::from("stdio"),
            OsString::from("--leader-socket"),
            socket
                .path()
                .canonicalize()
                .unwrap()
                .join("leader.sock")
                .into_os_string(),
        ]
    } else {
        vec![
            OsString::from("agent"),
            OsString::from("--no-leader"),
            OsString::from("stdio"),
        ]
    };
    // 入口核验三份完整摘要；运行器另在运行前后核验，避免每场景重读大型调试文件。
    let executable = &inputs.grok.path;
    let supervisor = &inputs.supervisor.path;
    let mut child = managed_process::spawn_with_isolated_home(
        &state,
        generation,
        executable,
        &arguments,
        &project,
        Some(&home),
        None,
    )
    .await
    .unwrap();
    let directory = state
        .join("cli-agent-processes")
        .join(generation.to_string());
    let manifest = directory.join("manifest.json");
    let manifest_hash = file_digest(&manifest);
    let manifest_value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(manifest_value["generation"], json!(generation));
    assert_eq!(manifest_value["launch_allowed"], true);
    assert_eq!(manifest_value["executable"], json!(executable));
    assert_eq!(manifest_value["isolated_home"], json!(home));
    assert_eq!(
        serde_json::from_value::<Vec<OsString>>(manifest_value["arguments"].clone()).unwrap(),
        arguments
    );
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    record(
        file,
        json!({"event":"case_stage","scenario":name,"stage":"acp"}),
    );
    let setup_phases = boundaries(&mut stdin, &mut stdout, &project, leader).await;
    record(
        file,
        json!({"event":"case_stage","scenario":name,"stage":"native_identity"}),
    );
    let native = native_processes(
        supervisor, executable, &manifest, &arguments, generation, leader,
    );
    assert!(
        managed_process::confirmed_exit(&state, generation)
            .unwrap()
            .is_none()
    );
    record(
        file,
        json!({"event":"case_stage","scenario":name,"stage":"finish"}),
    );
    drop(stdin);
    let finish = async move {
        if eof {
            child.finish_after_stdin_close().await
        } else {
            child.finish().await
        }
    };
    let drain = async move {
        let mut tail = Vec::new();
        stdout
            .take(MAX_BYTES + 1)
            .read_to_end(&mut tail)
            .await
            .unwrap();
        assert!(tail.len() as u64 <= MAX_BYTES, "退出尾部输出超限");
        tail.len()
    };
    let (receipt, drained) = futures::join!(finish, drain.with_timeout(Duration::from_secs(30)));
    let receipt = receipt.expect("生产监督清理必须返回可信回执");
    let drained = drained.expect("真实 stdout 必须有界关闭");
    record(
        file,
        json!({"event":"case_stage","scenario":name,"stage":"receipt"}),
    );
    assert_eq!(receipt.generation, generation);
    assert!(receipt.cleanup_confirmed);
    #[cfg(windows)]
    assert_eq!(receipt.containment, "windows_job");
    #[cfg(target_os = "linux")]
    assert_eq!(receipt.containment, "linux_subtree");
    #[cfg(target_os = "macos")]
    assert_eq!(receipt.containment, "macos_resource_coalition");
    assert_eq!(
        managed_process::confirmed_exit(&state, generation).unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(file_digest(&manifest), manifest_hash);
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["manifest_sha256"],
        manifest_hash
    );
    for process in &native {
        process.wait_for_exit(5000);
    }
    let old_generation = Uuid::new_v4();
    assert!(
        managed_process::confirmed_exit(&state, old_generation)
            .unwrap()
            .is_none()
    );
    let old_directory = state
        .join("cli-agent-processes")
        .join(old_generation.to_string());
    private_directory(&old_directory);
    fs::copy(&manifest, old_directory.join("manifest.json")).unwrap();
    fs::copy(directory.join("exit.json"), old_directory.join("exit.json")).unwrap();
    assert!(
        managed_process::confirmed_exit(&state, old_generation).is_err(),
        "旧代次不得接受另一个代次的真实回执"
    );
    for path in [
        home.join("grok/auth.json"),
        home.join("home/.grok/auth.json"),
    ] {
        assert!(!path.exists());
    }
    record(
        file,
        json!({"event":"case_passed","scenario":name,"cli_version":"1.0.41",
        "generation":generation,"inputs_manifest_sha256":input_digest,"manifest_sha256":manifest_hash,
        "exit_receipt_sha256":file_digest(&directory.join("exit.json")),"exit_receipt":receipt,
        "observed_native_processes":native.len(),"observed_native_processes_exited":true,
        "native_identity_mechanism":identity::MECHANISM,"stdout_eof_confirmed":true,"drained_tail_bytes":drained,
        "old_generation_rejected":true,"isolated_home":true,"credentials_provided":false,
        "setup_phases":setup_phases,
        "model_commands_sent":0,"authentication_commands_sent":0,"acp_messages_sent":5,
        "missing_cancel_ack_claimed":false,"full_adapter_lifecycle_claimed":false,"fixed_policy_permissions_claimed":false}),
    );
}

#[test]
#[ignore = "需要同提交监督 worker 和已核验 Grok 1.0.41；仅运行零模型 ACP 与真实进程清理"]
fn real_grok_1041_supervised_exit_without_credentials() {
    let path = PathBuf::from(std::env::var_os(MANIFEST_ENV).expect("请通过专用运行器提供隔离输入"));
    assert!(path.is_absolute());
    let bytes = fs::read(&path).unwrap();
    let inputs: Inputs = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(inputs.schema, 1);
    assert_eq!(inputs.cli_version, "1.0.41");
    assert_eq!(inputs.source_commit.len(), 40);
    assert_eq!(inputs.source_tree.len(), 40);
    assert_eq!(fs::read(inputs.root.join(MARKER)).unwrap(), MARKER_BYTES);
    assert_eq!(inputs.grok.path, verify_binary(&inputs.grok));
    assert_eq!(
        std::env::current_exe().unwrap().canonicalize().unwrap(),
        verify_binary(&inputs.test_binary)
    );
    assert_eq!(
        managed_process::supervisor_executable()
            .unwrap()
            .canonicalize()
            .unwrap(),
        verify_binary(&inputs.supervisor)
    );
    let mut events = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(inputs.root.join("events.private.ndjson"))
        .unwrap();
    block_on(async {
        for (name, leader, eof) in SCENARIOS {
            scenario(&inputs, &mut events, &digest(&bytes), name, leader, eof).await;
        }
    });
    assert_eq!(fs::read(path).unwrap(), bytes, "输入清单运行期间改变");
}
