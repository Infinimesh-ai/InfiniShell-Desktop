//! 固定 Grok CommandsSkills 的真实父子链与受审命令验收。
//! 必须补齐外置原生历史白名单审核，不能凭本测试关闭 G10。

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
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::local_tools::{MCP_SERVER_NAME, NativeLocalToolRequest};
use crate::ai::cli_agent_runtime::managed_process::{
    PreparedLaunchBinding, confirmed_exit_with_binding,
};
use crate::ai::cli_agent_runtime::permissions::GrokCreationPolicyV1;
use crate::ai::cli_agent_runtime::permissions::ceiling_from_parent;
use crate::ai::cli_agent_runtime::reviewed_project_commands::{
    ReviewedCommand, ReviewedCommandCeilingV1,
};
use crate::ai::cli_agent_runtime::runtime_host::{NativeProcessCompletion, confirmed_exit};
use crate::ai::cli_agent_runtime::{ApprovalDecision, current_state_dir};
use crate::ai::skills::SkillManager;
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

const SCOPE: &str = "real_grok_g10_commands_skills_parent_child_v2";
const MODEL: &str = "grok-4.7";
const CLI_VERSION: &str = "1.0.41";
const ALPHA: &str = "isp-g10-fixed-alpha";
const BETA: &str = "isp-g10-fixed-beta";
const SCRIPT: &str = "g10-once";
const CHILD_DONE: &str = "G10_GROK_CHILD_DONE";
const PARENT_DONE: &str = "G10_GROK_PARENT_COLLECTED";
const OUTSIDE_DONE: &str = "G10_GROK_OUTSIDE_REJECTED";
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
    root: PathBuf,
    prepared: Value,
    ceiling: ReviewedCommandCeilingV1,
    input: Value,
    command: ReviewedCommand,
    alpha_marker: String,
    search_input: Value,
    search_marker: String,
}

