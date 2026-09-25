use tempfile::TempDir;

use super::*;

fn skill(root: &Path, name: &str, body: &str) -> SelectedLocalSkill {
    let directory = root.join(name);
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("SKILL.md");
    std::fs::write(
        &path,
        format!("---\nname: {name}\ndescription: 验收技能\n---\n{body}\n"),
    )
    .unwrap();
    SelectedLocalSkill {
        name: name.into(),
        path,
    }
}

fn protocol(selected: &[SelectedLocalSkill]) -> ClaudeProtocol {
    let mut protocol = ClaudeProtocol::new(SessionOptions {
        executable: std::env::temp_dir().join("claude-fixture"),
        cwd: std::env::temp_dir(),
        state_dir: std::env::temp_dir(),
        target: SessionTarget::New,
        generation: Uuid::from_u128(1),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: selected.to_vec(),
    });
    protocol.initialized = true;
    protocol.probed_version = Some("2.1.280");
    protocol.skill_plugin = Some(prepare_empty_or_selected_claude_skill_plugin(selected).unwrap());
    protocol
}

fn input(skill: &SelectedLocalSkill) -> InputContent {
    InputContent::Skill {
        name: skill.name.clone(),
        path: skill.path.clone(),
    }
}

fn submit(id: u128, input: Vec<InputContent>) -> RuntimeCommand {
    RuntimeCommand {
        generation: Uuid::from_u128(1),
        message_id: Uuid::from_u128(id),
        action: RuntimeAction::Submit { input },
    }
}

fn ack(protocol: &ClaudeProtocol, request: &Value, names: &[&str]) -> Value {
    json!({"type":"control_response","response":{"subtype":"success","request_id":request["request_id"],"response":{
        "error_count":0,
        "commands":names.iter().map(|name|json!({"name":name})).collect::<Vec<_>>(),
        "plugins":[{"name":"infinishell-local-skills","version":"0.1.0","path":protocol.skill_plugin.as_ref().unwrap().plugin_directory()}]
    }}})
}

#[test]
fn restricted_file_profiles_reject_skills_before_registration_or_dispatch() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "禁止受限任务加载此技能");
    for policy in [
        PermissionPolicy::ClaudeRestrictedFilesV1,
        PermissionPolicy::ClaudeRestrictedFilesV2,
    ] {
        let mut protocol = protocol(&[]);
        protocol.options.permission_policy = policy;
        let effects = protocol.command(submit(2, vec![input(&alpha)]));
        assert!(effects.writes.is_empty());
        assert!(protocol.turns.is_empty());
        assert!(protocol.pending.is_empty());
        assert!(matches!(
            effects.events.as_slice(),
            [RuntimeEventKind::RequestFailed { message, .. }]
                if message == &crate::t!("cli-task-manager-permission-claude-files-skills")
        ));
    }
}

#[test]
fn multiple_registered_skills_reference_native_tools_without_inlining_bodies() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "ALPHA_PRIVATE_BODY");
    let beta = skill(root.path(), "beta", "BETA_PRIVATE_BODY");
    let mut protocol = protocol(&[alpha.clone(), beta.clone()]);
    let effects = protocol.command(submit(
        2,
        vec![
            input(&alpha),
            input(&beta),
            InputContent::Text("保留中文\n第二行".into()),
        ],
    ));
    assert!(effects.events.is_empty());
    assert_eq!(effects.writes.len(), 1);
    assert_eq!(
        effects.writes[0]["message"]["content"],
        "Invoke the Skill tool for each registered skill in order: [\"infinishell-local-skills:alpha\",\"infinishell-local-skills:beta\"].\n\n保留中文\n第二行"
    );
    assert!(!protocol.turns[&Uuid::from_u128(2)].accepted);
}

