//! 固定 Grok 图片与技能的适配器回归；不执行原生 CLI，不把夹具作为模型验收。

use std::fs;

use super::*;

fn skill(root: &Path, name: &str) -> SelectedLocalSkill {
    let path = root.join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!("---\nname: {name}\ndescription: 组合技能\nuser-invocable: true\n---\nPRIVATE_SKILL_BODY_{name}\n"),
    )
    .unwrap();
    SelectedLocalSkill {
        name: name.into(),
        path,
    }
}

fn entry(skill: &SelectedLocalSkill) -> Value {
    json!({"name":skill.name,"description":"原生目录夹具","input":null,
        "_meta":{"path":skill.path,"bareName":skill.name,"qualifiedName":format!("local:{}",skill.name),"scope":"local"}})
}

fn skill_input(skill: &SelectedLocalSkill) -> InputContent {
    InputContent::Skill {
        name: skill.name.clone(),
        path: skill.path.clone(),
    }
}

fn protocol(root: &TempDir, selected: &[SelectedLocalSkill]) -> GrokProtocol {
    let mut options = super::super::tests::options();
    options.state_dir = root.path().to_owned();
    options.selected_skills = selected.to_vec();
    let mut protocol = GrokProtocol::new(options);
    protocol.bind_cli_version(ROOT_VERSION_OUTPUT).unwrap();
    protocol.paired_version = Some(ROOT_VERSION);
    protocol.session_id = Some("image-skill-session".into());
    protocol.reported_metadata.models = Some(ReportedModels {
        current_model_id: "grok-4.7".into(),
        available_models: Vec::new(),
    });
    // 仅测试中提供已派生状态；生产值只能来自私有 socket 对应的真实进程。
    protocol.owned_skill_leader = true;
    protocol.skill_catalog = Some(
        skills::SkillCatalog::from_native(&json!(selected.iter().map(entry).collect::<Vec<_>>()))
            .unwrap(),
    );
    protocol
}

fn request(protocol: &GrokProtocol, id: Uuid, input: Vec<InputContent>) -> RuntimeCommand {
    RuntimeCommand {
        generation: protocol.options.generation,
        message_id: id,
        action: RuntimeAction::Submit { input },
    }
}

fn reload_reply(protocol: &mut GrokProtocol, reload: &Value) -> Effects {
    protocol
        .receive(json!({"jsonrpc":"2.0","id":reload["id"],"result":{"result":{"reloaded":1}}}))
        .unwrap()
}

fn catalog_reply(catalog: &Value, selected: &[SelectedLocalSkill]) -> Value {
    json!({"jsonrpc":"2.0","id":catalog["id"],"result":{
        "commands":selected.iter().map(entry).collect::<Vec<_>>(),"tools":["read_file"]}})
}

#[test]
fn single_skill_image_preserves_original_bytes_and_duplicate_input_is_not_resent() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let (mut input, data) = png_input(&root.path().join("local-cli-attachments"));
    input.push(skill_input(&alpha));
    let mut protocol = protocol(&root, &[alpha]);
    let command = request(&protocol, Uuid::new_v4(), input);

    let effects = protocol.command(command.clone());

    assert!(effects.events.is_empty());
    assert_eq!(effects.writes.len(), 1);
    assert_eq!(
        effects.writes[0]["params"]["prompt"],
        json!([
            {"type":"text","text":"/alpha 图片\n第二行"},
            {"type":"image","mimeType":"image/png","data":data}
        ])
    );
    assert!(!effects.writes[0].to_string().contains("PRIVATE_SKILL_BODY"));
    let duplicate = protocol.command(command);
    assert!(duplicate.writes.is_empty());
    assert!(duplicate.events.is_empty());
}

#[test]
fn multiple_skills_keep_selected_order_before_text_and_original_image() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let (mut input, data) = png_input(&root.path().join("local-cli-attachments"));
    input.extend([skill_input(&beta), skill_input(&alpha)]);
    let mut protocol = protocol(&root, &[alpha, beta]);
    let command = request(&protocol, Uuid::new_v4(), input);

    let effects = protocol.command(command);

    assert_eq!(
        effects.writes[0]["params"]["prompt"],
        json!([
            {"type":"text","text":"/beta"},{"type":"text","text":"/alpha"},
            {"type":"text","text":"图片\n第二行"},
            {"type":"image","mimeType":"image/png","data":data}
        ])
    );
    assert!(!effects.writes[0].to_string().contains("PRIVATE_SKILL_BODY"));
}

