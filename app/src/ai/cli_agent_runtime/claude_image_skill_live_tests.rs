//! 精确选定单技能与图片同轮的生产适配器验收；不属于默认离线测试。

use ai::skills::parse_skill;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use warp_cli::agent::Harness;

use super::super::{encode_input, InputProjection};
use super::image_probe_live_tests::quadrant_png;
use super::{
    env, fs, json, record_permissions, ApprovalDecision, Duration, Evidence, File, InputContent,
    LiveSession, Path, PathBuf, PermissionPolicy, RuntimeAction, RuntimeEventKind, SessionOptions,
    SessionTarget, TurnOutcome, Uuid,
};
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::local_skills::{prepare_claude_skill_plugin, SelectedLocalSkill};
use crate::ai::cli_agent_runtime::managed_input::{
    prepare_managed_input, restore_claude_managed_images,
};
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};

const SKILL_NAME: &str = "inspect-managed-image";
const SKILL_COMMAND: &str = "infinishell-local-skills:inspect-managed-image";
const PROMPT: &str = "请针对这次附件实际调用所选 Skill，再按技能规定回答。即使历史中用过该技能，也须重新调用一次；不得调用其他工具，不得读取文件或修改任何内容。";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn prepared_input(
    root: &Path,
    selected: &SelectedLocalSkill,
    seed: Uuid,
    resumed: bool,
    evidence: &mut Evidence,
) -> Result<(Vec<InputContent>, String), String> {
    let (bytes, colors) = quadrant_png(seed);
    let store = root.join("local-cli-attachments");
    let skill = parse_skill(&selected.path).map_err(|_| "所选技能解析失败")?;
    let input = prepare_managed_input(
        Harness::Claude,
        PROMPT.into(),
        &[ImageContext {
            data: STANDARD.encode(&bytes),
            mime_type: "image/png".into(),
            file_name: "本次图片.png".into(),
            is_figma: false,
        }],
        vec![skill],
        &store,
    )?;
    let image_path = input
        .iter()
        .find_map(|part| match part {
            InputContent::LocalImage(path) => Some(path.clone()),
            InputContent::Text(_) | InputContent::Skill { .. } => None,
        })
        .ok_or("缺少持久图片")?;
    let checkpoint = root.join(if resumed {
        "resume-input.json"
    } else {
        "new-input.json"
    });
    fs::write(
        &checkpoint,
        serde_json::to_vec(&input).map_err(|_| "引用编码失败")?,
    )
    .map_err(|_| "引用保存失败")?;
    let restored: Vec<InputContent> =
        serde_json::from_slice(&fs::read(&checkpoint).map_err(|_| "引用读取失败")?)
            .map_err(|_| "引用恢复失败")?;
    let images = restore_claude_managed_images(vec![image_path], &store)?;
    if restored != input || images.len() != 1 || images[0].data != STANDARD.encode(&bytes) {
        return Err("持久图片或技能引用发生变化".into());
    }
    let plugin = prepare_claude_skill_plugin(std::slice::from_ref(selected))?;
    let InputProjection::Blocks { content, .. } =
        encode_input(restored.clone(), plugin.as_ref(), &store)?
    else {
        return Err("缺少生产图加技能数组".into());
    };
    evidence.record(json!({"event":"attachment_prepared","resumed":resumed,
        "image_sha256":digest(&bytes),"image_bytes":bytes.len(),"media_type":"image/png",
        "native_array_sha256":digest(&serde_json::to_vec(&content).map_err(|_| "数组编码失败")?),
        "skill_command":SKILL_COMMAND,"selected_skill_path_sha256":digest(selected.path.to_string_lossy().as_bytes()),
        "persistent_reference_restored":true}))?;
    Ok((restored, colors))
}

