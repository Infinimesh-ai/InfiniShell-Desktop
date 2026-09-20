//! 等待精确 Edit 审批时的真实取消校准；未授权编辑，也不放宽生产取消判据。

use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::{
    ApprovalDecision, Duration, Evidence, File, HashSet, LiveSession, Path, PathBuf,
    PermissionPolicy, RuntimeAction, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
    Uuid, Value, env, fs, json, observe_permissions, start, submit,
};
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};

const SCOPE: &str = "rust_adapter_pending_edit_cancel";
const BEFORE: &str = "APPROVAL_CANCEL_BEFORE\n";
const AFTER: &str = "APPROVAL_CANCEL_AFTER\n";
const FILE_REF: &str = "edit-cancel.txt";
const MAX_NATIVE_INPUTS: usize = 2;
const MAX_NATIVE_RECORDS: usize = 512;

fn digest(bytes: &[u8]) -> String {
    let checksum = Sha256::digest(bytes);
    format!("{checksum:x}")
}

fn exact_edit(details: &Value, path: &Path) -> bool {
    details["subtype"] == "can_use_tool"
        && details["tool_name"] == "Edit"
        && details["input"]["file_path"] == json!(path)
        && details["input"]["old_string"] == BEFORE
        && details["input"]["new_string"] == AFTER
        && details["input"].as_object().is_some_and(|input| {
            input.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "file_path" | "old_string" | "new_string" | "replace_all"
                )
            }) && input
                .get("replace_all")
                .is_none_or(|value| value.as_bool() == Some(false))
        })
        && details["tool_use_id"].as_str().is_some_and(|id| {
            id.starts_with("toolu_")
                && id.len() <= 160
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
}

fn unchanged(path: &Path, phase: &str, evidence: &mut Evidence) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "隔离目标文件不可读")?;
    if !metadata.is_file() || path.is_symlink() {
        return Err("隔离目标文件不是普通文件".into());
    }
    let bytes = fs::read(path).map_err(|_| "隔离目标文件读取失败")?;
    if bytes != BEFORE.as_bytes() {
        return Err("未授权 Edit 改变了目标文件".into());
    }
    evidence.record(
        json!({"event":"file_unchanged","phase":phase,"file_ref":FILE_REF,
        "full_bytes_verified":true,"file_bytes":bytes.len(),"file_sha256":digest(&bytes)}),
    )
}

