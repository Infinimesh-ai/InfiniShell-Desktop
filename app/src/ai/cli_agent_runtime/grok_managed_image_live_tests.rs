//! 固定 Grok 默认生产连接的图片与冷恢复验收，不启用候选能力或替换协议。

use std::env;
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use uuid::Uuid;
use warp_cli::agent::Harness;

use super::{connect, encode_prompt_content};
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::managed_input::{prepare_managed_input, restore_managed_images};
use crate::ai::cli_agent_runtime::{
    managed_process, ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction,
    RuntimeCommand, RuntimeError, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
};

struct AbortOnDrop(JoinHandle<Result<(), RuntimeError>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "证据写入失败".into())
}

fn image_input(root: &Path, resumed: bool, file: &mut File) -> Result<Vec<InputContent>, String> {
    let mut picture = RgbImage::new(256, 128);
    let colors = if resumed {
        [[0, 255, 0], [255, 255, 0]]
    } else {
        [[255, 0, 0], [0, 0, 255]]
    };
    for (x, _, pixel) in picture.enumerate_pixels_mut() {
        *pixel = Rgb(colors[usize::from(x >= 128)]);
    }
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(picture)
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|_| "PNG 夹具生成失败")?;
    let bytes = encoded.into_inner();
    let store = root.join("state/local-cli-attachments");
    let image = ImageContext {
        data: STANDARD.encode(&bytes),
        mime_type: "image/png".into(),
        file_name: "忽略标签路径.png".into(),
        is_figma: false,
    };
    let prompt = "Inspect only the attached image. Identify the left and right solid colors. Reply with exactly two uppercase English color names separated by one space. Do not use tools or refer to earlier images.";
    let input = prepare_managed_input(Harness::Grok, prompt.into(), &[image], Vec::new(), &store)?;
    let path = input
        .iter()
        .find_map(|part| match part {
            InputContent::LocalImage(path) => Some(path.clone()),
            InputContent::Text(_) | InputContent::Skill { .. } => None,
        })
        .ok_or("缺少持久图片引用")?;
    let checkpoint = root.join(if resumed {
        "resumed-input.json"
    } else {
        "initial-input.json"
    });
    fs::write(
        &checkpoint,
        serde_json::to_vec(&input).map_err(|_| "引用编码失败")?,
    )
    .map_err(|_| "引用保存失败")?;
    let restored: Vec<InputContent> =
        serde_json::from_slice(&fs::read(checkpoint).map_err(|_| "引用读取失败")?)
            .map_err(|_| "引用恢复失败")?;
    let images = restore_managed_images(vec![path], &store)?;
    let rebuilt = prepare_managed_input(Harness::Grok, prompt.into(), &images, Vec::new(), &store)?;
    if rebuilt != input || restored != input || images[0].data != STANDARD.encode(&bytes) {
        return Err("持久图片恢复内容改变".into());
    }
    let native = encode_prompt_content(restored.clone(), &store)?;
    if native[1]["data"] != STANDARD.encode(&bytes) || native[1]["mimeType"] != "image/png" {
        return Err("生产 ACP 编码未保留图片字节".into());
    }
    record(
        file,
        json!({"event":"attachment_prepared","resumed":resumed,"image_sha256":hash(&bytes),
        "image_bytes":bytes.len(),"persistent_reference_restored":true,
        "native_image_content_sha256":hash(&serde_json::to_vec(&native[1]).map_err(|_| "ACP 图片编码失败")?),
        "native_prompt_sha256":hash(&serde_json::to_vec(&native).map_err(|_| "ACP 编码失败")?)}),
    )?;
    Ok(restored)
}

