//! 独占生产 SDK 的真实租约验收；不会替代协调器、父权限或完整 CLI 生命周期验收。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{GrokLeaseAudit, GrokProtocol, connect_protocol, validate_options};
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome, managed_process,
};

const SCOPE: &str = "authenticated_process_capability_and_native_tool_lease";
const REPLY: &str = "INFINISHELL_NATIVE_LEASE_OK";
const PROMPT: &str = "INFINISHELL_NATIVE_LEASE：只调用 infinishell-local-tasks 服务的 inspect_local_tasks 工具一次，参数必须为 {}。若需发现工具，只允许 search_tool 一次，参数严格为 {\"limit\":5,\"query\":\"infinishell-local-tasks inspect_local_tasks\"}。成功后只回复 INFINISHELL_NATIVE_LEASE_OK。不得执行其它工具、读取文件、修改文件、执行命令、发送消息或派发任务。";

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "证据写入失败".into())
}

fn exact_approval(details: &Value, native_session: &str) -> Option<bool> {
    if details["sessionId"] != native_session || details["toolCall"]["kind"] != "other" {
        return None;
    }
    let input = &details["toolCall"]["rawInput"];
    if input
        == &json!({"tool_name":"infinishell-local-tasks__inspect_local_tasks","tool_input":{},"variant":"UseTool"})
    {
        return Some(true);
    }
    if input
        == &json!({"limit":5,"query":"infinishell-local-tasks inspect_local_tasks","variant":"SearchTool"})
    {
        return Some(false);
    }
    None
}

