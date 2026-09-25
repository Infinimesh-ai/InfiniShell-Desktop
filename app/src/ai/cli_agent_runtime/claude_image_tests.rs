//! 持久图片输入与原生数组回放的离线安全回归，不执行真实 CLI。

use std::fs;
use std::io::Cursor as ImageCursor;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat, RgbImage};
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use tempfile::TempDir;

use super::*;

fn image_scope(format: ImageFormat) -> (TempDir, PathBuf) {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    fs::create_dir(&store).unwrap();
    let mut encoded = ImageCursor::new(Vec::new());
    DynamicImage::new_rgb8(2, 2)
        .write_to(&mut encoded, format)
        .unwrap();
    let bytes = encoded.into_inner();
    let extension = format.extensions_str()[0];
    let path = store.join(format!("{:x}.{extension}", Sha256::digest(&bytes)));
    fs::write(&path, bytes).unwrap();
    (directory, path)
}

fn scoped_protocol(directory: &TempDir) -> ClaudeProtocol {
    let mut protocol = ready_protocol();
    protocol.options.state_dir = directory.path().to_owned();
    protocol.attachment_store = directory.path().join("local-cli-attachments");
    protocol
        .bind_probed_version(true, "2.1.280 (Claude Code)")
        .unwrap();
    protocol
        .receive(json!({"type":"system","subtype":"init","claude_code_version":"2.1.280","permissionMode":"default",
        "tools":[],"mcp_servers":[],"session_id":"3ddff71c-4062-4198-a130-502e4c15684e"}))
        .unwrap();
    protocol
}

fn image_command(id: Uuid, path: PathBuf) -> RuntimeCommand {
    command(
        id,
        RuntimeAction::Submit {
            input: vec![
                InputContent::Text("中文与 English\n保留多行及 `$()`".into()),
                InputContent::LocalImage(path),
            ],
        },
    )
}

fn replay(sent: &Value) -> Value {
    let mut replay = sent.clone();
    replay["isReplay"] = json!(true);
    replay["session_id"] = json!("3ddff71c-4062-4198-a130-502e4c15684e");
    replay
}

#[test]
fn host_process_scope_preserves_application_attachment_scope() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    protocol.options.state_dir = directory
        .path()
        .join("cli-agent-hosts")
        .join(protocol.options.generation.to_string())
        .join("native");
    fs::create_dir_all(&protocol.options.state_dir).unwrap();

    let sent = protocol.command(image_command(Uuid::from_u128(330), path.clone()));
    assert_eq!(sent.writes.len(), 1);
    assert_eq!(
        sent.writes[0]["message"]["content"][1]["source"]["data"],
        STANDARD.encode(fs::read(&path).unwrap())
    );

    // 相同图片放在进程目录也不能绕过应用附件目录的边界。
    let foreign = protocol.options.state_dir.join("local-cli-attachments");
    fs::create_dir(&foreign).unwrap();
    let foreign_path = foreign.join(path.file_name().unwrap());
    fs::copy(&path, &foreign_path).unwrap();
    let mut other_protocol = scoped_protocol(&directory);
    other_protocol
        .options
        .state_dir
        .clone_from(&protocol.options.state_dir);
    let rejected = other_protocol.command(image_command(Uuid::from_u128(331), foreign_path));
    assert!(rejected.writes.is_empty());
    assert!(rejected.events.iter().any(
        |event| matches!(event, RuntimeEventKind::RequestFailed { message, .. }
                if message == &crate::t!("cli-agent-input-attachment-invalid"))
    ));
}

