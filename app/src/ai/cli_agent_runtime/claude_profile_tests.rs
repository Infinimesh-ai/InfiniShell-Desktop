use super::*;

fn external_settings(source: &str, deny: Vec<String>, ask: Vec<String>) -> Value {
    let permissions = json!({"deny":deny,"ask":ask});
    json!({"effective":{"permissions":permissions},"sources":[{"source":source,"settings":{"permissions":permissions}}]})
}

fn isolated_settings() -> Value {
    json!({"effective":{},"sources":[]})
}

fn rules(cwd: &Path, rows: Value) -> Value {
    json!({"state":{"originalCwd":cwd,"workspaceDirectories":[],"managedOnly":false,"rules":rows}})
}

fn hooks() -> Value {
    json!({"hooks":[],"policy":{"policyHookCount":0}})
}

fn profile(cwd: &Path) -> ClaudeRestrictedFilesV1 {
    ClaudeRestrictedFilesV1::compile(
        cwd,
        "a".repeat(64),
        None,
        &isolated_settings(),
        &rules(cwd, json!([])),
        &hooks(),
    )
    .unwrap()
}

#[test]
fn isolated_source_contract_rejects_user_project_and_local_settings() {
    let directory = std::env::temp_dir();
    let cwd = directory.as_path();
    for source in ["userSettings", "projectSettings", "localSettings"] {
        assert!(
            ClaudeRestrictedFilesV1::compile(
                cwd,
                "a".repeat(64),
                None,
                &external_settings(source, vec!["Read(//private/secret)".into()], vec![]),
                &rules(
                    cwd,
                    json!([{"source":source,"behavior":"deny","rule":"Read(//private/secret)"}])
                ),
                &hooks()
            )
            .is_err()
        );
    }
    let profile = profile(cwd);
    let arguments = profile.arguments();
    assert!(
        ISOLATED_SETTINGS_ARGUMENTS
            .iter()
            .all(|argument| arguments.iter().any(|value| value == argument))
    );
    assert!(!arguments.iter().any(|argument| argument == "--bare"));
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "--permission-mode=manual")
    );
    assert!(!arguments.iter().any(|argument| {
        argument == "--permission-mode=plan"
            || argument == "--permission-mode=default"
            || argument.starts_with("--allowedTools")
            || argument.starts_with("--allowed-tools")
    }));
}

#[test]
fn read_ask_is_not_silently_promoted_to_automatic_read() {
    let directory = std::env::temp_dir();
    let cwd = directory.as_path();
    let result = ClaudeRestrictedFilesV1::compile(
        cwd,
        "a".repeat(64),
        None,
        &external_settings("userSettings", vec![], vec!["Read".into()]),
        &rules(
            cwd,
            json!([{"source":"userSettings","behavior":"ask","rule":"Read"}]),
        ),
        &hooks(),
    );
    assert!(result.is_err());
}

#[test]
fn incomplete_admin_and_hook_boundaries_reject_only_the_affected_configuration() {
    let directory = std::env::temp_dir();
    let cwd = directory.as_path();
    assert!(profile(cwd).validate(cwd).is_ok());
    let managed = json!({"effective":{},"sources":[{"source":"policySettings","settings":{"permissions":{"deny":["Read"]}}}]});
    assert!(
        ClaudeRestrictedFilesV1::compile(
            cwd,
            "a".repeat(64),
            None,
            &managed,
            &rules(cwd, json!([])),
            &hooks()
        )
        .is_err()
    );
    let unreadable_settings =
        json!({"effective":{},"sources":[],"errors":[{"source":"policySettings"}]});
    assert!(
        ClaudeRestrictedFilesV1::compile(
            cwd,
            "a".repeat(64),
            None,
            &unreadable_settings,
            &rules(cwd, json!([])),
            &hooks()
        )
        .is_err()
    );
    let unreadable_policy =
        json!({"hooks":[],"policy":{"policyHookCount":0,"policyUnreadable":true}});
    assert!(
        ClaudeRestrictedFilesV1::compile(
            cwd,
            "a".repeat(64),
            None,
            &isolated_settings(),
            &rules(cwd, json!([])),
            &unreadable_policy
        )
        .is_err()
    );
    let hook =
        json!({"hooks":[{"source":"userSettings","disabled":true}],"policy":{"policyHookCount":0}});
    assert!(
        ClaudeRestrictedFilesV1::compile(
            cwd,
            "a".repeat(64),
            None,
            &isolated_settings(),
            &rules(cwd, json!([])),
            &hook
        )
        .is_err()
    );
}

