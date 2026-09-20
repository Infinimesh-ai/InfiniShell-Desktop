use ai::skills::parse_skill;
use tempfile::TempDir;

use super::*;

fn image_context() -> ImageContext {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    ImageContext {
        data: STANDARD.encode(bytes.into_inner()),
        mime_type: "image/png".to_owned(),
        file_name: "../../不应访问的路径 $(touch ignored).jpg".to_owned(),
        is_figma: false,
    }
}

fn local_image(input: &[InputContent]) -> &Path {
    input
        .iter()
        .find_map(|part| match part {
            InputContent::LocalImage(path) => Some(path.as_path()),
            InputContent::Text(_) | InputContent::Skill { .. } => None,
        })
        .expect("图片不能被静默丢弃")
}

fn skill(directory: &Path) -> ParsedSkill {
    let path = directory.join("SKILL.md");
    fs::write(
        &path,
        "---\nname: local-review\ndescription: Review local changes\n---\n检查修改及相邻资源。\n",
    )
    .unwrap();
    parse_skill(&path).unwrap()
}

#[test]
fn managed_input_preserves_utf8_context_and_typed_image_and_skill() {
    let directory = TempDir::new().unwrap();
    let skill = skill(directory.path());
    let image = image_context();
    let text = "中文和 English\n文件上下文: C:\\Users\\Name With Space\\a.rs\n`$()` 'quoted'";
    let prepared = prepare_managed_input(
        Harness::Codex,
        text.to_owned(),
        &[image.clone()],
        vec![skill],
        &directory.path().join("local-cli-attachments"),
    )
    .unwrap();
    assert_eq!(prepared.len(), 3);
    assert_eq!(prepared[0], InputContent::Text(text.to_owned()));
    assert!(
        matches!(&prepared[2], InputContent::Skill { name, path } if name == "local-review" && path == &directory.path().join("SKILL.md"))
    );
    let saved_path = local_image(&prepared);
    assert!(saved_path.is_absolute());
    assert_eq!(saved_path.file_name().unwrap().to_string_lossy().len(), 68);
    assert_eq!(saved_path.extension().unwrap(), "png");
    assert_eq!(
        fs::read(saved_path).unwrap(),
        STANDARD.decode(image.data).unwrap()
    );
}

#[test]
fn duplicate_image_reuses_identical_file_and_cancellation_keeps_durable_path() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let image = image_context();
    let first = prepare_managed_input(
        Harness::Codex,
        String::new(),
        &[image.clone()],
        Vec::new(),
        &store,
    )
    .unwrap();
    let path = local_image(&first).to_owned();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let second = prepare_managed_input(
        Harness::Codex,
        "第二轮".to_owned(),
        &[image],
        Vec::new(),
        &store,
    )
    .unwrap();
    assert_eq!(local_image(&second), path);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read_dir(&store).unwrap().count(), 1);
    // 丢弃准备结果模拟用户取消或面板关闭；历史消息仍可使用相同附件路径。
    drop(first);
    drop(second);
    assert!(path.is_file());
    assert!(!fs::read(&path).unwrap().is_empty());
}

#[test]
fn corrupt_existing_attachment_is_reported_without_overwrite_or_partial_input() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let image = image_context();
    let first = prepare_managed_input(
        Harness::Codex,
        String::new(),
        &[image.clone()],
        Vec::new(),
        &store,
    )
    .unwrap();
    let path = local_image(&first);
    let mut damaged = fs::read(path).unwrap();
    damaged[0] ^= 0xff;
    fs::write(path, &damaged).unwrap();
    assert!(
        prepare_managed_input(
            Harness::Codex,
            "保留草稿".to_owned(),
            &[image],
            Vec::new(),
            &store
        )
        .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), damaged);
    assert_eq!(fs::read_dir(store).unwrap().count(), 1);
}