async fn exercise(root: &Path, file: &mut File) -> Result<(), String> {
    let generation = Uuid::new_v4();
    let state_dir = root.join("state");
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少受限原生入口")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "缺少空项目")?,
        state_dir: state_dir.clone(),
        target: SessionTarget::New,
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: Some(LocalToolPermissions {
            allow_spawn: false,
            allow_message: false,
        }),
        selected_skills: Vec::new(),
    };
    // 与公开 connect 共用验证及构造；以下两个句柄只读取生产事件，不修改协议。
    validate_options(&options).map_err(|_| "生产连接参数拒绝")?;
    let mut protocol = GrokProtocol::new(options);
    let audit = Arc::new(Mutex::new(GrokLeaseAudit::default()));
    let histories = Arc::new(Mutex::new(HashMap::new()));
    protocol.lease_audit_for_live = Some(audit.clone());
    protocol.verified_final_histories_for_live = Some(histories.clone());
    let connection = connect_protocol(protocol);
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = tokio::spawn(connection.task);
    let input_id = Uuid::new_v4();
    let mut session = None;
    let mut turn = None;
    let mut submitted = 0_u64;
    let mut accepted = 0_u64;
    let mut inspect_count = 0_u64;
    let mut inspect_approvals = 0_u64;
    let mut search_approvals = 0_u64;
    let mut unexpected_tools = 0_u64;
    let mut approval_ids = HashSet::new();
    let mut resolved_ids = HashSet::new();
    let mut reply_command = None;
    let mut reply_dispatched = false;
    let mut started = false;
    let mut final_verified = false;
    let mut final_digest = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(360);
    let result: Result<(), String> = async {
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| "租约验收期限耗尽")?
                .ok_or("生产事件流已关闭")?;
            if event.generation != generation {
                return Err("收到过时连接事件".into());
            }
            if let Some(id) = event.native_session_id {
                if Uuid::parse_str(&id).is_err()
                    || session.as_ref().is_some_and(|previous| previous != &id)
                {
                    return Err("原生会话身份改变".into());
                }
                session = Some(id);
            }
            match event.kind {
                RuntimeEventKind::SessionReady {
                    effective_permissions,
                    ..
                } => {
                    if submitted != 0
                        || session.is_none()
                        || effective_permissions["verifiedCapabilities"]["childTasks"] != false
                    {
                        return Err("初始化或父权限边界不满足".into());
                    }
                    submitted += 1;
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: input_id,
                            action: RuntimeAction::Submit {
                                input: vec![InputContent::Text(PROMPT.into())],
                            },
                        })
                        .await
                        .map_err(|_| "无法提交唯一输入")?;
                }
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } => {
                    if message_id != input_id
                        || accepted != 0
                        || turn_id
                            .as_deref()
                            .is_none_or(|id| Uuid::parse_str(id).is_err())
                    {
                        return Err("接收回执身份不匹配".into());
                    }
                    accepted += 1;
                    turn = turn_id;
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if turn.as_deref() != Some(turn_id.as_str()) || started {
                        return Err("运行回合不匹配".into());
                    }
                    started = true;
                }
                RuntimeEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    method,
                    details,
                } => {
                    let approved_kind = session
                        .as_deref()
                        .and_then(|id| exact_approval(&details, id));
                    let allowed = started
                        && turn.as_deref() == Some(turn_id.as_str())
                        && method == "session/request_permission"
                        && approval_ids.insert(approval_id.clone())
                        && approved_kind.is_some_and(|inspect| {
                            if inspect {
                                inspect_approvals == 0
                            } else {
                                search_approvals == 0
                            }
                        });
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: Uuid::new_v4(),
                            action: RuntimeAction::RespondApproval {
                                approval_id,
                                decision: if allowed {
                                    ApprovalDecision::AllowOnce
                                } else {
                                    ApprovalDecision::DenyOnce
                                },
                            },
                        })
                        .await
                        .map_err(|_| "审批响应发送失败")?;
                    if !allowed {
                        unexpected_tools += 1;
                        return Err("只允许单次精确 inspect 或发现工具审批".into());
                    }
                    if approved_kind == Some(true) {
                        inspect_approvals += 1;
                    } else {
                        search_approvals += 1;
                    }
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } => {
                    if !approval_ids.contains(&approval_id)
                        || !resolved_ids.insert(approval_id)
                        || decision != ApprovalDecision::AllowOnce
                    {
                        return Err("审批实际响应不匹配".into());
                    }
                }
                RuntimeEventKind::LocalToolRequested { request } => {
                    if !started
                        || turn.as_deref() != Some(request.turn_id.as_str())
                        || inspect_count != 0
                        || request.tool != "inspect_local_tasks"
                        || request.arguments != json!({})
                        || inspect_approvals != 1
                    {
                        unexpected_tools += 1;
                        return Err("生产本地工具请求越界".into());
                    }
                    inspect_count += 1;
                    let message_id = Uuid::new_v4();
                    reply_command = Some(message_id);
                    // 空隔离任务集的固定结果；此步骤只验证 SDK 传输，不声称协调器数据库验收。
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id,
                            action: RuntimeAction::RespondLocalTool {
                                turn_id: request.turn_id,
                                call_id: request.call_id,
                                result: Ok(
                                    json!({"tasks":[],"fixture_scope":"isolated_empty_tasks"}),
                                ),
                            },
                        })
                        .await
                        .map_err(|_| "本地工具结果发送失败")?;
                }
                RuntimeEventKind::CommandDispatched {
                    message_id,
                    turn_id,
                } => {
                    if reply_command == Some(message_id) {
                        if reply_dispatched || turn_id.as_deref() != turn.as_deref() {
                            return Err("工具响应写入回执不匹配".into());
                        }
                        reply_dispatched = true;
                    }
                }
                RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } => {
                    if turn.as_deref() != Some(turn_id.as_str())
                        || !started
                        || outcome != TurnOutcome::Completed
                        || !reply_dispatched
                        || approval_ids != resolved_ids
                    {
                        return Err("尚未获得真实完成条件".into());
                    }
                    let snapshots = histories.lock().expect("历史验收锁未损坏");
                    let snapshot = snapshots.get(&turn_id).ok_or("没有生产完整最终历史")?;
                    let receipt = super::live_tests::final_response_receipt(
                        snapshot,
                        session.as_deref().ok_or("缺少原生会话")?,
                        &turn_id,
                        &outcome,
                    )?;
                    if receipt.final_response.trim() != REPLY || receipt.full_output != output {
                        return Err("真实最终结果不匹配".into());
                    }
                    final_digest = Some(format!(
                        "{:x}",
                        Sha256::digest(receipt.final_response.trim().as_bytes())
                    ));
                    final_verified = true;
                    return Ok(());
                }
                RuntimeEventKind::TextDelta { turn_id, .. }
                | RuntimeEventKind::Progress { turn_id, .. } => {
                    if turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("收到其它回合输出".into());
                    }
                }
                RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::Disconnected { .. } => {
                    return Err("生产连接提前终止或发生未允许操作".into());
                }
            }
        }
    }
    .await;
    // 正常 Shutdown 也会退役进程能力，先保存业务终态时的租约账本。
    let audit_before_shutdown = audit.lock().expect("租约验收锁未损坏").clone();
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: Uuid::new_v4(),
            action: RuntimeAction::Shutdown,
        })
        .await;
    let joined = tokio::time::timeout(Duration::from_secs(30), &mut task).await;
    let transport_ok = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
    }
    let receipt = managed_process::confirmed_exit(&state_dir, generation);
    let cleanup = receipt
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .is_some_and(|receipt| receipt.cleanup_confirmed);
    let no_files = fs::read_dir(root.join("project"))
        .ok()
        .is_some_and(|mut entries| entries.next().is_none());
    let lease = &audit_before_shutdown;
    let verified = lease.owned_process_confirmed
        && lease.capability_confirmed
        && lease.registration_requests >= 2
        && lease.native_tool_frames >= 3
        && lease.native_initial_inputs >= 1
        && lease.native_complete_inputs >= 1
        && lease.permission_writes_allow == 1
        && lease.permission_writes_deny == 0
        && lease.business_dispatches == 1
        && lease.reply_writes == 1
        && lease.native_completions == 1
        && !lease.retired
        && lease.protocol_errors == 0
        && lease.sdk_origin_observations == 1
        && lease.full_native_sdk_origin_fields_observed == Some(false)
        && audit.lock().expect("租约验收锁未损坏").protocol_errors == 0;
    let passed = result.is_ok()
        && transport_ok
        && cleanup
        && no_files
        && verified
        && submitted == 1
        && accepted == 1
        && inspect_count == 1
        && inspect_approvals == 1
        && search_approvals <= 1
        && unexpected_tools == 0
        && final_verified;
    let failure = if passed {
        None
    } else if !transport_ok {
        Some("transport")
    } else if !cleanup {
        Some("cleanup")
    } else if result.is_err() {
        Some("fixture_loop")
    } else {
        Some("acceptance")
    };
    record(
        file,
        json!({"event":"lease_finished","scope":SCOPE,"passed":passed,
        "audit_before_shutdown":audit_before_shutdown,"failure_source":failure,"final_response_sha256":final_digest,
        "production_connect_path":true,"production_sdk_registration":true,"test_only_sdk_hook":false,
        "final_history_verified":final_verified,"authenticated_native_tool_lease_verified":verified && passed,
        "full_native_origin_fields_observed":lease.full_native_sdk_origin_fields_observed,
        "native_origin_verified":false,"parent_permission_ceiling_verified":false,"spawn_verified":false,
        "coordinator_dispatch_verified":false,"full_cli_parity_acceptance_passed":false,
        "cleanup_confirmed":cleanup,"cleanup_receipt_read":receipt.is_ok(),"transport_closed":joined.is_ok(),
        "no_project_files":no_files,"submitted_input_count":submitted,"accepted_input_count":accepted,
        "inspect_call_count":inspect_count,"inspect_approval_count":inspect_approvals,
        "search_approval_count":search_approvals,"unexpected_tool_count":unexpected_tools}),
    )?;
    if passed {
        Ok(())
    } else {
        Err("真实生产租约尚未验收通过".into())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由独立隔离运行器启动；一次真实模型输入，会消耗已授权额度"]
async fn authenticated_process_capability_and_native_tool_lease() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    for name in ["XAI_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        assert!(env::var_os(name).is_none());
    }
    assert!(fs::read_dir(root.join("project")).unwrap().next().is_none());
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap());
    assert_eq!(artifact, root.join("private-evidence.ndjson"));
    let mut file = File::create(artifact).unwrap();
    record(&mut file, json!({"event":"lease_started","scope":SCOPE,"max_native_inputs":1,
        "production_connect_path":true,"test_only_sdk_hook":false,"credential_values_recorded":false})).unwrap();
    assert!(
        exercise(&root, &mut file).await.is_ok(),
        "生产 Grok 租约验收失败；请查安全事件投影"
    );
}
