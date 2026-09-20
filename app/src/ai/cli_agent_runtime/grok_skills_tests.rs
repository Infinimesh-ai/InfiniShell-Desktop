use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::super::{Effects, GrokProtocol, tests::options, validate_options};
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::{
    InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeEventKind,
    SessionOptions, SessionTarget,
};

fn skill() -> (TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("中文 技能").join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "---\nname: parity-review\ndescription: 审查\nuser-invocable: true\n---\n实际技能内容。\n",
    )
    .unwrap();
    (root, path)
}

fn response(id: u64) -> Value {
    include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-authenticated-quota-error.ndjson"
    )
    .lines()
    .map(|line| serde_json::from_str::<Value>(line).unwrap())
    .find(|row| row["direction"] == "stdout" && row["message"]["id"] == id)
    .unwrap()["message"]
        .clone()
}

fn opening(launch: SessionOptions) -> GrokProtocol {
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.initialize().unwrap();
    protocol.receive(response(1)).unwrap();
    protocol.receive(response(2)).unwrap();
    protocol
}

fn ready() -> GrokProtocol {
    let mut protocol = opening(options());
    protocol.receive(response(3)).unwrap();
    protocol
}

fn catalog(session: &Value, path: &std::path::Path) -> Value {
    json!({"jsonrpc":"2.0", "method":"session/update", "params":{
    "sessionId":session, "update":{"sessionUpdate":"available_commands_update", "availableCommands":[{
        "name":"parity-review", "description":"审查", "_meta":{"path":path,"bareName":"parity-review","scope":"local"}
    }]}}})
}

fn input(path: &std::path::Path) -> Vec<InputContent> {
    vec![InputContent::Skill {
        name: "parity-review".into(),
        path: path.to_owned(),
    }]
}

fn submit(protocol: &mut GrokProtocol, input: Vec<InputContent>) -> (Uuid, Effects) {
    let id = Uuid::new_v4();
    (
        id,
        protocol.command(RuntimeCommand {
            generation: protocol.options.generation,
            message_id: id,
            action: RuntimeAction::Submit { input },
        }),
    )
}

#[test]
fn initial_selected_skill_binds_early_catalog_and_preserves_text_blocks_without_exposing_paths() {
    let (_root, path) = skill();
    let mut launch = options();
    launch.selected_skills = vec![SelectedLocalSkill {
        name: "parity-review".into(),
        path: path.clone(),
    }];
    assert!(validate_options(&launch).is_ok());
    let mut protocol = opening(launch);
    protocol
        .receive(catalog(&response(3)["result"]["sessionId"], &path))
        .unwrap();
    let ready = protocol.receive(response(3)).unwrap();
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { .. }]
    ));
    assert!(ready.writes.is_empty());
    let mut content = vec![InputContent::Text(
        "中文\nEnglish\n真实文件: /tmp/含 空格.rs\nReview: 保留完整意见".into(),
    )];
    content.extend(input(&path));
    content.push(InputContent::Text("第二文本块\n$() `literal`".into()));
    let (_, effects) = submit(&mut protocol, content);
    assert_eq!(
        effects.writes[0]["params"]["prompt"],
        json!([
            {"type":"text","text":"/parity-review 中文\nEnglish\n真实文件: /tmp/含 空格.rs\nReview: 保留完整意见"},
            {"type":"text","text":"第二文本块\n$() `literal`"}
        ])
    );
    assert!(
        !serde_json::to_string(&protocol.reported_metadata)
            .unwrap()
            .contains("_meta")
    );
}

#[test]
fn selected_new_task_waits_for_current_catalog_but_resume_does_not_replay_or_require_old_skill_file()
 {
    let (_root, path) = skill();
    let mut launch = options();
    launch.selected_skills = vec![SelectedLocalSkill {
        name: "parity-review".into(),
        path: path.clone(),
    }];
    let mut protocol = opening(launch.clone());
    assert!(protocol.receive(response(3)).unwrap().events.is_empty());
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
    assert!(
        protocol
            .receive(catalog(&json!("unrelated-old-session"), &path))
            .unwrap()
            .events
            .is_empty()
    );
    let session = json!(protocol.session_id);
    let ready = protocol.receive(catalog(&session, &path)).unwrap();
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { .. }]
    ));
    assert!(ready.writes.is_empty());
    fs::remove_file(&path).unwrap();
    launch.target = SessionTarget::Resume {
        native_session_id: "restored-session".into(),
    };
    let mut resumed = opening(launch);
    let ready = resumed
        .receive(json!({"jsonrpc":"2.0","id":3,"result":{}}))
        .unwrap();
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { .. }]
    ));
    assert!(ready.writes.is_empty());
    assert!(resumed.submitted_messages.is_empty());
    assert!(submit(&mut resumed, input(&path)).1.writes.is_empty());
}

