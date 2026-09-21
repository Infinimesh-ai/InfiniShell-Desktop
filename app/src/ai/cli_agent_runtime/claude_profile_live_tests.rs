//! 固定文件策略的真实验收；作为既有 live_tests 子模块，复用生产监督连接与证据写入。

use super::{
    ApprovalDecision, Duration, Evidence, File, HashSet, LiveSession, Path, PathBuf,
    PermissionPolicy, RuntimeAction, RuntimeError, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, Uuid, Value, connect, env, fs, json,
};
use crate::ai::cli_agent_runtime::permissions::ClaudeRestrictedFilesV1;

const MARKER: &str = "isolated Claude restricted files profile verification\n";
const BEFORE: &str = "PROFILE_BEFORE";
const AFTER: &str = "PROFILE_AFTER";

fn observe_profile(
    permissions: &Value,
    expected: Option<&ClaudeRestrictedFilesV1>,
    native_id: Option<&str>,
    evidence: &mut Evidence,
) -> Result<ClaudeRestrictedFilesV1, String> {
    let profile: ClaudeRestrictedFilesV1 =
        serde_json::from_value(permissions["claudeRestrictedFilesV1"].clone())
            .map_err(|error| format!("原生预检未返回固定配置：{error}"))?;
    if permissions["permissionMode"]
        != super::super::super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE
        || permissions["fixedProfileVerified"] != true
        || permissions["fixedProfileSha256"] != profile.digest()
        || expected.is_some_and(|expected| expected != &profile)
    {
        return Err("固定策略未验证、模式发生变化或恢复时配置被扩大".into());
    }
    evidence.record(
        json!({"event":"profile_verified", "native_session_id":native_id,
        "profile_sha256":profile.digest(),
        "permission_mode":super::super::super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE,
        "fixed_profile_verified":true,
        "filesystem_sandbox_verified":false}),
    )?;
    Ok(profile)
}

async fn ready(
    session: &mut LiveSession,
    expected: Option<&ClaudeRestrictedFilesV1>,
    evidence: &mut Evidence,
) -> Result<ClaudeRestrictedFilesV1, String> {
    let event = session.next().await?;
    match event.kind {
        RuntimeEventKind::SessionReady {
            effective_permissions,
            ..
        } => observe_profile(
            &effective_permissions,
            expected,
            event.native_session_id.as_deref(),
            evidence,
        ),
        RuntimeEventKind::Disconnected { reason } => {
            if let Some(mut task) = session.task.take() {
                match tokio::time::timeout(Duration::from_secs(10), &mut task).await {
                    Ok(Ok(Err(error))) => evidence
                        .record(json!({"event":"profile_start_failed",
                        "reason":reason, "details":error.permission_ceiling_evidence()}))?,
                    Ok(Ok(Ok(()))) | Ok(Err(_)) => evidence
                        .record(json!({"event":"profile_start_failed",
                        "reason":reason, "details":null}))?,
                    Err(_) => {
                        task.abort();
                        evidence.record(json!({"event":"profile_start_failed", "reason":reason,
                            "task_exit_timed_out":true}))?;
                    }
                }
            }
            Err(format!("固定策略初始化断开：{reason}"))
        }
        unexpected => Err(format!("固定策略初始化未就绪：{unexpected:?}")),
    }
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
}

