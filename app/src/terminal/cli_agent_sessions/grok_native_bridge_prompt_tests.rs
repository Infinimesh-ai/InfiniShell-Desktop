use std::fs;

use serde_json::json;

use super::*;

const RED: &str = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAQCAIAAAD4YuoOAAAAHklEQVR4nGP4z8BAU0Rb00ctGLVg1IJRC0YtoBICAE7E/hC4KNCvAAAAAElFTkSuQmCC";
const BLUE: &str = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAQCAIAAAD4YuoOAAAAHElEQVR4nGNgYPhPYzRqwagFoxaMWjBqwRCwAABSiP4QftsDtAAAAABJRU5ErkJggg==";

fn image(data: &str) -> ImageContext {
    ImageContext {
        data: data.into(),
        mime_type: "image/png".into(),
        file_name: "../../显示名不参与路径.png".into(),
        is_figma: false,
    }
}

#[test]
fn native_rich_prompt_keeps_order_bytes_and_only_persists_scoped_references() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let body = prepare_in_store("中文\n".into(), &[image(RED), image(BLUE)], &store).unwrap();
    assert!(!body.contains(RED));
    assert!(!body.contains(BLUE));
    assert!(!body.contains("显示名"));
    let prompt = decode_from_store(RICH_INPUT_SUBJECT, &body, &store).unwrap();
    assert_eq!(prompt.text_value(), "中文\n");
    assert_eq!(prompt.images[0].data, RED);
    assert_eq!(prompt.images[1].data, BLUE);
    assert_eq!(
        prompt.payload_digest(),
        "blake3:97e469a43a4f801f94c4a09285886353720487402268d93c3bac94dc9d2c09d5"
    );
    let reversed =
        NativeBridgePrompt::with_png("中文\n".into(), &[image(BLUE), image(RED)]).unwrap();
    assert_ne!(prompt.payload_digest(), reversed.payload_digest());
    assert_eq!(
        reversed.payload_digest(),
        "blake3:4179efe33f074957eae316395d881d07756a8c8edc73c7234227f9fd0950fc3e"
    );
    assert_eq!(
        prompt.payload_digest(),
        NativeBridgePrompt::with_png("中文\n".into(), &[image(RED), image(BLUE)])
            .unwrap()
            .payload_digest()
    );
    assert_eq!(fs::read_dir(&store).unwrap().count(), 2);
}

#[test]
fn native_text_subject_never_decodes_json_as_an_image_prompt() {
    let directory = tempfile::tempdir().unwrap();
    let body = r#"{"version":1,"input":[{"LocalImage":"/other/image.png"}]}"#;
    let prompt = decode_from_store(INPUT_SUBJECT, body, directory.path()).unwrap();
    assert!(!prompt.has_images());
    assert_eq!(prompt.text_value(), body);
    assert_eq!(
        prompt.payload_digest(),
        format!("blake3:{}", blake3::hash(body.as_bytes()).to_hex())
    );
    assert!(decode_from_store(RICH_INPUT_SUBJECT, body, directory.path()).is_err());
    assert!(decode_from_store("unknown_subject", body, directory.path()).is_err());
}

#[test]
fn native_pure_png_prompt_rejects_changed_missing_and_foreign_scope_references() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let body = prepare_in_store(String::new(), &[image(RED)], &store).unwrap();
    let prompt = decode_from_store(RICH_INPUT_SUBJECT, &body, &store).unwrap();
    assert!(prompt.has_images());
    assert_eq!(prompt.text_value(), "");
    assert_eq!(
        prompt.payload_digest(),
        "blake3:06867bbf1b6ab21af733242d8be039ebbacb2fee3e9976d42831caaaf48a5483"
    );
    assert!(decode_from_store(RICH_INPUT_SUBJECT, &body, &directory.path().join("other")).is_err());
    let saved: PersistedPrompt = serde_json::from_str(&body).unwrap();
    let InputContent::LocalImage(path) = &saved.input[0] else {
        panic!("纯图必须保存引用")
    };
    fs::write(path, STANDARD.decode(BLUE).unwrap()).unwrap();
    assert!(decode_from_store(RICH_INPUT_SUBJECT, &body, &store).is_err());
    fs::remove_file(path).unwrap();
    assert!(decode_from_store(RICH_INPUT_SUBJECT, &body, &store).is_err());
}

#[test]
fn native_invalid_late_png_prevents_any_attachment_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let mut corrupt = STANDARD.decode(BLUE).unwrap();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert!(
        prepare_in_store(
            "保留原稿".into(),
            &[image(RED), image(&STANDARD.encode(corrupt))],
            &store
        )
        .is_err()
    );
    assert!(!store.exists());
    let truncated = &STANDARD.decode(RED).unwrap()[..60];
    assert!(
        NativeBridgePrompt::with_png(String::new(), &[image(&STANDARD.encode(truncated))]).is_err()
    );
    assert!(NativeBridgePrompt::with_png(String::new(), &[image("not base64")]).is_err());
    let mut trailing = STANDARD.decode(RED).unwrap();
    trailing.extend_from_slice(b"extra");
    assert!(
        NativeBridgePrompt::with_png(String::new(), &[image(&STANDARD.encode(trailing))]).is_err()
    );
    let mut jpeg = image(RED);
    jpeg.mime_type = "image/jpeg".into();
    assert!(NativeBridgePrompt::with_png(String::new(), &[jpeg]).is_err());
}

#[test]
fn native_png_bounds_reject_small_large_and_oversized_batches_without_resizing() {
    let mut small = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut small, ImageFormat::Png)
        .unwrap();
    assert!(
        NativeBridgePrompt::with_png(
            String::new(),
            &[image(&STANDARD.encode(small.into_inner()))]
        )
        .is_err()
    );
    let mut large = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(1151, 1000)
        .write_to(&mut large, ImageFormat::Png)
        .unwrap();
    assert!(
        NativeBridgePrompt::with_png(
            String::new(),
            &[image(&STANDARD.encode(large.into_inner()))]
        )
        .is_err()
    );
    assert_eq!(
        NativeBridgePrompt::with_png(String::new(), &vec![image(RED); 21])
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidInput,
    );
    assert!(NativeBridgePrompt::with_png("x".repeat(MAX_TEXT_BYTES + 1), &[image(RED)]).is_err());
}

#[test]
fn native_rich_history_rejects_extra_fields_text_after_image_and_skills() {
    let directory = tempfile::tempdir().unwrap();
    let body = prepare_in_store("首段".into(), &[image(RED)], directory.path()).unwrap();
    let mut saved: serde_json::Value = serde_json::from_str(&body).unwrap();
    saved["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Text":"第二段"}));
    assert!(decode_from_store(RICH_INPUT_SUBJECT, &saved.to_string(), directory.path()).is_err());
    let mut saved: serde_json::Value = serde_json::from_str(&body).unwrap();
    saved["authority"] = json!(true);
    assert!(decode_from_store(RICH_INPUT_SUBJECT, &saved.to_string(), directory.path()).is_err());
    let saved = json!({"version":1,"input":[{"Skill":{"name":"not-supported","path":"/other"}}]});
    assert!(decode_from_store(RICH_INPUT_SUBJECT, &saved.to_string(), directory.path()).is_err());
}
