//! 真实合并批次取消探针；只接受明确合并、原生全批取消和同会话继续的完整证据。

use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::super::{ClaudeProtocol, run_process, validate_options};
use super::{
    ApprovalDecision, Duration, Evidence, File, HashSet, InputContent, LiveSession, Path, PathBuf,
    PermissionPolicy, RuntimeAction, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
    Uuid, Value, env, fs, json,
};
use crate::ai::cli_agent_runtime::{channels, local_tools::LocalToolPermissions, managed_process};

#[path = "claude_approval_cancel_live_tests.rs"]
mod approval_cancel_live_tests;

const SCOPE: &str = "rust_adapter_joined_batch_cancel";
const TOOL: &str = "mcp__infinishell-local-tasks__inspect_local_tasks";
const MAX_NATIVE_INPUTS: usize = 3;

fn start(
    options: SessionOptions,
    native_ids: Arc<Mutex<Vec<Value>>>,
) -> Result<LiveSession, String> {
    validate_options(&options).map_err(|_| "真实探针配置无效".to_owned())?;
    let generation = options.generation;
    let (controller, commands, sender, events) = channels(generation);
    let mut protocol = ClaudeProtocol::new(options);
    protocol.native_ids_for_live = Some(native_ids);
    let task = tokio::spawn(async move {
        let result = run_process(&mut protocol, commands, &sender).await;
        let _ = sender.try_send(protocol.event(RuntimeEventKind::Disconnected {
            reason: "真实批次探针连接已关闭".into(),
        }));
        result
    });
    Ok(LiveSession {
        generation,
        controller,
        events,
        task: Some(task),
        native_id: None,
        expected_native_id: None,
    })
}

fn observe_permissions(permissions: &Value, evidence: &mut Evidence) -> Result<(), String> {
    if permissions["permissionMode"]
        != super::super::super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE
        || permissions["fixedProfileVerified"] != true
        || permissions["fixedProfileSha256"].as_str().is_none()
    {
        return Err("原生固定权限策略没有通过验证".into());
    }
    evidence.record(json!({"event":"profile_verified",
        "permission_mode":super::super::super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE,
        "fixed_profile_verified":true,"profile_sha256":permissions["fixedProfileSha256"],
        "filesystem_sandbox_verified":false,"parent_permission_ceiling_verified":false}))
}

fn batch_result_matches(row: &Value, running: Uuid, joined: Uuid, native_id: &str) -> bool {
    let Some(inputs) = row["user_message_uuids"].as_array() else {
        return false;
    };
    let expected = HashSet::from([running.to_string(), joined.to_string()]);
    let actual = inputs
        .iter()
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();
    row["direction"] == "stdout"
        && row["type"] == "result"
        && row["session_id"] == native_id
        && inputs.len() == 2
        && actual.len() == 2
        && expected.iter().all(|id| actual.contains(id.as_str()))
        && row["user_message_uuid"]
            .as_str()
            .is_some_and(|id| expected.contains(id))
        && ((matches!(
            row["terminal_reason"].as_str(),
            Some("aborted_streaming" | "aborted_tools")
        ) && row["subtype"] == "error_during_execution"
            && row["is_error"] == true)
            || (matches!(
                row["terminal_reason"].as_str(),
                Some("interrupted" | "cancelled")
            ) && row["is_error"] == false
                && row["subtype"] == "success"))
}

