//! 项目 (Project) 数据层 — 项目实体 + 项目↔SSH 服务器关联的持久化。
//!
//! 项目聚合:名称、Git 地址、可选本地目录、项目规则/习惯、备注、默认执行
//! profile、关联的 SSH 服务器(引用 `ssh_nodes.id`)。UI 与 Agent 上下文注入
//! 放在 `app/src/project_manager/` 与 `app/src/ai/`,这里保持纯 Rust、无
//! warpui 依赖、可单独 `cargo test` 跑(与 `warp_ssh_manager` 同款分层)。

pub mod db;
pub mod repository;
pub mod types;

pub use db::{set_database_path, with_conn};
pub use repository::{ProjectRepository, ProjectRepositoryError};
pub use types::Project;
