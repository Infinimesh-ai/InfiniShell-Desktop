//! Shared agent-event stream utilities used by third-party harness bridges.

use anyhow::{Result, anyhow};
use async_trait::async_trait;

mod driver;
mod message_hydrator;

pub(crate) use driver::{
    AgentEventConsumer, AgentEventConsumerControlFlow, AgentEventDriverConfig, AgentEventFilter,
    AgentEventStreamClientEventSource, AgentMessageEventMetadata, run_agent_event_driver,
};
#[cfg(test)]
pub(crate) use driver::{
    AgentEventDriverState, AgentEventSource, AgentEventSourceItem,
    DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS,
    DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS, agent_event_backoff,
    agent_event_failures_exceeded_threshold,
};
pub(crate) use message_hydrator::MessageHydrator;

/// 本地 agent 事件流入口。Zap 保留接口以支持本地 driver 注入,默认实现禁用云端 RTC。
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub(crate) trait AgentEventStreamClient: 'static + Send + Sync {
    async fn stream_agent_events(
        &self,
        run_ids: &[String],
        since_sequence: i64,
    ) -> Result<http_client::EventSourceStream>;
}

pub(crate) struct DisabledAgentEventStreamClient;

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventStreamClient for DisabledAgentEventStreamClient {
    async fn stream_agent_events(
        &self,
        _run_ids: &[String],
        _since_sequence: i64,
    ) -> Result<http_client::EventSourceStream> {
        Err(anyhow!(
            "Agent event stream disabled in InfiniShell - RTC endpoint is removed"
        ))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentRunEvent {
    pub event_type: String,
    pub run_id: String,
    pub ref_id: Option<String>,
    pub execution_id: Option<String>,
    pub occurred_at: String,
    pub sequence: i64,
}

#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod message_hydrator_tests;
