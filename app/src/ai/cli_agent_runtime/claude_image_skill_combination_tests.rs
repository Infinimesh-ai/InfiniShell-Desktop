//! 图片与多技能的编码、注册和审批回归；不执行真实 CLI，也不替代组合验收。

use crate::ai::cli_agent_runtime::local_skills::{
    SelectedLocalSkill, prepare_empty_or_selected_claude_skill_plugin,
};

use super::*;

fn selected_skill(directory: &TempDir, name: &str) -> SelectedLocalSkill {
    let root = directory.path().join(name);
    fs::create_dir(&root).unwrap();
    let path = root.join("SKILL.md");
    fs::write(
        &path,
        format!("---\nname: {name}\ndescription: 组合验收技能\n---\nPRIVATE_SKILL_BODY_{name}\n"),
    )
    .unwrap();
    SelectedLocalSkill {
        name: name.into(),
        path,
    }
}

fn skill_input(skill: &SelectedLocalSkill) -> InputContent {
    InputContent::Skill {
        name: skill.name.clone(),
        path: skill.path.clone(),
    }
}

fn with_skills(directory: &TempDir, skills: &[SelectedLocalSkill]) -> ClaudeProtocol {
    let mut protocol = scoped_protocol(directory);
    protocol.options.selected_skills = skills.to_vec();
    protocol.skill_plugin = Some(prepare_empty_or_selected_claude_skill_plugin(skills).unwrap());
    protocol
}

fn reload_reply(protocol: &ClaudeProtocol, request: &Value, names: &[&str]) -> Value {
    json!({"type":"control_response","response":{"subtype":"success","request_id":request["request_id"],"response":{
        "error_count":0,
        "commands":names.iter().map(|name|json!({"name":name})).collect::<Vec<_>>(),
        "plugins":[{"name":"infinishell-local-skills","version":"0.1.0","path":protocol.skill_plugin.as_ref().unwrap().plugin_directory()}]
    }}})
}

#[test]
fn image_multi_skill_preserves_selected_order_bytes_and_native_replay() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let alpha = selected_skill(&directory, "alpha");
    let beta = selected_skill(&directory, "beta");
    let mut protocol = with_skills(&directory, &[alpha.clone(), beta.clone()]);
    let id = Uuid::from_u128(700);
    let request = command(
        id,
        RuntimeAction::Submit {
            input: vec![
                skill_input(&beta),
                InputContent::Text("中文图片问题\n保留 `$()`".into()),
                InputContent::LocalImage(path.clone()),
                skill_input(&alpha),
            ],
        },
    );

    let sent = protocol.command(request.clone());

    assert!(sent.events.is_empty());
    assert_eq!(sent.writes.len(), 1);
    let content = &sent.writes[0]["message"]["content"];
    assert_eq!(
        content[0]["text"],
        "Invoke the Skill tool for each registered skill in order: [\"infinishell-local-skills:beta\",\"infinishell-local-skills:alpha\"]. Apply each skill to the attached images after its tool call completes.\n\n中文图片问题\n保留 `$()`"
    );
    assert_eq!(
        content[1]["source"]["data"],
        STANDARD.encode(fs::read(&path).unwrap())
    );
    assert!(!content.to_string().contains("PRIVATE_SKILL_BODY"));
    let accepted = protocol.receive(replay(&sent.writes[0])).unwrap();
    assert_eq!(
        accepted.events,
        vec![RuntimeEventKind::MessageAccepted {
            message_id: id,
            turn_id: Some(id.to_string())
        }]
    );
    fs::remove_file(path).unwrap();
    assert!(protocol.command(request).writes.is_empty());
}

#[test]
fn image_multi_skill_waits_for_every_registration_before_dispatch() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let alpha = selected_skill(&directory, "alpha");
    let beta = selected_skill(&directory, "beta");
    let mut protocol = with_skills(&directory, &[]);
    let id = Uuid::from_u128(701);
    let request = command(
        id,
        RuntimeAction::Submit {
            input: vec![
                InputContent::LocalImage(path.clone()),
                skill_input(&beta),
                skill_input(&alpha),
            ],
        },
    );

    let pending = protocol.command(request.clone());

    assert_eq!(pending.writes.len(), 1);
    assert_eq!(pending.writes[0]["request"]["subtype"], "reload_plugins");
    assert!(protocol.turns.is_empty());
    assert!(protocol.command(request).writes.is_empty());
    let ack = reload_reply(
        &protocol,
        &pending.writes[0],
        &[
            "infinishell-local-skills:alpha",
            "infinishell-local-skills:beta",
        ],
    );
    let sent = protocol.receive(ack.clone()).unwrap();
    assert_eq!(sent.writes.len(), 1);
    assert_eq!(sent.writes[0]["type"], "user");
    assert_eq!(
        sent.writes[0]["message"]["content"][1]["source"]["data"],
        STANDARD.encode(fs::read(path).unwrap())
    );
    assert_eq!(protocol.options.selected_skills, vec![beta, alpha]);
    assert!(!protocol.turns[&id].accepted);
    assert!(protocol.receive(ack).unwrap().writes.is_empty());
}

