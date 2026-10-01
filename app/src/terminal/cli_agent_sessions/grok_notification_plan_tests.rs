use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn fixture() -> (tempfile::TempDir, NotificationPlan) {
    let directory = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = directory.path().canonicalize().unwrap();
    let worker = root.join("宿主 worker");
    fs::write(&worker, b"private test worker").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
    let plan = NotificationPlan::create(&root, Uuid::from_u128(1), &root, &worker).unwrap();
    (directory, plan)
}

#[test]
fn frozen_plugin_binds_exact_session_and_worker_without_terminal_route() {
    let (_directory, plan) = fixture();
    plan.verify_current().unwrap();
    let descriptor: serde_json::Value =
        serde_json::from_slice(&fs::read(plan.descriptor_path()).unwrap()).unwrap();
    assert_eq!(descriptor["version"], 1);
    assert_eq!(
        descriptor["session_id"],
        "00000000-0000-0000-0000-000000000001"
    );
    assert_eq!(descriptor.as_object().unwrap().len(), 5);
    assert_eq!(descriptor["files"].as_object().unwrap().len(), 3);
    let hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(plan.root.join("plugin/hooks/hooks.json")).unwrap())
            .unwrap();
    let events = hooks["hooks"].as_object().unwrap();
    assert_eq!(events.len(), 10);
    for rules in events.values() {
        let handler = &rules[0]["hooks"][0];
        assert_eq!(handler["timeout"], 5);
        assert_eq!(
            handler["command"],
            "node -e \"require(process.env.GROK_PLUGIN_ROOT + '/hooks/notify.cjs').main()\""
        );
        assert_eq!(
            handler["env"],
            json!({
                "WARP_CLI_AGENT_PROTOCOL_VERSION": "1",
                "WARP_CLI_AGENT_NOTIFY_EXECUTABLE": plan.worker,
                "WARP_CLI_AGENT_SESSION_ROUTE": "1",
            })
        );
    }
    assert_eq!(
        fs::read(plan.root.join("plugin/hooks/notify.cjs")).unwrap(),
        NOTIFY
    );
    assert_eq!(
        fs::read(plan.root.join("plugin/.grok-plugin/plugin.json")).unwrap(),
        MANIFEST
    );
}

#[test]
fn frozen_plugin_rejects_another_session_working_directory_or_worker() {
    let (_directory, plan) = fixture();
    assert!(plan.verify_session(Uuid::from_u128(2), &plan.cwd).is_err());
    assert!(
        plan.verify(plan.session_id, &plan.root, &plan.worker)
            .is_err()
    );
    assert!(
        plan.verify(plan.session_id, &plan.cwd, &plan.root.join("other-worker"))
            .is_err()
    );
    assert!(plan.verify_parent(&plan.root).is_err());
}

#[test]
fn frozen_plugin_rejects_same_bytes_replaced_with_a_new_file() {
    let (_directory, plan) = fixture();
    let path = plan.root.join("plugin/hooks/notify.cjs");
    let replacement = plan.cwd.join("replacement.cjs");
    write_new(&replacement, NOTIFY).unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert!(plan.verify_current().is_err());
}

#[test]
fn frozen_plugin_rejects_resource_edits_even_if_a_recorded_digest_is_replaced() {
    let (_directory, mut plan) = fixture();
    let path = plan.root.join("plugin/hooks/hooks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["hooks"]["SessionStart"][0]["hooks"][0]["env"]["SSH_TTY"] = "/dev/ttys099".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    plan.files.insert(
        "plugin/hooks/hooks.json".into(),
        frozen_file(&path).unwrap(),
    );
    assert!(plan.verify_current().is_err());
}

#[test]
fn frozen_plugin_rejects_unlisted_files() {
    let (_directory, plan) = fixture();
    write_new(&plan.root.join("plugin/hooks/extra.cjs"), b"extra").unwrap();
    assert!(plan.verify_current().is_err());
}

#[test]
fn frozen_plugin_rejects_symlinked_resource() {
    let (_directory, plan) = fixture();
    let path = plan.root.join("plugin/hooks/notify.cjs");
    let target = plan.cwd.join("notify.cjs");
    fs::rename(&path, &target).unwrap();
    symlink(&target, &path).unwrap();
    assert!(plan.verify_current().is_err());
}

#[test]
fn frozen_plugin_rejects_worker_replacement() {
    let (_directory, plan) = fixture();
    let replacement = plan.cwd.join("new-worker");
    fs::write(&replacement, b"private test worker").unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(&replacement, &plan.worker).unwrap();
    assert!(plan.verify_current().is_err());
}

#[test]
fn frozen_plugin_cannot_be_created_twice_for_one_ticket() {
    let (_directory, plan) = fixture();
    assert!(NotificationPlan::create(&plan.cwd, plan.session_id, &plan.cwd, &plan.worker).is_err());
    plan.verify_current().unwrap();
}
