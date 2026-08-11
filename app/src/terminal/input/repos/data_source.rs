//! Async data source for the inline repos menu.
//!
//! 历史上这里从 `PersistedWorkspace` 拉「之前打开过的 git 仓库」列表。
//! LSP + workspace 历史下线后,这个候选源已不存在,因此本 data source
//! 仅保留 trait 与 view 接线,永远返回空结果 —— 也就是说菜单仍能被
//! 唤出但永远没有候选项。这样可以避免大改上层 view / suggestions mode
//! 的接线,等未来若要接入「当前 pane group 实时 cwd」再补回数据来源。

#[cfg(feature = "local_fs")]
use std::collections::HashMap;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;
#[cfg(feature = "local_fs")]
use std::sync::{Arc, Mutex};

use warpui::{AppContext, Entity};

use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper};
use crate::terminal::input::repos::AcceptRepo;
#[cfg(feature = "local_fs")]
use crate::util::git::RepoGitSummary;

/// Cache of per-repo git summaries (branch + diff stats) keyed by repo path.
///
/// Shared between the data source, which reads it to render results immediately,
/// and the view, which populates it in the background. This lets the menu show
/// the repo list synchronously while the (relatively expensive) git data is
/// lazily loaded and filled in as it arrives.
#[cfg(feature = "local_fs")]
pub type GitSummaryCache = Arc<Mutex<HashMap<PathBuf, RepoGitSummary>>>;

pub struct RepoMenuDataSource {
    /// Git summaries populated in the background by the view. Reads never block
    /// on git; missing entries simply render without branch/diff-stat suffixes.
    ///
    /// Zap:候选源(`PersistedWorkspace`)已下线,`run_query` 永远返回空结果,
    /// 因此这份缓存暂时无人读取;保留字段是为了不改动 view 侧的接线。
    #[cfg(feature = "local_fs")]
    #[allow(dead_code)]
    git_summaries: GitSummaryCache,
}

impl RepoMenuDataSource {
    #[cfg(feature = "local_fs")]
    pub fn new(git_summaries: GitSummaryCache) -> Self {
        Self { git_summaries }
    }

    #[cfg(not(feature = "local_fs"))]
    pub fn new() -> Self {
        Self
    }
}

impl AsyncDataSource for RepoMenuDataSource {
    type Action = AcceptRepo;

    fn run_query(
        &self,
        _query: &Query,
        _app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

impl Entity for RepoMenuDataSource {
    type Event = ();
}