async fn cancel_and_continue(
    session: &mut LiveSession,
    path: &Path,
    evidence: &mut Evidence,
) -> Result<(Uuid, Uuid, String, String), String> {
    let running = Uuid::new_v4();
    let continuation = Uuid::new_v4();
    let marker = format!("APPROVAL_CANCEL_CONTINUED_{}", Uuid::new_v4().simple());
    let mut accepted = HashSet::new();
    let mut started = HashSet::new();
    let mut approval = None;
    let mut tool_use_id = None;
    let mut approval_cancelled = false;
    let mut interrupt = None;
    let mut interrupt_acknowledged = false;
    let mut cancelled = false;
    let mut continued = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let file_path = path.display();
    submit(session, running, format!(
        "这是隔离审批取消验收。若 Edit 需要先读取文件，仅允许一次 Read 读取 {file_path}。\
         不要读取其它路径或调用 Bash、搜索等其它工具。只调用一次 Edit，\
         file_path 为 {file_path}，old_string 为 {BEFORE:?}，new_string 为 {AFTER:?}，replace_all 为 false。\
         编辑需要宿主审批；等待决定，不要另建任务或追加工具。"
    )).await?;
    evidence.record(json!({"event":"input_submitted","phase":"pending_edit",
        "message_id":running,"submitted_input_count":1}))?;
    while !continued {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "等待审批取消和继续超过180秒")??;
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
                if interrupt == Some(message_id) {
                    if turn_id.as_deref() != Some(running.to_string().as_str())
                        || interrupt_acknowledged
                    {
                        return Err("原生取消确认关联错误或重复".into());
                    }
                    interrupt_acknowledged = true;
                    evidence
                        .record(json!({"event":"interrupt_accepted","message_id":message_id,
                        "turn_id":turn_id,"native_session_id":native_id,"native_receipt":true}))?;
                } else {
                    if !((message_id == running && !cancelled)
                        || (message_id == continuation && cancelled))
                        || turn_id.as_deref() != Some(message_id.to_string().as_str())
                        || !accepted.insert(message_id)
                    {
                        return Err("原生接收确认不属于本轮唯一输入".into());
                    }
                    evidence.record(json!({"event":"message_accepted","message_id":message_id,
                        "turn_id":turn_id,"native_session_id":native_id,"native_receipt":true}))?;
                }
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                let id = Uuid::parse_str(&turn_id).map_err(|_| "执行ID不是UUID")?;
                if !accepted.contains(&id)
                    || !started.insert(id)
                    || !((id == running && !cancelled) || (id == continuation && cancelled))
                {
                    return Err("原生执行未经接收确认或发生重复".into());
                }
                evidence.record(json!({"event":"turn_started","turn_id":turn_id,
                    "native_session_id":native_id}))?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                if approval.is_some()
                    || cancelled
                    || turn_id != running.to_string()
                    || !accepted.contains(&running)
                    || !started.contains(&running)
                    || method != "can_use_tool"
                    || !exact_edit(&details, path)
                {
                    session
                        .send(
                            Uuid::new_v4(),
                            RuntimeAction::RespondApproval {
                                approval_id,
                                decision: ApprovalDecision::DenyOnce,
                            },
                        )
                        .await?;
                    return Err("出现超出唯一待取消Edit范围的审批，已拒绝".into());
                }
                let native_tool = details["tool_use_id"]
                    .as_str()
                    .ok_or("缺少原生工具ID")?
                    .to_owned();
                approval = Some(approval_id.clone());
                tool_use_id = Some(native_tool.clone());
                let mut keys = details["input"]
                    .as_object()
                    .ok_or("Edit输入不是对象")?
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                keys.sort();
                evidence.record(json!({"event":"edit_approval_requested","approval_id":approval_id,
                    "turn_id":turn_id,"native_session_id":native_id,"tool_use_id":native_tool,
                    "exact_edit_verified":true,"edit_allowed":false,"file_ref":FILE_REF,
                    "old_string_sha256":digest(BEFORE.as_bytes()),"new_string_sha256":digest(AFTER.as_bytes()),
                    "input_keys":keys,"replace_all":details["input"]["replace_all"]}))?;
                let id = Uuid::new_v4();
                interrupt = Some(id);
                session
                    .send(
                        id,
                        RuntimeAction::Interrupt {
                            turn_id: running.to_string(),
                        },
                    )
                    .await?;
                evidence.record(json!({"event":"interrupt_submitted","message_id":id,
                    "execution_turn_id":running,"approval_id":approval,"approval_still_pending":true,
                    "edit_allowed":false}))?;
            }
            RuntimeEventKind::ApprovalCancelled { approval_id } => {
                if approval.as_ref() != Some(&approval_id)
                    || interrupt.is_none()
                    || approval_cancelled
                {
                    return Err("审批撤销没有对应本轮唯一取消".into());
                }
                approval_cancelled = true;
                evidence.record(
                    json!({"event":"edit_approval_cancelled","approval_id":approval_id,
                    "native_session_id":native_id,"native_execution_cancelled_verified":false}),
                )?;
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                let expected = if cancelled { continuation } else { running };
                let (outcome_name, error_bytes, error_sha256) = match &outcome {
                    TurnOutcome::Completed => ("Completed", 0, None),
                    TurnOutcome::Cancelled => ("Cancelled", 0, None),
                    TurnOutcome::Failed { message } => {
                        ("Failed", message.len(), Some(digest(message.as_bytes())))
                    }
                };
                evidence.record(json!({"event":"cancel_terminal_observed",
                    "phase":if cancelled {"continue"} else {"pending_edit"},
                    "turn_id":turn_id,"native_session_id":native_id,"outcome":outcome_name,
                    "expected_turn_id":expected,"turn_id_matches":turn_id == expected.to_string(),
                    "interrupt_acknowledged":interrupt_acknowledged,"approval_cancelled":approval_cancelled,
                    "turn_started":started.contains(&expected),"error_bytes":error_bytes,
                    "error_sha256":error_sha256}))?;
                if !cancelled {
                    if turn_id != running.to_string()
                        || outcome != TurnOutcome::Cancelled
                        || !interrupt_acknowledged
                        || !approval_cancelled
                        || !started.contains(&running)
                    {
                        return Err("等待Edit审批的执行没有取得生产原生取消终态".into());
                    }
                    cancelled = true;
                    evidence.record(json!({"event":"turn_finished","phase":"pending_edit",
                        "turn_id":turn_id,"native_session_id":native_id,"outcome":"Cancelled",
                        "output_bytes":output.len(),"full_output_sha256":digest(output.as_bytes())}))?;
                    unchanged(path, "after_cancel", evidence)?;
                    submit(
                        session,
                        continuation,
                        format!("前一轮已经取消。不要继续编辑或调用任何工具。只回复 {marker}。"),
                    )
                    .await?;
                    evidence.record(json!({"event":"input_submitted","phase":"continue",
                        "message_id":continuation,"submitted_input_count":2}))?;
                } else {
                    if turn_id != continuation.to_string()
                        || outcome != TurnOutcome::Completed
                        || !accepted.contains(&continuation)
                        || !started.contains(&continuation)
                        || output.trim() != marker
                    {
                        return Err("同会话继续没有完成精确随机标记".into());
                    }
                    continued = true;
                    evidence.record(json!({"event":"turn_finished","phase":"continue",
                        "turn_id":turn_id,"native_session_id":native_id,"outcome":"Completed",
                        "output":marker,"output_bytes":output.len(),
                        "full_output_sha256":digest(output.as_bytes()),
                        "trimmed_output_sha256":digest(output.trim().as_bytes()),
                        "same_native_session_verified":true}))?;
                    unchanged(path, "after_continue", evidence)?;
                }
            }
            RuntimeEventKind::TextDelta { .. } | RuntimeEventKind::Progress { .. } => {}
            RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::RequestFailed { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err(
                    "出现审批响应、未知工具、合并输入或连接失败，不能确认待审批取消".into(),
                );
            }
        }
    }
    Ok((
        running,
        continuation,
        approval.ok_or("没有实际Edit审批")?,
        tool_use_id.ok_or("没有实际Edit工具ID")?,
    ))
}

