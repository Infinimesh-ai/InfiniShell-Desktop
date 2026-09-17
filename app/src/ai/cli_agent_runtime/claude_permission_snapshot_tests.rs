use super::*;

const OBSERVED: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-permission-snapshot-dynamic-macos.json"
);

fn controlled() -> (Observation, Value, Value, Value) {
    let initialize =
        json!({"pid":123, "current_permission_mode":"default", "session_state":"idle"});
    let settings = json!({"effective":{"sandbox":{"enabled":false}, "permissions":{"deny":["Bash"]}},
        "sources":[{"source":"flagSettings", "settings":{"sandbox":{"enabled":false}, "permissions":{"deny":["Bash"]}}}]});
    let rules = json!({"state":{"rules":[{"source":"flagSettings", "behavior":"deny", "rule":"Bash", "editability":"readonly"}],
        "workspaceDirectories":[], "originalCwd":"/isolated/project", "managedOnly":false}});
    (
        Observation::new(Uuid::from_u128(9), &initialize),
        initialize,
        settings,
        rules,
    )
}

fn observe(snapshot: &Value) -> Observation {
    let initialize = json!({"pid":snapshot["native_pid"], "current_permission_mode":snapshot["mode_before"], "session_state":"idle"});
    let mut observation = Observation::new(Uuid::from_u128(9), &initialize);
    let settings = json!({"effective":snapshot["settings"]["effective"], "sources":snapshot["settings"]["sources"]});
    let rules = &snapshot["rules"];
    observation.settings(&settings);
    observation.rules(&json!({"state":{"rules":rules["rules"], "workspaceDirectories":rules["workspaceDirectories"],
        "originalCwd":rules["originalCwd"], "managedOnly":rules["managedOnly"]}}));
    observation.finish(&json!({"pid":snapshot["native_pid"], "current_permission_mode":snapshot["mode_after"], "session_state":"idle"}),
        Path::new(rules["originalCwd"].as_str().unwrap()));
    observation
}

#[test]
fn captured_same_connection_observation_preserves_sources_without_authorizing_dispatch() {
    let fixture: Value = serde_json::from_str(OBSERVED).unwrap();
    let observation = observe(&fixture["sessions"][0]["snapshots"][0]);
    assert_eq!(observation.rejections, Vec::<Rejection>::new());
    let projected = observation.value();
    assert_eq!(
        projected["connectionGeneration"],
        Uuid::from_u128(9).to_string()
    );
    assert_eq!(
        projected["rules"]["workspaceDirectories"][0]["source"],
        "cliArg"
    );
    assert_eq!(
        projected["settings"]["sources"][0]["source"],
        "flagSettings"
    );
    assert_eq!(projected["atomicPermissionCeilingProven"], false);
    assert_eq!(projected["dispatchAuthorized"], false);
}

#[test]
fn captured_live_rules_lagging_file_settings_are_rejected() {
    let fixture: Value = serde_json::from_str(OBSERVED).unwrap();
    let observation = observe(&fixture["sessions"][3]["snapshots"][1]);
    assert_eq!(
        observation.rejections,
        vec![Rejection::SettingsLiveRulesMismatch]
    );
    assert_eq!(observation.value()["dispatchAuthorized"], false);
}

#[test]
fn unknown_settings_and_error_details_are_not_serialized() {
    let (mut observation, initialize, mut settings, rules) = controlled();
    settings["effective"]["env"] = json!({"PRIVATE_API_KEY":"private-token"});
    settings["sources"][0]["settings"]["private-key-name"] = json!("private-source-token");
    settings["applied"] = json!({"account":"private-applied-token"});
    settings["errors"] = json!([{"message":"private-error-token"}]);
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(
        observation.rejections,
        vec![Rejection::NativeErrors, Rejection::UnknownFields]
    );
    let serialized = observation.value().to_string();
    assert!(!serialized.contains("private"));
    assert!(!serialized.contains("API_KEY"));
    assert!(!serialized.contains("applied"));
}

