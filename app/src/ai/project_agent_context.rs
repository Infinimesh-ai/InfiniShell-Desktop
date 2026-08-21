//! 项目级 Agent 上下文加载 — 会话推断(session inference)链路。
//!
//! 与 `machine_memory` 同款决策:legacy SSH 会话解析出 host+port 后反查 SSH
//! 管理器节点,再经项目↔服务器关联拿到该主机所属项目,把项目规则/备注/主机
//! 清单注入 system prompt(渲染见 `chat_stream::render_project_context_block`)。
//! 显式会话绑定(会话落库的 `project_id`)只服务历史过滤/入口 UX,不参与
//! 这里的上下文加载。

use std::collections::HashMap;
use std::fmt::Display;

use infinishell_projects::{Project, ProjectRepository};
use warp_core::features::FeatureFlag;
use warp_ssh_manager::SshRepository;

use crate::ai::blocklist::SessionContext;
use crate::terminal::ssh::util::InteractiveSshCommand;

/// 项目规则注入上限(Unicode 字符),超出截断并追加 [`TRUNCATION_MARKER`]。
pub const RULES_MAX_CHARS: usize = 8_000;
/// 项目备注注入上限(Unicode 字符)。
pub const NOTES_MAX_CHARS: usize = 2_000;
/// 单项目主机清单条数上限,防止巨型项目撑爆 prompt。
pub const HOSTS_MAX_PER_PROJECT: usize = 50;
/// 注入项目数上限:一台主机同属多项目时只取排序靠前的若干个。
pub const PROJECTS_MAX: usize = 5;
/// 截断标记,追加在被截断的规则/备注末尾。
const TRUNCATION_MARKER: &str = "…(截断)";

/// 项目关联主机的展示信息(已过滤悬挂引用)。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectHostInfo {
    pub node_id: String,
    /// SSH 树节点名称(用户命名)。
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}

/// 单个项目注入 prompt 的内容,rules/notes 已按上限截断。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectContextEntry {
    pub project_id: String,
    pub name: String,
    pub git_url: Option<String>,
    pub rules: String,
    pub notes: String,
    pub hosts: Vec<ProjectHostInfo>,
}

/// 当前会话命中的项目上下文集合。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAgentContext {
    pub projects: Vec<ProjectContextEntry>,
    /// 会话所在主机对应的 SSH 节点 id(host+port 反查所得)。
    pub current_host_node_id: String,
}

/// 数据库层取出的单项目原始记录,主机引用尚未解析/过滤。
struct ProjectRecord {
    project: Project,
    host_node_ids: Vec<String>,
}

/// 仅为可定位主机的 legacy SSH 会话加载项目上下文。
pub fn load_for_session(session_context: &SessionContext) -> Option<ProjectAgentContext> {
    load_with(
        FeatureFlag::InfiniShellProjects.is_enabled(),
        session_context.is_legacy_ssh(),
        session_context.ssh_connection_info(),
        |host, port| {
            warp_ssh_manager::with_conn(|conn| {
                Ok(SshRepository::find_server_node_by_host_port(
                    conn, host, port,
                )?)
            })
        },
        |node_id| {
            infinishell_projects::with_conn(|conn| {
                let mut records = Vec::new();
                for project_id in ProjectRepository::projects_for_node(conn, node_id)? {
                    let Some(project) = ProjectRepository::get(conn, &project_id)? else {
                        continue;
                    };
                    let host_node_ids = ProjectRepository::servers_for_project(conn, &project_id)?;
                    records.push(ProjectRecord {
                        project,
                        host_node_ids,
                    });
                }
                Ok(records)
            })
        },
        resolve_hosts_from_ssh_manager,
    )
}