#[test]
fn durable_png_input_is_native_text_image_array_and_ledger_retains_only_summary() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(300);
    let sent = protocol.command(image_command(id, path.clone()));
    assert_eq!(sent.writes.len(), 1);
    assert!(sent.events.is_empty());
    let content = &sent.writes[0]["message"]["content"];
    assert_eq!(
        content,
        &json!([
            {"type":"text","text":"中文与 English\n保留多行及 `$()`"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":STANDARD.encode(fs::read(path).unwrap())}}
        ])
    );
    let ExpectedReplay::Blocks(summary) = &protocol.turns[&id].expected_replay else {
        panic!("图片必须保留有界摘要");
    };
    assert_eq!(
        summary.kinds,
        vec![InputBlockKind::Text, InputBlockKind::Image]
    );
    assert_eq!(summary.bytes, serde_json::to_vec(content).unwrap().len());
    assert_eq!(summary.sha256, fingerprint(content));
    assert!(!format!("{summary:?}").contains(content[1]["source"]["data"].as_str().unwrap()));
    let accepted = protocol.receive(replay(&sent.writes[0])).unwrap();
    assert_eq!(
        accepted.events,
        vec![RuntimeEventKind::MessageAccepted {
            message_id: id,
            turn_id: Some(id.to_string())
        }]
    );
    assert!(
        protocol
            .receive(replay(&sent.writes[0]))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn matching_image_replay_preserves_real_started_assistant_and_result_path() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(301);
    let sent = protocol.command(image_command(id, path));
    protocol.receive(replay(&sent.writes[0])).unwrap();
    assert_eq!(
        protocol.receive(lifecycle(id, "started")).unwrap().events,
        vec![RuntimeEventKind::TurnStarted {
            turn_id: id.to_string()
        }]
    );
    protocol.receive(json!({"type":"assistant","uuid":Uuid::from_u128(330),"session_id":"3ddff71c-4062-4198-a130-502e4c15684e",
        "user_message_uuid":id,"message":{"role":"assistant","content":[{"type":"text","text":"NATIVE_IMAGE_RESULT"}]}})).unwrap();
    let result = protocol
        .receive(json!({"type":"result","uuid":Uuid::from_u128(331),
        "session_id":"3ddff71c-4062-4198-a130-502e4c15684e","subtype":"success","is_error":false,
        "user_message_uuid":id,"user_message_uuids":[id],"result":"NATIVE_IMAGE_RESULT"}))
        .unwrap();
    assert!(result.events.iter().any(|event|matches!(event,RuntimeEventKind::TurnFinished {
        turn_id,outcome:TurnOutcome::Completed,output } if turn_id == &id.to_string() && output == "NATIVE_IMAGE_RESULT")));
}

#[test]
fn image_replay_rejects_changed_text_image_data_order_mime_and_extra_shape_fields() {
    for mutation in 0..9 {
        let (directory, path) = image_scope(ImageFormat::Png);
        let mut protocol = scoped_protocol(&directory);
        let id = Uuid::from_u128(302);
        let sent = protocol.command(image_command(id, path));
        let mut input = replay(&sent.writes[0]);
        match mutation {
            0 => input["message"]["content"][0]["text"] = json!("changed"),
            1 => input["message"]["content"][1]["source"]["data"] = json!("changed"),
            2 => input["message"]["content"]
                .as_array_mut()
                .unwrap()
                .swap(0, 1),
            3 => input["message"]["content"][1]["source"]["media_type"] = json!("image/jpeg"),
            4 => input["message"]["content"][1]["source"]["type"] = json!("url"),
            5 => input["message"]["content"][1]["source"]["extra"] = json!(true),
            6 => input["message"]["content"][0]["extra"] = json!(true),
            7 => input["message"]["content"] = json!("same text is insufficient"),
            8 => input["message"]["content"][0]["text"] = json!({"text":"wrong type"}),
            _ => unreachable!("有限测试范围"),
        }
        assert!(protocol.receive(input).is_err());
        assert_eq!(protocol.turns[&id].accepted, false);
    }
}

#[test]
fn image_replay_requires_exact_role_native_session_and_null_or_missing_parent() {
    for mutation in 0..5 {
        let (directory, path) = image_scope(ImageFormat::Png);
        let mut protocol = scoped_protocol(&directory);
        let id = Uuid::from_u128(303);
        let sent = protocol.command(image_command(id, path));
        let mut input = replay(&sent.writes[0]);
        match mutation {
            0 => input["message"]["role"] = json!("assistant"),
            1 => input["session_id"] = json!("other-native-session"),
            2 => {
                input.as_object_mut().unwrap().remove("session_id");
            }
            3 => input["session_id"] = json!(""),
            4 => input["parent_tool_use_id"] = json!("subagent-tool"),
            _ => unreachable!("有限测试范围"),
        }
        let received = protocol.receive(input);
        assert!(received.is_err() || received.unwrap().events.is_empty());
        assert_eq!(protocol.turns[&id].accepted, false);
    }
}

#[test]
fn image_replay_missing_parent_is_valid_and_unknown_user_uuid_cannot_use_current_turn() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(304);
    let sent = protocol.command(image_command(id, path));
    let mut unknown = replay(&sent.writes[0]);
    unknown["uuid"] = json!(Uuid::from_u128(999));
    protocol.active_turn = Some(id);
    assert!(protocol.receive(unknown).unwrap().events.is_empty());
    assert_eq!(protocol.turns[&id].accepted, false);
    let mut input = replay(&sent.writes[0]);
    input.as_object_mut().unwrap().remove("parent_tool_use_id");
    assert!(
        protocol
            .receive(input)
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event,
        RuntimeEventKind::MessageAccepted { message_id,.. } if message_id == &id))
    );
}

