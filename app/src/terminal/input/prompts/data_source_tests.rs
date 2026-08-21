//! `#` prompts 菜单的窗口作用域测试。
//!
//! Zap 说明:上游这里用 `ServerApiProvider` / `MockTeamClient` / `MockWorkspaceClient`
//! 播种 team 空间的云端 prompt,断言"只返回当前窗口所属 team 的 prompt"。
//! 这些云端网关(`server::server_api`、`server::sync_queue`、`workspaces::team_tester`)
//! 已在 Zap 中物理删除,`UserWorkspaces::team_from_uid` 恒返回 `None`,
//! `owner_to_space` 把 `Owner::Team` 一律映射到 `Space::Shared`——team 空间在本地
//! 优先分支下不存在。
//!
//! 因此三个 team 作用域断言无法保留,改为覆盖仍然存在的行为:窗口只返回其可见
//! space(Personal)里的 prompt,非本人拥有的对象被过滤掉。播种改走
//! `ObjectStoreModel::add_object`(上游的 `upsert_from_server_workflow` 已随
//! 云端 workflow 同步链路删除)。

use itertools::Itertools;
use settings::manager::SettingsManager;
use warpui::{App, SingletonEntity};

use super::*;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::model::view::ObjectStoreViewModel;
use crate::cloud_object::update_manager::UpdateManager;
use crate::cloud_object::{Owner, Space, StoredObjectMetadata, StoredObjectPermissions};
use crate::network::NetworkStatus;
use crate::notebooks::manager::NotebookManager;
use crate::search::data_source::Query;
use crate::settings::AISettings;
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowId, WorkflowObject, WorkflowObjectModel};
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;

const PERSONAL_PROMPT_ID: i64 = 1;
const FOREIGN_PROMPT_ID: i64 = 2;

fn mock_prompt(id: WorkflowId, owner: Owner, name: &str) -> WorkflowObject {
    WorkflowObject::new(
        SyncId::ServerId(id.into()),
        WorkflowObjectModel::new(Workflow::AgentMode {
            name: name.to_owned(),
            query: format!("do {name}"),
            description: None,
            arguments: Vec::new(),
        }),
        StoredObjectMetadata::mock(),
        StoredObjectPermissions {
            owner,
            guests: Vec::new(),
            anyone_with_link: None,
            permissions_last_updated_ts: None,
        },
    )
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(|ctx| UpdateManager::new(None, ctx));
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(ObjectStoreViewModel::new);
    app.add_singleton_model(NotebookManager::mock);
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.update(crate::settings::init_and_register_user_preferences);
    app.update(AISettings::register_and_subscribe_to_events);
}

/// Seeds one prompt owned by the local personal user and one owned by a foreign
/// team, so only the window's own space can narrow the result set. Every name
/// starts with "a" so a single-character prefix search cannot do the filtering.
fn seed_prompts(app: &mut App) {
    ObjectStoreModel::handle(app).update(app, |model, _ctx| {
        let personal = mock_prompt(
            PERSONAL_PROMPT_ID.into(),
            Owner::mock_current_user(),
            "annotate my scratch notes",
        );
        model.add_object(personal.id, personal);
        let foreign = mock_prompt(
            FOREIGN_PROMPT_ID.into(),
            Owner::Team {
                team_uid: 456.into(),
            },
            "audit the other team's billing",
        );
        model.add_object(foreign.id, foreign);
    });
}

fn prompt_uid(id: i64) -> String {
    SyncId::ServerId(WorkflowId::from(id).into()).uid()
}

fn prompt_ids_for_query(
    data_source: &ModelHandle<PromptsMenuDataSource>,
    query: &str,
    app: &App,
) -> Vec<String> {
    app.read(|app| {
        data_source
            .as_ref(app)
            .run_query(&Query::from(query), app)
            .expect("prompts menu query should succeed")
            .iter()
            .map(|result| result.accept_result().id.uid())
            .sorted()
            .collect()
    })
}

/// The `#` menu's empty-query path reads the object store directly rather than going
/// through the InfiniShell Drive data source, so it needs its own window scoping.
#[test]
fn test_prompts_menu_empty_query_only_returns_prompts_in_the_window() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        seed_prompts(&mut app);

        let window_id = WindowId::new();
        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        assert_eq!(
            prompt_ids_for_query(&data_source, "", &app),
            vec![prompt_uid(PERSONAL_PROMPT_ID)]
        );
    })
}

/// The single-character path takes a separate prefix match that also reads the object store.
#[test]
fn test_prompts_menu_single_character_query_only_returns_prompts_in_the_window() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        seed_prompts(&mut app);

        let window_id = WindowId::new();
        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        assert_eq!(
            prompt_ids_for_query(&data_source, "a", &app),
            vec![prompt_uid(PERSONAL_PROMPT_ID)]
        );
    })
}

/// A window with no team still sees the user's personal prompts.
#[test]
fn test_prompts_menu_teamless_window_returns_personal_prompts() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        seed_prompts(&mut app);

        let window_id = WindowId::new();
        let data_source = app.add_model(|ctx| PromptsMenuDataSource::new(window_id, ctx));

        app.read(|app| {
            assert_eq!(
                UserWorkspaces::as_ref(app).spaces_for_window(window_id, app),
                vec![Space::Personal]
            );
        });
        assert_eq!(
            prompt_ids_for_query(&data_source, "", &app),
            vec![prompt_uid(PERSONAL_PROMPT_ID)]
        );
    })
}
