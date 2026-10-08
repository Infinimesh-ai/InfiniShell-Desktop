//! 固定 Claude 真实父子命令取消；只核本轮一条运行命令及其真实 Node 后代。

use command::managed::{
    MacosCoalition, MacosProcessIdentity, macos_boot_session, macos_process_identity,
};

use super::*;

const SCOPE: &str = "real_claude_g10_running_command_cancel_v1";
const SEARCH_DONE: &str = "G10_SEARCH_DONE";

struct CancelFixture {
    project: PathBuf,
    prepared: Value,
    ceiling: ReviewedCommandCeilingV1,
    input: Value,
    command: ReviewedCommand,
    search: Value,
    marker: String,
}

impl CancelFixture {
    fn load(root: &Path) -> Result<Self, String> {
        let project = root
            .join("project")
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let prepared = json_file(&root.join("prepared.safe.json"), 64 * 1024)?;
        if prepared["scope"] != SCOPE || prepared["project"] != json!(project) {
            return Err("运行命令取消夹具绑定不符".into());
        }
        let ceiling =
            ReviewedCommandCeilingV1::capture(&project).map_err(|error| error.to_string())?;
        let value = serde_json::to_value(&ceiling).map_err(|error| error.to_string())?;
        let bindings = value["unix"]["bindings"]
            .as_array()
            .ok_or("缺少真实 Unix 上限")?;
        let selected = bindings
            .iter()
            .filter(|row| row["capability"] == json!({"packageScript":"g10-wait"}))
            .collect::<Vec<_>>();
        let [binding] = selected.as_slice() else {
            return Err("没有唯一的 Node/npm 固定脚本入口".into());
        };
        let input = json!({"executable":binding["executable"],"argv":binding["arguments"],
            "cwd":project,"timeoutMs":120000});
        let command = ceiling
            .approve_host(&input)
            .ok_or("取消夹具命令不在生产上限内")?;
        let marker = String::from_utf8(read_owned(
            &root.join("expected-search-marker.private"),
            256,
        )?)
        .map_err(|error| error.to_string())?;
        if digest(marker.as_bytes()) != prepared["search_marker_sha256"] {
            return Err("搜索答案摘要不符".into());
        }
        let search = json!({"pattern":prepared["search_needle"],"path":project.join("search.txt"),
            "output_mode":"content","-n":false});
        let fixture = Self {
            project,
            prepared,
            ceiling,
            input,
            command,
            search,
            marker,
        };
        fixture.verify_sources()?;
        fixture.require_absent()?;
        Ok(fixture)
    }

    fn verify_sources(&self) -> Result<(), String> {
        self.ceiling
            .verify_sources()
            .map_err(|error| error.to_string())?;
        for name in ["package.json", "g10-running.cjs", "search.txt"] {
            if self.prepared["files_sha256"][name]
                != digest(read_owned(&self.project.join(name), 64 * 1024)?)
            {
                return Err("取消验收项目源码已改变".into());
            }
        }
        Ok(())
    }

    fn require_absent(&self) -> Result<(), String> {
        for name in [
            "parent.ready.json",
            "child.ready.json",
            "parent.heartbeat",
            "child.heartbeat",
        ] {
            match fs::symlink_metadata(self.project.join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) | Err(_) => return Err("执行前就绪或活动记录已存在".into()),
            }
        }
        Ok(())
    }
}