#[test]
fn image_callback_after_started_still_rejects_changed_content_without_emitting_completion() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(305);
    let sent = protocol.command(image_command(id, path));
    protocol.receive(lifecycle(id, "started")).unwrap();
    let mut input = replay(&sent.writes[0]);
    input["message"]["content"][1]["source"]["data"] = json!("wrong payload");
    assert!(protocol.receive(input).is_err());
    assert_eq!(protocol.turns[&id].finished, false);
}

#[test]
fn missing_corrupt_and_cross_scope_durable_images_fail_without_partial_write() {
    for mutation in 0..3 {
        let (directory, path) = image_scope(ImageFormat::Png);
        let mut protocol = scoped_protocol(&directory);
        match mutation {
            0 => {
                fs::remove_file(&path).unwrap();
            }
            1 => fs::write(&path, b"corrupt PNG bytes").unwrap(),
            2 => protocol.attachment_store = directory.path().join("another-scope"),
            _ => unreachable!("有限测试范围"),
        }
        let id = Uuid::from_u128(306);
        let failure = protocol.command(image_command(id, path));
        assert!(failure.writes.is_empty());
        assert!(matches!(
            failure.events[0],
            RuntimeEventKind::RequestFailed { .. }
        ));
        assert_eq!(protocol.turns.contains_key(&id), false);
    }
}

fn assert_native_image_format(format: ImageFormat, mime_type: &str) {
    let (directory, path) = image_scope(format);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(307);
    let sent = protocol.command(image_command(id, path.clone()));
    assert_eq!(sent.writes.len(), 1);
    assert!(sent.events.is_empty());
    assert_eq!(
        sent.writes[0]["message"]["content"][1]["source"]["media_type"],
        mime_type
    );
    assert_eq!(
        sent.writes[0]["message"]["content"][1]["source"]["data"],
        STANDARD.encode(fs::read(path).unwrap())
    );
    let accepted = protocol.receive(replay(&sent.writes[0])).unwrap();
    assert_eq!(
        accepted.events,
        vec![RuntimeEventKind::MessageAccepted {
            message_id: id,
            turn_id: Some(id.to_string())
        }]
    );
}

#[test]
fn durable_jpeg_preserves_native_mime_and_exact_replay() {
    assert_native_image_format(ImageFormat::Jpeg, "image/jpeg");
}

#[test]
fn durable_webp_preserves_native_mime_and_exact_replay() {
    assert_native_image_format(ImageFormat::WebP, "image/webp");
}

#[test]
fn durable_gif_preserves_native_mime_and_exact_replay() {
    assert_native_image_format(ImageFormat::Gif, "image/gif");
}

#[test]
fn newly_verified_formats_require_the_fixed_native_version() {
    let (directory, path) = image_scope(ImageFormat::Jpeg);
    let mut protocol = scoped_protocol(&directory);
    protocol.probed_version = Some("2.1.278");
    let rejected = protocol.command(image_command(Uuid::from_u128(337), path));
    assert!(rejected.writes.is_empty());
    assert!(
        matches!(&rejected.events[0], RuntimeEventKind::RequestFailed { message, .. }
        if message == &crate::t!("cli-agent-claude-rich-images-version"))
    );
    assert!(protocol.turns.is_empty());
}

#[test]
fn pure_images_require_the_fixed_native_version() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    protocol.probed_version = Some("2.1.278");
    let rejected = protocol.command(command(
        Uuid::from_u128(338),
        RuntimeAction::Submit {
            input: vec![InputContent::LocalImage(path)],
        },
    ));
    assert!(rejected.writes.is_empty());
    assert!(
        matches!(&rejected.events[0], RuntimeEventKind::RequestFailed { message, .. }
        if message == &crate::t!("cli-agent-claude-rich-images-version"))
    );
    assert!(protocol.turns.is_empty());
}

