pub use cloud_object_models::{
    CloudTemplatableMCPServer, CloudTemplatableMCPServerModel, GalleryData, JsonTemplate,
    TemplatableMCPServer, TemplateVariable,
};
use warp_core::ui::appearance::Appearance;
use warpui::{AppContext, SingletonEntity as _};

use crate::cloud_object::model::generic_string_model::{
    GenericStringModel, GenericStringObjectId, StringModel,
};
use crate::cloud_object::model::json_model::{JsonModel, JsonSerializer};
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::{
    GenericStoredObject, GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType,
    UniquePer,
};
use crate::drive::items::WarpDriveItem;
use crate::server::ids::SyncId;

const UNIQUENESS_KEY_PREFIX: &str = "templatable_mcp_server";

pub type TemplatableMCPServerObject =
    GenericStoredObject<GenericStringObjectId, TemplatableMCPServerObjectModel>;
pub type TemplatableMCPServerObjectModel = GenericStringModel<TemplatableMCPServer, JsonSerializer>;

impl TemplatableMCPServerObject {
    pub fn get_all(app: &AppContext) -> Vec<TemplatableMCPServerObject> {
        ObjectStoreModel::as_ref(app)
            .get_all_objects_of_type::<GenericStringObjectId, TemplatableMCPServerObjectModel>()
            .cloned()
            .collect()
    }

    pub fn get_by_id<'a>(
        sync_id: &'a SyncId,
        app: &'a AppContext,
    ) -> Option<&'a TemplatableMCPServerObject> {
        ObjectStoreModel::as_ref(app)
            .get_object_of_type::<GenericStringObjectId, TemplatableMCPServerObjectModel>(sync_id)
    }

    pub fn get_by_uuid<'a>(
        uuid: &'a uuid::Uuid,
        app: &'a AppContext,
    ) -> Option<&'a TemplatableMCPServerObject> {
        ObjectStoreModel::as_ref(app)
            .get_all_objects_of_type::<GenericStringObjectId, TemplatableMCPServerObjectModel>()
            .find(|server| server.model().string_model.uuid == *uuid)
    }
}

// Zap:上游新增的 `CloudObjectUuid` trait 及其 `CloudObjectUuidLookup` 泛型查找
// 未随本地化的 `cloud_object` 模块保留;本类型上方已有等价的 inherent `get_by_uuid`,
// 直接读取 `string_model.uuid`,故这里不需要 trait 实现。

impl StringModel for TemplatableMCPServer {
    type StoredObjectType = TemplatableMCPServerObject;

    fn model_type_name(&self) -> &'static str {
        "MCP server"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer)
    }

    fn should_show_activity_toasts() -> bool {
        true
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        Some(GenericStringObjectUniqueKey {
            key: format!("{UNIQUENESS_KEY_PREFIX}_{}", self.uuid),
            unique_per: UniquePer::User,
        })
    }

    fn renders_in_warp_drive(&self) -> bool {
        false
    }

    fn to_warp_drive_item(
        &self,
        _id: SyncId,
        _appearance: &Appearance,
        _templatable_mcp_server: &TemplatableMCPServerObject,
    ) -> Option<Box<dyn WarpDriveItem>> {
        None
    }
}

impl JsonModel for TemplatableMCPServer {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::TemplatableMCPServer
    }
}
