use super::*;

fn base(cwd: &Path) -> ClaudeRestrictedFilesV1 {
    let release: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/claude-2.1.280-release-manifest.json"
    ))
    .unwrap();
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "win32-x64",
        _ => panic!("当前平台没有 V2 固定验收版本"),
    };
    ClaudeRestrictedFilesV1::compile(cwd,
        release["platforms"][platform]["checksum"].as_str().unwrap().into(), None,
        &json!({"effective":{},"sources":[]}),
        &json!({"state":{"originalCwd":cwd,"workspaceDirectories":[],"managedOnly":false,"rules":[]}}),
        &json!({"hooks":[],"policy":{"policyHookCount":0}})).unwrap()
}

#[test]
fn v1_serialization_and_write_denial_survive_versioned_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let base = base(directory.path());
    let old_bytes = serde_json::to_vec(&base).unwrap();
    let saved: ClaudeFileProfile = serde_json::from_slice(&old_bytes).unwrap();
    assert_eq!(serde_json::to_vec(&saved).unwrap(), old_bytes);
    assert_eq!(saved.arguments(), base.arguments());
    assert_eq!(saved.digest(), base.digest());
    assert_eq!(saved.policy(), PermissionPolicy::ClaudeRestrictedFilesV1);
    assert!(!saved.approval_allowed(
        "Write",
        &json!({"file_path":directory.path().join("new.txt"),"content":"new"})
    ));
    let expanded =
        ClaudeFileProfile::compile(PermissionPolicy::ClaudeRestrictedFilesV2, base).unwrap();
    assert!(!saved.same_scope(&expanded));
    assert!(saved.verify_source(&expanded).is_err());
    assert!(expanded.verify_source(&saved).is_err());
    assert!(serde_json::from_value::<ClaudeRestrictedFilesV1>(json!(expanded)).is_err());
}

#[test]
fn v2_binds_version_binary_and_tool_set_without_enabling_commands_or_skills() {
    let directory = tempfile::tempdir().unwrap();
    let profile = ClaudeFileProfile::compile(
        PermissionPolicy::ClaudeRestrictedFilesV2,
        base(directory.path()),
    )
    .unwrap();
    let args = profile.arguments();
    assert!(args.contains(&"--tools=Read,Edit,Write".into()));
    assert!(args.contains(&"--permission-mode=manual".into()));
    assert!(args.contains(&"--setting-sources=".into()));
    let settings: Value = serde_json::from_str(
        args.iter()
            .find_map(|arg| arg.strip_prefix("--settings="))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(settings["permissions"]["ask"], json!(["Edit", "Write"]));
    assert!(profile.verify_system_init(&json!({"permissionMode":"default","tools":["Read","Edit","Write","EndConversation"]})).is_ok());
    assert!(
        profile
            .verify_system_init(
                &json!({"permissionMode":"default","tools":["Read","Write","Bash"]})
            )
            .is_err()
    );
    for (path, value) in [("version", json!(3)), ("cliVersion", json!("2.1.281"))] {
        let mut changed = json!(profile);
        changed[path] = value;
        let changed: ClaudeFileProfile = serde_json::from_value(changed).unwrap();
        assert!(changed.validate(directory.path()).is_err());
    }
    let mut changed = json!(profile);
    changed["base"]["executableSha256"] = json!("a".repeat(64));
    assert!(
        serde_json::from_value::<ClaudeFileProfile>(changed)
            .unwrap()
            .validate(directory.path())
            .is_err()
    );
}

#[test]
fn v2_write_requires_exact_fields_and_safe_existing_parent() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let profile =
        ClaudeFileProfile::compile(PermissionPolicy::ClaudeRestrictedFilesV2, base(&cwd)).unwrap();
    assert!(profile.approval_allowed(
        "Write",
        &json!({"file_path":cwd.join("new.txt"),"content":"new"})
    ));
    std::fs::write(cwd.join("existing.txt"), "old").unwrap();
    assert!(profile.approval_allowed(
        "Write",
        &json!({"file_path":cwd.join("existing.txt"),"content":"new"})
    ));
    for path in [
        cwd.join(".claude/settings.json"),
        cwd.join(".git/config"),
        cwd.join(".mcp.json"),
        cwd.join("missing/new.txt"),
        cwd.join("../outside.txt"),
        cwd.join("stream:secret"),
        cwd.join(".claude. "),
    ] {
        assert!(
            !profile.approval_allowed("Write", &json!({"file_path":path,"content":"new"})),
            "{path:?}"
        );
    }
    assert!(!profile.approval_allowed(
        "Write",
        &json!({"file_path":cwd.join("new.txt"),"content":"new","unknown":true})
    ));
    assert!(!profile.approval_allowed(
        "Write",
        &json!({"file_path":cwd.join("new.txt"),"content":[]})
    ));
}

#[cfg(unix)]
#[test]
fn v2_rechecks_internal_external_and_replaced_symlinks() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let profile =
        ClaudeFileProfile::compile(PermissionPolicy::ClaudeRestrictedFilesV2, base(&cwd)).unwrap();
    std::fs::write(cwd.join("target.txt"), "old").unwrap();
    symlink(cwd.join("target.txt"), cwd.join("internal.txt")).unwrap();
    symlink(
        cwd.parent().unwrap().join("outside.txt"),
        cwd.join("external.txt"),
    )
    .unwrap();
    for name in ["internal.txt", "external.txt"] {
        assert!(!profile.approval_allowed(
            "Write",
            &json!({"file_path":cwd.join(name),"content":"new"})
        ));
    }
    let input = json!({"file_path":cwd.join("pending.txt"),"content":"new"});
    assert!(profile.approval_allowed("Write", &input));
    symlink(cwd.join("target.txt"), cwd.join("pending.txt")).unwrap();
    assert!(!profile.approval_allowed("Write", &input));
}

#[test]
fn v2_write_cannot_modify_a_protected_file_through_a_hard_link() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let profile =
        ClaudeFileProfile::compile(PermissionPolicy::ClaudeRestrictedFilesV2, base(&cwd)).unwrap();
    std::fs::create_dir(cwd.join(".claude")).unwrap();
    std::fs::write(cwd.join(".claude/settings.json"), "protected").unwrap();
    std::fs::hard_link(cwd.join(".claude/settings.json"), cwd.join("alias.json")).unwrap();
    assert!(!profile.approval_allowed(
        "Write",
        &json!({"file_path":cwd.join("alias.json"),"content":"changed"})
    ));
    assert_eq!(
        std::fs::read_to_string(cwd.join(".claude/settings.json")).unwrap(),
        "protected"
    );
}