#[test]
fn unregistered_image_skill_is_rejected_before_reading_any_attachment() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    fs::remove_file(&path).unwrap();
    let id = Uuid::from_u128(308);
    let failure = protocol.command(command(
        id,
        RuntimeAction::Submit {
            input: vec![
                InputContent::Text("中文".into()),
                InputContent::LocalImage(path),
                InputContent::Skill {
                    name: "review".into(),
                    path: directory.path().join("SKILL.md"),
                },
            ],
        },
    ));
    assert!(failure.writes.is_empty());
    assert!(
        matches!(&failure.events[0],RuntimeEventKind::RequestFailed {message,..}
        if message == "skill was not selected and registered for this connection")
    );
}

#[test]
fn image_skill_uses_registered_native_tool_and_preserves_user_input_and_replay() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let selected_directory = directory.path().join("selected-skill");
    fs::create_dir(&selected_directory).unwrap();
    let skill_path = selected_directory.join("SKILL.md");
    fs::write(&skill_path, "---\nname: inspect-picture\ndescription: Inspect attached pictures\n---\nInspect only the attached picture.\n").unwrap();
    protocol.skill_plugin =
        prepare_claude_skill_plugin(&[super::super::super::local_skills::SelectedLocalSkill {
            name: "inspect-picture".into(),
            path: skill_path.clone(),
        }])
        .unwrap();
    let id = Uuid::from_u128(339);
    let sent = protocol.command(command(
        id,
        RuntimeAction::Submit {
            input: vec![
                InputContent::Text("中文问题\n保留 `$()`".into()),
                InputContent::LocalImage(path),
                InputContent::Skill {
                    name: "inspect-picture".into(),
                    path: skill_path,
                },
            ],
        },
    ));
    assert_eq!(sent.writes.len(), 1);
    assert_eq!(
        sent.writes[0]["message"]["content"][0]["text"],
        "Invoke the Skill tool with skill=infinishell-local-skills:inspect-picture, then apply that skill to the attached images.\n\n中文问题\n保留 `$()`"
    );
    assert_eq!(sent.writes[0]["message"]["content"][1]["type"], "image");
    let accepted = protocol.receive(replay(&sent.writes[0])).unwrap();
    assert_eq!(
        accepted.events,
        vec![RuntimeEventKind::MessageAccepted {
            message_id: id,
            turn_id: Some(id.to_string())
        }]
    );
}

#[test]
fn pure_image_has_no_synthetic_text_and_requires_matching_native_replay() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(309);
    let sent = protocol.command(command(
        id,
        RuntimeAction::Submit {
            input: vec![InputContent::LocalImage(path)],
        },
    ));
    assert_eq!(sent.writes.len(), 1);
    assert_eq!(
        sent.writes[0]["message"]["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(sent.writes[0]["message"]["content"][0]["type"], "image");
    let accepted = protocol.receive(replay(&sent.writes[0])).unwrap();
    assert_eq!(
        accepted.events,
        vec![RuntimeEventKind::MessageAccepted {
            message_id: id,
            turn_id: Some(id.to_string())
        }]
    );
}

#[test]
fn oversized_image_text_does_not_enter_native_turn_ledger() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let oversized = protocol.command(command(
        Uuid::from_u128(310),
        RuntimeAction::Submit {
            input: vec![
                InputContent::Text("x".repeat(MAX_INPUT_BYTES + 1)),
                InputContent::LocalImage(path),
            ],
        },
    ));
    assert!(oversized.writes.is_empty());
    assert_eq!(protocol.turns.len(), 0);
}

#[test]
fn replay_rejects_text_only_arrays_and_too_many_pure_images() {
    assert!(summarize_blocks(&json!([{ "type": "text", "text": "缺失图片" }])).is_err());
    let images = vec![
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}});
        21
    ];
    assert!(summarize_blocks(&json!(images)).is_err());
}

