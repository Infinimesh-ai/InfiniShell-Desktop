use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddSingletonModel, App, SingletonEntity, TypedActionView, ViewHandle};

use super::{DriveIndex, DriveIndexAction};
use crate::ASSETS;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::model::view::ObjectStoreViewModel;
use crate::cloud_object::update_manager::UpdateManager;
use crate::cloud_object::{ObjectType, Owner, Space, StoredObjectSyncStatus};
use crate::drive::ObjectTypeAndId;
use crate::drive::items::WarpDriveItemId;
use crate::menu::MenuItem;
use crate::network::NetworkStatus;
use crate::notebooks::{NotebookObject, NotebookObjectModel};
use crate::server::ids::{ClientId, SyncId};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowObject, WorkflowObjectModel};
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(ObjectStoreViewModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

fn create_index(app: &mut App) -> ViewHandle<DriveIndex> {
    let (_, index) = app.add_window(WindowStyle::NotStealFocus, DriveIndex::new);
    index
}

fn create_workflow(app: &mut App) -> SyncId {
    ObjectStoreModel::handle(app).update(app, |object_store_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let workflow = Workflow::new("my workflow", "my command");
        object_store_model.create_object(
            sync_id,
            WorkflowObject::new_local(
                WorkflowObjectModel::new(workflow),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        sync_id
    })
}

fn create_notebook(app: &mut App) -> SyncId {
    ObjectStoreModel::handle(app).update(app, |object_store_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        object_store_model.create_object(
            sync_id,
            NotebookObject::new_local(
                NotebookObjectModel::default(),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        sync_id
    })
}

fn set_object_in_error(app: &mut App, object_type_and_id: &ObjectTypeAndId) {
    ObjectStoreModel::handle(app).update(
        app,
        |object_store_model, _ctx: &mut warpui::ModelContext<'_, ObjectStoreModel>| {
            if let Some(object) = object_store_model.get_mut_by_uid(&object_type_and_id.uid()) {
                object.set_pending_content_changes_status(StoredObjectSyncStatus::Errored);
            }
        },
    );
}

fn label_for_menu_item(item: &MenuItem<DriveIndexAction>) -> &str {
    if let MenuItem::Item(item) = item {
        item.label()
    } else {
        panic!("item provided wasn't of type MenuItem::Item")
    }
}

#[test]
fn test_retry_menu_item_visibility() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);
        let index = create_index(&mut app);
        let sync_id = create_workflow(&mut app);
        let object_type_and_id: ObjectTypeAndId =
            ObjectTypeAndId::from_id_and_type(sync_id, ObjectType::Workflow);
        let warp_drive_item_id = WarpDriveItemId::Object(object_type_and_id);

        // by default, it doesn't show up
        index.update(&mut app, |index, ctx| {
            let menu_items = index.menu_items(&Space::Personal, &warp_drive_item_id, ctx);
            assert_eq!(menu_items.len(), 5);
            assert_eq!(label_for_menu_item(&menu_items[0]), "Edit");
            assert_eq!(label_for_menu_item(&menu_items[1]), "Copy workflow text");
            assert_eq!(label_for_menu_item(&menu_items[2]), "Duplicate");
            assert_eq!(label_for_menu_item(&menu_items[3]), "Export");
            assert_eq!(label_for_menu_item(&menu_items[4]), "Trash");
        });

        // when the object is in error, it should show up
        set_object_in_error(&mut app, &object_type_and_id);
        index.update(&mut app, |index, ctx| {
            let menu_items = index.menu_items(&Space::Personal, &warp_drive_item_id, ctx);
            assert_eq!(menu_items.len(), 6);
            assert_eq!(label_for_menu_item(&menu_items[0]), "Retry");
            assert_eq!(label_for_menu_item(&menu_items[1]), "Edit");
            assert_eq!(label_for_menu_item(&menu_items[2]), "Copy workflow text");
            assert_eq!(label_for_menu_item(&menu_items[3]), "Duplicate");
            assert_eq!(label_for_menu_item(&menu_items[4]), "Export");
            assert_eq!(label_for_menu_item(&menu_items[5]), "Trash");
        });

        // but if we're offline, it shouldn't show up
        NetworkStatus::handle(&app).update(&mut app, |network_status, ctx| {
            network_status.reachability_changed(false, ctx);
        });
        index.update(&mut app, |index, ctx| {
            let menu_items = index.menu_items(&Space::Personal, &warp_drive_item_id, ctx);
            assert_eq!(menu_items.len(), 5);
            assert_eq!(label_for_menu_item(&menu_items[0]), "Edit");
            assert_eq!(label_for_menu_item(&menu_items[1]), "Copy workflow text");
            assert_eq!(label_for_menu_item(&menu_items[2]), "Duplicate");
            assert_eq!(label_for_menu_item(&menu_items[3]), "Export");
            assert_eq!(label_for_menu_item(&menu_items[4]), "Trash");
        });
    })
}

#[test]
fn test_retry_menu_item_logic() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);
        let index = create_index(&mut app);
        let sync_id = create_workflow(&mut app);
        let object_type_and_id: ObjectTypeAndId =
            ObjectTypeAndId::from_id_and_type(sync_id, ObjectType::Workflow);

        // Zap(Wave 4):SyncQueue 整删,原本验证 SyncQueue 队列变化的
        // 断言全部变为无意义。跳过留下调用流程本身以验证不报 panic。

        index.update(&mut app, |index, ctx| {
            index.retry_failed_object(&object_type_and_id, ctx);
        });

        // the item is now in flight
        ObjectStoreModel::handle(&app).update(&mut app, |object_store_model, _ctx| {
            if let Some(object) = object_store_model.get_mut_by_uid(&object_type_and_id.uid()) {
                let _ = object;
            }
        });

        // Zap(Wave 4):原验证 SyncQueue 队头是 CreateWorkflow,SyncQueue 整删后不适用。

        // Zap(Wave 4):原验证 SyncQueue 队列长度 + UpdateWorkflow tag,SyncQueue 整删后不适用。
    })
}

#[test]
fn test_warp_drive_navigation_states() {
    use crate::drive::index::DriveIndexAction;
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let index = create_index(&mut app);
        let sync_id = create_notebook(&mut app);
        let object_type_and_id: ObjectTypeAndId =
            ObjectTypeAndId::from_id_and_type(sync_id, ObjectType::Notebook);

        index.read(&app, |index, _| {
            assert_eq!(index.selected, None, "Expect selected to be None");
            assert_eq!(
                index.focused_index,
                Some(0),
                "Expect focused_index to be initialized"
            );
        });

        index.update(&mut app, |index, ctx| {
            index.handle_action(&DriveIndexAction::OpenObject(object_type_and_id), ctx);
        });

        index.read(&app, |index, _| {
            assert_eq!(
                index.selected,
                Some(WarpDriveItemId::Object(object_type_and_id)),
                "Expect selected to have correct value"
            );
        });
    });
}
