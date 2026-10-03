//! 有界的终端线路挑战，不是 CLI 通知，也不包含用户正文。

use std::{fmt, str};
use uuid::Uuid;
use warp_core::SessionId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TerminalBindingChallenge {
    pub session_id: SessionId,
    pub attempt: Uuid,
    pub nonce: Uuid,
}

impl fmt::Debug for TerminalBindingChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 即使调用方记录整个事件，也不能输出 nonce。
        formatter.write_str("TerminalBindingChallenge(<redacted>)")
    }
}

pub(super) fn parse_terminal_binding_challenge(
    params: &[&[u8]],
) -> Option<TerminalBindingChallenge> {
    if params.len() != 6
        || params.iter().map(|param| param.len()).sum::<usize>() + 5 > 256
        || params[0] != b"9278"
        || params[1] != b"t"
        || params[2] != b"1"
    {
        return None;
    }
    let session_text = str::from_utf8(params[3]).ok()?;
    let session = session_text.parse::<u64>().ok()?;
    if session == 0 || session.to_string() != session_text {
        return None;
    }
    fn parse_id(bytes: &[u8]) -> Option<Uuid> {
        let text = str::from_utf8(bytes).ok()?;
        let id = Uuid::parse_str(text).ok()?;
        (!id.is_nil() && id.to_string() == text).then_some(id)
    }
    Some(TerminalBindingChallenge {
        session_id: SessionId::from(session),
        attempt: parse_id(params[4])?,
        nonce: parse_id(params[5])?,
    })
}

#[cfg(test)]
#[path = "terminal_binding_tests.rs"]
mod tests;