async fn exercise_connection(
    root: &Path,
    previous: Option<String>,
    file: &mut File,
) -> Result<String, String> {
    let resumed = previous.is_some();
    let input = image_input(root, resumed, file)?;
    let generation = Uuid::new_v4();
    let state_dir = root.join("state");
    let connection = connect(SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少原生入口")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "缺少项目")?,
        state_dir: state_dir.clone(),
        target: previous
            .clone()
            .map_or(SessionTarget::New, |native_session_id| {
                SessionTarget::Resume { native_session_id }
            }),
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    })
    .map_err(|error| error.to_string())?;
    let mut task = AbortOnDrop(tokio::spawn(connection.task));
    let controller = connection.controller;
    let mut events = connection.events;
    let ready = tokio::time::timeout(Duration::from_secs(60), events.recv())
        .await
        .map_err(|_| "初始化超时")?
        .ok_or("初始化前事件流关闭")?;
    let RuntimeEventKind::SessionReady {
        verified_cli_version,
        effective_permissions,
        ..
    } = ready.kind
    else {
        return Err("真实根任务初始化失败".into());
    };
    let native_id = ready.native_session_id.ok_or("初始化缺少原生会话")?;
    if ready.generation != generation
        || verified_cli_version.as_deref() != Some("1.0.41")
        || effective_permissions["requestedPolicy"] != "inherit"
        || effective_permissions["reportedMetadata"]["models"]["currentModelId"] != "grok-4.7"
        || previous.as_ref().is_some_and(|id| id != &native_id)
    {
        return Err("原生版本、模型、权限或冷恢复会话不匹配".into());
    }
    record(
        file,
        json!({"event":"ready","generation":generation,"native_session_id":native_id,
        "resumed":resumed,"version":verified_cli_version,"model":"grok-4.7","candidate_flags_used":false}),
    )?;
    let message_id = Uuid::new_v4();
    let command = RuntimeCommand {
        generation,
        message_id,
        action: RuntimeAction::Submit { input },
    };
    controller
        .send(command.clone())
        .await
        .map_err(|_| "提交失败")?;
    controller
        .send(command.clone())
        .await
        .map_err(|_| "相同消息重投失败")?;
    let mut turn_id = None;
    let mut started = false;
    let expected = if resumed { "GREEN YELLOW" } else { "RED BLUE" };
    loop {
        let event = tokio::time::timeout(Duration::from_secs(120), events.recv())
            .await
            .map_err(|_| "图片回合超时")?
            .ok_or("图片回合事件流关闭")?;
        if event.generation != generation || event.native_session_id.as_deref() != Some(&native_id)
        {
            return Err("图片事件关联到旧代次或错误会话".into());
        }
        match event.kind {
            RuntimeEventKind::MessageAccepted {
                message_id: accepted_id,
                turn_id: accepted_turn,
            } => {
                if accepted_id != message_id || turn_id.is_some() || accepted_turn.is_none() {
                    return Err("图片原生 ACK 重复或未关联本输入".into());
                }
                turn_id = accepted_turn;
                record(
                    file,
                    json!({"event":"accepted","generation":generation,"native_session_id":native_id,
                    "message_id":message_id,"turn_id":turn_id}),
                )?;
            }
            RuntimeEventKind::TurnStarted {
                turn_id: started_turn,
            } => {
                if started || turn_id.as_ref() != Some(&started_turn) {
                    return Err("图片回合重复启动或身份不匹配".into());
                }
                started = true;
            }
            RuntimeEventKind::TextDelta {
                turn_id: output_turn,
                ..
            }
            | RuntimeEventKind::Progress {
                turn_id: output_turn,
                ..
            } => {
                if turn_id.as_ref() != Some(&output_turn) {
                    return Err("输出关联到其他回合".into());
                }
            }
            RuntimeEventKind::TurnFinished {
                turn_id: finished_turn,
                outcome,
                output,
            } => {
                record(
                    file,
                    json!({"event":"finished","generation":generation,"native_session_id":native_id,
                    "turn_id":finished_turn,"outcome":outcome,"result":output,"expected":expected}),
                )?;
                if !started
                    || turn_id.as_ref() != Some(&finished_turn)
                    || outcome != TurnOutcome::Completed
                    || output.trim() != expected
                {
                    return Err("图片实际回答、结果关联或终态不匹配".into());
                }
                break;
            }
            RuntimeEventKind::ApprovalRequested { approval_id, .. } => {
                controller
                    .send(RuntimeCommand {
                        generation,
                        message_id: Uuid::new_v4(),
                        action: RuntimeAction::RespondApproval {
                            approval_id,
                            decision: ApprovalDecision::DenyOnce,
                        },
                    })
                    .await
                    .map_err(|_| "额外工具拒绝失败")?;
                return Err("图片校准意外请求工具，已拒绝".into());
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                controller
                    .send(RuntimeCommand {
                        generation,
                        message_id: Uuid::new_v4(),
                        action: RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id,
                            call_id: request.call_id,
                            result: Err("图片校准不授权工具".into()),
                        },
                    })
                    .await
                    .map_err(|_| "本地工具拒绝失败")?;
                return Err("图片校准意外请求本地工具，已拒绝".into());
            }
            RuntimeEventKind::SessionReady { .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::RequestFailed { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err("图片回合出现未预期的运行时事件".into());
            }
        }
    }
    controller
        .send(command)
        .await
        .map_err(|_| "完成后相同 ID 重投失败")?;
    controller
        .send(RuntimeCommand {
            generation,
            message_id: Uuid::new_v4(),
            action: RuntimeAction::Shutdown,
        })
        .await
        .map_err(|_| "关闭提交失败")?;
    tokio::time::timeout(Duration::from_secs(25), &mut task.0)
        .await
        .map_err(|_| "监督进程退出超时")?
        .map_err(|_| "监督任务异常")?
        .map_err(|error| error.to_string())?;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.kind,
            RuntimeEventKind::TurnStarted { .. } | RuntimeEventKind::TurnFinished { .. }
        ) {
            return Err("消息重投导致重复原生执行".into());
        }
    }
    let receipt = managed_process::confirmed_exit(&state_dir, generation)
        .map_err(|_| "退出收据无效")?
        .ok_or("缺少监督者退出收据")?;
    record(
        file,
        json!({"event":"cleanup","generation":generation,"native_session_id":native_id,
        "cleanup_confirmed":receipt.cleanup_confirmed,"exit_code":receipt.exit_code,"exit_reason":receipt.exit_reason,
        "controller_submit_count":3,"native_turn_count":1,"same_id_replay_did_not_start_another_turn":true}),
    )?;
    if !receipt.cleanup_confirmed || receipt.exit_code != Some(0) {
        return Err("原生进程未自然退出；监督者清理结果已单独记录".into());
    }
    Ok(native_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要已认证的固定 Grok、真实监督者与官方在线模型；两次图片输入"]
async fn real_grok_managed_image_resume() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-managed-image-probe")).unwrap(),
        "isolated Grok managed image verification\n"
    );
    let mut file = File::create(root.join("events.ndjson")).unwrap();
    record(
        &mut file,
        json!({"event":"started","scope":"production_grok_images_new_and_cold_resume",
        "max_native_inputs":2,"production_connect":true,"candidate_flags_used":false}),
    )
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(480), async {
        let first = exercise_connection(&root, None, &mut file).await?;
        let resumed = exercise_connection(&root, Some(first.clone()), &mut file).await?;
        if first != resumed {
            return Err("冷恢复更换了原生会话".to_owned());
        }
        record(
            &mut file,
            json!({"event":"acceptance_passed","native_session_id":first,
            "production_adapter_verified":true,"new_and_resume_distinct_images_verified":true,
            "native_inputs":2,"runtime_generations":2,"app_restart_or_gui_verified":false}),
        )
    })
    .await;
    if let Ok(Err(reason)) = &result {
        record(
            &mut file,
            json!({"event":"acceptance_failed","reason":reason}),
        )
        .unwrap();
    }
    assert!(
        matches!(result, Ok(Ok(()))),
        "Grok 生产图片验收失败，保留原始结果检查"
    );
}

