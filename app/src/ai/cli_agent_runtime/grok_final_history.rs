//! 仅从完整原生历史回放提取同一 Grok 回合的最终文本和完成水位。

use std::collections::HashSet;

use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GrokFinalOutcome {
    Completed,
    Cancelled,
}

/// 持久化文本会按 chunkId 合并；辅助 hook 可在正文之后写入较大 eventId。
/// 因此只按 eventId 核对目标正文和完成水位的顺序，同时要求全历史身份唯一。
pub(crate) fn verified_final_snapshot(
    result: &Value,
    session_id: &str,
    turn_id: &str,
    outcome: GrokFinalOutcome,
    max_updates: usize,
    max_output_bytes: usize,
) -> Result<Option<(String, String)>, &'static str> {
    let updates = result["updates"]
        .as_array()
        .ok_or("native history has no updates")?;
    if updates.len() > max_updates
        || result["hasMore"] != false
        || result["totalCount"].as_u64() != Some(updates.len() as u64)
    {
        return Err("native history is truncated or exceeds the verified limit");
    }
    let expected_reason = match outcome {
        GrokFinalOutcome::Completed => "end_turn",
        GrokFinalOutcome::Cancelled => "cancelled",
    };
    let mut output = String::new();
    let mut identities = HashSet::with_capacity(updates.len());
    let mut last_target_sequence = None;
    let mut last_event_id = None;
    let mut watermark = None;
    for record in updates {
        let params = &record["params"];
        if params["sessionId"].as_str() != Some(session_id) {
            return Err("native history changed the session id");
        }
        let event_id = params["_meta"]["eventId"]
            .as_str()
            .ok_or("native history has no event identity")?;
        let sequence = event_id
            .strip_prefix(session_id)
            .and_then(|id| id.strip_prefix('-'))
            .and_then(|suffix| {
                suffix
                    .parse::<u64>()
                    .ok()
                    .filter(|sequence| sequence.to_string() == suffix)
            })
            .ok_or("native history has an invalid event identity")?;
        if !identities.insert(sequence) {
            return Err("native history repeats an event identity");
        }
        last_event_id = Some(event_id);
        let update = &params["update"];
        if record["method"] == "session/update"
            && update["sessionUpdate"] == "agent_message_chunk"
            && params["_meta"]["promptId"].as_str() == Some(turn_id)
        {
            if watermark.is_some() {
                return Err("native history contains text after its completion watermark");
            }
            if last_target_sequence.is_some_and(|previous| previous >= sequence) {
                return Err("native history target output order is inconsistent");
            }
            last_target_sequence = Some(sequence);
            let text = update["content"]["text"]
                .as_str()
                .filter(|_| update["content"]["type"] == "text")
                .ok_or("native history contains unsupported output")?;
            if output.len().saturating_add(text.len()) > max_output_bytes {
                return Err("native history output exceeds the verified limit");
            }
            output.push_str(text);
        }
        if record["method"] == "_x.ai/session/update"
            && update["sessionUpdate"] == "turn_completed"
            && update["prompt_id"].as_str() == Some(turn_id)
        {
            if watermark.is_some() || update["stop_reason"].as_str() != Some(expected_reason) {
                return Err("native history completion does not match the RPC result");
            }
            if last_target_sequence.is_some_and(|previous| previous >= sequence) {
                return Err("native history completion precedes target output");
            }
            last_target_sequence = Some(sequence);
            watermark = Some(event_id.to_owned());
        }
    }
    if result["lastEventId"].as_str() != last_event_id {
        return Err("native history last event identity is inconsistent");
    }
    Ok(watermark.map(|watermark| (output, watermark)))
}

#[cfg(test)]
#[path = "grok_final_history_tests.rs"]
mod tests;
