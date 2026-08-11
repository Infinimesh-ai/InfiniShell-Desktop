//! Shared git "action" orchestration: the commit-chain, push, create-PR, and
//! view-PR workflows behind the code-review git buttons.
//!
//! These compose the single-command primitives in [`crate::util::git`] into the
//! end-to-end actions a button triggers.
//! They are intentionally backend-agnostic: the local code-review dialog and
//! the remote-server daemon both call them, so local and remote behave
//! identically. Git ops are host-scoped and not tied to a diff-state model, so
//! this logic lives here rather than on a model.
//!
//! Callers own everything *around* the action: UI (toasts, telemetry, dialog
//! lifecycle), transport/model (applying the returned delta to a
//! `DiffStateModel`, building wire responses), and any execution-time guards
//! (e.g. the daemon's `git_operation_in_progress` backstop).

use std::path::Path;

// Zap:PR 标题/正文与 commit message 的 AI 生成走的是云端 `server::server_api` 的
// `AIClient` + `ai::generate_code_review_content`,两者均已随云端网关删除。
// 这里的 git 动作退化为纯 git/gh 操作(`gh pr create --fill` 兜底)。
use crate::code_review::diff_state::CommitChainMode;
use crate::util::git::{self, Commit, PrInfo};

/// Runs the commit chain — always commits, then optionally pushes, then
/// optionally creates a PR per `mode` — and returns the post-chain delta
/// (refreshed unpushed commits + upstream ref) plus any created PR. The delta
/// is computed once after the whole chain settles.
///
/// Zap:上游在这里可选地用云端 `AIClient` 生成 PR 标题/正文,该链路已删除,
/// 建 PR 恒定走 `gh pr create --fill`,因此不再接收 `ai_client` 参数。
pub async fn run_commit_chain(
    repo_path: &Path,
    mode: CommitChainMode,
    message: &str,
    include_unstaged: bool,
    branch: &str,
    path_env: Option<&str>,
) -> anyhow::Result<(Vec<Commit>, Option<String>, Option<PrInfo>)> {
    git::run_commit(repo_path, message, include_unstaged, path_env).await?;
    let pr_info = match mode {
        CommitChainMode::CommitOnly => None,
        CommitChainMode::CommitAndPush => {
            git::run_push(repo_path, branch, path_env).await?;
            None
        }
        CommitChainMode::CommitAndCreatePr => {
            git::run_push(repo_path, branch, path_env).await?;
            Some(create_pr(repo_path, branch, path_env).await?)
        }
    };
    let (commits, upstream_ref) = git::compute_unpushed_state(repo_path).await;
    Ok((commits, upstream_ref, pr_info))
}

/// Pushes `branch` (setting upstream) and returns the refreshed
/// unpushed/upstream delta.
pub async fn run_push(
    repo_path: &Path,
    branch: &str,
    path_env: Option<&str>,
) -> anyhow::Result<(Vec<Commit>, Option<String>)> {
    git::run_push(repo_path, branch, path_env).await?;
    Ok(git::compute_unpushed_state(repo_path).await)
}

/// Creates a PR for `branch` with `gh pr create --fill`.
///
/// Zap:上游在此按 `ai_client` 是否存在决定用 AI 生成标题/正文;云端 `AIClient`
/// 已删除,恒定走 `--fill`。
pub async fn create_pr(
    repo_path: &Path,
    branch: &str,
    path_env: Option<&str>,
) -> anyhow::Result<PrInfo> {
    let _ = branch;
    git::create_pr(repo_path, None, None, path_env).await
}

/// Generates an AI commit message for the working-tree changes.
///
/// Zap:commit message 的生成依赖云端 `AIClient::generate_code_review_content`,
/// 该链路已删除。保留函数签名(去掉 `ai_client` 参数)让调用点继续编译,
/// 调用时直接返回错误,由上层把失败透传给用户。
pub async fn generate_commit_message(
    repo_path: &Path,
    branch_name: &str,
    include_unstaged: bool,
) -> anyhow::Result<String> {
    let _ = (repo_path, branch_name, include_unstaged);
    anyhow::bail!("AI commit message generation is not available in InfiniShell")
}
