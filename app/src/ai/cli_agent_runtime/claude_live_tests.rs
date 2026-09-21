//! 真实 Claude 适配器验收；只通过显式认证运行器启动，不属于默认单元测试。

#[path = "claude_profile_live_tests.rs"]
mod profile_live_tests;

#[path = "claude_batch_cancel_live_tests.rs"]
mod batch_cancel_live_tests;

#[path = "claude_managed_image_live_tests.rs"]
mod managed_image_live_tests;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::connect;
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeController, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome,
};

struct Evidence {
    file: File,
    root: PathBuf,
}

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        let encoded = serde_json::to_string(&value).map_err(|error| error.to_string())?;
        let sanitized = encoded.replace(&*self.root.to_string_lossy(), "<probe-root>");
        writeln!(self.file, "{sanitized}").map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())
    }
}

struct LiveSession {
    generation: Uuid,
    controller: RuntimeController,
    events: mpsc::Receiver<RuntimeEvent>,
    task: Option<JoinHandle<Result<(), RuntimeError>>>,
    native_id: Option<String>,
    expected_native_id: Option<String>,
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        // 失败也释放适配器；监督者通过控制管道 EOF 收尾，运行器保留现场而不抢先删目录。
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl LiveSession {
    fn start(options: SessionOptions) -> Result<Self, String> {
        let generation = options.generation;
        let expected_native_id = match &options.target {
            SessionTarget::New => None,
            SessionTarget::Resume { native_session_id } => Some(native_session_id.clone()),
        };
        let connection = connect(options).map_err(|error| error.to_string())?;
        Ok(Self {
            generation,
            controller: connection.controller,
            events: connection.events,
            task: Some(tokio::spawn(connection.task)),
            native_id: None,
            expected_native_id,
        })
    }

    async fn send(&self, id: Uuid, action: RuntimeAction) -> Result<(), String> {
        self.controller
            .send(RuntimeCommand {
                generation: self.generation,
                message_id: id,
                action,
            })
            .await
            .map_err(|error| error.to_string())
    }

    async fn submit(&self, id: Uuid, prompt: String) -> Result<(), String> {
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text(prompt)],
        };
        self.send(id, action.clone()).await?;
        // 同一消息重投不能生成第二次执行；仍要求原生接收和终态。
        self.send(id, action).await
    }

    async fn next(&mut self) -> Result<RuntimeEvent, String> {
        let event = tokio::time::timeout(Duration::from_secs(90), self.events.recv())
            .await
            .map_err(|_| "等待真实 Claude 事件超时".to_owned())?
            .ok_or_else(|| "真实 Claude 事件流关闭".to_owned())?;
        if event.generation != self.generation {
            return Err("收到过时连接代次".into());
        }
        if let Some(id) = &event.native_session_id {
            Uuid::parse_str(id).map_err(|_| "原生会话 ID 不是 UUID".to_owned())?;
            if self
                .native_id
                .as_ref()
                .is_some_and(|previous| previous != id)
                || self
                    .expected_native_id
                    .as_ref()
                    .is_some_and(|expected| expected != id)
            {
                return Err("原生会话身份改变，不能视为成功恢复".into());
            }
            self.native_id = Some(id.clone());
        } else if self.native_id.is_some() {
            return Err("已关联原生会话的事件丢失身份".into());
        }
        Ok(event)
    }

    async fn ready(&mut self, evidence: &mut Evidence) -> Result<(), String> {
        let event = self.next().await?;
        let RuntimeEventKind::SessionReady {
            effective_permissions,
            ..
        } = event.kind
        else {
            return Err(format!("Claude 初始化失败：{:?}", event.kind));
        };
        record_permissions(evidence, &effective_permissions, self.native_id.as_deref())
    }

    async fn shutdown(&mut self, evidence: &mut Evidence) -> Result<(), String> {
        self.send(Uuid::new_v4(), RuntimeAction::Shutdown).await?;
        let mut task = self.task.take().ok_or_else(|| "连接已关闭".to_owned())?;
        let result = tokio::time::timeout(Duration::from_secs(15), &mut task).await;
        if result.is_err() {
            task.abort();
            return Err("监督进程关闭超时".into());
        }
        result
            .expect("超时已处理")
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        evidence.record(json!({"event":"connection_shutdown", "native_session_id":self.native_id}))
    }
}