#[test]
fn selected_skill_requires_unique_native_path_and_command_identity() {
    let (_root, path) = skill();
    let (_other_root, other_path) = skill();
    let mut protocol = ready();
    let session = json!(protocol.session_id);
    protocol.receive(catalog(&session, &other_path)).unwrap();
    assert!(matches!(
        submit(&mut protocol, input(&path)).1.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    let mut duplicated = catalog(&session, &path);
    let command = duplicated["params"]["update"]["availableCommands"][0].clone();
    duplicated["params"]["update"]["availableCommands"]
        .as_array_mut()
        .unwrap()
        .push(command);
    protocol.receive(duplicated).unwrap();
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
    let mut missing = catalog(&session, &path);
    missing["params"]["update"]["availableCommands"][0]
        .as_object_mut()
        .unwrap()
        .remove("_meta");
    protocol.receive(missing).unwrap();
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
}

#[test]
fn replayed_or_other_session_catalog_cannot_replace_the_current_skill_binding() {
    let (_root, path) = skill();
    let (_other_root, other_path) = skill();
    let mut protocol = ready();
    let session = json!(protocol.session_id);
    let mut old = catalog(&session, &other_path);
    old["params"]["_meta"] = json!({"eventId":"catalog-old"});
    protocol.receive(old.clone()).unwrap();
    protocol.receive(catalog(&session, &path)).unwrap();
    protocol.receive(old.clone()).unwrap();
    old["params"]["_meta"] = json!({"isReplay":true});
    protocol.receive(old).unwrap();
    protocol
        .receive(catalog(&json!("old-session"), &other_path))
        .unwrap();
    let (_, effects) = submit(&mut protocol, input(&path));
    assert_eq!(
        effects.writes[0]["params"]["prompt"],
        json!([{"type":"text","text":"/parity-review"}])
    );
}

#[test]
fn queued_skill_file_change_fails_its_message_and_dispatches_the_next_input() {
    let (_root, path) = skill();
    let mut protocol = ready();
    let session = json!(protocol.session_id);
    protocol.receive(catalog(&session, &path)).unwrap();
    let (_, first) = submit(&mut protocol, vec![InputContent::Text("第一轮".into())]);
    let (skill_id, queued) = submit(&mut protocol, input(&path));
    assert!(queued.writes.is_empty());
    let (next_id, next) = submit(
        &mut protocol,
        vec![InputContent::Text("继续后续输入".into())],
    );
    assert!(next.writes.is_empty());
    fs::write(
        &path,
        "---\nname: parity-review\ndescription: 审查\n---\n排队期间已变更。\n",
    )
    .unwrap();
    let finished = protocol.receive(json!({"jsonrpc":"2.0","id":first.writes[0]["id"],"error":{"code":-32000,"message":"fixture failure"}})).unwrap();
    assert!(finished.events.iter().any(|event| matches!(event, RuntimeEventKind::RequestFailed { message_id, .. } if *message_id == skill_id)));
    assert_eq!(
        finished.writes[0]["params"]["prompt"],
        json!([{"type":"text","text":"继续后续输入"}])
    );
    assert_eq!(protocol.prompt.as_ref().unwrap().message_id, next_id);
    let duplicate = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: skill_id,
        action: RuntimeAction::Submit {
            input: input(&path),
        },
    });
    assert!(duplicate.writes.is_empty());
    assert!(duplicate.events.is_empty());
}

#[test]
fn hidden_renamed_multiple_skills_images_and_fixed_policies_are_rejected_before_prompt() {
    let (_root, path) = skill();
    let mut protocol = ready();
    let session = json!(protocol.session_id);
    protocol.receive(catalog(&session, &path)).unwrap();
    let mut multiple = input(&path);
    multiple.extend(input(&path));
    assert!(submit(&mut protocol, multiple).1.writes.is_empty());
    let mut image = input(&path);
    image.push(InputContent::LocalImage(path.clone()));
    assert!(submit(&mut protocol, image).1.writes.is_empty());
    fs::write(
        &path,
        "---\nname: parity-review\ndescription: 审查\nuser-invocable: false\n---\n隐藏\n",
    )
    .unwrap();
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
    fs::write(
        &path,
        "---\nname: renamed\ndescription: 审查\n---\n重命名\n",
    )
    .unwrap();
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
    let mut fixed = options();
    fixed.selected_skills = vec![SelectedLocalSkill {
        name: "parity-review".into(),
        path: path.clone(),
    }];
    fixed.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    assert!(validate_options(&fixed).is_err());
    fixed.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    assert!(validate_options(&fixed).is_err());
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    assert!(submit(&mut protocol, input(&path)).1.writes.is_empty());
}

#[test]
fn old_runtime_generation_cannot_dispatch_a_selected_skill() {
    let (_root, path) = skill();
    let mut protocol = ready();
    let session = json!(protocol.session_id);
    protocol.receive(catalog(&session, &path)).unwrap();
    let effects = protocol.command(RuntimeCommand {
        generation: Uuid::new_v4(),
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: input(&path),
        },
    });
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.prompt.is_none());
}

#[test]
fn queued_skill_is_rejected_when_the_current_native_catalog_removes_its_path() {
    let (_root, path) = skill();
    let mut protocol = ready();
    let session = json!(protocol.session_id);
    protocol.receive(catalog(&session, &path)).unwrap();
    let (_, first) = submit(&mut protocol, vec![InputContent::Text("第一轮".into())]);
    let (skill_id, queued) = submit(&mut protocol, input(&path));
    assert!(queued.writes.is_empty());
    let mut removed = catalog(&session, &path);
    removed["params"]["update"]["availableCommands"] = json!([]);
    protocol.receive(removed).unwrap();
    let finished = protocol.receive(json!({"jsonrpc":"2.0","id":first.writes[0]["id"],"error":{"code":-32000,"message":"fixture failure"}})).unwrap();
    assert!(finished.writes.is_empty());
    assert!(finished.events.iter().any(|event| matches!(event, RuntimeEventKind::RequestFailed { message_id, .. } if *message_id == skill_id)));
    assert!(protocol.prompt.is_none());
}