#[test]
fn image_count_limit_is_checked_before_reading_a_missing_attachment() {
    let directory = TempDir::new().unwrap();
    let mut protocol = scoped_protocol(&directory);
    let mut input = vec![InputContent::Text("检查附件数量".into())];
    input.extend(std::iter::repeat_n(
        InputContent::LocalImage(directory.path().join("local-cli-attachments/missing.png")),
        21,
    ));
    let failure = protocol.command(command(
        Uuid::from_u128(311),
        RuntimeAction::Submit { input },
    ));
    assert!(failure.writes.is_empty());
    assert!(
        matches!(&failure.events[0],RuntimeEventKind::RequestFailed {message,..}
        if message == &crate::t!("editor-images-disabled-query-limit",limit = MAX_IMAGE_COUNT_FOR_QUERY))
    );
}

#[test]
fn same_message_id_replay_does_not_reopen_missing_image_and_changed_payload_is_rejected() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(312);
    let command = image_command(id, path.clone());
    let sent = protocol.command(command.clone());
    protocol.receive(replay(&sent.writes[0])).unwrap();
    fs::remove_file(path).unwrap();
    let repeated = protocol.command(command.clone());
    assert!(repeated.writes.is_empty());
    assert!(matches!(
        repeated.events[0],
        RuntimeEventKind::MessageAccepted { .. }
    ));
    let mut conflict = command;
    conflict.action = RuntimeAction::Submit {
        input: vec![InputContent::Text("changed payload".into())],
    };
    let failure = protocol.command(conflict);
    assert!(failure.writes.is_empty());
    assert!(matches!(
        failure.events[0],
        RuntimeEventKind::RequestFailed { .. }
    ));
}

#[test]
fn native_replay_summary_is_canonical_for_json_field_order() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(313);
    let sent = protocol.command(image_command(id, path));
    let mut input = replay(&sent.writes[0]);
    let data = input["message"]["content"][1]["source"]["data"].clone();
    input["message"]["content"][1] =
        json!({"source":{"data":data,"media_type":"image/png","type":"base64"},"type":"image"});
    assert!(
        protocol
            .receive(input)
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event,
        RuntimeEventKind::MessageAccepted {message_id,..} if message_id == &id))
    );
}

#[test]
fn full_image_envelope_and_newline_reserve_room_for_native_replay() {
    let mut message = json!({"type":"user","message":{"role":"user","content":[
        {"type":"text","text":"中文"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":""}}
    ]},"parent_tool_use_id":null,"session_id":"3ddff71c-4062-4198-a130-502e4c15684e","uuid":Uuid::from_u128(314)});
    let envelope_bytes = encode_message(&message).unwrap().len();
    message["message"]["content"][1]["source"]["data"] =
        json!("A".repeat(MAX_IMAGE_MESSAGE_BYTES - envelope_bytes));
    assert_eq!(
        encode_message(&message).unwrap().len(),
        MAX_IMAGE_MESSAGE_BYTES
    );
    let native_replay = replay(&message);
    assert!(serde_json::to_vec(&native_replay).unwrap().len() + 1 <= MAX_LINE_BYTES);
    assert!(summarize_blocks(&native_replay["message"]["content"]).is_ok());
    message["message"]["content"][1]["source"]["data"] =
        json!("A".repeat(MAX_IMAGE_MESSAGE_BYTES - envelope_bytes + 1));
    assert!(encode_message(&message).is_err());
}

#[test]
fn image_trace_projects_only_native_user_content_without_base64_or_thinking() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let sent = protocol.command(image_command(Uuid::from_u128(320), path.clone()));
    let frame = replay(&sent.writes[0]);
    let projection = live_native_image_projection(&frame);
    assert_eq!(projection["block_types"], json!(["text", "image"]));
    assert_eq!(projection["media_type"], "image/png");
    assert_eq!(
        projection["image_sha256"],
        format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
    );
    let data = frame["message"]["content"][1]["source"]["data"]
        .as_str()
        .unwrap();
    assert!(!serde_json::to_string(&projection).unwrap().contains(data));
    assert_eq!(
        live_native_image_projection(&json!({"type":"assistant", "message":{
        "role":"assistant", "content":[{"type":"thinking", "thinking":"PRIVATE_THINKING_CANARY"}]}})),
        Value::Null
    );
    let mut changed = frame.clone();
    changed["message"]["role"] = json!("assistant");
    assert_eq!(live_native_image_projection(&changed), Value::Null);
    changed = frame;
    changed["message"]["content"][1]["private"] = json!("PRIVATE_PAYLOAD_CANARY");
    assert_eq!(live_native_image_projection(&changed), Value::Null);
}