fn record_permissions(
    evidence: &mut Evidence,
    permissions: &Value,
    native_id: Option<&str>,
) -> Result<(), String> {
    // Inherit 不能宣称沙箱或父任务上限已获证明；此验收只接受普通审批模式。
    if permissions["permissionMode"] != "default" {
        return Err("验收要求 Claude 实际处于 default 审批模式，未自动改写配置".into());
    }
    evidence.record(json!({
        "event":"session_ready", "native_session_id":native_id,
        "permission_mode":"default", "native_session_association_confirmed":native_id.is_some(),
        "filesystem_sandbox_verified":false, "parent_permission_ceiling_verified":false
    }))
}

enum TurnMode {
    Plain,
    Approval {
        decision: ApprovalDecision,
        path: PathBuf,
        content: String,
    },
    Queue {
        marker: String,
    },
    Cancel,
}

struct ObservedTurn {
    outcome: TurnOutcome,
    output: String,
}

fn approval_matches_fixture(details: &Value, path: &Path, content: &str) -> bool {
    // 不接受 shell、相似路径或额外参数，也不采用权限模式建议；只允许固定 Write 输入。
    details["subtype"] == "can_use_tool"
        && details["tool_name"] == "Write"
        && details["input"] == json!({"file_path":path, "content":content})
}

