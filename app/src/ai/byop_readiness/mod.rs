//! BYOP 请求发送前的工具调用就绪性分类。
//!
//! 这个模块只处理已经投影出来的安全元数据,不读取原始 prompt、工具参数或工具输出,
//! 也不修改 controller、serializer 或 conversation 状态。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const REPAIR_STATE_VERSION: u32 = 1;
pub const BLOCKED_BYOP_REQUEST_MESSAGE: &str =
    "Can't continue this conversation: an earlier tool result is missing or corrupted in this conversation's history, so InfiniShell can't safely send the request to your provider. Start a new conversation or fork from an earlier point to continue.";
pub const PENDING_BYOP_TOOL_RESULTS_MESSAGE: &str =
    "Waiting for a running tool to finish before sending your next request.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadinessTriggerLayer {
    ControllerPreflight,
    SerializerValidation,
}

impl ReadinessTriggerLayer {
    fn as_str(self) -> &'static str {
        match self {
            ReadinessTriggerLayer::ControllerPreflight => "controller_preflight",
            ReadinessTriggerLayer::SerializerValidation => "serializer_validation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessDiagnosticLevel {
    Debug,
    Info,
    Error,
}

impl ReadinessDiagnosticLevel {
    fn as_log_level(self) -> log::Level {
        match self {
            ReadinessDiagnosticLevel::Debug => log::Level::Debug,
            ReadinessDiagnosticLevel::Info => log::Level::Info,
            ReadinessDiagnosticLevel::Error => log::Level::Error,
        }
    }
}

pub struct ReadinessDiagnosticContext<'a> {
    pub conversation_id: &'a str,
    pub request_attempt_id: &'a str,
    pub trigger_layer: ReadinessTriggerLayer,
    pub iteration: Option<usize>,
}

impl<'a> ReadinessDiagnosticContext<'a> {
    pub fn new(
        conversation_id: &'a str,
        request_attempt_id: &'a str,
        trigger_layer: ReadinessTriggerLayer,
    ) -> Self {
        Self {
            conversation_id,
            request_attempt_id,
            trigger_layer,
            iteration: None,
        }
    }

