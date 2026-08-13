use std::time::Duration;

use chrono::Utc;
use instant::Instant;
use settings::manager::SettingsManager;
use warpui::{App, ModelContext, SingletonEntity, WindowId};

use super::*;
use crate::auth::{AuthStateProvider, UserUid};
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::model::view::ObjectStoreViewModel;
use crate::cloud_object::update_manager::UpdateManager;
use crate::cloud_object::{
    Owner, StoredObjectGuest, StoredObjectMetadata, StoredObjectPermissions,
};
use crate::drive::sharing::{SharingAccessLevel, Subject, UserKind};
use crate::features::FeatureFlag;
use crate::network::NetworkStatus;
use crate::notebooks::manager::NotebookManager;
use crate::notebooks::{NotebookId, NotebookObject, NotebookObjectModel};
use crate::search::data_source::Query;
use crate::server::ids::ObjectUid;
use crate::server::ids::SyncId::{self};
use crate::settings::{AISettings, PrivacySettings};
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowId, WorkflowObject, WorkflowObjectModel};
use crate::workspaces::team::Team;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

/// Zap:上游这些测试通过 `CloudModel::upsert_from_server_*`(服务端下推路径)播种对象。
/// 本地优先形态下服务端同步链路已剥离,对象存储只保留本地创建路径,因此改用
/// `ObjectStoreModel::create_object` —— 它同样发出 `ObjectStoreEvent::ObjectCreated`,
/// 命令面板的 drive data source 依旧会据此增量索引,测试覆盖的行为不变。
trait ObjectStoreTestSeed {
    fn seed_workflow(&mut self, workflow: WorkflowObject, ctx: &mut ModelContext<ObjectStoreModel>);
    fn seed_notebook(&mut self, notebook: NotebookObject, ctx: &mut ModelContext<ObjectStoreModel>);
}

impl ObjectStoreTestSeed for ObjectStoreModel {
    fn seed_workflow(
        &mut self,
        workflow: WorkflowObject,
        ctx: &mut ModelContext<ObjectStoreModel>,
    ) {
        self.create_object(workflow.id, workflow, ctx);
    }

    fn seed_notebook(
        &mut self,
        notebook: NotebookObject,
        ctx: &mut ModelContext<ObjectStoreModel>,
    ) {
        self.create_object(notebook.id, notebook, ctx);
    }
}

fn mock_server_metadata() -> StoredObjectMetadata {
    StoredObjectMetadata::mock()
}

fn mock_server_permissions(owner: Owner) -> StoredObjectPermissions {
    StoredObjectPermissions {
        owner,
        guests: Vec::new(),
        anyone_with_link: None,
        permissions_last_updated_ts: Some(Utc::now().into()),
    }
}

fn mock_server_workflow(id: WorkflowId, owner: Owner) -> WorkflowObject {
    mock_named_server_workflow(id, owner, format!("foo{id}"), format!("bar{id}"))
}

fn mock_named_server_workflow(
    id: WorkflowId,
    owner: Owner,
    name: impl Into<String>,
    command: impl Into<String>,
) -> WorkflowObject {
    WorkflowObject::new(
        SyncId::ServerId(id.into()),
        WorkflowObjectModel::new(Workflow::new(name, command)),
        mock_server_metadata(),
        mock_server_permissions(owner),
    )
}

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_owned(),
        color: None,
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_for_test(teams: Vec<Team>) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn mock_server_notebook(id: NotebookId, owner: Owner) -> NotebookObject {
    NotebookObject::new(
        SyncId::ServerId(id.into()),
        NotebookObjectModel {
            title: format!("foo{id}"),
            data: format!("bar{id}"),
            ai_document_id: None,
            conversation_id: None,
        },
        mock_server_metadata(),
        mock_server_permissions(owner),
    )
}

fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
    // Add the necessary singleton models to the App.
    // Zap:`ServerApiProvider` / `SyncQueue` / `TeamTesterStatus` 属云端同步链路,
    // 已随本地优先改造删除,这里不再注册(`UserWorkspaces::mock` 也不再需要云端 client)。
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(|ctx| UserWorkspaces::mock(workspaces, ctx));
    // `update_workspaces` pushes enterprise settings into PrivacySettings.
    app.add_singleton_model(PrivacySettings::mock);
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

#[test]
fn test_drive_data_source_correctly_filters_drive_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize ObjectStoreModel
        ObjectStoreModel::handle(&app).update(&mut app, |model, ctx| {
            model.seed_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.seed_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with the drive filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Drive]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect both of the results to be included
            assert_eq!(results.len(), 2);
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_no_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize ObjectStoreModel
        ObjectStoreModel::handle(&app).update(&mut app, |model, ctx| {
            model.seed_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.seed_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::new(),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect both of the results to be included
            assert_eq!(results.len(), 2);
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_workflow_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize ObjectStoreModel
        ObjectStoreModel::handle(&app).update(&mut app, |model, ctx| {
            model.seed_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.seed_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Workflows]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect only the workflow result to be included
            assert_eq!(results.len(), 1);

            assert!(results[0].accessibility_label().starts_with("Workflow:"));
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_notebook_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize ObjectStoreModel
        ObjectStoreModel::handle(&app).update(&mut app, |model, ctx| {
            model.seed_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.seed_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Notebooks]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect only the workflow result to be included
            assert_eq!(results.len(), 1);

            assert!(results[0].accessibility_label().starts_with("Notebook:"));
        });
    })
}