#[test]
fn image_only_skill_and_image_before_text_both_put_native_slash_first() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let (input, data) = png_input(&root.path().join("local-cli-attachments"));
    for text in [None, Some("中文原文")] {
        let mut input = vec![input[1].clone(), skill_input(&alpha)];
        if let Some(text) = text {
            input.push(InputContent::Text(text.into()));
        }
        let mut protocol = protocol(&root, &[alpha.clone()]);
        let command = request(&protocol, Uuid::new_v4(), input);

        let effects = protocol.command(command);

        let expected = text.map_or("/alpha".to_owned(), |text| format!("/alpha {text}"));
        assert_eq!(
            effects.writes[0]["params"]["prompt"],
            json!([
                {"type":"text","text":expected},
                {"type":"image","mimeType":"image/png","data":data}
            ])
        );
    }
}

#[test]
fn image_skill_requires_both_fixed_versions_model_inherit_and_private_leader() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let (mut input, _) = png_input(&root.path().join("local-cli-attachments"));
    input.push(skill_input(&alpha));
    for mutation in 0..5 {
        let mut protocol = protocol(&root, &[alpha.clone()]);
        match mutation {
            0 => protocol.probed_version = Some(CURRENT_VERSION),
            1 => protocol.paired_version = Some(CURRENT_VERSION),
            2 => {
                protocol
                    .reported_metadata
                    .models
                    .as_mut()
                    .unwrap()
                    .current_model_id = "unknown".into()
            }
            3 => protocol.owned_skill_leader = false,
            4 => protocol.options.permission_policy = PermissionPolicy::GrokRestrictedFilesV1,
            _ => unreachable!("穷尽五种合同变更"),
        }
        let command = request(&protocol, Uuid::new_v4(), input.clone());

        let effects = protocol.command(command);

        assert!(effects.writes.is_empty(), "变更 {mutation} 不得发送图片");
        assert!(matches!(
            effects.events.as_slice(),
            [RuntimeEventKind::RequestFailed { .. }]
        ));
        assert!(protocol.prompt.is_none());
    }
}

#[test]
fn queued_image_skill_rechecks_model_version_leader_and_source_before_dispatch() {
    let root = TempDir::new().unwrap();
    for mutation in 0..4 {
        let alpha = skill(root.path(), "alpha");
        let (mut input, _) = png_input(&root.path().join("local-cli-attachments"));
        input.push(skill_input(&alpha));
        let mut protocol = protocol(&root, &[alpha.clone()]);
        let first = request(
            &protocol,
            Uuid::new_v4(),
            vec![InputContent::Text("先完成此回合".into())],
        );
        let first = protocol.command(first);
        let queued_id = Uuid::new_v4();
        let queued = request(&protocol, queued_id, input);
        assert!(protocol.command(queued).writes.is_empty());
        match mutation {
            0 => {
                protocol
                    .reported_metadata
                    .models
                    .as_mut()
                    .unwrap()
                    .current_model_id = "unknown".into()
            }
            1 => protocol.paired_version = Some(CURRENT_VERSION),
            2 => protocol.owned_skill_leader = false,
            3 => fs::write(
                &alpha.path,
                "---\nname: alpha\ndescription: 已改\n---\n不同正文",
            )
            .unwrap(),
            _ => unreachable!("穷尽四种排队变更"),
        }

        let effects = protocol.receive(json!({"jsonrpc":"2.0","id":first.writes[0]["id"],"error":{"code":-32000,"message":"结束夹具回合"}})).unwrap();

        assert!(effects.writes.is_empty());
        assert!(effects.events.iter().any(|event| matches!(event, RuntimeEventKind::RequestFailed { message_id, .. } if *message_id==queued_id)));
        assert!(protocol.prompt.is_none());
    }
}

#[test]
fn hot_image_skills_wait_for_full_catalog_ack_and_repeated_input_does_not_dispatch() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let (mut input, data) = png_input(&root.path().join("local-cli-attachments"));
    input.extend([skill_input(&alpha), skill_input(&beta)]);
    let mut protocol = protocol(&root, &[]);
    let command = request(&protocol, Uuid::new_v4(), input);
    let reload = protocol.command(command.clone());
    assert_eq!(reload.writes[0]["method"], "_x.ai/internal/reload_skills");
    assert!(protocol.prompt.is_none());
    assert!(protocol.command(command.clone()).writes.is_empty());
    let catalog = reload_reply(&mut protocol, &reload.writes[0]);
    assert_eq!(catalog.writes[0]["method"], "_x.ai/commands/list");
    assert!(protocol.prompt.is_none());
    assert!(protocol.options.selected_skills.is_empty());

    let effects = protocol
        .receive(catalog_reply(
            &catalog.writes[0],
            &[alpha.clone(), beta.clone()],
        ))
        .unwrap();

    assert_eq!(effects.writes.len(), 1);
    assert_eq!(effects.writes[0]["params"]["prompt"][3]["data"], data);
    assert_eq!(protocol.options.selected_skills, vec![alpha, beta]);
    assert!(protocol.command(command).writes.is_empty());
}

