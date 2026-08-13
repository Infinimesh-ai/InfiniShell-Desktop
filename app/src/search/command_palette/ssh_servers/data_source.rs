use fuzzy_match::{FuzzyMatchResult, match_indices_case_insensitive};
use itertools::Itertools;
use warp_ssh_manager::{NodeKind, SshRepository};
use warpui::{AppContext, Entity};

use super::SshServerSearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

/// 上限。SSH 一般几个到几十个,不会爆。
const MAX_SSH_SERVERS_CONSIDERED: usize = 200;

#[derive(Default)]
pub struct SshServersDataSource;

impl SshServersDataSource {
    pub fn new() -> Self {
        Self
    }
}

impl Entity for SshServersDataSource {
    type Event = ();
}

impl SyncDataSource for SshServersDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        // 走自家的 with_conn(独立写连接),不污染 PaneGroup 的主写线程。
        // DataSourceRunErrorWrapper 是 Box<dyn DataSourceRunError> 自定义 trait,
        // 包装成本太高 — 失败时 log + 返回空结果(palette 里不显示 SSH,但其他
        // source 不受影响)。
        let nodes = match warp_ssh_manager::with_conn(|c| Ok(SshRepository::list_nodes(c)?)) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("command palette ssh: failed to load nodes: {e}");
                return Ok(Vec::new());
            }
        };

        // 只展示 server 节点。把每个节点拉一次详情,失败的跳过(folder 没详情会被 None)。
        let server_nodes: Vec<_> = nodes
            .into_iter()
            .filter(|n| matches!(n.kind, NodeKind::Server))
            .take(MAX_SSH_SERVERS_CONSIDERED)
            .collect();

        let query_str = query.text.as_str();
        let results = server_nodes
            .into_iter()
            .filter_map(|node| {
                let server =
                    warp_ssh_manager::with_conn(|c| Ok(SshRepository::get_server(c, &node.id)?))
                        .ok()
                        .flatten()?;

                // 用 name + " " + host 作为搜索文本,name 或 host 任一命中都行。
                let display_name = node.name.clone();
                let host_user = if server.username.is_empty() {
                    server.host.clone()
                } else {
                    format!("{}@{}", server.username, server.host)
                };
                let haystack = format!("{display_name} {host_user}");

                let match_result = if query_str.is_empty() {
                    Some(FuzzyMatchResult::no_match())
                } else {
                    match_indices_case_insensitive(&haystack, query_str)
                }?;

                let mut item = SshServerSearchItem::new(node, server, host_user, display_name);
                let mut mr = match_result;
                // 跟 RepoDataSource 一样略 boost,让 ssh 结果在混合面板里有竞争力。
                mr.score *= 4;
                item.match_result = mr;
                Some(item.into())
            })
            .collect_vec();

        Ok(results)
    }
}
