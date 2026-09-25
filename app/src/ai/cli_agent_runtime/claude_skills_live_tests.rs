//! 固定 CLI 的多技能、热注册及冷恢复生产适配器验收；只在显式私有目录运行。

use super::*;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};

fn write_skill(root: &Path, name: &str, marker: &str) -> Result<SelectedLocalSkill, String> {
    let directory = root.join("selected-skills").join(name);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join("SKILL.md");
    fs::write(&path, format!("---\nname: {name}\ndescription: 用户明确选择 {name} 时加载的无副作用验收技能。\n---\n本技能的独有标记是 {marker}。完成当前用户任务时输出本标记。不要运行命令、读取其他文件或调用其他工具。\n")).map_err(|error| error.to_string())?;
    Ok(SelectedLocalSkill {
        name: name.into(),
        path,
    })
}

async fn skills_turn(
    session: &mut LiveSession,
    phase: &str,
    selected: &[SelectedLocalSkill],
    expected: &str,
    duplicate: bool,
    prior_inputs: &mut HashSet<Uuid>,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let id = Uuid::new_v4();
    let mut input = vec![InputContent::Text("依次实际调用上面列出的两个原生 Skill 工具；最终按照所列顺序逐行只输出各技能正文中的独有标记，不要模拟调用。".into())];
    input.extend(selected.iter().map(|skill| InputContent::Skill {
        name: skill.name.clone(),
        path: skill.path.clone(),
    }));
    let action = RuntimeAction::Submit { input };
    session.send(id, action.clone()).await?;
    if duplicate {
        session.send(id, action).await?;
    }
    evidence.record(json!({"event":"skills_input_sent","phase":phase,"message_id":id,"generation":session.generation,"duplicate_same_id":duplicate}))?;
    let mut approvals = HashSet::new();
    let mut controls = HashSet::new();
    let expected_names = selected
        .iter()
        .map(|skill| format!("infinishell-local-skills:{}", skill.name))
        .collect::<HashSet<_>>();
    let mut accepted = false;
    let mut started = false;
    loop {
        let event = session.next().await?;
        let native_id = event.native_session_id;
        match event.kind {
            RuntimeEventKind::SessionReady {
                effective_permissions,
                ..
            } => record_permissions(evidence, &effective_permissions, native_id.as_deref())?,
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } if message_id == id && turn_id == Some(id.to_string()) => {
                accepted = true;
                evidence.record(json!({"event":"skills_native_ack","phase":phase,"message_id":id,"native_session_id":native_id}))?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } if prior_inputs.contains(&message_id) && turn_id == Some(message_id.to_string()) => {
                evidence.record(json!({"event":"skills_duplicate_native_ack","phase":phase,"message_id":message_id,"native_session_id":native_id}))?;
            }
            RuntimeEventKind::TurnStarted { turn_id } if turn_id == id.to_string() && accepted => {
                started = true;
                evidence.record(json!({"event":"skills_turn_started","phase":phase,"turn_id":turn_id,"native_session_id":native_id}))?;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                let name = details["input"]["skill"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let safe = started
                    && turn_id == id.to_string()
                    && method == "can_use_tool"
                    && details["tool_name"] == "Skill"
                    && expected_names.contains(&name)
                    && approvals.insert(name.clone());
                let decision = if safe {
                    ApprovalDecision::AllowOnce
                } else {
                    ApprovalDecision::DenyOnce
                };
                let control = Uuid::new_v4();
                controls.insert(control);
                session
                    .send(
                        control,
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision,
                        },
                    )
                    .await?;
                evidence.record(json!({"event":"skills_native_approval","phase":phase,"approval_id":approval_id,"tool_use_id":details["tool_use_id"],"registered_skill":name,"decision":decision,"exact_fixture":safe}))?;
                if !safe {
                    return Err("技能验收收到越界或重复审批，已拒绝".into());
                }
            }
            RuntimeEventKind::ApprovalResolved {
                decision: ApprovalDecision::AllowOnce,
                ..
            } => {}
            RuntimeEventKind::CommandDispatched { message_id, .. }
                if controls.remove(&message_id) => {}
            RuntimeEventKind::TextDelta { turn_id, .. }
            | RuntimeEventKind::Progress { turn_id, .. }
                if turn_id == id.to_string() => {}
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } if turn_id == id.to_string() => {
                evidence.record(json!({"event":"skills_turn_finished","phase":phase,"native_session_id":native_id,"turn_id":turn_id,"outcome":outcome,"output":output,"native_approval_count":approvals.len()}))?;
                if !accepted
                    || !started
                    || outcome != TurnOutcome::Completed
                    || output.trim() != expected
                    || approvals != expected_names
                {
                    return Err("技能回合缺少真实调用、审批或独有结果".into());
                }
                prior_inputs.insert(id);
                return Ok(());
            }
            RuntimeEventKind::MessageAccepted { .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::TurnStarted { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::TextDelta { .. }
            | RuntimeEventKind::Progress { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::TurnFinished { .. }
            | RuntimeEventKind::RequestFailed { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err("技能验收收到非预期生命周期事件".into());
            }
        }
    }
}

async fn close(
    session: &mut LiveSession,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<(), String> {
    session.shutdown(evidence).await?;
    let receipt = managed_process::confirmed_exit(root, session.generation)
        .map_err(|error| error.to_string())?
        .ok_or("缺少进程清理收据")?;
    evidence.record(
        json!({"event":"skills_cleanup","generation":session.generation,"receipt":receipt}),
    )?;
    if !receipt.cleanup_confirmed || receipt.exit_reason != ExitReason::StdioClosed {
        return Err("技能验收进程未正常关闭".into());
    }
    Ok(())
}

async fn exercise_skills(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let alpha = write_skill(root, "alpha", "ADAPTER_ALPHA_17c438e2")?;
    let beta = write_skill(root, "beta", "ADAPTER_BETA_8d039a25")?;
    let mut options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|error| error.to_string())?,
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: Some(env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "缺少固定模型")?),
        local_tools: None,
        selected_skills: vec![alpha.clone(), beta.clone()],
    };
    let mut prior_inputs = HashSet::new();
    let mut session = LiveSession::start(options.clone())?;
    session.ready(evidence).await?;
    skills_turn(
        &mut session,
        "multiple",
        &[alpha.clone(), beta.clone()],
        "ADAPTER_ALPHA_17c438e2\nADAPTER_BETA_8d039a25",
        true,
        &mut prior_inputs,
        evidence,
    )
    .await?;
    let gamma = write_skill(root, "gamma", "ADAPTER_GAMMA_HOT_bf6e0241")?;
    skills_turn(
        &mut session,
        "hot",
        &[gamma.clone(), alpha.clone()],
        "ADAPTER_GAMMA_HOT_bf6e0241\nADAPTER_ALPHA_17c438e2",
        false,
        &mut prior_inputs,
        evidence,
    )
    .await?;
    let native_id = session.native_id.clone().ok_or("缺少原生会话身份")?;
    close(&mut session, root, evidence).await?;
    write_skill(root, "gamma", "ADAPTER_GAMMA_COLD_e51a973b")?;
    options.selected_skills = vec![alpha.clone(), beta, gamma.clone()];
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    options.generation = Uuid::new_v4();
    let mut resumed = LiveSession::start(options)?;
    resumed.ready(evidence).await?;
    skills_turn(
        &mut resumed,
        "cold",
        &[gamma, alpha],
        "ADAPTER_GAMMA_COLD_e51a973b\nADAPTER_ALPHA_17c438e2",
        false,
        &mut prior_inputs,
        evidence,
    )
    .await?;
    close(&mut resumed, root, evidence).await?;
    evidence.record(json!({"event":"skills_acceptance_passed","native_session_id":native_id,"production_adapter":true,"native_inputs":3,"app_ui_verified":false,"sqlite_verified":false,"parent_permission_ceiling_verified":false}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要用户已授权账户、隔离配置和精确版本 CLI；消耗在线模型额度"]
async fn real_claude_multi_skill_hot_reload_and_cold_resume() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须显式提供隔离目录"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-skills-live")).unwrap(),
        "isolated Claude skills adapter verification\n"
    );
    let config = PathBuf::from(env::var_os("CLAUDE_CONFIG_DIR").expect("必须使用私有配置"))
        .canonicalize()
        .unwrap();
    assert!(config.starts_with(&root));
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION").unwrap(),
        "2.1.280"
    );
    let mut evidence = Evidence {
        file: File::create(root.join("skills-adapter-events.ndjson")).unwrap(),
        root: root.clone(),
    };
    evidence.record(json!({"event":"skills_acceptance_started","scope":"production_adapter_multiple_skills_reload_resume","credential_material_read":false,"app_ui_verified":false})).unwrap();
    let result = exercise_skills(&root, &mut evidence).await;
    assert!(result.is_ok(), "{result:?}");
}
