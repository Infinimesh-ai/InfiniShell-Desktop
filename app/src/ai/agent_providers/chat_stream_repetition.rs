//! 识别单次响应中长时间重复的完整正文段落，保留原文后终止失控的流。

use std::collections::VecDeque;
use std::time::Duration;

use futures::StreamExt;
use genai::chat::ChatStreamEvent;
use warpui::r#async::FutureExt as _;

use super::ProviderChatEventStream;
use crate::ai::agent_providers::openai_compatible::OpenAiCompatibleError;
use crate::ai::agent_providers::responses::NativeResponseControl;

const MIN_REPETITIONS: usize = 12;
const MIN_REPEATED_BYTES: usize = 2 * 1024;
const MAX_PERIOD: usize = 4;
const MAX_PARAGRAPHS: usize = 256;
const MAX_PARAGRAPH_BYTES: usize = 16 * 1024;

/// 先转发触发阈值的原始分片，再返回不可自动重试的协议错误。
/// 上游流在报错前释放，后续工具分片及 End 中的工具不会进入执行路径。
pub(super) fn guard(
    mut input: ProviderChatEventStream,
    cancel_control: Option<NativeResponseControl>,
) -> ProviderChatEventStream {
    Box::pin(async_stream::stream! {
        let mut detector = RepetitionDetector::default();
        while let Some(event) = input.next().await {
            let repeated = match &event {
                Ok(ChatStreamEvent::Chunk(chunk)) => detector.push(&chunk.content),
                Ok(ChatStreamEvent::ToolCallChunk(_)) => {
                    detector = RepetitionDetector::default();
                    false
                }
                Ok(ChatStreamEvent::Start
                    | ChatStreamEvent::ReasoningChunk(_)
                    | ChatStreamEvent::ThoughtSignatureChunk(_)
                    | ChatStreamEvent::End(_))
                | Err(_) => false,
            };
            if repeated {
                drop(input);
                yield event;
                log::warn!("[byop] stopped response after sustained repeated paragraphs");
                // 后台 Responses 必须显式取消；有界等待避免取消端点失联时继续卡住输入。
                if let Some(control) = cancel_control
                    && !matches!(control.cancel().with_timeout(Duration::from_secs(3)).await, Ok(Ok(())))
                {
                    log::warn!("[byop] background response cancellation failed or timed out");
                }
                yield Err(OpenAiCompatibleError::RepeatedResponse);
                return;
            }
            let failed = event.is_err();
            yield event;
            if failed {
                return;
            }
        }
    })
}

#[derive(Default)]
struct RepetitionDetector {
    line: String,
    paragraph: String,
    paragraph_blocked: bool,
    line_overflowed: bool,
    fence: Option<(u8, usize)>,
    recent: VecDeque<String>,
}

impl RepetitionDetector {
    fn push(&mut self, text: &str) -> bool {
        for character in text.chars() {
            if character == '\n' {
                let line = std::mem::take(&mut self.line);
                if self.line_overflowed {
                    self.line_overflowed = false;
                } else if self.finish_line(line.trim_end_matches('\r')) {
                    return true;
                }
            } else if !self.line_overflowed {
                if self.line.len() + character.len_utf8() > MAX_PARAGRAPH_BYTES {
                    self.block_paragraph();
                    self.line.clear();
                    self.line_overflowed = true;
                } else {
                    self.line.push(character);
                }
            }
        }
        false
    }

    fn finish_line(&mut self, line: &str) -> bool {
        if let Some((marker, count, suffix)) = fence_marker(line) {
            match self.fence {
                Some((open_marker, open_count)) => {
                    if marker == open_marker && count >= open_count && suffix.trim().is_empty() {
                        self.fence = None;
                    }
                }
                None => self.fence = Some((marker, count)),
            }
            self.block_paragraph();
            return false;
        }
        if self.fence.is_some() {
            return false;
        }
        if line.trim().is_empty() {
            if self.paragraph_blocked {
                self.paragraph_blocked = false;
                return false;
            }
            return self.finish_paragraph();
        }
        // 围栏和缩进代码不参与正文重复判断，也不能跨越它们拼成重复周期。
        if line.starts_with("    ") || line.starts_with('\t') {
            self.block_paragraph();
            return false;
        }
        if self.paragraph_blocked {
            return false;
        }
        if self.paragraph.len() + line.len() + 1 > MAX_PARAGRAPH_BYTES {
            self.block_paragraph();
            return false;
        }
        self.paragraph.push_str(line);
        self.paragraph.push('\n');
        false
    }

    fn block_paragraph(&mut self) {
        self.paragraph.clear();
        self.recent.clear();
        self.paragraph_blocked = true;
    }

    fn finish_paragraph(&mut self) -> bool {
        let paragraph = std::mem::take(&mut self.paragraph);
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            return false;
        }
        // 短标签、分隔符和符号行不能触发停止，避免误伤表格与常见输出格式。
        if paragraph
            .chars()
            .filter(|character| character.is_alphanumeric())
            .take(16)
            .count()
            < 16
        {
            self.recent.clear();
            return false;
        }
        self.recent.push_back(paragraph.to_owned());
        if self.recent.len() > MAX_PARAGRAPHS {
            self.recent.pop_front();
        }
        let length = self.recent.len();
        for period in 1..=MAX_PERIOD {
            if length < period * MIN_REPETITIONS {
                continue;
            }
            let mut repetitions = 1;
            while length >= (repetitions + 1) * period {
                let previous_start = length - (repetitions + 1) * period;
                if !(0..period).all(|offset| {
                    self.recent[previous_start + offset] == self.recent[length - period + offset]
                }) {
                    break;
                }
                repetitions += 1;
            }
            let repeated_bytes: usize = self
                .recent
                .iter()
                .skip(length - period)
                .map(String::len)
                .sum();
            if repetitions >= MIN_REPETITIONS && repetitions * repeated_bytes >= MIN_REPEATED_BYTES
            {
                return true;
            }
        }
        false
    }
}

fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = *trimmed.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let count = trimmed.bytes().take_while(|byte| *byte == marker).count();
    (count >= 3).then(|| (marker, count, &trimmed[count..]))
}

#[cfg(test)]
#[path = "chat_stream_repetition_tests.rs"]
mod tests;