#[test]
fn unknown_path_patterns_and_removed_saved_denies_cannot_create_a_valid_profile() {
    assert!(validate_deny("Read(/relative-to-source/**)").is_err());
    assert!(validate_deny("Edit(//workspace/../secret)").is_err());
    assert!(validate_deny("Read(!secret)").is_err());
    let directory = std::env::temp_dir();
    let cwd = directory.as_path();
    let mut saved = profile(cwd);
    saved.source_rules.push(SourceRule {
        source: "userSettings".into(),
        behavior: "deny".into(),
        rule: "Read".into(),
    });
    assert!(saved.validate(cwd).is_err());
}

#[test]
fn saved_local_tool_permissions_reject_unknown_capabilities() {
    let mut saved = serde_json::to_value(profile(&std::env::temp_dir())).unwrap();
    saved["localTools"] = json!({"allow_spawn":true,"allow_message":true,"allow_shell":true});
    assert!(serde_json::from_value::<ClaudeRestrictedFilesV1>(saved).is_err());
}

#[test]
fn captured_fixed_cli_rules_must_include_every_blocked_tool() {
    let cwd = std::env::temp_dir();
    let profile = profile(&cwd);
    let mut captured: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-fixed-profile-control.json"
    ))
    .unwrap();
    captured["rules"]["state"]["originalCwd"] = json!(cwd);
    profile
        .verify_live(
            &captured["settings"],
            &captured["rules"],
            &captured["hooks"],
            &captured["mcp"],
        )
        .unwrap();
    captured["rules"]["state"]["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|row| row["rule"] != "ExitPlanMode");
    assert!(
        profile
            .verify_live(
                &captured["settings"],
                &captured["rules"],
                &captured["hooks"],
                &captured["mcp"]
            )
            .is_err()
    );
}

#[test]
fn fixed_settings_changes_or_foreign_mcp_servers_never_pass_live_verification() {
    let directory = std::env::temp_dir();
    let cwd = directory.as_path();
    let profile = profile(cwd);
    let fixed = json!({"permissions":{"ask":["Edit"]},"sandbox":{"enabled":false}});
    let mut live =
        json!({"effective":fixed,"sources":[{"source":"flagSettings","settings":fixed}]});
    let mut captured: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-fixed-profile-control.json"
    ))
    .unwrap();
    captured["rules"]["state"]["originalCwd"] = json!(cwd);
    let scope = &captured["rules"];
    profile
        .verify_live(&live, scope, &hooks(), &json!({"mcpServers":[]}))
        .unwrap();
    assert!(
        profile
            .verify_live(
                &live,
                scope,
                &hooks(),
                &json!({"mcpServers":[{"name":"foreign"}]})
            )
            .is_err()
    );
    live["effective"]["permissions"]["allow"] = json!(["Edit"]);
    assert!(
        profile
            .verify_live(&live, scope, &hooks(), &json!({"mcpServers":[]}))
            .is_err()
    );
}

#[test]
fn approval_cannot_escape_the_workspace_or_rewrite_permission_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let profile = profile(&cwd);
    std::fs::create_dir(cwd.join(".claude")).unwrap();
    assert!(profile.approval_allowed("Edit", &json!({"file_path":cwd.join("new.txt")})));
    assert!(!profile.approval_allowed(
        "Edit",
        &json!({"file_path":cwd.join(".claude/settings.json")})
    ));
    assert!(!profile.approval_allowed("Edit", &json!({"file_path":cwd.join("../outside.txt")})));
    assert!(!profile.approval_allowed("Bash", &json!({"command":"true"})));
}

