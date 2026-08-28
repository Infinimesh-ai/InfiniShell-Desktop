//! 项目级 Agent 上下文加载。
//!
//! 只有从项目入口创建、显式绑定 `project_id` 的会话才会加载项目规则、
//! 仓库映射和主机清单。普通 SSH 会话不会反向推断所属项目。

use std::collections::HashMap;

use infinishell_projects::{Project, ProjectRepository};
use warp_core::features::FeatureFlag;
use warp_ssh_manager::SshRepository;

/// 项目规则注入上限(Unicode 字符),超出截断并追加 [`TRUNCATION_MARKER`]。
pub const RULES_MAX_CHARS: usize = 8_000;
/// 项目备注注入上限(Unicode 字符)。
pub const NOTES_MAX_CHARS: usize = 2_000;
/// 单项目主机清单条数上限,防止巨型项目撑爆 prompt。
pub const HOSTS_MAX_PER_PROJECT: usize = 50;
/// 单项目仓库清单条数上限。
pub const REPOSITORIES_MAX_PER_PROJECT: usize = 50;
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

/// Git 仓库及其有效服务器映射。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRepositoryInfo {
    pub git_url: String,
    pub server_node_ids: Vec<String>,
}

/// 单个项目注入 prompt 的内容,rules/notes 已按上限截断。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectContextEntry {
    pub project_id: String,
    pub name: String,
    pub repositories: Vec<ProjectRepositoryInfo>,
    pub rules: String,
    pub notes: String,
    pub hosts: Vec<ProjectHostInfo>,
}

/// 当前会话命中的项目上下文集合。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAgentContext {
    pub projects: Vec<ProjectContextEntry>,
}

/// 数据库层取出的单项目原始记录,主机引用尚未解析/过滤。
struct ProjectRecord {
    project: Project,
    host_node_ids: Vec<String>,
}

/// 仅使用会话显式绑定的项目，不从终端/SSH 会话推断。
pub fn load_for_conversation(explicit_project_id: Option<&str>) -> Option<ProjectAgentContext> {
    let project_id = explicit_project_id?;
    if !FeatureFlag::InfiniShellProjects.is_enabled() {
        return None;
    }
    load_explicit_project(project_id)
}

fn load_explicit_project(project_id: &str) -> Option<ProjectAgentContext> {
    let record = match infinishell_projects::with_conn(|conn| {
        let Some(project) = ProjectRepository::get(conn, project_id)? else {
            return Ok(None);
        };
        let host_node_ids = ProjectRepository::servers_for_project(conn, project_id)?;
        Ok(Some(ProjectRecord {
            project,
            host_node_ids,
        }))
    }) {
        Ok(Some(record)) => record,
        Ok(None) => return None,
        Err(err) => {
            log::warn!("explicit project context load failed for project {project_id}: {err}");
            return None;
        }
    };

    Some(context_from_record(record, resolve_hosts_from_ssh_manager))
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

fn context_from_record(
    record: ProjectRecord,
    resolve_hosts: impl Fn(&[String]) -> Vec<ProjectHostInfo>,
) -> ProjectAgentContext {
    ProjectAgentContext {
        projects: vec![build_entry(
            record.project,
            resolve_hosts(&record.host_node_ids),
        )],
    }
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
    let known_host_ids = hosts
        .iter()
        .map(|host| host.node_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let repositories = project
        .repositories
        .into_iter()
        .take(REPOSITORIES_MAX_PER_PROJECT)
        .map(|repository| ProjectRepositoryInfo {
            git_url: repository.git_url,
            server_node_ids: repository
                .server_node_ids
                .into_iter()
                .filter(|node_id| known_host_ids.contains(node_id.as_str()))
                .collect(),
        })
        .collect();
    ProjectContextEntry {
        project_id: project.id,
        name: project.name,
        repositories,
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
