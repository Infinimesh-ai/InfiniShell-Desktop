//! This module contains functions for loading conversation data from the local database.

use std::collections::HashMap;
use std::future::Future;

use chrono::TimeZone;
use futures::FutureExt;
use itertools::Itertools as _;
use persistence::model::AgentConversationRecord;

use super::{
    AIConversationMetadata, BlocklistAIHistoryModel, MAX_HISTORICAL_CONVERSATIONS,
    agent_id_key_from_persisted_data,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ServerAIConversationMetadata,
};
#[cfg(feature = "local_fs")]
use crate::persistence::agent::read_agent_conversation_by_id;
use crate::persistence::model::{
    AgentConversation, AgentConversationData, AgentConversationSummary,
};
use crate::terminal::model::block::SerializedBlock;

/// A conversation transcript from a CLI agent harness (e.g. Claude Code).
#[derive(Debug, Clone)]
pub struct CLIAgentConversation {
    /// Server metadata about this conversation.
    pub metadata: ServerAIConversationMetadata,
    /// A snapshot of the final agent TUI state.
    pub block: SerializedBlock,
}

/// 已加载的本地会话数据表示。
///
/// 具体格式取决于生成该会话的 agent harness。
pub enum LoadedConversationData {
    /// 由 Oz harness 生成、可还原为 [`AIConversation`] 数据模型的会话。
    Oz(Box<AIConversation>),
    /// 由外部 CLI agent harness 生成的会话。
    CLIAgent(Box<CLIAgentConversation>),
}

/// Converts an `AgentConversation` from the database to an `AIConversation`.
/// This utility function extracts the conversion logic that was originally embedded
/// in the terminal view restoration process.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub fn convert_persisted_conversation_to_ai_conversation(
    persisted_conversation: AgentConversation,
) -> Option<AIConversation> {
    convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
}

/// Enhanced version of the conversion function with additional metadata.
/// This version supports the full feature set needed by terminal view restoration.
pub fn convert_persisted_conversation_to_ai_conversation_with_metadata(
    persisted_conversation: AgentConversation,
) -> Option<AIConversation> {
    let AgentConversation {
        tasks,
        conversation:
            AgentConversationRecord {
                conversation_id,
                conversation_data,
                last_modified_at,
                ..
            },
    } = persisted_conversation;

    let conversation_id = match AIConversationId::try_from(conversation_id) {
        Ok(id) => id,
        Err(e) => {
            log::warn!("Failed to convert conversation ID: {e:?}");
            return None;
        }
    };

    let conversation_data = serde_json::from_str::<AgentConversationData>(&conversation_data).ok();

    // 本地 DB 恢复走宽松分支:空的 `agent_tasks` 是「子会话在首个服务端响应前就被持久化」
    // 的正常形态,严格版 `new_restored` 会把它当成畸形输入直接丢弃。
    match AIConversation::new_restored_synthesizing_on_empty(
        conversation_id,
        tasks,
        conversation_data,
    ) {
        Ok(mut conversation) => {
            // 持久化 Task 里的旧消息可能没有 CurrentTime/timestamp,恢复 exchange 时会退到
            // Unix epoch。SQLite 行级更新时间是这个会话最后写入的可靠兜底时间。
            let fallback_timestamp = chrono::Local.from_utc_datetime(&last_modified_at);
            conversation.repair_default_restored_exchange_timestamps(fallback_timestamp);
            Some(conversation)
        }
        Err(e) => {
            log::debug!("Skipping persisted conversation (legacy/incomplete): {e:?}");
            None
        }
    }
}

/// Boxes a future with the right type for the platform.
/// On WASM, futures must not implement Send.
fn box_future<F>(f: F) -> warpui::r#async::BoxFuture<'static, Option<LoadedConversationData>>
where
    F: Future<Output = Option<LoadedConversationData>> + warpui::r#async::Spawnable,
{
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            f.boxed_local()
        } else {
            f.boxed()
        }
    }
}

impl BlocklistAIHistoryModel {
    /// Loads conversation data from memory or the local database.
    ///
    /// This method automatically determines whether to load from memory or local storage:
    /// - If the conversation is already in memory, returns it immediately
    /// - If is_restorable_locally is true, loads from the local database synchronously
    ///
    /// Note: This does NOT insert the conversation into memory. Callers are responsible
    /// for inserting the loaded conversation if needed.
    pub fn load_conversation_data(
        &self,
        conversation_id: AIConversationId,
    ) -> warpui::r#async::BoxFuture<'static, Option<LoadedConversationData>> {
        // First check if the conversation is already in memory
        if let Some(conversation) = self.conversations_by_id.get(&conversation_id) {
            return box_future(futures::future::ready(Some(LoadedConversationData::Oz(
                Box::new(conversation.clone()),
            ))));
        }