#[test]
fn missing_image_skill_registration_keeps_images_and_sends_no_partial_turn() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let alpha = selected_skill(&directory, "alpha");
    let beta = selected_skill(&directory, "beta");
    let mut protocol = with_skills(&directory, &[]);
    let id = Uuid::from_u128(702);
    let pending = protocol.command(command(
        id,
        RuntimeAction::Submit {
            input: vec![
                InputContent::LocalImage(path.clone()),
                skill_input(&alpha),
                skill_input(&beta),
            ],
        },
    ));
    let ack = reload_reply(
        &protocol,
        &pending.writes[0],
        &["infinishell-local-skills:alpha"],
    );

    let rejected = protocol.receive(ack).unwrap();

    assert!(rejected.writes.is_empty());
    assert!(
        matches!(rejected.events.as_slice(), [RuntimeEventKind::RequestFailed { message_id, .. }] if *message_id == id)
    );
    assert!(protocol.turns.is_empty());
    assert!(protocol.options.selected_skills.is_empty());
    assert!(path.is_file());
    assert!(
        protocol
            .skill_plugin
            .as_ref()
            .unwrap()
            .selected()
            .is_empty()
    );
}

#[test]
fn image_multi_skill_rejects_an_unverified_claude_version() {
    let (directory, path) = image_scope(ImageFormat::Png);
    let alpha = selected_skill(&directory, "alpha");
    let beta = selected_skill(&directory, "beta");
    let mut protocol = with_skills(&directory, &[alpha.clone(), beta.clone()]);
    protocol.probed_version = Some("2.1.278");

    let rejected = protocol.command(command(
        Uuid::from_u128(703),
        RuntimeAction::Submit {
            input: vec![
                InputContent::LocalImage(path),
                skill_input(&alpha),
                skill_input(&beta),
            ],
        },
    ));

    assert!(rejected.writes.is_empty());
    assert!(protocol.turns.is_empty());
    assert!(
        matches!(rejected.events.as_slice(), [RuntimeEventKind::RequestFailed { message, .. }]
        if message == &crate::t!("cli-agent-claude-skill-version-required"))
    );
}

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(windows, target_arch = "x86_64")
))]
mod fixed {
    use crate::ai::cli_agent_runtime::claude_profile::{
        ClaudeFileProfile, ClaudeRestrictedFilesV1, ClaudeRestrictedSkillsV1,
    };

    use super::*;

    fn profile(directory: &TempDir, selected: &[SelectedLocalSkill]) -> ClaudeRestrictedSkillsV1 {
        let digest = match std::env::consts::OS {
            "macos" => "387a5c5dcdbb815085edf0baf79591f9d8894efe922bceaf3d75b1b08055229d",
            "linux" => "1e08503dbdf3c2cb0d706d32f3408277388d1c76ef108673e8fe42c1b322925b",
            "windows" => "0e4195524b73eb77efbdf3e2b36de5322a29f0ca575dfd2d9b4f946b1d425469",
            _ => unreachable!("仅编译三个目标平台的固定来源夹具"),
        };
        let cwd = directory.path().canonicalize().unwrap();
        let base: ClaudeRestrictedFilesV1 = serde_json::from_value(json!({
            "version":1,"workingDirectory":cwd,"canonicalWorkingDirectory":cwd,
            "executableSha256":digest,"denyRules":[],"sourceRules":[],"localTools":null
        }))
        .unwrap();
        ClaudeRestrictedSkillsV1::compile(base, selected).unwrap()
    }

    fn fixed_protocol(directory: &TempDir, selected: &[SelectedLocalSkill]) -> ClaudeProtocol {
        let mut protocol = with_skills(directory, selected);
        protocol.options.cwd = directory.path().canonicalize().unwrap();
        protocol.options.permission_policy = PermissionPolicy::ClaudeRestrictedSkillsV1;
        protocol.options.claude_profile =
            Some(ClaudeFileProfile::SkillsV1(profile(directory, selected)));
        // 本用例从权限观察和注册确认之后的状态检查组合输入；不替代真实原生确认。
        protocol.fixed_skill_registration_verified = true;
        protocol.profile_command_authorized = true;
        protocol
    }

    #[test]
    fn fixed_image_multi_skill_requires_registration_and_unchanged_plugin() {
        let (directory, path) = image_scope(ImageFormat::Png);
        let alpha = selected_skill(&directory, "alpha");
        let beta = selected_skill(&directory, "beta");
        let mut protocol = fixed_protocol(&directory, &[alpha.clone(), beta.clone()]);
        let input = vec![
            InputContent::LocalImage(path),
            skill_input(&beta),
            skill_input(&alpha),
        ];
        protocol.fixed_skill_registration_verified = false;
        assert!(protocol.encode_fixed_skill_input(input.clone()).is_err());
        protocol.fixed_skill_registration_verified = true;
        let encoded = protocol.encode_fixed_skill_input(input.clone()).unwrap();
        assert!(matches!(encoded, InputProjection::Blocks { .. }));
        fs::write(
            protocol
                .skill_plugin
                .as_ref()
                .unwrap()
                .plugin_directory()
                .join("skills/beta/SKILL.md"),
            "改变注册副本",
        )
        .unwrap();
        assert!(protocol.encode_fixed_skill_input(input).is_err());
    }