async fn turn(
    session: &mut LiveSession,
    profile: &ClaudeRestrictedFilesV1,
    phase: &str,
    prompt: String,
    edit: Option<(&Path, ApprovalDecision)>,
    expected_output: &str,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let id = Uuid::new_v4();
    let turn_id = id.to_string();
    let mut accepted = false;
    let mut started = false;
    let mut finished = false;
    let mut approvals = HashSet::new();
    let mut resolved = HashSet::new();
    let mut controls = HashSet::new();
    let mut dispatched = HashSet::new();
    evidence.record(json!({"event":"phase_started", "phase":phase, "message_id":id}))?;
    session.submit(id, prompt).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "固定策略回合超过总时限".to_owned())??;
        let native_id = event.native_session_id;
        match event.kind {
            RuntimeEventKind::InputJoined { .. } => {
                return Err("单输入固定策略验收意外收到合并事件".into());
            }
            RuntimeEventKind::SessionReady {
                effective_permissions,
                ..
            } => {
                observe_profile(
                    &effective_permissions,
                    Some(profile),
                    native_id.as_deref(),
                    evidence,
                )?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id: actual,
            } => {
                if message_id != id || actual.as_deref() != Some(turn_id.as_str()) {
                    return Err("用户输入原生 ACK 关联错误".into());
                }
                if !accepted {
                    evidence.record(json!({"event":"message_accepted", "phase":phase,
                        "message_id":id, "turn_id":turn_id, "native_session_id":native_id}))?;
                }
                accepted = true;
            }
            RuntimeEventKind::TurnStarted { turn_id: actual } => {
                if actual != turn_id || started {
                    return Err("未知或重复执行的回合".into());
                }
                started = true;
                evidence.record(
                    json!({"event":"turn_started", "phase":phase, "turn_id":turn_id,
                    "native_session_id":native_id}),
                )?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id: actual,
                method,
                details,
            } => {
                let safe = actual == turn_id
                    && method == "can_use_tool"
                    && approvals.is_empty()
                    && edit.is_some_and(|(path, _)| exact_edit(&details, path));
                let decision = if safe {
                    edit.expect("已验证固定编辑").1
                } else {
                    ApprovalDecision::DenyOnce
                };
                let control = Uuid::new_v4();
                controls.insert(control);
                approvals.insert(approval_id.clone());
                session
                    .send(
                        control,
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision,
                        },
                    )
                    .await?;
                evidence.record(json!({"event":"approval_requested", "phase":phase,
                    "approval_id":approval_id, "decision":decision, "exact_edit_fixture":safe}))?;
                if !safe {
                    return Err("收到固定 Read/Edit 项目范围之外或重复的审批，已拒绝".into());
                }
            }
            RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id: actual,
            } => {
                if !controls.contains(&message_id) || actual.as_deref() != Some(turn_id.as_str()) {
                    return Err("审批控制发送关联错误；不能作为原生输入 ACK".into());
                }
                dispatched.insert(message_id);
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                if !approvals.contains(&approval_id)
                    || !resolved.insert(approval_id.clone())
                    || edit.map(|(_, expected)| expected) != Some(decision)
                {
                    return Err("审批结束关联错误或重复".into());
                }
                evidence.record(json!({"event":"approval_resolved", "phase":phase,
                    "approval_id":approval_id, "decision":decision, "native_receipt":false}))?;
            }
            RuntimeEventKind::TurnFinished {
                turn_id: actual,
                outcome,
                output,
            } => {
                evidence.record(json!({"event":"turn_finished", "phase":phase, "turn_id":actual,
                    "native_session_id":native_id, "outcome":outcome, "output":output.chars().take(4096).collect::<String>()}))?;
                if actual != turn_id
                    || !started
                    || finished
                    || outcome != TurnOutcome::Completed
                    || output.trim() != expected_output
                {
                    return Err("固定策略回合未返回关联的预期完成结果".into());
                }
                finished = true;
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } => return Err(format!("请求 {message_id} 失败：{message}")),
            RuntimeEventKind::Disconnected { reason } => {
                return Err(format!("固定策略连接中断：{reason}"));
            }
            RuntimeEventKind::ApprovalCancelled { .. } => return Err("审批尚未验证就被取消".into()),
            RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::LocalToolCancelled { .. } => {
                return Err("本验收未启用本地 MCP 工具".into());
            }
            RuntimeEventKind::TextDelta { .. } | RuntimeEventKind::Progress { .. } => {}
        }
        if accepted && finished && controls == dispatched && approvals == resolved {
            if approvals.len() != usize::from(edit.is_some()) {
                return Err("没有恰好一次真实 Edit 审批".into());
            }
            return Ok(());
        }
    }
}