#[test]
fn unknown_sandbox_field_is_rejected_without_persisting_its_value() {
    let (mut observation, initialize, mut settings, rules) = controlled();
    settings["effective"]["sandbox"]["future-network-token"] = json!("private-content");
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(observation.rejections, vec![Rejection::UnknownFields]);
    assert!(!observation.value().to_string().contains("private-content"));
    assert!(
        !observation
            .value()
            .to_string()
            .contains("future-network-token")
    );
}

#[test]
fn missing_sandbox_cannot_be_interpreted_as_disabled() {
    let (mut observation, initialize, mut settings, rules) = controlled();
    settings["effective"]
        .as_object_mut()
        .unwrap()
        .remove("sandbox");
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(
        observation.rejections,
        vec![Rejection::SandboxNotExplicitlyDisabled]
    );
}

#[test]
fn changed_native_pid_is_rejected_even_with_identical_policy_text() {
    let (mut observation, mut initialize, settings, rules) = controlled();
    observation.settings(&settings);
    observation.rules(&rules);
    initialize["pid"] = json!(124);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(observation.rejections, vec![Rejection::IdentityChanged]);
}

#[test]
fn mode_change_during_queries_or_after_observation_is_retained() {
    let (mut observation, initialize, settings, rules) = controlled();
    observation.settings(&settings);
    observation.rules(&rules);
    observation.invalidate_mode(&json!("dontAsk"));
    observation.finish(&initialize, Path::new("/isolated/project"));
    observation.invalidate_mode(&json!("default"));
    assert_eq!(observation.rejections, vec![Rejection::ModeChanged]);
}

#[test]
fn source_relative_rule_and_session_grants_are_preserved() {
    let (mut observation, initialize, settings, mut rules) = controlled();
    rules["state"]["rules"].as_array_mut().unwrap().push(json!({"source":"session", "behavior":"allow", "rule":"Read(./relative/**)", "editability":"session"}));
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(
        observation.value()["rules"]["rules"][1],
        json!({"source":"session", "behavior":"allow", "rule":"Read(./relative/**)", "editability":"session"})
    );
    assert_eq!(observation.value()["dispatchAuthorized"], false);
}

#[test]
fn managed_only_and_inactive_rule_cannot_be_called_consistent() {
    let (mut observation, initialize, settings, mut rules) = controlled();
    rules["state"]["managedOnly"] = json!(true);
    rules["state"]["rules"][0]["notInEffect"] = json!(true);
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(
        observation.rejections,
        vec![
            Rejection::ManagedOnlyUnverified,
            Rejection::SettingsLiveRulesMismatch
        ]
    );
}

#[test]
fn unknown_rule_source_and_nonboolean_sandbox_are_invalid_shapes() {
    let (mut observation, initialize, mut settings, mut rules) = controlled();
    settings["effective"]["sandbox"]["enabled"] = json!(0);
    rules["state"]["rules"][0]["source"] = json!("future");
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(observation.rejections, vec![Rejection::InvalidShape]);
    assert_eq!(observation.value()["settings"], Value::Null);
    assert_eq!(observation.value()["rules"], Value::Null);
}

#[test]
fn different_working_directory_cannot_reuse_relative_rule_observation() {
    let (mut observation, initialize, settings, rules) = controlled();
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/other"));
    assert_eq!(
        observation.rejections,
        vec![Rejection::WorkingDirectoryChanged]
    );
}

#[test]
fn effective_rule_missing_from_source_and_live_views_is_rejected() {
    let (mut observation, initialize, mut settings, rules) = controlled();
    settings["effective"]["permissions"]["deny"] = json!(["Read(./new-deny/**)"]);
    observation.settings(&settings);
    observation.rules(&rules);
    observation.finish(&initialize, Path::new("/isolated/project"));
    assert_eq!(
        observation.rejections,
        vec![Rejection::SettingsLiveRulesMismatch]
    );
}
