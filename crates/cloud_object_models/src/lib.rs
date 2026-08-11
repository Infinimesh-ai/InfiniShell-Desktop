//! This crate defines the concrete Warp cloud object models and typed cloud object aliases built
//! on top of `cloud_objects`.
//!
//! Each model module should own the model payload for one cloud object family, plus any model-specific
//! adapters that should move with that model during future verticalization.
//!
//! Zap 剥离了云端同步链路,因此这里不再包含服务端 GraphQL 适配层
//! (`server_cloud_object`/`user_profile`)与 SQLite 持久化适配层;本地持久化由
//! `app/src/persistence` 自行负责。

// 各模型模块通过 glob 重导出到 crate 根,允许同名项被后续模块覆盖。
#![allow(ambiguous_glob_reexports)]

pub mod ai_execution_profile;
pub mod ai_fact;
pub mod cloud_agent_config;
pub mod cloud_environment;
pub mod env_vars;
pub mod folder;
pub mod json_model;
pub mod mcp;
pub mod notebook;
pub mod preference;
pub mod scheduled_ambient_agent;
pub mod workflow;
pub mod workflow_enum;

pub use ai_execution_profile::*;
pub use ai_fact::*;
pub use cloud_agent_config::*;
pub use cloud_environment::*;
pub use env_vars::*;
pub use folder::*;
pub use json_model::*;
pub use mcp::*;
pub use notebook::*;
pub use preference::*;
pub use scheduled_ambient_agent::*;
pub use workflow::*;
pub use workflow_enum::*;