#[test]
fn invalid_image_batch_fails_before_creating_assets() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let valid = image_context();
    let mut invalid = valid.clone();
    invalid.data = "无效 base64".to_owned();
    assert!(
        prepare_managed_input(
            Harness::Codex,
            "全部保留".to_owned(),
            &[valid.clone(), invalid],
            Vec::new(),
            &store
        )
        .is_err()
    );
    assert!(!store.exists());
    let mut wrong_mime = valid.clone();
    wrong_mime.mime_type = "image/jpeg".to_owned();
    assert!(
        prepare_managed_input(
            Harness::Codex,
            String::new(),
            &[wrong_mime],
            Vec::new(),
            &store
        )
        .is_err()
    );
    let mut truncated = valid;
    let mut bytes = STANDARD.decode(&truncated.data).unwrap();
    bytes.truncate(30);
    truncated.data = STANDARD.encode(bytes);
    assert!(
        prepare_managed_input(
            Harness::Codex,
            String::new(),
            &[truncated],
            Vec::new(),
            &store
        )
        .is_err()
    );
    assert!(!store.exists());
}

#[test]
fn unsupported_cli_and_missing_skills_do_not_silently_remove_content() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let image = image_context();
    assert!(
        prepare_managed_input(
            Harness::Grok,
            "Grok gate".to_owned(),
            &[image.clone()],
            Vec::new(),
            &store
        )
        .is_err()
    );
    let skill = skill(directory.path());
    assert!(
        prepare_managed_input(
            Harness::Claude,
            "多个技能".to_owned(),
            &[],
            vec![skill.clone(), skill.clone()],
            &store
        )
        .is_err()
    );
    fs::remove_file(directory.path().join("SKILL.md")).unwrap();
    assert!(
        prepare_managed_input(
            Harness::Codex,
            "缺失技能".to_owned(),
            &[image],
            vec![skill],
            &store
        )
        .is_err()
    );
    assert!(!store.exists());
}

#[test]
fn claude_png_preparation_preserves_multiline_context_and_durable_asset() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let original = image_context();
    let text = "中文图片说明\nEnglish file context: /project/a.rs\nReview: 修正边界";
    let input = prepare_managed_input(
        Harness::Claude,
        text.to_owned(),
        &[original.clone()],
        Vec::new(),
        &store,
    )
    .unwrap();
    assert_eq!(input.len(), 2);
    assert_eq!(input[0], InputContent::Text(text.to_owned()));
    let path = local_image(&input).to_owned();
    let envelope = serde_json::to_string(&super::super::RuntimeAction::Submit { input }).unwrap();
    assert!(!envelope.contains(&original.data));
    let restored = restore_managed_images(vec![path.clone()], &store).unwrap();
    assert_eq!(restored[0].data, original.data);
    assert_eq!(restored[0].mime_type, "image/png");
    assert_eq!(fs::read_dir(store).unwrap().count(), 1);
    assert_eq!(
        fs::read(path).unwrap(),
        STANDARD.decode(original.data).unwrap()
    );
}

#[test]
fn claude_unsupported_image_batches_fail_before_creating_assets() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let original = image_context();
    let mut jpeg_bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut jpeg_bytes, ImageFormat::Jpeg)
        .unwrap();
    let mut jpeg = original.clone();
    jpeg.data = STANDARD.encode(jpeg_bytes.into_inner());
    jpeg.mime_type = "image/jpeg".into();
    assert!(
        prepare_managed_input(
            Harness::Claude,
            "保留全部图片".into(),
            &[original.clone(), jpeg],
            Vec::new(),
            &store,
        )
        .is_err()
    );
    assert!(!store.exists());
    assert!(
        prepare_managed_input(
            Harness::Claude,
            "说明".into(),
            &[original.clone()],
            vec![skill(directory.path())],
            &store,
        )
        .is_err()
    );
    assert!(!store.exists());
    assert!(
        prepare_managed_input(
            Harness::Claude,
            " \n\t".into(),
            &[original.clone()],
            Vec::new(),
            &store,
        )
        .is_err()
    );
    assert!(!store.exists());
    let mut damaged = original.clone();
    damaged.data = "破损base64".into();
    assert!(
        prepare_managed_input(
            Harness::Claude,
            "保留全部图片".into(),
            &[original, damaged],
            Vec::new(),
            &store,
        )
        .is_err()
    );
    assert!(!store.exists());
}