#[test]
fn first_hot_skill_waits_for_exact_registration_ack_and_duplicate_input_does_not_repeat() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "原生加载正文");
    let mut protocol = protocol(&[]);
    let command = submit(2, vec![input(&alpha)]);
    let effects = protocol.command(command.clone());
    assert_eq!(effects.writes[0]["request"]["subtype"], "reload_plugins");
    assert!(effects.events.is_empty());
    assert!(protocol.turns.is_empty());
    assert!(
        protocol
            .skill_plugin
            .as_ref()
            .unwrap()
            .command_for("alpha", &alpha.path)
            .is_none()
    );
    assert!(protocol.command(command).writes.is_empty());
    let blocked = protocol.command(submit(3, vec![InputContent::Text("新消息".into())]));
    assert!(matches!(
        blocked.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(blocked.writes.is_empty());

    let reply = ack(
        &protocol,
        &effects.writes[0],
        &["infinishell-local-skills:alpha"],
    );
    let confirmed = protocol.receive(reply.clone()).unwrap();
    assert_eq!(confirmed.writes.len(), 1);
    assert_eq!(
        confirmed.writes[0]["message"]["content"],
        "/infinishell-local-skills:alpha"
    );
    assert_eq!(protocol.options.selected_skills, vec![alpha]);
    assert!(confirmed.events.is_empty());
    assert!(!protocol.turns[&Uuid::from_u128(2)].accepted);
    assert!(protocol.receive(reply).unwrap().writes.is_empty());
}

#[test]
fn missing_registered_command_rolls_back_addition_and_retains_existing_skill() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "已有正文");
    let beta = skill(root.path(), "beta", "新增正文");
    let mut protocol = protocol(&[alpha.clone()]);
    let plugin = protocol
        .skill_plugin
        .as_ref()
        .unwrap()
        .plugin_directory()
        .to_owned();
    let request = protocol
        .command(submit(2, vec![input(&beta)]))
        .writes
        .remove(0);
    assert!(plugin.join("skills/beta/SKILL.md").is_file());
    let reply = ack(&protocol, &request, &["infinishell-local-skills:alpha"]);
    let failed = protocol.receive(reply).unwrap();
    assert!(failed.writes.is_empty());
    assert!(
        matches!(failed.events.as_slice(), [RuntimeEventKind::RequestFailed { message_id, .. }] if *message_id == Uuid::from_u128(2))
    );
    assert!(!plugin.join("skills/beta").exists());
    assert!(plugin.join("skills/alpha/SKILL.md").is_file());
    assert_eq!(protocol.options.selected_skills, vec![alpha]);
    assert!(protocol.turns.is_empty());
    assert!(
        protocol
            .command(submit(3, vec![InputContent::Text("不可继续".into())]))
            .writes
            .is_empty()
    );
}

#[test]
fn old_or_foreign_ack_does_not_release_waiting_input() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    let request = protocol
        .command(submit(2, vec![input(&alpha)]))
        .writes
        .remove(0);
    let mut reply = ack(&protocol, &request, &["infinishell-local-skills:alpha"]);
    reply["response"]["request_id"] = json!("infinishell-old-generation-1");
    assert!(protocol.receive(reply).unwrap().writes.is_empty());
    assert!(protocol.turns.is_empty());
    assert!(
        protocol
            .skill_plugin
            .as_ref()
            .unwrap()
            .selected()
            .is_empty()
    );
    let mut stale = submit(3, vec![input(&alpha)]);
    stale.generation = Uuid::from_u128(99);
    assert!(protocol.command(stale).writes.is_empty());
    assert!(protocol.turns.is_empty());
}

#[test]
fn unverified_plugin_path_or_error_count_cannot_release_input() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    let request = protocol
        .command(submit(2, vec![input(&alpha)]))
        .writes
        .remove(0);
    let mut reply = ack(&protocol, &request, &["infinishell-local-skills:alpha"]);
    reply["response"]["response"]["plugins"][0]["path"] = json!(root.path());
    let failed = protocol.receive(reply).unwrap();
    assert!(failed.writes.is_empty());
    assert!(protocol.skill_reload_failed);
    assert!(protocol.turns.is_empty());
}