/// Upper bound on how long the background indexer may take before a test gives up. Only a
/// broken assertion ever waits this long; the poll below exits as soon as the state matches.
const INDEX_TIMEOUT: Duration = Duration::from_secs(10);
const INDEX_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// First id reserved for index markers, kept clear of the ids the tests assert on.
const INDEX_MARKER_ID: i64 = 900;

fn workflow_labels(
    mixer: &ModelHandle<CommandPaletteMixer>,
    query: &str,
    app: &mut App,
) -> Vec<String> {
    mixer.update(app, |mixer, ctx| {
        mixer.run_query(
            Query {
                filters: HashSet::from([QueryFilter::Workflows]),
                text: query.into(),
            },
            ctx,
        );
    });
    app.read(|app| {
        let mut labels = mixer
            .as_ref(app)
            .results()
            .iter()
            .map(|result| result.accessibility_label())
            .collect::<Vec<_>>();
        labels.sort();
        labels
    })
}

fn workflow_label(name: &str) -> String {
    format!("Workflow: {name}")
}

/// Polls the palette until it reports `expected`, so tests synchronise on the background indexer
/// instead of racing a fixed delay.
fn assert_workflow_labels_eventually(
    mixer: &ModelHandle<CommandPaletteMixer>,
    query: &str,
    expected: &[String],
    app: &mut App,
) {
    let deadline = Instant::now() + INDEX_TIMEOUT;
    let mut observed = workflow_labels(mixer, query, app);
    while observed != expected && Instant::now() < deadline {
        std::thread::sleep(INDEX_POLL_INTERVAL);
        observed = workflow_labels(mixer, query, app);
    }
    assert_eq!(observed, expected);
}

/// Indexes a fresh in-scope workflow and waits for it to become searchable.
///
/// The searcher drains its queue in order, so once the marker is visible every operation queued
/// before it has been applied. Without this, asserting that something is *absent* from the index
/// would pass simply because the indexer had not run yet.
fn drain_index(marker_id: i64, mixer: &ModelHandle<CommandPaletteMixer>, app: &mut App) {
    let marker_name = format!("indexmarker{marker_id}");
    ObjectStoreModel::handle(app).update(app, |model, ctx| {
        model.seed_workflow(
            mock_named_server_workflow(
                marker_id.into(),
                Owner::mock_current_user(),
                marker_name.clone(),
                "echo marker",
            ),
            ctx,
        );
    });
    assert_workflow_labels_eventually(mixer, &marker_name, &[workflow_label(&marker_name)], app);
}

fn prompt_or_workflow_uid(id: i64) -> ObjectUid {
    SyncId::ServerId(WorkflowId::from(id).into()).uid()
}

// 此处另有 4 个上游测试,断言的是「窗口按所属 team 过滤 Drive 对象」:本 fork 已剥离 team
// 维度(`UserWorkspaces::team_from_uid` 恒返回 None,窗口拿不到 Space::Team),
// 属已剥离能力,随之删除。Personal 空间的 Drive 索引仍由本文件其余用例覆盖。

/// Enough out-of-window workflows to more than fill the full-text searcher's result cap. They must
/// never reach the ranker: they are not in this window's corpus at all.
const CROWDING_WORKFLOW_COUNT: i64 = 25;

// 此处原有 3 个上游云端能力测试(team 维度的 Drive 索引),随该能力剥离一并删除。

fn mock_shared_server_permissions(shared_with: UserUid) -> StoredObjectPermissions {
    StoredObjectPermissions {
        owner: Owner::User {
            user_uid: UserUid::new("someone-else"),
        },
        guests: vec![StoredObjectGuest {
            subject: Subject::User(UserKind::Account(shared_with)),
            access_level: SharingAccessLevel::View,
            source: None,
        }],
        anyone_with_link: None,
        permissions_last_updated_ts: Some(Utc::now().into()),
    }
}

fn current_user_uid(app: &App) -> UserUid {
    app.read(|app| {
        AuthStateProvider::as_ref(app)
            .get()
            .user_id()
            .expect("test user should be authenticated")
    })
}

/// The shared space is only in scope while a directly shared object exists, so the very first one
/// widens the scope of an index that already exists.
#[test]
fn test_full_text_drive_data_source_indexes_the_first_directly_shared_object() {
    let _shared_with_me = FeatureFlag::SharedWithMe.override_enabled(true);
    let _tantivy = FeatureFlag::UseTantivySearch.override_enabled(true);
    let team = team_for_test(123, "selected");
    let workspace = workspace_for_test(vec![team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, team.uid, ctx);
        });

        // No shared object exists yet, so the data source starts without the shared space.
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        mixer.update(&mut app, |mixer, _| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);
        });
        drain_index(INDEX_MARKER_ID, &mixer, &mut app);
        assert_workflow_labels_eventually(&mixer, "bequeathed", &[], &mut app);

        let shared_with = current_user_uid(&app);
        ObjectStoreModel::handle(&app).update(&mut app, |model, ctx| {
            model.seed_workflow(
                WorkflowObject::new(
                    SyncId::ServerId(WorkflowId::from(1).into()),
                    WorkflowObjectModel::new(Workflow::new("bequeathed workflow", "echo shared")),
                    mock_server_metadata(),
                    mock_shared_server_permissions(shared_with),
                ),
                ctx,
            );
        });

        assert_workflow_labels_eventually(
            &mixer,
            "bequeathed",
            &[workflow_label("bequeathed workflow")],
            &mut app,
        );
    })
}
