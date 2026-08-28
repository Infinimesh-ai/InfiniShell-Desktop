//! 项目领域类型。把 ORM 行结构(`persistence::model::InfiniShellProjectRow`)挡在
//! repository 边界内,对 UI/Agent 层只暴露这里的 plain 数据结构。

use chrono::NaiveDateTime;

/// 一个 Git 仓库及其对应的 SSH 服务器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGitRepository {
    pub id: String,
    pub git_url: String,
    /// `ssh_nodes.id` 列表；顺序与项目详情中的服务器顺序一致。
    pub server_node_ids: Vec<String>,
}

/// 一个项目:聚合 SSH 服务器、Git 仓库、项目规则/习惯。
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// 仓库仅展示 + 注入 Agent 上下文,不做 clone/fetch。
    pub repositories: Vec<ProjectGitRepository>,
    /// 可选本地目录;存在则该目录下的 WARP.md/AGENTS.md 文件规则自动生效。
    pub root_path: Option<String>,
    /// 项目级规则/配置习惯,直接注入 Agent system prompt(注入时截断)。
    pub rules: String,
    pub notes: String,
    /// 项目默认 AIExecutionProfile id(可空)。
    pub default_profile_id: Option<String>,
    pub sort_order: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