async fn rejected_start(
    options: SessionOptions,
    phase: &str,
    reason: &str,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let result = match connect(options) {
        Err(error) => Err(error),
        Ok(mut connection) => {
            let mut task = tokio::spawn(connection.task);
            let result = tokio::time::timeout(Duration::from_secs(60), &mut task).await;
            let result = match result {
                Ok(result) => result.map_err(|error| error.to_string())?,
                Err(_) => {
                    task.abort();
                    return Err("拒绝路径预检没有按期结束".into());
                }
            };
            while let Ok(event) = connection.events.try_recv() {
                if !matches!(event.kind, RuntimeEventKind::Disconnected { .. }) {
                    return Err("应拒绝的恢复仍公布就绪或执行事件".into());
                }
            }
            result
        }
    };
    let details = result
        .as_ref()
        .err()
        .and_then(RuntimeError::permission_ceiling_evidence);
    let rejected = details.is_some_and(|details| details["reason"] == reason);
    evidence.record(
        json!({"event":"resume_rejected", "phase":phase, "rejected":rejected,
        "expected_reason":reason, "details":details, "user_input_sent":false}),
    )?;
    if !rejected {
        return Err(format!("恢复未按固定策略拒绝：{phase}"));
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
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少显式 Claude 路径")?,
        ),
        cwd: cwd.clone(),
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::ClaudeRestrictedFilesV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: env::var("INFINISHELL_CLAUDE_LIVE_MODEL").ok(),
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let mut session = LiveSession::start(options.clone())?;
    let profile = ready(&mut session, None, evidence).await?;
    let marker = format!("PROFILE_RESTORED_{}", Uuid::new_v4().simple());
    turn(
        &mut session,
        &profile,
        "first_turn",
        format!("第一轮：记住恢复标记 {marker}，不输出标记、不调用工具，只回复 PROFILE_ONE。"),
        None,
        "PROFILE_ONE",
        evidence,
    )
    .await?;
    let native_id = session
        .native_id
        .clone()
        .ok_or("首轮完成后没有原生会话 ID")?;
    turn(
        &mut session,
        &profile,
        "second_turn",
        "第二轮：不调用工具，只回复 PROFILE_TWO。".into(),
        None,
        "PROFILE_TWO",
        evidence,
    )
    .await?;
    for (label, decision, output) in [
        ("allow", ApprovalDecision::AllowOnce, "EDIT_ALLOWED"),
        ("deny", ApprovalDecision::DenyOnce, "EDIT_DENIED"),
    ] {
        let path = cwd.join(format!("edit-{label}.txt"));
        if fs::read_to_string(&path).map_err(|error| error.to_string())? != BEFORE {
            return Err("文件审批夹具不是预期初始内容".into());
        }
        let phase = format!("edit_{label}");
        turn(&mut session, &profile, &phase,
            format!("这是隔离临时目录中的文件审批测试。保持当前权限模式，先用 Read 读取 {}，然后只调用 Edit 一次，将 old_string=\"{BEFORE}\" 替换为 new_string=\"{AFTER}\"，replace_all=false。不得调用其他工具或更改其他文件。获准后只回复 EDIT_ALLOWED；被拒绝后只回复 EDIT_DENIED，不要重试。",json!(path)),
            Some((&path, decision)), output, evidence).await?;
        let expected = if decision == ApprovalDecision::AllowOnce {
            AFTER
        } else {
            BEFORE
        };
        if fs::read_to_string(&path).map_err(|error| error.to_string())? != expected {
            return Err("Edit 审批与真实文件效果不一致".into());
        }
        evidence.record(json!({"event":"file_effect_verified", "phase":phase,
            "allowed":decision == ApprovalDecision::AllowOnce, "expected_content":expected}))?;
    }
    session.shutdown(evidence).await?;
    // 配置经序列化再读回，以验证恢复使用已保存的创建时策略，而非重新选取策略。
    let saved = root.join("saved-profile.json");
    fs::write(
        &saved,
        serde_json::to_vec(&profile).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    options.claude_profile = Some(
        serde_json::from_slice(&fs::read(&saved).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?,
    );
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    options.generation = Uuid::new_v4();
    let mut resumed = LiveSession::start(options.clone())?;
    ready(&mut resumed, Some(&profile), evidence).await?;
    turn(
        &mut resumed,
        &profile,
        "resume_result",
        "不要调用工具，只回复第一轮要求记住的完整 PROFILE_RESTORED 标记。".into(),
        None,
        &marker,
        evidence,
    )
    .await?;
    if resumed.native_id.as_ref() != Some(&native_id) {
        return Err("恢复后的原生会话 ID 不同".into());
    }
    resumed.shutdown(evidence).await?;
    evidence.record(
        json!({"event":"same_profile_resume_verified", "native_session_id":native_id,
        "profile_sha256":profile.digest(), "marker":marker, "saved_profile_reloaded":true}),
    )?;

    let mut changed = options.clone();
    changed.generation = Uuid::new_v4();
    changed.cwd = root
        .join("outside")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    rejected_start(
        changed,
        "changed_directory",
        "claude_profile_identity_invalid",
        evidence,
    )
    .await?;
    let mut changed = options.clone();
    changed.generation = Uuid::new_v4();
    changed.permission_policy = PermissionPolicy::Inherit;
    rejected_start(
        changed,
        "changed_policy",
        "claude_profile_wrong_policy",
        evidence,
    )
    .await?;
    let settings = cwd.join(".claude/settings.local.json");
    let previous = fs::read(&settings).map_err(|error| error.to_string())?;
    fs::write(&settings, b"{\"permissions\":{\"allow\":[\"Edit\"]}}\n")
        .map_err(|error| error.to_string())?;
    options.generation = Uuid::new_v4();
    let rejection = rejected_start(
        options,
        "expanded_source_rules",
        "claude_profile_source_changed",
        evidence,
    )
    .await;
    fs::write(&settings, previous).map_err(|error| error.to_string())?;
    rejection?;
    evidence.record(json!({"event":"acceptance_passed", "scope":"claude_restricted_files_process_restart",
        "native_session_id":native_id, "profile_sha256":profile.digest(), "same_profile_restored":true,
        "edit_allow_and_deny_verified":true, "resume_escalation_rejections":3,
        "filesystem_sandbox_verified":false, "app_restart_and_ui_verified":false,
        "native_mid_turn_mode_change_verified":false, "outside_directory_tool_execution_verified":false}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要 run_claude_profile_live.py、固定 Claude 2.1.273 与显式 API 环境；会消耗模型额度"]
async fn real_claude_restricted_files_lifecycle() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须使用专用运行器"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-profile-probe")).unwrap(),
        MARKER
    );
    let actual = PathBuf::from(env::var_os("CLAUDE_CONFIG_DIR").unwrap())
        .canonicalize()
        .unwrap();
    let declared = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(actual, declared);
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").unwrap(),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence.record(json!({"event":"acceptance_started", "scope":"claude_restricted_files_process_restart",
        "permission_policy":"ClaudeRestrictedFilesV1", "native_credential_files_read_or_copied":false})).unwrap();
    let result = exercise(&root, &mut evidence).await;
    if let Err(error) = &result {
        evidence
            .record(json!({"event":"acceptance_failed", "reason":error}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "固定 Claude 策略未通过真实验收；请检查脱敏证据"
    );
}

#[test]
fn edit_fixture_rejects_other_tools_paths_replacements_and_extra_arguments() {
    let path = env::temp_dir().join("fixed-edit.txt");
    let valid = json!({"subtype":"can_use_tool","tool_name":"Edit","input":{
        "file_path":path,"old_string":BEFORE,"new_string":AFTER,"replace_all":false}});
    assert!(exact_edit(&valid, &path));
    for pointer in [
        "/tool_name",
        "/input/file_path",
        "/input/old_string",
        "/input/new_string",
        "/input/replace_all",
    ] {
        let mut changed = valid.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("unexpected");
        assert!(!exact_edit(&changed, &path));
    }
    let mut changed = valid;
    changed["input"]["permission_mode"] = json!("bypassPermissions");
    assert!(!exact_edit(&changed, &path));
}
