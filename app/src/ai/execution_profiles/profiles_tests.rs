use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_util::path::EscapeChar;
use warpui::{App, EntityId, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::{
    AIExecutionProfile, AIExecutionProfileObject, AIExecutionProfileObjectModel, ActionPermission,
    ExecutionProfileId, create_default_for_tui_from_legacy_settings,
    create_default_from_legacy_settings,
};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::{ObjectStoreEvent, ObjectStoreModel};
use crate::cloud_object::update_manager::UpdateManager;
use crate::cloud_object::{StoredObjectMetadata, StoredObjectPermissions};
use crate::network::NetworkStatus;
use crate::server::ids::{ServerId, SyncId};
use crate::settings::{AISettings, AgentModeCommandExecutionPredicate, PrivacySettings};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{LaunchMode, TuiEntryPoint};

/// Install the minimal singleton graph needed to construct an
/// `AIExecutionProfilesModel` and exercise its ObjectStoreModel interactions.
fn install_singletons(app: &mut App, auth_state: AuthStateProvider) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| auth_state);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(UserWorkspaces::default_mock);
}

#[test]
fn tui_missing_collection_seeds_agent_decides_for_execute_commands() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());

        let expected_legacy_seed = app.read(create_default_from_legacy_settings);
        let expected_tui_seed = app.read(create_default_for_tui_from_legacy_settings);
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    entrypoint: TuiEntryPoint::Interactive {
                        mount: Box::new(|_| {}),
                        api_key: None,
                    },
                },
                ctx,
            )
        });

        profile_model.read(&app, |model, ctx| {
            let profile_info = model.default_profile(ctx);
            let profile = profile_info.data();
            assert_eq!(profile, &expected_tui_seed);
            assert_eq!(
                profile.execute_commands,
                ActionPermission::AgentDecides,
                "a fresh TUI profile should let the agent decide whether to execute commands"
            );
            assert_eq!(
                expected_tui_seed,
                AIExecutionProfile {
                    execute_commands: ActionPermission::AgentDecides,
                    ..expected_legacy_seed
                },
                "the TUI default should change no other legacy-seeded fields"
            );
        });
    })
}

#[test]
fn tui_default_denylist_overrides_agent_decides_command_execution() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    entrypoint: TuiEntryPoint::Interactive {
                        mount: Box::new(|_| {}),
                        api_key: None,
                    },
                },
                ctx,
            )
        });
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let permissions = app.add_singleton_model(BlocklistAIPermissions::new);
        let terminal_view_id = EntityId::new();
        let conversation_id = AIConversationId::new();

        profile_model.update(&mut app, |model, ctx| {
            let profile_id = model.default_profile_id();
            model.add_to_command_denylist(
                &profile_id,
                &AgentModeCommandExecutionPredicate::new_regex("rm .*").unwrap(),
                ctx,
            );
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().execute_commands,
                ActionPermission::AgentDecides
            );
        });

        permissions.read(&app, |model, ctx| {
            let result = model.can_autoexecute_command(
                &conversation_id,
                "rm important.txt",
                EscapeChar::Backslash,
                false,
                Some(false),
                Some(terminal_view_id),
                ctx,
            );
            assert!(!result.is_allowed());
            assert!(
                format!("{result:?}").contains("ExplicitlyDenylisted"),
                "TUI denylist should take precedence over AgentDecides: {result:?}"
            );
        });
    })
}

#[test]
fn tui_explicit_collection_preserves_execute_commands() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let explicit_profile = AIExecutionProfile {
            name: "Explicit TUI profile".to_string(),
            is_default_profile: true,
            execute_commands: ActionPermission::AlwaysAsk,
            ..Default::default()
        };
        app.update(|ctx| {
            let mut profiles = crate::ai::execution_profiles::ExecutionProfilesConfig::default();
            profiles.insert(ExecutionProfileId::default_profile(), explicit_profile);
            AISettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.execution_profiles.set_value(profiles, ctx)
                })
                .unwrap();
        });

        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    entrypoint: TuiEntryPoint::Interactive {
                        mount: Box::new(|_| {}),
                        api_key: None,
                    },
                },
                ctx,
            )
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().execute_commands,
                ActionPermission::AlwaysAsk
            );
        });
    })
}

#[test]
fn gui_default_execute_commands_remains_always_ask() {
    let _guard = FeatureFlag::FileBackedExecutionProfiles.override_enabled(false);

    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().execute_commands,
                ActionPermission::AlwaysAsk,
                "the GUI/legacy default must remain conservative"
            );
        });
    })
}

