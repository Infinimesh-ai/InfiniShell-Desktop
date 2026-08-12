//! 项目详情编辑器 pane(中央 pane,通过左侧项目管理器打开)。
//!
//! 逐行仿 `ssh_server_pane.rs` 的极简壳:PaneView 包装 + PaneId + attach +
//! LeafContents 序列化。pane 不持久化(`LeafContents::Project { .. }` 在
//! `is_persisted()` 返回 false),业务数据走 `zap_projects::ProjectRepository`。

use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use crate::{
    app_state::LeafContents,
    pane_group::{
        pane::{ShareableLink, ShareableLinkError},
        BackingView, PaneConfiguration, PaneContent, PaneGroup, PaneView,
    },
    project_manager::project_view::ProjectView,
};

use super::PaneId;

pub struct ProjectPane {
    view: ViewHandle<PaneView<ProjectView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// 业务项目 id(不是 pane view id),用于 snapshot 序列化。
    project_id: String,
}

impl ProjectPane {
    pub fn new<V: View>(project_id: String, ctx: &mut ViewContext<V>) -> Self {
        let id_for_view = project_id.clone();
        let project_view = ctx.add_typed_action_view(move |ctx| ProjectView::new(id_for_view, ctx));
        let pane_configuration = project_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_project_pane_ctx(ctx);
            PaneView::new(pane_id, project_view, (), pane_configuration.clone(), ctx)
        });
        Self {
            view: pane_view,
            pane_configuration,
            project_id,
        }
    }
}

impl PaneContent for ProjectPane {
    fn id(&self) -> PaneId {
        PaneId::from_project_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));
        let child = self.view.as_ref(ctx).child(ctx);

        let pane_id = self.id();
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: super::DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, _ctx: &AppContext) -> LeafContents {
        LeafContents::Project {
            project_id: self.project_id.clone(),
        }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.view
            .as_ref(ctx)
            .child(ctx)
            .update(ctx, BackingView::focus_contents)
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