#[test]
fn claude_total_frame_and_text_limits_fail_before_persisting_assets() {
    use rand::{RngCore, SeedableRng, rngs::StdRng};

    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let mut pixels = vec![0u8; 1024 * 1100 * 3];
    StdRng::seed_from_u64(602).fill_bytes(&mut pixels);
    let image = image::RgbImage::from_raw(1024, 1100, pixels).unwrap();
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let bytes = encoded.into_inner();
    assert!(bytes.len() < MAX_IMAGE_SIZE_BYTES);
    let image = ImageContext {
        data: STANDARD.encode(bytes),
        mime_type: "image/png".into(),
        file_name: "两张合格图片.png".into(),
        is_figma: false,
    };
    assert!(
        prepare_managed_input(
            Harness::Claude,
            "总帧预算".into(),
            &[image.clone(), image],
            Vec::new(),
            &store,
        )
        .is_err()
    );
    assert!(!store.exists());
    assert!(
        prepare_managed_input(
            Harness::Claude,
            "中".repeat(1024 * 1024 / 3 + 1),
            &[image_context()],
            Vec::new(),
            &store,
        )
        .is_err()
    );
    assert!(!store.exists());
    let prepared = prepare_managed_input(
        Harness::Claude,
        "中".repeat(1024 * 1024 / 3),
        &[image_context()],
        Vec::new(),
        &store,
    )
    .unwrap();
    assert!(local_image(&prepared).exists());
}

#[test]
fn managed_images_reuse_editor_size_count_and_pixel_limits() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let mut oversized = image_context();
    oversized.data = "A".repeat(MAX_IMAGE_SIZE_BYTES.div_ceil(3) * 4 + 1);
    assert!(
        prepare_managed_input(
            Harness::Codex,
            String::new(),
            &[oversized],
            Vec::new(),
            &store
        )
        .is_err()
    );
    let too_many = vec![image_context(); MAX_IMAGE_COUNT_FOR_QUERY + 1];
    assert!(
        prepare_managed_input(Harness::Codex, String::new(), &too_many, Vec::new(), &store)
            .is_err()
    );
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(1200, 1000)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let mut too_many_pixels = image_context();
    too_many_pixels.data = STANDARD.encode(bytes.into_inner());
    assert!(
        prepare_managed_input(
            Harness::Codex,
            String::new(),
            &[too_many_pixels],
            Vec::new(),
            &store
        )
        .is_err()
    );
    assert!(!store.exists());
}

#[test]
fn concurrent_identical_images_commit_one_complete_asset() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                prepare_managed_input(
                    Harness::Codex,
                    String::new(),
                    &[image_context()],
                    Vec::new(),
                    &store,
                )
                .unwrap()
            })
        })
        .collect();
    let paths: Vec<_> = threads
        .into_iter()
        .map(|thread| local_image(&thread.join().unwrap()).to_owned())
        .collect();
    assert!(paths.iter().all(|path| path == &paths[0]));
    assert_eq!(fs::read_dir(store).unwrap().count(), 1);
    assert_eq!(
        fs::read(&paths[0]).unwrap(),
        STANDARD.decode(image_context().data).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn managed_attachment_symlink_cannot_redirect_or_modify_existing_assets() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let prepared = prepare_managed_input(
        Harness::Codex,
        String::new(),
        &[image_context()],
        Vec::new(),
        &store,
    )
    .unwrap();
    let path = local_image(&prepared);
    let target = directory.path().join("unrelated.png");
    let original = fs::read(path).unwrap();
    fs::write(&target, &original).unwrap();
    fs::remove_file(path).unwrap();
    std::os::unix::fs::symlink(&target, path).unwrap();
    assert!(
        prepare_managed_input(
            Harness::Codex,
            String::new(),
            &[image_context()],
            Vec::new(),
            &store
        )
        .is_err()
    );
    assert_eq!(fs::read(target).unwrap(), original);
}