async fn run_turn(
    session: &mut LiveSession,
    phase: &str,
    prompt: String,
    mode: TurnMode,
    evidence: &mut Evidence,
) -> Result<ObservedTurn, String> {
    let primary = Uuid::new_v4();
    let mut turns = HashSet::from([primary.to_string()]);
    let mut receipts = HashSet::from([primary]);
    let mut local_controls = HashSet::new();
    let mut acknowledged = HashSet::new();
    let mut started = HashSet::new();
    let mut finished = HashMap::new();
    let mut approvals = HashSet::new();
    let mut resolved = HashSet::new();
    let mut queued_id = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    evidence.record(json!({"event":"phase_started", "phase":phase, "message_id":primary}))?;
    session.submit(primary, prompt).await?;
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "真实模型回合超过总时限".to_owned())??;
        let native_id = event.native_session_id;
        match event.kind {
            RuntimeEventKind::SessionReady {
                effective_permissions,
                ..
            } => {
                // 首次 initialize 无 ID；system/init 和权限变化可再次公布状态。
                record_permissions(evidence, &effective_permissions, native_id.as_deref())?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                let expected_turn = if turns.contains(&message_id.to_string()) {
                    message_id.to_string()
                } else {
                    primary.to_string()
                };
                if !receipts.contains(&message_id)
                    || turn_id.as_deref() != Some(expected_turn.as_str())
                {
                    return Err("原生确认关联到未知消息或错误回合".into());
                }
                acknowledged.insert(message_id);
                evidence.record(json!({"event":"message_accepted", "phase":phase,
                    "message_id":message_id, "turn_id":turn_id, "native_session_id":native_id}))?;
            }
            RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id,
            } => {
                if !local_controls.contains(&message_id)
                    || turn_id.as_deref() != Some(primary.to_string().as_str())
                {
                    return Err("不能把用户输入或取消的本地写入当原生确认".into());
                }
                acknowledged.insert(message_id);
                evidence.record(json!({"event":"command_dispatched", "phase":phase,
                    "message_id":message_id, "turn_id":turn_id, "native_receipt":false}))?;
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                if !turns.contains(&turn_id) || !started.insert(turn_id.clone()) {
                    return Err("未知或重复开始的回合，重投去重未通过".into());
                }
                evidence.record(json!({"event":"turn_started", "phase":phase,
                    "turn_id":turn_id, "native_session_id":native_id}))?;
                if turn_id == primary.to_string() {
                    match &mode {
                        TurnMode::Queue { marker } => {
                            let id = Uuid::new_v4();
                            turns.insert(id.to_string());
                            receipts.insert(id);
                            queued_id = Some(id.to_string());
                            session.submit(id, format!("这是运行中追加、作为下一条输入排队的指令。请记住完整标记 {marker}；不要调用工具，只回复 {marker}。")).await?;
                            evidence.record(json!({"event":"queued_input_submitted", "phase":phase,
                                "message_id":id, "active_turn_id":turn_id,
                                "submitted_while_running":true, "same_turn_steering_verified":false}))?;
                        }
                        TurnMode::Cancel => {
                            let id = Uuid::new_v4();
                            receipts.insert(id);
                            session
                                .send(id, RuntimeAction::Interrupt { turn_id })
                                .await?;
                        }
                        TurnMode::Plain | TurnMode::Approval { .. } => {}
                    }
                }
            }
            RuntimeEventKind::InputJoined {
                message_id,
                turn_id,
            } => {
                let input_id = message_id.to_string();
                if queued_id.as_ref() != Some(&input_id)
                    || turn_id != primary.to_string()
                    || !acknowledged.contains(&message_id)
                    || !started.contains(&turn_id)
                    || finished.contains_key(&turn_id)
                    || !started.insert(input_id.clone())
                {
                    return Err("合并输入没有先行原生确认或错误绑定了执行".into());
                }
                evidence.record(json!({"event":"input_joined", "phase":phase,
                    "message_id":message_id, "turn_id":turn_id, "native_session_id":native_id}))?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                let safe = match &mode {
                    TurnMode::Approval { path, content, .. } => {
                        method == "can_use_tool"
                            && turn_id == primary.to_string()
                            && approvals.is_empty()
                            && approval_matches_fixture(&details, path, content)
                    }
                    TurnMode::Plain | TurnMode::Queue { .. } | TurnMode::Cancel => false,
                };
                let decision = match &mode {
                    TurnMode::Approval { decision, .. } if safe => *decision,
                    TurnMode::Plain
                    | TurnMode::Approval { .. }
                    | TurnMode::Queue { .. }
                    | TurnMode::Cancel => ApprovalDecision::DenyOnce,
                };
                let id = Uuid::new_v4();
                receipts.insert(id);
                local_controls.insert(id);
                approvals.insert(approval_id.clone());
                session
                    .send(
                        id,
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision,
                        },
                    )
                    .await?;
                evidence.record(json!({"event":"approval_requested", "phase":phase,
                    "approval_id":approval_id, "decision":decision, "exact_write_fixture":safe}))?;
                if !safe {
                    return Err("收到超出固定文件测试范围或重复的审批，已拒绝".into());
                }
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                let expected_decision = match &mode {
                    TurnMode::Approval { decision, .. } => Some(*decision),
                    TurnMode::Plain | TurnMode::Queue { .. } | TurnMode::Cancel => None,
                };
                if !approvals.contains(&approval_id)
                    || !resolved.insert(approval_id.clone())
                    || expected_decision != Some(decision)
                {
                    return Err("未知或重复的审批结束事件".into());
                }
                evidence.record(json!({"event":"approval_resolved", "phase":phase,
                    "approval_id":approval_id, "decision":decision, "native_receipt":false}))?;
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                if !started.contains(&turn_id) || finished.contains_key(&turn_id) {
                    return Err("没有已关联运行事件，或收到重复终态".into());
                }
                evidence.record(json!({"event":"turn_finished", "phase":phase,
                    "turn_id":turn_id, "native_session_id":native_id, "outcome":outcome,
                    "output":output.chars().take(4096).collect::<String>()}))?;
                finished.insert(turn_id, ObservedTurn { outcome, output });
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } => {
                return Err(format!("Claude 请求 {message_id} 失败：{message}"));
            }
            RuntimeEventKind::Disconnected { reason } => {
                return Err(format!("Claude 连接中断：{reason}"));
            }
            RuntimeEventKind::ApprovalCancelled { .. } => return Err("审批尚未验证就被取消".into()),
            RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::LocalToolCancelled { .. } => {
                return Err("未启用本地工具的验收收到了工具回调".into());
            }
            RuntimeEventKind::TextDelta { .. } | RuntimeEventKind::Progress { .. } => {}
        }
        if finished.len() == turns.len()
            && receipts.is_subset(&acknowledged)
            && approvals == resolved
        {
            if matches!(mode, TurnMode::Approval { .. }) && approvals.len() != 1 {
                return Err("未观察到恰好一次真实 Write 审批，不能计为通过".into());
            }
            if let TurnMode::Queue { marker } = &mode {
                let queued = queued_id
                    .as_ref()
                    .and_then(|id| finished.get(id))
                    .ok_or_else(|| "运行中追加输入没有独立关联结果".to_owned())?;
                completed(queued, marker)?;
                evidence.record(
                    json!({"event":"queued_input_result_verified", "marker":marker,
                    "native_acknowledgement_verified":true, "same_turn_steering_supported":false}),
                )?;
            }
            return finished
                .remove(&primary.to_string())
                .ok_or_else(|| "主回合结果缺失".into());
        }
    }
}

