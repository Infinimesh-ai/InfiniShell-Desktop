use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat};
use tempfile::TempDir;

use super::*;
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::managed_input::prepare_managed_input;

fn png_input(store: &Path) -> (Vec<InputContent>, String) {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(2, 3)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let data = STANDARD.encode(bytes.into_inner());
    let input = prepare_managed_input(
        Harness::Grok,
        "图片\n第二行".into(),
        &[ImageContext {
            data: data.clone(),
            mime_type: "image/png".into(),
            file_name: "图片.png".into(),
            is_figma: false,
        }],
        Vec::new(),
        store,
    )
    .unwrap();
    (input, data)
}

#[test]
fn durable_png_is_sent_as_original_bytes_and_restores_after_new_encoder() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let (input, data) = png_input(&store);
    let expected = vec![
        json!({"type":"text", "text":"图片\n第二行"}),
        json!({"type":"image", "mimeType":"image/png", "data":data}),
    ];
    assert_eq!(
        encode_prompt_content(input.clone(), &store).unwrap(),
        expected
    );
    assert_eq!(encode_prompt_content(input, &store).unwrap(), expected);
}

#[test]
fn invalid_attachment_never_produces_partial_prompt() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let (mut input, _) = png_input(&store);
    input.push(InputContent::LocalImage(store.join("missing.png")));
    assert!(encode_prompt_content(input, &store).is_err());
    let (input, _) = png_input(&store);
    assert!(encode_prompt_content(input, directory.path()).is_err());
}

#[test]
fn image_byte_tampering_is_rejected_before_dispatch() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let (input, _) = png_input(&store);
    let InputContent::LocalImage(path) = &input[1] else {
        panic!("缺少图片引用")
    };
    std::fs::write(path, b"invalid image").unwrap();
    assert!(encode_prompt_content(input, &store).is_err());
}

#[test]
fn image_batch_limits_and_json_escaping_count_toward_frame_budget() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let (input, _) = png_input(&store);
    assert!(
        encode_prompt_content(
            vec![input[1].clone(); MAX_IMAGE_COUNT_FOR_QUERY + 1],
            &store
        )
        .is_err()
    );
    assert!(
        encode_prompt_content(
            vec![InputContent::Text("\u{0001}".repeat(MAX_LINE_BYTES / 4))],
            &store
        )
        .is_err()
    );
}

#[test]
fn native_image_advertisement_does_not_expand_fixed_version_model_or_policy() {
    let mut protocol = GrokProtocol::new(super::tests::options());
    protocol.probed_version = Some(ROOT_VERSION);
    protocol.paired_version = Some(ROOT_VERSION);
    protocol.reported_metadata.models = Some(ReportedModels {
        current_model_id: "grok-4.7".into(),
        available_models: Vec::new(),
    });
    protocol.reported_capabilities = json!({"promptCapabilities":{"image":false}});
    assert!(protocol.image_input_verified());
    protocol.reported_capabilities["promptCapabilities"]["image"] = json!(true);
    protocol.probed_version = Some(CURRENT_VERSION);
    assert!(!protocol.image_input_verified());
    protocol.probed_version = Some(ROOT_VERSION);
    protocol
        .reported_metadata
        .models
        .as_mut()
        .unwrap()
        .current_model_id = "unknown".into();
    assert!(!protocol.image_input_verified());
    protocol
        .reported_metadata
        .models
        .as_mut()
        .unwrap()
        .current_model_id = "grok-4.7".into();
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    assert!(!protocol.image_input_verified());
    protocol.options.permission_policy = PermissionPolicy::Inherit;
    protocol.options.selected_skills.push(SelectedLocalSkill {
        name: "image-session-skill".into(),
        path: protocol.options.cwd.join("SKILL.md"),
    });
    // 本轮没有技能块，也不能将纯图片证据扩大到启动时已注册技能的会话。
    assert!(!protocol.image_input_verified());
}
