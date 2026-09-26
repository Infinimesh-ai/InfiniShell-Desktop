use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::{
    Effects, GrokProtocol, ROOT_VERSION, ROOT_VERSION_OUTPUT, ReportedModels, skills::SkillCatalog,
    tests::options,
};
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::{
    InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeError, RuntimeEventKind,
};

fn skill(root: &Path, name: &str) -> SelectedLocalSkill {
    let path = root.join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("---\nname: {name}\ndescription: 原生注册夹具\nuser-invocable: true\n---\n只读验收正文。\n")).unwrap();
    SelectedLocalSkill {
        name: name.into(),
        path,
    }
}

fn entry(skill: &SelectedLocalSkill) -> Value {
    json!({"name":skill.name,"description":"原生目录夹具","input":null,
        "_meta":{"path":skill.path,"bareName":skill.name,"qualifiedName":format!("local:{}",skill.name),"scope":"local"}})
}

fn root_protocol(selected: Vec<SelectedLocalSkill>, commands: Value) -> GrokProtocol {
    let mut launch = options();
    launch.selected_skills = selected;
    let mut protocol = GrokProtocol::new(launch);
    protocol.bind_cli_version(ROOT_VERSION_OUTPUT).unwrap();
    protocol.paired_version = Some(ROOT_VERSION);
    protocol.session_id = Some("g06-current-session".into());
    protocol.reported_metadata.models = Some(ReportedModels {
        current_model_id: "grok-4.7".into(),
        available_models: Vec::new(),
    });
    // 单测明确提供派生结果；产品只能在私有 socket 的真实进程成功派生后设置此位。
    protocol.owned_skill_leader = true;
    protocol.skill_catalog = Some(SkillCatalog::from_native(&commands).unwrap());
    protocol
}

fn input(skill: &SelectedLocalSkill) -> InputContent {
    InputContent::Skill {
        name: skill.name.clone(),
        path: skill.path.clone(),
    }
}

fn submit(protocol: &mut GrokProtocol, message_id: Uuid, input: Vec<InputContent>) -> Effects {
    protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id,
        action: RuntimeAction::Submit { input },
    })
}

fn reloaded(protocol: &mut GrokProtocol, request: &Value) -> Effects {
    protocol
        .receive(json!({"jsonrpc":"2.0","id":request["id"],"result":{"result":{"reloaded":1}}}))
        .unwrap()
}

fn pulled(protocol: &mut GrokProtocol, request: &Value, commands: Value) -> Effects {
    protocol.receive(json!({"jsonrpc":"2.0","id":request["id"],"result":{"commands":commands,"tools":["read_file"]}})).unwrap()
}

#[test]
fn two_initial_skills_keep_native_blocks_and_user_text_without_injecting_bodies() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let mut protocol = root_protocol(
        vec![alpha.clone(), beta.clone()],
        json!([entry(&alpha), entry(&beta)]),
    );
    let effects = submit(
        &mut protocol,
        Uuid::new_v4(),
        vec![
            InputContent::Text("中文完整输入".into()),
            input(&alpha),
            input(&beta),
        ],
    );
    assert_eq!(effects.writes[0]["method"], "session/prompt");
    assert_eq!(
        effects.writes[0]["params"]["prompt"],
        json!([
            {"type":"text","text":"/alpha"},{"type":"text","text":"/beta"},{"type":"text","text":"中文完整输入"}
        ])
    );
    assert!(effects.events.is_empty());
    assert!(protocol.refreshing_skill_prompt.is_none());
}

#[test]
fn every_initial_selected_skill_requires_its_exact_native_source() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let other = TempDir::new().unwrap();
    let replacement = skill(other.path(), "beta");
    let mut protocol = root_protocol(
        vec![alpha.clone(), beta.clone()],
        json!([entry(&alpha), entry(&replacement)]),
    );
    assert!(
        protocol
            .verify_initial_skill_catalog(&json!([entry(&alpha), entry(&replacement)]))
            .is_err()
    );
    let effects = submit(
        &mut protocol,
        Uuid::new_v4(),
        vec![input(&alpha), input(&beta)],
    );
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn hot_skill_waits_for_reload_and_current_catalog_before_dispatch() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let message_id = Uuid::new_v4();
    let reload = submit(&mut protocol, message_id, vec![input(&gamma)]);
    assert_eq!(reload.writes[0]["method"], "_x.ai/internal/reload_skills");
    assert_eq!(
        reload.writes[0]["params"],
        json!({"sessionId":"g06-current-session"})
    );
    assert!(reload.events.is_empty());
    assert!(protocol.prompt.is_none());
    assert!(protocol.options.selected_skills.is_empty());
    let catalog = reloaded(&mut protocol, &reload.writes[0]);
    assert_eq!(catalog.writes[0]["method"], "_x.ai/commands/list");
    assert!(protocol.prompt.is_none());
    let result = pulled(&mut protocol, &catalog.writes[0], json!([entry(&gamma)]));
    assert_eq!(
        result.writes[0]["params"]["prompt"],
        json!([{"type":"text","text":"/gamma"}])
    );
    assert_eq!(protocol.options.selected_skills, vec![gamma]);
    assert_eq!(protocol.prompt.as_ref().unwrap().message_id, message_id);
    assert!(result.events.is_empty());
}

