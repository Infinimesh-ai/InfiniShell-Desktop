use super::*;

#[test]
fn historical_downgrade_intent_keeps_its_persisted_name() {
    let old = r#""claude_npm_stable21280_to21278""#;
    assert_eq!(
        serde_json::from_str::<Intent>(old).unwrap(),
        Intent::ClaudeNpmStable21280To21278
    );
    assert_eq!(
        serde_json::to_string(&Intent::ClaudeNpmStable21280To21278).unwrap(),
        old
    );
    assert_eq!(
        serde_json::to_string(&Intent::ClaudeNpmStable21287To21285).unwrap(),
        r#""claude_npm_stable21287_to21285""#
    );
    assert!(serde_json::from_str::<Intent>(r#""claude_npm_stable21288_to21285""#).is_err());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn selected(installed: &str, target: &str, channel: Channel) -> Result<Option<Intent>, Error> {
    select(CLIAgent::Claude, Source::Npm, installed, target, channel)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn reviewed_consumer_upgrades_do_not_require_downgrade_intent() {
    assert_eq!(selected("2.1.280", "2.1.285", Channel::Stable), Ok(None));
    assert_eq!(selected("2.1.280", "2.1.287", Channel::Latest), Ok(None));
    assert_eq!(
        selected("2.1.285", "2.1.287", Channel::FollowInstallation),
        Ok(None)
    );
    assert_eq!(validate(None, "2.1.280", "2.1.285", &None), Ok(()));
    assert_eq!(validate(None, "2.1.280", "2.1.287", &None), Ok(()));
    assert_eq!(validate(None, "2.1.285", "2.1.287", &None), Ok(()));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn consumer_downgrade_requires_the_explicit_stable_choice() {
    assert_eq!(
        selected("2.1.287", "2.1.285", Channel::Stable),
        Ok(Some(Intent::ClaudeNpmStable21287To21285))
    );
    assert_eq!(
        selected("2.1.287", "2.1.285", Channel::Latest),
        Err(Error::ChannelMismatch)
    );
    assert_eq!(
        selected("2.1.287", "2.1.285", Channel::FollowInstallation),
        Err(Error::ChannelMismatch)
    );
    assert_eq!(
        selected("2.1.287", "2.1.285", Channel::Alpha),
        Err(Error::ChannelMismatch)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn consumer_migration_rejects_unreviewed_origins_and_backward_edges() {
    assert_eq!(
        selected("2.1.278", "2.1.285", Channel::Stable),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        selected("2.1.286", "2.1.287", Channel::Latest),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        selected("2.1.288", "2.1.285", Channel::Stable),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        selected("2.1.285", "2.1.280", Channel::Stable),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        selected("2.1.287", "2.1.280", Channel::Stable),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        selected("2.1.285", "2.1.278", Channel::Stable),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        validate(None, "2.1.286", "2.1.287", &None),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(
        validate(None, "2.1.287", "2.1.280", &None),
        Err(Error::RecoveryRequired)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn resumed_consumer_downgrade_binds_intent_and_published_channel() {
    use super::super::{ConfigDesired, ConfigKind};

    let mut config = ConfigBackup {
        kind: ConfigKind::Claude,
        path: PathBuf::from("/private/unused-claude-settings.json"),
        before: Some(br#"{"autoUpdatesChannel":"latest"}"#.to_vec()),
        after: None,
        desired: Some(ConfigDesired {
            bytes: Some(br#"{"autoUpdatesChannel":"stable"}"#.to_vec()),
        }),
        before_mode: None,
        restore_stage: None,
    };
    let intent = Some(Intent::ClaudeNpmStable21287To21285);
    assert_eq!(
        validate(intent, "2.1.287", "2.1.285", &Some(config.clone())),
        Ok(())
    );
    assert_eq!(
        validate(None, "2.1.287", "2.1.285", &Some(config.clone())),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(
        validate(
            Some(Intent::ClaudeNpmStable21280To21278),
            "2.1.287",
            "2.1.285",
            &Some(config.clone())
        ),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(
        validate(intent, "2.1.280", "2.1.285", &Some(config.clone())),
        Err(Error::RecoveryRequired)
    );
    config.desired = None;
    assert_eq!(
        validate(intent, "2.1.287", "2.1.285", &Some(config)),
        Err(Error::ChannelMismatch)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn historical_pair_and_same_version_transactions_remain_available() {
    assert_eq!(
        selected("2.1.280", "2.1.278", Channel::Stable),
        Ok(Some(Intent::ClaudeNpmStable21280To21278))
    );
    assert_eq!(selected("2.1.278", "2.1.280", Channel::Latest), Ok(None));
    assert_eq!(validate(None, "2.1.278", "2.1.280", &None), Ok(()));
    assert_eq!(validate(None, "2.1.285", "2.1.285", &None), Ok(()));
    assert_eq!(validate(None, "2.1.287", "2.1.287", &None), Ok(()));
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn new_consumer_migrations_are_unavailable_on_other_hosts() {
    assert_eq!(
        select(
            CLIAgent::Claude,
            Source::Npm,
            "2.1.280",
            "2.1.285",
            Channel::Stable
        ),
        Err(Error::UnsupportedPlatform)
    );
    assert_eq!(
        validate(None, "2.1.280", "2.1.287", &None),
        Err(Error::UnsupportedPlatform)
    );
}

#[cfg(feature = "local_fs")]
fn historical_task() -> crate::persistence::model::LocalCliTask {
    serde_json::from_value(serde_json::json!({
        "task_id":"fixture-history", "harness":"claude", "working_directory":"/private/history",
        "config_json":serde_json::json!({
            "cli_version":"2.1.278", "runtime_generation":"00000000-0000-0000-0000-000000000001",
            "cli_version_runtime_generation":"00000000-0000-0000-0000-000000000001",
            "permission_policy":"Inherit"
        }).to_string(),
        "native_session_id":"00000000-0000-0000-0000-000000000002",
        "generation":1, "revision":1, "state":"completed"
    }))
    .unwrap()
}

#[cfg(feature = "local_fs")]
#[test]
fn new_intent_does_not_reuse_the_old_native_history_contract() {
    let mut task = historical_task();
    assert!(compatible_history(
        Intent::ClaudeNpmStable21280To21278,
        [&task].into_iter()
    ));
    assert!(!compatible_history(
        Intent::ClaudeNpmStable21287To21285,
        [&task].into_iter()
    ));
    task.config_json = task.config_json.replace("2.1.278", "2.1.285");
    assert!(!compatible_history(
        Intent::ClaudeNpmStable21287To21285,
        [&task].into_iter()
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn consumer_downgrade_accepts_only_terminal_history_without_native_sessions() {
    use crate::persistence::model::LocalCliTaskState;

    let intent = Intent::ClaudeNpmStable21287To21285;
    assert!(compatible_history(intent, std::iter::empty()));
    let mut task = historical_task();
    task.native_session_id = None;
    task.state = LocalCliTaskState::Failed;
    assert!(compatible_history(intent, [&task].into_iter()));
    task.state = LocalCliTaskState::Running;
    assert!(!compatible_history(intent, [&task].into_iter()));
    task.state = LocalCliTaskState::Unknown;
    assert!(!compatible_history(intent, [&task].into_iter()));
    task.state = LocalCliTaskState::Disconnected;
    assert!(!compatible_history(intent, [&task].into_iter()));
}
