//! 项目管理器 UI。M2 part A:项目列表面板(左侧 Tool Panel);part B:项目
//! 详情编辑 pane(中央区,`project_view`);项目级 Agent 对话在 M3 落地。
//!
//! 数据层在独立 crate `zap_projects`(`crates/zap_projects/`)。
//! 入口统一由 `FeatureFlag::ZapProjects` 运行时门控。

pub mod notifier;
pub mod panel;
pub mod project_view;
pub mod session_router;

pub use notifier::{ProjectsChangedEvent, ProjectsChangedNotifier};
pub use panel::ProjectManagerPanel;
// Re-exports for downstream UI consumers(详情 pane / workspace 接线)。
#[allow(unused_imports)]
pub use panel::{ProjectManagerPanelAction, ProjectManagerPanelEvent};
pub use session_router::{ProjectHostRouterEvent, ProjectHostSessionRouter};
