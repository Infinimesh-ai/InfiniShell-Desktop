//! Zap:上游这里还有一整套把本地 codebase 向量索引状态
//! (`ai::index::full_source_code_embedding` 的 `CodebaseIndexStatus` /
//! `CodebaseIndexFinishedStatus` / `SyncProgress`)翻译成 proto 的转换函数
//! (`codebase_index_status_to_proto` 及其 state/progress/failure 辅助函数)。
//! codebase 向量索引在本地优先形态下整条链路已下线,`full_source_code_embedding`
//! 模块被物理删除,因此这些转换函数一并移除,只保留不依赖索引类型的
//! 「静态状态」构造函数——远端 server 仍需要用它们回一个
//! NotEnabled / Disabled / Unavailable 的 codebase index 状态。
//! 对应的单元测试(`codebase_index_status_tests.rs`)全部针对已删除的转换逻辑,
//! 故不再挂载。

#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use super::proto::{CodebaseIndexStatus, CodebaseIndexStatusState};

fn current_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(super) fn queued_codebase_index_status(repo_path: String) -> CodebaseIndexStatus {
    base_codebase_index_status(repo_path, CodebaseIndexStatusState::Queued)
}

pub(super) fn not_enabled_codebase_index_status(repo_path: String) -> CodebaseIndexStatus {
    base_codebase_index_status(repo_path, CodebaseIndexStatusState::NotEnabled)
}

pub(super) fn disabled_codebase_index_status(repo_path: String) -> CodebaseIndexStatus {
    base_codebase_index_status(repo_path, CodebaseIndexStatusState::Disabled)
}

pub(super) fn unavailable_codebase_index_status(
    repo_path: String,
    failure_message: String,
) -> CodebaseIndexStatus {
    CodebaseIndexStatus {
        failure_message: Some(failure_message),
        ..base_codebase_index_status(repo_path, CodebaseIndexStatusState::Unavailable)
    }
}

fn base_codebase_index_status(
    repo_path: String,
    state: CodebaseIndexStatusState,
) -> CodebaseIndexStatus {
    CodebaseIndexStatus {
        repo_path,
        state: state.into(),
        last_updated_epoch_millis: Some(current_epoch_millis()),
        progress_completed: None,
        progress_total: None,
        failure_message: None,
        root_hash: None,
    }
}