fn audit_batch(
    rows: &[Value],
    running: Uuid,
    joined: Uuid,
    native_id: &str,
) -> Result<Value, String> {
    let lifecycle = |id: Uuid, state: &str| {
        rows.iter().position(|row| {
            row["direction"] == "stdout"
                && row["type"] == "command_lifecycle"
                && row["command_uuid"] == id.to_string()
                && row["session_id"] == native_id
                && row["state"] == state
        })
    };
    let accepted = |id: Uuid| lifecycle(id, "queued").or_else(|| lifecycle(id, "started"));
    let running_ack = accepted(running).ok_or("原生协议没有接收第一输入")?;
    let joined_ack = accepted(joined).ok_or("原生协议没有接收合并输入")?;
    let running_start = lifecycle(running, "started").ok_or("原生真实执行没有开始")?;
    let joined_start = lifecycle(joined, "started").ok_or("原生合并输入没有开始标记")?;
    let interrupts = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row["direction"] == "stdin"
                && row["type"] == "control_request"
                && row["request_subtype"] == "interrupt"
        })
        .collect::<Vec<_>>();
    let [(interrupt_index, interrupt)] = interrupts.as_slice() else {
        return Err("必须恰好派发一次原生取消请求".into());
    };
    let ack_index = rows
        .iter()
        .position(|row| {
            row["direction"] == "stdout"
                && row["type"] == "control_response"
                && row["response_request_id"] == interrupt["request_id"]
                && row["response_subtype"] == "success"
        })
        .ok_or("原生取消请求没有真实成功 ACK")?;
    let cancelled_index =
        lifecycle(running, "cancelled").ok_or("真实执行没有原生 cancelled 生命周期")?;
    let results = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| batch_result_matches(row, running, joined, native_id))
        .collect::<Vec<_>>();
    let [(result_index, result)] = results.as_slice() else {
        return Err("没有恰好一份覆盖全批 UUID 的原生取消结果".into());
    };
    if running_ack > running_start
        || joined_ack > joined_start
        || running_start >= joined_start
        || joined_start >= *interrupt_index
        || *interrupt_index >= ack_index
        || *interrupt_index >= cancelled_index
        || *interrupt_index >= *result_index
    {
        return Err("输入合并或原生取消证据顺序错误".into());
    }
    Ok(
        json!({"event":"native_batch_cancel_verified","native_session_id":native_id,
        "execution_turn_id":running,"joined_input_id":joined,
        "interrupt_request_id":interrupt["request_id"],"native_result_uuid":result["uuid"],
        "terminal_reason":result["terminal_reason"],"is_error":result["is_error"],
        "user_message_uuids":result["user_message_uuids"],
        "native_input_ack_count":2,"native_interrupt_ack_verified":true,
        "native_execution_cancelled_verified":true,"native_complete_batch_result_verified":true,
        "all_three_signals_verified":true}),
    )
}

async fn submit(session: &LiveSession, id: Uuid, text: String) -> Result<(), String> {
    session
        .send(
            id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text(text)],
            },
        )
        .await
}

