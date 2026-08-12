//! 全局项目变更广播 — 任何 view 改了项目数据(增/删/改字段/改服务器关联)
//! 后调一次 `notify_changed`,ProjectManagerPanel 与(未来的)项目详情 pane
//! 等订阅者据此 refresh。
//!
//! 跟 `SshTreeChangedNotifier`(`app/src/ssh_manager/notifier.rs`)一个套路:
//! Empty struct + SingletonEntity + 单个 Event 变体。

use warpui::{Entity, GetSingletonModelHandle, SingletonEntity, UpdateModel};

#[derive(Default)]
pub struct ProjectsChangedNotifier {}

impl ProjectsChangedNotifier {
    pub fn new() -> Self {
        Default::default()
    }

    /// 广播一次"项目数据已变"。写路径完成持久化后调用,订阅方重新
    /// `ProjectRepository::list`。泛型约束让 `ViewContext` / `AppContext`
    /// 都能直接传入。
    pub fn notify_changed<C>(ctx: &mut C)
    where
        C: GetSingletonModelHandle + UpdateModel,
    {
        Self::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(ProjectsChangedEvent::ProjectsChanged);
        });
    }
}

#[derive(Clone, Debug)]
pub enum ProjectsChangedEvent {
    /// 项目列表 / 字段 / 服务器关联已变,需要重新 list。
    ProjectsChanged,
}

impl Entity for ProjectsChangedNotifier {
    type Event = ProjectsChangedEvent;
}

impl SingletonEntity for ProjectsChangedNotifier {}
