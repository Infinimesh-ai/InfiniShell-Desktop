use std::cell::Cell;
use std::collections::HashMap;

use warpui::{AppContext, ModelContext, ModelHandle, SingletonEntity};

use super::BlocklistAIController;
use super::response_stream::{ResponseStream, ResponseStreamId};
use crate::BlocklistAIHistoryModel;
use crate::ai::agent::CancellationReason;
use crate::ai::agent::conversation::AIConversationId;

pub(super) struct PendingResponseStreams {
    streams: HashMap<ResponseStreamId, PendingResponseStream>,
}

struct PendingResponseStream {
    stream: ModelHandle<ResponseStream>,
    // 历史截断或任务异常会移除关联；取消必须仍能找到请求的最近所属会话。
    conversation_id: Cell<AIConversationId>,
}

impl PendingResponseStreams {
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
        }
    }

    pub fn has_active_stream_for_conversation(
        &self,
        conversation_id: AIConversationId,
        app: &AppContext,
    ) -> bool {
        self.streams
            .keys()
            .any(|stream_id| self.conversation_for_stream(stream_id, app) == Some(conversation_id))
    }

    /// Returns the IDs of all in-flight streams owned by the given conversation.
    pub fn stream_ids_for_conversation(
        &self,
        conversation_id: AIConversationId,
        app: &AppContext,
    ) -> Vec<ResponseStreamId> {
        self.streams
            .keys()
            .filter(|stream_id| {
                self.conversation_for_stream(stream_id, app) == Some(conversation_id)
            })
            .cloned()
            .collect()
    }

    pub fn conversation_for_stream(
        &self,
        stream_id: &ResponseStreamId,
        app: &AppContext,
    ) -> Option<AIConversationId> {
        let pending = self.streams.get(stream_id)?;
        // 会话拆分后以最新历史关联为准，同时保存下来供后续取消使用。
        if let Some(conversation_id) =
            BlocklistAIHistoryModel::as_ref(app).conversation_for_response_stream(stream_id)
        {
            pending.conversation_id.set(conversation_id);
        }
        Some(pending.conversation_id.get())
    }

    pub fn register_new_stream(
        &mut self,
        stream_id: ResponseStreamId,
        conversation_id: AIConversationId,
        stream: ModelHandle<ResponseStream>,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) {
        self.try_cancel_streams_for_conversation(conversation_id, reason, ctx);
        self.streams.insert(
            stream_id,
            PendingResponseStream {
                stream,
                conversation_id: Cell::new(conversation_id),
            },
        );
    }

    pub fn cleanup_stream(&mut self, stream_id: &ResponseStreamId) {
        self.streams.remove(stream_id);
    }

    pub fn try_cancel_stream(
        &mut self,
        stream_id: &ResponseStreamId,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) -> bool {
        let conversation_id = self.conversation_for_stream(stream_id, ctx);
        if let Some(pending) = self.streams.remove(stream_id) {
            pending.stream.update(ctx, |stream, ctx| {
                stream.cancel(
                    reason,
                    conversation_id.unwrap_or(pending.conversation_id.get()),
                    ctx,
                )
            });
            return true;
        }
        false
    }

    /// Cancels all streams for the given conversation
    pub fn try_cancel_streams_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) -> bool {
        let streams_to_cancel = self.stream_ids_for_conversation(conversation_id, ctx);
        let did_cancel = !streams_to_cancel.is_empty();
        for stream_id in streams_to_cancel {
            self.try_cancel_stream(&stream_id, reason, ctx);
        }
        did_cancel
    }
}
