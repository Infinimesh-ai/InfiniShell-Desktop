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