#[test]
fn total_block_limit_rejects_large_base64_array_even_when_each_image_is_below_frame_limit() {
    let content = json!([
        {"type":"text","text":"检查总预算"},
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(5_000_000)}},
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(5_000_000)}}
    ]);
    assert!(summarize_blocks(&content).is_err());
}

#[tokio::test]
async fn oversized_message_is_rejected_before_writing_any_bytes() {
    let mut output = Cursor::new(Vec::new());
    let message =
        json!({"type":"user","message":{"role":"user","content":"x".repeat(MAX_LINE_BYTES)}});
    assert!(write_message(&mut output, &message).await.is_err());
    assert_eq!(output.into_inner().len(), 0);
}

#[cfg(unix)]
#[test]
fn symlink_attachment_is_rejected_and_never_deleted_by_failed_submit() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let target = directory.path().join("retained-original.png");
    fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let mut protocol = scoped_protocol(&directory);
    let failed = protocol.command(image_command(Uuid::from_u128(315), path.clone()));
    assert!(failed.writes.is_empty());
    assert_eq!(target.exists(), true);
    assert_eq!(
        fs::symlink_metadata(path).unwrap().file_type().is_symlink(),
        true
    );
}

#[test]
fn valid_png_with_wrong_persisted_hash_filename_is_rejected() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let wrong = path
        .parent()
        .unwrap()
        .join("0000000000000000000000000000000000000000000000000000000000000000.png");
    fs::rename(path, &wrong).unwrap();
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(316);
    let failure = protocol.command(image_command(id, wrong));
    assert!(failure.writes.is_empty());
    assert_eq!(protocol.turns.contains_key(&id), false);
}

#[test]
fn queued_image_join_requires_result_identifying_complete_native_input_batch() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let mut protocol = scoped_protocol(&directory);
    let first = Uuid::from_u128(317);
    let second = Uuid::from_u128(318);
    let sent = protocol.command(image_command(first, path.clone()));
    protocol.receive(replay(&sent.writes[0])).unwrap();
    protocol.receive(lifecycle(first, "started")).unwrap();
    let followup = protocol.command(image_command(second, path));
    protocol.receive(replay(&followup.writes[0])).unwrap();
    assert_eq!(
        protocol
            .receive(lifecycle(second, "started"))
            .unwrap()
            .events,
        vec![RuntimeEventKind::InputJoined {
            message_id: second,
            turn_id: first.to_string()
        }]
    );
    let incomplete = json!({"type":"result","uuid":Uuid::from_u128(332),
        "session_id":"3ddff71c-4062-4198-a130-502e4c15684e","subtype":"success","is_error":false,
        "user_message_uuid":second,"user_message_uuids":[second],"result":"not a complete batch"});
    assert!(protocol.receive(incomplete).is_err());
    assert_eq!(protocol.turns[&first].finished, false);
    assert_eq!(protocol.turns[&second].finished, false);
}

#[test]
fn individually_valid_png_files_cannot_exceed_total_native_message_budget() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    fs::create_dir(&store).unwrap();
    // 固定随机种子生成低压缩率的实际 PNG，逐图大小与像素均在共享上限内。
    let mut pixels = vec![0u8; 1024 * 1100 * 3];
    StdRng::seed_from_u64(601).fill_bytes(&mut pixels);
    let image = RgbImage::from_raw(1024, 1100, pixels).unwrap();
    let mut encoded = ImageCursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let bytes = encoded.into_inner();
    assert!(bytes.len() < crate::util::image::MAX_IMAGE_SIZE_BYTES);
    let path = store.join(format!("{:x}.png", Sha256::digest(&bytes)));
    fs::write(&path, bytes).unwrap();
    let mut protocol = scoped_protocol(&directory);
    let id = Uuid::from_u128(319);
    let failure = protocol.command(command(
        id,
        RuntimeAction::Submit {
            input: vec![
                InputContent::Text("检查总帧预算".into()),
                InputContent::LocalImage(path.clone()),
                InputContent::LocalImage(path),
            ],
        },
    ));
    assert!(failure.writes.is_empty());
    assert!(
        matches!(&failure.events[0],RuntimeEventKind::RequestFailed {message,..}
        if message == &crate::t!("editor-image-too-large"))
    );
    assert_eq!(protocol.turns.contains_key(&id), false);
}