impl Fixture {
    fn selected(&self, name: &str) -> SelectedLocalSkill {
        SelectedLocalSkill {
            name: name.into(),
            path: self.root.join("skills").join(name).join("SKILL.md"),
        }
    }
    fn load(root: &Path) -> Result<Self, String> {
        let project = root
            .join("project")
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let prepared = json_file(&root.join("prepared.safe.json"), 64 * 1024)?;
        if prepared["scope"] != SCOPE || prepared["project"] != json!(project) {
            return Err("本轮夹具来源不符".into());
        }
        let ceiling =
            ReviewedCommandCeilingV1::capture(&project).map_err(|error| error.to_string())?;
        let value = serde_json::to_value(&ceiling).map_err(|error| error.to_string())?;
        let bindings = value["unix"]["bindings"]
            .as_array()
            .ok_or("缺少实际命令映像绑定")?;
        let [binding] = bindings.as_slice() else {
            return Err("生产 capture 并非唯一命令，禁止继续".into());
        };
        if value["commands"] != json!([{"packageScript":SCRIPT}])
            || binding["capability"] != json!({"packageScript":SCRIPT})
        {
            return Err("本轮 PATH 捕获到额外命令能力".into());
        }
        let input = json!({"executable":binding["executable"],"argv":binding["arguments"],"cwd":project,"timeoutMs":10000});
        let command = ceiling
            .approve_host(&input)
            .ok_or("命令不在实际捕获上限内")?;
        let skill = read_owned(&root.join("skills").join(ALPHA).join("SKILL.md"), 64 * 1024)?;
        let skill = String::from_utf8(skill).map_err(|error| error.to_string())?;
        let alpha_marker = skill
            .lines()
            .last()
            .filter(|line| line.starts_with("G10_ALPHA_"))
            .ok_or("技能隐藏标记缺失")?
            .to_owned();
        let search_bytes = read_owned(&project.join("search-target.txt"), 4096)?;
        let search_text = String::from_utf8(search_bytes).map_err(|error| error.to_string())?;
        let search_input = prepared["search_input"].clone();
        let needle = search_input["pattern"].as_str().ok_or("搜索针缺失")?;
        let search_marker = search_text
            .lines()
            .nth(1)
            .and_then(|line| line.strip_prefix(&format!("{needle} ")))
            .filter(|marker| marker.starts_with("G10_SEARCH_"))
            .ok_or("搜索隐藏标记缺失")?
            .to_owned();
        if search_input
            != json!({"pattern":needle,"path":"search-target.txt","glob":null,
            "-i":false,"type":null,"head_limit":5,"multiline":false})
            || prepared["search_marker_sha256"] != digest(search_marker.as_bytes())
            || search_text.lines().count() != 3
        {
            return Err("搜索夹具或唯一文件参数不符".into());
        }
        let fixture = Self {
            project,
            root: root.to_owned(),
            prepared,
            ceiling,
            input,
            command,
            alpha_marker,
            search_input,
            search_marker,
        };
        fixture.verify_sources()?;
        fixture.require_absent()?;
        Ok(fixture)
    }
    fn verify_sources(&self) -> Result<(), String> {
        self.ceiling
            .verify_sources()
            .map_err(|error| error.to_string())?;
        for relative in [
            "project/package.json",
            "project/g10-command.cjs",
            "project/search-target.txt",
            "skills/isp-g10-fixed-alpha/SKILL.md",
            "skills/isp-g10-fixed-beta/SKILL.md",
        ] {
            if self.prepared["files_sha256"][relative]
                != digest(read_owned(&self.root.join(relative), 64 * 1024)?)
            {
                return Err("夹具资源字节改变".into());
            }
        }
        Ok(())
    }
    fn search_approval(&self) -> Value {
        let mut input = self.search_input.clone();
        input["variant"] = json!("GrepSearch");
        input
    }
    fn require_absent(&self) -> Result<(), String> {
        match fs::symlink_metadata(self.project.join("a.marker")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) | Err(_) => Err("拒绝调用产生文件或文件状态未知".into()),
        }
    }
    fn marker(&self) -> Result<String, String> {
        let bytes = read_owned(&self.project.join("a.marker"), 256)?;
        if self.prepared["command_marker_sha256"] != digest(&bytes) {
            return Err("实际命令标记不符".into());
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

    fn denied_directory_still_absent(&self) -> Result<(), String> {
        if self.decision != ApprovalDecision::DenyOnce || !self.resolved || !self.dispatched {
            return Err("拒绝控制没有完成".into());
        }
        match fs::symlink_metadata(self.command_state()?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) | Err(_) => Err("被拒调用后来出现领取目录或状态未知".into()),
        }
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
    native_calls: &HashSet<String>,
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
    let command_bytes = serde_json::to_vec(&fixture.command).map_err(|error| error.to_string())?;
    if !native_calls.contains(call)
        || generation != expected_generation
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
    let marker = fixture.marker()?;
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
    evidence.record(json!({"event":"command_exit_verified","fixture":"a",
        "command_generation":generation,"runtime_generation":decision.runtime_generation,
        "call_id":call,"approval_id":decision.approval_id,
        "record_sha256":digest(record_bytes),"result_sha256":digest(result_bytes),
        "command_sha256":digest(command_bytes),"exit_receipt_sha256":digest(exit_bytes),
        "stdout_sha256":digest(&stdout),"stdout_bytes":stdout.len(),
        "marker_sha256":digest(marker.as_bytes()),"cleanup_confirmed":true,
        "exit_code":0,"containment":receipt.containment}))
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
    fixture: &Fixture,
    initial: Uuid,
    outside: Option<Uuid>,
    outside_spawn: &Value,
    evidence: &mut Evidence,
) -> Result<bool, String> {
    let tasks = load_tasks(sender, false)?
        .await
        .map_err(|_| "任务查询关闭")??;
    if tasks.len() != 2 {
        return Err("任务总数不是精确父子两条".into());
    }
    let parent = tasks
        .iter()
        .find(|task| task.task_id == parent_id)
        .ok_or("父身份缺失")?;
    let child = tasks
        .iter()
        .find(|task| task.task_id == child_id)
        .ok_or("子身份缺失")?;
    if parent.state != LocalCliTaskState::Completed || child.state != LocalCliTaskState::Completed {
        return Ok(false);
    }
    let expected_parent = if outside.is_some() {
        OUTSIDE_DONE
    } else {
        PARENT_DONE
    };
    if !parent
        .result
        .as_ref()
        .is_some_and(|text| text.contains(expected_parent))
    {
        return Ok(false);
    }
    let marker = fixture.marker()?;
    if !child.result.as_ref().is_some_and(|text| {
        text.contains(CHILD_DONE)
            && text.contains(&fixture.alpha_marker)
            && text.contains(marker.trim())
            && text.contains(&fixture.search_marker)
    }) {
        return Err("子结果未包含实际 Skill、搜索和命令的隐藏输出标记".into());
    }
    if outside.is_none()
        && !parent.result.as_ref().is_some_and(|text| {
            text.contains(&fixture.alpha_marker)
                && text.contains(marker.trim())
                && text.contains(&fixture.search_marker)
        })
    {
        return Ok(false);
    }
    let parents = load_task_generations(sender, parent_id.into())?
        .await
        .map_err(|_| "父代查询关闭")??;
    let source = parents
        .iter()
        .find(|task| task.generation == 1)
        .ok_or("创建父代缺失")?;
    let source_config: Value =
        serde_json::from_str(&source.config_json).map_err(|error| error.to_string())?;
    let child_config: Value =
        serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
    let ceiling = ceiling_from_parent(source, "grok").map_err(|error| error.to_string())?;
    let parent_profile = ceiling.grok_profile().ok_or("原父 Grok 上限缺失")?;
    let child_profile: GrokCreationPolicyV1 =
        serde_json::from_value(child_config["grok_profile"].clone())
            .map_err(|error| error.to_string())?;
    parent_profile
        .validate_child(&child_profile)
        .map_err(|error| error.to_string())?;
    if child.parent_task_id.as_deref() != Some(parent_id)
        || child.parent_generation != Some(1)
        || child.generation != 1
        || child_config["permission_ceiling"] != json!(ceiling)
        || !parent_profile.skill_selection_matches(&[fixture.selected(ALPHA)])
        || !child_profile.skill_selection_matches(&[fixture.selected(ALPHA)])
        || parent_profile.permits_selected_skill(BETA, &fixture.selected(BETA).path)
        || child_profile.permits_selected_skill(BETA, &fixture.selected(BETA).path)
        || json!(parent_profile.host_commands()) != json!(Some(&fixture.ceiling))
        || json!(child_profile.host_commands()) != json!(Some(&fixture.ceiling))
        || child_config["effective_permissions"]["grokCreationPolicyV1"]
            != child_config["grok_profile"]
        || child_config["effective_permissions"]["appCreationPolicyApplied"] != true
        || child_config["effective_permissions"]["permissionEnforcementVerified"] != false
        || parent.native_session_id.is_none()
        || child.native_session_id.is_none()
        || parent.native_session_id == child.native_session_id
    {
        return Err("父子原生身份、创建上限或实际选择不符".into());
    }
    let mut messages = load_task_messages(sender, parent_id.into())?
        .await
        .map_err(|_| "父邮箱查询关闭")??;
    messages.extend(
        load_task_messages(sender, child_id.into())?
            .await
            .map_err(|_| "子邮箱查询关闭")??,
    );
    let messages = messages
        .into_iter()
        .map(|message| (message.message_id.clone(), message))
        .collect::<HashMap<_, _>>();
    let native_ack = |message: &LocalCliMessage| {
        message.state == LocalCliMessageState::Acknowledged
            && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
    };
    let initial_row = messages
        .get(&initial.to_string())
        .ok_or("父初始输入记录缺失")?;
    let child_inputs = messages
        .values()
        .filter(|message| message.subject == "user_input" && message.recipient_task_id == child_id)
        .collect::<Vec<_>>();
    let results = messages
        .values()
        .filter(|message| {
            message.subject == "local_task_result"
                && message.sender_task_id == child_id
                && message.recipient_task_id == parent_id
        })
        .collect::<Vec<_>>();
    if child_inputs.len() != 1 || results.len() != 1 {
        return Err("子输入或自动结果数量不符".into());
    }
    if initial_row.body.contains(&fixture.search_marker)
        || child_inputs[0].body.contains(&fixture.search_marker)
    {
        return Err("搜索隐藏答案泄漏到父子初始输入".into());
    }
    if !native_ack(initial_row) || !native_ack(child_inputs[0]) || !native_ack(results[0]) {
        return Ok(false);
    }
    let calls = messages
        .values()
        .filter(|message| message.subject == "native_tool_call")
        .collect::<Vec<_>>();
    if calls.len() != if outside.is_some() { 2 } else { 1 } {
        return Ok(false);
    }
    if let Some(id) = outside {
        let row = messages
            .get(&id.to_string())
            .ok_or("越界回合原输入记录缺失")?;
        if !native_ack(row) {
            return Ok(false);
        }
        let mut refused = 0;
        for message in calls {
            let request: NativeLocalToolRequest =
                serde_json::from_str(&message.body).map_err(|error| error.to_string())?;
            if request.tool != "run_agents" || message.sender_task_id != parent_id {
                return Err("出现其它协调器工具调用".into());
            }
            if request.arguments == *outside_spawn {
                let id = Uuid::parse_str(&message.message_id).map_err(|error| error.to_string())?;
                let mut hash = Sha256::new();
                hash.update(id.as_bytes());
                hash.update(b"result");
                let receipt_digest = hash.finalize();
                let result_id =
                    Uuid::from_bytes(receipt_digest[..16].try_into().expect("SHA-256 固定长度"))
                        .to_string();
                let Some(result) = messages.get(&result_id) else {
                    return Ok(false);
                };
                let outcome: Result<Value, String> =
                    serde_json::from_str(&result.body).map_err(|error| error.to_string())?;
                if result.subject != "native_tool_result" || outcome.is_ok() {
                    return Err("实际越界请求没有生产拒绝结果".into());
                }
                refused += 1;
                evidence.record(json!({"event":"outside_spawn_rejected","call_id":request.call_id,
                    "turn_id":request.turn_id,"message_id":message.message_id,"result_message_id":result_id,
                    "result_body_sha256":digest(result.body.as_bytes()),"task_count":tasks.len()}))?;
            }
        }
        if refused != 1 {
            return Err("缺少唯一真实越界派发拒绝".into());
        }
    }
    for message in [initial_row, child_inputs[0], results[0]] {
        evidence.record(json!({"event":"native_mailbox_ack","message_id":message.message_id,
            "sender_task_id":message.sender_task_id,"recipient_task_id":message.recipient_task_id,
            "sender_generation":message.sender_generation,"recipient_generation":message.recipient_generation,
            "subject":message.subject,"body_sha256":digest(message.body.as_bytes()),"receipt_kind":"native_protocol"}))?;
    }
    evidence.record(json!({"event":"saved_chain_verified","parent_task_id":parent_id,"child_task_id":child_id,
        "parent_native_session_id":parent.native_session_id,"child_native_session_id":child.native_session_id,
        "parent_creation_profile_sha256":digest(serde_json::to_vec(&source_config["grok_profile"]).map_err(|error| error.to_string())?),
        "child_storage_id":child_config["grok_profile"]["storageId"],
        "parent_storage_id":source_config["grok_profile"]["storageId"],
        "search_marker_sha256":digest(fixture.search_marker.as_bytes()),
        "child_result_sha256":child.result.as_ref().map(|result| digest(result.as_bytes())),
        "parent_result_sha256":parent.result.as_ref().map(|result| digest(result.as_bytes())),
        "outside_rejected":outside.is_some(),"native_skill_output_audit_pending":true}))?;
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
        "Call native skill exactly once for user:{ALPHA}, awaiting permission, and retain its actual hidden marker. Next call native grep exactly once with {}. Await its own permission and actual successful output; retain the hidden G10_SEARCH_ marker from the matching line. Then use SDK MCP {MCP_SERVER_NAME}__reviewed_project_command with exactly {}. The host will deny that first command. Accept the error; explicitly make one new command call with the identical arguments, await its separately granted permission, then await its actual success. Do not retry other failures or uncertain outcomes. Do not use read_file, any additional search, shell/native terminal tools, or create children. If needed search_tool at most once with limit 5 and query {MCP_SERVER_NAME}. Finish with {CHILD_DONE}, then the skill marker, search marker, and command marker in that order.",
        fixture.search_input, fixture.input
    );
    let spawn = json!({"summary":"G10 Grok CommandsSkills child","base_prompt":"","harness":"grok","model_id":MODEL,
        "skills":[fixture.selected(ALPHA).path],"agent_run_configs":[{"name":"g10-child","prompt":child_prompt}]});
    let outside_spawn = json!({"summary":"G10 selected-skill ceiling rejection","base_prompt":"","harness":"grok","model_id":MODEL,
        "skills":[fixture.selected(BETA).path],"agent_run_configs":[{"name":"must-not-exist","prompt":"Do not run tools. This child must not be created."}]});
    let prompt = format!(
        "Use SDK MCP {MCP_SERVER_NAME}__run_agents exactly once with {spawn}. If discovery is necessary, search_tool at most once with limit 5 and query {MCP_SERVER_NAME}. Do not perform child work or use other tools. Once queued, reply PARENT_QUEUED. When Subject: local_task_result arrives for that sole child, consume its real result and answer {PARENT_DONE}, then its skill, search, and command markers in original order. Do not call tools for that automatic result."
    );
    if prompt.contains(&fixture.alpha_marker) || prompt.contains(&fixture.search_marker) {
        return Err("隐藏技能或搜索标记进入 prompt".into());
    }
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("固定 Grok 缺失")?,
        ),
        cwd: fixture.project.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::GrokReviewedCommandsSkillsV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: Some(MODEL.into()),
        local_tools: Some(LocalToolPermissions {
            allow_spawn: true,
            allow_message: false,
            allow_project_commands: true,
        }),
        selected_skills: vec![fixture.selected(ALPHA)],
    };
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: fixture.project.to_string_lossy().into(),
        config_json: json!({"model":MODEL,"permission_policy":"GrokReviewedCommandsSkillsV1"})
            .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let initial = Uuid::new_v4();
    evidence.record(json!({"event":"run_started","parent_task_id":parent_id,"initial_message_id":initial,
        "runtime_generation":options.generation,"requested_model":MODEL,"actual_model_verified":false}))?;
    coordinator.update(app, |model, ctx| {
        model.start_with_input(
            task,
            options,
            None,
            initial,
            vec![InputContent::Text(prompt)],
            ctx,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(450);
    let mut identities = HashMap::<String, (Uuid, String)>::new();
    let mut ready = HashSet::new();
    let mut spawn_permissions = HashSet::new();
    let mut child_id = None;
    let mut decisions = Vec::<Decision>::new();
    let mut commands = Vec::<usize>::new();
    let mut command_calls = HashSet::new();
    let mut skill_calls = HashSet::new();
    let mut searches = HashSet::new();
    let mut file_search_calls = HashSet::new();
    let mut spawn_calls = HashSet::new();
    let mut outside = None;
    let mut seen = 0;
    loop {
        if Instant::now() >= deadline {
            return Err("真实链超出 450 秒".into());
        }
        let wake = match receiver
            .recv()
            .with_timeout(
                Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await
        {
            Ok(Some(wake)) => wake,
            Ok(None) => return Err("事件流关闭".into()),
            Err(_) => None,
        };
        if let Some((task, event)) = wake {
            seen += 1;
            if seen > 768 {
                return Err("事件超出固定预算".into());
            }
            let parent = task.task_id == parent_id;
            if !parent {
                if task.parent_task_id.as_deref() != Some(parent_id.as_str())
                    || child_id.as_ref().is_some_and(|id| id != &task.task_id)
                {
                    return Err("出现额外子任务".into());
                }
                child_id = Some(task.task_id.clone());
            }
            if let Some(sid) = &event.native_session_id {
                Uuid::parse_str(sid).map_err(|_| "原生 SID 格式错误")?;
                if identities
                    .insert(task.task_id.clone(), (event.generation, sid.clone()))
                    .is_some_and(|old| old != (event.generation, sid.clone()))
                {
                    return Err("原生连接身份改变".into());
                }
            }
            match &event.kind {
                RuntimeEventKind::SessionReady { verified_cli_version, effective_permissions } => {
                    if verified_cli_version.as_deref() != Some(CLI_VERSION) || event.native_session_id.is_none()
                        || !ready.insert(task.task_id.clone())
                        || effective_permissions["requestedPolicy"] != "GrokReviewedCommandsSkillsV1"
                        || effective_permissions["appCreationPolicyApplied"] != true
                        || effective_permissions["permissionEnforcementVerified"] != false {
                        return Err("Grok 实际创建策略或版本没有配对".into());
                    }
                    evidence.record(json!({"event":"session_ready","task_id":task.task_id,
                        "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                        "verified_cli_version":verified_cli_version,"permissions_sha256":digest(serde_json::to_vec(effective_permissions).map_err(|error| error.to_string())?)}))?;
                }
                RuntimeEventKind::ApprovalRequested { approval_id, turn_id, method, details } => {
                    if !ready.contains(&task.task_id) || decisions.iter().any(|row| row.task_id==task.task_id && row.approval_id==*approval_id) { return Err("审批 ID 重复".into()); }
                    let mut command_generation = None;
                    let decision = if method == "infinishell/reviewed_project_command" && !parent {
                        let step = commands.len();
                        if step >= 2 { return Err("额外宿主命令请求".into()); }
                        fixture.verify_sources()?;
                        let generation: Uuid = serde_json::from_value(details["commandId"].clone()).map_err(|error| error.to_string())?;
                        let mut expected = fixture.command.approval_context();
                        expected["commandId"] = json!(generation); expected["parentGeneration"] = json!(event.generation);
                        if *details != expected || *approval_id != format!("project-command:{generation}")
                            || decisions.iter().any(|row| row.command_generation==Some(generation)) { return Err("命令审批参数或独立代次不符".into()); }
                        if step == 0 { fixture.require_absent()?; }
                        else {
                            decisions[commands[0]].denied_without_spawn(fixture)?;
                            evidence.record(json!({"event":"denied_without_spawn",
                                "command_generation":decisions[commands[0]].command_generation,
                                "zero_command_record":true,"marker_absent":true}))?;
                        }
                        command_generation = Some(generation); commands.push(decisions.len());
                        if step == 0 { ApprovalDecision::DenyOnce } else { ApprovalDecision::AllowOnce }
                    } else if method == "session/request_permission" && details["sessionId"] == event.native_session_id.as_deref().unwrap_or("") {
                        let call = &details["toolCall"]; let raw = &call["rawInput"];
                        let name = call["_meta"]["x.ai/tool"]["name"].as_str().ok_or("原生审批工具名缺失")?;
                        let call_id = call["toolCallId"].as_str().filter(|id| !id.is_empty()).ok_or("原生工具 call ID 缺失")?;
                        if name == "skill" && !parent && call["kind"] == "other"
                            && *raw == json!({"variant":"Dynamic","name":format!("user:{ALPHA}")}) && skill_calls.is_empty() {
                            skill_calls.insert(call_id.to_owned());
                        } else if name == "grep" && !parent && call["kind"] == "search"
                            && *raw == fixture.search_approval() && file_search_calls.is_empty() {
                            fixture.verify_sources()?;
                            file_search_calls.insert(call_id.to_owned());
                            evidence.record(json!({"event":"file_search_permission","task_id":task.task_id,
                                "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                                "approval_id":approval_id,"turn_id":turn_id,"call_id":call_id,
                                "arguments_sha256":digest(serde_json::to_vec(&fixture.search_input).map_err(|error| error.to_string())?),
                                "file_sha256":fixture.prepared["files_sha256"]["project/search-target.txt"],
                                "marker_sha256":digest(fixture.search_marker.as_bytes())}))?;
                        } else if name == "search_tool" && (*raw == json!({"variant":"SearchTool","limit":5,"query":MCP_SERVER_NAME})
                            || *raw == json!({"limit":5,"query":MCP_SERVER_NAME})) && searches.insert(task.task_id.clone()) {
                        } else if name == "use_tool" && call["kind"] == "other" && raw["variant"] == "UseTool"
                            && raw.as_object().is_some_and(|raw| raw.len()==3) {
                            if parent && raw["tool_name"] == format!("{MCP_SERVER_NAME}__run_agents")
                                && raw["tool_input"] == (if outside.is_some() { outside_spawn.clone() } else { spawn.clone() })
                                && spawn_permissions.len() < 2 && spawn_permissions.insert(call_id.to_owned()) {
                            } else if !parent && raw["tool_name"] == format!("{MCP_SERVER_NAME}__reviewed_project_command")
                                && raw["tool_input"] == fixture.input && command_calls.len()<2 && command_calls.insert(call_id.to_owned()) {
                            } else { return Err("原生 SDK 目标或参数越界".into()); }
                        } else { return Err("原生工具不是本轮固定技能/文件搜索/命令/派发".into()); }
                        ApprovalDecision::AllowOnce
                    } else { return Err("出现其它审批".into()); };
                    let message_id = control(app,coordinator,&task,RuntimeAction::RespondApproval { approval_id:approval_id.clone(),decision }).await?;
                    evidence.record(json!({"event":"approval_decision","task_id":task.task_id,"runtime_generation":event.generation,
                        "native_session_id":event.native_session_id,"approval_id":approval_id,"turn_id":turn_id,"method":method,
                        "command_generation":command_generation,"message_id":message_id,"decision":decision,
                        "details_sha256":digest(serde_json::to_vec(details).map_err(|error| error.to_string())?)}))?;
                    decisions.push(Decision { task_id:task.task_id.clone(),approval_id:approval_id.clone(),turn_id:turn_id.clone(),
                        runtime_generation:event.generation,command_generation,message_id,decision,resolved:false,dispatched:false });
                }
                RuntimeEventKind::ApprovalResolved { approval_id, decision } => {
                    let row = decisions.iter_mut().find(|row| row.task_id==task.task_id && row.approval_id==*approval_id).ok_or("未知审批解决")?;
                    if row.resolved || row.decision!=*decision { return Err("审批重复或变更".into()); } row.resolved=true;
                    evidence.record(json!({"event":"approval_resolved","task_id":task.task_id,
                        "runtime_generation":event.generation,"approval_id":approval_id,"decision":decision}))?;
                }
                RuntimeEventKind::CommandDispatched { message_id, .. } => {
                    if let Some(row) = decisions.iter_mut().find(|row| row.message_id==*message_id) {
                        if row.dispatched || row.task_id!=task.task_id { return Err("控制确认重复或串任务".into()); } row.dispatched=true;
                        evidence.record(json!({"event":"approval_control_dispatched","task_id":task.task_id,
                            "runtime_generation":event.generation,"approval_id":row.approval_id,"message_id":message_id}))?;
                    }
                }
                RuntimeEventKind::LocalToolRequested { request } => {
                    if !parent || request.tool!="run_agents" || spawn_calls.len() >= 2 || !spawn_calls.insert(request.call_id.clone())
                        || request.arguments != (if outside.is_some() { outside_spawn.clone() } else { spawn.clone() }) {
                        return Err("协调器工具请求不符".into());
                    }
                    evidence.record(json!({"event":"spawn_requested","task_id":task.task_id,"native_session_id":event.native_session_id,
                        "turn_id":request.turn_id,"call_id":request.call_id,"outside":outside.is_some()}))?;
                }
                RuntimeEventKind::MessageAccepted { message_id, turn_id } => evidence.record(json!({"event":"native_input_ack",
                    "task_id":task.task_id,"native_session_id":event.native_session_id,"message_id":message_id,"turn_id":turn_id}))?,
                RuntimeEventKind::TurnStarted { turn_id } => evidence.record(json!({"event":"turn_started","task_id":task.task_id,
                    "native_session_id":event.native_session_id,"runtime_generation":event.generation,"turn_id":turn_id}))?,
                RuntimeEventKind::TurnFinished { outcome, turn_id, .. } => {
                    if *outcome!=TurnOutcome::Completed { return Err("模型回合非正常完成".into()); }
                    evidence.record(json!({"event":"turn_finished","task_id":task.task_id,"native_session_id":event.native_session_id,"turn_id":turn_id,"outcome":"completed"}))?;
                }
                RuntimeEventKind::TextDelta { .. } | RuntimeEventKind::Progress { .. } => {},
                RuntimeEventKind::RequestFailed { .. } | RuntimeEventKind::Disconnected { .. }
                | RuntimeEventKind::ApprovalCancelled { .. } | RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::InputJoined { .. } => return Err("原生连接或回合意外失败/合并".into()),
            }
        }
        if commands.len() == 2
            && decisions.iter().all(|row| row.resolved && row.dispatched)
            && skill_calls.len() == 1
            && file_search_calls.len() == 1
            && command_calls.len() == 2
            && spawn_calls.len() == (if outside.is_some() { 2 } else { 1 })
            && let Some(child) = child_id.as_deref()
            && verify_saved(
                sender,
                &parent_id,
                child,
                fixture,
                initial,
                outside,
                &outside_spawn,
                evidence,
            )
            .await?
        {
            verify_command_exit(fixture, &decisions[commands[1]], &command_calls, evidence)?;
            decisions[commands[0]].denied_directory_still_absent()?;
            let allowed = decisions[commands[1]].command_state()?;
            let count = fs::read_dir(allowed.parent().ok_or("命令父目录缺失")?)
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
                .len();
            if count != 1 {
                return Err("实际命令监督目录并非唯一允许项".into());
            }
            if outside.is_some() {
                evidence.record(json!({"event":"rust_chain_completed","commands_executed":1,"command_requests":2,
                    "children_created":1,"outside_spawn_rejected":true,"file_search_calls":1,"native_search_output_audit_pending":true,"native_skill_output_audit_pending":true,"gap_closed":false}))?;
                return Ok(());
            }
            let parent = coordinator
                .read(app, |model, _| {
                    model
                        .snapshot(&parent_id)
                        .map(|snapshot| snapshot.task.clone())
                })
                .ok_or("父快照缺失")?;
            let prompt = format!(
                "This is the explicit ceiling test. Call {MCP_SERVER_NAME}__run_agents once with {outside_spawn}. It selects a skill outside your parent ceiling and must be rejected before creating any child. Do not work around the rejection or retry; reply only {OUTSIDE_DONE} after the tool returns that error."
            );
            outside = Some(
                control(
                    app,
                    coordinator,
                    &parent,
                    RuntimeAction::Submit {
                        input: vec![InputContent::Text(prompt)],
                    },
                )
                .await?,
            );
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
#[ignore = "仅由私有短目录运行器执行；固定 Grok 1.0.41 的真实父子命令链会产生模型费用"]
fn real_grok_g10_commands_skills_parent_child() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    let temporary = PathBuf::from(env::var_os("TMPDIR").expect("缺少本轮短 TMPDIR"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        temporary.parent().unwrap(),
        Path::new("/Users/zhishi/InfiniShell-Tests")
    );
    assert_eq!(root, temporary.join("g10-grok-commands-skills"));
    assert_eq!(
        read_owned(&root.join(".infinishell-g10-grok-commands-skills"), 256).unwrap(),
        format!("{SCOPE}\n").as_bytes()
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_EXPECTED_VERSION").unwrap(),
        CLI_VERSION
    );
    assert_eq!(env::var("INFINISHELL_GROK_LIVE_MODEL").unwrap(), MODEL);
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    assert!(env::var_os("INFINISHELL_GROK_LIVE_CONFIG_DIR").is_none());
    let home = PathBuf::from(env::var_os("HOME").expect("保留默认账号 HOME"))
        .canonicalize()
        .unwrap();
    assert!(!home.starts_with(&temporary));
    let profile = env::var("INFINISHELL_GROK_LIVE_STATE_PROFILE").unwrap();
    let nonce = profile
        .strip_prefix("grok-g10-commands-skills-")
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
    evidence.record(json!({"event":"acceptance_started","scope":SCOPE,"cli_version":CLI_VERSION,
        "requested_model":MODEL,"actual_model_verified":false,"policy":"GrokReviewedCommandsSkillsV1","max_children":1,
        "max_host_command_requests":2,"max_command_executions":1,"max_native_file_search_calls":1,"real_gui_verified":false,
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
        let executable = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap());
        app.add_singleton_model(|_| {
            CLIAgentInstallModel::with_installation_for_test(
                CLIAgent::Grok,
                CLIAgentInstallation {
                    executable: Some(executable),
                    version: CLIAgentVersionStatus::Detected(CLI_VERSION.into()),
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
                .record(json!({"event":"rust_chain_completed_pending_native_audit","scope":SCOPE,
                "file_search_calls":1,"native_search_output_audit_pending":true,
                "host_decisions":["DenyOnce","AllowOnce"],"command_executions":1,
                "parent_child_input_and_result_native_ack":true,"all_runtime_hosts_cleaned":true,
                "gap_closed":false,"real_gui_verified":false,"actual_model_verified":false,"native_skill_output_audit_pending":true}))
                .unwrap(),
            Err(_) => panic!("真实 G10 受审命令最小父子链失败；查看本轮白名单阶段证据"),
        }
    });
}