        // Check metadata to determine the source
        let Some(metadata) = self
            .all_conversations_metadata
            .get(&conversation_id)
            .cloned()
        else {
            log::warn!("No metadata found for conversation {conversation_id}");
            return box_future(futures::future::ready(None));
        };

        if metadata.is_restorable_locally {
            // Load from local database synchronously
            let result = self
                .load_conversation_from_db(&conversation_id)
                .map(|c| LoadedConversationData::Oz(Box::new(c)));
            box_future(futures::future::ready(result))
        } else {
            log::warn!("Cannot load conversation {conversation_id}: no local data");
            box_future(futures::future::ready(None))
        }
    }

    /// 按历史服务端 token 查找本地已知会话，不发起任何云端请求。
    pub fn load_conversation_by_server_token(
        &self,
        server_token: &ServerConversationToken,
    ) -> warpui::r#async::BoxFuture<'static, Option<LoadedConversationData>> {
        let Some(conversation_id) = self.find_conversation_id_by_server_token(server_token) else {
            return box_future(futures::future::ready(None));
        };
        self.load_conversation_data(conversation_id)
    }

    /// Loads a conversation from local DB and returns it.
    /// This is a private helper method. Use `get_load_conversation_data_future` instead.
    ///
    /// Note: This does NOT insert the conversation into memory. Callers are responsible
    /// for inserting the loaded conversation if needed.
    pub(super) fn load_conversation_from_db(
        &self,
        conversation_id: &AIConversationId,
    ) -> Option<AIConversation> {
        // First check if the conversation is in memory
        if let Some(conversation) = self.conversations_by_id.get(conversation_id) {
            return Some(conversation.clone());
        }

        // If not in memory, try to load from the database
        #[cfg(feature = "local_fs")]
        {
            let persisted_ai_conversation = self.db_connection.clone().and_then(|conn| {
                let mut conn = conn.lock().ok()?;

                let id_str = conversation_id.to_string();
                log::info!("Loading conversation {id_str} from db");
                match read_agent_conversation_by_id(&mut conn, &id_str) {
                    Ok(Some(conv)) => Some(conv),
                    Ok(None) => {
                        log::warn!("No AgentConversation found with id {id_str}");
                        None
                    }
                    Err(e) => {
                        log::warn!("Failed to read AgentConversation {id_str}: {e:?}");
                        None
                    }
                }
            });

            // Convert the persisted conversation to an AIConversation
            if let Some(persisted_conversation) = persisted_ai_conversation {
                if let Some(conversation) =
                    convert_persisted_conversation_to_ai_conversation(persisted_conversation)
                {
                    return Some(conversation);
                }
            }
        }

        None
    }

    /// Initializes historical conversations from restored agent conversations.
    ///
    /// At startup the conversations carry only `agent_conversations` records
    /// (empty task lists) whose summaries were computed at write time (or
    /// derived once at read time); tests may pass fully-hydrated
    /// conversations, whose summaries are derived from their tasks here.
    pub(super) fn initialize_historical_conversations(
        &mut self,
        conversations: &[AgentConversation],
    ) {
        struct HistoricalConversationRow<'a> {
            agent_conversation: &'a AgentConversation,
            conversation_id: AIConversationId,
            conversation_data: Option<AgentConversationData>,
            summary: AgentConversationSummary,
        }

        let historical_rows: Vec<_> = conversations
            .iter()
            .sorted_by_key(|c| c.conversation.last_modified_at)
            .rev()
            .take(MAX_HISTORICAL_CONVERSATIONS)
            .filter_map(|agent_conversation| {
                let conversation_id = match AIConversationId::try_from(
                    agent_conversation.conversation.conversation_id.clone(),
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        log::warn!("Failed to convert conversation ID: {e:?}");
                        return None;
                    }
                };

                // Prefer the write-time summary from the `summary` column;
                // fall back to deriving from tasks for fully-hydrated inputs.
                let summary = agent_conversation
                    .conversation
                    .summary
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<AgentConversationSummary>(json).ok())
                    .unwrap_or_else(|| {
                        AgentConversationSummary::from_tasks(agent_conversation.tasks.iter())
                    });

                if !summary.is_restorable {
                    return None;
                }

                let conversation_data = serde_json::from_str::<AgentConversationData>(
                    &agent_conversation.conversation.conversation_data,
                )
                .ok();

                // Seed the reverse indexes before any child linkage runs so a
                // child row that names its parent by `parent_agent_id` (run_id)
                // can resolve it regardless of row ordering.
                if let Some(data) = conversation_data.as_ref() {
                    if let Some(agent_id) = agent_id_key_from_persisted_data(data) {
                        self.agent_id_to_conversation_id
                            .insert(agent_id.to_owned(), conversation_id);
                    }
                    if let Some(token) = data.server_conversation_token.as_ref() {
                        self.server_token_to_conversation_id
                            .insert(ServerConversationToken::new(token.clone()), conversation_id);
                    }
                }

                Some(HistoricalConversationRow {
                    agent_conversation,
                    conversation_id,
                    conversation_data,
                    summary,
                })
            })
            .collect();

        let collected: HashMap<AIConversationId, AIConversationMetadata> = historical_rows
            .into_iter()
            .filter_map(|row| {
                let HistoricalConversationRow {
                    agent_conversation,
                    conversation_id,
                    conversation_data,
                    summary,
                } = row;

                // Child agent conversations are managed by their parent's
                // status card and should not appear in navigation/history.
                // Record the parent→child mapping before filtering so that
                // create_missing_child_agent_panes can discover children
                // before they are loaded into conversations_by_id.
                if let Some(parent_id) = conversation_data
                    .as_ref()
                    .and_then(|data| self.resolved_parent_conversation_id_from_persisted_data(data))
                {
                    self.index_child_conversation(conversation_id, parent_id);
                    // Eagerly hydrate the child conversation into
                    // `conversations_by_id` so the pill bar and orchestration
                    // transcript name resolution can find it before the
                    // parent's hidden child pane materializes lazily. This is
                    // restricted to orchestration children only — non-child
                    // historical conversations continue to load lazily via
                    // `restore_conversations`. A subsequent `restore_conversations`
                    // call replaces this entry idempotently.
                    //
                    // Startup rows carry no tasks, so the child's task payload
                    // is loaded from the local DB; fully-hydrated inputs
                    // convert directly.
                    let child_conversation = if agent_conversation.tasks.is_empty() {
                        self.load_conversation_from_db(&conversation_id)
                    } else {
                        convert_persisted_conversation_to_ai_conversation_with_metadata(
                            agent_conversation.clone(),
                        )
                    };
                    if let Some(child_conversation) = child_conversation {
                        self.conversations_by_id
                            .insert(conversation_id, child_conversation);
                    } else {
                        log::warn!(
                            "Failed to eagerly hydrate orchestration child {conversation_id}; \
                             pill bar / name resolution will fall back to lazy materialization",
                        );
                    }
                    return None;
                }

                // Skip conversations that only contain passive AutoCodeDiff
                // system queries the user never interacted with (past
                // accepting or rejecting the diff).
                if summary.is_unlisted_auto_code_diff {
                    return None;
                }

                let AgentConversationSummary {
                    initial_query,
                    title,
                    initial_working_directory,
                    ..
                } = summary;

                if initial_query.is_empty() {
                    log::debug!(
                        "Skipping legacy conversation {conversation_id} (no initial query)"
                    );
                    return None;
                }

                let credits_spent = conversation_data
                    .as_ref()
                    .and_then(|data| data.conversation_usage_metadata.as_ref())
                    .map(|m| m.credits_spent);
                let artifacts = conversation_data
                    .as_ref()
                    .and_then(|data| data.artifacts_json.as_ref())
                    .and_then(|json| serde_json::from_str(json).ok())
                    .unwrap_or_default();
                let server_conversation_token = conversation_data
                    .as_ref()
                    .and_then(|data| data.server_conversation_token.as_ref())
                    .map(|token| ServerConversationToken::new(token.clone()));

                Some((
                    conversation_id,
                    AIConversationMetadata {
                        id: conversation_id,
                        title,
                        initial_query,
                        last_modified_at: agent_conversation.conversation.last_modified_at,
                        initial_working_directory,
                        credits_spent,
                        server_conversation_token,
                        is_restorable_locally: true,
                        artifacts,
                        ambient_agent_task_id: None,
                        // 父子链接信息即使父会话本地不可解析也要保留,
                        // 未加载的会话靠它判断是否为子 agent 会话。
                        parent_conversation_id: conversation_data
                            .as_ref()
                            .and_then(|data| data.parent_conversation_id.as_deref())
                            .and_then(|id| AIConversationId::try_from(id.to_owned()).ok()),
                        parent_agent_id: conversation_data
                            .as_ref()
                            .and_then(|data| data.parent_agent_id.clone()),
                    },
                ))
            })
            .collect();
        self.all_conversations_metadata = collected;
    }
}
