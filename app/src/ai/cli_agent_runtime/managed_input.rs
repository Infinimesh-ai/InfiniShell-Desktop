//! 托管富输入在后台准备；图片与持久消息同生命周期，不随面板关闭删除。

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use ai::skills::ParsedSkill;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{ImageFormat, ImageReader, Limits};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use warp_cli::agent::Harness;

use super::InputContent;
use super::local_skills::prepare_local_cli_skill_inputs;
use crate::ai::agent::ImageContext;
use crate::util::image::{
    MAX_IMAGE_COUNT_FOR_QUERY, MAX_IMAGE_PIXELS, MAX_IMAGE_SIZE_BYTES, is_supported_image_mime_type,
};

/// 调用方传入已由现有文件、选区和评审 builder 生成的文本，不在这里重写上下文语法。
/// attachment_store 必须是当前 SQLite scope 数据库旁的绝对附件目录；此函数不能在 UI 线程调用。
pub(crate) fn prepare_managed_input(
    harness: Harness,
    text: String,
    images: &[ImageContext],
    skills: Vec<ParsedSkill>,
    attachment_store: &Path,
) -> Result<Vec<InputContent>, String> {
    match harness {
        Harness::Codex | Harness::Claude | Harness::Grok => {}
        Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Unknown => {
            return Err(crate::t!("cli-agent-managed-version-unavailable"));
        }
    }
    if harness == Harness::Grok && !images.is_empty() {
        return Err(crate::t!(
            "cli-agent-input-images-unverified",
            cli = harness.display_name()
        ));
    }
    if harness == Harness::Claude && !images.is_empty() {
        // 当前原生校准仅覆盖 PNG 与文本，不把其他格式或技能混用计作可用能力。
        if images.iter().any(|image| image.mime_type != "image/png") {
            return Err(crate::t!("cli-agent-claude-png-only"));
        }
        if !skills.is_empty() {
            return Err(crate::t!("cli-agent-claude-image-skill-unverified"));
        }
        if text.trim().is_empty() {
            return Err(crate::t!("cli-task-manager-empty-prompt"));
        }
    }
    if images.len() > MAX_IMAGE_COUNT_FOR_QUERY {
        return Err(crate::t!(
            "editor-images-disabled-query-limit",
            limit = MAX_IMAGE_COUNT_FOR_QUERY
        ));
    }
    let skill_inputs = prepare_local_cli_skill_inputs(skills, harness, true)?;
    let validated_images = images
        .iter()
        .map(validate_image)
        .collect::<Result<Vec<_>, _>>()?;
    if harness == Harness::Claude && !images.is_empty() {
        super::claude::verify_prepared_png_budget(&text, images)?;
    }
    if text.is_empty() && images.is_empty() && skill_inputs.is_empty() {
        return Err(crate::t!("cli-task-manager-empty-prompt"));
    }
    // 所有内容先验证再写文件，坏图片或技能不能让输入被部分派发。
    let mut input = Vec::new();
    if !text.is_empty() {
        input.push(InputContent::Text(text));
    }
    if !images.is_empty() {
        prepare_attachment_store(attachment_store).map_err(storage_error)?;
        for image in validated_images {
            let path = persist_image(attachment_store, &image).map_err(storage_error)?;
            input.push(InputContent::LocalImage(path));
        }
    }
    input.extend(skill_inputs);
    Ok(input)
}

/// 从当前数据库 scope 的持久附件还原草稿；在后台调用，失败不返回部分图片。
pub(crate) fn restore_managed_images(
    paths: Vec<PathBuf>,
    attachment_store: &Path,
) -> Result<Vec<ImageContext>, String> {
    if paths.len() > MAX_IMAGE_COUNT_FOR_QUERY {
        return Err(crate::t!(
            "editor-images-disabled-query-limit",
            limit = MAX_IMAGE_COUNT_FOR_QUERY
        ));
    }
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let directory = fs::symlink_metadata(attachment_store).map_err(|_| invalid_image())?;
    if !attachment_store.is_absolute() || !directory.is_dir() || directory.file_type().is_symlink()
    {
        return Err(invalid_image());
    }
    paths
        .into_iter()
        .map(|path| {
            // 仅接受本 scope 直接保存的 hash 文件，不解析用户文件名、跨目录路径或符号链接。
            if !path.is_absolute() || path.parent() != Some(attachment_store) {
                return Err(invalid_image());
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| invalid_image())?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_IMAGE_SIZE_BYTES as u64
            {
                return Err(invalid_image());
            }
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .map_err(|_| invalid_image())?
                .take(MAX_IMAGE_SIZE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid_image())?;
            if bytes.len() > MAX_IMAGE_SIZE_BYTES {
                return Err(crate::t!("editor-image-too-large"));
            }
            let format = image::guess_format(&bytes).map_err(|_| invalid_image())?;
            let extension = format.extensions_str().first().ok_or_else(invalid_image)?;
            let digest = Sha256::digest(&bytes);
            let file_name = format!("{digest:x}.{extension}");
            if path.file_name().and_then(|name| name.to_str()) != Some(file_name.as_str()) {
                return Err(invalid_image());
            }
            let image = ImageContext {
                data: STANDARD.encode(&bytes),
                mime_type: format.to_mime_type().to_owned(),
                file_name,
                is_figma: false,
            };
            validate_image(&image)?;
            Ok(image)
        })
        .collect()
}

