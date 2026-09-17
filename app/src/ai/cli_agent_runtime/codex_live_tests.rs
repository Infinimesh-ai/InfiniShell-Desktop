//! 真实 Codex 适配器验收；必须通过隔离运行脚本显式启用，不属于默认单元测试。

use std::collections::HashSet;
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

const EVENT_TIMEOUT: Duration = Duration::from_secs(90);

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
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        // 断言或超时也必须释放真实进程，避免测试退出后留下继续运行的任务。
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl LiveSession {
    fn start(options: SessionOptions) -> Result<Self, String> {
        let generation = options.generation;
        let connection = connect(options).map_err(|error| error.to_string())?;
        Ok(Self {
            generation,
            controller: connection.controller,
            events: connection.events,
            task: Some(tokio::spawn(connection.task)),
            native_id: None,
        })
    }

    async fn send(&self, message_id: Uuid, action: RuntimeAction) -> Result<(), String> {
        self.controller
            .send(RuntimeCommand {
                generation: self.generation,
                message_id,
                action,
            })
            .await
            .map_err(|error| error.to_string())
    }

    async fn next(&mut self) -> Result<RuntimeEvent, String> {
        let event = tokio::time::timeout(EVENT_TIMEOUT, self.events.recv())
            .await
            .map_err(|_| "等待真实适配器事件超时".to_owned())?
            .ok_or_else(|| "真实适配器事件流已关闭".to_owned())?;
        if event.generation != self.generation {
            return Err("适配器返回了过时连接代次".into());
        }
        if let Some(native_id) = &self.native_id
            && event.native_session_id.as_ref() != Some(native_id)
        {
            return Err("适配器在同一连接中更换了原生会话身份".into());
        }
        Ok(event)
    }

    async fn ready(&mut self, evidence: &mut Evidence) -> Result<String, String> {
        let event = self.next().await?;
        let RuntimeEventKind::SessionReady {
            effective_permissions,
        } = event.kind
        else {
            return Err(format!("原生会话初始化失败：{:?}", event.kind));
        };
        let native_id = event
            .native_session_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "原生会话未返回 ID".to_owned())?;
        if effective_permissions["approvalPolicy"] != "untrusted"
            || effective_permissions["sandbox"]["type"] != "workspaceWrite"
        {
            return Err(format!("原生权限与验收策略不一致：{effective_permissions}"));
        }
        evidence.record(json!({
            "event": "session_ready", "native_session_id": native_id,
            "permissions": effective_permissions
        }))?;
        self.native_id = Some(native_id.clone());
        Ok(native_id)
    }

    async fn shutdown(&mut self, evidence: &mut Evidence) -> Result<(), String> {
        self.send(Uuid::new_v4(), RuntimeAction::Shutdown).await?;
        let mut task = self.task.take().ok_or_else(|| "连接已经关闭".to_owned())?;
        let result = tokio::time::timeout(Duration::from_secs(10), &mut task).await;
        if result.is_err() {
            task.abort();
            return Err("关闭真实适配器超时".into());
        }
        result
            .expect("超时已单独处理")
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        evidence
            .record(json!({"event": "connection_shutdown", "native_session_id": self.native_id}))
    }
}

enum TurnMode {
    Plain,
    Images {
        paths: Vec<PathBuf>,
    },
    Approval {
        decision: ApprovalDecision,
        command: String,
        argv: Value,
    },
    Steer {
        instruction: String,
    },
    Cancel,
}

struct ObservedTurn {
    turn_id: String,
    output: String,
    approval_count: usize,
    outcome: TurnOutcome,
}

