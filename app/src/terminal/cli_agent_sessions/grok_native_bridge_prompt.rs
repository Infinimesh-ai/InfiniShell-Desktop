//! 普通 PTY 桥的图片正文使用独立类型和持久引用；不将旧文本解释为 JSON 请求。

use std::io::{self, Cursor};
#[cfg(feature = "local_fs")]
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crc::{CRC_32_ISO_HDLC, Crc};
use image::{ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
#[cfg(feature = "local_fs")]
use warp_cli::agent::Harness;

use crate::ai::agent::ImageContext;
#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::{InputContent, current_state_dir, managed_input};
#[cfg(feature = "local_fs")]
use crate::persistence::local_cli_tasks::grok_native_bridge::{INPUT_SUBJECT, RICH_INPUT_SUBJECT};

const MAX_TEXT_BYTES: usize = 128 * 1024;
const MAX_IMAGES: usize = 20;
const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_PIXELS: u64 = 1_150_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg(feature = "local_fs")]
struct PersistedPrompt {
    version: u32,
    input: Vec<InputContent>,
}

#[derive(Serialize)]
pub(super) struct NativeBridgeImage {
    mime_type: &'static str,
    data: String,
}

/// 只在后台持有实际发送内容；不序列化到消息表，也不能恢复提交租约。
pub(crate) struct NativeBridgePrompt {
    text: String,
    images: Vec<NativeBridgeImage>,
    payload_digest: String,
}

impl NativeBridgePrompt {
    pub(crate) fn text(text: &str) -> io::Result<Self> {
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(invalid());
        }
        Ok(Self {
            text: text.to_owned(),
            images: Vec::new(),
            payload_digest: format!("blake3:{}", blake3::hash(text.as_bytes()).to_hex()),
        })
    }

    pub(super) fn with_png(text: String, images: &[ImageContext]) -> io::Result<Self> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(frame_too_large());
        }
        if images.is_empty() {
            return Err(invalid());
        }
        if images.len() > MAX_IMAGES {
            return Err(frame_too_large());
        }
        let mut digest = blake3::Hasher::new();
        digest.update(b"infinishell-terminal-bridge-png-v1\0");
        digest.update(&(text.len() as u64).to_be_bytes());
        digest.update(text.as_bytes());
        digest.update(&(images.len() as u32).to_be_bytes());
        let mut wire_images = Vec::with_capacity(images.len());
        for image in images {
            // 帧本身只有 256 KiB，先限制编码长度，避免解码一个必然无法投递的大对象。
            if image.data.len() > MAX_FRAME_BYTES {
                return Err(frame_too_large());
            }
            if image.mime_type != "image/png" {
                return Err(invalid());
            }
            let bytes = STANDARD.decode(&image.data).map_err(|_| invalid())?;
            if STANDARD.encode(&bytes) != image.data {
                return Err(invalid());
            }
            validate_png(&bytes)?;
            digest.update(&9_u32.to_be_bytes());
            digest.update(b"image/png");
            digest.update(&(bytes.len() as u64).to_be_bytes());
            digest.update(&bytes);
            wire_images.push(NativeBridgeImage {
                mime_type: "image/png",
                data: image.data.clone(),
            });
        }
        let prompt = Self {
            text,
            images: wire_images,
            payload_digest: format!("blake3:{}", digest.finalize().to_hex()),
        };
        // 此处排除必然超限的正文；领取前还须用实际租约核验整个认证 Envelope。
        if serde_json::to_vec(&(&prompt.text, &prompt.images))
            .map_err(|_| invalid())?
            .len()
            > MAX_FRAME_BYTES
        {
            return Err(frame_too_large());
        }
        Ok(prompt)
    }

    pub(crate) fn payload_digest(&self) -> &str {
        &self.payload_digest
    }

    pub(crate) fn has_images(&self) -> bool {
        !self.images.is_empty()
    }

    pub(super) fn text_value(&self) -> &str {
        &self.text
    }

    pub(super) fn images(&self) -> &[NativeBridgeImage] {
        &self.images
    }
}

fn validate_png(bytes: &[u8]) -> io::Result<()> {
    validate_png_chunks(bytes)?;
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png)
        .into_dimensions()
        .map_err(|_| invalid())?;
    let pixels = u64::from(width) * u64::from(height);
    if width < 8 || height < 8 || !(512..=MAX_PIXELS).contains(&pixels) {
        return Err(invalid());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(MAX_PIXELS * 16);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    reader.limits(limits);
    // 像素解码与所有块的 CRC 都要通过，不能只核尺寸头。
    reader.decode().map_err(|_| invalid())?;
    Ok(())
}

