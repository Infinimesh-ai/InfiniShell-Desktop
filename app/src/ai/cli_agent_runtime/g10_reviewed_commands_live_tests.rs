//! 固定 Claude 父子受审命令的最小真实链；不覆盖 GUI、恢复或其它命令族。

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
use serde_json::Value;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};
use watcher::HomeDirectoryWatcher;

use super::*;
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::managed_process::{
    PreparedLaunchBinding, confirmed_exit_with_binding,
};
use crate::ai::cli_agent_runtime::permissions::ceiling_from_parent;
use crate::ai::cli_agent_runtime::reviewed_project_commands::{
    ReviewedCommand, ReviewedCommandCeilingV1,
};
use crate::ai::cli_agent_runtime::reviewed_project_commands_windows::CLAUDE_TOOL;
use crate::ai::cli_agent_runtime::runtime_host::{NativeProcessCompletion, confirmed_exit};
use crate::ai::cli_agent_runtime::{ApprovalDecision, current_state_dir};
use crate::ai::skills::SkillManager;
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

const SCOPE: &str = "real_claude_g10_reviewed_commands_parent_child_v1";
const MODEL: &str = "claude-opus-5-5";
const CHILD_DONE: &str = "G10_COMMAND_CHILD_DONE";
const PARENT_DONE: &str = "G10_COMMAND_PARENT_COLLECTED";
type Wake = Option<(LocalCliTask, RuntimeEvent)>;

struct Evidence(File);

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        // 调用点仅传本轮身份、固定状态和摘要，不写模型正文、配置或原生历史。
        writeln!(self.0, "{value}").map_err(|error| error.to_string())?;
        self.0.flush().map_err(|error| error.to_string())
    }
}

fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

