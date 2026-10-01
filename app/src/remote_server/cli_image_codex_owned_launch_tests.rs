use super::*;

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn legacy_manifest(directory: &Path) -> Manifest {
    let token = Token::capture(std::process::id() as i32).unwrap();
    let reservation = Reservation {
        version: 1,
        id: Uuid::new_v4(),
        key_sha256: digest(Uuid::new_v4().as_bytes()),
        host: "fixture-host".into(),
        terminal_session: 7,
        cwd: directory.into(),
        app: directory.join("absent-app"),
        app_sha256: "fixture-app".into(),
        notifications: None,
    };
    write_new(&directory.join("ticket.json"), &reservation).unwrap();
    Manifest {
        version: 1,
        ticket: reservation.id,
        reservation_sha256: digest(&read_private(&directory.join("ticket.json")).unwrap()),
        executable: directory.join("absent-codex"),
        cwd: reservation.cwd,
        codex_home: directory.join("unchanged-home"),
        app: reservation.app,
        wrapper: token.clone(),
        shell: token,
        group: 7,
        tty_device: 11,
        socket: directory.join("control.sock"),
        notifications: None,
    }
}

#[test]
fn normalized_notification_hash_matches_existing_native_discovery_receipts() {
    // 已归档原生 hooks/list 的旧命令金样；不是拿本实现生成本实现的期望值。
    let trusted: serde_json::Value = serde_json::from_str(include_str!(
        "../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST.json"
    ))
    .unwrap();
    for event in [
        NotificationEvent::SessionStart,
        NotificationEvent::UserPromptSubmit,
        NotificationEvent::PermissionRequest,
        NotificationEvent::PostToolUse,
        NotificationEvent::Stop,
    ] {
        let (_, label, script) = event.fields();
        let key = format!("warp@codex-warp:hooks/hooks.json:{label}:0:0");
        let receipt = trusted["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == key)
            .unwrap();
        let command = format!("bash \"$PLUGIN_ROOT/scripts/{script}\"");
        assert_eq!(receipt["command"], command);
        assert_eq!(
            notification_hash(event, &command).unwrap(),
            receipt["currentHash"].as_str().unwrap()
        );
    }
}

#[test]
fn typed_session_flags_contain_only_five_notifications_and_exact_trust() {
    let directory = directory();
    let root = directory
        .path()
        .join("通知 ' $(touch INJECTED) `touch INJECTED`");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    let arguments = plan.arguments();
    assert_eq!(arguments.len(), 20);
    let mut merged = toml::Table::new();
    for pair in arguments.chunks_exact(2) {
        assert_eq!(pair[0], "-c");
        let parsed: toml::Table = pair[1].parse().unwrap();
        assert_eq!(parsed.len(), 1);
        let hooks = parsed["hooks"].as_table().unwrap();
        for (key, value) in hooks {
            if key == "state" {
                let states = value.as_table().unwrap();
                assert_eq!(states.len(), 1);
                let (key, state) = states.iter().next().unwrap();
                let hook = plan.hooks.iter().find(|hook| &hook.key == key).unwrap();
                assert_eq!(state["enabled"].as_bool(), Some(true));
                assert_eq!(
                    state["trusted_hash"].as_str(),
                    Some(hook.normalized_hash.as_str())
                );
                assert_eq!(state.as_table().unwrap().len(), 2);
            } else {
                assert!(merged.insert(key.clone(), value.clone()).is_none());
                let handler = &value.as_array().unwrap()[0]["hooks"][0];
                assert_eq!(handler["type"].as_str(), Some("command"));
                assert_eq!(handler["timeout"].as_integer(), Some(600));
                assert_eq!(handler["async"].as_bool(), Some(false));
                assert_eq!(handler.as_table().unwrap().len(), 4);
                let command = shlex::split(handler["command"].as_str().unwrap()).unwrap();
                assert_eq!(command.len(), 2);
                assert_eq!(command[0], "bash");
                assert_eq!(
                    Path::new(&command[1]).parent(),
                    Some(plan.root.join("scripts").as_path())
                );
            }
        }
    }
    assert_eq!(merged.len(), 5);
    assert_eq!(plan.files.len(), 8);
    assert!(!root.join(".codex").exists());
}

#[test]
fn changed_notification_bytes_are_rejected() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    fs::write(plan.root.join("scripts/on-session-start.sh"), b"exit 0\n").unwrap();
    assert!(plan.verify(&root).is_err());
}