fn audit_native(
    rows: &[Value],
    generation: Uuid,
    native_id: &str,
    running: Uuid,
    continuation: Uuid,
    approval: &str,
    tool_use_id: &str,
) -> Result<Value, String> {
    let select = |direction: &str, kind: &str| {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| row["direction"] == direction && row["type"] == kind)
            .collect::<Vec<_>>()
    };
    let users = select("stdin", "user");
    if users.len() != MAX_NATIVE_INPUTS
        || users[0].1["uuid"] != running.to_string()
        || users[1].1["uuid"] != continuation.to_string()
        || users[1].1["session_id"] != native_id
    {
        return Err("原生输入预算或继续身份不匹配".into());
    }
    let controls = select("stdin", "control_request")
        .into_iter()
        .filter(|(_, row)| row["request_subtype"] == "interrupt")
        .collect::<Vec<_>>();
    let [(control_index, control)] = controls.as_slice() else {
        return Err("没有恰好一条真实原生取消请求".into());
    };
    let request_id = control["request_id"].as_str().ok_or("取消请求没有ID")?;
    if !request_id.starts_with(&format!("infinishell-{generation}-")) {
        return Err("取消请求来自过时运行代".into());
    }
    let acks = select("stdout", "control_response")
        .into_iter()
        .filter(|(_, row)| row["response_request_id"] == request_id)
        .collect::<Vec<_>>();
    let [(ack_index, ack)] = acks.as_slice() else {
        return Err("取消没有唯一原生ACK".into());
    };
    let lifecycles = select("stdout", "command_lifecycle")
        .into_iter()
        .filter(|(_, row)| {
            row["session_id"] == native_id
                && row["command_uuid"] == running.to_string()
                && row["state"] == "cancelled"
        })
        .collect::<Vec<_>>();
    let [(cancelled_index, _)] = lifecycles.as_slice() else {
        return Err("执行没有唯一原生取消生命周期".into());
    };
    let results = select("stdout", "result");
    let [(result_index, result), (continue_index, continued)] = results.as_slice() else {
        return Err("没有恰好两份原生执行结果".into());
    };
    let shape = (matches!(
        result["terminal_reason"].as_str(),
        Some("aborted_streaming" | "aborted_tools")
    ) && result["subtype"] == "error_during_execution"
        && result["is_error"] == true)
        || (matches!(
            result["terminal_reason"].as_str(),
            Some("interrupted" | "cancelled")
        ) && result["subtype"] == "success"
            && result["is_error"] == false);
    let permission = select("stdout", "control_request")
        .into_iter()
        .filter(|(_, row)| row["request_subtype"] == "can_use_tool")
        .collect::<Vec<_>>();
    let [(permission_index, pending)] = permission.as_slice() else {
        return Err("没有唯一原生待审批请求".into());
    };
    let permission_answers = select("stdin", "control_response")
        .into_iter()
        .filter(|(_, row)| row["response_request_id"] == approval)
        .count();
    if !shape
        || ack["response_subtype"] != "success"
        || result["session_id"] != native_id
        || result["user_message_uuid"] != running.to_string()
        || result["user_message_uuids"] != json!([running])
        || pending["request_id"] != approval
        || pending["tool_use_id"] != tool_use_id
        || permission_answers != 0
        || continued["session_id"] != native_id
        || continued["user_message_uuid"] != continuation.to_string()
        || continued["user_message_uuids"] != json!([continuation])
        || continued["subtype"] != "success"
        || continued["is_error"] != false
        || !matches!(
            continued["terminal_reason"].as_str(),
            None | Some("completed" | "end_turn")
        )
        || !(*permission_index < *control_index
            && *control_index < *ack_index
            && *control_index < *cancelled_index
            && *control_index < *result_index
            && *ack_index < users[1].0
            && *cancelled_index < users[1].0
            && *result_index < users[1].0
            && users[1].0 < *continue_index)
    {
        return Err("原生请求、确认、取消生命周期或完整结果关联错误".into());
    }
    let mut reads = HashSet::new();
    for (index, row) in select("stdout", "assistant") {
        for tool in row["tools"].as_array().into_iter().flatten() {
            if tool["name"] == "Read" && index < *permission_index {
                reads.insert(tool["id"].as_str().ok_or("读取工具没有原生ID")?);
            } else if tool["id"] != tool_use_id || tool["name"] != "Edit" {
                return Err("出现未授权的原生工具".into());
            }
        }
    }
    if reads.len() > 1 {
        return Err("读取工具超过一次固定预算".into());
    }
    Ok(
        json!({"event":"native_pending_edit_cancel_verified","native_session_id":native_id,
        "runtime_generation":generation,"execution_input_id":running,"continuation_input_id":continuation,
        "approval_id":approval,"tool_use_id":tool_use_id,"interrupt_request_id":request_id,
        "native_result_uuid":result["uuid"],"continue_result_uuid":continued["uuid"],
        "terminal_reason":result["terminal_reason"],"is_error":result["is_error"],
        "user_message_uuids":result["user_message_uuids"],"edit_allowed":false,
        "read_call_count":reads.len(),"all_four_evidence_verified":true}),
    )
}