#[test]
fn reload_count_cannot_replace_exact_catalog_identity() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let message_id = Uuid::new_v4();
    let reload = submit(&mut protocol, message_id, vec![input(&gamma)]);
    let catalog = reloaded(&mut protocol, &reload.writes[0]);
    let result = pulled(&mut protocol, &catalog.writes[0], json!([]));
    assert!(result.writes.is_empty());
    assert!(
        matches!(result.events.as_slice(),[RuntimeEventKind::RequestFailed { message_id: failed, .. }] if *failed==message_id)
    );
    assert!(protocol.prompt.is_none());
    assert!(protocol.options.selected_skills.is_empty());
}

#[test]
fn changed_skill_bytes_during_reload_cannot_reach_the_model() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let reload = submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    let catalog = reloaded(&mut protocol, &reload.writes[0]);
    fs::write(
        &gamma.path,
        "---\nname: gamma\ndescription: 修改后内容\nuser-invocable: true\n---\n不同正文\n",
    )
    .unwrap();
    let result = pulled(&mut protocol, &catalog.writes[0], json!([entry(&gamma)]));
    assert!(result.writes.is_empty());
    assert!(matches!(
        result.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.prompt.is_none());
}

#[test]
fn duplicate_reload_and_submit_do_not_repeat_the_model_input() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let message_id = Uuid::new_v4();
    let reload = submit(&mut protocol, message_id, vec![input(&gamma)]);
    assert!(
        submit(&mut protocol, message_id, vec![input(&gamma)])
            .writes
            .is_empty()
    );
    let catalog = reloaded(&mut protocol, &reload.writes[0]);
    assert!(reloaded(&mut protocol, &reload.writes[0]).writes.is_empty());
    let sent = pulled(&mut protocol, &catalog.writes[0], json!([entry(&gamma)]));
    assert_eq!(sent.writes.len(), 1);
    assert!(
        pulled(&mut protocol, &catalog.writes[0], json!([entry(&gamma)]))
            .writes
            .is_empty()
    );
    assert!(
        submit(&mut protocol, message_id, vec![input(&gamma)])
            .writes
            .is_empty()
    );
}

#[test]
fn hot_skill_does_not_refresh_a_running_session() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let first = submit(
        &mut protocol,
        Uuid::new_v4(),
        vec![InputContent::Text("正在运行".into())],
    );
    let result = submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    assert_eq!(first.writes[0]["method"], "session/prompt");
    assert!(result.writes.is_empty());
    assert!(matches!(
        result.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.queued.is_empty());
    assert!(protocol.refreshing_skill_prompt.is_none());
}

#[test]
fn default_policy_alone_cannot_claim_an_owned_leader_for_hot_skills() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    protocol.owned_skill_leader = false;
    let result = submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    assert!(result.writes.is_empty());
    assert!(matches!(
        result.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn changed_actual_model_cannot_expand_multi_skill_coverage() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let mut protocol = root_protocol(
        vec![alpha.clone(), beta.clone()],
        json!([entry(&alpha), entry(&beta)]),
    );
    protocol
        .reported_metadata
        .models
        .as_mut()
        .unwrap()
        .current_model_id = "unknown".into();
    let result = submit(
        &mut protocol,
        Uuid::new_v4(),
        vec![input(&alpha), input(&beta)],
    );
    assert!(result.writes.is_empty());
    assert!(matches!(
        result.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn changed_session_during_reload_is_not_accepted_as_current() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let reload = submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    protocol.session_id = Some("different-session".into());
    assert!(protocol.receive(json!({"jsonrpc":"2.0","id":reload.writes[0]["id"],"result":{"result":{"reloaded":1}}})).is_err());
    assert!(protocol.prompt.is_none());
}

#[test]
fn unexpected_reload_scope_never_dispatches_a_skill() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let reload = submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    assert!(protocol.receive(json!({"jsonrpc":"2.0","id":reload.writes[0]["id"],"result":{"result":{"reloaded":2}}})).is_err());
    assert!(protocol.prompt.is_none());
}

#[test]
fn refresh_failure_reports_its_message_without_creating_a_turn() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let message_id = Uuid::new_v4();
    let reload = submit(&mut protocol, message_id, vec![input(&gamma)]);
    let failed = protocol.receive(json!({"jsonrpc":"2.0","id":reload.writes[0]["id"],"error":{"code":-32601,"message":"unavailable"}})).unwrap();
    assert!(failed.writes.is_empty());
    assert!(
        matches!(failed.events.as_slice(),[RuntimeEventKind::RequestFailed { message_id: failed, .. }] if *failed==message_id)
    );
    assert!(protocol.prompt.is_none());
    assert!(protocol.refreshing_skill_prompt.is_none());
    assert!(matches!(protocol.pending, None));
}

#[test]
fn transport_loss_fails_the_input_waiting_for_catalog() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let message_id = Uuid::new_v4();
    let reload = submit(&mut protocol, message_id, vec![input(&gamma)]);
    let _catalog = reloaded(&mut protocol, &reload.writes[0]);
    let failed = protocol.finish_transport(&Err(RuntimeError::ControllerClosed));
    assert!(
        matches!(failed.events.as_slice(),[RuntimeEventKind::RequestFailed { message_id: failed, .. }] if *failed==message_id)
    );
    assert!(protocol.refreshing_skill_prompt.is_none());
}

