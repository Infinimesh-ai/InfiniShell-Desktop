// Zap:以下类型上游已抽到 `crates/cloud_objects`,合并时我方保留了一份结构相同的
// 本地副本,导致同名类型在两处定义、跨 crate 传递时触发 E0308/E0277
// (`ObjectIdType` / `ObjectType` 等)。改为直接复用 crate 版本消除分裂;
// 下面 `StoredObject*` 系列是 Zap 自有的命名体系(上游叫 `CloudObject*`),继续本地定义。
#[cfg(test)]
use chrono::Utc;
pub use cloud_objects::cloud_object::{
    GENERIC_STRING_OBJECT_PREFIX, GenericStringObjectFormat, JSON_OBJECT_PREFIX, JsonObjectType,
    NumInFlightRequests, ObjectIdType, ObjectType, Owner, Revision, ServerObjectContainer,
};
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::Element;
use warpui::elements::{
    Align, ChildAnchor, ConstrainedBox, Hoverable, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Stack,
};
use warpui::ui_components::components::UiComponent;

use crate::auth::UserUid;
use crate::drive::sharing::{SharingAccessLevel, Subject};
use crate::server::ids::SyncId;
use crate::server_time::ServerTimestamp;

#[derive(Clone, Debug)]
pub enum StoredObjectSyncStatus {
    NoLocalChanges,
    InFlight(NumInFlightRequests),
    InConflict,
    Errored,
}

const SYNC_ICON_DIMENSIONS: f32 = 16.;
#[derive(Debug, Clone, PartialEq)]
pub struct StoredObjectPermissions {
    pub owner: Owner,
    pub permissions_last_updated_ts: Option<ServerTimestamp>,
    pub anyone_with_link: Option<LinkSharing>,
    pub guests: Vec<StoredObjectGuest>,
}

impl StoredObjectPermissions {
    #[cfg(test)]
    pub fn mock_personal() -> Self {
        Self {
            owner: Owner::mock_current_user(),
            permissions_last_updated_ts: Some(Utc::now().into()),
            guests: Vec::new(),
            anyone_with_link: None,
        }
    }

    pub fn has_direct_user_access(&self, user_uid: UserUid) -> bool {
        self.anyone_with_link.is_some() || self.guests.iter().any(|g| g.subject.is_user(user_uid))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkSharing {
    pub access_level: SharingAccessLevel,
    pub source: Option<ServerObjectContainer>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredObjectGuest {
    pub subject: Subject,
    pub access_level: SharingAccessLevel,
    pub source: Option<ServerObjectContainer>,
}

#[derive(Clone, Debug)]
pub struct StoredObjectMetadata {
    pub revision: Option<Revision>,
    pub metadata_last_updated_ts: Option<ServerTimestamp>,
    pub current_editor_uid: Option<String>,
    pub pending_changes_statuses: StoredObjectStatuses,
    pub trashed_ts: Option<ServerTimestamp>,
    pub folder_id: Option<SyncId>,
    pub is_welcome_object: bool,
    pub last_editor_uid: Option<String>,
    pub creator_uid: Option<String>,
    pub last_task_run_ts: Option<ServerTimestamp>,
}

impl StoredObjectMetadata {
    #[cfg(test)]
    pub fn mock() -> Self {
        Self {
            revision: Some(Revision::now()),
            current_editor_uid: None,
            metadata_last_updated_ts: Some(Utc::now().into()),
            pending_changes_statuses: StoredObjectStatuses::mock(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            last_editor_uid: None,
            creator_uid: None,
            last_task_run_ts: None,
        }
    }

    pub fn has_pending_content_changes(&self) -> bool {
        !matches!(
            self.pending_changes_statuses.content_sync_status,
            StoredObjectSyncStatus::NoLocalChanges | StoredObjectSyncStatus::InConflict
        )
    }

    pub fn is_errored(&self) -> bool {
        matches!(
            self.pending_changes_statuses.content_sync_status,
            StoredObjectSyncStatus::Errored
        )
    }

    pub fn has_pending_online_only_change(&self) -> bool {
        self.pending_changes_statuses.has_pending_permissions_change
            || self.pending_changes_statuses.has_pending_metadata_change
            || self.pending_changes_statuses.pending_untrash
            || self.pending_changes_statuses.pending_delete
    }

    pub fn set_current_editor(&mut self, editor_uid: Option<String>) {
        self.current_editor_uid = editor_uid;
    }
}

#[derive(Clone, Debug)]
pub struct StoredObjectStatuses {
    pub content_sync_status: StoredObjectSyncStatus,
    pub has_pending_permissions_change: bool,
    pub has_pending_metadata_change: bool,
    pub pending_untrash: bool,
    pub pending_delete: bool,
}

impl StoredObjectStatuses {
    #[cfg(test)]
    pub fn mock() -> Self {
        Self {
            content_sync_status: StoredObjectSyncStatus::NoLocalChanges,
            has_pending_permissions_change: false,
            has_pending_metadata_change: false,
            pending_untrash: false,
            pending_delete: false,
        }
    }

    pub fn render_icon(
        &self,
        _sync_queue_is_dequeueing: bool,
        hover_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let should_show_error_indicator = matches!(
            self.content_sync_status,
            StoredObjectSyncStatus::Errored | StoredObjectSyncStatus::InConflict
        );

        let icon_and_tooltip_text = if should_show_error_indicator {
            Some((
                Icon::AlertTriangle.to_warpui_icon(Fill::Solid(theme.ui_error_color())),
                crate::t!("object-sync-failed"),
            ))
        } else {
            None
        };

        if let Some((icon, tooltip_text)) = icon_and_tooltip_text {
            return Some(
                Align::new(
                    Hoverable::new(hover_state, move |hover_state| {
                        let mut stack = Stack::new().with_child(
                            ConstrainedBox::new(icon.finish())
                                .with_height(SYNC_ICON_DIMENSIONS)
                                .with_width(SYNC_ICON_DIMENSIONS)
                                .finish(),
                        );

                        if hover_state.is_hovered() {
                            let tooltip = appearance
                                .ui_builder()
                                .tool_tip(tooltip_text.to_string())
                                .build()
                                .finish();

                            stack.add_positioned_overlay_child(
                                tooltip,
                                OffsetPositioning::offset_from_parent(
                                    vec2f(0., -24.),
                                    ParentOffsetBounds::Unbounded,
                                    ParentAnchor::Center,
                                    ChildAnchor::Center,
                                ),
                            );
                        }

                        stack.finish()
                    })
                    .finish(),
                )
                .finish(),
            );
        }

        None
    }
}

#[derive(Copy, Default, Clone, Debug, Eq, PartialEq)]
pub enum StoredObjectEventEntrypoint {
    TeamSettings,
    ResourceCenter,
    UniversalSearch,
    ManagementUI,
    Blocklist,
    ImportModal,
    Onboarding,
    #[default]
    Unknown,
}