#[test]
fn approval_rejects_uppercase_permission_configuration_components() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let profile = profile(&cwd);
    std::fs::create_dir(cwd.join(".CLAUDE")).unwrap();
    std::fs::write(cwd.join(".CLAUDE/settings.json"), "{}").unwrap();
    std::fs::create_dir(cwd.join(".GIT")).unwrap();
    std::fs::write(cwd.join(".GIT/config"), "").unwrap();
    let settings = std::fs::canonicalize(cwd.join(".CLAUDE/settings.json")).unwrap();
    let git_config = std::fs::canonicalize(cwd.join(".GIT/config")).unwrap();

    assert!(!profile.approval_allowed("Edit", &json!({"file_path":settings})));
    assert!(!profile.approval_allowed("Edit", &json!({"file_path":git_config})));
}

#[test]
fn approval_rejects_mixed_case_permission_configuration_components() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let profile = profile(&cwd);
    std::fs::create_dir(cwd.join(".ClAuDe")).unwrap();
    std::fs::write(cwd.join(".ClAuDe/settings.json"), "{}").unwrap();
    std::fs::create_dir(cwd.join(".GiT")).unwrap();
    std::fs::write(cwd.join(".GiT/config"), "").unwrap();
    let settings = std::fs::canonicalize(cwd.join(".ClAuDe/settings.json")).unwrap();
    let git_config = std::fs::canonicalize(cwd.join(".GiT/config")).unwrap();

    assert!(!profile.approval_allowed("Edit", &json!({"file_path":settings})));
    assert!(!profile.approval_allowed("Edit", &json!({"file_path":git_config})));
}

#[test]
fn approval_rejects_uppercase_mcp_configuration_file() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let profile = profile(&cwd);
    std::fs::write(cwd.join(".MCP.JSON"), "{}").unwrap();
    let mcp_config = std::fs::canonicalize(cwd.join(".MCP.JSON")).unwrap();

    assert!(!profile.approval_allowed("Edit", &json!({"file_path":mcp_config})));
}

#[test]
fn approval_allows_ordinary_files_with_similar_configuration_names() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let profile = profile(&cwd);
    std::fs::write(cwd.join("CLAUDE.md"), "项目说明").unwrap();
    std::fs::write(cwd.join(".MCP.JSON.example"), "{}").unwrap();
    std::fs::create_dir(cwd.join(".claude-docs")).unwrap();
    std::fs::write(cwd.join(".claude-docs/guide.md"), "操作说明").unwrap();
    let project_help = std::fs::canonicalize(cwd.join("CLAUDE.md")).unwrap();
    let example = std::fs::canonicalize(cwd.join(".MCP.JSON.example")).unwrap();
    let guide = std::fs::canonicalize(cwd.join(".claude-docs/guide.md")).unwrap();

    assert!(profile.approval_allowed("Edit", &json!({"file_path":project_help})));
    assert!(profile.approval_allowed("Edit", &json!({"file_path":example})));
    assert!(profile.approval_allowed("Edit", &json!({"file_path":guide})));
}

#[cfg(unix)]
#[test]
fn approval_rechecks_symlink_targets_before_allowing_a_write() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(workspace.path()).unwrap();
    std::os::unix::fs::symlink(outside.path(), cwd.join("escape")).unwrap();
    assert!(
        !profile(&cwd).approval_allowed("Edit", &json!({"file_path":cwd.join("escape/new.txt")}))
    );
}