#[test]
fn old_generation_cannot_start_a_skill_refresh() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let result = protocol.command(RuntimeCommand {
        generation: Uuid::new_v4(),
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![input(&gamma)],
        },
    });
    assert!(result.writes.is_empty());
    assert!(protocol.pending.is_none());
}

#[test]
fn fixed_file_policy_does_not_inherit_skill_permissions() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(vec![gamma.clone()], json!([entry(&gamma)]));
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    assert!(
        submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)])
            .writes
            .is_empty()
    );
    assert!(!protocol.skill_updates_verified());
}

#[test]
fn catalog_must_preserve_local_qualified_identity() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let reload = submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    let catalog = reloaded(&mut protocol, &reload.writes[0]);
    let mut changed = entry(&gamma);
    changed["_meta"]["qualifiedName"] = json!("user:gamma");
    let result = pulled(&mut protocol, &catalog.writes[0], json!([changed]));
    assert!(result.writes.is_empty());
    assert!(matches!(
        result.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.options.selected_skills.is_empty());
}

#[test]
fn sdk_option_cannot_inherit_default_leader_skill_permissions() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(vec![gamma.clone()], json!([entry(&gamma)]));
    protocol.options.local_tools = Some(LocalToolPermissions::default());
    assert!(
        submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)])
            .writes
            .is_empty()
    );
    assert!(!protocol.skill_updates_verified());
}

#[test]
fn shutdown_fails_a_refresh_waiting_input() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    let message_id = Uuid::new_v4();
    submit(&mut protocol, message_id, vec![input(&gamma)]);
    let result = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Shutdown,
    });
    assert!(result.writes.is_empty());
    assert!(result.events.iter().any(|event| matches!(event,RuntimeEventKind::RequestFailed { message_id: failed, .. } if *failed == message_id)));
    assert!(protocol.closed);
    assert!(protocol.refreshing_skill_prompt.is_none());
}

#[test]
fn unsolicited_catalog_response_cannot_release_waiting_input() {
    let root = TempDir::new().unwrap();
    let gamma = skill(root.path(), "gamma");
    let mut protocol = root_protocol(Vec::new(), json!([]));
    submit(&mut protocol, Uuid::new_v4(), vec![input(&gamma)]);
    assert!(protocol.receive(json!({"jsonrpc":"2.0","id":99999,"result":{"commands":[entry(&gamma)],"tools":["read_file"]}})).is_err());
    assert!(protocol.prompt.is_none());
    assert!(protocol.refreshing_skill_prompt.is_some());
}

#[test]
fn skill_name_collision_cannot_replace_an_existing_source() {
    let root = TempDir::new().unwrap();
    let original = skill(root.path(), "gamma");
    let other = TempDir::new().unwrap();
    let replacement = skill(other.path(), "gamma");
    let mut protocol = root_protocol(vec![original.clone()], json!([entry(&original)]));
    let effects = submit(&mut protocol, Uuid::new_v4(), vec![input(&replacement)]);
    assert!(effects.writes.is_empty());
    assert!(protocol.refreshing_skill_prompt.is_none());
    assert_eq!(protocol.options.selected_skills, vec![original]);
}

#[test]
fn skill_encoding_rejects_escaped_json_over_the_frame_budget() {
    let root = TempDir::new().unwrap();
    let alpha = skill(root.path(), "alpha");
    let beta = skill(root.path(), "beta");
    let mut protocol = root_protocol(
        vec![alpha.clone(), beta.clone()],
        json!([entry(&alpha), entry(&beta)]),
    );
    let effects = submit(
        &mut protocol,
        Uuid::new_v4(),
        vec![
            input(&alpha),
            input(&beta),
            InputContent::Text("\u{0001}".repeat(super::MAX_LINE_BYTES / 4)),
        ],
    );
    assert!(effects.writes.is_empty());
    assert!(protocol.prompt.is_none());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}