async fn run_turn(
    session: &mut LiveSession,
    phase: &str,
    prompt: String,
    mode: TurnMode,
    cwd: &Path,
    evidence: &mut Evidence,
) -> Result<ObservedTurn, String> {
    evidence.record(json!({"event": "phase_started", "phase": phase}))?;
    let message_id = Uuid::new_v4();
    let mut input = vec![InputContent::Text(prompt)];
    if let TurnMode::Images { paths } = &mode {
        input.extend(paths.iter().cloned().map(InputContent::LocalImage));
    }
    let submit = RuntimeAction::Submit { input };
    session.send(message_id, submit.clone()).await?;
    // 用同一个消息 ID 重投一次，验证生产适配器不会创建第二个原生回合。
    session.send(message_id, submit).await?;
    let mut expected_acknowledgements = HashSet::from([message_id]);
    let mut local_control_ids = HashSet::new();
    let mut accepted = HashSet::new();
    let mut turn_id = None;
    let mut result = None;
    let mut approval_count = 0;
    let mut resolved_approvals = HashSet::new();
    let mut requested_approvals = HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "真实模型回合超过总时限".to_owned())??;
        match event.kind {
            RuntimeEventKind::InputJoined { .. } => {
                return Err("Codex 不支持 Claude 合并输入事件".into());
            }
            RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id,
            } => {
                if !local_control_ids.contains(&message_id) {
                    return Err("输入或中断只有本地交付，不能计为原生确认".into());
                }
                accepted.insert(message_id);
                evidence.record(json!({"event":"command_dispatched","phase":phase,
                    "message_id":message_id,"turn_id":turn_id,"native_receipt":false}))?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id: acknowledged_turn,
            } => {
                accepted.insert(message_id);
                evidence.record(json!({
                    "event": "message_accepted", "phase": phase,
                    "message_id": message_id, "turn_id": acknowledged_turn
                }))?;
            }
            RuntimeEventKind::TurnStarted { turn_id: started } => {
                if turn_id.is_some() {
                    return Err("同一消息重投后产生了额外的原生回合".into());
                }
                turn_id = Some(started.clone());
                evidence
                    .record(json!({"event": "turn_started", "phase": phase, "turn_id": started}))?;
                let control = match &mode {
                    TurnMode::Steer { instruction } => Some(RuntimeAction::Steer {
                        expected_turn_id: started,
                        input: vec![InputContent::Text(instruction.clone())],
                    }),
                    TurnMode::Cancel => Some(RuntimeAction::Interrupt { turn_id: started }),
                    TurnMode::Plain | TurnMode::Images { .. } | TurnMode::Approval { .. } => None,
                };
                if let Some(control) = control {
                    let id = Uuid::new_v4();
                    expected_acknowledgements.insert(id);
                    session.send(id, control).await?;
                }
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id: requested_turn,
                method,
                details,
            } => {
                let safe = match &mode {
                    TurnMode::Approval { command, argv, .. } => {
                        method == "item/commandExecution/requestApproval"
                            && requested_turn == turn_id.as_deref().unwrap_or_default()
                            && approval_matches_fixture(&details, command, argv, cwd)
                    }
                    TurnMode::Plain
                    | TurnMode::Images { .. }
                    | TurnMode::Steer { .. }
                    | TurnMode::Cancel => false,
                };
                let decision = match &mode {
                    TurnMode::Approval { decision, .. } if safe => *decision,
                    TurnMode::Plain
                    | TurnMode::Images { .. }
                    | TurnMode::Approval { .. }
                    | TurnMode::Steer { .. }
                    | TurnMode::Cancel => ApprovalDecision::DenyOnce,
                };
                evidence.record(json!({
                    "event": "approval_requested", "phase": phase, "approval_id": approval_id,
                    "method": method, "command_within_fixture": safe, "decision": decision,
                    "request_details": details
                }))?;
                let id = Uuid::new_v4();
                expected_acknowledgements.insert(id);
                local_control_ids.insert(id);
                requested_approvals.insert(approval_id.clone());
                session
                    .send(
                        id,
                        RuntimeAction::RespondApproval {
                            approval_id,
                            decision,
                        },
                    )
                    .await?;
                if !safe {
                    return Err("审批命令超出预先列明的临时文件测试范围，已拒绝".into());
                }
                approval_count += 1;
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                resolved_approvals.insert(approval_id.clone());
                evidence.record(json!({"event": "approval_resolved", "phase": phase, "approval_id": approval_id, "decision": decision}))?;
            }
            RuntimeEventKind::TurnFinished {
                turn_id: finished,
                outcome,
                output,
            } => {
                if turn_id.as_ref() != Some(&finished) || result.is_some() {
                    return Err("收到未启动或重复结束的原生回合".into());
                }
                evidence.record(json!({
                    "event": "turn_finished", "phase": phase, "turn_id": finished,
                    "outcome": outcome, "output": output.chars().take(4096).collect::<String>()
                }))?;
                result = Some(ObservedTurn {
                    turn_id: finished,
                    output,
                    approval_count,
                    outcome,
                });
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } => {
                return Err(format!("真实控制请求 {message_id} 失败：{message}"));
            }
            RuntimeEventKind::Disconnected { reason } => {
                return Err(format!("真实连接意外中断：{reason}"));
            }
            RuntimeEventKind::SessionReady { .. } => {
                return Err("模型回合中重复初始化了会话".into());
            }
            RuntimeEventKind::ApprovalCancelled { .. } => {
                return Err("审批在完成允许或拒绝验证前被取消".into());
            }
            RuntimeEventKind::Progress { turn_id, message } => {
                evidence.record(json!({"event":"native_tool_progress","phase":phase,"turn_id":turn_id,"details":message}))?;
            }
            RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::LocalToolRequested { .. } => {
                return Err("未授权的测试连接收到了本地工具回调".into());
            }
            RuntimeEventKind::TextDelta { .. } => {}
        }
        if result.is_some()
            && expected_acknowledgements.is_subset(&accepted)
            && requested_approvals.is_subset(&resolved_approvals)
        {
            return Ok(result.expect("结果已由真实终态事件提供"));
        }
    }
}