async fn image_skill_turn(
    session: &mut LiveSession,
    input: Vec<InputContent>,
    expected: &str,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let id = Uuid::new_v4();
    let turn = id.to_string();
    let mut accepted = false;
    let mut started = false;
    let mut approval = None;
    let mut response_id = None;
    let mut resolved = false;
    let mut dispatched = false;
    evidence.record(
        json!({"event":"input_submitted","generation":session.generation,
        "message_id":id,"expected_reply_sha256":digest(expected.as_bytes())}),
    )?;
    session.send(id, RuntimeAction::Submit { input }).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "图片技能回合超时")??;
        let native = event.native_session_id;
        match event.kind {
            RuntimeEventKind::SessionReady {
                effective_permissions,
                ..
            } => {
                record_permissions(evidence, &effective_permissions, native.as_deref())?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                if accepted
                    || message_id != id
                    || turn_id.as_deref() != Some(&turn)
                    || native.is_none()
                {
                    return Err("图片技能原生 ACK 重复或身份不匹配".into());
                }
                accepted = true;
                evidence.record(json!({"event":"accepted","generation":session.generation,
                    "message_id":id,"turn_id":turn,"native_session_id":native}))?;
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                if !accepted || started || turn_id != turn {
                    return Err("图片技能回合启动身份不匹配".into());
                }
                started = true;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                let exact = started
                    && approval.is_none()
                    && turn_id == turn
                    && method == "can_use_tool"
                    && details["tool_name"] == "Skill"
                    && details["input"] == json!({"skill":SKILL_COMMAND})
                    && details["tool_use_id"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty());
                let decision = if exact {
                    ApprovalDecision::AllowOnce
                } else {
                    ApprovalDecision::DenyOnce
                };
                let control = Uuid::new_v4();
                session
                    .send(
                        control,
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision,
                        },
                    )
                    .await?;
                evidence.record(json!({"event":"skill_approval","generation":session.generation,
                    "native_session_id":native,"turn_id":turn_id,"approval_id":approval_id,
                    "tool_use_id":if exact { details["tool_use_id"].clone() } else { json!(null) },
                    "decision":decision,"exact_registered_command":exact,
                    "input_sha256":if exact { Some(digest(&serde_json::to_vec(&details["input"]).map_err(|_| "工具输入编码失败")?)) } else { None }}))?;
                if !exact {
                    return Err("非精确所选 Skill 请求已拒绝".into());
                }
                approval = Some(approval_id);
                response_id = Some(control);
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                if resolved
                    || approval.as_ref() != Some(&approval_id)
                    || decision != ApprovalDecision::AllowOnce
                {
                    return Err("Skill 审批完成身份不匹配".into());
                }
                resolved = true;
            }
            RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id,
            } => {
                if dispatched
                    || response_id != Some(message_id)
                    || turn_id.as_deref() != Some(&turn)
                {
                    return Err("Skill 审批响应未关联当前回合".into());
                }
                dispatched = true;
            }
            RuntimeEventKind::TextDelta { turn_id, .. }
            | RuntimeEventKind::Progress { turn_id, .. } => {
                if turn_id != turn {
                    return Err("图片技能输出串入其他回合".into());
                }
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                evidence.record(json!({"event":"finished","generation":session.generation,
                    "native_session_id":native,"turn_id":turn_id,"outcome":outcome,
                    "result":output,"expected":expected,"skill_approval_resolved":resolved,
                    "skill_response_dispatched":dispatched}))?;
                if !accepted
                    || !started
                    || !resolved
                    || !dispatched
                    || turn_id != turn
                    || outcome != TurnOutcome::Completed
                    || output.trim() != expected
                {
                    return Err("图片技能实际执行、识图或终态不匹配".into());
                }
                return Ok(());
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                session
                    .send(
                        Uuid::new_v4(),
                        RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id,
                            call_id: request.call_id,
                            result: Err("图片技能校准不允许其他工具".into()),
                        },
                    )
                    .await?;
                return Err("额外本地工具请求已拒绝".into());
            }
            RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::RequestFailed { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err("图片技能收到未预期运行时事件".into())
            }
        }
    }
}

async fn close(
    session: &mut LiveSession,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let generation = session.generation;
    session.shutdown(evidence).await?;
    let receipt = managed_process::confirmed_exit(root, generation)
        .map_err(|_| "监督者退出收据无效")?
        .ok_or("缺少监督者退出收据")?;
    let natural = receipt.cleanup_confirmed
        && receipt.exit_reason == ExitReason::StdioClosed
        && receipt.exit_code == Some(0);
    evidence.record(
        json!({"event":"cleanup","generation":generation,"native_session_id":session.native_id,
        "cleanup_confirmed":receipt.cleanup_confirmed,"exit_code":receipt.exit_code,
        "exit_reason":receipt.exit_reason,"natural_exit":natural}),
    )?;
    if !natural {
        return Err("图片技能进程未自然退出并清理".into());
    }
    Ok(())
}

