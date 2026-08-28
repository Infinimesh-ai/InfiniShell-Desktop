use chrono::{Duration, Utc};
use warpui::App;

use super::{ObjectAction, ObjectActionSubtype, ObjectActionType, ObjectActions};

#[test]
fn test_object_actions_daily() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let actions: Vec<ObjectAction> = vec![
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::minutes(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::minutes(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::hours(23),
                    processed_at_timestamp: Some(Utc::now() - Duration::hours(23)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(10),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(28),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(28)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::BundledActions {
                    count: 5,
                    oldest_timestamp: Utc::now() - Duration::days(250),
                    latest_timestamp: Utc::now() - Duration::days(50),
                    latest_processed_at_timestamp: Utc::now() - Duration::days(50),
                },
            },
        ];

        let object_actions_handle = app.add_model(|_| ObjectActions::new(actions));

        object_actions_handle.read(&app, |handle, _ctx| {
            assert_eq!(
                handle.get_action_history_summary_for_action_type(
                    &"asdfljk".to_string(),
                    ObjectActionType::Execute,
                ),
                Some(crate::t!("cloud-object-action-runs-last-day", count = 2))
            )
        });
    });
}

#[test]
fn test_object_actions_rollup_weekly() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let actions: Vec<ObjectAction> = vec![
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::minutes(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::minutes(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::hours(23),
                    processed_at_timestamp: Some(Utc::now() - Duration::hours(23)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(4),
                    processed_at_timestamp: Some(Utc::now()),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(10),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(28),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::BundledActions {
                    count: 5,
                    oldest_timestamp: Utc::now() - Duration::days(250),
                    latest_timestamp: Utc::now() - Duration::days(50),
                    latest_processed_at_timestamp: Utc::now() - Duration::days(50),
                },
            },
        ];

        let object_actions_handle = app.add_model(|_| ObjectActions::new(actions));

        object_actions_handle.read(&app, |handle, _ctx| {
            assert_eq!(
                handle.get_action_history_summary_for_action_type(
                    &"q23423aaf".to_string(),
                    ObjectActionType::Execute,
                ),
                Some(crate::t!("cloud-object-action-runs-last-week", count = 1))
            )
        });
    });
}

#[test]
fn test_object_actions_rollup_monthly() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let actions: Vec<ObjectAction> = vec![
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::minutes(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::minutes(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::hours(23),
                    processed_at_timestamp: Some(Utc::now() - Duration::hours(23)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(10),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(15),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(15)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(28),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(28)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::BundledActions {
                    count: 5,
                    oldest_timestamp: Utc::now() - Duration::days(250),
                    latest_timestamp: Utc::now() - Duration::days(50),
                    latest_processed_at_timestamp: Utc::now() - Duration::days(50),
                },
            },
        ];

        let object_actions_handle = app.add_model(|_| ObjectActions::new(actions));

        object_actions_handle.read(&app, |handle, _ctx| {
            assert_eq!(
                handle.get_action_history_summary_for_action_type(
                    &"q23423aaf".to_string(),
                    ObjectActionType::Execute,
                ),
                Some(crate::t!("cloud-object-action-runs-last-month", count = 3))
            )
        });
    });
}

#[test]
fn test_object_actions_rollup_yearly() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let actions: Vec<ObjectAction> = vec![
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::minutes(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::minutes(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::hours(23),
                    processed_at_timestamp: Some(Utc::now() - Duration::hours(23)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(10),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(15),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(15)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(28),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(28)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "q23423aaf".to_string(),
                action_subtype: ObjectActionSubtype::BundledActions {
                    count: 5,
                    oldest_timestamp: Utc::now() - Duration::days(250),
                    latest_timestamp: Utc::now() - Duration::days(50),
                    latest_processed_at_timestamp: Utc::now() - Duration::days(50),
                },
            },
        ];

        let object_actions_handle = app.add_model(|_| ObjectActions::new(actions));

        object_actions_handle.read(&app, |handle, _ctx| {
            assert_eq!(
                handle.get_action_history_summary_for_action_type(
                    &"q23423aaf".to_string(),
                    ObjectActionType::Execute,
                ),
                Some(crate::t!("cloud-object-action-runs-last-year", count = 5))
            )
        });
    });
}

#[test]
fn test_object_actions_rollup_out_of_date_bundle() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let actions: Vec<ObjectAction> = vec![
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::minutes(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::minutes(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::hours(23),
                    processed_at_timestamp: Some(Utc::now() - Duration::hours(23)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(10),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(15),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(15)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(28),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(28)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "q23423aaf".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::BundledActions {
                    count: 5,
                    oldest_timestamp: Utc::now() - Duration::days(400),
                    latest_timestamp: Utc::now() - Duration::days(350),
                    latest_processed_at_timestamp: Utc::now() - Duration::days(350),
                },
            },
        ];

        let object_actions_handle = app.add_model(|_| ObjectActions::new(actions));

        object_actions_handle.read(&app, |handle, _ctx| {
            assert_eq!(
                handle.get_action_history_summary_for_action_type(
                    &"q23423aaf".to_string(),
                    ObjectActionType::Execute,
                ),
                Some(crate::t!("cloud-object-action-runs-last-year", count = 0))
            )
        });
    });
}

#[test]
fn test_object_actions_rollup_none() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let actions: Vec<ObjectAction> = vec![
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::minutes(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::minutes(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::hours(23),
                    processed_at_timestamp: Some(Utc::now() - Duration::hours(23)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(4),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(4)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(10),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(10)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(15),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(15)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::SingleAction {
                    timestamp: Utc::now() - Duration::days(28),
                    processed_at_timestamp: Some(Utc::now() - Duration::days(28)),
                    data: Some("Some data".to_string()),
                    pending: false,
                },
            },
            ObjectAction {
                action_type: ObjectActionType::Execute,
                uid: "asdfljk".to_string(),
                hashed_sqlite_id: "asdfljk".to_string(),
                action_subtype: ObjectActionSubtype::BundledActions {
                    count: 5,
                    oldest_timestamp: Utc::now() - Duration::days(400),
                    latest_timestamp: Utc::now() - Duration::days(350),
                    latest_processed_at_timestamp: Utc::now() - Duration::days(350),
                },
            },
        ];

        let object_actions_handle = app.add_model(|_| ObjectActions::new(actions));

        object_actions_handle.read(&app, |handle, _ctx| {
            assert_eq!(
                handle.get_action_history_summary_for_action_type(
                    &"q23423aaf".to_string(),
                    ObjectActionType::Execute,
                ),
                Some(crate::t!("cloud-object-action-runs-last-year", count = 0))
            )
        });
    });
}
