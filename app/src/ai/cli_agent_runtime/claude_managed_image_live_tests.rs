//! 生产 Claude PNG 路径的隔离验收准备；必须由专用运行器显式启动。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest as _, Sha256};
use warp_cli::agent::Harness;

use super::super::{InputProjection, encode_input};
use super::image_probe_live_tests::quadrant_png;
use super::{
    ApprovalDecision, Duration, Evidence, File, InputContent, LiveSession, Path, PathBuf,
    PermissionPolicy, RuntimeAction, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
    Uuid, env, fs, json, record_permissions,
};
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::managed_input::{prepare_managed_input, restore_managed_images};
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};

const SCOPE: &str = "claude_managed_png_process_resume";
const IMAGE_PROMPT: &str = "Inspect only the attached image. Identify the solid color of each quadrant in this order: top-left, top-right, bottom-left, bottom-right. Reply with exactly four uppercase color names separated by single spaces. Use the names RED, GREEN, BLUE, YELLOW. Do not use tools, read files, explain, or add punctuation.";
const TEXT_PROMPT: &str =
    "第二轮中文多行输入。\n不要调用工具或修改文件。\n只回复 CLAUDE_PNG_TEXT。";
const RECALL_PROMPT: &str = "Do not use tools, read files, or request another image. Recall the earlier attached image and reply with its quadrant colors in this order: top-left, top-right, bottom-left, bottom-right. Reply with exactly four uppercase color names separated by single spaces, without explanation or punctuation.";
const TEXT_REPLY: &str = "CLAUDE_PNG_TEXT";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct CachedNativeAck {
    generation: Uuid,
    message_id: Uuid,
    native_session_id: String,
    replayed: bool,
}