async fn cancel_and_continue(
    session: &mut LiveSession,
    evidence: &mut Evidence,
) -> Result<(Uuid, Uuid, Uuid), String> {
    let running = Uuid::new_v4();
    let joined = Uuid::new_v4();
    let continuation = Uuid::new_v4();
    let marker = format!("BATCH_CONTINUED_{}", Uuid::new_v4().simple());
    let mut inputs = HashSet::from([running]);
    let mut accepted = HashSet::new();
    let mut started = HashSet::new();
    let mut terminals = Vec::new();
    let mut approval = None;
    let mut approval_sent = false;
    let mut approval_resolved = false;
    let mut inspect_replied = false;
    let mut inspect_reply = None;
    let mut local_controls = HashSet::new();
    let mut joined_verified = false;
    let mut interrupt_id = None;
    let mut interrupt_acknowledged = false;
    let mut continuation_submitted = false;
    let mut continued = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    submit(session, running,
        "This is an isolated batch cancellation probe. Call inspect_local_tasks exactly once with {}. \
         The host returns an inspect literal without reading or changing project files. \
         Retain the pending inspect when an additional input arrives. After that tool returns, \
         do not call any more tools and list integers from 1 to 100000 one per line. \
         Do not edit files, launch commands, spawn agents, send messages or finish early.".into()).await?;
    evidence.record(json!({"event":"input_submitted","phase":"batch_cancel",
        "message_id":running,"submitted_input_count":1}))?;
    while !continued {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "真实合并取消探针超过总时限；未合并也不得算通过")??;
        let native_id = event.native_session_id;
        match event.kind {
            RuntimeEventKind::SessionReady {
                effective_permissions,
                ..
            } => {
                observe_permissions(&effective_permissions, evidence)?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                let expected = if inputs.contains(&message_id) {
                    message_id.to_string()
                } else if interrupt_id == Some(message_id) {
                    interrupt_acknowledged = true;
                    running.to_string()
                } else {
                    return Err("接收确认关联了未知输入或控制消息".into());
                };
                if turn_id.as_deref() != Some(expected.as_str()) {
                    return Err("原生接收确认绑定到错误执行".into());
                }
                if inputs.contains(&message_id) && accepted.insert(message_id) {
                    evidence.record(json!({"event":"message_accepted","message_id":message_id,
                        "turn_id":turn_id,"native_session_id":native_id,"native_receipt":true}))?;
                } else if interrupt_id == Some(message_id) {
                    evidence
                        .record(json!({"event":"interrupt_accepted","message_id":message_id,
                        "turn_id":turn_id,"native_session_id":native_id,"native_receipt":true}))?;
                }
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                let id = Uuid::parse_str(&turn_id).map_err(|_| "执行 ID 不是 UUID")?;
                if !accepted.contains(&id)
                    || !started.insert(id)
                    || !((id == running && terminals.is_empty())
                        || (id == continuation && continuation_submitted && terminals.len() == 2))
                {
                    return Err("新增输入没有明确合并，或出现未知、重复、未确认执行".into());
                }
                evidence.record(json!({"event":"turn_started","turn_id":turn_id,
                    "native_session_id":native_id}))?;
            }
            RuntimeEventKind::InputJoined {
                message_id,
                turn_id,
            } => {
                if message_id != joined
                    || turn_id != running.to_string()
                    || !accepted.contains(&running)
                    || !accepted.contains(&joined)
                    || !started.contains(&running)
                    || joined_verified
                    || !terminals.is_empty()
                {
                    return Err("没有明确确认两个输入合并到同一真实执行".into());
                }
                joined_verified = true;
                evidence.record(json!({"event":"input_joined","message_id":message_id,
                    "turn_id":turn_id,"native_session_id":native_id}))?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                let allowed = !continuation_submitted
                    && approval.is_none()
                    && turn_id == running.to_string()
                    && method == "can_use_tool"
                    && details["subtype"] == "can_use_tool"
                    && details["tool_name"] == TOOL
                    && details["input"] == json!({});
                if !allowed {
                    session
                        .send(
                            Uuid::new_v4(),
                            RuntimeAction::RespondApproval {
                                approval_id,
                                decision: ApprovalDecision::DenyOnce,
                            },
                        )
                        .await?;
                    return Err("超出单次无副作用 inspect 审批范围，已拒绝".into());
                }
                approval = Some(approval_id.clone());
                evidence.record(json!({"event":"inspect_approval_requested",
                    "approval_id":approval_id,"turn_id":turn_id,"exact_inspect_fixture":true}))?;
                inputs.insert(joined);
                submit(session, joined,
                    "This is the second input to the already running batch. Retain its pending inspect. \
                     Do not call any additional tool. After inspect returns, keep listing integers \
                     from 1 to 100000 one per line until the host interrupts. Do not finish early.".into()).await?;
                evidence.record(json!({"event":"input_submitted","phase":"batch_cancel",
                    "message_id":joined,"active_turn_id":running,"submitted_input_count":2,
                    "submitted_while_running":true}))?;
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                if approval.as_ref() != Some(&approval_id)
                    || !approval_sent
                    || decision != ApprovalDecision::AllowOnce
                    || approval_resolved
                {
                    return Err("未知、重复或不符合固定范围的审批结束".into());
                }
                approval_resolved = true;
                evidence.record(
                    json!({"event":"inspect_approval_allowed","approval_id":approval_id,
                    "decision":"AllowOnce","native_receipt":false}),
                )?;
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                if request.turn_id != running.to_string()
                    || request.tool != "inspect_local_tasks"
                    || request.arguments != json!({})
                    || !approval_resolved
                    || inspect_reply.is_some()
                {
                    session
                        .send(
                            Uuid::new_v4(),
                            RuntimeAction::RespondLocalTool {
                                turn_id: request.turn_id,
                                call_id: request.call_id,
                                result: Err("仅授权一次固定 inspect 探针".into()),
                            },
                        )
                        .await?;
                    return Err("未知、重复或含副作用的本地工具请求".into());
                }
                let id = Uuid::new_v4();
                local_controls.insert(id);
                inspect_reply = Some((id, request.call_id.clone()));
                session
                    .send(
                        id,
                        RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id.clone(),
                            call_id: request.call_id.clone(),
                            result: Ok(json!({"probe":"BATCH_CANCEL_INSPECT_OK"})),
                        },
                    )
                    .await?;
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                let expected = if continuation_submitted {
                    continuation
                } else if terminals.is_empty() {
                    joined
                } else {
                    running
                };
                let (outcome_name, error_bytes, error_sha256) = match &outcome {
                    TurnOutcome::Completed => ("Completed", 0, None),
                    TurnOutcome::Cancelled => ("Cancelled", 0, None),
                    TurnOutcome::Failed { message } => (
                        "Failed",
                        message.len(),
                        Some(format!("{:x}", Sha256::digest(message.as_bytes()))),
                    ),
                };
                evidence.record(json!({"event":"cancel_terminal_observed",
                    "phase":if continuation_submitted {"continue"} else {"batch_cancel"},
                    "turn_id":turn_id,"native_session_id":native_id,"outcome":outcome_name,
                    "expected_turn_id":expected,"turn_id_matches":turn_id == expected.to_string(),
                    "interrupt_acknowledged":interrupt_acknowledged,"turn_started":started.contains(&expected),
                    "error_bytes":error_bytes,"error_sha256":error_sha256}))?;
                if continuation_submitted {
                    if turn_id != continuation.to_string()
                        || outcome != TurnOutcome::Completed
                        || !started.contains(&continuation)
                        || !accepted.contains(&continuation)
                        || output.trim() != marker
                    {
                        return Err("同会话继续没有真实完成，或旧回调重复关闭了新执行".into());
                    }
                    continued = true;
                    evidence.record(json!({"event":"turn_finished","phase":"continue",
                        "turn_id":turn_id,"native_session_id":native_id,"outcome":"Completed",
                        "output":marker,"same_native_session_verified":true}))?;
                } else {
                    let expected = if terminals.is_empty() {
                        joined
                    } else {
                        running
                    };
                    if interrupt_id.is_none()
                        || !interrupt_acknowledged
                        || !joined_verified
                        || terminals.len() >= 2
                        || turn_id != expected.to_string()
                        || outcome != TurnOutcome::Cancelled
                    {
                        return Err("合并批次没有以加入输入先、真实执行后的顺序原生确认取消".into());
                    }
                    terminals.push(expected);
                    evidence.record(json!({"event":"turn_finished","phase":"batch_cancel",
                        "turn_id":turn_id,"native_session_id":native_id,"outcome":"Cancelled",
                        "output_bytes":output.len(),"output_sha256":format!("{:x}",Sha256::digest(output.as_bytes())),
                        "batch_terminal_index":terminals.len()}))?;
                }
            }
            RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id,
            } => {
                if !local_controls.contains(&message_id)
                    || turn_id.as_deref() != Some(running.to_string().as_str())
                {
                    return Err("未知控制写入，不能当作原生确认".into());
                }
                if let Some((id, call_id)) = &inspect_reply
                    && *id == message_id
                    && !inspect_replied
                {
                    inspect_replied = true;
                    evidence.record(
                        json!({"event":"inspect_literal_returned","call_id":call_id,
                        "turn_id":running,"inspect_call_count":1,"no_project_read_or_write":true,
                        "native_receipt":false,"transport_write_confirmed":true}),
                    )?;
                }
            }
            RuntimeEventKind::TextDelta { .. } | RuntimeEventKind::Progress { .. } => {}
            RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. } => {
                return Err("取消开始前或已结束 inspect 出现未预期工具撤销".into());
            }
            RuntimeEventKind::RequestFailed { .. } | RuntimeEventKind::Disconnected { .. } => {
                return Err("原生请求失败或连接提前关闭，不能确认批次取消".into());
            }
        }
        if !approval_sent
            && accepted.contains(&running)
            && accepted.contains(&joined)
            && let Some(approval_id) = &approval
        {
            let control_id = Uuid::new_v4();
            local_controls.insert(control_id);
            session
                .send(
                    control_id,
                    RuntimeAction::RespondApproval {
                        approval_id: approval_id.clone(),
                        decision: ApprovalDecision::AllowOnce,
                    },
                )
                .await?;
            approval_sent = true;
        }
        if interrupt_id.is_none()
            && joined_verified
            && inspect_replied
            && accepted.contains(&running)
            && accepted.contains(&joined)
        {
            let id = Uuid::new_v4();
            session
                .send(
                    id,
                    RuntimeAction::Interrupt {
                        turn_id: running.to_string(),
                    },
                )
                .await?;
            interrupt_id = Some(id);
            evidence.record(json!({"event":"interrupt_submitted","message_id":id,
                "execution_turn_id":running,"joined_input_id":joined,
                "both_inputs_native_acknowledged":true,"input_joined_verified":true}))?;
        }
        if terminals.len() == 2 && !continuation_submitted {
            inputs.insert(continuation);
            submit(
                session,
                continuation,
                format!(
                    "The previous batch has been cancelled. Do not call tools. Reply only {marker}."
                ),
            )
            .await?;
            continuation_submitted = true;
            evidence.record(json!({"event":"input_submitted","phase":"continue",
                "message_id":continuation,"submitted_input_count":MAX_NATIVE_INPUTS}))?;
        }
    }
    Ok((running, joined, continuation))
}