#[test]
fn same_bytes_replaced_notification_inode_is_rejected() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    let path = plan.root.join("scripts/on-session-start.sh");
    let original = File::open(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert_ne!(
        original.metadata().unwrap().ino(),
        fs::metadata(&path).unwrap().ino()
    );
    assert!(plan.verify(&root).is_err());
}

#[test]
fn undeclared_notification_resource_is_rejected() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    fs::write(plan.root.join("scripts/additional.sh"), b"exit 0\n").unwrap();
    assert!(plan.verify(&root).is_err());
}

#[test]
fn serialized_notification_plan_cannot_add_an_arbitrary_command() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    let mut value = serde_json::to_value(&plan).unwrap();
    value["hooks"][0]["command"] = serde_json::Value::String("touch unreviewed".into());
    let changed: NotificationPlan = serde_json::from_value(value).unwrap();
    assert!(changed.verify(&root).is_err());
    let mut value = serde_json::to_value(plan).unwrap();
    value["extra_argv"] = serde_json::json!(["--dangerously-bypass-approvals-and-sandbox"]);
    assert!(serde_json::from_value::<NotificationPlan>(value).is_err());
}

#[test]
fn legacy_manifest_remains_queryable_but_reserved_ticket_cannot_start_again() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let manifest = legacy_manifest(&root);
    let encoded = serde_json::to_vec(&manifest).unwrap();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&encoded)
            .unwrap()
            .get("notifications")
            .is_none()
    );
    let restored = bound_manifest(&root, &encoded).unwrap();
    assert_eq!(
        arguments(&restored, "server"),
        vec![
            "app-server".to_string(),
            "--listen".to_string(),
            format!("unix://{}", root.join("control.sock").display())
        ]
    );
    let ticket = root.join("ticket.json");
    assert!(supervise(&ticket, &digest(&read_private(&ticket).unwrap())).is_err());
    assert!(!root.join("entered.json").exists());
    assert!(!root.join("manifest.json").exists());
    assert!(!root.join("server-birth.json").exists());
}

#[test]
fn native_identity_requires_same_typed_flags_for_server_and_tui() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let mut manifest = legacy_manifest(&root);
    manifest.version = 2;
    manifest.notifications = Some(NotificationPlan::create(&root).unwrap());
    let flags = manifest.notifications.as_ref().unwrap().arguments();
    let helper = helper_command(
        &manifest,
        &root.join("manifest.json"),
        "fixture-hash",
        "server",
    );
    assert!(helper.get_envs().next().is_none());
    for role in ["server", "tui"] {
        let expected = arguments(&manifest, role);
        assert_eq!(&expected[..20], &flags);
        let mut actual = vec![b"codex".to_vec()];
        actual.extend(expected.iter().map(|item| item.as_bytes().to_vec()));
        assert!(arguments_match(&actual, &manifest, role));
        actual.push(b"--dangerously-bypass-approvals-and-sandbox".to_vec());
        assert!(!arguments_match(&actual, &manifest, role));
        actual.pop();
        actual.remove(1);
        assert!(!arguments_match(&actual, &manifest, role));
    }
}

#[test]
fn new_manifest_must_match_the_reserved_notification_plan() {
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let mut manifest = legacy_manifest(&root);
    let mut reservation: Reservation = read_json(&root.join("ticket.json")).unwrap();
    let plan = NotificationPlan::create(&root).unwrap();
    reservation.version = 2;
    reservation.notifications = Some(plan.clone());
    fs::write(
        root.join("ticket.json"),
        serde_json::to_vec(&reservation).unwrap(),
    )
    .unwrap();
    manifest.version = 2;
    manifest.notifications = Some(plan);
    manifest.reservation_sha256 = digest(&read_private(&root.join("ticket.json")).unwrap());
    let encoded = serde_json::to_vec(&manifest).unwrap();
    assert!(bound_manifest(&root, &encoded).is_ok());
    manifest.notifications.as_mut().unwrap().hooks[0].normalized_hash = "sha256:changed".into();
    assert!(bound_manifest(&root, &serde_json::to_vec(&manifest).unwrap()).is_err());
    manifest.notifications = None;
    assert!(bound_manifest(&root, &serde_json::to_vec(&manifest).unwrap()).is_err());
}
