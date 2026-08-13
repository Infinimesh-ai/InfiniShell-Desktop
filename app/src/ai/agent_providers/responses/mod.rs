//! OpenAI Responses API 的原生协议、传输和会话状态支持。
//!
//! 这里刻意不复用 `genai::chat::ChatRequest`：Responses 的 item/event 联合、后台任务、
//! token 计数、compaction、WebSocket 和 Conversations 都不是 Chat Completions 的子集。

mod chat_stream;
mod client;
mod state;
mod types;
mod websocket;

pub use chat_stream::*;
pub use client::*;
pub use state::*;
pub use types::*;
pub use websocket::*;