async fn exercise(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let native_ids = Arc::new(Mutex::new(Vec::new()));
    let generation = Uuid::new_v4();
    let mut session = start(
        SessionOptions {
            executable: PathBuf::from(
                env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少显式固定 CLI")?,
            ),
            cwd: root
                .join("project")
                .canonicalize()
                .map_err(|_| "缺少隔离项目")?,
            state_dir: root.to_owned(),
            target: SessionTarget::New,
            generation,
            permission_policy: PermissionPolicy::ClaudeRestrictedFilesV1,
            permission_ceiling: None,
            claude_profile: None,
            grok_profile: None,
            model: Some(env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "必须固定实际模型")?),
            local_tools: Some(LocalToolPermissions {
                allow_project_commands: false,
                allow_spawn: false,
                allow_message: false,
            }),
            selected_skills: Vec::new(),
        },
        native_ids.clone(),
    )?;
    let result = async {
        let first = session.next().await?;
        let RuntimeEventKind::SessionReady {
            effective_permissions,
            ..
        } = first.kind
        else {
            return Err("初始化没有返回原生固定策略就绪状态".into());
        };
        observe_permissions(&effective_permissions, evidence)?;
        cancel_and_continue(&mut session, evidence).await
    }
    .await;
    let shutdown = session.shutdown(evidence).await;
    let receipt = managed_process::confirmed_exit(root, generation);
    let cleanup_confirmed = matches!(&receipt, Ok(Some(_)));
    evidence.record(json!({"event":"cleanup_checked","generation":generation,
        "cleanup_confirmed":cleanup_confirmed,"transport_closed":shutdown.is_ok(),
        "cleanup_receipt_read":true,"receipt":receipt.as_ref().ok().and_then(|receipt| receipt.as_ref()).map(|receipt| json!({
            "generation":receipt.generation,"containment":receipt.containment,
            "cleanup_confirmed":receipt.cleanup_confirmed,"exit_code":receipt.exit_code,
            "exit_reason":receipt.exit_reason}))}))?;
    let rows = native_ids.lock().map_err(|_| "原生证据账本锁失败")?.clone();
    for (index, row) in rows.iter().enumerate() {
        evidence.record(json!({"event":"native_protocol_ids","sequence":index+1,"native":row}))?;
    }
    let (running, joined, continuation) = result?;
    shutdown?;
    receipt.map_err(|_| "生产监督者退出回执验证失败")?;
    if !cleanup_confirmed {
        return Err("没有真实生产清理回执，不能计为通过".into());
    }
    let native_id = session.native_id.as_deref().ok_or("真实会话没有原生 ID")?;
    evidence.record(audit_batch(&rows, running, joined, native_id)?)?;
    let continuation_results = rows
        .iter()
        .filter(|row| {
            row["direction"] == "stdout"
                && row["type"] == "result"
                && row["session_id"] == native_id
                && row["subtype"] == "success"
                && row["is_error"] == false
                && row["user_message_uuid"] == continuation.to_string()
                && row["user_message_uuids"] == json!([continuation])
        })
        .count();
    let native_inputs = rows
        .iter()
        .filter(|row| row["direction"] == "stdin" && row["type"] == "user")
        .count();
    if continuation_results != 1 || native_inputs != MAX_NATIVE_INPUTS {
        return Err("继续缺少唯一原生结果，或原生输入超出三个提交预算".into());
    }
    evidence.record(json!({"event":"acceptance_passed","scope":SCOPE,
        "native_session_id":native_id,"native_inputs":native_inputs,"native_executions":2,
        "joined_inputs":1,"cancelled_inputs":2,"continued_inputs":1,"inspect_call_count":1,
        "all_three_signals_verified":true,"same_native_session_verified":true,
        "cleanup_confirmed":true,"transport_closed":true,"credential_files_read_by_probe":false,
        "persistence_verified":false,"app_restart_and_ui_verified":false,
        "parent_permission_ceiling_verified":false}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "仅由既有隔离运行器显式启动；调用真实模型并产生费用"]
async fn real_claude_joined_batch_cancel() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-live-probe")).unwrap(),
        "isolated Claude Rust adapter verification\n"
    );
    let config = PathBuf::from(env::var_os("CLAUDE_CONFIG_DIR").expect("缺少私有配置边界"))
        .canonicalize()
        .unwrap();
    let declared =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").expect("缺少明确配置边界"))
            .canonicalize()
            .unwrap();
    assert_eq!(config, declared);
    assert!(!root.starts_with(&config));
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").expect("缺少私有证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence
        .record(json!({"event":"acceptance_started","scope":SCOPE,
        "max_native_inputs":MAX_NATIVE_INPUTS,"max_inspect_calls":1,
        "permission_policy":"ClaudeRestrictedFilesV1","credential_files_read_by_probe":false,
        "real_gui_verified":false,"persistence_verified":false}))
        .unwrap();
    let result = exercise(&root, &mut evidence).await;
    if let Err(reason) = &result {
        evidence
            .record(json!({"event":"acceptance_failed","scope":SCOPE,"reason":reason}))
            .unwrap();
    }
    assert!(result.is_ok(), "真实共享批次取消未通过；检查安全投影证据");
}