#[test]
fn partial_skill_registration_never_sends_a_partial_image_prompt() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let (mut input, _) = png_input(&root.path().join("local-cli-attachments"));
    let InputContent::LocalImage(path) = input[1].clone() else {
        panic!("缺少图片");
    };
    input.extend([skill_input(&alpha), skill_input(&beta)]);
    let mut protocol = protocol(&root, &[]);
    let id = Uuid::new_v4();
    let command = request(&protocol, id, input);
    let reload = protocol.command(command);
    let catalog = reload_reply(&mut protocol, &reload.writes[0]);

    let effects = protocol
        .receive(catalog_reply(&catalog.writes[0], &[alpha]))
        .unwrap();

    assert!(effects.writes.is_empty());
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::RequestFailed { message_id, .. }] if *message_id==id)
    );
    assert!(protocol.prompt.is_none());
    assert!(protocol.options.selected_skills.is_empty());
    assert!(path.is_file());
}

#[test]
fn attachment_change_during_registration_cannot_reach_the_model() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let (mut input, _) = png_input(&root.path().join("local-cli-attachments"));
    let InputContent::LocalImage(path) = input[1].clone() else {
        panic!("缺少图片");
    };
    input.push(skill_input(&alpha));
    let mut protocol = protocol(&root, &[]);
    let command = request(&protocol, Uuid::new_v4(), input);
    let reload = protocol.command(command);
    let catalog = reload_reply(&mut protocol, &reload.writes[0]);
    fs::write(path, "损坏图片").unwrap();

    let effects = protocol
        .receive(catalog_reply(&catalog.writes[0], &[alpha]))
        .unwrap();

    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.prompt.is_none());
    assert!(protocol.options.selected_skills.is_empty());
}

#[test]
fn model_change_during_registration_cannot_dispatch_an_image_prompt() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let (mut input, _) = png_input(&root.path().join("local-cli-attachments"));
    input.push(skill_input(&alpha));
    let mut protocol = protocol(&root, &[]);
    let command = request(&protocol, Uuid::new_v4(), input);
    let reload = protocol.command(command);
    let catalog = reload_reply(&mut protocol, &reload.writes[0]);
    protocol
        .reported_metadata
        .models
        .as_mut()
        .unwrap()
        .current_model_id = "unknown".into();

    assert!(
        protocol
            .receive(catalog_reply(&catalog.writes[0], &[alpha]))
            .is_err()
    );
    assert!(protocol.prompt.is_none());
    assert!(protocol.options.selected_skills.is_empty());
}

#[test]
fn full_image_skill_frame_budget_counts_slash_prefix_and_all_images() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let store = root.path().join("local-cli-attachments");
    let (input, _) = png_input(&store);
    let image = input[1].clone();
    let content = encode_prompt_content(
        vec![InputContent::Text(String::new()), image.clone()],
        &store,
    )
    .unwrap();
    let available = MAX_LINE_BYTES - 64 * 1024 - serde_json::to_vec(&content).unwrap().len();
    let plain = vec![InputContent::Text("x".repeat(available)), image.clone()];
    assert!(encode_prompt_content(plain.clone(), &store).is_ok());
    let selected = [skills::SelectedSkill::new(alpha.name.clone(), alpha.path.clone()).unwrap()];
    let catalog = skills::SkillCatalog::from_native(&json!([entry(&alpha)])).unwrap();
    let mut combined = plain;
    combined.push(skill_input(&alpha));
    assert_eq!(
        catalog.encode_selected(combined, &selected, &store),
        Err(crate::t!("cli-agent-runtime-data-too-large"))
    );
    let mut too_many = vec![image; MAX_IMAGE_COUNT_FOR_QUERY + 1];
    too_many.push(skill_input(&alpha));
    assert!(
        catalog
            .encode_selected(too_many, &selected, &store)
            .is_err()
    );
}