#[test]
fn new_skill_is_rejected_while_a_previous_input_is_unconfirmed() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    assert_eq!(
        protocol
            .command(submit(2, vec![InputContent::Text("在途输入".into())]))
            .writes
            .len(),
        1
    );
    let blocked = protocol.command(submit(3, vec![input(&alpha)]));
    assert!(blocked.writes.is_empty());
    assert!(matches!(
        blocked.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(
        !protocol
            .skill_plugin
            .as_ref()
            .unwrap()
            .plugin_directory()
            .join("skills/alpha")
            .exists()
    );
}

#[test]
fn duplicate_skill_selection_does_not_start_registration() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    let effects = protocol.command(submit(2, vec![input(&alpha), input(&alpha)]));
    assert!(effects.writes.is_empty());
    assert!(protocol.pending.is_empty());
    assert!(
        !protocol
            .skill_plugin
            .as_ref()
            .unwrap()
            .plugin_directory()
            .join("skills/alpha")
            .exists()
    );
}

#[test]
fn cold_resume_rebuilds_registered_skills_from_original_paths() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "旧正文");
    let mut live = protocol(&[]);
    let request = live
        .command(submit(2, vec![input(&alpha)]))
        .writes
        .remove(0);
    let reply = ack(&live, &request, &["infinishell-local-skills:alpha"]);
    live.receive(reply).unwrap();
    let selected = live.options.selected_skills.clone();
    let old_plugin = live
        .skill_plugin
        .as_ref()
        .unwrap()
        .plugin_directory()
        .to_owned();
    drop(live);
    assert!(!old_plugin.exists());
    std::fs::write(
        &alpha.path,
        "---\nname: alpha\ndescription: 验收\n---\n冷恢复新正文\n",
    )
    .unwrap();
    let restored = protocol(&selected);
    assert_eq!(restored.options.selected_skills[0].path, alpha.path);
    assert_eq!(
        std::fs::read_to_string(
            restored
                .skill_plugin
                .as_ref()
                .unwrap()
                .plugin_directory()
                .join("skills/alpha/SKILL.md")
        )
        .unwrap(),
        "---\nname: alpha\ndescription: 验收\n---\n冷恢复新正文\n"
    );
}

#[test]
fn timed_out_registration_retains_input_and_ignores_a_late_success_ack() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    let request = protocol
        .command(submit(2, vec![input(&alpha)]))
        .writes
        .remove(0);
    let reply = ack(&protocol, &request, &["infinishell-local-skills:alpha"]);
    protocol
        .pending
        .get_mut(request["request_id"].as_str().unwrap())
        .unwrap()
        .sent_at = Instant::now() - REQUEST_TIMEOUT;
    let effects = protocol.expire_skill_registrations();
    assert!(effects.writes.is_empty());
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::RequestFailed { message_id, .. }] if *message_id == Uuid::from_u128(2))
    );
    assert!(protocol.receive(reply).unwrap().writes.is_empty());
    assert!(protocol.turns.is_empty());
    assert!(protocol.options.selected_skills.is_empty());
    assert!(
        !protocol
            .skill_plugin
            .as_ref()
            .unwrap()
            .plugin_directory()
            .join("skills/alpha")
            .exists()
    );
}

#[test]
fn native_reload_error_count_prevents_dispatch_even_when_the_skill_is_listed() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    let request = protocol
        .command(submit(2, vec![input(&alpha)]))
        .writes
        .remove(0);
    let mut reply = ack(&protocol, &request, &["infinishell-local-skills:alpha"]);
    reply["response"]["response"]["error_count"] = json!(1);
    let failed = protocol.receive(reply).unwrap();
    assert!(failed.writes.is_empty());
    assert!(protocol.skill_reload_failed);
    assert!(protocol.turns.is_empty());
}

#[test]
fn session_identity_change_while_reloading_does_not_dispatch_the_staged_input() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha", "正文");
    let mut protocol = protocol(&[]);
    let request = protocol
        .command(submit(2, vec![input(&alpha)]))
        .writes
        .remove(0);
    let reply = ack(&protocol, &request, &["infinishell-local-skills:alpha"]);
    protocol.session_id = Some(Uuid::from_u128(88).to_string());
    let failed = protocol.receive(reply).unwrap();
    assert!(failed.writes.is_empty());
    assert!(protocol.options.selected_skills.is_empty());
}