#[test]
fn tools_batch_result_requires_the_same_complete_cancellation_identity() {
    let running = Uuid::from_u128(10);
    let joined = Uuid::from_u128(11);
    let native_id = Uuid::from_u128(20).to_string();
    let mut row = json!({"direction":"stdout","type":"result","subtype":"error_during_execution",
        "is_error":true,"terminal_reason":"aborted_tools","session_id":native_id,
        "user_message_uuid":joined,"user_message_uuids":[running,joined]});
    assert!(batch_result_matches(&row, running, joined, &native_id));
    row["is_error"] = json!(false);
    assert!(!batch_result_matches(&row, running, joined, &native_id));
    row["is_error"] = json!(true);
    row["user_message_uuids"] = json!([joined]);
    assert!(!batch_result_matches(&row, running, joined, &native_id));
}

#[test]
fn batch_result_requires_exact_complete_native_cancellation_identity() {
    let running = Uuid::from_u128(10);
    let joined = Uuid::from_u128(11);
    let native_id = Uuid::from_u128(20).to_string();
    let row = json!({"direction":"stdout","type":"result","subtype":"error_during_execution",
        "is_error":true,"terminal_reason":"aborted_streaming","session_id":native_id,
        "user_message_uuid":joined,"user_message_uuids":[running,joined]});
    assert!(batch_result_matches(&row, running, joined, &native_id));
    for key in [
        "session_id",
        "user_message_uuid",
        "terminal_reason",
        "subtype",
        "is_error",
    ] {
        let mut invalid = row.clone();
        invalid[key] = Value::Null;
        assert!(
            !batch_result_matches(&invalid, running, joined, &native_id),
            "{key}"
        );
    }
    let mut incomplete = row.clone();
    incomplete["user_message_uuids"] = json!([joined]);
    assert!(!batch_result_matches(
        &incomplete,
        running,
        joined,
        &native_id
    ));
    incomplete["user_message_uuids"] = json!([running, joined, joined]);
    assert!(!batch_result_matches(
        &incomplete,
        running,
        joined,
        &native_id
    ));
}