async fn exercise(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let path = root.join("project").join(FILE_REF);
    unchanged(&path, "before", evidence)?;
    let generation = Uuid::new_v4();
    let native_ids = Arc::new(Mutex::new(Vec::new()));
    let mut session = start(
        SessionOptions {
            executable: PathBuf::from(
                env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定CLI")?,
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
            model: Some(env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "缺少固定模型")?),
            local_tools: None,
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
            return Err("固定策略没有就绪".into());
        };
        observe_permissions(&effective_permissions, evidence)?;
        cancel_and_continue(&mut session, &path, evidence).await
    }
    .await;
    // 验收失败也先关闭真实监督连接，再保存严格白名单协议账本；不把清理当作取消成功。
    let shutdown = session.shutdown(evidence).await;
    let receipt = managed_process::confirmed_exit(root, generation);
    let normal_exit = receipt
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .is_some_and(|receipt| {
            receipt.exit_reason == ExitReason::StdioClosed && receipt.exit_code == Some(0)
        });
    evidence.record(json!({"event":"cleanup_checked","generation":generation,
        "cleanup_confirmed":matches!(&receipt, Ok(Some(_))),"normal_exit":normal_exit,
        "transport_closed":shutdown.is_ok(),"cleanup_receipt_read":true,
        "receipt":receipt.as_ref().ok().and_then(Option::as_ref).map(|receipt| json!({
            "version":receipt.version,"generation":receipt.generation,"containment":receipt.containment,
            "cleanup_confirmed":receipt.cleanup_confirmed,"exit_code":receipt.exit_code,
            "exit_reason":receipt.exit_reason}))}))?;
    let rows = native_ids.lock().map_err(|_| "原生账本锁失败")?.clone();
    for (index, row) in rows.iter().take(MAX_NATIVE_RECORDS).enumerate() {
        evidence.record(
            json!({"event":"native_protocol_ids","generation":generation,
            "sequence":index+1,"native":row}),
        )?;
    }
    if rows.len() > MAX_NATIVE_RECORDS {
        return Err("原生投影超过512条，验收失败".into());
    }
    let (running, continuation, approval, tool) = result?;
    shutdown?;
    receipt.map_err(|_| "生产清理回执验证失败")?;
    if !normal_exit {
        return Err("未取得生产stdio_closed和exit0".into());
    }
    unchanged(&path, "after_shutdown", evidence)?;
    let native_id = session.native_id.as_deref().ok_or("缺少原生会话ID")?;
    evidence.record(audit_native(
        &rows,
        generation,
        native_id,
        running,
        continuation,
        &approval,
        &tool,
    )?)?;
    evidence.record(json!({"event":"acceptance_passed","scope":SCOPE,"native_session_id":native_id,
        "native_inputs":MAX_NATIVE_INPUTS,"edit_allowed":false,"all_four_evidence_verified":true,
        "full_file_unchanged_verified":true,"same_native_session_verified":true,
        "normal_exit":true,"cleanup_confirmed":true,"transport_closed":true,
        "real_gui_verified":false,"persistence_verified":false,"http_request_count_verified":false}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要隔离API运行器、固定Claude 2.1.273与生产监督入口；调用真实模型"]
async fn real_claude_pending_edit_cancel() {
    let root = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("缺少隔离根目录"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-live-probe")).unwrap(),
        "isolated Claude Rust adapter verification\n"
    );
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_APPROVAL_CANCEL_MAX_NATIVE_INPUTS").unwrap(),
        "2"
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
        root: root.clone(),
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").expect("缺少证据路径"),
        ))
        .unwrap(),
    };
    evidence.record(json!({"event":"acceptance_started","scope":SCOPE,"max_native_inputs":2,
        "phase_deadline_seconds":180,"permission_policy":"ClaudeRestrictedFilesV1",
        "credential_files_read_by_probe":false,"real_gui_verified":false,"persistence_verified":false})).unwrap();
    let result = exercise(&root, &mut evidence).await;
    if let Err(reason) = &result {
        evidence
            .record(json!({"event":"acceptance_failed","scope":SCOPE,
            "reason_sha256":digest(reason.as_bytes())}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "等待Edit审批取消未通过；检查白名单协议投影，不以ACK代替执行取消"
    );
}

#[test]
fn exact_edit_rejects_other_paths_replacements_and_extra_fields() {
    let path = Path::new("/isolated/project/edit-cancel.txt");
    let request = json!({"subtype":"can_use_tool","tool_name":"Edit","tool_use_id":"toolu_fixture",
        "input":{"file_path":path,"old_string":BEFORE,"new_string":AFTER,"replace_all":false}});
    assert!(exact_edit(&request, path));
    for (key, value) in [
        ("file_path", json!("/isolated/project/other.txt")),
        ("new_string", json!("不同内容")),
        ("replace_all", json!(true)),
        ("unexpected", json!(true)),
    ] {
        let mut changed = request.clone();
        changed["input"][key] = value;
        assert!(!exact_edit(&changed, path));
    }
    let mut different_tool = request;
    different_tool["tool_name"] = json!("Write");
    assert!(!exact_edit(&different_tool, path));
}