    pub fn with_iteration(mut self, iteration: usize) -> Self {
        self.iteration = Some(iteration);
        self
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{BLOCKED_BYOP_REQUEST_MESSAGE}")]
pub struct BlockedByopReadinessError {
    category: ReadinessCategory,
}

impl BlockedByopReadinessError {
    pub fn new(category: ReadinessCategory) -> Self {
        Self { category }
    }

    pub fn category(&self) -> ReadinessCategory {
        self.category
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{PENDING_BYOP_TOOL_RESULTS_MESSAGE}")]
pub struct PendingByopToolResultsError {
    tool_call_count: usize,
}

impl PendingByopToolResultsError {
    pub fn new(tool_call_count: usize) -> Self {
        Self { tool_call_count }
    }

    pub fn category(&self) -> ReadinessCategory {
        ReadinessCategory::PendingToolResults
    }

    pub fn tool_call_count(&self) -> usize {
        self.tool_call_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallKey {
    pub task_id: String,
    pub assistant_tool_call_message_id: String,
    pub tool_call_id: String,
}

impl ToolCallKey {
    pub fn new(
        task_id: impl Into<String>,
        assistant_tool_call_message_id: impl Into<String>,
        tool_call_id: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            assistant_tool_call_message_id: assistant_tool_call_message_id.into(),
            tool_call_id: tool_call_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RedactedToolKind(String);

impl RedactedToolKind {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RedactedToolKind {
    fn default() -> Self {
        Self::new("unknown")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRef {
    pub key: ToolCallKey,
    pub redacted_tool_kind: RedactedToolKind,
}

impl ToolCallRef {
    pub fn new(key: ToolCallKey, redacted_tool_kind: RedactedToolKind) -> Self {
        Self {
            key,
            redacted_tool_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultRef {
    pub message_id: String,
    pub source: ToolResultSource,
    pub result_kind: TerminalResultKind,
}

impl ToolResultRef {
    fn from_result(result: &ProjectedToolResult) -> Self {
        Self {
            message_id: result.message_id.clone(),
            source: result.source,
            result_kind: result.result_kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolResultSource {
    PersistedHistory,
    CurrentInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalResultKind {
    Real,
    Cancellation,
    StructuredError,
    LocalInterception,
    Compacted,
    UnreadableLocalInterception,
}

impl TerminalResultKind {
    fn satisfies_readiness(self) -> bool {
        match self {
            TerminalResultKind::Real
            | TerminalResultKind::Cancellation
            | TerminalResultKind::StructuredError
            | TerminalResultKind::LocalInterception
            | TerminalResultKind::Compacted => true,
            TerminalResultKind::UnreadableLocalInterception => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedToolCall {
    pub key: ToolCallKey,
    pub redacted_tool_kind: RedactedToolKind,
}

impl ProjectedToolCall {
    pub fn new(
        task_id: impl Into<String>,
        assistant_tool_call_message_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        redacted_tool_kind: RedactedToolKind,
    ) -> Self {
        Self {
            key: ToolCallKey::new(task_id, assistant_tool_call_message_id, tool_call_id),
            redacted_tool_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedToolResult {
    pub task_id: String,
    pub message_id: String,
    pub assistant_tool_call_message_id: Option<String>,
    pub tool_call_id: String,
    pub redacted_tool_kind: RedactedToolKind,
    pub source: ToolResultSource,
    pub result_kind: TerminalResultKind,
}

impl ProjectedToolResult {
    pub fn new(
        task_id: impl Into<String>,
        message_id: impl Into<String>,
        assistant_tool_call_message_id: Option<String>,
        tool_call_id: impl Into<String>,
        redacted_tool_kind: RedactedToolKind,
        source: ToolResultSource,
        result_kind: TerminalResultKind,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            message_id: message_id.into(),
            assistant_tool_call_message_id,
            tool_call_id: tool_call_id.into(),
            redacted_tool_kind,
            source,
            result_kind,
        }
    }

    pub fn current_input(
        index: usize,
        task_id: impl Into<String>,
        assistant_tool_call_message_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        redacted_tool_kind: RedactedToolKind,
        result_kind: TerminalResultKind,
    ) -> Self {
        let task_id = task_id.into();
        let assistant_tool_call_message_id = assistant_tool_call_message_id.into();
        let tool_call_id = tool_call_id.into();
        Self {
            task_id,
            message_id: format!("current_input:{index}:{tool_call_id}"),
            assistant_tool_call_message_id: Some(assistant_tool_call_message_id),
            tool_call_id,
            redacted_tool_kind,
            source: ToolResultSource::CurrentInput,
            result_kind,
        }
    }

    fn key(&self) -> Option<ToolCallKey> {
        self.assistant_tool_call_message_id
            .as_ref()
            .map(|assistant_tool_call_message_id| {
                ToolCallKey::new(
                    self.task_id.clone(),
                    assistant_tool_call_message_id.clone(),
                    self.tool_call_id.clone(),
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionItemKind {
    UserBoundary,
    AssistantBoundary,
    SystemBoundary,
    OtherBoundary,
    AssistantToolCalls(Vec<ProjectedToolCall>),
    ToolResult(ProjectedToolResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionItem {
    pub task_id: String,
    pub message_id: String,
    pub kind: ProjectionItemKind,
}

impl ProjectionItem {
    pub fn user_boundary(task_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            message_id: message_id.into(),
            kind: ProjectionItemKind::UserBoundary,
        }
    }

    pub fn assistant_boundary(task_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            message_id: message_id.into(),
            kind: ProjectionItemKind::AssistantBoundary,
        }
    }

    pub fn system_boundary(task_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            message_id: message_id.into(),
            kind: ProjectionItemKind::SystemBoundary,
        }
    }

    pub fn other_boundary(task_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            message_id: message_id.into(),
            kind: ProjectionItemKind::OtherBoundary,
        }
    }

    pub fn assistant_tool_calls(
        task_id: impl Into<String>,
        message_id: impl Into<String>,
        tool_calls: Vec<ProjectedToolCall>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            message_id: message_id.into(),
            kind: ProjectionItemKind::AssistantToolCalls(tool_calls),
        }
    }

    pub fn tool_result(result: ProjectedToolResult) -> Self {
        Self {
            task_id: result.task_id.clone(),
            message_id: result.message_id.clone(),
            kind: ProjectionItemKind::ToolResult(result),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairSource {
    /// 用户通过 fork(/fork、rewind)从 source conversation 派生新 conversation 时,
    /// 由 `byop_fork_repair_state_json` 为被丢弃的 tool result 记录的修复来源。
    ForkedHistory,
    /// 旧版客户端写入但当前协议无法回放的 tool result 缺口在恢复时记录的修复来源。
    /// 见 `specs/byop-placeholder-tool-results/ISSUES.md` BYOP-PR-6:目前未在恢复路径中产生此 variant,
    /// 老 BYOP conversation 中无法解释的缺口默认按 corrupted history 阻断处理。
    /// **不要随意删除该 variant**:持久化 sidecar 中可能反序列化出 `restored_legacy_history`,
    /// 删除会破坏 Sidecar 格式兼容性。
    RestoredLegacyHistory,
}

impl RepairSource {
    pub fn placeholder_reason(self) -> &'static str {
        match self {
            RepairSource::ForkedHistory => "forked_history_repair",
            RepairSource::RestoredLegacyHistory => "restored_legacy_history_repair",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepairRecord {
    pub source: RepairSource,
    #[serde(flatten)]
    pub key: ToolCallKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_id: Option<String>,
}

impl RepairRecord {
    pub fn new(source: RepairSource, key: ToolCallKey) -> Self {
        Self {
            source,
            key,
            fork_point_message_id: None,
            exchange_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairState {
    pub version: u32,
    #[serde(default)]
    pub records: Vec<RepairRecord>,
}

impl RepairState {
    pub fn new(records: Vec<RepairRecord>) -> Self {
        Self {
            version: REPAIR_STATE_VERSION,
            records,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for RepairState {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepairStateLoadError {
    InvalidJson,
    UnsupportedVersion,
}

#[derive(Clone, PartialEq, Eq)]
pub struct InvalidRepairState {
    pub error_category: RepairStateLoadError,
    raw_json: String,
}

impl std::fmt::Debug for InvalidRepairState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvalidRepairState")
            .field("error_category", &self.error_category)
            .field(
                "raw_json",
                &format_args!("<redacted:{} bytes>", self.raw_json.len()),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairStateStatus {
    Valid(RepairState),
    Invalid(InvalidRepairState),
}

impl RepairStateStatus {
    pub fn from_sidecar_json(sidecar_json: Option<String>) -> Self {
        let Some(raw_json) = sidecar_json else {
            return Self::Valid(RepairState::default());
        };

        let value = match serde_json::from_str::<serde_json::Value>(&raw_json) {
            Ok(value) => value,
            Err(_) => {
                return Self::invalid(RepairStateLoadError::InvalidJson, raw_json);
            }
        };

        let Some(version) = value.get("version").and_then(serde_json::Value::as_u64) else {
            return Self::invalid(RepairStateLoadError::UnsupportedVersion, raw_json);
        };

        if version != u64::from(REPAIR_STATE_VERSION) {
            return Self::invalid(RepairStateLoadError::UnsupportedVersion, raw_json);
        }

        match serde_json::from_value::<RepairState>(value) {
            Ok(state) => Self::Valid(state),
            Err(_) => Self::invalid(RepairStateLoadError::InvalidJson, raw_json),
        }
    }

    pub fn repair_records(&self) -> &[RepairRecord] {
        match self {
            Self::Valid(state) => &state.records,
            Self::Invalid(_) => &[],
        }
    }

    pub fn to_sidecar_json(&self) -> Option<String> {
        match self {
            Self::Valid(state) if state.is_empty() => None,
            Self::Valid(state) => serde_json::to_string(state)
                .map_err(|e| {
                    log::error!("[byop-repair] failed to serialize repair state: {e}");
                })
                .ok(),
            Self::Invalid(invalid) => Some(invalid.raw_json.clone()),
        }
    }

    pub fn error_category(&self) -> Option<RepairStateLoadError> {
        match self {
            Self::Valid(_) => None,
            Self::Invalid(invalid) => Some(invalid.error_category),
        }
    }

    fn invalid(error_category: RepairStateLoadError, raw_json: String) -> Self {
        Self::Invalid(InvalidRepairState {
            error_category,
            raw_json,
        })
    }
}

impl Default for RepairStateStatus {
    fn default() -> Self {
        Self::Valid(RepairState::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiveToolCallState {
    Running,
    CancellationRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveToolCall {
    pub tool_call: ToolCallRef,
    pub state: LiveToolCallState,
}

impl LiveToolCall {
    pub fn new(tool_call: ToolCallRef, state: LiveToolCallState) -> Self {
        Self { tool_call, state }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadinessContext {
    pub repair_records: Vec<RepairRecord>,
    pub live_tool_calls: Vec<LiveToolCall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadinessCategory {
    Ready,
    AcceptedHistoryRepair,
    PendingToolResults,
    NeedsCancellationCommit,
    DuplicateToolResults,
    OrphanToolResult,
    OutOfOrderToolResult,
    MissingResultWithoutRepairSource,
    StaleRepairRecordIgnored,
    ReadinessLoopDidNotConverge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissingResultReason {
    NoResult,
    UnreadableLocalInterception,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReadinessDiagnosticKey {
    category: ReadinessCategory,
    conversation_id: String,
    task_id: String,
    assistant_tool_call_message_id: String,
    tool_call_id: String,
    trigger_layer: ReadinessTriggerLayer,
}

#[derive(Debug, Clone)]
struct ReadinessDiagnosticTarget {
    task_id: String,
    assistant_tool_call_message_id: String,
    tool_call_id: String,
    redacted_tool_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadinessDiagnosticLogEntry {
    level: ReadinessDiagnosticLevel,
    message: String,
}

impl ReadinessDiagnosticLogEntry {
    fn emit(&self) {
        log::log!(self.level.as_log_level(), "{}", self.message);
    }
}

#[derive(Debug, Default)]
pub struct ReadinessDiagnosticCoalescer {
    suppressed_by_key: HashMap<ReadinessDiagnosticKey, usize>,
}

impl ReadinessDiagnosticCoalescer {
    pub fn log_state(
        &mut self,
        state: &ReadinessState,
        context: &ReadinessDiagnosticContext<'_>,
        level: ReadinessDiagnosticLevel,
    ) {
        for entry in self.log_category_targets(
            state.category(),
            readiness_state_targets(state),
            context,
            level,
        ) {
            entry.emit();
        }
    }

    pub fn log_category(
        &mut self,
        category: ReadinessCategory,
        context: &ReadinessDiagnosticContext<'_>,
        level: ReadinessDiagnosticLevel,
    ) {
        for entry in self.log_category_targets(
            category,
            vec![ReadinessDiagnosticTarget {
                task_id: "unknown".to_owned(),
                assistant_tool_call_message_id: "unknown".to_owned(),
                tool_call_id: "unknown".to_owned(),
                redacted_tool_kind: "unknown".to_owned(),
            }],
            context,
            level,
        ) {
            entry.emit();
        }
    }

    pub fn finish(self, context: &ReadinessDiagnosticContext<'_>, level: ReadinessDiagnosticLevel) {
        for entry in self.finish_entries(context, level) {
            entry.emit();
        }
    }

    #[cfg(test)]
    fn test_log_state_entries(
        &mut self,
        state: &ReadinessState,
        context: &ReadinessDiagnosticContext<'_>,
        level: ReadinessDiagnosticLevel,
    ) -> Vec<ReadinessDiagnosticLogEntry> {
        self.log_category_targets(
            state.category(),
            readiness_state_targets(state),
            context,
            level,
        )
    }

    #[cfg(test)]
    fn test_finish_entries(
        self,
        context: &ReadinessDiagnosticContext<'_>,
        level: ReadinessDiagnosticLevel,
    ) -> Vec<ReadinessDiagnosticLogEntry> {
        self.finish_entries(context, level)
    }

    fn finish_entries(
        self,
        context: &ReadinessDiagnosticContext<'_>,
        level: ReadinessDiagnosticLevel,
    ) -> Vec<ReadinessDiagnosticLogEntry> {
        let mut entries = Vec::new();
        for (key, suppressed_count) in self.suppressed_by_key {
            if suppressed_count == 0 {
                continue;
            }
            let trigger_layer = key.trigger_layer.as_str();
            let request_attempt_id = context.request_attempt_id;
            entries.push(ReadinessDiagnosticLogEntry {
                level,
                message: format!(
                    "[byop-readiness] diagnostic coalesced suppressed_count={suppressed_count} \
                     category={:?} conversation_id={} task_id={} \
                     assistant_tool_call_message_id={} tool_call_id={} \
                     redacted_tool_kind=omitted trigger_layer={trigger_layer} \
                     request_attempt_id={request_attempt_id}",
                    key.category,
                    key.conversation_id,
                    key.task_id,
                    key.assistant_tool_call_message_id,
                    key.tool_call_id,
                ),
            });
        }
        entries
    }

    fn log_category_targets(
        &mut self,
        category: ReadinessCategory,
        targets: Vec<ReadinessDiagnosticTarget>,
        context: &ReadinessDiagnosticContext<'_>,
        level: ReadinessDiagnosticLevel,
    ) -> Vec<ReadinessDiagnosticLogEntry> {
        let mut entries = Vec::new();
        for target in targets {
            let key = ReadinessDiagnosticKey {
                category,
                conversation_id: context.conversation_id.to_owned(),
                task_id: target.task_id.clone(),
                assistant_tool_call_message_id: target.assistant_tool_call_message_id.clone(),
                tool_call_id: target.tool_call_id.clone(),
                trigger_layer: context.trigger_layer,
            };

            match self.suppressed_by_key.entry(key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(0);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    *entry.get_mut() += 1;
                    continue;
                }
            }

            let trigger_layer = context.trigger_layer.as_str();
            let iteration = context
                .iteration
                .map(|iteration| iteration.to_string())
                .unwrap_or_else(|| "none".to_owned());
            let request_attempt_id = context.request_attempt_id;
            entries.push(ReadinessDiagnosticLogEntry {
                level,
                message: format!(
                    "[byop-readiness] diagnostic category={category:?} conversation_id={} \
                     task_id={} assistant_tool_call_message_id={} tool_call_id={} \
                     redacted_tool_kind={} trigger_layer={trigger_layer} \
                     request_attempt_id={request_attempt_id} iteration={iteration}",
                    context.conversation_id,
                    target.task_id,
                    target.assistant_tool_call_message_id,
                    target.tool_call_id,
                    target.redacted_tool_kind,
                ),
            });
        }
        entries
    }
}

fn readiness_state_targets(state: &ReadinessState) -> Vec<ReadinessDiagnosticTarget> {
    match state {
        ReadinessState::Ready => Vec::new(),
        ReadinessState::AcceptedHistoryRepair { repairs } => repairs
            .iter()
            .map(|repair| target_from_tool_call_ref(&repair.tool_call))
            .collect(),
        ReadinessState::PendingToolResults { tool_calls }
        | ReadinessState::NeedsCancellationCommit { tool_calls }
        | ReadinessState::MissingResultWithoutRepairSource {
            tool_calls,
            reason: _,
        } => tool_calls.iter().map(target_from_tool_call_ref).collect(),
        ReadinessState::DuplicateToolResults {
            tool_call,
            results: _,
        } => vec![target_from_tool_call_ref(tool_call)],
        ReadinessState::OrphanToolResult { result }
        | ReadinessState::OutOfOrderToolResult { result } => {
            vec![target_from_tool_result(result)]
        }
    }
}

fn target_from_tool_call_ref(tool_call: &ToolCallRef) -> ReadinessDiagnosticTarget {
    ReadinessDiagnosticTarget {
        task_id: tool_call.key.task_id.clone(),
        assistant_tool_call_message_id: tool_call.key.assistant_tool_call_message_id.clone(),
        tool_call_id: tool_call.key.tool_call_id.clone(),
        redacted_tool_kind: tool_call.redacted_tool_kind.as_str().to_owned(),
    }
}

fn target_from_tool_result(result: &ProjectedToolResult) -> ReadinessDiagnosticTarget {
    ReadinessDiagnosticTarget {
        task_id: result.task_id.clone(),
        assistant_tool_call_message_id: result
            .assistant_tool_call_message_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
        tool_call_id: result.tool_call_id.clone(),
        redacted_tool_kind: result.redacted_tool_kind.as_str().to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedRepair {
    pub record: RepairRecord,
    pub tool_call: ToolCallRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessState {
    Ready,
    AcceptedHistoryRepair {
        repairs: Vec<AcceptedRepair>,
    },
    PendingToolResults {
        tool_calls: Vec<ToolCallRef>,
    },
    NeedsCancellationCommit {
        tool_calls: Vec<ToolCallRef>,
    },
    DuplicateToolResults {
        tool_call: ToolCallRef,
        results: Vec<ToolResultRef>,
    },
    OrphanToolResult {
        result: ProjectedToolResult,
    },
    OutOfOrderToolResult {
        result: ProjectedToolResult,
    },
    MissingResultWithoutRepairSource {
        tool_calls: Vec<ToolCallRef>,
        reason: MissingResultReason,
    },
}

impl ReadinessState {
    pub fn category(&self) -> ReadinessCategory {
        match self {
            ReadinessState::Ready => ReadinessCategory::Ready,
            ReadinessState::AcceptedHistoryRepair { .. } => {
                ReadinessCategory::AcceptedHistoryRepair
            }
            ReadinessState::PendingToolResults { .. } => ReadinessCategory::PendingToolResults,
            ReadinessState::NeedsCancellationCommit { .. } => {
                ReadinessCategory::NeedsCancellationCommit
            }
            ReadinessState::DuplicateToolResults { .. } => ReadinessCategory::DuplicateToolResults,
            ReadinessState::OrphanToolResult { .. } => ReadinessCategory::OrphanToolResult,
            ReadinessState::OutOfOrderToolResult { .. } => ReadinessCategory::OutOfOrderToolResult,
            ReadinessState::MissingResultWithoutRepairSource { .. } => {
                ReadinessCategory::MissingResultWithoutRepairSource
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredRepairRecord {
    pub record: RepairRecord,
    pub category: ReadinessCategory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessReport {
    pub state: ReadinessState,
    pub ignored_repair_records: Vec<IgnoredRepairRecord>,
}

pub fn normalize_projection(mut items: Vec<ProjectionItem>) -> Vec<ProjectionItem> {
    let mut active_group: Option<InferenceGroup> = None;

    for item in &mut items {
        let task_id = item.task_id.clone();
        let message_id = item.message_id.clone();
        match &mut item.kind {
            ProjectionItemKind::AssistantToolCalls(tool_calls) => {
                active_group = Some(InferenceGroup::new(task_id, message_id, tool_calls));
            }
            ProjectionItemKind::ToolResult(result) => {
                if result.assistant_tool_call_message_id.is_none() {
                    if let Some(group) = &active_group {
                        if let Some(assistant_message_id) = group.infer_assistant_message_id(result)
                        {
                            result.assistant_tool_call_message_id = Some(assistant_message_id);
                        }
                    }
                }
            }
            ProjectionItemKind::UserBoundary
            | ProjectionItemKind::AssistantBoundary
            | ProjectionItemKind::SystemBoundary
            | ProjectionItemKind::OtherBoundary => {
                active_group = None;
            }
        }
    }

    items
}

pub fn classify_projection(
    items: &[ProjectionItem],
    context: &ReadinessContext,
) -> ReadinessReport {
    let normalized_items = normalize_projection(items.to_vec());
    let mut classifier = Classifier::new(context);

    for item in normalized_items {
        let state = match item.kind {
            ProjectionItemKind::AssistantToolCalls(tool_calls) => {
                let finished = classifier.finish_active_group(true);
                if finished.is_none() && !tool_calls.is_empty() {
                    classifier.start_group(item.task_id, item.message_id, tool_calls);
                }
                finished
            }
            ProjectionItemKind::ToolResult(result) => classifier.handle_tool_result(result),
            ProjectionItemKind::UserBoundary
            | ProjectionItemKind::AssistantBoundary
            | ProjectionItemKind::SystemBoundary
            | ProjectionItemKind::OtherBoundary => classifier.finish_active_group(true),
        };

        if let Some(state) = state {
            return classifier.report(state);
        }
    }

    if let Some(state) = classifier.finish_active_group(false) {
        return classifier.report(state);
    }

    classifier.ready_report()
}

struct InferenceGroup {
    task_id: String,
    assistant_message_id: String,
    tool_call_ids: Vec<String>,
}

impl InferenceGroup {
    fn new(
        task_id: String,
        assistant_message_id: String,
        tool_calls: &[ProjectedToolCall],
    ) -> Self {
        Self {
            task_id,
            assistant_message_id,
            tool_call_ids: tool_calls
                .iter()
                .map(|tool_call| tool_call.key.tool_call_id.clone())
                .collect(),
        }
    }

    fn infer_assistant_message_id(&self, result: &ProjectedToolResult) -> Option<String> {
        if result.task_id != self.task_id {
            return None;
        }

        let matching_count = self
            .tool_call_ids
            .iter()
            .filter(|tool_call_id| *tool_call_id == &result.tool_call_id)
            .count();

        if matching_count == 1 {
            Some(self.assistant_message_id.clone())
        } else {
            None
        }
    }
}

struct ActiveToolCall {
    tool_call: ToolCallRef,
    results: Vec<ToolResultRef>,
}

impl ActiveToolCall {
    fn new(tool_call: ToolCallRef) -> Self {
        Self {
            tool_call,
            results: Vec::new(),
        }
    }
}

struct ActiveGroup {
    task_id: String,
    assistant_message_id: String,
    calls: Vec<ActiveToolCall>,
}

impl ActiveGroup {
    fn new(
        task_id: String,
        assistant_message_id: String,
        tool_calls: Vec<ProjectedToolCall>,
    ) -> Self {
        Self {
            task_id,
            assistant_message_id,
            calls: tool_calls
                .into_iter()
                .map(|tool_call| {
                    ActiveToolCall::new(ToolCallRef::new(
                        tool_call.key,
                        tool_call.redacted_tool_kind,
                    ))
                })
                .collect(),
        }
    }

    fn matching_call_mut(&mut self, result: &ProjectedToolResult) -> Option<&mut ActiveToolCall> {
        self.calls.iter_mut().find(|call| {
            call.tool_call.key.task_id == result.task_id
                && call.tool_call.key.tool_call_id == result.tool_call_id
                && result
                    .assistant_tool_call_message_id
                    .as_ref()
                    .map(|assistant_message_id| {
                        assistant_message_id == &call.tool_call.key.assistant_tool_call_message_id
                    })
                    .unwrap_or(false)
        })
    }
}

struct Classifier<'a> {
    context: &'a ReadinessContext,
    active_group: Option<ActiveGroup>,
    seen_tool_calls: HashSet<ToolCallKey>,
    seen_task_tool_call_ids: HashSet<(String, String)>,
    satisfied_tool_calls: HashSet<ToolCallKey>,
    accepted_repairs: Vec<AcceptedRepair>,
    deferred_missing_tool_calls: Vec<ToolCallRef>,
    used_repair_indices: HashSet<usize>,
    stale_repair_indices: HashSet<usize>,
}

impl<'a> Classifier<'a> {
    fn new(context: &'a ReadinessContext) -> Self {
        Self {
            context,
            active_group: None,
            seen_tool_calls: HashSet::new(),
            seen_task_tool_call_ids: HashSet::new(),
            satisfied_tool_calls: HashSet::new(),
            accepted_repairs: Vec::new(),
            deferred_missing_tool_calls: Vec::new(),
            used_repair_indices: HashSet::new(),
            stale_repair_indices: HashSet::new(),
        }
    }

    fn start_group(
        &mut self,
        task_id: String,
        assistant_message_id: String,
        tool_calls: Vec<ProjectedToolCall>,
    ) {
        for tool_call in &tool_calls {
            self.seen_tool_calls.insert(tool_call.key.clone());
            self.seen_task_tool_call_ids.insert((
                tool_call.key.task_id.clone(),
                tool_call.key.tool_call_id.clone(),
            ));
        }
        self.active_group = Some(ActiveGroup::new(task_id, assistant_message_id, tool_calls));
    }

    fn handle_tool_result(&mut self, result: ProjectedToolResult) -> Option<ReadinessState> {
        // 把"是否归属当前 active group"的判断与"修改 active_call 状态"分成两步,
        // 先通过不可变借用筛选,失败则直接走 unattached 路径,
        // 通过后再获取可变借用,避免 expect/unwrap 出现在生产路径上。
        match self.active_group.as_ref() {
            Some(active_group)
                if result.task_id == active_group.task_id
                    && result.assistant_tool_call_message_id.as_deref()
                        == Some(active_group.assistant_message_id.as_str()) => {}
            _ => return Some(self.unattached_result_state(result)),
        }

        let active_group = match self.active_group.as_mut() {
            Some(active_group) => active_group,
            None => return Some(self.unattached_result_state(result)),
        };
        let Some(active_call) = active_group.matching_call_mut(&result) else {
            return Some(self.unattached_result_state(result));
        };

        if !result.result_kind.satisfies_readiness() {
            return Some(ReadinessState::MissingResultWithoutRepairSource {
                tool_calls: vec![active_call.tool_call.clone()],
                reason: MissingResultReason::UnreadableLocalInterception,
            });
        }

        self.satisfied_tool_calls
            .insert(active_call.tool_call.key.clone());
        active_call
            .results
            .push(ToolResultRef::from_result(&result));
        if active_call.results.len() > 1 {
            return Some(ReadinessState::DuplicateToolResults {
                tool_call: active_call.tool_call.clone(),
                results: active_call.results.clone(),
            });
        }

        None
    }

    fn finish_active_group(&mut self, defer_unexplained_missing: bool) -> Option<ReadinessState> {
        let active_group = self.active_group.take()?;
        let mut repair_candidates = Vec::new();
        let mut cancellation_needed = Vec::new();
        let mut pending = Vec::new();
        let mut missing = Vec::new();

        for call in active_group.calls {
            if call.results.len() == 1 {
                continue;
            }

            if let Some(live_call) = self.live_call_for_key(&call.tool_call.key) {
                match live_call.state {
                    LiveToolCallState::CancellationRequested => {
                        cancellation_needed.push(call.tool_call);
                        continue;
                    }
                    LiveToolCallState::Running => {
                        pending.push(call.tool_call);
                        continue;
                    }
                }
            }

            if let Some((index, record)) = self.repair_record_for_key(&call.tool_call.key) {
                repair_candidates.push(AcceptedRepair {
                    record: record.clone(),
                    tool_call: call.tool_call,
                });
                self.used_repair_indices.insert(index);
                continue;
            }

            self.mark_stale_repair_records_for_visible_gap(&call.tool_call.key);

            if defer_unexplained_missing {
                self.deferred_missing_tool_calls.push(call.tool_call);
            } else {
                missing.push(call.tool_call);
            }
        }

        if !cancellation_needed.is_empty() {
            return Some(ReadinessState::NeedsCancellationCommit {
                tool_calls: cancellation_needed,
            });
        }

        if !pending.is_empty() {
            return Some(ReadinessState::PendingToolResults {
                tool_calls: pending,
            });
        }

        if !missing.is_empty() {
            return Some(ReadinessState::MissingResultWithoutRepairSource {
                tool_calls: missing,
                reason: MissingResultReason::NoResult,
            });
        }

        self.accepted_repairs.extend(repair_candidates);
        None
    }

    fn unattached_result_state(&self, result: ProjectedToolResult) -> ReadinessState {
        let exact_key_seen = result
            .key()
            .map(|key| self.seen_tool_calls.contains(&key))
            .unwrap_or(false);
        let task_tool_call_seen = self
            .seen_task_tool_call_ids
            .contains(&(result.task_id.clone(), result.tool_call_id.clone()));

        if exact_key_seen || task_tool_call_seen {
            ReadinessState::OutOfOrderToolResult { result }
        } else {
            ReadinessState::OrphanToolResult { result }
        }
    }

    fn live_call_for_key(&self, key: &ToolCallKey) -> Option<&LiveToolCall> {
        self.context
            .live_tool_calls
            .iter()
            .find(|live_call| &live_call.tool_call.key == key)
    }

    fn repair_record_for_key(&self, key: &ToolCallKey) -> Option<(usize, &RepairRecord)> {
        self.context
            .repair_records
            .iter()
            .enumerate()
            .find(|(_, record)| &record.key == key)
    }

    fn mark_stale_repair_records_for_visible_gap(&mut self, key: &ToolCallKey) {
        // Repair record 的授权语义要求 task_id + assistant_tool_call_message_id + tool_call_id
        // 三字段完全相等;但当出现真实可见 gap(即同 `tool_call_id` 已被当前对话历史明确标记缺失)时,
        // 任何"同 tool_call_id 但来自其他 (task_id, assistant_message_id)"的 repair record 都不再适用,
        // 标记为 stale 以便上层在 diagnostics 中提示该 record 已被忽略。
        for (index, record) in self.context.repair_records.iter().enumerate() {
            if record.key.tool_call_id == key.tool_call_id && record.key != *key {
                self.stale_repair_indices.insert(index);
            }
        }
    }

    fn ready_report(self) -> ReadinessReport {
        let state = if !self.deferred_missing_tool_calls.is_empty() {
            ReadinessState::MissingResultWithoutRepairSource {
                tool_calls: self.deferred_missing_tool_calls.clone(),
                reason: MissingResultReason::NoResult,
            }
        } else if self.accepted_repairs.is_empty() {
            ReadinessState::Ready
        } else {
            ReadinessState::AcceptedHistoryRepair {
                repairs: self.accepted_repairs.clone(),
            }
        };
        self.report(state)
    }

    fn report(self, state: ReadinessState) -> ReadinessReport {
        let ignored_repair_records = self
            .context
            .repair_records
            .iter()
            .enumerate()
            .filter(|(index, record)| {
                self.stale_repair_indices.contains(index)
                    || (!self.used_repair_indices.contains(index)
                        && self.satisfied_tool_calls.contains(&record.key))
            })
            .map(|(_, record)| IgnoredRepairRecord {
                record: record.clone(),
                category: ReadinessCategory::StaleRepairRecordIgnored,
            })
            .collect();

        ReadinessReport {
            state,
            ignored_repair_records,
        }
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