/// Regression test for the onboarding autonomy bug where
/// `edit_profile_internal` would silently drop edits made to an `Unsynced`
/// default profile whenever `personal_drive` returned `None` (logged-out
/// users). `apply_agent_settings` calls `set_*` on the default profile the
/// moment onboarding completes, which can happen before the user logs in
/// (e.g. `LoginSlideEvent::LoginLaterConfirmed`), so those edits must
/// persist on the local `Unsynced` state rather than being dropped.
#[test]
fn edits_persist_on_unsynced_default_profile_when_logged_out() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_logged_out_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        let default_profile_id = profile_model.read(&app, |model, _ctx| model.default_profile_id());

        // Sanity-check the precondition: the baseline `apply_code_diffs`
        // on a fresh default profile is the enum default (`AgentDecides`).
        profile_model.read(&app, |model, ctx| {
            assert!(
                matches!(
                    model.default_profile(ctx).data().apply_code_diffs,
                    ActionPermission::AgentDecides
                ),
                "unexpected baseline apply_code_diffs"
            );
        });

        // Apply the edit that onboarding would make for the Full autonomy
        // preset. Before the fix, this call no-ops because
        // `personal_drive` is `None` while the profile is `Unsynced` — the
        // `set_apply_code_diffs` value was cloned, mutated, then dropped
        // without being written back to `default_profile_state`.
        profile_model.update(&mut app, |model, ctx| {
            model.set_apply_code_diffs(&default_profile_id, &ActionPermission::AlwaysAllow, ctx);
        });

        profile_model.read(&app, |model, ctx| {
            assert_eq!(
                model.default_profile(ctx).data().apply_code_diffs,
                ActionPermission::AlwaysAllow,
                "edit was dropped: default profile still has the baseline \
                 apply_code_diffs value after an edit made while logged out",
            );
        });
    })
}

/// Regression test for the "log in to an existing user after onboarding"
/// bug. Objects restored from local storage can already exist in `ObjectStoreModel`
/// before `AIExecutionProfilesModel` observes per-object `ObjectCreated` events.
/// The model reconciles when it receives `ObjectStoreEvent::InitialLoadCompleted`.
/// Without the reconciliation handler for `InitialLoadCompleted`, the
/// existing user's default profile sits in `ObjectStoreModel` but
/// `AIExecutionProfilesModel` stays in `Unsynced`, so a subsequent
/// onboarding edit creates a duplicate cloud default profile instead of
/// editing the existing one. This test drives that sequence and asserts
/// the model adopts the cloud profile's sync id.
#[test]
fn reconciles_unsynced_default_profile_with_cloud_after_initial_load() {
    App::test((), |mut app| async move {
        install_singletons(&mut app, AuthStateProvider::new_for_test());
        let profile_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });

        // Baseline: ObjectStoreModel is empty, so the model starts Unsynced and
        // `sync_id` is `None`.
        profile_model.read(&app, |model, ctx| {
            assert!(
                model.default_profile(ctx).sync_id().is_none(),
                "default profile should be Unsynced at startup"
            );
        });

        // Simulate the user's existing default profile object arriving via
        // initial bulk load. We construct the existing profile with
        // `apply_code_diffs = AlwaysAllow` so we can verify the model is
        // reading that stored object after reconciliation.
        let cloud_uid = ServerId::from(42);
        let cloud_sync_id = SyncId::ServerId(cloud_uid);
        let local_profile = AIExecutionProfile {
            name: "Default".to_string(),
            is_default_profile: true,
            apply_code_diffs: ActionPermission::AlwaysAllow,
            ..Default::default()
        };
        let profile_object = AIExecutionProfileObject::new(
            cloud_sync_id,
            AIExecutionProfileObjectModel::new(local_profile),
            StoredObjectMetadata::mock(),
            StoredObjectPermissions::mock_personal(),
        );

        // Insert the object into ObjectStoreModel without per-object events and then
        // emit `InitialLoadCompleted` so the reconciliation handler fires.
        ObjectStoreModel::handle(&app).update(&mut app, move |object_store_model, ctx| {
            object_store_model.add_object(cloud_sync_id, profile_object);
            ctx.emit(ObjectStoreEvent::InitialLoadCompleted);
        });

        // The model should now be Synced with the stored profile object's sync_id,
        // and `default_profile` should read values from the existing local
        // object (proving we're not backed by a fresh client-side default).
        profile_model.read(&app, |model, ctx| {
            let info = model.default_profile(ctx);
            assert_eq!(
                info.sync_id(),
                Some(cloud_sync_id),
                "model did not adopt the existing default profile object's sync_id"
            );
            assert_eq!(
                info.data().apply_code_diffs,
                ActionPermission::AlwaysAllow,
                "default profile should now surface the existing stored value"
            );
        });

        // Further edits should now target the existing profile object in
        // place, rather than falling through the `Unsynced` branch and
        // creating a duplicate.
        let default_profile_id = profile_model.read(&app, |model, _ctx| model.default_profile_id());
        profile_model.update(&mut app, |model, ctx| {
            model.set_apply_code_diffs(&default_profile_id, &ActionPermission::AlwaysAsk, ctx);
        });
        profile_model.read(&app, |model, ctx| {
            let info = model.default_profile(ctx);
            assert_eq!(
                info.sync_id(),
                Some(cloud_sync_id),
                "edit should target the same cloud sync_id, not create a duplicate"
            );
            assert_eq!(
                info.data().apply_code_diffs,
                ActionPermission::AlwaysAsk,
                "edit should be reflected on the existing profile object"
            );
        });
    })
}