fn completed(turn: &ObservedTurn, expected: &str) -> Result<(), String> {
    if turn.outcome != TurnOutcome::Completed || turn.output.trim() != expected {
        return Err(format!("原生回合 {} 未给出预期完成结果", turn.turn_id));
    }
    Ok(())
}

fn quoted_command(program: &str, script: &str) -> String {
    #[cfg(not(windows))]
    {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
        format!("{} {}", quote(program), quote(script))
    }
    #[cfg(windows)]
    {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
        format!("& {} {}", quote(program), quote(script))
    }
}

fn approval_matches_fixture(details: &Value, command: &str, argv: &Value, cwd: &Path) -> bool {
    // Codex 可能提议直接 argv 或完整 shell argv。两者都必须严格对应固定测试命令，不能只比前缀。
    let shell_argv = details["command"]
        .as_str()
        .and_then(|command| shell_words::split(command).ok())
        .filter(|parts| {
            cfg!(unix)
                && parts.len() == 3
                && matches!(parts[0].as_str(), "/bin/zsh" | "/bin/bash" | "/bin/sh")
                && matches!(parts[1].as_str(), "-lc" | "-c")
                && parts[2] == command
        });
    let actual_command_matches = details["command"] == command || shell_argv.is_some();
    let policy_matches = details["proposedExecpolicyAmendment"] == *argv
        || shell_argv.is_some_and(|parts| details["proposedExecpolicyAmendment"] == json!(parts));
    actual_command_matches
        && policy_matches
        && details["commandActions"]
            .as_array()
            .is_some_and(|actions| actions.len() == 1 && actions[0]["command"] == command)
        && details["cwd"]
            .as_str()
            .is_some_and(|path| Path::new(path).canonicalize().ok().as_deref() == Some(cwd))
}

#[test]
#[cfg(unix)]
fn approval_fixture_accepts_exact_shell_wrapper_and_rejects_other_commands() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let command = quoted_command("/usr/bin/python3", "approval-allow.py");
    let wrapper = vec!["/bin/zsh", "-lc", command.as_str()];
    let mut details = json!({
        "command":shell_words::join(wrapper.clone()),
        "proposedExecpolicyAmendment":wrapper,
        "commandActions":[{"command":command}],
        "cwd":cwd
    });
    let argv = json!(["/usr/bin/python3", "approval-allow.py"]);
    assert!(approval_matches_fixture(&details, &command, &argv, &cwd));
    details["proposedExecpolicyAmendment"] = argv.clone();
    assert!(approval_matches_fixture(&details, &command, &argv, &cwd));
    details["command"] = json!(format!("{command}; another-command"));
    assert!(!approval_matches_fixture(&details, &command, &argv, &cwd));
    details["command"] = json!(shell_words::join(["/bin/zsh", "-lc", command.as_str()]));
    details["proposedExecpolicyAmendment"] = json!(["/bin/zsh", "-lc", "another-command"]);
    assert!(!approval_matches_fixture(&details, &command, &argv, &cwd));
    details["proposedExecpolicyAmendment"] = argv;
    details["commandActions"][0]["command"] = json!("another-command");
    assert!(!approval_matches_fixture(
        &details,
        &command,
        &json!([]),
        &cwd
    ));
}