async fn turn(
    session: &mut LiveSession,
    phase: &str,
    input: Vec<InputContent>,
    expected: &str,
    duplicate: bool,
    cached_ack: &mut Option<CachedNativeAck>,
    evidence: &mut Evidence,
) -> Result<(), String> {
    if duplicate && (phase != "image" || cached_ack.is_some()) {
        return Err("duplicate_submit_scope_invalid".into());
    }
    let id = Uuid::new_v4();
    evidence.record(json!({"event":"input_submitted","phase":phase,
        "generation":session.generation,"message_id":id,"typed_input":true,
        "controller_send_count":if duplicate {2} else {1},
        "identical_message_retransmitted":duplicate,"expected_reply_sha256":sha(expected.as_bytes())}))?;
    let action = RuntimeAction::Submit { input };
    session
        .send(id, action.clone())
        .await
        .map_err(|_| "submit_failed")?;
    if duplicate {
        // 相同UUID和完全相同的类型化输入；不生成另一条提示词或另一个消息ID。
        session
            .send(id, action)
            .await
            .map_err(|_| "duplicate_submit_failed")?;
    }
    let mut accepted = false;
    let mut started = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "turn_timeout")?
            .map_err(|_| "runtime_identity_or_stream_failed")?;
        let native = event.native_session_id;
        match event.kind {
            RuntimeEventKind::SessionReady {
                effective_permissions,
            } => {
                record_permissions(evidence, &effective_permissions, native.as_deref())
                    .map_err(|_| "permission_mode_changed")?;
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                // 重投可能晚于原生ACK处理；缓存回执也可能在下一轮读取，不能视为新输入。
                if let Some(cached) = cached_ack.as_mut()
                    && message_id == cached.message_id
                {
                    if cached.replayed
                        || session.generation != cached.generation
                        || native.as_deref() != Some(cached.native_session_id.as_str())
                        || turn_id.as_deref() != Some(cached.message_id.to_string().as_str())
                        || !matches!(phase, "image" | "multiline")
                    {
                        return Err("wrong_or_repeated_cached_native_ack".into());
                    }
                    cached.replayed = true;
                    evidence.record(json!({"event":"cached_native_ack_replayed","phase":"image",
                        "observed_phase":phase,"generation":session.generation,"message_id":message_id,
                        "turn_id":turn_id,"native_session_id":native,
                        "receipt_source":"CachedNativeProtocol","native_input_added":false}))?;
                    continue;
                }
                if accepted
                    || native.is_none()
                    || message_id != id
                    || turn_id.as_deref() != Some(id.to_string().as_str())
                {
                    return Err("wrong_or_duplicate_native_ack".into());
                }
                accepted = true;
                if duplicate {
                    *cached_ack = Some(CachedNativeAck {
                        generation: session.generation,
                        message_id: id,
                        native_session_id: native.clone().ok_or("native_ack_identity_missing")?,
                        replayed: false,
                    });
                }
                evidence.record(json!({"event":"message_accepted","phase":phase,
                    "generation":session.generation,"message_id":id,"turn_id":turn_id,
                    "native_session_id":native,"receipt_source":"NativeProtocol"}))?;
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                if !accepted || started || turn_id != id.to_string() {
                    return Err("wrong_or_duplicate_started".into());
                }
                started = true;
                evidence.record(json!({"event":"turn_started","phase":phase,
                    "generation":session.generation,"turn_id":turn_id,"native_session_id":native}))?;
            }
            RuntimeEventKind::TextDelta { turn_id, .. }
            | RuntimeEventKind::Progress { turn_id, .. } => {
                if !started || turn_id != id.to_string() {
                    return Err("old_or_unstarted_output".into());
                }
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                if !accepted
                    || !started
                    || turn_id != id.to_string()
                    || outcome != TurnOutcome::Completed
                    || output.trim() != expected
                {
                    return Err("completion_or_exact_answer_failed".into());
                }
                evidence.record(json!({"event":"turn_finished","phase":phase,
                    "generation":session.generation,"turn_id":turn_id,"native_session_id":native,
                    "outcome":"Completed","full_output_sha256":sha(output.as_bytes()),
                    "trimmed_output_sha256":sha(output.trim().as_bytes()),"output_bytes":output.len()}))?;
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
                    .await
                    .map_err(|_| "unexpected_approval_denial_failed")?;
                return Err("unexpected_approval_denied".into());
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                session
                    .send(
                        Uuid::new_v4(),
                        RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id,
                            call_id: request.call_id,
                            result: Err("此校准不允许工具调用".into()),
                        },
                    )
                    .await
                    .map_err(|_| "unexpected_tool_denial_failed")?;
                return Err("unexpected_tool_denied".into());
            }
            RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::RequestFailed { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err("unexpected_runtime_event".into());
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
    session
        .shutdown(evidence)
        .await
        .map_err(|_| "normal_shutdown_failed")?;
    let receipt = managed_process::confirmed_exit(root, generation)
        .map_err(|_| "exit_receipt_invalid")?
        .ok_or("exit_receipt_missing")?;
    let normal = receipt.version == 1
        && receipt.generation == generation
        && receipt.cleanup_confirmed
        && receipt.exit_reason == ExitReason::StdioClosed
        && receipt.exit_code == Some(0);
    evidence.record(json!({"event":"cleanup_checked","generation":generation,
        "native_session_id":session.native_id,"transport_closed":true,
        "cleanup_receipt_read":true,"cleanup_confirmed":receipt.cleanup_confirmed,
        "normal_exit":normal,"receipt":{"version":receipt.version,"generation":receipt.generation,
            "cleanup_confirmed":receipt.cleanup_confirmed,"containment":receipt.containment,
            "exit_reason":receipt.exit_reason,"exit_code":receipt.exit_code}}))?;
    if !normal {
        return Err("normal_exit_not_confirmed".into());
    }
    Ok(())
}