fn identity(value: &Value) -> Result<MacosProcessIdentity, String> {
    let pid = value["pid"]
        .as_i64()
        .and_then(|pid| i32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .ok_or("进程 PID 无效")?;
    let pid_version = value["pid_version"]
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
        .ok_or("进程代际无效")?;
    let unique_id = value["unique_id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("内核生存期 ID 无效")?;
    let resource_cid = value["resource_cid"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("资源域无效")?;
    Ok(MacosProcessIdentity {
        pid,
        pid_version,
        unique_id,
        resource_cid,
    })
}

fn identity_value(identity: MacosProcessIdentity) -> Value {
    json!({"pid":identity.pid,"pid_version":identity.pid_version,
        "unique_id":identity.unique_id,"resource_cid":identity.resource_cid})
}

struct RunningCommand {
    state: PathBuf,
    generation: Uuid,
    boot_session: String,
    members: [MacosProcessIdentity; 2],
    claim_sha256: String,
    native_sha256: String,
    record_sha256: String,
}

fn inspect_running(
    fixture: &CancelFixture,
    decision: &Decision,
    identities: &HashMap<String, (Uuid, String)>,
    evidence: &mut Evidence,
) -> Result<Option<RunningCommand>, String> {
    let state = decision.command_state()?;
    for name in ["parent.ready.json", "child.ready.json"] {
        match fs::symlink_metadata(fixture.project.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
    }
    fixture.verify_sources()?;
    if !decision.dispatched
        || !decision.resolved
        || decision.decision != ApprovalDecision::AllowOnce
    {
        return Ok(None);
    }
    let generation = decision.command_generation.ok_or("缺少命令代次")?;
    let record_bytes = read_owned(&state.join("reviewed-command.json"), 16 * 1024)?;
    let record: Value = serde_json::from_slice(&record_bytes).map_err(|error| error.to_string())?;
    let call_id = record["callId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("缺少实际 MCP ID")?;
    let bytes = Sha256::digest(
        serde_json::to_vec(&(
            "infinishell-project-command-v1",
            decision.runtime_generation,
            decision.turn_id.as_str(),
            call_id,
        ))
        .map_err(|error| error.to_string())?,
    );
    let derived = Uuid::from_bytes(bytes[..16].try_into().expect("固定 SHA 长度"));
    let command_sha =
        digest(serde_json::to_vec(&fixture.command).map_err(|error| error.to_string())?);
    if generation != derived
        || record["version"] != 1
        || record["platform"] != "macos"
        || record["parentGeneration"] != json!(decision.runtime_generation)
        || record["commandGeneration"] != json!(generation)
        || record["turnId"] != decision.turn_id
        || record["approvalId"] != decision.approval_id
        || record["commandSha256"] != command_sha
    {
        return Err("运行命令领取记录不属于精确审批".into());
    }
    let process = state
        .join("cli-agent-processes")
        .join(generation.to_string());
    let claim_bytes = read_owned(&process.join("macos-coalition.json"), 16 * 1024)?;
    let native_bytes = read_owned(&process.join("macos-native.json"), 16 * 1024)?;
    let manifest_bytes = read_owned(&process.join("manifest.json"), 1024 * 1024)?;
    let claim: Value = serde_json::from_slice(&claim_bytes).map_err(|error| error.to_string())?;
    let native: Value = serde_json::from_slice(&native_bytes).map_err(|error| error.to_string())?;
    let wrapper = identity(&claim["wrapper"])?;
    let native_identity = identity(&native["identity"])?;
    let boot_session = macos_boot_session().map_err(|error| error.to_string())?;
    if claim["version"] != 1
        || claim["generation"] != json!(generation)
        || claim["manifest_sha256"] != digest(manifest_bytes)
        || claim["label"] != format!("dev.infinishell.cli-agent.{generation}")
        || claim["boot_session"] != boot_session
        || native["generation"] != json!(generation)
        || native["claim_sha256"] != digest(&claim_bytes)
        || native_identity.resource_cid != wrapper.resource_cid
        || macos_process_identity(native_identity.pid).map_err(|error| error.to_string())?
            != native_identity
        || macos_process_identity(wrapper.pid).map_err(|error| error.to_string())? != wrapper
        || MacosCoalition::is_destroyed(wrapper.resource_cid, &boot_session)
            .map_err(|error| error.to_string())?
    {
        return Err("运行命令的原生资源域或内核身份没有绑定".into());
    }
    let ready = [
        json_file(&fixture.project.join("parent.ready.json"), 4096)?,
        json_file(&fixture.project.join("child.ready.json"), 4096)?,
    ];
    let member = |row: &Value, role: &str| -> Result<MacosProcessIdentity, String> {
        if row["version"] != 1
            || row["role"] != role
            || digest(
                row["nonce"]
                    .as_str()
                    .ok_or("候选记录缺少随机值")?
                    .as_bytes(),
            ) != fixture.prepared["nonce_sha256"]
            || row["executable"] != fixture.input["executable"]
        {
            return Err("候选就绪记录不属于固定脚本".into());
        }
        let pid = row["pid"]
            .as_i64()
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or("候选 PID 无效")?;
        let actual = macos_process_identity(pid).map_err(|error| error.to_string())?;
        if actual.resource_cid != wrapper.resource_cid || actual.unique_id == 0 {
            return Err("候选进程不在此命令实际资源域中".into());
        }
        Ok(actual)
    };
    let members = [member(&ready[0], "parent")?, member(&ready[1], "child")?];
    if members[0].pid == members[1].pid
        || members[0].unique_id == members[1].unique_id
        || ready[0]["child_pid"] != json!(members[1].pid)
        || ready[1]["ppid"] != json!(members[0].pid)
        || members
            .iter()
            .any(|member| member.pid == native_identity.pid || member.pid == wrapper.pid)
    {
        return Err("固定脚本未产生两个独立真实进程".into());
    }
    if identities.len() != 2 {
        return Err("未取得两个原生 CLI 身份".into());
    }
    for (runtime, native_session) in identities.values() {
        let path = current_state_dir()
            .join("cli-agent-hosts")
            .join(runtime.to_string())
            .join("native")
            .join("cli-agent-processes")
            .join(runtime.to_string());
        let cli_claim = json_file(&path.join("macos-coalition.json"), 16 * 1024)?;
        let cli_native = json_file(&path.join("macos-native.json"), 16 * 1024)?;
        let authorized_cli = identity(&cli_native["identity"])?;
        let actual_cli =
            macos_process_identity(authorized_cli.pid).map_err(|error| error.to_string())?;
        evidence.record(
            json!({"event":"cli_exec_identity_observed","runtime_generation":runtime,
            "native_session_id":native_session,"authorized_identity":identity_value(authorized_cli),
            "actual_identity":identity_value(actual_cli)}),
        )?;
        // 与生产 validate_after_exec 一致：exec 可变更 PID version，但不能变更生存期或资源域。
        if cli_claim["generation"] != json!(runtime)
            || cli_native["generation"] != json!(runtime)
            || cli_native["claim_sha256"]
                != digest(read_owned(&path.join("macos-coalition.json"), 16 * 1024)?)
            || actual_cli.resource_cid == wrapper.resource_cid
            || actual_cli.pid != authorized_cli.pid
            || actual_cli.unique_id != authorized_cli.unique_id
            || actual_cli.resource_cid != authorized_cli.resource_cid
        {
            return Err("命令与父子 CLI 资源域未保持独立".into());
        }
    }
    evidence.record(
        json!({"event":"running_command_owned","runtime_generation":decision.runtime_generation,
        "command_generation":generation,"record_sha256":digest(&record_bytes),
        "claim_sha256":digest(&claim_bytes),"native_sha256":digest(&native_bytes),
        "boot_session":boot_session,"command_cid":wrapper.resource_cid,
        "members":[identity_value(members[0]),identity_value(members[1])],
        "candidate_nonce_verified":true,"kernel_identity_verified":true,
        "distinct_from_both_native_cli_coalitions":true,"pid_signals_sent":0}),
    )?;
    Ok(Some(RunningCommand {
        state,
        generation,
        boot_session,
        members,
        claim_sha256: digest(claim_bytes),
        native_sha256: digest(native_bytes),
        record_sha256: digest(record_bytes),
    }))
}

fn verify_cancelled(
    fixture: &CancelFixture,
    decision: &Decision,
    running: &RunningCommand,
    evidence: &mut Evidence,
) -> Result<(), String> {
    fixture.verify_sources()?;
    let command_sha =
        digest(serde_json::to_vec(&fixture.command).map_err(|error| error.to_string())?);
    let binding = PreparedLaunchBinding::unix_reviewed_project_command(command_sha.clone())
        .map_err(|error| error.to_string())?;
    let receipt = confirmed_exit_with_binding(&running.state, running.generation, &binding)
        .map_err(|error| error.to_string())?
        .ok_or("取消终态发布前命令清理未确认")?;
    let process = running
        .state
        .join("cli-agent-processes")
        .join(running.generation.to_string());
    let result_bytes = read_owned(&running.state.join("result.json"), 512 * 1024)?;
    let result: Value = serde_json::from_slice(&result_bytes).map_err(|error| error.to_string())?;
    let stdout = read_owned(&running.state.join("stdout.bin"), 256 * 1024)?;
    let exit_bytes = read_owned(&process.join("exit.json"), 16 * 1024)?;
    if !receipt.cleanup_confirmed
        || receipt.containment != "macos_resource_coalition"
        || result["commandGeneration"] != json!(running.generation)
        || result["error"] != "reviewed_command_cancelled"
        || result["cleanup"] != json!(receipt)
        || result["exitCode"] != json!(receipt.exit_code)
        || result["outputSha256"] != digest(&stdout)
        || result["outputBytes"] != json!(stdout.len())
        || result["outputPreviewTruncated"] != false
        || serde_json::from_slice::<Value>(&exit_bytes).map_err(|error| error.to_string())?
            != json!(receipt)
        || digest(read_owned(
            &process.join("macos-coalition.json"),
            16 * 1024,
        )?) != running.claim_sha256
        || digest(read_owned(&process.join("macos-native.json"), 16 * 1024)?)
            != running.native_sha256
        || digest(read_owned(
            &running.state.join("reviewed-command.json"),
            16 * 1024,
        )?) != running.record_sha256
        || !MacosCoalition::is_destroyed(running.members[0].resource_cid, &running.boot_session)
            .map_err(|error| error.to_string())?
    {
        return Err("真实取消结果、独立清理收据或资源域销毁不符".into());
    }
    for expected in running.members {
        if macos_process_identity(expected.pid).is_ok_and(|actual| actual == expected) {
            return Err("取消后精确生存期的原命令成员仍存在".into());
        }
    }
    let command_parent = running.state.parent().ok_or("命令缺少父目录")?;
    let actual = fs::read_dir(command_parent)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if actual.len() != 1 || actual[0].path() != running.state {
        return Err("取消链出现后续命令派生".into());
    }
    evidence.record(json!({"event":"command_cancel_cleanup_verified","runtime_generation":decision.runtime_generation,
        "command_generation":running.generation,"command_sha256":command_sha,
        "record_sha256":running.record_sha256,"result_sha256":digest(result_bytes),
        "exit_receipt_sha256":digest(exit_bytes),"stdout_sha256":digest(&stdout),"stdout_bytes":stdout.len(),
        "error":"reviewed_command_cancelled","cleanup_confirmed":true,
        "exit_code":receipt.exit_code,"containment":receipt.containment,
        "live_cid_destroyed":true,"exact_member_identities_gone":true,
        "subsequent_command_spawn_count":0,"queued_command_cancel_verified":false}))
}

async fn drive_cancel(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    fixture: &CancelFixture,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let parent_id = Uuid::new_v4().to_string();
    let child_prompt = format!(
        "First call native Grep exactly with {} once. Wait for its actual result, then say {SEARCH_DONE} followed by the actual matching line. Do not guess the hidden marker. Then request the SDK MCP reviewed_project_command exactly with {} once. Wait for the result. The application will interrupt this running command. Do not retry or call any further tool after the interrupt. Do not use Read, Write, Edit, Bash, skills or child agents.",
        fixture.search, fixture.input
    );
    let spawn = json!({"summary":"isolated running project command cancellation","base_prompt":"",
        "harness":"claude","model_id":MODEL,"skills":[],
        "agent_run_configs":[{"name":"running-command-cancel","prompt":child_prompt}]});
    let prompt = format!(
        "Call run_agents exactly once with {spawn}. Do not use other tools or do the child work yourself. After the child is queued say PARENT_QUEUED. If a local_task_result arrives, acknowledge it without tools or retries."
    );
    if prompt.contains(&fixture.marker) {
        return Err("提示泄露随机搜索答案".into());
    }
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
    evidence.record(json!({"event":"run_started","parent_task_id":parent_id,"runtime_generation":options.generation,
        "initial_message_id":initial_message,"requested_model":MODEL,"actual_model_verified":false}))?;
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
    let mut paired = HashSet::new();
    let mut child_id = None;
    let mut child_turn = None;
    let mut child_task = None;
    let mut decisions = Vec::<Decision>::new();
    let mut command_decision = None;
    let mut grep_id = None;
    let mut native_command_seen = false;
    let mut spawn_seen = false;
    let mut search_answer = false;
    let mut search_text = String::new();
    let mut running = None;
    let mut interrupt_id = None;
    let mut interrupt_ack = false;
    let mut child_cancelled = false;
    let mut seen = 0;
    loop {
        if Instant::now() >= deadline {
            return Err("运行命令取消链超过 450 秒".into());
        }
        if coordinator.read(app, |model, _| {
            model.snapshots().any(|row| row.error.is_some())
        }) {
            return Err("协调器报告实际任务错误".into());
        }
        let wake = match receiver
            .recv()
            .with_timeout(Duration::from_millis(100))
            .await
        {
            Ok(Some(wake)) => wake,
            Ok(None) => return Err("真实事件通道关闭".into()),
            Err(_) => None,
        };
        if let Some((task, event)) = wake {
            seen += 1;
            if seen > 768 {
                return Err("事件超过验收预算".into());
            }
            let is_parent = task.task_id == parent_id;
            if !is_parent {
                if task.parent_task_id.as_deref() != Some(parent_id.as_str())
                    || child_id.as_ref().is_some_and(|id| id != &task.task_id)
                {
                    return Err("出现额外或无关子任务".into());
                }
                child_id = Some(task.task_id.clone());
                child_task = Some(task.clone());
            }
            if let Some(sid) = &event.native_session_id {
                Uuid::parse_str(sid).map_err(|_| "原生 SID 格式无效")?;
                if identities
                    .insert(task.task_id.clone(), (event.generation, sid.clone()))
                    .is_some_and(|old| old != (event.generation, sid.clone()))
                {
                    return Err("原生连接身份改变".into());
                }
            }
            match &event.kind {
                RuntimeEventKind::SessionReady {
                    verified_cli_version: Some(version),
                    effective_permissions,
                } => {
                    if version != "2.1.280"
                        || event.native_session_id.is_none()
                        || effective_permissions["fixedProfileVerified"] != true
                        || effective_permissions["claudeReviewedCommandsV1"].is_null()
                    {
                        return Err("实际原生版本或策略未配对".into());
                    }
                    paired.insert(task.task_id.clone());
                    evidence.record(json!({"event":"session_ready","task_id":task.task_id,
                        "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                        "verified_cli_version":version,"permission_sha256":digest(serde_json::to_vec(effective_permissions).map_err(|error| error.to_string())?)}))?;
                }
                RuntimeEventKind::TurnStarted { turn_id } if !is_parent => {
                    if child_turn.replace(turn_id.clone()).is_some() {
                        return Err("子任务出现额外回合".into());
                    }
                }
                RuntimeEventKind::TextDelta { turn_id, text, .. }
                    if !is_parent && !search_answer =>
                {
                    if child_turn.as_deref() != Some(turn_id.as_str())
                        || search_text.len().saturating_add(text.len()) > 64 * 1024
                    {
                        return Err("搜索回答代次或累计预算不符".into());
                    }
                    search_text.push_str(text);
                    if search_text.contains(SEARCH_DONE) && search_text.contains(&fixture.marker) {
                        if grep_id.is_none() || command_decision.is_some() {
                            return Err("搜索回答顺序不符".into());
                        }
                        search_answer = true;
                        evidence.record(json!({"event":"search_answer_observed","task_id":task.task_id,
                            "native_session_id":event.native_session_id,"marker_sha256":digest(fixture.marker.as_bytes()),
                            "native_tool_result_audit_pending":true}))?;
                    }
                }
                RuntimeEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    method,
                    details,
                } => {
                    if interrupt_id.is_some()
                        || !paired.contains(&task.task_id)
                        || decisions.iter().any(|row| {
                            row.task_id == task.task_id && row.approval_id == *approval_id
                        })
                    {
                        return Err("取消后出现审批或审批身份未配对".into());
                    }
                    let mut command_generation = None;
                    if is_parent {
                        if !decisions.is_empty()
                            || method != "can_use_tool"
                            || details["tool_name"] != "mcp__infinishell-local-tasks__run_agents"
                            || details["input"] != spawn
                        {
                            return Err("父任务不是唯一精确派发审批".into());
                        }
                    } else if method == "can_use_tool" && details["tool_name"] == "Grep" {
                        if grep_id.is_some() || !spawn_seen || details["input"] != fixture.search {
                            return Err("搜索审批越界".into());
                        }
                        let id = details["tool_use_id"]
                            .as_str()
                            .filter(|id| {
                                id.starts_with("toolu_")
                                    && id.len() <= 160
                                    && id.bytes().all(|byte| {
                                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                                    })
                            })
                            .ok_or("搜索原生工具 ID 无效")?;
                        grep_id = Some(id.to_owned());
                        evidence.record(json!({"event":"grep_approved","task_id":task.task_id,
                            "native_session_id":event.native_session_id,"turn_id":turn_id,"tool_use_id":id,
                            "input_sha256":digest(serde_json::to_vec(&fixture.search).map_err(|error| error.to_string())?),
                            "marker_sha256":digest(fixture.marker.as_bytes()),"answer_not_in_prompt":true}))?;
                    } else if method == "can_use_tool" && details["tool_name"] == CLAUDE_TOOL {
                        if native_command_seen
                            || !search_answer
                            || details["input"] != fixture.input
                        {
                            return Err("原生命令审批越界或搜索尚未完成".into());
                        }
                        native_command_seen = true;
                    } else if method == "infinishell/reviewed_project_command" && !is_parent {
                        if command_decision.is_some()
                            || !native_command_seen
                            || child_turn.as_deref() != Some(turn_id)
                        {
                            return Err("宿主命令审批次数或回合不符".into());
                        }
                        fixture.verify_sources()?;
                        fixture.require_absent()?;
                        let generation: Uuid = serde_json::from_value(details["commandId"].clone())
                            .map_err(|error| error.to_string())?;
                        let mut expected = fixture.command.approval_context();
                        expected["commandId"] = json!(generation);
                        expected["parentGeneration"] = json!(event.generation);
                        if *details != expected
                            || *approval_id != format!("project-command:{generation}")
                        {
                            return Err("宿主命令未绑定精确参数".into());
                        }
                        command_generation = Some(generation);
                        command_decision = Some(decisions.len());
                    } else {
                        return Err("出现范围外工具审批".into());
                    }
                    let message_id = control(
                        app,
                        coordinator,
                        &task,
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision: ApprovalDecision::AllowOnce,
                        },
                    )
                    .await?;
                    decisions.push(Decision {
                        task_id: task.task_id.clone(),
                        approval_id: approval_id.clone(),
                        turn_id: turn_id.clone(),
                        runtime_generation: event.generation,
                        command_generation,
                        message_id,
                        decision: ApprovalDecision::AllowOnce,
                        resolved: false,
                        dispatched: false,
                    });
                    evidence.record(json!({"event":"approval_decision","task_id":task.task_id,"runtime_generation":event.generation,
                        "native_session_id":event.native_session_id,"approval_id":approval_id,"turn_id":turn_id,
                        "method":method,"command_generation":command_generation,"message_id":message_id,"decision":"allow_once"}))?;
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } => {
                    let row = decisions
                        .iter_mut()
                        .find(|row| row.task_id == task.task_id && row.approval_id == *approval_id)
                        .ok_or("未记录的审批解决")?;
                    if row.resolved || row.decision != *decision {
                        return Err("审批解决重复或决定变化".into());
                    }
                    row.resolved = true;
                }
                RuntimeEventKind::CommandDispatched { message_id, .. } => {
                    if let Some(row) = decisions
                        .iter_mut()
                        .find(|row| row.message_id == *message_id)
                    {
                        if row.dispatched || row.task_id != task.task_id {
                            return Err("审批控制串任务或重复".into());
                        }
                        row.dispatched = true;
                    }
                }
                RuntimeEventKind::LocalToolRequested { request } => {
                    if !is_parent
                        || spawn_seen
                        || request.tool != "run_agents"
                        || request.arguments != spawn
                    {
                        return Err("实际派发工具越界".into());
                    }
                    spawn_seen = true;
                }
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } if Some(*message_id) == interrupt_id => {
                    if is_parent || interrupt_ack || turn_id.as_deref() != child_turn.as_deref() {
                        return Err("Interrupt 原生 ACK 身份不符".into());
                    }
                    interrupt_ack = true;
                    evidence.record(json!({"event":"interrupt_native_ack","task_id":task.task_id,"native_session_id":event.native_session_id,
                        "runtime_generation":event.generation,"message_id":message_id,"turn_id":turn_id}))?;
                }
                RuntimeEventKind::TurnFinished {
                    turn_id, outcome, ..
                } if !is_parent => {
                    if child_cancelled
                        || !interrupt_ack
                        || *outcome != TurnOutcome::Cancelled
                        || child_turn.as_deref() != Some(turn_id)
                    {
                        return Err("子回合未取得原生确认取消终态".into());
                    }
                    verify_cancelled(
                        fixture,
                        &decisions[command_decision.ok_or("缺少命令审批")?],
                        running.as_ref().ok_or("未验证运行状态")?,
                        evidence,
                    )?;
                    child_cancelled = true;
                    evidence.record(json!({"event":"child_cancelled","task_id":task.task_id,"native_session_id":event.native_session_id,
                        "runtime_generation":event.generation,"turn_id":turn_id,"cleanup_before_terminal_observation":true}))?;
                }
                RuntimeEventKind::RequestFailed { .. } | RuntimeEventKind::Disconnected { .. } => {
                    return Err("实际连接或控制请求失败".into());
                }
                _ => {}
            }
        }
        if interrupt_id.is_none()
            && let Some(index) = command_decision
            && decisions.iter().all(|row| row.resolved && row.dispatched)
            && let Some(owned) = inspect_running(fixture, &decisions[index], &identities, evidence)?
        {
            // 就绪文件仅作候选；两次内核生存期与写入增长确认命令仍在运行。
            let before = [
                read_owned(&fixture.project.join("parent.heartbeat"), 64 * 1024)?,
                read_owned(&fixture.project.join("child.heartbeat"), 64 * 1024)?,
            ];
            Timer::after(Duration::from_millis(300)).await;
            for (index, name) in ["parent.heartbeat", "child.heartbeat"].iter().enumerate() {
                let after = read_owned(&fixture.project.join(name), 64 * 1024)?;
                if !after.starts_with(&before[index])
                    || after.len() <= before[index].len()
                    || macos_process_identity(owned.members[index].pid)
                        .map_err(|error| error.to_string())?
                        != owned.members[index]
                {
                    return Err("取消前精确命令成员已退出或未持续运行".into());
                }
            }
            let task = child_task.as_ref().ok_or("缺少子任务快照")?;
            let message_id = control(
                app,
                coordinator,
                task,
                RuntimeAction::Interrupt {
                    turn_id: child_turn.clone().ok_or("缺少子回合")?,
                },
            )
            .await?;
            interrupt_id = Some(message_id);
            evidence.record(json!({"event":"interrupt_application_enqueued","task_id":task.task_id,"generation":task.generation,
                "message_id":message_id,"turn_id":child_turn,"command_generation":owned.generation,
                "both_members_live_before_interrupt":true,"both_heartbeats_grew":true,"native_ack_observed":false}))?;
            running = Some(owned);
        }
        if child_cancelled && interrupt_ack {
            break;
        }
    }
    let child_id = child_id.ok_or("没有真实子任务")?;
    let saved = load_tasks(sender, false)?
        .await
        .map_err(|_| "持久任务查询关闭")??;
    let parent = saved
        .iter()
        .find(|row| row.task_id == parent_id)
        .ok_or("父记录丢失")?;
    let child = saved
        .iter()
        .find(|row| row.task_id == child_id)
        .ok_or("子记录丢失")?;
    let parent_config: Value =
        serde_json::from_str(&parent.config_json).map_err(|error| error.to_string())?;
    let child_config: Value =
        serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
    let history = load_task_generations(sender, parent_id.clone())?
        .await
        .map_err(|_| "父代查询关闭")??;
    let original = history
        .iter()
        .find(|row| row.generation == 1)
        .ok_or("原始父代丢失")?;
    let ceiling = ceiling_from_parent(original, "claude").map_err(|error| error.to_string())?;
    let profile = ceiling
        .claude_profile()
        .ok_or("父上限没有 Claude 固定策略")?;
    if serde_json::to_value(profile.host_commands().ok_or("父上限未包含真实命令")?)
        .map_err(|error| error.to_string())?
        != json!(fixture.ceiling)
    {
        return Err("持久父上限与本轮实际命令捕获不同".into());
    }
    if saved.len() != 2
        || child.parent_task_id.as_deref() != Some(parent_id.as_str())
        || child.parent_generation != Some(1)
        || child.generation != 1
        || child.state != LocalCliTaskState::Cancelled
        || child.terminal_evidence.is_none()
        || child_config["permission_policy"] != "ClaudeReviewedCommandsV1"
        || child_config["permission_ceiling"] != json!(ceiling)
        || child_config["claude_profile"] != parent_config["claude_profile"]
        || child_config["effective_permissions"]["claudeReviewedCommandsV1"]
            != parent_config["claude_profile"]
        || child_config["effective_permissions"]["fixedProfileVerified"] != true
        || parent.native_session_id.is_none()
        || child.native_session_id.is_none()
        || parent.native_session_id == child.native_session_id
    {
        return Err("持久父子身份、上限或取消终态不符".into());
    }
    let mut all = HashMap::new();
    for task_id in [&parent_id, &child_id] {
        for row in load_task_messages(sender, task_id.clone())?
            .await
            .map_err(|_| "输入查询关闭")??
        {
            all.insert(row.message_id.clone(), row);
        }
    }
    let inputs = all
        .values()
        .filter(|row| row.subject == "user_input")
        .collect::<Vec<_>>();
    if inputs.len() != 2
        || inputs.iter().any(|row| {
            row.state != LocalCliMessageState::Acknowledged
                || row.receipt_kind != Some(LocalCliReceiptKind::NativeProtocol)
        })
    {
        return Err("父子输入不是恰好两次原生 ACK".into());
    }
    evidence.record(json!({"event":"saved_cancel_chain_verified","parent_task_id":parent_id,"child_task_id":child_id,
        "parent_native_session_id":parent.native_session_id,"child_native_session_id":child.native_session_id,
        "parent_ceiling_saved":true,"child_state":"cancelled","input_native_acks":2,
        "initial_message_id":initial_message,"search_native_result_audit_pending":true}))?;
    let stable = [
        read_owned(&fixture.project.join("parent.heartbeat"), 64 * 1024)?,
        read_owned(&fixture.project.join("child.heartbeat"), 64 * 1024)?,
    ];
    Timer::after(Duration::from_millis(300)).await;
    for (index, name) in ["parent.heartbeat", "child.heartbeat"].iter().enumerate() {
        if read_owned(&fixture.project.join(name), 64 * 1024)? != stable[index] {
            return Err("取消清理后出现迟到写入".into());
        }
    }
    verify_cancelled(
        fixture,
        &decisions[command_decision.ok_or("缺少命令审批")?],
        running.as_ref().ok_or("缺少运行身份")?,
        evidence,
    )?;
    evidence.record(
        json!({"event":"no_late_write_verified","observation_millis":300,
        "parent_heartbeat_sha256":digest(&stable[0]),"child_heartbeat_sha256":digest(&stable[1]),
        "cid_destroyed_and_exact_identities_gone":true,"queued_command_cancel_verified":false}),
    )?;
    Ok(())
}

#[test]
#[ignore = "仅由私有短目录运行器执行；固定 Claude 2.1.280 的真实父子命令链会产生模型费用"]
fn real_claude_g10_running_command_cancel() {
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
    assert_eq!(root, temporary.join("g10-command-cancel"));
    assert_eq!(
        read_owned(&root.join(".infinishell-g10-command-cancel"), 256).unwrap(),
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
        .strip_prefix("claude-g10-cancel-")
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
    let fixture = CancelFixture::load(&root).expect("固定命令夹具预检失败；尚未派发模型输入");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("command-cancel.events.safe.ndjson"))
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
        "max_host_command_requests":1,"max_command_executions":1,"real_gui_verified":false,
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
        let result = drive_cancel(
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
                "host_decisions":["AllowOnce"],"command_executions":1,
                "running_command_cancel_verified":true,"queued_command_cancel_verified":false,
                "search_native_result_audit_pending":true,"all_runtime_hosts_cleaned":true,
                "gap_closed":false,"real_gui_verified":false}))
                .unwrap(),
            Err(_) => panic!("真实 G10 受审命令最小父子链失败；查看本轮白名单阶段证据"),
        }
    });
}
