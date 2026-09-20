use super::*;
use tempfile::TempDir;

fn files() -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let cwd = dunce::canonicalize(temp.path()).unwrap();
    let skill = cwd.join(".grok/skills/test/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(&skill, "只在此文件中的随机标记").unwrap();
    (temp, cwd, skill)
}

fn read_history(skill: &Path) -> Vec<Value> {
    vec![
        json!({"method":"session/update","params":{"update":{
            "sessionUpdate":"tool_call","toolCallId":"read-1","kind":"other","status":"pending",
            "_meta":{"x.ai/tool":{"name":"read_file"}},"rawInput":{"target_file":skill}}}}),
        json!({"method":"session/update","params":{"update":{
            "sessionUpdate":"tool_call_update","toolCallId":"read-1","kind":"read",
            "_meta":{"x.ai/tool":{"name":"read_file"}},"rawInput":{"variant":"ReadFile","target_file":skill}}}}),
        json!({"method":"session/update","params":{"update":{
            "sessionUpdate":"tool_call_update","toolCallId":"read-1","status":"completed"}}}),
    ]
}

#[test]
fn exact_read_allows_only_the_generated_file_without_extra_arguments() {
    let (_temp, cwd, skill) = files();
    assert!(exact_skill_read_input(
        &json!({"target_file":skill}),
        &cwd,
        &skill,
        false
    ));
    assert!(exact_skill_read_input(
        &json!({"target_file":".grok/skills/test/SKILL.md"}),
        &cwd,
        &skill,
        false
    ));
    let other = cwd.join("other");
    fs::write(&other, "其它文件").unwrap();
    for input in [
        json!({"target_file":other}),
        json!({"target_file":skill,"offset":0}),
        json!({"target_file":skill,"command":"cat"}),
        json!({"file_path":skill}),
        json!({"target_file":skill,"variant":"Write"}),
    ] {
        assert!(!exact_skill_read_input(&input, &cwd, &skill, false));
        assert!(!exact_skill_read_input(&input, &cwd, &skill, true));
    }
}

#[test]
fn read_permission_requires_native_read_kind_name_and_tagged_input() {
    let (_temp, cwd, skill) = files();
    let call = json!({"toolCallId":"read-1","kind":"read",
        "_meta":{"x.ai/tool":{"name":"read_file"}},
        "rawInput":{"variant":"ReadFile","target_file":skill}});
    assert!(exact_skill_read_permission(&call, &cwd, &skill));
    let mut changed = call.clone();
    changed["kind"] = json!("edit");
    assert!(!exact_skill_read_permission(&changed, &cwd, &skill));
    let mut changed = call.clone();
    changed["_meta"]["x.ai/tool"]["name"] = json!("use_tool");
    assert!(!exact_skill_read_permission(&changed, &cwd, &skill));
    let mut changed = call;
    changed["rawInput"]["tool_name"] = json!("infinishell-local-tasks__inspect_local_tasks");
    assert!(!exact_skill_read_permission(&changed, &cwd, &skill));
}

#[test]
fn visible_skill_read_requires_one_completed_native_call_and_hidden_allows_none() {
    let (_temp, cwd, skill) = files();
    let history = read_history(&skill);
    assert_eq!(
        verify_skill_read_history(&history, &cwd, &skill, "visible", &HashSet::new()),
        Ok((3, 1))
    );
    assert_eq!(
        verify_skill_read_history(&history, &cwd, &skill, "visible", &["read-1".into()].into()),
        Ok((3, 1))
    );
    assert!(verify_skill_read_history(&history, &cwd, &skill, "hidden", &HashSet::new()).is_err());
    assert_eq!(
        verify_skill_read_history(&[], &cwd, &skill, "hidden", &HashSet::new()),
        Ok((0, 0))
    );
    assert!(
        verify_skill_read_history(&history[..2], &cwd, &skill, "visible", &HashSet::new()).is_err()
    );
    assert!(
        verify_skill_read_history(
            &history,
            &cwd,
            &skill,
            "visible",
            &["old-call".into()].into()
        )
        .is_err()
    );
}

#[test]
fn replay_rejects_changed_inputs_unknown_tools_and_duplicate_calls() {
    let (_temp, cwd, skill) = files();
    for field in ["input", "identity", "duplicate", "status"] {
        let mut history = read_history(&skill);
        match field {
            "input" => history[1]["params"]["update"]["rawInput"]["target_file"] = json!(cwd),
            "identity" => {
                history[1]["params"]["update"]["_meta"]["x.ai/tool"]["name"] = json!("write")
            }
            "duplicate" => history.push(history[0].clone()),
            "status" => history[2]["params"]["update"]["status"] = json!("failed"),
            _ => unreachable!(),
        }
        assert!(
            verify_skill_read_history(&history, &cwd, &skill, "visible", &HashSet::new()).is_err()
        );
    }
}