#[test]
fn native_batch_cancel_audit_rejects_missing_signals_and_cancel_before_join() {
    let running = Uuid::from_u128(10);
    let joined = Uuid::from_u128(11);
    let native_id = Uuid::from_u128(20).to_string();
    let request_id = format!("infinishell-{}-3", Uuid::from_u128(1));
    let lifecycle = |id, state| {
        json!({"direction":"stdout","type":"command_lifecycle",
        "session_id":native_id,"command_uuid":id,"state":state})
    };
    let rows = vec![
        lifecycle(running, "queued"),
        lifecycle(running, "started"),
        lifecycle(joined, "queued"),
        lifecycle(joined, "started"),
        json!({"direction":"stdin","type":"control_request","request_subtype":"interrupt",
            "request_id":request_id}),
        json!({"direction":"stdout","type":"control_response","response_subtype":"success",
            "response_request_id":request_id}),
        json!({"direction":"stdout","type":"result","subtype":"error_during_execution",
            "is_error":true,"terminal_reason":"aborted_streaming","session_id":native_id,
            "uuid":Uuid::from_u128(30),"user_message_uuid":joined,
            "user_message_uuids":[running,joined]}),
        lifecycle(running, "cancelled"),
    ];
    assert!(audit_batch(&rows, running, joined, &native_id).is_ok());
    for missing in [0, 1, 2, 3, 4, 5, 6, 7] {
        let mut invalid = rows.clone();
        invalid.remove(missing);
        // started 本身也是原生接收证据，缺 queued 不应误报失败。
        if matches!(missing, 0 | 2) {
            assert!(audit_batch(&invalid, running, joined, &native_id).is_ok());
        } else {
            assert!(
                audit_batch(&invalid, running, joined, &native_id).is_err(),
                "{missing}"
            );
        }
    }
    let mut early = rows.clone();
    early.swap(3, 4);
    assert!(audit_batch(&early, running, joined, &native_id).is_err());
    let mut failed_ack = rows;
    failed_ack[5]["response_subtype"] = json!("error");
    assert!(audit_batch(&failed_ack, running, joined, &native_id).is_err());
}