struct ValidatedImage {
    bytes: Vec<u8>,
    format: ImageFormat,
}

fn validate_image(image: &ImageContext) -> Result<ValidatedImage, String> {
    // ImageContext 已是编辑器处理后的数据，沿用其大小上限，不用原始剪贴板的 500 MB 上限。
    if image.data.len() > MAX_IMAGE_SIZE_BYTES.div_ceil(3) * 4 {
        return Err(crate::t!("editor-image-too-large"));
    }
    let bytes = STANDARD.decode(&image.data).map_err(|_| invalid_image())?;
    if bytes.len() > MAX_IMAGE_SIZE_BYTES {
        return Err(crate::t!("editor-image-too-large"));
    }
    let format = image::guess_format(&bytes).map_err(|_| invalid_image())?;
    let declared_mime = if image.mime_type == "image/jpg" {
        "image/jpeg"
    } else {
        image.mime_type.as_str()
    };
    if !is_supported_image_mime_type(declared_mime) || format.to_mime_type() != declared_mime {
        return Err(invalid_image());
    }
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|_| invalid_image())?;
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_IMAGE_PIXELS as u64 {
        return Err(crate::t!("editor-image-too-large"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS as u64 * 16);
    reader.limits(limits);
    reader.decode().map_err(|_| invalid_image())?;
    Ok(ValidatedImage { bytes, format })
}

fn prepare_attachment_store(directory: &Path) -> std::io::Result<()> {
    if !directory.is_absolute() {
        return Err(std::io::Error::other("附件目录必须是绝对路径"));
    }
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other("附件目录不能是符号链接或普通文件"));
    }
    Ok(())
}

fn persist_image(directory: &Path, image: &ValidatedImage) -> std::io::Result<PathBuf> {
    let digest = Sha256::digest(&image.bytes);
    let extension = image
        .format
        .extensions_str()
        .first()
        .ok_or_else(|| std::io::Error::other("图片格式缺少扩展名"))?;
    // file_name 仅是展示标签，绝不能将其解释为原图路径或目标路径。
    let path = directory.join(format!("{digest:x}.{extension}"));
    match verify_existing_image(&path, &image.bytes)? {
        true => return Ok(path),
        false => {}
    }
    let mut temporary = NamedTempFile::new_in(directory)?;
    temporary.write_all(&image.bytes)?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(&path) {
        Ok(file) => file.sync_all()?,
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !verify_existing_image(&path, &image.bytes)? {
                return Err(std::io::Error::other("并发写入的附件未找到"));
            }
        }
        Err(error) => return Err(error.error),
    }
    // Unix 还同步目录项；Windows 原子持久化交给 tempfile 的平台实现。
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(path)
}

fn verify_existing_image(path: &Path, expected: &[u8]) -> std::io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != expected.len() as u64
    {
        return Err(std::io::Error::other(
            "已保存附件的类型或大小不匹配，未覆盖原文件",
        ));
    }
    let mut actual = Vec::new();
    fs::File::open(path)?
        .take(MAX_IMAGE_SIZE_BYTES as u64 + 1)
        .read_to_end(&mut actual)?;
    if actual != expected {
        return Err(std::io::Error::other("已保存附件校验失败，未覆盖原文件"));
    }
    Ok(true)
}

fn invalid_image() -> String {
    crate::t!("cli-agent-input-attachment-invalid")
}

fn storage_error(error: std::io::Error) -> String {
    crate::t!(
        "cli-agent-input-attachment-save-failed",
        error = error.to_string()
    )
}

#[cfg(test)]
#[path = "managed_input_tests.rs"]
mod tests;
