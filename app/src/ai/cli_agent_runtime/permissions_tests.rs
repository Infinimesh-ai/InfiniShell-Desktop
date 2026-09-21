use super::*;

#[cfg(feature = "local_fs")]
fn parent(permissions: Value) -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: "parent".into(),
        parent_task_id: None,
        parent_generation: None,
        harness: "codex".into(),
        working_directory: std::env::temp_dir().to_string_lossy().into(),
        config_json: json!({"cli_version":"0.147.0","effective_permissions":permissions})
            .to_string(),
        native_session_id: Some("native-parent".into()),
        generation: 1,
        revision: 2,
        state: crate::persistence::model::LocalCliTaskState::Running,
        result: None,
        terminal_evidence: None,
    }
}

fn captured_permissions(fixture: &str) -> Value {
    let result = fixture
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|record| {
            record
                .get("message")?
                .get("result")?
                .get("sandbox")
                .map(|_| record["message"]["result"].clone())
        })
        .unwrap();
    json!({"approvalPolicy":result["approvalPolicy"],"sandbox":result["sandbox"],"approvalsReviewer":result["approvalsReviewer"]})
}

const READ_ONLY: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-local-tools-registration.ndjson"
);
const WORKSPACE_WRITE: &str =
    include_str!("../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-two-turns.ndjson");

