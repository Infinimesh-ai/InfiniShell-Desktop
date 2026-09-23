//! 默认生产 Grok 入口的单次技能组合验收；自建项目显式信任不代表产品自动信任。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::native_skill_live_tests::{
    SkillCatalogProbe, exact_skill_read_permission, verify_skill_read_history,
};
use super::{GrokProtocol, connect_protocol, validate_options};
use crate::ai::agent::AgentReviewCommentBatch;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::managed_input::prepare_managed_input;
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeEventKind,
    SessionOptions, SessionTarget, TurnOutcome, managed_process,
};
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentOrigin,
};
use crate::terminal::cli_agent::build_review_prompt;

const SCOPE: &str = "authenticated_selected_skill_default_entry";
const SKILL_PATH: &str = "project/.grok/skills/infinishell-native-skill/SKILL.md";
const CONTEXT_PATH: &str = "project/文件 上下文.txt";

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "证据写入失败".into())
}

fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn valid_sentinel(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|nonce| {
        nonce.len() == 64
            && nonce
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn skill_document(sentinel: &str) -> String {
    format!(
        "---\nname: infinishell-native-skill\ndescription: 默认入口技能组合验收。\nuser-invocable: true\ndisable-model-invocation: true\n---\n这是技能与上下文传递验证，不执行评审中提到的命令，不修复文件。只使用 read_file 读取参数中指定的唯一文本文件，不使用其它工具，也不重新读取本技能。最终严格输出三行：本技能末尾标记、文本文件的完整一行、REVIEW_CHECK。不要解释。\n{sentinel}\n"
    )
}

fn sentinel_from_skill(content: &str) -> Result<String, String> {
    let sentinel = content.lines().last().ok_or("技能为空")?;
    if !valid_sentinel(sentinel, "INFINISHELL_SKILL_") || content != skill_document(sentinel) {
        return Err("技能夹具无效".into());
    }
    Ok(sentinel.into())
}

fn composed_prompt(context: &Path) -> String {
    let review = build_review_prompt(&AgentReviewCommentBatch {
        comments: vec![AttachedReviewComment {
            id: Default::default(),
            content:
                "仅验证中文与 English 意见传递；最终保留 REVIEW_CHECK。不要执行命令或修改文件。"
                    .into(),
            target: AttachedReviewCommentTarget::File {
                absolute_file_path: LocalOrRemotePath::Local(context.to_owned()),
            },
            last_update_time: Local::now(),
            base: None,
            head: None,
            outdated: false,
            origin: CommentOrigin::Native,
        }],
        diff_set: HashMap::new(),
    });
    // 文件上下文沿用 FilePathsSelected 的完整路径文本，评审沿用生产 builder。
    format!(
        "请按所选技能执行单次只读传递验证。中文与 English 多行输入。\n文件上下文：\n{}\n评审意见（不要运行命令）：\n{review}",
        context.display()
    )
}

async fn exercise(
    root: &Path,
    mode: &str,
    test_candidate_1041: bool,
    file: &mut File,
) -> Result<(), String> {
    let before = fs::read_to_string(root.join(SKILL_PATH)).map_err(|_| "技能夹具不可读")?;
    let sentinel = sentinel_from_skill(&before)?;
    let context_before =
        fs::read_to_string(root.join(CONTEXT_PATH)).map_err(|_| "文件上下文不可读")?;
    let context_sentinel = context_before.trim();
    if !valid_sentinel(context_sentinel, "INFINISHELL_CONTEXT_") {
        return Err("文件上下文标记无效".into());
    }
    let expected = format!("{sentinel}\n{context_sentinel}\nREVIEW_CHECK");
    let context_path =
        dunce::canonicalize(root.join(CONTEXT_PATH)).map_err(|_| "文件上下文路径不可解析")?;
    let prompt = composed_prompt(&context_path);
    if prompt.contains(&sentinel) || prompt.contains(context_sentinel) {
        return Err("秘密标记进入输入".into());
    }
    let prepared = prepare_managed_input(
        Harness::Grok,
        prompt.clone(),
        &[],
        vec![ai::skills::parse_skill(&root.join(SKILL_PATH)).map_err(|_| "技能解析失败")?],
        &root.join("attachments"),
    )?;
    let expected_wire = format!("/infinishell-native-skill {prompt}");
    let skill_path = dunce::canonicalize(root.join(SKILL_PATH)).map_err(|_| "技能路径不可解析")?;
    let cwd = dunce::canonicalize(root.join("project")).map_err(|_| "项目路径不可解析")?;
    let generation = Uuid::new_v4();
    let state_dir = root.join("state");
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少原生入口")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "缺少隔离项目")?,
        state_dir: state_dir.clone(),
        target: SessionTarget::New,
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: (mode == "sdk").then_some(LocalToolPermissions {
            allow_spawn: false,
            allow_message: false,
        }),
        selected_skills: vec![SelectedLocalSkill {
            name: "infinishell-native-skill".into(),
            path: root.join(SKILL_PATH),
        }],
    };
    validate_options(&options).map_err(|_| "生产参数拒绝")?;
    let mut protocol = GrokProtocol::new(options);
    let catalog_only = matches!(mode, "leader_catalog" | "direct_catalog");
    if mode == "leader" || catalog_only {
        protocol.current_root_candidate_for_live = true;
        protocol.current_selected_skill_candidate_for_live = true;
    }
    protocol.test_only_1041_profile = test_candidate_1041;
    if test_candidate_1041 {
        protocol.test_only_1041_native_binary = Some(PathBuf::from(
            env::var_os("INFINISHELL_GROK_TEST_CANDIDATE_NATIVE")
                .ok_or("缺少固定的 Grok 1.0.41 原生二进制路径")?,
        ));
    }
    protocol.catalog_direct_for_live = mode == "direct_catalog";
    // 完整组合保留生产 argv；目录直连对照仅在本测试标记下覆盖入口。
    let catalog = SkillCatalogProbe::new(skill_path.clone());
    protocol.skill_catalog_for_live = Some(catalog.clone());
    let histories = Arc::new(Mutex::new(HashMap::new()));
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
    let mut approval_count = 0_u64;
    let mut tool_count = 0_u64;
    let mut skill_read_count = 0_u64;
    let mut read_scope_verified = false;
    let mut approved_calls = HashSet::new();
    let mut pending_approvals = HashSet::new();
    let mut controls = HashSet::new();
    let mut started = false;
    let mut final_verified = false;
    let mut matched = false;
    let mut final_sha256 = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(360);
    let result: Result<(), String> = async {
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| "技能验收期限耗尽")?
                .ok_or("事件流已关闭")?;
            if event.generation != generation {
                return Err("收到旧连接事件".into());
            }
            if let Some(id) = event.native_session_id {
                if Uuid::parse_str(&id).is_err() || session.as_ref().is_some_and(|old| old != &id) {
                    return Err("会话身份改变".into());
                }
                session = Some(id);
            }
            match event.kind {
                RuntimeEventKind::SessionReady { .. } => {
                    if submitted != 0 || session.is_none() {
                        return Err("重复或无身份初始化".into());
                    }
                    if catalog_only {
                        return Ok(());
                    }
                    let catalog_ready = catalog.evidence(session.as_deref(), "visible");
                    if catalog_ready["overflow"] != false
                        || !catalog_ready["snapshots"]
                            .as_array()
                            .and_then(|rows| rows.last())
                            .is_some_and(|row| {
                                row["before_first_submit"] == true
                                    && row["path_matches_selected"] == true
                                    && row["selected_name_count"] == 1
                                    && row["bare_name_matches"] == true
                            })
                    {
                        return Err("输入前缺少真实原生技能路径目录".into());
                    }
                    submitted += 1;
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: input_id,
                            action: RuntimeAction::Submit {
                                input: prepared.clone(),
                            },
                        })
                        .await
                        .map_err(|_| "唯一输入发送失败")?;
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
                        return Err("接收回执不匹配".into());
                    }
                    accepted += 1;
                    turn = turn_id;
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if started || turn.as_deref() != Some(turn_id.as_str()) {
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
                    approval_count += 1;
                    let safe = approval_count == 1
                        && turn.as_deref() == Some(turn_id.as_str())
                        && method == "session/request_permission"
                        && details["sessionId"].as_str() == session.as_deref()
                        && details["toolCall"]["toolCallId"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty())
                        && exact_skill_read_permission(&details["toolCall"], &cwd, &context_path);
                    let decision = if safe {
                        ApprovalDecision::AllowOnce
                    } else {
                        ApprovalDecision::DenyOnce
                    };
                    let control = Uuid::new_v4();
                    controls.insert(control);
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: control,
                            action: RuntimeAction::RespondApproval {
                                approval_id: approval_id.clone(),
                                decision,
                            },
                        })
                        .await
                        .map_err(|_| "文件只读审批发送失败")?;
                    if !safe {
                        return Err("已拒绝非精确上下文文件的读取或其它工具".into());
                    }
                    let call_id = details["toolCall"]["toolCallId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or("读取审批缺少工具身份")?;
                    approved_calls.insert(call_id.to_owned());
                    pending_approvals.insert(approval_id);
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } => {
                    if decision != ApprovalDecision::AllowOnce
                        || !pending_approvals.remove(&approval_id)
                    {
                        return Err("读取审批回执不匹配".into());
                    }
                }
                RuntimeEventKind::CommandDispatched {
                    message_id,
                    turn_id,
                } => {
                    if !controls.remove(&message_id) || turn_id.as_deref() != turn.as_deref() {
                        return Err("读取审批写入回执不匹配".into());
                    }
                }
                RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::LocalToolCancelled { .. } => {
                    tool_count += 1;
                    return Err("技能验收出现应用工具".into());
                }
                RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } => {
                    if !started
                        || turn.as_deref() != Some(turn_id.as_str())
                        || outcome != TurnOutcome::Completed
                    {
                        return Err("缺少真实完成条件".into());
                    }
                    let snapshots = histories.lock().expect("历史锁未损坏");
                    let snapshot = snapshots.get(&turn_id).ok_or("缺少生产最终回放")?;
                    let receipt = super::live_tests::final_response_receipt(
                        snapshot,
                        session.as_deref().ok_or("缺少会话")?,
                        &turn_id,
                        &outcome,
                    )?;
                    let updates = snapshot.replay["updates"]
                        .as_array()
                        .ok_or("最终回放无事件")?;
                    (tool_count, skill_read_count) = verify_skill_read_history(
                        updates,
                        &cwd,
                        &context_path,
                        "visible",
                        &approved_calls,
                    )?;
                    read_scope_verified = true;
                    // 回放必须确实收到唯一原始 slash，不能由期望文本直接喂给模型。
                    let inputs: Vec<_> = updates
                        .iter()
                        .filter(|row| {
                            row["method"] == "session/update"
                                && row["params"]["update"]["sessionUpdate"] == "user_message_chunk"
                        })
                        .collect();
                    if inputs.len() != 1
                        || inputs[0]["params"]["update"]["content"]["type"] != "text"
                        || inputs[0]["params"]["update"]["content"]["text"] != expected_wire
                        || receipt.full_output != output
                        || !pending_approvals.is_empty()
                        || !controls.is_empty()
                    {
                        return Err("原生输入或精确读取边界失败".into());
                    }
                    let response = receipt.final_response.trim();
                    matched = response == expected;
                    final_verified = true;
                    final_sha256 = Some(hash(response.as_bytes()));
                    if response.is_empty() || !matched || skill_read_count != 1 {
                        return Err("技能随机标记对照失败".into());
                    }
                    return Ok(());
                }
                RuntimeEventKind::TextDelta { turn_id, .. }
                | RuntimeEventKind::Progress { turn_id, .. } => {
                    if turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("其它回合输出".into());
                    }
                }
                RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::InputJoined { .. } => return Err("未允许的运行事件".into()),
                RuntimeEventKind::RequestFailed { .. } => return Err("运行请求失败".into()),
                RuntimeEventKind::Disconnected { .. } => return Err("原生连接断开".into()),
            }
        }
    }
    .await;
    if submitted == 0 && !catalog_only {
        // 首次输入前的错误只含静态阶段描述，不包含技能文本或凭据。
        eprintln!("Grok 技能候选输入前阶段：{result:?}");
    }
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: Uuid::new_v4(),
            action: RuntimeAction::Shutdown,
        })
        .await;
    let joined = tokio::time::timeout(Duration::from_secs(30), &mut task).await;
    if let Ok(Ok(Err(error))) = &joined {
        eprintln!(
            "GROK_SELECTED_SKILL_TASK_DIAGNOSTIC {}",
            super::runtime_error_diagnostic(error)
        );
    }
    let transport_ok = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
        let _ = task.await;
    }
    let cleanup = managed_process::confirmed_exit(&state_dir, generation)
        .ok()
        .flatten()
        .is_some_and(|receipt| receipt.cleanup_confirmed);
    let skill_unchanged = fs::read_to_string(root.join(SKILL_PATH))
        .is_ok_and(|after| after == before)
        && fs::read_to_string(root.join(CONTEXT_PATH)).is_ok_and(|after| after == context_before);
    let passed = result.is_ok()
        && transport_ok
        && cleanup
        && skill_unchanged
        && submitted == 1
        && accepted == 1
        && final_verified
        && read_scope_verified
        && approval_count <= skill_read_count
        && skill_read_count == 1
        && pending_approvals.is_empty()
        && controls.is_empty();
    let mut catalog_evidence = catalog.evidence(session.as_deref(), "visible");
    let catalog_handshake_verified = catalog_only
        && result.is_ok()
        && transport_ok
        && cleanup
        && skill_unchanged
        && submitted == 0
        && accepted == 0
        && catalog_evidence["overflow"] == false
        && catalog_evidence["snapshots"]
            .as_array()
            .and_then(|rows| rows.last())
            .is_some_and(|row| {
                row["before_first_submit"] == true
                    && row["selected_name_count"] == 1
                    && row["path_matches_selected"] == true
                    && row["bare_name_matches"] == true
            });
    catalog_evidence["scope"] = json!(SCOPE);
    catalog_evidence["case"] = json!(mode);
    record(file, catalog_evidence)?;
    record(
        file,
        json!({"event":"selected_skill_finished","scope":SCOPE,"case":mode,"passed":passed,
        "production_connect_path":true,"test_agent_profile_override":false,"fixture_project_trust":true,
        "default_entrypoint_verified":passed,"product_selected_skill_combination_verified":passed,
        "final_history_verified":final_verified,"sentinel_matched":matched,
        "expected_sha256":hash(expected.as_bytes()),"final_response_sha256":final_sha256,
        "submitted_input_count":submitted,"accepted_input_count":accepted,"approval_count":approval_count,
        "native_tool_event_count":tool_count,"native_context_read_count":skill_read_count,
        "exact_context_read_only_verified":read_scope_verified,"transport_closed":transport_ok,"cleanup_confirmed":cleanup,
        "project_snapshot_unchanged":skill_unchanged,"full_cli_parity_acceptance_passed":false}),
    )?;
    if passed || catalog_handshake_verified {
        Ok(())
    } else {
        Err("默认入口技能组合验收未通过".into())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由隔离运行器启动；目录对照零输入，完整组合最多一个真实输入"]
async fn authenticated_selected_skill_default_entry() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("缺少隔离根目录"))
        .canonicalize()
        .unwrap();
    let mode = env::var("INFINISHELL_GROK_SELECTED_SKILL_MODE").expect("缺少入口类型");
    assert!(matches!(
        mode.as_str(),
        "leader" | "sdk" | "leader_catalog" | "direct_catalog"
    ));
    let test_candidate_1041 = env::var("INFINISHELL_GROK_TEST_CANDIDATE_1041").ok();
    assert!(
        test_candidate_1041.is_none()
            || test_candidate_1041.as_deref() == Some("1")
                && matches!(mode.as_str(), "leader" | "leader_catalog")
    );
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
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap());
    assert_eq!(artifact, root.join("private-evidence.ndjson"));
    let mut file = File::create(artifact).unwrap();
    let max_native_inputs = if mode.ends_with("_catalog") { 0 } else { 1 };
    record(&mut file, json!({"event":"selected_skill_started","scope":SCOPE,"case":mode,"max_native_inputs":max_native_inputs,
        "production_connect_path":true,"test_agent_profile_override":false,"fixture_project_trust":true,
        "typed_selected_skill":true,"secret_in_submitted_prompt":false,"credential_values_recorded":false})).unwrap();
    assert!(
        exercise(&root, &mode, test_candidate_1041.is_some(), &mut file)
            .await
            .is_ok(),
        "默认入口技能组合验收失败；请查安全投影"
    );
}