fn read_owned(path: &Path, budget: u64) -> Result<Vec<u8>, String> {
    if path.canonicalize().map_err(|error| error.to_string())? != path {
        return Err("验收文件路径重定向".into());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file()
        || before.nlink() != 1
        || before.uid() != unsafe { libc::geteuid() }
        || before.len() > budget
    {
        return Err("验收文件身份或预算不符".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(budget + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    let named = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > budget
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        )
        || (after.dev(), after.ino()) != (named.dev(), named.ino())
    {
        return Err("验收文件在读取期间改变".into());
    }
    Ok(bytes)
}

fn json_file(path: &Path, budget: u64) -> Result<Value, String> {
    serde_json::from_slice(&read_owned(path, budget)?).map_err(|error| error.to_string())
}

struct Fixture {
    project: PathBuf,
    prepared: Value,
    ceiling: ReviewedCommandCeilingV1,
    inputs: [Value; 2],
    commands: [ReviewedCommand; 2],
}

impl Fixture {
    fn load(root: &Path) -> Result<Self, String> {
        let project = root
            .join("project")
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let prepared = json_file(&root.join("prepared.safe.json"), 64 * 1024)?;
        if prepared["scope"] != SCOPE || prepared["project"] != json!(project) {
            return Err("外置命令夹具绑定不符".into());
        }
        let ceiling =
            ReviewedCommandCeilingV1::capture(&project).map_err(|error| error.to_string())?;
        let value = serde_json::to_value(&ceiling).map_err(|error| error.to_string())?;
        let bindings = value["unix"]["bindings"]
            .as_array()
            .ok_or("缺少实际 Unix 命令绑定")?;
        let input = |name| -> Result<Value, String> {
            let selected = bindings
                .iter()
                .filter(|binding| binding["capability"] == json!({"packageScript":name}))
                .collect::<Vec<_>>();
            let [binding] = selected.as_slice() else {
                return Err("本机没有唯一 Node/npm 固定脚本入口".into());
            };
            Ok(
                json!({"executable":binding["executable"],"argv":binding["arguments"],
                "cwd":project,"timeoutMs":10000}),
            )
        };
        let inputs = [input("g10-a")?, input("g10-b")?];
        let commands = [
            ceiling
                .approve_host(&inputs[0])
                .ok_or("A 不在实际命令上限内")?,
            ceiling
                .approve_host(&inputs[1])
                .ok_or("B 不在实际命令上限内")?,
        ];
        let fixture = Self {
            project,
            prepared,
            ceiling,
            inputs,
            commands,
        };
        fixture.verify_sources()?;
        fixture.require_absent()?;
        Ok(fixture)
    }

    fn verify_sources(&self) -> Result<(), String> {
        self.ceiling
            .verify_sources()
            .map_err(|error| error.to_string())?;
        for name in ["package.json", "g10-command.cjs"] {
            if self.prepared["files_sha256"][name]
                != digest(read_owned(&self.project.join(name), 64 * 1024)?)
            {
                return Err("本轮命令夹具字节改变".into());
            }
        }
        Ok(())
    }

    fn require_absent(&self) -> Result<(), String> {
        for name in ["a.marker", "b.marker"] {
            match fs::symlink_metadata(self.project.join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) | Err(_) => return Err("拒绝的命令产生了输出或文件状态未知".into()),
            }
        }
        Ok(())
    }

    fn marker(&self, index: usize) -> Result<String, String> {
        let name = ["a", "b"][index];
        let bytes = read_owned(&self.project.join(format!("{name}.marker")), 256)?;
        if self.prepared["marker_sha256"][name] != digest(&bytes) {
            return Err("实际命令标记与本轮随机夹具不同".into());
        }
        String::from_utf8(bytes).map_err(|error| error.to_string())
    }
}

struct Decision {
    task_id: String,
    approval_id: String,
    turn_id: String,
    runtime_generation: Uuid,
    command_generation: Option<Uuid>,
    message_id: Uuid,
    decision: ApprovalDecision,
    resolved: bool,
    dispatched: bool,
}

impl Decision {
    fn command_state(&self) -> Result<PathBuf, String> {
        Ok(current_state_dir()
            .join("cli-agent-hosts")
            .join(self.runtime_generation.to_string())
            .join("native")
            .join("reviewed-project-commands")
            .join(self.runtime_generation.to_string())
            .join(self.command_generation.ok_or("缺少命令代次")?.to_string()))
    }

    fn denied_without_spawn(&self, fixture: &Fixture) -> Result<(), String> {
        if self.decision != ApprovalDecision::DenyOnce || !self.resolved || !self.dispatched {
            return Err("首个命令拒绝没有解决或控制确认".into());
        }
        match fs::symlink_metadata(self.command_state()?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fixture.require_absent(),
            Ok(_) | Err(_) => Err("被拒调用出现命令领取目录或状态未知".into()),
        }
    }
}

fn verify_command_exit(
    fixture: &Fixture,
    decision: &Decision,
    index: usize,
    evidence: &mut Evidence,
) -> Result<(), String> {
    fixture.verify_sources()?;
    if decision.decision != ApprovalDecision::AllowOnce
        || !decision.resolved
        || !decision.dispatched
    {
        return Err("命令缺少单次允许及控制确认".into());
    }
    let state = decision.command_state()?;
    let generation = decision.command_generation.ok_or("缺少命令代次")?;
    let record_bytes = read_owned(&state.join("reviewed-command.json"), 16 * 1024)?;
    let record: Value = serde_json::from_slice(&record_bytes).map_err(|error| error.to_string())?;
    let call = record["callId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("命令缺少实际 MCP 调用 ID")?;
    let identity = Sha256::digest(
        serde_json::to_vec(&(
            "infinishell-project-command-v1",
            decision.runtime_generation,
            decision.turn_id.as_str(),
            call,
        ))
        .map_err(|error| error.to_string())?,
    );
    let expected_generation =
        Uuid::from_bytes(identity[..16].try_into().expect("SHA-256 固定长度"));
    let command_bytes =
        serde_json::to_vec(&fixture.commands[index]).map_err(|error| error.to_string())?;
    if generation != expected_generation
        || record["version"] != 1
        || record["platform"] != "macos"
        || record["parentGeneration"] != json!(decision.runtime_generation)
        || record["commandGeneration"] != json!(generation)
        || record["turnId"] != decision.turn_id
        || record["approvalId"] != decision.approval_id
        || record["commandSha256"] != digest(&command_bytes)
    {
        return Err("实际命令领取记录不属于本次精确审批".into());
    }
    let binding = PreparedLaunchBinding::unix_reviewed_project_command(digest(&command_bytes))
        .map_err(|error| error.to_string())?;
    let receipt = confirmed_exit_with_binding(&state, generation, &binding)
        .map_err(|error| error.to_string())?
        .ok_or("命令原生退出尚未确认")?;
    let exit_bytes = read_owned(
        &state
            .join("cli-agent-processes")
            .join(generation.to_string())
            .join("exit.json"),
        16 * 1024,
    )?;
    if serde_json::from_slice::<Value>(&exit_bytes).map_err(|error| error.to_string())?
        != json!(receipt)
    {
        return Err("退出收据原件与受验证收据不同".into());
    }
    if receipt.exit_code != Some(0)
        || !receipt.cleanup_confirmed
        || receipt.containment != "macos_resource_coalition"
    {
        return Err("命令退出或整树清理未成功".into());
    }
    let result_bytes = read_owned(&state.join("result.json"), 512 * 1024)?;
    let result: Value = serde_json::from_slice(&result_bytes).map_err(|error| error.to_string())?;
    let stdout = read_owned(&state.join("stdout.bin"), 32 * 1024)?;
    let marker = fixture.marker(index)?;
    if result["commandGeneration"] != json!(generation)
        || result["exitCode"] != 0
        || result["error"] != Value::Null
        || result["outputSha256"] != digest(&stdout)
        || result["outputBytes"] != json!(stdout.len())
        || result["outputPreviewTruncated"] != false
        || result["cleanup"] != json!(receipt)
        || !String::from_utf8_lossy(&stdout).contains(marker.trim())
    {
        return Err("命令真实输出、结果与退出收据不一致".into());
    }
    evidence.record(
        json!({"event":"command_exit_verified","fixture":(["a","b"][index]),
        "command_generation":generation,"runtime_generation":decision.runtime_generation,
        "call_id":call,"approval_id":decision.approval_id,
        "record_sha256":digest(record_bytes),"result_sha256":digest(result_bytes),
        "command_sha256":digest(command_bytes),"exit_receipt_sha256":digest(exit_bytes),
        "stdout_sha256":digest(&stdout),"stdout_bytes":stdout.len(),
        "marker_sha256":digest(marker.as_bytes()),"cleanup_confirmed":true,
        "exit_code":0,"containment":receipt.containment}),
    )
}

async fn control(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    task: &LocalCliTask,
    action: RuntimeAction,
) -> Result<Uuid, String> {
    let message_id = Uuid::new_v4();
    coordinator
        .update(app, |model, _| {
            model.request_for_generation(&task.task_id, task.generation, message_id, action)
        })?
        .await
        .map_err(|_| "控制确认通道关闭")??;
    Ok(message_id)
}

async fn verify_saved(
    sender: &SyncSender<ModelEvent>,
    parent_id: &str,
    child_id: &str,
    initial_message: Uuid,
    fixture: &Fixture,
    identities: &HashMap<String, (Uuid, String)>,
    evidence: &mut Evidence,
) -> Result<bool, String> {
    let saved = load_tasks(sender, false)?
        .await
        .map_err(|_| "任务查询确认关闭")??;
    if saved.len() != 2 {
        return Err("出现额外任务或父子记录丢失".into());
    }
    let parent = saved
        .iter()
        .find(|row| row.task_id == parent_id)
        .ok_or("父记录丢失")?;
    let child = saved
        .iter()
        .find(|row| row.task_id == child_id)
        .ok_or("子记录丢失")?;
    if parent.state != LocalCliTaskState::Completed || child.state != LocalCliTaskState::Completed {
        return Ok(false);
    }
    let markers = [fixture.marker(0)?, fixture.marker(1)?];
    if !parent.result.as_ref().is_some_and(|text| {
        text.contains(PARENT_DONE) && markers.iter().all(|marker| text.contains(marker.trim()))
    }) {
        return Ok(false);
    }
    if !child.result.as_ref().is_some_and(|text| {
        text.contains(CHILD_DONE) && markers.iter().all(|marker| text.contains(marker.trim()))
    }) {
        return Err("子最终结果没有实际两条命令标记".into());
    }
    let history = load_task_generations(sender, parent_id.to_owned())?
        .await
        .map_err(|_| "父代查询关闭")??;
    let source = history
        .iter()
        .find(|row| row.generation == 1)
        .ok_or("缺少原始创建父代")?;
    let parent_config: Value =
        serde_json::from_str(&source.config_json).map_err(|error| error.to_string())?;
    let child_config: Value =
        serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
    let ceiling = ceiling_from_parent(source, "claude").map_err(|error| error.to_string())?;
    let profile = ceiling.claude_profile().ok_or("父上限缺少受审命令策略")?;
    let actual_commands = profile.host_commands().ok_or("父上限缺少受审命令")?;
    if child.parent_task_id.as_deref() != Some(parent_id)
        || child.parent_generation != Some(1)
        || child.generation != 1
        || child.terminal_evidence.is_none()
        || child_config["permission_policy"] != "ClaudeReviewedCommandsV1"
        || child_config["permission_ceiling"] != json!(ceiling)
        || child_config["claude_profile"] != parent_config["claude_profile"]
        || child_config["effective_permissions"]["claudeReviewedCommandsV1"]
            != parent_config["claude_profile"]
        || child_config["effective_permissions"]["fixedProfileVerified"] != true
        || serde_json::to_value(actual_commands).map_err(|error| error.to_string())?
            != json!(fixture.ceiling)
    {
        return Err("真实父子命令权限或创建代次不匹配".into());
    }
    for task in [parent, child] {
        if task.native_session_id.as_ref() != identities.get(&task.task_id).map(|(_, sid)| sid) {
            return Err("持久任务原生 SID 与已配对连接不同".into());
        }
    }
    if parent.native_session_id == child.native_session_id {
        return Err("父子复用了同一原生 SID".into());
    }
    let rows = load_task_messages(sender, parent_id.into())?
        .await
        .map_err(|_| "父消息查询关闭")??;
    let child_rows = load_task_messages(sender, child_id.into())?
        .await
        .map_err(|_| "子消息查询关闭")??;
    let mut messages = HashMap::new();
    for row in rows.into_iter().chain(child_rows) {
        messages.insert(row.message_id.clone(), row);
    }
    let acknowledged = |row: &LocalCliMessage| {
        row.state == LocalCliMessageState::Acknowledged
            && row.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
    };
    let initial = messages
        .get(&initial_message.to_string())
        .ok_or("父原始输入不存在")?;
    let child_inputs = messages
        .values()
        .filter(|row| row.subject == "user_input" && row.recipient_task_id == child_id)
        .collect::<Vec<_>>();
    let results = messages
        .values()
        .filter(|row| {
            row.subject == "local_task_result"
                && row.sender_task_id == child_id
                && row.recipient_task_id == parent_id
        })
        .collect::<Vec<_>>();
    if messages
        .values()
        .filter(|row| row.subject == "user_input")
        .count()
        != 2
        || child_inputs.len() > 1
        || results.len() > 1
    {
        return Err("父子产生额外输入或重复结果".into());
    }
    if child_inputs.len() != 1 || results.len() != 1 {
        return Ok(false);
    }
    if !acknowledged(initial) || !acknowledged(child_inputs[0]) || !acknowledged(results[0]) {
        return Ok(false);
    }
    let requests = messages
        .values()
        .filter(|row| row.subject == "native_tool_call")
        .collect::<Vec<_>>();
    let [request] = requests.as_slice() else {
        return Err("父子派发产生额外协调器工具调用".into());
    };
    let call: NativeLocalToolRequest =
        serde_json::from_str(&request.body).map_err(|error| error.to_string())?;
    if request.sender_task_id != parent_id || call.tool != "run_agents" {
        return Err("唯一协调器工具调用不是父任务派发".into());
    }
    for row in [initial, child_inputs[0], results[0]] {
        evidence.record(json!({"event":"native_mailbox_ack","message_id":row.message_id,
            "sender_task_id":row.sender_task_id,"recipient_task_id":row.recipient_task_id,
            "sender_generation":row.sender_generation,"recipient_generation":row.recipient_generation,
            "subject":row.subject,"receipt_kind":"native_protocol","body_sha256":digest(row.body.as_bytes())}))?;
    }
    evidence.record(json!({"event":"saved_chain_verified","parent_task_id":parent_id,
        "child_task_id":child_id,"parent_native_session_id":parent.native_session_id,
        "child_native_session_id":child.native_session_id,"parent_ceiling_saved":true,
        "fixed_profile_sha256":digest(serde_json::to_vec(&parent_config["claude_profile"]).map_err(|error| error.to_string())?),
        "parent_result_sha256":parent.result.as_ref().map(|text| digest(text.as_bytes())),
        "child_result_sha256":child.result.as_ref().map(|text| digest(text.as_bytes())),
        "child_input_count":1,"automatic_result_count":1,"real_gui_verified":false}))?;
    Ok(true)
}

async fn drive(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    fixture: &Fixture,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let parent_id = Uuid::new_v4().to_string();
    let child_prompt = format!(
        "Use only the SDK MCP reviewed_project_command tool. Request A exactly with {}. \
         The application will deny this first request. Accept that denial, then explicitly make \
         one new request A with the identical arguments. The app will separately allow it. \
         After its actual successful result, request B exactly with {} once and await its result. \
         Do not retry other errors or unknown results. Do not use files, native Bash, skills, \
         other tools or child agents. Finally reply {CHILD_DONE} followed by the two different \
         G10_COMMAND markers actually returned by the allowed commands, in A then B order.",
        fixture.inputs[0], fixture.inputs[1],
    );
    let spawn = json!({"summary":"isolated reviewed project commands","base_prompt":"",
        "harness":"claude","model_id":MODEL,"skills":[],
        "agent_run_configs":[{"name":"reviewed-commands","prompt":child_prompt}]});
    let prompt = format!(
        "Call run_agents exactly once with {spawn}. Do not call another tool or \
        perform the child work yourself. After the child is queued reply PARENT_QUEUED. \
        When the runtime delivers Subject: local_task_result for this child, read its actual \
        completed result and reply {PARENT_DONE} followed by its two G10_COMMAND markers in \
        the child's original order, without additional tools."
    );
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?,
        ),
        cwd: fixture.project.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::ClaudeReviewedCommandsV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: Some(MODEL.into()),
        local_tools: Some(LocalToolPermissions {
            allow_spawn: true,
            allow_message: false,
            allow_project_commands: true,
        }),
        selected_skills: Vec::new(),
    };
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "claude".into(),
        working_directory: fixture.project.to_string_lossy().into(),
        config_json: json!({"model":MODEL,"permission_policy":"ClaudeReviewedCommandsV1"})
            .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let initial_message = Uuid::new_v4();
    evidence.record(json!({"event":"run_started","parent_task_id":parent_id,
        "runtime_generation":options.generation,"initial_message_id":initial_message,
        "requested_model":MODEL,"actual_model_verified":false}))?;
    coordinator.update(app, |model, ctx| {
        model.start_with_input(
            task,
            options,
            None,
            initial_message,
            vec![InputContent::Text(prompt)],
            ctx,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(450);
    let mut identities = HashMap::<String, (Uuid, String)>::new();
    let mut paired_ready = HashSet::new();
    let mut child_id = None;
    let mut child_turn = None;
    let mut decisions = Vec::<Decision>::new();
    let mut commands = Vec::<usize>::new();
    let mut parent_approved = false;
    let mut parent_spawned = false;
    let mut native_commands = 0;
    let mut a_exit_verified = false;
    let mut child_finished = false;
    let mut events_seen = 0;
    loop {
        if Instant::now() >= deadline {
            return Err("真实命令父子链超过 450 秒".into());
        }
        if let Some(error) = coordinator.read(app, |model, _| {
            model
                .snapshots()
                .find_map(|snapshot| snapshot.error.clone())
        }) {
            evidence.record(
                json!({"event":"coordinator_error","reason_sha256":digest(error.as_bytes())}),
            )?;
            return Err("协调器报告错误".into());
        }
        let wake = receiver
            .recv()
            .with_timeout(
                Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        let wake = match wake {
            Ok(Some(wake)) => wake,
            Ok(None) => return Err("协调器事件通道关闭".into()),
            Err(_) => None,
        };
        if let Some((task, event)) = wake {
            events_seen += 1;
            if events_seen > 768 {
                return Err("原生事件超过本轮预算".into());
            }
            let is_parent = task.task_id == parent_id;
            if !is_parent {
                if task.parent_task_id.as_deref() != Some(parent_id.as_str())
                    || child_id.as_ref().is_some_and(|id| id != &task.task_id)
                {
                    return Err("出现额外或不相干子任务".into());
                }
                child_id = Some(task.task_id.clone());
            }
            if let Some(sid) = &event.native_session_id {
                Uuid::parse_str(sid).map_err(|_| "原生会话 ID 格式无效")?;
                if let Some(previous) =
                    identities.insert(task.task_id.clone(), (event.generation, sid.clone()))
                {
                    if previous != (event.generation, sid.clone()) {
                        return Err("连接身份在验收中改变".into());
                    }
                }
            }
            if let RuntimeEventKind::SessionReady {
                verified_cli_version,
                effective_permissions,
            } = &event.kind
            {
                match verified_cli_version.as_deref() {
                    None => {
                        if event.native_session_id.is_some() {
                            return Err("未配对连接提前暴露原生 SID".into());
                        }
                    }
                    Some("2.1.280") => {
                        if event.native_session_id.is_none()
                            || effective_permissions["fixedProfileVerified"] != true
                            || effective_permissions["claudeReviewedCommandsV1"].is_null()
                        {
                            return Err("原生版本或受审命令策略未实际配对".into());
                        }
                        paired_ready.insert(task.task_id.clone());
                        evidence.record(json!({"event":"session_ready","task_id":task.task_id,
                            "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                            "verified_cli_version":verified_cli_version,
                            "permission_sha256":digest(serde_json::to_vec(effective_permissions).map_err(|error| error.to_string())?)}))?;
                    }
                    Some(_) => return Err("原生 CLI 版本不符".into()),
                }
            }
            if let RuntimeEventKind::TurnStarted { turn_id } = &event.kind {
                if !is_parent && child_turn.replace(turn_id.clone()).is_some() {
                    return Err("子任务出现额外回合".into());
                }
                evidence.record(json!({"event":"turn_started","task_id":task.task_id,
                    "runtime_generation":event.generation,"native_session_id":event.native_session_id,"turn_id":turn_id}))?;
            }
            if let RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } = &event.kind
            {
                if !paired_ready.contains(&task.task_id) {
                    return Err("审批发生在原生身份配对前".into());
                }
                if decisions
                    .iter()
                    .any(|row| row.task_id == task.task_id && row.approval_id == *approval_id)
                {
                    return Err("审批 ID 重复".into());
                }
                let mut command_generation = None;
                let decision = if method == "can_use_tool" {
                    if is_parent {
                        if parent_approved
                            || details["tool_name"] != "mcp__infinishell-local-tasks__run_agents"
                            || details["input"] != spawn
                        {
                            return Err("父原生审批不是唯一精确派发".into());
                        }
                        parent_approved = true;
                    } else {
                        let index = if commands.len() < 2 { 0 } else { 1 };
                        if commands.len() >= 3
                            || native_commands >= 3
                            || details["tool_name"] != CLAUDE_TOOL
                            || details["input"] != fixture.inputs[index]
                        {
                            return Err("子原生工具审批越出本轮精确命令".into());
                        }
                        native_commands += 1;
                    }
                    ApprovalDecision::AllowOnce
                } else if method == "infinishell/reviewed_project_command" && !is_parent {
                    let step = commands.len();
                    if step >= 3 || child_turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("宿主命令审批次数或子回合不符".into());
                    }
                    fixture.verify_sources()?;
                    let generation: Uuid = serde_json::from_value(details["commandId"].clone())
                        .map_err(|error| error.to_string())?;
                    let mut expected = fixture.commands[usize::from(step == 2)].approval_context();
                    expected["commandId"] = json!(generation);
                    expected["parentGeneration"] = json!(event.generation);
                    if details != &expected
                        || *approval_id != format!("project-command:{generation}")
                        || decisions
                            .iter()
                            .any(|row| row.command_generation == Some(generation))
                    {
                        return Err("宿主命令审批未绑定真实精确参数或独立代次".into());
                    }
                    if step == 0 {
                        fixture.require_absent()?;
                    }
                    if step == 1 {
                        decisions[commands[0]].denied_without_spawn(fixture)?;
                        evidence.record(json!({"event":"denied_without_spawn","command_generation":decisions[commands[0]].command_generation,
                            "approval_id":decisions[commands[0]].approval_id,"zero_command_record":true,"markers_absent":true,
                            "native_call_id_exposed":false}))?;
                    }
                    if step == 2 {
                        verify_command_exit(fixture, &decisions[commands[1]], 0, evidence)?;
                        a_exit_verified = true;
                    }
                    command_generation = Some(generation);
                    commands.push(decisions.len());
                    if step == 0 {
                        ApprovalDecision::DenyOnce
                    } else {
                        ApprovalDecision::AllowOnce
                    }
                } else {
                    return Err("出现未授权的其它审批".into());
                };
                let message_id = control(
                    app,
                    coordinator,
                    &task,
                    RuntimeAction::RespondApproval {
                        approval_id: approval_id.clone(),
                        decision,
                    },
                )
                .await?;
                evidence.record(json!({"event":"approval_decision","task_id":task.task_id,
                    "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                    "approval_id":approval_id,"turn_id":turn_id,"method":method,
                    "command_generation":command_generation,"message_id":message_id,"decision":decision,
                    "details_sha256":digest(serde_json::to_vec(details).map_err(|error| error.to_string())?)}))?;
                decisions.push(Decision {
                    task_id: task.task_id.clone(),
                    approval_id: approval_id.clone(),
                    turn_id: turn_id.clone(),
                    runtime_generation: event.generation,
                    command_generation,
                    message_id,
                    decision,
                    resolved: false,
                    dispatched: false,
                });
            }
            if let RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } = &event.kind
            {
                let row = decisions
                    .iter_mut()
                    .find(|row| row.task_id == task.task_id && row.approval_id == *approval_id)
                    .ok_or("解决了未观察到的审批")?;
                if row.resolved || row.decision != *decision {
                    return Err("审批解决重复或决定改变".into());
                }
                row.resolved = true;
                evidence.record(json!({"event":"approval_resolved","task_id":task.task_id,
                    "runtime_generation":event.generation,"approval_id":approval_id,"decision":decision}))?;
            }
            if let RuntimeEventKind::CommandDispatched { message_id, .. } = &event.kind {
                if let Some(row) = decisions
                    .iter_mut()
                    .find(|row| row.message_id == *message_id)
                {
                    if row.dispatched || row.task_id != task.task_id {
                        return Err("审批控制确认重复或串任务".into());
                    }
                    row.dispatched = true;
                    evidence.record(json!({"event":"approval_control_dispatched","task_id":task.task_id,
                        "runtime_generation":event.generation,"approval_id":row.approval_id,"message_id":message_id}))?;
                }
            }
            if let RuntimeEventKind::LocalToolRequested { request } = &event.kind {
                if !is_parent
                    || parent_spawned
                    || request.tool != "run_agents"
                    || request.arguments != spawn
                {
                    return Err("原生协调器工具请求越界".into());
                }
                parent_spawned = true;
                evidence.record(json!({"event":"parent_spawn_requested","task_id":task.task_id,
                    "native_session_id":event.native_session_id,"turn_id":request.turn_id,"call_id":request.call_id}))?;
            }
            if let RuntimeEventKind::TurnFinished {
                outcome, turn_id, ..
            } = &event.kind
            {
                if *outcome != TurnOutcome::Completed {
                    return Err("实际模型回合未正常完成".into());
                }
                if !is_parent {
                    child_finished = true;
                }
                evidence.record(json!({"event":"turn_finished","task_id":task.task_id,
                    "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                    "turn_id":turn_id,"outcome":"completed"}))?;
            }
            if matches!(
                event.kind,
                RuntimeEventKind::RequestFailed { .. }
                    | RuntimeEventKind::Disconnected { .. }
                    | RuntimeEventKind::ApprovalCancelled { .. }
                    | RuntimeEventKind::LocalToolCancelled { .. }
            ) {
                return Err("真实连接、审批或调用意外终止".into());
            }
        }
        if commands.len() == 3
            && child_finished
            && parent_spawned
            && parent_approved
            && decisions.iter().all(|row| row.resolved && row.dispatched)
        {
            let id = child_id.as_deref().ok_or("子身份丢失")?;
            if verify_saved(
                sender,
                &parent_id,
                id,
                initial_message,
                fixture,
                &identities,
                evidence,
            )
            .await?
            {
                if !a_exit_verified {
                    return Err("第二条审批前未核第一条完整清理".into());
                }
                verify_command_exit(fixture, &decisions[commands[2]], 1, evidence)?;
                let denied = decisions[commands[0]].command_state()?;
                match fs::symlink_metadata(&denied) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(_) | Err(_) => return Err("拒绝调用后来被执行或状态未知".into()),
                }
                let parent = denied.parent().ok_or("命令目录缺少父级")?;
                let actual = fs::read_dir(parent)
                    .map_err(|error| error.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                if actual.len() != 2 {
                    return Err("真实命令监督代不是恰好两条".into());
                }
                evidence.record(
                    json!({"event":"minimal_chain_verified","command_requests":3,
                    "command_executions":2,"native_command_approvals":native_commands,
                    "first_cleanup_before_second_approval":true,"native_input_replay":false,
                    "explicit_bidirectional_send_message_covered":false}),
                )?;
                return Ok(());
            }
        }
    }
}

async fn shutdown(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let ids = coordinator.read(app, |model, _| {
        model
            .snapshots()
            .filter(|row| row.connected)
            .map(|row| row.task.task_id.clone())
            .collect::<Vec<_>>()
    });
    for id in ids {
        if let Ok(reply) = coordinator.update(app, |model, _| {
            model.request(&id, Uuid::new_v4(), RuntimeAction::Shutdown)
        }) {
            let _ = reply.with_timeout(Duration::from_secs(5)).await;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while coordinator.read(app, |model, _| model.snapshots().any(|row| row.connected)) {
        receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "协调器关闭尚未确认")?
            .ok_or("关闭事件通道丢失")?;
    }
    let snapshots = coordinator.read(app, |model, _| {
        model.snapshots().cloned().collect::<Vec<_>>()
    });
    for snapshot in snapshots {
        let config: Value =
            serde_json::from_str(&snapshot.task.config_json).map_err(|error| error.to_string())?;
        let generation: Uuid = serde_json::from_value(config["runtime_generation"].clone())
            .map_err(|error| error.to_string())?;
        let receipt = loop {
            if let Some(receipt) = confirmed_exit(&current_state_dir(), generation)
                .map_err(|error| error.to_string())?
            {
                break receipt;
            }
            if Instant::now() >= deadline {
                return Err("父子原生退出回执缺失".into());
            }
            Timer::after(Duration::from_millis(50)).await;
        };
        if receipt.native_process != NativeProcessCompletion::Exited
            || receipt.native_cleanup_sha256.is_none()
            || !receipt.adapter_task_terminated
            || !receipt.adapter_succeeded
            || !receipt.event_journal_completed
        {
            return Err("父子原生退出与清理未确认".into());
        }
        evidence.record(
            json!({"event":"runtime_cleanup_confirmed","task_id":snapshot.task.task_id,
            "native_session_id":snapshot.task.native_session_id,"runtime_generation":generation,
            "receipt":receipt}),
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "仅由私有短目录运行器执行；固定 Claude 2.1.280 的真实父子命令链会产生模型费用"]
fn real_claude_g10_reviewed_commands_parent_child() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    let temporary = PathBuf::from(env::var_os("TMPDIR").expect("缺少本轮短 TMPDIR"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        temporary.parent().unwrap(),
        Path::new("/Users/zhishi/InfiniShell-Tests")
    );
    assert_eq!(root, temporary.join("g10-reviewed-commands"));
    assert_eq!(
        read_owned(&root.join(".infinishell-g10-reviewed-commands"), 256).unwrap(),
        format!("{SCOPE}\n").as_bytes()
    );
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION").unwrap(),
        "2.1.280"
    );
    assert_eq!(env::var("INFINISHELL_CLAUDE_LIVE_MODEL").unwrap(), MODEL);
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_LIVE_AUTH_MODE").unwrap(),
        "authorized_default_account"
    );
    assert!(env::var_os("CLAUDE_CONFIG_DIR").is_none());
    assert!(env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").is_none());
    let home = PathBuf::from(env::var_os("HOME").expect("保留默认账号 HOME"))
        .canonicalize()
        .unwrap();
    assert!(!home.starts_with(&temporary));
    let profile = env::var("INFINISHELL_CLAUDE_LIVE_STATE_PROFILE").unwrap();
    let nonce = profile
        .strip_prefix("claude-g10-commands-")
        .expect("必须使用专属 profile");
    assert_eq!(nonce.len(), 32);
    assert!(
        nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(env::var("WARP_DATA_PROFILE").unwrap(), profile);
    let state = current_state_dir();
    assert!(state.starts_with(&home));
    assert_eq!(
        state.parent().unwrap().canonicalize().unwrap(),
        state.parent().unwrap()
    );
    assert!(
        state
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(&format!("-{profile}"))
    );
    assert_eq!(
        fs::symlink_metadata(&state).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let fixture = Fixture::load(&root).expect("固定命令夹具预检失败；尚未派发模型输入");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("reviewed-commands.events.safe.ndjson"))
        .unwrap();
    let mut evidence = Evidence(file);
    let state_metadata = fs::symlink_metadata(&state).unwrap();
    evidence
        .record(json!({"event":"state_binding","state_directory":state,
        "device":state_metadata.dev(),"inode":state_metadata.ino(),"uid":state_metadata.uid(),
        "mode":state_metadata.mode() & 0o777}))
        .unwrap();
    evidence.record(json!({"event":"acceptance_started","scope":SCOPE,"cli_version":"2.1.280",
        "requested_model":MODEL,"actual_model_verified":false,"policy":"ClaudeReviewedCommandsV1","max_children":1,
        "max_host_command_requests":3,"max_command_executions":2,"real_gui_verified":false,
        "cold_recovery_verified":false,"other_command_families_verified":false})).unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let _bundled = FeatureFlag::BundledSkills.override_enabled(false);
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        app.add_singleton_model(SkillManager::new);
        let executable = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").unwrap());
        app.add_singleton_model(|_| {
            CLIAgentInstallModel::with_installation_for_test(
                CLIAgent::Claude,
                CLIAgentInstallation {
                    executable: Some(executable),
                    version: CLIAgentVersionStatus::Detected("2.1.280".into()),
                },
            )
        });
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let (wake, mut receiver) = mpsc::unbounded_channel();
        app.update(|ctx| {
            ctx.subscribe_to_model(&coordinator, move |_, event, _| {
                let event = if let LocalCLITaskCoordinatorEvent::Runtime { task, event } = event {
                    Some((task.clone(), event.clone()))
                } else {
                    None
                };
                let _ = wake.send(event);
            })
        });
        let result = drive(
            &mut app,
            &coordinator,
            &writer.sender,
            &mut receiver,
            &fixture,
            &mut evidence,
        )
        .await;
        let cleanup = shutdown(&mut app, &coordinator, &mut receiver, &mut evidence).await;
        evidence.record(json!({"event":"phase_results","scenario_ok":result.is_ok(),"cleanup_ok":cleanup.is_ok(),
            "scenario_error_sha256":result.as_ref().err().map(|error| digest(error.as_bytes())),
            "cleanup_error_sha256":cleanup.as_ref().err().map(|error| digest(error.as_bytes()))})).unwrap();
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleanup) {
            Ok(()) => evidence
                .record(json!({"event":"acceptance_passed","scope":SCOPE,
                "host_decisions":["DenyOnce","AllowOnce","AllowOnce"],"command_executions":2,
                "parent_child_input_and_result_native_ack":true,"all_runtime_hosts_cleaned":true,
                "gap_closed":false,"real_gui_verified":false}))
                .unwrap(),
            Err(_) => panic!("真实 G10 受审命令最小父子链失败；查看本轮白名单阶段证据"),
        }
    });
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
#[path = "g10_command_cancel_live_tests.rs"]
mod command_cancel_live_tests;