fn validate_png_chunks(bytes: &[u8]) -> io::Result<()> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid());
    }
    let crc = Crc::<u32>::new(&CRC_32_ISO_HDLC);
    let mut offset = 8;
    while let Some(header) = bytes.get(offset..offset + 8) {
        let length = u32::from_be_bytes(header[..4].try_into().map_err(|_| invalid())?) as usize;
        let end = (offset + 8).checked_add(length).ok_or_else(invalid)?;
        let end_with_crc = end.checked_add(4).ok_or_else(invalid)?;
        let expected = bytes.get(end..end_with_crc).ok_or_else(invalid)?;
        let expected = u32::from_be_bytes(expected.try_into().map_err(|_| invalid())?);
        let payload = bytes.get(offset + 4..end).ok_or_else(invalid)?;
        if crc.checksum(payload) != expected {
            return Err(invalid());
        }
        if &header[4..] == b"IEND" {
            return if length == 0 && end_with_crc == bytes.len() {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        offset = end_with_crc;
    }
    Err(invalid())
}

/// 在后台完成全部 PNG 校验后，才将内容寻址附件写入当前数据库的目录。
#[cfg(feature = "local_fs")]
pub(crate) fn prepare(text: String, images: &[ImageContext]) -> Result<String, String> {
    prepare_in_store(
        text,
        images,
        &current_state_dir().join("local-cli-attachments"),
    )
}

#[cfg(feature = "local_fs")]
fn prepare_in_store(text: String, images: &[ImageContext], store: &Path) -> Result<String, String> {
    NativeBridgePrompt::with_png(text.clone(), images).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            crate::t!("cli-agent-grok-owned-image-budget")
        } else {
            crate::t!("cli-agent-input-attachment-invalid")
        }
    })?;
    let input =
        managed_input::prepare_managed_input(Harness::Grok, text, images, Vec::new(), store)?;
    serde_json::to_string(&PersistedPrompt { version: 1, input })
        .map_err(|_| crate::t!("cli-agent-input-image-delivery-failed"))
}

#[cfg(feature = "local_fs")]
pub(crate) fn decode(subject: &str, body: &str) -> io::Result<NativeBridgePrompt> {
    decode_from_store(
        subject,
        body,
        &current_state_dir().join("local-cli-attachments"),
    )
}

/// SQLite 后台线程使用该连接自身的附件目录，不能借另一个进程 scope 的路径复原输入。
#[cfg(feature = "local_fs")]
pub(crate) fn decode_from_store(
    subject: &str,
    body: &str,
    store: &Path,
) -> io::Result<NativeBridgePrompt> {
    if subject == INPUT_SUBJECT {
        return NativeBridgePrompt::text(body);
    }
    if subject != RICH_INPUT_SUBJECT {
        return Err(invalid());
    }
    let (text, paths) = persisted_input(body)?;
    let images = managed_input::restore_managed_images(paths, store).map_err(|_| invalid())?;
    NativeBridgePrompt::with_png(text, &images)
}

/// 已持久领取的历史回执只验证不可变引用正文，不因旧图丢失阻断无关会话查询。
#[cfg(feature = "local_fs")]
pub(crate) fn validate_persisted_body(body: &str) -> io::Result<()> {
    persisted_input(body).map(|_| ())
}

#[cfg(feature = "local_fs")]
fn persisted_input(body: &str) -> io::Result<(String, Vec<PathBuf>)> {
    if body.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let saved: PersistedPrompt = serde_json::from_str(body).map_err(|_| invalid())?;
    if saved.version != 1 {
        return Err(invalid());
    }
    let mut text = None;
    let mut paths = Vec::new();
    for part in saved.input {
        match part {
            InputContent::Text(value) if text.is_none() && paths.is_empty() => text = Some(value),
            InputContent::LocalImage(path) if path.is_absolute() => paths.push(path),
            InputContent::Text(_) | InputContent::LocalImage(_) | InputContent::Skill { .. } => {
                return Err(invalid());
            }
        }
    }
    if paths.is_empty()
        || paths.len() > MAX_IMAGES
        || text
            .as_ref()
            .is_some_and(|text| text.len() > MAX_TEXT_BYTES)
    {
        return Err(invalid());
    }
    Ok((text.unwrap_or_default(), paths))
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "grok_native_bridge.invalid_prompt",
    )
}

pub(super) fn frame_too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "grok_native_bridge.frame_too_large",
    )
}

#[cfg(all(test, feature = "local_fs"))]
#[path = "grok_native_bridge_prompt_tests.rs"]
mod tests;