#[cfg(unix)]
#[test]
fn canonical_workspace_is_persisted_and_symlink_retargeting_never_moves_its_scope() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let replacement = directory.path().join("replacement");
    let alias = directory.path().join("alias");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&replacement).unwrap();
    std::os::unix::fs::symlink(&original, &alias).unwrap();
    let canonical = std::fs::canonicalize(&original).unwrap();
    // 原生通过别名启动时返回物理 originalCwd，与配置保存的用户路径不同。
    let created = ClaudeRestrictedFilesV1::compile(
        &alias,
        "a".repeat(64),
        None,
        &isolated_settings(),
        &rules(&canonical, json!([])),
        &hooks(),
    )
    .unwrap();
    let restored: ClaudeRestrictedFilesV1 =
        serde_json::from_value(serde_json::to_value(&created).unwrap()).unwrap();
    assert_eq!(restored.canonical_working_directory, canonical);
    assert!(restored.approval_allowed("Edit", &json!({"file_path":alias.join("safe.txt")})));
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&replacement, &alias).unwrap();
    assert!(restored.validate(&alias).is_err());
    assert!(restored.verify_source(&profile(&alias)).is_err());
    assert!(!restored.approval_allowed("Edit", &json!({"file_path":alias.join("outside.txt")})));
    assert!(restored.approval_allowed(
        "Edit",
        &json!({"file_path":original.join("still-authorized.txt")})
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_tmp_alias_accepts_files_inside_the_saved_physical_workspace() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let profile = profile(directory.path());
    assert_ne!(
        profile.working_directory,
        profile.canonical_working_directory
    );
    assert!(profile.approval_allowed(
        "Edit",
        &json!({"file_path":directory.path().join("safe.txt")})
    ));
}

#[cfg(windows)]
#[test]
fn windows_extended_canonical_prefix_does_not_reject_a_normal_workspace_path() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(directory.path()).unwrap();
    let value = canonical.to_str().unwrap();
    let normal = PathBuf::from(value.strip_prefix(r"\\?\").unwrap());
    let profile = profile(&normal);
    assert_ne!(normal, canonical);
    assert_eq!(profile.canonical_working_directory, canonical);
    assert!(profile.approval_allowed("Edit", &json!({"file_path":normal.join("safe.txt")})));
}

#[test]
fn only_manuals_native_default_mode_and_the_narrow_tool_set_are_accepted() {
    let directory = std::env::temp_dir();
    let profile = profile(&directory);
    profile
        .verify_system_init(&json!({"permissionMode":"default","tools":["Read","Edit"]}))
        .unwrap();
    for mode in ["plan", "dontAsk", "acceptEdits", "auto"] {
        assert!(
            profile
                .verify_system_init(&json!({"permissionMode":mode,"tools":["Read","Edit"]}))
                .is_err()
        );
    }
    assert!(
        profile
            .verify_system_init(&json!({"permissionMode":"default","tools":["Read","Edit","Bash"]}))
            .is_err()
    );
}

#[test]
fn legacy_user_denies_and_unknown_tool_globs_cannot_reenter_the_profile() {
    let cwd = std::env::temp_dir();
    assert!(
        ClaudeRestrictedFilesV1::compile(
            &cwd,
            "a".repeat(64),
            None,
            &external_settings("userSettings", vec!["*".into()], vec![]),
            &rules(
                &cwd,
                json!([{"source":"userSettings","behavior":"deny","rule":"*"}]),
            ),
            &hooks(),
        )
        .is_err()
    );
    assert!(
        ClaudeRestrictedFilesV1::compile(
            &cwd,
            "a".repeat(64),
            None,
            &external_settings("userSettings", vec!["Read*".into()], vec![]),
            &rules(
                &cwd,
                json!([{"source":"userSettings","behavior":"deny","rule":"Read*"}])
            ),
            &hooks()
        )
        .is_err()
    );
}

#[test]
fn local_task_mcp_keeps_an_explicit_approval_rule_and_cannot_expand_saved_rights() {
    let cwd = std::env::temp_dir();
    let mut profile = profile(&cwd);
    profile.local_tools = Some(LocalToolPermissions {
        allow_project_commands: false,
        allow_spawn: false,
        allow_message: true,
    });
    assert_eq!(
        profile.fixed_settings(),
        json!({"permissions":{"ask":["Edit",
        "mcp__infinishell-local-tasks__inspect_local_tasks", "mcp__infinishell-local-tasks__send_message_to_agent"]},"sandbox":{"enabled":false}})
    );
    assert!(profile.approval_allowed(
        "mcp__infinishell-local-tasks__inspect_local_tasks",
        &json!({})
    ));
    assert!(!profile.approval_allowed("mcp__infinishell-local-tasks__run_agents", &json!({})));
    let mut wider = profile.clone();
    wider.local_tools.as_mut().unwrap().allow_spawn = true;
    assert!(profile.verify_source(&wider).is_err());
}