/// 从 SSH 管理器解析主机清单;节点/服务器详情缺失(悬挂引用)直接跳过,
/// 数据库错误降级为空清单。
fn resolve_hosts_from_ssh_manager(node_ids: &[String]) -> Vec<ProjectHostInfo> {
    let lookup_result = warp_ssh_manager::with_conn(|conn| {
        let names: HashMap<String, String> = SshRepository::list_nodes(conn)?
            .into_iter()
            .map(|node| (node.id, node.name))
            .collect();
        let mut resolved: HashMap<String, ProjectHostInfo> = HashMap::new();
        for node_id in node_ids {
            let Some(name) = names.get(node_id) else {
                continue;
            };
            let Some(server) = SshRepository::get_server(conn, node_id)? else {
                continue;
            };
            resolved.insert(
                node_id.clone(),
                ProjectHostInfo {
                    node_id: node_id.clone(),
                    name: name.clone(),
                    host: server.host,
                    port: server.port,
                    username: server.username,
                },
            );
        }
        Ok(resolved)
    });
    match lookup_result {
        Ok(resolved) => hosts_from_lookup(node_ids, |node_id| resolved.get(node_id).cloned()),
        Err(err) => {
            log::warn!("project context host resolution failed: {err}");
            Vec::new()
        }
    }
}

/// 可测试的纯核心:gating + 主机定位 + 项目装配。
fn load_with<E>(
    enabled: bool,
    is_legacy_ssh: bool,
    ssh_connection_info: Option<&InteractiveSshCommand>,
    find_node: impl FnOnce(&str, u16) -> Result<Option<String>, E>,
    load_projects: impl FnOnce(&str) -> Result<Vec<ProjectRecord>, E>,
    resolve_hosts: impl Fn(&[String]) -> Vec<ProjectHostInfo>,
) -> Option<ProjectAgentContext>
where
    E: Display,
{
    if !enabled || !is_legacy_ssh {
        return None;
    }

    let info = ssh_connection_info?;
    let (host, port) = normalize_host_port(info)?;
    let node_id = match find_node(&host, port) {
        Ok(Some(node_id)) => node_id,
        Ok(None) => return None,
        Err(err) => {
            log::warn!("project context node lookup failed for {host}:{port}: {err}");
            return None;
        }
    };
    let records = match load_projects(&node_id) {
        Ok(records) => records,
        Err(err) => {
            log::warn!("project context load failed for node {node_id}: {err}");
            return None;
        }
    };

    let projects = records
        .into_iter()
        .take(PROJECTS_MAX)
        .map(|record| build_entry(record.project, resolve_hosts(&record.host_node_ids)))
        .collect::<Vec<_>>();
    if projects.is_empty() {
        return None;
    }

    Some(ProjectAgentContext {
        projects,
        current_host_node_id: node_id,
    })
}

/// 复用 `resolve_machine_key` 的归一化(去 `user@` 前缀、小写、缺省端口 22),
/// 拆回 (host, port) 供节点反查。
fn normalize_host_port(info: &InteractiveSshCommand) -> Option<(String, u16)> {
    let machine_key =
        warp_ssh_manager::resolve_machine_key(info.host.as_deref(), info.port.as_deref())?;
    let (host, port) = machine_key.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    Some((host.to_owned(), port))
}

/// 按 node_ids 原顺序解析主机,查不到(悬挂引用)的直接过滤掉。
fn hosts_from_lookup(
    node_ids: &[String],
    lookup: impl Fn(&str) -> Option<ProjectHostInfo>,
) -> Vec<ProjectHostInfo> {
    node_ids
        .iter()
        .filter_map(|node_id| lookup(node_id))
        .collect()
}

/// 装配单项目条目:rules/notes 截断、主机清单封顶。
fn build_entry(project: Project, mut hosts: Vec<ProjectHostInfo>) -> ProjectContextEntry {
    hosts.truncate(HOSTS_MAX_PER_PROJECT);
    ProjectContextEntry {
        project_id: project.id,
        name: project.name,
        git_url: project.git_url,
        rules: truncate_with_marker(&project.rules, RULES_MAX_CHARS),
        notes: truncate_with_marker(&project.notes, NOTES_MAX_CHARS),
        hosts,
    }
}

/// 按 Unicode 字符截断,超限时追加截断标记。
fn truncate_with_marker(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_owned();
    }
    let mut truncated: String = content.chars().take(max_chars).collect();
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

#[cfg(test)]
#[path = "project_agent_context_tests.rs"]
mod tests;