    #[test]
    fn fixed_image_multi_skill_cannot_expand_the_parent_skill_set() {
        let (directory, path) = image_scope(ImageFormat::Png);
        let alpha = selected_skill(&directory, "alpha");
        let beta = selected_skill(&directory, "beta");
        let parent = profile(&directory, &[alpha.clone(), beta.clone()]);
        let child = parent.derive_child(&[alpha.clone()]).unwrap();
        assert!(parent.allows_child(&child));
        assert!(child.derive_child(&[alpha.clone(), beta.clone()]).is_err());
        let mut protocol = fixed_protocol(&directory, &[alpha.clone()]);
        protocol.options.claude_profile = Some(ClaudeFileProfile::SkillsV1(child));

        let rejected = protocol.command(command(
            Uuid::from_u128(704),
            RuntimeAction::Submit {
                input: vec![
                    InputContent::LocalImage(path.clone()),
                    skill_input(&alpha),
                    skill_input(&beta),
                ],
            },
        ));

        assert!(rejected.writes.is_empty());
        assert!(protocol.turns.is_empty());
        assert!(path.is_file());
    }

    #[test]
    fn fixed_image_multi_skill_keeps_each_native_approval_independent() {
        let (directory, path) = image_scope(ImageFormat::Png);
        let alpha = selected_skill(&directory, "alpha");
        let beta = selected_skill(&directory, "beta");
        let mut protocol = fixed_protocol(&directory, &[alpha.clone(), beta.clone()]);
        let id = Uuid::from_u128(705);
        let sent = protocol.command(command(
            id,
            RuntimeAction::Submit {
                input: vec![
                    InputContent::LocalImage(path),
                    skill_input(&alpha),
                    skill_input(&beta),
                ],
            },
        ));
        assert_eq!(sent.writes.len(), 1);
        protocol.receive(replay(&sent.writes[0])).unwrap();
        protocol.receive(lifecycle(id, "started")).unwrap();
        protocol.receive(json!({"type":"assistant","uuid":Uuid::from_u128(706),
            "session_id":"3ddff71c-4062-4198-a130-502e4c15684e","user_message_uuid":id,
            "message":{"id":"image-skills-assistant","role":"assistant","content":[
                {"type":"tool_use","id":"skill-alpha","name":"Skill","input":{"skill":"infinishell-local-skills:alpha"}},
                {"type":"tool_use","id":"skill-beta","name":"Skill","input":{"skill":"infinishell-local-skills:beta"}}
            ]}})).unwrap();
        let alpha_request = json!({"type":"control_request","request_id":"approve-alpha","request":{
            "subtype":"can_use_tool","tool_name":"Skill","tool_use_id":"skill-alpha","input":{"skill":"infinishell-local-skills:alpha"}}});
        let beta_request = json!({"type":"control_request","request_id":"approve-beta","request":{
            "subtype":"can_use_tool","tool_name":"Skill","tool_use_id":"skill-beta","input":{"skill":"infinishell-local-skills:beta"}}});
        assert!(
            matches!(protocol.receive(alpha_request.clone()).unwrap().events.as_slice(), [RuntimeEventKind::ApprovalRequested { approval_id, .. }] if approval_id == "approve-alpha")
        );
        assert!(
            matches!(protocol.receive(beta_request.clone()).unwrap().events.as_slice(), [RuntimeEventKind::ApprovalRequested { approval_id, .. }] if approval_id == "approve-beta")
        );

        let allowed = protocol.command(command(
            Uuid::from_u128(707),
            RuntimeAction::RespondApproval {
                approval_id: "approve-alpha".into(),
                decision: ApprovalDecision::AllowOnce,
            },
        ));
        assert_eq!(
            allowed.writes[0]["response"]["response"],
            json!({"behavior":"allow","updatedInput":{"skill":"infinishell-local-skills:alpha"}})
        );
        assert!(protocol.approvals["approve-beta"].response.is_none());
        assert_eq!(
            protocol.receive(alpha_request).unwrap().writes[0]["response"]["subtype"],
            "error"
        );
        let denied = protocol.command(command(
            Uuid::from_u128(708),
            RuntimeAction::RespondApproval {
                approval_id: "approve-beta".into(),
                decision: ApprovalDecision::DenyOnce,
            },
        ));
        assert_eq!(denied.writes[0]["response"]["response"]["behavior"], "deny");
        assert!(
            denied.writes[0]["response"]["response"]
                .get("updatedPermissions")
                .is_none()
        );
    }
}