/// 仅在生产已验证的最终历史上投影本回合输入；不把出站编码当作原生接收证明。
fn project_native_image_history(
    result: &Value,
    generation: Uuid,
    session: &str,
    turn: &str,
) -> Result<Value, &'static str> {
    let (_, watermark) =
        super::verified_final_snapshot(result, session, turn, &TurnOutcome::Completed)?
            .ok_or("missing_verified_completion")?;
    let rows = result["updates"].as_array().ok_or("missing_history")?;
    let end = rows
        .iter()
        .position(|row| row["params"]["_meta"]["eventId"] == watermark)
        .ok_or("missing_completion_row")?;
    let first = rows[..end]
        .iter()
        .position(|row| row["params"]["_meta"]["promptId"] == turn)
        .ok_or("missing_correlated_output")?;
    let start = rows[..first]
        .iter()
        .rposition(|row| {
            row["method"] == "_x.ai/session/update"
                && row["params"]["update"]["sessionUpdate"] == "turn_completed"
        })
        .map_or(0, |index| index + 1);
    if rows[first..=end]
        .iter()
        .any(|row| row["params"]["update"]["sessionUpdate"] == "user_message_chunk")
    {
        return Err("user_input_crossed_turn_boundary");
    }
    let input_rows = rows[start..first]
        .iter()
        .filter(|row| {
            row["method"] == "session/update"
                && row["params"]["update"]["sessionUpdate"] == "user_message_chunk"
        })
        .collect::<Vec<_>>();
    let mut prompt_index = None;
    let mut images = Vec::new();
    for row in input_rows {
        let params = &row["params"];
        let update = &params["update"];
        let index = update["_meta"]["promptIndex"]
            .as_u64()
            .ok_or("missing_native_prompt_index")?;
        if prompt_index.is_some_and(|previous| previous != index)
            || update["_meta"]["modelId"] != "grok-4.7"
        {
            return Err("native_input_group_or_model_changed");
        }
        prompt_index = Some(index);
        let content = &update["content"];
        match content["type"].as_str() {
            Some("text") => {}
            Some("image") => {
                if content["mimeType"] != "image/png" {
                    return Err("native_image_mime_changed");
                }
                let data = content["data"]
                    .as_str()
                    .filter(|data| data.len() <= super::MAX_LINE_BYTES)
                    .ok_or("native_image_data_invalid")?;
                let decoded = STANDARD
                    .decode(data)
                    .map_err(|_| "native_image_base64_invalid")?;
                images.push(json!({"event_id":params["_meta"]["eventId"],"type":content["type"],
                    "mime_type":content["mimeType"],"image_sha256":hash(&decoded),"image_bytes":decoded.len(),
                    "native_content_sha256":hash(&serde_json::to_vec(content).map_err(|_| "native_content_encoding_failed")?)}));
            }
            Some(_) | None => return Err("unexpected_native_input_block"),
        }
    }
    if images.len() != 1 {
        return Err("native_image_count_mismatch");
    }
    Ok(
        json!({"runtime_generation":generation,"session_id":session,"turn_id":turn,
        "source":"verified_native_final_history","completion_event_id":watermark,
        "prompt_index":prompt_index,"images":images}),
    )
}