async fn exercise(root: &Path, evidence: &mut Evidence) -> Result<(), String> {
    let (png, expected) = quadrant_png(Uuid::new_v4());
    let image_sha = sha(&png);
    let store = root.join("local-cli-attachments");
    let image = ImageContext {
        data: STANDARD.encode(&png),
        mime_type: "image/png".into(),
        file_name: "managed-quadrants.png".into(),
        is_figma: false,
    };
    let input = prepare_managed_input(
        Harness::Claude,
        IMAGE_PROMPT.into(),
        &[image],
        Vec::new(),
        &store,
    )
    .map_err(|_| "production_png_preparation_failed")?;
    if input.len() != 2 || input[0] != InputContent::Text(IMAGE_PROMPT.into()) {
        return Err("prepared_input_shape_changed".into());
    }
    let InputContent::LocalImage(path) = &input[1] else {
        return Err("production_local_image_missing".into());
    };
    if path.parent() != Some(store.as_path())
        || sha(&fs::read(path).map_err(|_| "stored_png_missing")?) != image_sha
    {
        return Err("stored_png_hash_failed".into());
    }
    let saved_path = path.clone();
    // 使用生产编码器计算期望数组SHA，不另造出站帧或提前提交图片。
    let InputProjection::Blocks { content, .. } = encode_input(input.clone(), None, &store)
        .map_err(|_| "production_image_encoding_failed")?
    else {
        return Err("production_image_encoding_not_blocks".into());
    };
    let replay_array_sha =
        sha(&serde_json::to_vec(&content).map_err(|_| "image_array_encode_failed")?);
    let checkpoint = root.join("managed-image-input.json");
    if checkpoint.exists() {
        return Err("prepared_checkpoint_not_fresh".into());
    }
    fs::write(
        &checkpoint,
        serde_json::to_vec(&input).map_err(|_| "input_serialize_failed")?,
    )
    .map_err(|_| "input_checkpoint_failed")?;
    evidence.record(
        json!({"event":"attachment_prepared","image_sha256":image_sha,
        "image_bytes":png.len(),"prompt_sha256":sha(IMAGE_PROMPT.as_bytes()),
        "prompt_bytes":IMAGE_PROMPT.len(),"block_types":["text","image"],"media_type":"image/png",
        "replay_array_sha256":replay_array_sha,
        "attachment_hash_name":saved_path.file_name().and_then(|name|name.to_str()),
        "reference_persisted":true,"expected_reply_sha256":sha(expected.as_bytes())}),
    )?;
    let mut options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("cli_path_missing")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "project_missing")?,
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        model: Some(env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "fixed_model_missing")?),
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let mut session =
        LiveSession::start(options.clone()).map_err(|_| "production_connect_failed")?;
    session
        .ready(evidence)
        .await
        .map_err(|_| "production_ready_failed")?;
    evidence.record(json!({"event":"managed_connection_ready","phase":"new",
        "generation":session.generation,"native_session_id":session.native_id,
        "requested_native_session_id":null,"native_session_association_confirmed":session.native_id.is_some(),
        "inputs_replayed":0}))?;
    let mut cached_ack = None;
    turn(
        &mut session,
        "image",
        input,
        &expected,
        true,
        &mut cached_ack,
        evidence,
    )
    .await?;
    let native_id = session.native_id.clone().ok_or("native_session_missing")?;
    let text = prepare_managed_input(Harness::Claude, TEXT_PROMPT.into(), &[], Vec::new(), &store)
        .map_err(|_| "multiline_preparation_failed")?;
    turn(
        &mut session,
        "multiline",
        text,
        TEXT_REPLY,
        false,
        &mut cached_ack,
        evidence,
    )
    .await?;
    close(&mut session, root, evidence).await?;
    // 从磁盘读取原类型化引用，再通过生产附件恢复函数验证SHA；不向模型重投图片。
    let recovered: Vec<InputContent> =
        serde_json::from_slice(&fs::read(&checkpoint).map_err(|_| "input_checkpoint_missing")?)
            .map_err(|_| "input_checkpoint_invalid")?;
    let images = restore_managed_images(vec![saved_path.clone()], &store)
        .map_err(|_| "durable_attachment_restore_failed")?;
    let rebuilt = prepare_managed_input(
        Harness::Claude,
        IMAGE_PROMPT.into(),
        &images,
        Vec::new(),
        &store,
    )
    .map_err(|_| "durable_input_rebuild_failed")?;
    if recovered != rebuilt
        || sha(&fs::read(&saved_path).map_err(|_| "durable_png_missing")?) != image_sha
    {
        return Err("durable_attachment_changed".into());
    }
    evidence.record(
        json!({"event":"attachment_restore_verified","image_sha256":image_sha,
        "image_bytes":png.len(),"typed_reference_matches":true,"image_replayed_to_native":false}),
    )?;
    options.generation = Uuid::new_v4();
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    let mut resumed = LiveSession::start(options).map_err(|_| "resume_connect_failed")?;
    resumed
        .ready(evidence)
        .await
        .map_err(|_| "resume_ready_failed")?;
    // initialize 尚无原生ID；先校验显式历史目标，后续原生回放再逐帧确认同一会话。
    if resumed.expected_native_id.as_ref() != Some(&native_id)
        || resumed
            .native_id
            .as_ref()
            .is_some_and(|id| id != &native_id)
    {
        return Err("resume_ready_identity_failed".into());
    }
    evidence.record(json!({"event":"managed_connection_ready","phase":"resume",
        "generation":resumed.generation,"native_session_id":resumed.native_id,
        "requested_native_session_id":native_id,"native_session_association_confirmed":resumed.native_id.is_some(),
        "inputs_replayed":0}))?;
    let recall = prepare_managed_input(
        Harness::Claude,
        RECALL_PROMPT.into(),
        &[],
        Vec::new(),
        &store,
    )
    .map_err(|_| "recall_preparation_failed")?;
    turn(
        &mut resumed,
        "recall",
        recall,
        &expected,
        false,
        &mut cached_ack,
        evidence,
    )
    .await?;
    close(&mut resumed, root, evidence).await?;
    evidence.record(
        json!({"event":"acceptance_passed","scope":SCOPE,"native_session_id":native_id,
        "native_inputs":3,"runtime_generations":2,"same_id_deduplication_verified":true,
        "durable_attachment_restore_verified":true,"production_adapter_verified":true,
        "native_framing_only":false,"app_restart_and_ui_verified":false,"sqlite_verified":false,
        "parent_permission_ceiling_verified":false,"http_request_count_verified":false}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要专用隔离运行器、固定CLI与API环境；最多3次原生输入，会消耗模型额度"]
async fn real_claude_managed_png_lifecycle() {
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_MANAGED_IMAGE_TRACE").unwrap(),
        "1"
    );
    assert_eq!(
        env::var("INFINISHELL_CLAUDE_MANAGED_IMAGE_MAX_NATIVE_INPUTS").unwrap(),
        "3"
    );
    let root = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-live-probe")).unwrap(),
        "isolated Claude Rust adapter verification\n"
    );
    let config = PathBuf::from(env::var_os("CLAUDE_CONFIG_DIR").unwrap())
        .canonicalize()
        .unwrap();
    let declared = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(config, declared);
    assert!(!root.starts_with(&config));
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").unwrap(),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence.record(json!({"event":"acceptance_started","scope":SCOPE,"max_native_inputs":3,
        "production_adapter":true,"credential_files_read_by_probe":false,
        "real_gui_verified":false,"sqlite_verified":false,"parent_permission_ceiling_verified":false,
        "http_request_count_verified":false})).unwrap();
    let result =
        tokio::time::timeout(Duration::from_secs(850), exercise(&root, &mut evidence)).await;
    let reason = match &result {
        Ok(Ok(())) => None,
        Ok(Err(reason)) => Some(reason.as_str()),
        Err(_) => Some("fixture_deadline_exceeded"),
    };
    if let Some(reason) = reason {
        evidence
            .record(json!({"event":"acceptance_failed","scope":SCOPE,"reason":reason}))
            .unwrap();
    }
    assert!(
        matches!(result, Ok(Ok(()))),
        "生产Claude图片验收未通过；检查安全证据"
    );
}