#[cfg(feature = "local_fs")]
#[test]
fn verified_native_snapshots_require_exact_permissions_instead_of_inherit_label() {
    for fixture in [READ_ONLY, WORKSPACE_WRITE] {
        let permissions = captured_permissions(fixture);
        let parent = parent(permissions.clone());
        let ceiling = ceiling_from_parent(&parent, "codex").unwrap();
        verify_effective_permissions(
            Some(&ceiling),
            "codex",
            Path::new(&parent.working_directory),
            &permissions,
        )
        .unwrap();
        for field in ["approvalPolicy", "approvalsReviewer"] {
            let mut changed = permissions.clone();
            changed[field] = json!("unknown-or-bypass");
            assert!(
                verify_effective_permissions(
                    Some(&ceiling),
                    "codex",
                    Path::new(&parent.working_directory),
                    &changed
                )
                .is_err()
            );
        }
        let mut changed = permissions;
        changed["sandbox"]["networkAccess"] = json!(true);
        let error = verify_effective_permissions(
            Some(&ceiling),
            "codex",
            Path::new(&parent.working_directory),
            &changed,
        )
        .unwrap_err();
        assert_eq!(
            error.permission_ceiling_evidence().unwrap()["task_input_sent"],
            false
        );
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn wider_native_sandbox_is_rejected_and_unknown_fields_cannot_be_ignored() {
    let parent = parent(captured_permissions(READ_ONLY));
    let ceiling = ceiling_from_parent(&parent, "codex").unwrap();
    let cwd = Path::new(&parent.working_directory);
    let wider = captured_permissions(WORKSPACE_WRITE);
    assert!(verify_effective_permissions(Some(&ceiling), "codex", cwd, &wider).is_err());
    for changed in [
        json!({"approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":{"type":"readOnly"}}),
        json!({"approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":{"type":"readOnly","networkAccess":false,"futureOverride":true}}),
        json!({"approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":{"type":"dangerFullAccess"}}),
    ] {
        assert!(verify_effective_permissions(Some(&ceiling), "codex", cwd, &changed).is_err());
        assert!(ceiling_from_parent(&self::parent(changed), "codex").is_err());
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn incomplete_claude_modes_missing_evidence_and_cross_cli_inheritance_fail_closed() {
    let mut task = parent(captured_permissions(READ_ONLY));
    assert!(ceiling_from_parent(&task, "claude").is_err());
    task.harness = "claude".into();
    task.config_json = json!({"cli_version":"2.1.273","effective_permissions":{"permissionMode":"default","capabilities":[]}}).to_string();
    assert!(ceiling_from_parent(&task, "claude").is_err());
    task = parent(captured_permissions(READ_ONLY));
    task.native_session_id = None;
    assert!(ceiling_from_parent(&task, "codex").is_err());
    task = parent(captured_permissions(READ_ONLY));
    task.config_json = "{}".into();
    assert!(ceiling_from_parent(&task, "codex").is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn restored_ceiling_keeps_original_parent_generation_and_directory() {
    let parent = parent(captured_permissions(READ_ONLY));
    let original = ceiling_from_parent(&parent, "codex").unwrap();
    let saved = serde_json::to_string(&original).unwrap();
    let restored: ParentPermissionCeiling = serde_json::from_str(&saved).unwrap();
    let mut child = parent.clone();
    child.task_id = "child".into();
    child.parent_task_id = Some(parent.task_id.clone());
    child.parent_generation = Some(parent.generation);
    verify_parent_binding(Some(&restored), &parent, &child).unwrap();
    let mut next_parent = parent.clone();
    next_parent.generation += 1;
    assert!(verify_parent_binding(Some(&restored), &next_parent, &child).is_err());
    assert!(verify_parent_binding(None, &parent, &child).is_err());
    assert!(
        verify_effective_permissions(
            Some(&restored),
            "codex",
            &Path::new(&parent.working_directory).join("other"),
            &captured_permissions(READ_ONLY)
        )
        .is_err()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn fixed_claude_profile_is_bound_to_the_saved_parent_generation_and_same_child_scope() {
    let mut task = parent(json!({}));
    task.harness = "claude".into();
    let profile = json!({"version":1,"workingDirectory":task.working_directory,
        "canonicalWorkingDirectory":std::fs::canonicalize(&task.working_directory).unwrap(),
        "executableSha256":"953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb",
        "denyRules":[],"sourceRules":[],"localTools":{"allow_spawn":true,"allow_message":true}});
    let effective = json!({"permissionMode":"default","fixedProfileVerified":true,"claudeRestrictedFilesV1":profile});
    let saved_config = |version| {
        json!({"cli_version":version,"permission_policy":"ClaudeRestrictedFilesV1",
        "claude_profile":profile,"effective_permissions":effective})
        .to_string()
    };
    for version in ["2.1.273", "2.1.278"] {
        task.config_json = saved_config(version);
        assert!(ceiling_from_parent(&task, "claude").is_ok());
    }
    task.config_json = saved_config("2.1.279");
    assert!(ceiling_from_parent(&task, "claude").is_err());
    task.config_json = saved_config("2.1.278");
    let ceiling = ceiling_from_parent(&task, "claude").unwrap();
    assert!(ceiling.claude_profile().is_some());
    verify_effective_permissions(
        Some(&ceiling),
        "claude",
        Path::new(&task.working_directory),
        &effective,
    )
    .unwrap();
    let mut changed_mode = effective.clone();
    changed_mode["permissionMode"] = json!("plan");
    assert!(
        verify_effective_permissions(
            Some(&ceiling),
            "claude",
            Path::new(&task.working_directory),
            &changed_mode
        )
        .is_err()
    );
    let mut widened = effective.clone();
    widened["claudeRestrictedFilesV1"]["workingDirectory"] =
        json!(std::env::temp_dir().join("other"));
    assert!(
        verify_effective_permissions(
            Some(&ceiling),
            "claude",
            Path::new(&task.working_directory),
            &widened
        )
        .is_err()
    );
    let mut child = task.clone();
    child.task_id = "child".into();
    child.parent_task_id = Some(task.task_id.clone());
    child.parent_generation = Some(task.generation);
    verify_parent_binding(Some(&ceiling), &task, &child).unwrap();
    child.parent_generation = Some(task.generation + 1);
    assert!(verify_parent_binding(Some(&ceiling), &task, &child).is_err());
    assert!(ceiling_from_parent(&task, "codex").is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn fixed_claude_profile_requires_the_saved_policy_and_cannot_be_inferred_from_mode() {
    let mut task = parent(json!({"permissionMode":"default","fixedProfileVerified":true}));
    task.harness = "claude".into();
    task.config_json = json!({"cli_version":"2.1.273","permission_policy":"Inherit",
        "effective_permissions":{"permissionMode":"default","fixedProfileVerified":true}})
    .to_string();
    assert!(ceiling_from_parent(&task, "claude").is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn fixed_grok_creation_policy_binds_parent_without_claiming_native_verification() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    for (version, digest) in [
        (
            "1.0.30",
            "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb",
        ),
        (
            "1.0.34",
            "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e",
        ),
    ] {
        let profile = GrokCreationPolicyV1::compile(
            &cwd,
            version,
            digest.into(),
            "b".repeat(64),
            Some(super::super::local_tools::LocalToolPermissions {
                allow_spawn: true,
                allow_message: true,
            }),
            super::super::PermissionPolicy::GrokRestrictedReadV1,
        )
        .unwrap();
        let observed = json!({"requestedPolicy":"GrokRestrictedReadV1","appCreationPolicyApplied":true,"permissionEnforcementVerified":false,"grokCreationPolicyV1":profile});
        let mut task = parent(observed.clone());
        task.harness = "grok".into();
        task.working_directory = cwd.to_string_lossy().into_owned();
        task.config_json = json!({"cli_version":version,"permission_policy":"GrokRestrictedReadV1","grok_profile":profile,"effective_permissions":observed}).to_string();
        let ceiling = ceiling_from_parent(&task, "grok").unwrap();
        let child = profile.derive_child(None).unwrap();
        verify_effective_permissions(Some(&ceiling), "grok", &cwd, &json!({"requestedPolicy":"GrokRestrictedReadV1","appCreationPolicyApplied":true,"permissionEnforcementVerified":false,"grokCreationPolicyV1":child})).unwrap();
        assert!(verify_effective_permissions(Some(&ceiling), "grok", &cwd, &json!({"requestedPolicy":"GrokRestrictedReadV1","appCreationPolicyApplied":true,"permissionEnforcementVerified":true,"grokCreationPolicyV1":child})).is_err());
        let mut inherited: Value = serde_json::from_str(&task.config_json).unwrap();
        inherited["permission_policy"] = json!("Inherit");
        task.config_json = inherited.to_string();
        assert!(ceiling_from_parent(&task, "grok").is_err());
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn current_grok_fixed_profile_cannot_become_a_parent_permission_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let profile = GrokCreationPolicyV1::compile(
        &cwd,
        "1.0.40",
        "3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2".into(),
        "b".repeat(64),
        None,
        super::super::PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();
    let observed = json!({
        "requestedPolicy":"GrokRestrictedReadV1",
        "appCreationPolicyApplied":true,
        "permissionEnforcementVerified":false,
        "grokCreationPolicyV1":profile
    });
    let mut task = parent(observed.clone());
    task.harness = "grok".into();
    task.working_directory = cwd.to_string_lossy().into_owned();
    task.config_json = json!({
        "cli_version":"1.0.40",
        "permission_policy":"GrokRestrictedReadV1",
        "grok_profile":profile,
        "effective_permissions":observed
    })
    .to_string();

    assert!(ceiling_from_parent(&task, "grok").is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn fixed_file_parent_and_child_require_the_saved_and_reported_tool_set_to_match() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let profile = GrokCreationPolicyV1::compile(
        &cwd,
        "1.0.30",
        "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb".into(),
        "b".repeat(64),
        Some(super::super::local_tools::LocalToolPermissions {
            allow_spawn: true,
            allow_message: true,
        }),
        super::super::PermissionPolicy::GrokRestrictedFilesV1,
    )
    .unwrap();
    let observed = json!({"requestedPolicy":"GrokRestrictedFilesV1","appCreationPolicyApplied":true,
        "permissionEnforcementVerified":false,"grokCreationPolicyV1":profile});
    let mut task = parent(observed.clone());
    task.harness = "grok".into();
    task.working_directory = cwd.to_string_lossy().into_owned();
    let config = json!({"cli_version":"1.0.30","permission_policy":"GrokRestrictedFilesV1",
        "grok_profile":profile,"effective_permissions":observed});
    task.config_json = config.to_string();
    let ceiling = ceiling_from_parent(&task, "grok").unwrap();
    let child = profile.derive_child(None).unwrap();
    let mut actual = json!({"requestedPolicy":"GrokRestrictedFilesV1","appCreationPolicyApplied":true,
        "permissionEnforcementVerified":false,"grokCreationPolicyV1":child});
    verify_effective_permissions(Some(&ceiling), "grok", &cwd, &actual).unwrap();
    actual["requestedPolicy"] = json!("GrokRestrictedReadV1");
    assert!(verify_effective_permissions(Some(&ceiling), "grok", &cwd, &actual).is_err());
    let mut mismatched = config.clone();
    mismatched["permission_policy"] = json!("GrokRestrictedReadV1");
    task.config_json = mismatched.to_string();
    assert!(ceiling_from_parent(&task, "grok").is_err());
    mismatched = config;
    mismatched["effective_permissions"]["requestedPolicy"] = json!("GrokRestrictedReadV1");
    task.config_json = mismatched.to_string();
    assert!(ceiling_from_parent(&task, "grok").is_err());
}