pub(super) fn trace_verified_image_history(
    result: &Value,
    generation: Uuid,
    session: &str,
    turn: &str,
) {
    if env::var_os("INFINISHELL_GROK_MANAGED_IMAGE_TRACE").as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    let projection = match project_native_image_history(result, generation, session, turn) {
        Ok(value) => value,
        Err(reason) => json!({"runtime_generation":generation,"session_id":session,"turn_id":turn,
            "projection_failed":reason}),
    };
    eprintln!("GROK_NATIVE_IMAGE_HISTORY {projection}");
}

fn image_history_fixture() -> Value {
    json!({"updates":[
        {"method":"session/update","params":{"sessionId":"session","_meta":{"eventId":"session-1"},
            "update":{"sessionUpdate":"user_message_chunk","content":{"type":"image","mimeType":"image/png","data":"aW1hZ2U="},
                "_meta":{"modelId":"grok-4.7","promptIndex":0}}}},
        {"method":"session/update","params":{"sessionId":"session","_meta":{"eventId":"session-2","promptId":"turn"},
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"RED BLUE"}}}},
        {"method":"_x.ai/session/update","params":{"sessionId":"session","_meta":{"eventId":"session-3"},
            "update":{"sessionUpdate":"turn_completed","prompt_id":"turn","stop_reason":"end_turn"}}}
    ],"lastEventId":"session-3","hasMore":false,"totalCount":3})
}

#[test]
fn native_image_projection_requires_the_actual_session_and_turn() {
    let history = image_history_fixture();
    let projected = project_native_image_history(&history, Uuid::nil(), "session", "turn").unwrap();
    assert_eq!(projected["images"][0]["image_bytes"], 5);
    assert!(project_native_image_history(&history, Uuid::nil(), "old-session", "turn").is_err());
    assert!(project_native_image_history(&history, Uuid::nil(), "session", "other-turn").is_err());
}

#[test]
fn native_image_projection_rejects_input_after_the_correlated_output() {
    let mut history = image_history_fixture();
    history["updates"].as_array_mut().unwrap().swap(0, 1);
    history["updates"][0]["params"]["_meta"]["eventId"] = json!("session-1");
    history["updates"][1]["params"]["_meta"]["eventId"] = json!("session-2");
    assert!(project_native_image_history(&history, Uuid::nil(), "session", "turn").is_err());
}

#[test]
fn native_image_projection_rejects_mismatched_model_and_mime() {
    let mut history = image_history_fixture();
    history["updates"][0]["params"]["update"]["_meta"]["modelId"] = json!("other-model");
    assert!(project_native_image_history(&history, Uuid::nil(), "session", "turn").is_err());
    let mut history = image_history_fixture();
    history["updates"][0]["params"]["update"]["content"]["mimeType"] = json!("image/jpeg");
    assert!(project_native_image_history(&history, Uuid::nil(), "session", "turn").is_err());
}