#[test]
fn durable_images_restore_after_input_is_dropped_without_mutating_assets() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let original = image_context();
    let prepared = prepare_managed_input(
        Harness::Codex,
        "中文草稿\nEnglish".to_owned(),
        &[original.clone()],
        Vec::new(),
        &store,
    )
    .unwrap();
    let path = local_image(&prepared).to_owned();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    drop(prepared);
    let restored = restore_managed_images(vec![path.clone(), path.clone()], &store).unwrap();
    assert_eq!(restored.len(), 2);
    for image in restored {
        assert_eq!(image.data, original.data);
        assert_eq!(image.mime_type, original.mime_type);
        assert_eq!(image.file_name, path.file_name().unwrap().to_str().unwrap());
    }
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read_dir(&store).unwrap().count(), 1);
}

#[test]
fn restoring_images_rejects_other_scope_missing_and_corrupt_assets_as_a_whole() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let prepared = prepare_managed_input(
        Harness::Codex,
        String::new(),
        &[image_context()],
        Vec::new(),
        &store,
    )
    .unwrap();
    let path = local_image(&prepared).to_owned();
    let bytes = fs::read(&path).unwrap();
    let other_store = directory.path().join("other-scope");
    fs::create_dir(&other_store).unwrap();
    let foreign = other_store.join(path.file_name().unwrap());
    fs::write(&foreign, &bytes).unwrap();
    let wrong_hash = store.join(format!("{}.png", "0".repeat(64)));
    fs::write(&wrong_hash, &bytes).unwrap();
    for bad in [foreign, store.join("missing.png"), wrong_hash] {
        assert!(restore_managed_images(vec![path.clone(), bad], &store).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, b"corrupt").unwrap();
    assert!(restore_managed_images(vec![path.clone()], &store).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"corrupt");
}

#[test]
fn restoring_images_validates_decoding_and_limits_even_when_hash_matches() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    fs::create_dir(&store).unwrap();
    let bytes = b"\x89PNG\r\n\x1a\ninvalid image";
    let path = store.join(format!("{:x}.png", Sha256::digest(bytes)));
    fs::write(&path, bytes).unwrap();
    assert!(restore_managed_images(vec![path.clone()], &store).is_err());
    assert!(
        restore_managed_images(vec![path.clone(); MAX_IMAGE_COUNT_FOR_QUERY + 1], &store).is_err()
    );
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_IMAGE_SIZE_BYTES as u64 + 1)
        .unwrap();
    assert!(restore_managed_images(vec![path], &store).is_err());
}

#[cfg(unix)]
#[test]
fn restoring_images_rejects_symlink_assets_and_store() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let prepared = prepare_managed_input(
        Harness::Codex,
        String::new(),
        &[image_context()],
        Vec::new(),
        &store,
    )
    .unwrap();
    let path = local_image(&prepared).to_owned();
    let saved = directory.path().join("saved.png");
    fs::rename(&path, &saved).unwrap();
    std::os::unix::fs::symlink(&saved, &path).unwrap();
    assert!(restore_managed_images(vec![path.clone()], &store).is_err());
    fs::remove_file(&path).unwrap();
    fs::rename(&saved, &path).unwrap();
    let store_link = directory.path().join("store-link");
    std::os::unix::fs::symlink(&store, &store_link).unwrap();
    assert!(
        restore_managed_images(
            vec![store_link.join(path.file_name().unwrap())],
            &store_link
        )
        .is_err()
    );
}

#[test]
fn grok_preserves_text_and_selected_skill_but_rejects_images_before_writing() {
    let directory = TempDir::new().unwrap();
    let store = directory.path().join("local-cli-attachments");
    let text = "中文与 English\n文件上下文：note.txt\n评审意见：保留末尾换行";
    assert_eq!(
        prepare_managed_input(Harness::Grok, text.into(), &[], Vec::new(), &store).unwrap(),
        vec![InputContent::Text(text.into())]
    );
    assert_eq!(
        prepare_managed_input(
            Harness::Grok,
            text.into(),
            &[],
            vec![skill(directory.path())],
            &store
        )
        .unwrap(),
        vec![
            InputContent::Text(text.into()),
            InputContent::Skill {
                name: "local-review".into(),
                path: directory.path().join("SKILL.md")
            }
        ]
    );
    assert!(!store.exists());
    assert!(
        prepare_managed_input(
            Harness::Grok,
            text.into(),
            &[image_context()],
            Vec::new(),
            &store
        )
        .is_err()
    );
    assert!(!store.exists());
}