async fn exercise(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let skill_path = root.join("selected-skill/SKILL.md");
    let prefix = format!("IMAGE_SKILL_{}", Uuid::new_v4().simple());
    fs::create_dir(root.join("selected-skill")).map_err(|_| "技能目录创建失败")?;
    let body = format!("---\nname: {SKILL_NAME}\ndescription: Inspect this turn's attached image without tools\n---\nInspect only the image in the current user input. Never use another tool. Reply with exactly this prefix: {prefix}, followed by one space and four uppercase color names in top-left, top-right, bottom-left, bottom-right order. Use RED, GREEN, BLUE, YELLOW, without punctuation or explanation.\n");
    fs::write(&skill_path, &body).map_err(|_| "技能文件保存失败")?;
    let selected = SelectedLocalSkill {
        name: SKILL_NAME.into(),
        path: skill_path,
    };
    let mut options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "缺少临时项目")?,
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: Some("claude-opus-5-5".into()),
        local_tools: None,
        selected_skills: vec![selected.clone()],
    };
    let seed = Uuid::new_v4();
    let mut next_seed = *seed.as_bytes();
    next_seed[3] ^= 1;
    let seeds = [seed, Uuid::from_bytes(next_seed)];
    let mut first_colors = None;
    let mut native_id = None;
    for (index, seed) in seeds.into_iter().enumerate() {
        let resumed = index == 1;
        let (input, colors) = prepared_input(root, &selected, seed, resumed, evidence)?;
        if first_colors.as_ref() == Some(&colors) {
            return Err("恢复图片必须与第一图不同".into());
        }
        first_colors = Some(colors.clone());
        let mut session = LiveSession::start(options.clone())?;
        session.ready(evidence).await?;
        evidence.record(json!({"event":"ready","generation":session.generation,"resumed":resumed,
            "requested_native_session_id":native_id,"cli_version":"2.1.280","model":"claude-opus-5-5",
            "permission_policy":"inherit","selected_skill_command":SKILL_COMMAND,
            "selected_skill_body_sha256":digest(body.as_bytes())}))?;
        image_skill_turn(&mut session, input, &format!("{prefix} {colors}"), evidence).await?;
        let current = session.native_id.clone().ok_or("图片技能缺少原生会话 ID")?;
        if native_id
            .as_ref()
            .is_some_and(|previous| previous != &current)
        {
            return Err("图片技能冷恢复更换了原生会话".into());
        }
        native_id = Some(current.clone());
        close(&mut session, root, evidence).await?;
        options.generation = Uuid::new_v4();
        options.target = SessionTarget::Resume {
            native_session_id: current,
        };
    }
    evidence.record(
        json!({"event":"acceptance_passed","native_session_id":native_id,
        "production_adapter":true,"native_inputs":2,"runtime_generations":2,
        "app_restart_or_gui_verified":false,"parent_permission_ceiling_verified":false}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要固定已认证 Claude 与真实监督者；两次图片加所选技能输入及逐次审批"]
async fn real_claude_managed_image_skill_resume() {
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION").unwrap(),
        "2.1.280"
    );
    let root = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-image-skill-probe")).unwrap(),
        "isolated production Claude image and selected skill verification\n"
    );
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").unwrap(),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence.record(json!({"event":"acceptance_started","scope":"production_image_selected_skill_new_and_resume",
        "max_native_inputs":2,"credential_files_read_by_probe":false,"candidate_flags_used":false})).unwrap();
    let result =
        tokio::time::timeout(Duration::from_secs(480), exercise(&root, &mut evidence)).await;
    if let Ok(Err(reason)) = &result {
        evidence
            .record(json!({"event":"acceptance_failed","reason":reason}))
            .unwrap();
    }
    assert!(
        matches!(result, Ok(Ok(()))),
        "图片加技能的生产适配器验收失败，保留原始结果"
    );
}