fn completed(turn: &ObservedTurn, expected: &str) -> Result<(), String> {
    if turn.outcome != TurnOutcome::Completed || turn.output.trim() != expected {
        return Err("真实回合没有提供要求的完成结果".into());
    }
    Ok(())
}

async fn exercise(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let mut options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE")
                .ok_or_else(|| "缺少显式 Claude 路径".to_owned())?,
        ),
        cwd: cwd.clone(),
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: env::var("INFINISHELL_CLAUDE_LIVE_MODEL").ok(),
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let mut session = LiveSession::start(options.clone())?;
    session.ready(evidence).await?;
    let first = run_turn(
        &mut session,
        "first_turn",
        "第一轮：你好。\n不要调用工具，只回复 PARITY_ONE。".into(),
        TurnMode::Plain,
        evidence,
    )
    .await?;
    completed(&first, "PARITY_ONE")?;
    let native_id = session
        .native_id
        .clone()
        .ok_or_else(|| "第一轮结束仍无原生会话 ID".to_owned())?;
    let second = run_turn(
        &mut session,
        "second_turn",
        "第二轮：不要调用工具，只回复 PARITY_TWO。".into(),
        TurnMode::Plain,
        evidence,
    )
    .await?;
    completed(&second, "PARITY_TWO")?;
    for (label, decision) in [
        ("allow", ApprovalDecision::AllowOnce),
        ("deny", ApprovalDecision::DenyOnce),
    ] {
        let path = cwd.join(format!("approval-{label}.txt"));
        let content = "PARITY_APPROVAL".to_owned();
        if path.exists() {
            return Err("审批目标文件不是全新测试路径".into());
        }
        let prompt = format!(
            "这是临时项目的审批测试。只调用 Write 工具一次，file_path={}，content=\"PARITY_APPROVAL\"（没有换行）。不得调用其他工具或修改任何其他文件。允许后只回复 APPROVED；拒绝后只回复 DENIED 并结束，不得重试或绕过。",
            json!(path)
        );
        let observed = run_turn(
            &mut session,
            &format!("approval_{label}"),
            prompt,
            TurnMode::Approval {
                decision,
                path: path.clone(),
                content: content.clone(),
            },
            evidence,
        )
        .await?;
        match decision {
            ApprovalDecision::AllowOnce => {
                completed(&observed, "APPROVED")?;
                if fs::read_to_string(&path).map_err(|error| error.to_string())? != content {
                    return Err("允许 Write 后未回收真实文件效果".into());
                }
            }
            ApprovalDecision::DenyOnce => {
                completed(&observed, "DENIED")?;
                if path.exists() {
                    return Err("拒绝 Write 后仍出现目标文件".into());
                }
            }
        }
        evidence.record(
            json!({"event":"file_effect_verified", "phase":format!("approval_{label}"),
            "allowed":decision == ApprovalDecision::AllowOnce}),
        )?;
    }
    let marker = format!("QUEUED_APPLIED_{}", Uuid::new_v4().simple());
    let queued = run_turn(
        &mut session,
        "queued_input",
        "不要调用工具，列出从1到200的整数，最后写 QUEUE_PARENT_DONE。".into(),
        TurnMode::Queue {
            marker: marker.clone(),
        },
        evidence,
    )
    .await?;
    if queued.outcome != TurnOutcome::Completed {
        return Err("追加输入时原回合未真实完成".into());
    }
    let cancelled = run_turn(
        &mut session,
        "cancel",
        "不要调用工具，逐个列出从1到100000的整数。".into(),
        TurnMode::Cancel,
        evidence,
    )
    .await?;
    if cancelled.outcome != TurnOutcome::Cancelled {
        return Err("取消未得到原生 cancelled 终态".into());
    }
    session.shutdown(evidence).await?;
    options.generation = Uuid::new_v4();
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    let mut resumed = LiveSession::start(options)?;
    resumed.ready(evidence).await?;
    let recovered = run_turn(
        &mut resumed,
        "resume_result",
        "不要调用工具，只回复刚才运行中追加指令要求你记住的完整 QUEUED_APPLIED 标记。".into(),
        TurnMode::Plain,
        evidence,
    )
    .await?;
    completed(&recovered, &marker)?;
    if resumed.native_id.as_ref() != Some(&native_id) {
        return Err("继续历史未关联到原会话".into());
    }
    resumed.shutdown(evidence).await?;
    evidence.record(
        json!({"event":"acceptance_passed", "scope":"rust_adapter_process_restart",
        "native_session_id":native_id, "queued_input_verified":true,
        "same_turn_steering_supported":false, "app_restart_and_ui_verified":false,
        "parent_permission_ceiling_verified":false}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要运行器指定私有认证或用户授权的默认 Claude 在线账户；会消耗模型额度"]
async fn real_claude_managed_lifecycle() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .expect("私有项目目录存在");
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-live-probe")).unwrap(),
        "isolated Claude Rust adapter verification\n"
    );
    let auth_mode = env::var("INFINISHELL_CLAUDE_LIVE_AUTH_MODE").expect("缺少认证模式边界");
    match auth_mode.as_str() {
        "private_config" => {
            let configuration =
                PathBuf::from(env::var_os("CLAUDE_CONFIG_DIR").expect("缺少显式私有配置"))
                    .canonicalize()
                    .unwrap();
            let declared = PathBuf::from(
                env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").expect("缺少配置边界"),
            )
            .canonicalize()
            .unwrap();
            assert_eq!(configuration, declared);
            assert!(!root.starts_with(&configuration));
        }
        "authorized_default_account" => {
            assert!(env::var_os("CLAUDE_CONFIG_DIR").is_none());
            assert!(env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").is_none());
            let home = PathBuf::from(env::var_os("HOME").expect("默认账户模式缺少用户 HOME"))
                .canonicalize()
                .unwrap();
            assert!(!home.starts_with(&root));
        }
        _ => panic!("不支持的 Claude 认证模式"),
    }
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").expect("缺少证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence
        .record(
            json!({"event":"acceptance_started", "scope":"rust_adapter_process_restart",
        "credential_files_read_by_rust_probe":false, "authentication_source":auth_mode,
        "permission_policy":"Inherit"}),
        )
        .unwrap();
    let result = exercise(&root, &mut evidence).await;
    if let Err(error) = &result {
        evidence
            .record(json!({"event":"acceptance_failed", "reason":error}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "真实 Claude 适配器验收未通过；检查脱敏证据文件"
    );
}

#[test]
fn approval_fixture_accepts_only_exact_write_input() {
    let path = env::temp_dir().join("审批中文.txt");
    let details = json!({"subtype":"can_use_tool", "tool_name":"Write",
        "input":{"file_path":path, "content":"PARITY_APPROVAL"}});
    assert!(approval_matches_fixture(&details, &path, "PARITY_APPROVAL"));
    let mut changed = details.clone();
    changed["tool_name"] = json!("Bash");
    assert!(!approval_matches_fixture(
        &changed,
        &path,
        "PARITY_APPROVAL"
    ));
    changed = details.clone();
    changed["input"]["file_path"] = json!(env::temp_dir().join("其他文件.txt"));
    assert!(!approval_matches_fixture(
        &changed,
        &path,
        "PARITY_APPROVAL"
    ));
    changed = details.clone();
    changed["input"]["content"] = json!("PARITY_APPROVAL\n");
    assert!(!approval_matches_fixture(
        &changed,
        &path,
        "PARITY_APPROVAL"
    ));
    changed = details;
    changed["input"]["extra"] = json!(true);
    assert!(!approval_matches_fixture(
        &changed,
        &path,
        "PARITY_APPROVAL"
    ));
}

#[path = "claude_image_probe_live_tests.rs"]
mod image_probe_live_tests;