async fn exercise(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let executable = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE")
            .ok_or_else(|| "缺少显式 Codex 可执行路径".to_owned())?,
    );
    let python = env::var("INFINISHELL_CODEX_LIVE_PYTHON").map_err(|error| error.to_string())?;
    let mut options = SessionOptions {
        executable,
        cwd: cwd.clone(),
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::WorkspaceWrite,
        permission_ceiling: None,
        claude_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let mut session = LiveSession::start(options.clone())?;
    let native_id = session.ready(evidence).await?;
    let first = run_turn(
        &mut session,
        "first_turn",
        "第一轮：不要调用工具。\nReply exactly PARITY_ONE.".into(),
        TurnMode::Plain,
        &cwd,
        evidence,
    )
    .await?;
    completed(&first, "PARITY_ONE")?;
    let second = run_turn(
        &mut session,
        "second_turn",
        "第二轮：不要调用工具。\nReply exactly PARITY_TWO.".into(),
        TurnMode::Plain,
        &cwd,
        evidence,
    )
    .await?;
    completed(&second, "PARITY_TWO")?;

    for (label, decision) in [
        ("allow", ApprovalDecision::AllowOnce),
        ("deny", ApprovalDecision::DenyOnce),
    ] {
        let file_name = format!("approval-{label}.txt");
        let source =
            format!("from pathlib import Path; Path({file_name:?}).write_text('PARITY_APPROVAL')");
        // 固定脚本不经过模型改写 Python 字符串；审批只允许执行此临时目录内的文件。
        let script = format!("approval-{label}.py");
        fs::write(cwd.join(&script), source).map_err(|error| error.to_string())?;
        let command = quoted_command(&python, &script);
        let prompt = format!(
            "这是临时项目中的审批协议测试。必须只调用 exec_command 一次，原样执行以下命令：\n{command}\n不得使用 apply_patch 或其他文件工具。审批允许后只回复 APPROVED；拒绝后只回复 DENIED 并结束，不得绕过或重试。"
        );
        let mode = TurnMode::Approval {
            decision,
            command,
            argv: json!([python, script]),
        };
        let observed = run_turn(
            &mut session,
            &format!("approval_{label}"),
            prompt,
            mode,
            &cwd,
            evidence,
        )
        .await?;
        let expected = match decision {
            ApprovalDecision::AllowOnce => "APPROVED",
            ApprovalDecision::DenyOnce => "DENIED",
        };
        completed(&observed, expected)?;
        if observed.approval_count != 1 {
            return Err("没有恰好一次真实审批请求".into());
        }
        let file = cwd.join(file_name);
        match decision {
            ApprovalDecision::AllowOnce => {
                if fs::read_to_string(file).map_err(|error| error.to_string())? != "PARITY_APPROVAL"
                {
                    return Err("允许审批后未收回实际文件结果".into());
                }
            }
            ApprovalDecision::DenyOnce => {
                if file.exists() {
                    return Err("拒绝审批后仍出现了目标文件".into());
                }
            }
        }
        evidence.record(json!({"event": "file_effect_verified", "phase": format!("approval_{label}"), "allowed": decision == ApprovalDecision::AllowOnce}))?;
    }

    let steer_marker = format!("STEER_APPLIED_{}", Uuid::new_v4().simple());
    let steered = run_turn(
        &mut session,
        "steer_applied",
        "逐个列出从 1 到 100000 的整数，不要调用工具。".into(),
        TurnMode::Steer {
            instruction: format!(
                "停止列举数字。现在这条追加指令要求你的最终回复只有 {steer_marker}，不要调用工具。"
            ),
        },
        &cwd,
        evidence,
    )
    .await?;
    completed(&steered, &steer_marker)?;
    evidence.record(json!({"event": "steer_text_applied", "marker": steer_marker}))?;
    let cancelled = run_turn(
        &mut session,
        "cancel",
        "逐个列出从 1 到 100000 的整数，不要调用工具。".into(),
        TurnMode::Cancel,
        &cwd,
        evidence,
    )
    .await?;
    if cancelled.outcome != TurnOutcome::Cancelled {
        return Err("取消请求未得到真实 interrupted 终态".into());
    }
    session.shutdown(evidence).await?;

    options.generation = Uuid::new_v4();
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    let mut resumed = LiveSession::start(options)?;
    if resumed.ready(evidence).await? != native_id {
        return Err("恢复产生了新的原生会话 ID".into());
    }
    let recovered = run_turn(&mut resumed, "resume_result", "请只回复刚才在实际生效的追加指令中要求你输出的完整 STEER_APPLIED 标记，不要包含其他文字，不要调用工具。".into(), TurnMode::Plain, &cwd, evidence).await?;
    completed(&recovered, &steer_marker)?;
    resumed.shutdown(evidence).await?;
    evidence.record(json!({
        "event": "acceptance_passed", "scope": "rust_adapter_process_restart",
        "native_session_id": native_id,
        "app_restart_and_ui_verified": false
    }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要通过 run_codex_adapter_live.py 显式提供隔离登录和真实 Codex；会消耗模型额度"]
async fn real_codex_managed_lifecycle() {
    let root = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_LIVE_ROOT").expect("必须由隔离运行脚本启动此测试"),
    );
    let root = root.canonicalize().expect("隔离目录必须存在");
    assert!(
        root.join(".infinishell-live-probe").is_file(),
        "缺少隔离运行标记"
    );
    let configuration =
        PathBuf::from(env::var_os("CODEX_HOME").expect("必须显式设置隔离 CODEX_HOME"));
    assert_eq!(configuration.canonicalize().unwrap(), root.join("codex"));
    let artifact = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_LIVE_ARTIFACT").expect("必须提供证据输出路径"),
    );
    let mut evidence = Evidence {
        file: File::create(artifact).expect("证据文件可写"),
        root: root.clone(),
    };
    evidence
        .record(json!({"event": "acceptance_started", "scope": "rust_adapter_process_restart"}))
        .unwrap();
    let result = exercise(&root, &mut evidence).await;
    if let Err(error) = &result {
        evidence
            .record(json!({"event": "acceptance_failed", "reason": error}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "真实 Codex 适配器验收未通过；请检查脱敏证据文件"
    );
}

async fn run_inspect_tool_turn(
    session: &mut LiveSession,
    marker: &str,
    evidence: &mut Evidence,
) -> Result<(), String> {
    session.send(Uuid::new_v4(), RuntimeAction::Submit { input:vec![InputContent::Text(format!("Call inspect_local_tasks exactly once with empty arguments. Do not use any other tool. After its result, reply exactly {marker}."))] }).await?;
    let mut active = None;
    let mut calls = HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "本地工具真实回合超时".to_string())??;
        evidence
            .record(json!({"event":"native_tool_restore_probe", "phase":marker,"runtime":event}))?;
        match event.kind {
            RuntimeEventKind::TurnStarted { turn_id } => {
                active = Some(turn_id);
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                if request.tool != "inspect_local_tasks"
                    || request.arguments != json!({})
                    || Some(&request.turn_id) != active.as_ref()
                    || !calls.insert(request.call_id.clone())
                    || calls.len() > 1
                {
                    return Err("工具调用超出只读探测范围或重复执行".into());
                }
                session
                    .send(
                        Uuid::new_v4(),
                        RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id,
                            call_id: request.call_id,
                            result: Ok(json!({"probe":marker,"tasks":[]})),
                        },
                    )
                    .await?;
            }
            RuntimeEventKind::TurnFinished {
                outcome, output, ..
            } => {
                if outcome != TurnOutcome::Completed || output.trim() != marker || calls.len() != 1
                {
                    return Err(format!(
                        "真实原生工具未完成调用与结果回收：{outcome:?} {output}"
                    ));
                }
                return Ok(());
            }
            RuntimeEventKind::ApprovalRequested { approval_id, .. } => {
                session
                    .send(
                        Uuid::new_v4(),
                        RuntimeAction::RespondApproval {
                            approval_id,
                            decision: ApprovalDecision::DenyOnce,
                        },
                    )
                    .await?;
                return Err("只读本地工具探测出现额外审批，已拒绝".into());
            }
            RuntimeEventKind::RequestFailed { message, .. }
            | RuntimeEventKind::Disconnected { reason: message } => return Err(message),
            RuntimeEventKind::InputJoined { .. } => {
                return Err("Codex 本地工具收到其他协议的合并事件".into());
            }
            RuntimeEventKind::MessageAccepted { .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::TextDelta { .. }
            | RuntimeEventKind::Progress { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::SessionReady { .. } => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要隔离真实Codex；只探测原生动态工具保存恢复，会消耗模型额度"]
async fn real_codex_local_tool_restore() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_ROOT").expect("必须由隔离运行脚本启动"))
            .canonicalize()
            .unwrap();
    assert!(root.join(".infinishell-live-probe").is_file());
    assert_eq!(
        PathBuf::from(env::var_os("CODEX_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("codex")
    );
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CODEX_LIVE_ARTIFACT").expect("必须提供独立证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    let result: Result<(), String> = async {
        let mut options = SessionOptions {
            executable: PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE").unwrap()),
            cwd: root.join("project").canonicalize().unwrap(),
            state_dir: root.to_owned(),
            target: SessionTarget::New,
            generation: Uuid::new_v4(),
            permission_policy: PermissionPolicy::WorkspaceWrite,
            permission_ceiling: None,
            claude_profile: None,
            model: None,
            local_tools: Some(super::local_tools::LocalToolPermissions::default()),
            selected_skills: Vec::new(),
        };
        let mut initial = LiveSession::start(options.clone())?;
        let native_id = initial.ready(&mut evidence).await?;
        run_inspect_tool_turn(&mut initial, "TOOL_BEFORE_RESTART", &mut evidence).await?;
        initial.shutdown(&mut evidence).await?;
        options.generation = Uuid::new_v4();
        options.target = SessionTarget::Resume {
            native_session_id: native_id.clone(),
        };
        let mut resumed = LiveSession::start(options)?;
        let resumed_id = resumed.ready(&mut evidence).await?;
        if resumed_id != native_id {
            return Err("工具恢复返回了不同会话ID".into());
        }
        run_inspect_tool_turn(&mut resumed, "TOOL_AFTER_RESTART", &mut evidence).await?;
        resumed.shutdown(&mut evidence).await?;
        Ok(())
    }
    .await;
    evidence.record(json!({"event":"tool_restore_probe_finished","passed":result.is_ok(),"error":result.as_ref().err()})).unwrap();
    assert!(result.is_ok(), "动态工具真实保存恢复未通过，请检查独立证据");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要隔离真实 Codex，验证实际图片读取；会消耗模型额度"]
async fn real_codex_image_input() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_ROOT").expect("必须由隔离运行脚本启动"))
            .canonicalize()
            .unwrap();
    assert!(root.join(".infinishell-live-probe").is_file());
    assert_eq!(
        PathBuf::from(env::var_os("CODEX_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("codex")
    );
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CODEX_LIVE_ARTIFACT").expect("必须提供独立证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    let result: Result<(), String> = async {
        // 文件名和提示均不透露随机颜色，要求模型读取图片；文字回合不能替代图片验收。
        let colors = [
            ([255, 0, 0], "RED"),
            ([0, 0, 255], "BLUE"),
            ([0, 255, 0], "GREEN"),
        ];
        let (color, expected) = colors[(Uuid::new_v4().as_u128() % 3) as usize];
        let path = root.join("project").join("input.png");
        image::RgbImage::from_pixel(96, 96, image::Rgb(color))
            .save(&path)
            .map_err(|error| error.to_string())?;
        let cwd = root.join("project").canonicalize().unwrap();
        let mut session = LiveSession::start(SessionOptions {
            executable: PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE").unwrap()),
            cwd: cwd.clone(),
            state_dir: root.to_owned(),
            target: SessionTarget::New,
            generation: Uuid::new_v4(),
            permission_policy: PermissionPolicy::WorkspaceWrite,
            permission_ceiling: None,
            claude_profile: None,
            model: None,
            local_tools: None,
            selected_skills: Vec::new(),
        })?;
        session.ready(&mut evidence).await?;
        let observed = run_turn(
            &mut session,
            "image_input",
            "请查看附带图片，判断整张图片的颜色。只用一个英文大写颜色单词回答，不调用任何工具。"
                .into(),
            TurnMode::Images { paths: vec![path] },
            &cwd,
            &mut evidence,
        )
        .await?;
        completed(&observed, expected)?;
        session.shutdown(&mut evidence).await?;
        Ok(())
    }
    .await;
    evidence.record(json!({"event":"image_probe_finished","passed":result.is_ok(),"error":result.as_ref().err()})).unwrap();
    assert!(result.is_ok(), "真实图片输入未通过，请检查独立证据");
}
