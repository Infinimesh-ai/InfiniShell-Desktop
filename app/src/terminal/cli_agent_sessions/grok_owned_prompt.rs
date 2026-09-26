//! 普通 Grok 的图片消息独立编码；历史文本绝不按 JSON 猜测为图片请求。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use warp_cli::agent::Harness;

use super::grok_leader_input::{GrokLeaderInputError, GrokLeaderPrompt};
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::{InputContent, current_state_dir, managed_input};
use crate::persistence::local_cli_tasks::grok_terminal::{INPUT_SUBJECT, RICH_INPUT_SUBJECT};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPrompt {
    version: u32,
    input: Vec<InputContent>,
}

fn attachment_store() -> PathBuf {
    current_state_dir().join("local-cli-attachments")
}

/// 原始 PNG 与消息同生命周期保存；面板关闭不会删图，也不会触发消息重发。
/// 图片校验和文件持久化必须在后台执行。
pub(crate) fn prepare(text: String, images: &[ImageContext]) -> Result<String, String> {
    let input = managed_input::prepare_managed_input(
        Harness::Grok,
        text,
        images,
        Vec::new(),
        &attachment_store(),
    )?;
    let body = serde_json::to_string(&PersistedPrompt { version: 1, input })
        .map_err(|_| crate::t!("cli-agent-input-image-delivery-failed"))?;
    // 整批图片和实际双层 JSON 帧先过大小门禁，再允许创建持久消息。
    decode(RICH_INPUT_SUBJECT, &body).map_err(|error| {
        if matches!(error, GrokLeaderInputError::FrameTooLarge) {
            crate::t!("cli-agent-grok-owned-image-budget")
        } else {
            crate::t!("cli-agent-input-image-delivery-failed")
        }
    })?;
    Ok(body)
}

pub(crate) fn decode(subject: &str, body: &str) -> Result<GrokLeaderPrompt, GrokLeaderInputError> {
    if subject == INPUT_SUBJECT {
        return GrokLeaderPrompt::text(body);
    }
    if subject != RICH_INPUT_SUBJECT || body.len() > 1024 * 1024 {
        return Err(GrokLeaderInputError::Protocol);
    }
    let saved: PersistedPrompt =
        serde_json::from_str(body).map_err(|_| GrokLeaderInputError::Protocol)?;
    if saved.version != 1 {
        return Err(GrokLeaderInputError::Protocol);
    }
    let mut text = None;
    let mut paths = Vec::new();
    for part in saved.input {
        match part {
            InputContent::Text(value) if text.is_none() && paths.is_empty() => text = Some(value),
            InputContent::LocalImage(path) => paths.push(path),
            InputContent::Text(_) | InputContent::Skill { .. } => {
                return Err(GrokLeaderInputError::Protocol);
            }
        }
    }
    if paths.is_empty() {
        return Err(GrokLeaderInputError::Protocol);
    }
    // 重新读取时核对当前 scope、普通文件、内容哈希和 PNG，替换后的文件不能冒用旧消息。
    let images = managed_input::restore_managed_images(paths, &attachment_store())
        .map_err(|_| GrokLeaderInputError::Protocol)?;
    GrokLeaderPrompt::with_png(text, images)
}