#[test]
#[cfg(unix)]
fn read_path_alias_must_resolve_to_the_same_private_skill_file() {
    let (_temp, cwd, skill) = files();
    let alias = cwd.join("alias");
    std::os::unix::fs::symlink(&skill, &alias).unwrap();
    assert!(exact_skill_read_input(
        &json!({"target_file":alias}),
        &cwd,
        &skill,
        false
    ));
    fs::remove_file(&alias).unwrap();
    let other = cwd.join("other");
    fs::write(&other, "不同文件").unwrap();
    std::os::unix::fs::symlink(&other, &alias).unwrap();
    assert!(!exact_skill_read_input(
        &json!({"target_file":alias}),
        &cwd,
        &skill,
        false
    ));
}

#[test]
fn native_minimal_tool_start_requires_later_exact_parameters() {
    let (_temp, cwd, skill) = files();
    let mut history = read_history(&skill);
    history[0]["params"]["update"]["rawInput"] = Value::Null;
    assert_eq!(
        verify_skill_read_history(&history, &cwd, &skill, "visible", &HashSet::new()),
        Ok((3, 1))
    );
    history.remove(1);
    assert!(verify_skill_read_history(&history, &cwd, &skill, "visible", &HashSet::new()).is_err());
}

fn catalog_frame(session: &str, skill: &Path) -> Value {
    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,
        "update":{"sessionUpdate":"available_commands_update","availableCommands":[{
            "name":SKILL_NAME,"description":"不应出现在公开诊断中的正文",
            "_meta":{"path":skill,"scope":"local","bareName":SKILL_NAME,"qualifiedName":SKILL_NAME,
                "unrecognized":"不应公开的任意元数据"}}]}}})
}

#[test]
fn catalog_projection_preserves_only_selected_identity_and_safe_metadata_facts() {
    let (_temp, _cwd, skill) = files();
    let frame = catalog_frame("session-a", &skill);
    let projection = project_skill_catalog(
        &frame["params"]["update"]["availableCommands"],
        &skill,
        true,
    )
    .unwrap();
    assert_eq!(projection["path_matches_selected"], true);
    assert_eq!(projection["metadata_scope"], "local");
    assert_eq!(projection["bare_name_matches"], true);
    assert_eq!(projection["qualified_name_matches"], true);
    assert_eq!(projection["before_first_submit"], true);
    let encoded = projection.to_string();
    assert!(!encoded.contains(&skill.to_string_lossy().to_string()));
    assert!(!encoded.contains("不应"));
}

#[test]
fn catalog_path_and_duplicate_names_cannot_become_verified_bindings() {
    let (_temp, cwd, skill) = files();
    let other = cwd.join("other");
    fs::write(&other, "不是所选技能").unwrap();
    for path in [
        json!(other),
        json!(".grok/skills/test/SKILL.md"),
        Value::Null,
    ] {
        let mut frame = catalog_frame("session-a", &skill);
        frame["params"]["update"]["availableCommands"][0]["_meta"]["path"] = path;
        let projection = project_skill_catalog(
            &frame["params"]["update"]["availableCommands"],
            &skill,
            false,
        )
        .unwrap();
        assert_eq!(projection["path_matches_selected"], false);
        assert_eq!(projection["before_first_submit"], false);
    }
    let frame = catalog_frame("session-a", &skill);
    let command = frame["params"]["update"]["availableCommands"][0].clone();
    let projection = project_skill_catalog(&json!([command, command]), &skill, true).unwrap();
    assert_eq!(projection["selected_name_count"], 2);
    assert_eq!(projection["path_matches_selected"], false);
    assert_eq!(projection["metadata_present"], false);
}

#[test]
fn catalog_probe_associates_pre_ready_notifications_only_with_real_session() {
    let (_temp, _cwd, skill) = files();
    let probe = SkillCatalogProbe::new(skill.clone());
    let frame = catalog_frame("session-a", &skill);
    probe.observe(&frame, true);
    probe.observe(&frame, true);
    probe.observe(&catalog_frame("old-session", &skill), true);
    let evidence = probe.evidence(Some("session-a"), "visible");
    assert_eq!(evidence["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(evidence["unassociated_snapshot_count"], 1);
    assert!(!evidence.to_string().contains("session-a"));
    assert!(!evidence.to_string().contains("old-session"));
    assert!(
        probe.evidence(None, "visible")["snapshots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn catalog_probe_does_not_promote_requests_replay_or_unbounded_notifications() {
    let (_temp, _cwd, skill) = files();
    let probe = SkillCatalogProbe::new(skill.clone());
    let mut request = catalog_frame("session-a", &skill);
    request["id"] = json!(7);
    probe.observe(&request, true);
    let mut replay = catalog_frame("session-a", &skill);
    replay["params"]["_meta"] = json!({"isReplay":true});
    probe.observe(&replay, true);
    assert!(
        probe.evidence(Some("session-a"), "visible")["snapshots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for index in 0..=MAX_CATALOG_SNAPSHOTS {
        let mut frame = catalog_frame("session-a", &skill);
        frame["params"]["_meta"] = json!({"eventId":format!("event-{index}")});
        probe.observe(&frame, true);
    }
    let evidence = probe.evidence(Some("session-a"), "visible");
    assert_eq!(evidence["overflow"], true);
    assert_eq!(
        evidence["snapshots"].as_array().unwrap().len(),
        MAX_CATALOG_SNAPSHOTS
    );
}
