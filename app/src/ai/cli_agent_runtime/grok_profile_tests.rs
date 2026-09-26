use super::*;

fn policy(permissions: Option<LocalToolPermissions>) -> GrokCreationPolicyV1 {
    GrokCreationPolicyV1 {
        version: 1,
        storage_id: Uuid::new_v4(),
        canonical_working_directory: std::env::temp_dir().join("grok-policy-unit-test"),
        executable_sha256: "a".repeat(64),
        config_sha256: "b".repeat(64),
        tool_set: GrokToolSet::Read,
        local_tools: permissions,
        skills: None,
        commands: None,
    }
}

fn permissions(spawn: bool, message: bool) -> Option<LocalToolPermissions> {
    Some(LocalToolPermissions {
        allow_project_commands: false,
        allow_spawn: spawn,
        allow_message: message,
    })
}

#[test]
fn fixed_policy_binds_each_supported_version_to_its_exact_executable_digest() {
    for (version, other_version, digest) in [
        (
            "1.0.30",
            "1.0.34",
            "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb",
        ),
        (
            "1.0.30",
            "1.0.34",
            "504dd6546ab991b75d36698242875ce461489cd1f8cd84285873cb55bd5c7d54",
        ),
        (
            "1.0.30",
            "1.0.34",
            "ca24ea63272ba7881261f4a52498d1f5bd884b01da25845990422a10dd315266",
        ),
        (
            "1.0.34",
            "1.0.30",
            "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e",
        ),
        (
            "1.0.34",
            "1.0.30",
            "be5905e107d2b8b5f3c142d21ecfe4c8fd32a913d2fd551b788707930c4dc80d",
        ),
        (
            "1.0.34",
            "1.0.30",
            "021d8f7f6bdf9db48b6c87e799cd99130a76c463e6e6a3161510839aed016d94",
        ),
        (
            "1.0.40",
            "1.0.34",
            "3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2",
        ),
        (
            "1.0.40",
            "1.0.34",
            "92c997dfd109c0672d40d5ae6fbd15835d53ffaf12cf9ea124d22aaef3ff23fc",
        ),
        (
            "1.0.40",
            "1.0.34",
            "034c883fa3962ab6ca409c2d3c7501c642166535dd39fa936ecffe1ac2cad92e",
        ),
        (
            "1.0.41",
            "1.0.40",
            "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d",
        ),
        (
            "1.0.41",
            "1.0.40",
            "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407",
        ),
        (
            "1.0.41",
            "1.0.40",
            "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        ),
    ] {
        let mut profile = policy(None);
        profile.executable_sha256 = digest.into();
        assert!(profile.matches_cli_version(version));
        assert!(!profile.matches_cli_version(other_version));
        assert!(!profile.matches_cli_version("1.0.35"));
    }
}

#[test]
fn p0_policy_can_resume_only_with_the_same_verified_cli_identity() {
    let directory = tempfile::tempdir().unwrap();
    let executable_sha256 = "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e";
    let profile = GrokCreationPolicyV1::compile(
        directory.path(),
        "1.0.34",
        executable_sha256.into(),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();
    profile
        .validate_launch(
            directory.path(),
            "1.0.34",
            executable_sha256.into(),
            "b".repeat(64),
            PermissionPolicy::GrokRestrictedReadV1,
        )
        .unwrap();
    assert!(
        profile
            .validate_launch(
                directory.path(),
                "1.0.30",
                executable_sha256.into(),
                "b".repeat(64),
                PermissionPolicy::GrokRestrictedReadV1,
            )
            .is_err()
    );
}

#[test]
fn p0_runtime_scope_is_only_fixed_read_without_local_tools() {
    let directory = tempfile::tempdir().unwrap();
    let executable_sha256 = "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e";
    let read = GrokCreationPolicyV1::compile(
        directory.path(),
        "1.0.34",
        executable_sha256.into(),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();
    assert!(read.runtime_scope_verified("1.0.34", None, PermissionPolicy::GrokRestrictedReadV1));
    assert!(!read.runtime_scope_verified("1.0.34", None, PermissionPolicy::GrokRestrictedFilesV1));
    assert!(!read.runtime_scope_verified(
        "1.0.34",
        permissions(false, true),
        PermissionPolicy::GrokRestrictedReadV1
    ));
}

#[test]
fn p0_runtime_scope_rejects_unverified_file_profile_even_with_a_fixed_digest() {
    let directory = tempfile::tempdir().unwrap();
    let files = GrokCreationPolicyV1::compile(
        directory.path(),
        "1.0.34",
        "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e".into(),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedFilesV1,
    )
    .unwrap();

    assert!(!files.runtime_scope_verified("1.0.34", None, PermissionPolicy::GrokRestrictedFilesV1));
}

#[test]
fn current_fixed_bytes_do_not_enable_unverified_fixed_policy_scope() {
    let directory = tempfile::tempdir().unwrap();
    let read = GrokCreationPolicyV1::compile(
        directory.path(),
        "1.0.40",
        "3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2".into(),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();

    assert!(!read.runtime_scope_verified("1.0.40", None, PermissionPolicy::GrokRestrictedReadV1));
    assert!(!read.runtime_scope_verified(
        "1.0.40",
        permissions(false, true),
        PermissionPolicy::GrokRestrictedReadV1
    ));
}

#[test]
fn child_may_reduce_but_never_increase_sdk_permissions() {
    for parent_spawn in [false, true] {
        for parent_message in [false, true] {
            let parent = policy(permissions(parent_spawn, parent_message));
            assert!(parent.derive_child(None).is_ok());
            for child_spawn in [false, true] {
                for child_message in [false, true] {
                    let child = parent.derive_child(permissions(child_spawn, child_message));
                    let allowed =
                        (!child_spawn || parent_spawn) && (!child_message || parent_message);
                    assert_eq!(child.is_ok(), allowed);
                }
            }
        }
    }
    assert!(
        policy(None)
            .derive_child(permissions(false, false))
            .is_err()
    );
}

#[test]
fn child_cannot_change_the_immutable_creation_identity() {
    let parent = policy(permissions(true, true));
    assert!(parent.validate_child(&parent).is_ok());
    let mut child = parent.clone();
    child.config_sha256 = "c".repeat(64);
    assert!(parent.validate_child(&child).is_err());
    child = parent.clone();
    child.executable_sha256 = "d".repeat(64);
    assert!(parent.validate_child(&child).is_err());
    child = parent.clone();
    child.canonical_working_directory.push("other-project");
    assert!(parent.validate_child(&child).is_err());
}

#[test]
fn curated_profile_is_nonempty_and_contains_no_native_worker_or_write_tools() {
    for (permissions, expected) in [
        (None, vec![READ_TOOL]),
        (
            permissions(true, true),
            vec![READ_TOOL, SEARCH_TOOL, USE_TOOL],
        ),
    ] {
        let document = policy(permissions).profile_document().unwrap();
        let frontmatter = document.split("---\n").nth(1).unwrap();
        let profile: Value = serde_json::from_str(frontmatter.trim()).unwrap();
        let ids: Vec<&str> = profile["toolConfig"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, expected);
        // 原生托管工具另走 tools 白名单；只设 toolConfig 会留下继承全部的分支。
        assert_eq!(profile["tools"], json!(expected));
        assert_eq!(profile["injectDefaultTools"], false);
        assert_eq!(profile["discoverSkills"], false);
        assert_eq!(profile["inheritSkills"], false);
        assert_eq!(profile["agentsMd"], false);
        assert_eq!(profile["mcpInheritance"], "none");
        assert_eq!(profile["hooks"], json!({}));
    }
}

#[test]
fn sdk_permission_matches_the_complete_tool_target() {
    let parent = policy(permissions(true, true));
    for tool in tool_definitions(true, true) {
        let target = format!("{MCP_SERVER_NAME}__{}", tool["name"].as_str().unwrap());
        assert!(parent.permits_sdk_target(&target));
        assert!(!parent.permits_sdk_target(&format!("{target}__other")));
        assert!(!policy(None).permits_sdk_target(&target));
    }
    for target in [
        "other-server__run_agents",
        "infinishell-local-tasks__unknown",
        "infinishell-local-tasks-other__run_agents",
        "infinishell-local-tasks__",
    ] {
        assert!(!parent.permits_sdk_target(target));
    }
    let child = parent.derive_child(permissions(false, false)).unwrap();
    assert!(child.permits_sdk_target("infinishell-local-tasks__inspect_local_tasks"));
    assert!(!child.permits_sdk_target("infinishell-local-tasks__run_agents"));
    assert!(!child.permits_sdk_target("infinishell-local-tasks__send_message_to_agent"));
}

#[test]
fn malformed_or_unknown_persisted_policy_cannot_become_a_parent() {
    let parent = policy(permissions(true, true));
    for (key, value) in [
        ("version", json!(2)),
        ("executableSha256", json!("A".repeat(64))),
        ("configSha256", json!("b".repeat(63))),
        ("canonicalWorkingDirectory", json!("relative/project")),
    ] {
        let mut stored = serde_json::to_value(&parent).unwrap();
        stored[key] = value;
        let decoded: GrokCreationPolicyV1 = serde_json::from_value(stored).unwrap();
        assert!(decoded.derive_child(None).is_err());
        assert!(!decoded.permits_sdk_target("infinishell-local-tasks__run_agents"));
    }
    let mut stored = serde_json::to_value(&parent).unwrap();
    stored["fixedProfileVerified"] = json!(true);
    assert!(serde_json::from_value::<GrokCreationPolicyV1>(stored).is_err());
    let mut stored = serde_json::to_value(&parent).unwrap();
    stored["localTools"]["allow_shell"] = json!(true);
    assert!(serde_json::from_value::<GrokCreationPolicyV1>(stored).is_err());
}

#[test]
fn child_gets_separate_history_storage_without_expanding_creation_scope() {
    let parent = policy(permissions(true, true));
    let child = parent.derive_child(permissions(false, true)).unwrap();
    assert_ne!(child.storage_id, parent.storage_id);
    parent.validate_child(&child).unwrap();
    assert!(child.validate_child(&parent).is_err());
    assert_eq!(
        serde_json::from_value::<GrokCreationPolicyV1>(json!(child)).unwrap(),
        child
    );
}

#[test]
fn approval_cannot_increase_fixed_tool_set_or_use_another_server() {
    let profile = policy(permissions(true, false));
    assert!(profile.permits_native_request(&json!({"rawInput":{"variant":"UseTool","tool_name":"infinishell-local-tasks__run_agents","tool_input":{}}})));
    assert!(!profile.permits_native_request(&json!({"rawInput":{"variant":"UseTool","tool_name":"infinishell-local-tasks__send_message_to_agent","tool_input":{}}})));
    assert!(!profile.permits_native_request(&json!({"title":"read_file","rawInput":{"variant":"RunTerminalCommand","command":"touch forbidden"}})));
    assert!(!profile.permits_native_request(&json!({"rawInput":{"variant":"UseTool","tool_name":"other-server__run_agents","tool_input":{}}})));
}

#[test]
fn changed_saved_creation_files_are_rejected_instead_of_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    immutable_file(&path, b"original").unwrap();
    immutable_file(&path, b"original").unwrap();
    assert!(immutable_file(&path, b"expanded").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"original");
}

#[test]
fn catalog_expansion_and_missing_tool_never_confirm_fixed_creation() {
    let profile = policy(permissions(true, true));
    profile
        .verify_catalog(
            Some(&json!(["read_file", "search_tool", "use_tool"])),
            false,
        )
        .unwrap();
    assert!(
        profile
            .verify_catalog(
                Some(&json!(["read_file", "search_tool", "run_terminal_command"])),
                false
            )
            .is_err()
    );
    assert!(
        profile
            .verify_catalog(Some(&json!(["read_file", "read_file", "use_tool"])), false)
            .is_err()
    );
    assert!(
        profile
            .verify_catalog(Some(&json!(["read_file", "use_tool"])), false)
            .is_err()
    );
    assert!(profile.verify_catalog(Some(&json!(null)), false).is_err());
}

#[test]
fn cold_launch_drops_only_private_project_trust_and_preserves_history() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("grok")).unwrap();
    for name in ["trusted_folders.toml", "trusted-hook-projects"] {
        std::fs::write(home.path().join("grok").join(name), b"old project grant").unwrap();
    }
    let history = home.path().join("grok/session-history");
    std::fs::write(&history, b"saved native history").unwrap();
    reset_project_trust(home.path()).unwrap();
    reset_project_trust(home.path()).unwrap();
    assert!(!home.path().join("grok/trusted_folders.toml").exists());
    assert!(!home.path().join("grok/trusted-hook-projects").exists());
    assert_eq!(std::fs::read(history).unwrap(), b"saved native history");

    // 根目录由原生信任规则自动放行，不能成为固定策略父任务。
    let mut profile = policy(permissions(true, true));
    profile.canonical_working_directory = home.path().ancestors().last().unwrap().to_owned();
    assert!(profile.validate().is_err());
}

#[test]
fn auth_source_alias_is_rejected_before_any_original_file_is_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("grok");
    std::fs::create_dir(&home).unwrap();
    let auth = home.join("auth.json");
    std::fs::write(&auth, b"offline original cache").unwrap();
    let nested = home.join("nested");
    std::fs::create_dir(&nested).unwrap();
    for source in [&home, &nested.join("..")] {
        assert!(ensure_distinct_auth_source(source, &home).is_err());
        assert_eq!(std::fs::read(&auth).unwrap(), b"offline original cache");
    }
    #[cfg(unix)]
    {
        let alias = directory.path().join("source-link");
        std::os::unix::fs::symlink(&home, &alias).unwrap();
        assert!(ensure_distinct_auth_source(&alias, &home).is_err());
        assert_eq!(std::fs::read(&auth).unwrap(), b"offline original cache");
    }
    let distinct = directory.path().join("official-source");
    std::fs::create_dir(&distinct).unwrap();
    ensure_distinct_auth_source(&distinct, &home).unwrap();
    ensure_distinct_auth_source(&directory.path().join("no-source"), &home).unwrap();
}

#[test]
fn old_read_policy_round_trip_keeps_its_fields_and_profile_bytes() {
    let original = policy(None);
    let stored = json!({
        "version": 1,
        "storageId": original.storage_id,
        "canonicalWorkingDirectory": original.canonical_working_directory,
        "executableSha256": "a".repeat(64),
        "configSha256": "b".repeat(64),
        "localTools": null,
    });
    let restored: GrokCreationPolicyV1 = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), stored);
    assert_eq!(
        restored.permission_policy(),
        PermissionPolicy::GrokRestrictedReadV1
    );
    // 固定升级前字段及其插入次序，同时兼容工作区的 serde_json 排序特性。
    let legacy: Value = serde_json::from_str(r#"{"name":"infinishell-managed-grok-v1","description":"Application-managed read tools and local task tools.","permissionMode":"default","discoverSkills":false,"inheritSkills":false,"agentsMd":false,"injectDefaultTools":false,"toolConfig":{"tools":[{"id":"GrokBuild:read_file"}]},"tools":["GrokBuild:read_file"],"skills":[],"mcpServers":[],"mcpInheritance":"none","hooks":{}}"#).unwrap();
    assert_eq!(
        restored.profile_document().unwrap(),
        format!(
            "---\n{legacy}\n---\nUse the application local-task tools for managed child tasks.\n"
        )
    );
    assert!(!restored.permits_native_request(&json!({
        "kind":"edit", "_meta":{"x.ai/tool":{"name":"write"}},
        "rawInput":{"variant":"Write","file_path":std::env::temp_dir().join("example.txt"),"content":"blocked"}
    })));
}

fn file_policy(permissions: Option<LocalToolPermissions>) -> GrokCreationPolicyV1 {
    let mut profile = policy(permissions);
    profile.tool_set = GrokToolSet::Files;
    profile
}

#[test]
fn file_policy_is_explicit_and_its_catalog_cannot_add_shell_or_workers() {
    let profile = file_policy(None);
    let stored = serde_json::to_value(&profile).unwrap();
    assert_eq!(stored["toolSet"], "files");
    assert_eq!(
        serde_json::from_value::<GrokCreationPolicyV1>(stored).unwrap(),
        profile
    );
    let mut unknown = json!(profile);
    unknown["toolSet"] = json!("all");
    assert!(serde_json::from_value::<GrokCreationPolicyV1>(unknown).is_err());
    let document = profile.profile_document().unwrap();
    let native: Value =
        serde_json::from_str(document.split("---\n").nth(1).unwrap().trim()).unwrap();
    assert_eq!(native["name"], "infinishell-managed-grok-files-v1");
    assert_eq!(
        native["tools"],
        json!([
            "GrokBuild:read_file",
            "OpenCode:write",
            "GrokBuild:search_replace"
        ])
    );
    assert_eq!(native["permissionMode"], "default");
    assert_eq!(native["injectDefaultTools"], false);
    assert_eq!(native["hooks"], json!({}));
    assert_eq!(native["mcpInheritance"], "none");
    profile
        .verify_catalog(
            Some(&json!(["search_replace", "read_file", "write"])),
            false,
        )
        .unwrap();
    assert!(
        profile
            .verify_catalog(
                Some(&json!(["read_file", "write", "run_terminal_command"])),
                false
            )
            .is_err()
    );
    assert!(
        profile
            .verify_catalog(
                Some(&json!(["read_file", "write", "spawn_subagent"])),
                false
            )
            .is_err()
    );
    assert!(
        profile
            .verify_catalog(Some(&json!(["read_file", "write", "write"])), false)
            .is_err()
    );
    assert!(
        profile
            .verify_catalog(Some(&json!(["read_file", "search_replace"])), false)
            .is_err()
    );
}

#[test]
fn file_child_preserves_its_parent_tool_set_and_cannot_regain_removed_sdk_access() {
    let parent = file_policy(permissions(true, true));
    let child = parent.derive_child(permissions(false, true)).unwrap();
    assert_eq!(
        child.permission_policy(),
        PermissionPolicy::GrokRestrictedFilesV1
    );
    assert_ne!(child.storage_id, parent.storage_id);
    parent.validate_child(&child).unwrap();
    assert!(child.derive_child(permissions(true, true)).is_err());
    assert!(
        policy(permissions(true, true))
            .validate_child(&child)
            .is_err()
    );
    assert!(
        parent
            .validate_child(&policy(permissions(false, true)))
            .is_err()
    );
}

#[test]
fn resumed_tool_set_must_match_the_saved_policy_in_both_directions() {
    let directory = tempfile::tempdir().unwrap();
    let executable_sha256 = "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb";
    let read = GrokCreationPolicyV1::compile(
        directory.path(),
        "1.0.30",
        executable_sha256.into(),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();
    let files = GrokCreationPolicyV1::compile(
        directory.path(),
        "1.0.30",
        executable_sha256.into(),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedFilesV1,
    )
    .unwrap();
    read.validate_launch(
        directory.path(),
        "1.0.30",
        executable_sha256.into(),
        "b".repeat(64),
        PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();
    files
        .validate_launch(
            directory.path(),
            "1.0.30",
            executable_sha256.into(),
            "b".repeat(64),
            PermissionPolicy::GrokRestrictedFilesV1,
        )
        .unwrap();
    assert!(
        read.validate_launch(
            directory.path(),
            "1.0.30",
            executable_sha256.into(),
            "b".repeat(64),
            PermissionPolicy::GrokRestrictedFilesV1
        )
        .is_err()
    );
    assert!(
        files
            .validate_launch(
                directory.path(),
                "1.0.30",
                executable_sha256.into(),
                "b".repeat(64),
                PermissionPolicy::GrokRestrictedReadV1
            )
            .is_err()
    );
    assert!(
        read.validate_launch(
            directory.path(),
            "1.0.34",
            executable_sha256.into(),
            "b".repeat(64),
            PermissionPolicy::GrokRestrictedReadV1
        )
        .is_err()
    );
}

#[test]
fn file_approvals_require_exact_names_variants_and_known_input_fields() {
    let profile = file_policy(None);
    let write = json!({"kind":"edit","_meta":{"x.ai/tool":{"name":"write"}},
        "rawInput":{"variant":"Write","file_path":std::env::temp_dir().join("example.txt"),"content":"中文内容\n第二行"}});
    assert!(profile.permits_native_request(&write));
    assert!(!policy(None).permits_native_request(&write));
    let mut wrong = write.clone();
    wrong["_meta"]["x.ai/tool"]["name"] = json!("write_file");
    assert!(!profile.permits_native_request(&wrong));
    wrong = write.clone();
    wrong["rawInput"]["variant"] = json!("RunTerminalCommand");
    assert!(!profile.permits_native_request(&wrong));
    wrong = write.clone();
    wrong["rawInput"]["command"] = json!("echo expanded");
    assert!(!profile.permits_native_request(&wrong));
    wrong = write.clone();
    wrong["rawInput"]["file_path"] = json!("relative.txt");
    assert!(!profile.permits_native_request(&wrong));
    wrong = write.clone();
    wrong["rawInput"]["content"] = json!(false);
    assert!(!profile.permits_native_request(&wrong));
    wrong = write;
    wrong["kind"] = json!("execute");
    assert!(!profile.permits_native_request(&wrong));
}

#[test]
fn search_replace_contract_accepts_empty_strings_but_not_unknown_arguments() {
    let profile = file_policy(None);
    let edit = json!({"kind":"edit","_meta":{"x.ai/tool":{"name":"search_replace"}},
        "rawInput":{"variant":"SearchReplace","file_path":"src/example.rs","old_string":"","new_string":""}});
    assert!(profile.permits_native_request(&edit));
    assert!(!policy(None).permits_native_request(&edit));
    let mut replace_all = edit.clone();
    replace_all["rawInput"]["replace_all"] = json!(true);
    assert!(profile.permits_native_request(&replace_all));
    replace_all["rawInput"]["replace_all"] = Value::Null;
    assert!(!profile.permits_native_request(&replace_all));
    let mut unknown = edit.clone();
    unknown["rawInput"]["variant"] = json!("EditFile");
    assert!(!profile.permits_native_request(&unknown));
    unknown = edit.clone();
    unknown["rawInput"]["execute_after_write"] = json!(true);
    assert!(!profile.permits_native_request(&unknown));
    unknown = edit.clone();
    unknown["rawInput"]["file_path"] = json!("");
    assert!(!profile.permits_native_request(&unknown));
    unknown = edit;
    unknown["rawInput"]
        .as_object_mut()
        .unwrap()
        .remove("new_string");
    assert!(!profile.permits_native_request(&unknown));
}

#[test]
fn switching_saved_profile_mode_cannot_rewrite_immutable_launch_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = file_policy(None);
    let root = directory
        .path()
        .join("grok-managed")
        .join(files.storage_id.to_string());
    std::fs::create_dir_all(root.join("grok")).unwrap();
    immutable_file(
        &root.join("grok/config.toml"),
        FIXED_CONFIGURATION.as_bytes(),
    )
    .unwrap();
    let document = files.profile_document().unwrap();
    immutable_file(&root.join("profile.md"), document.as_bytes()).unwrap();
    files.verify_files(directory.path()).unwrap();
    files.tool_set = GrokToolSet::Read;
    assert!(files.verify_files(directory.path()).is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("profile.md")).unwrap(),
        document
    );
}

#[test]
fn persistent_profile_storage_survives_host_generations_without_crossing_scope() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().canonicalize().unwrap();
    let profile = policy(permissions(true, true));
    let home = profile
        .prepare_storage(&state, &SessionTarget::New)
        .unwrap();
    create_private_directory(&home.join("grok")).unwrap();
    immutable_file(
        &home.join("grok/config.toml"),
        profile.fixed_configuration().as_bytes(),
    )
    .unwrap();
    immutable_file(
        &home.join("profile.md"),
        profile.profile_document().unwrap().as_bytes(),
    )
    .unwrap();
    let saved = home.join("saved-session");
    std::fs::write(&saved, b"existing-session-state").unwrap();
    let target = SessionTarget::Resume {
        native_session_id: Uuid::new_v4().to_string(),
    };

    for generation in [Uuid::new_v4(), Uuid::new_v4()] {
        let process_state = state
            .join("cli-agent-hosts")
            .join(generation.to_string())
            .join("native");
        create_private_directory(&process_state).unwrap();
        assert_eq!(profile.prepare_storage(&state, &target).unwrap(), home);
        profile.verify_files(&state).unwrap();
        // 进程状态域缺少原存储时必须拒绝，不能搜索其他代次或创建空会话替代。
        assert!(profile.prepare_storage(&process_state, &target).is_err());
        assert!(
            !process_state
                .join("grok-managed")
                .join(profile.storage_id.to_string())
                .exists()
        );
    }
    let mut other = profile.clone();
    other.storage_id = Uuid::new_v4();
    assert!(other.prepare_storage(&state, &target).is_err());
    assert!(other.verify_files(&state).is_err());
    other = profile.clone();
    other.tool_set = GrokToolSet::Files;
    assert!(other.verify_files(&state).is_err());
    assert_eq!(std::fs::read(saved).unwrap(), b"existing-session-state");
}

#[cfg(unix)]
#[test]
fn resumed_profile_storage_rejects_symlink_to_another_scope() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().canonicalize().unwrap();
    let profile = policy(None);
    let other = state.join("other-scope");
    create_private_directory(&other).unwrap();
    let parent = state.join("grok-managed");
    create_private_directory(&parent).unwrap();
    std::os::unix::fs::symlink(&other, parent.join(profile.storage_id.to_string())).unwrap();
    assert!(
        profile
            .prepare_storage(
                &state,
                &SessionTarget::Resume {
                    native_session_id: Uuid::new_v4().to_string(),
                }
            )
            .is_err()
    );
    assert!(std::fs::read_dir(other).unwrap().next().is_none());
    assert!(
        profile
            .prepare_storage(Path::new("relative"), &SessionTarget::New)
            .is_err()
    );
}

#[test]
fn rejected_catalog_reports_only_known_names_and_structure() {
    let profile = policy(permissions(true, true));
    let tools = json!([
        "read_file", "read_file", "write", "search_replace", "search_tool", "use_tool",
        "OFFLINE_PRIVATE_TOOL:/private/secret", "OFFLINE_PRIVATE_TOOL:/private/secret",
        {"OFFLINE_PRIVATE_KEY": "OFFLINE_PRIVATE_BODY"}, null, 7
    ]);
    for task_input_sent in [false, true] {
        let error = profile
            .verify_catalog(Some(&tools), task_input_sent)
            .unwrap_err();
        let evidence = error.permission_ceiling_evidence().unwrap();
        assert_eq!(evidence["reason"], "grok_creation_catalog_changed");
        assert_eq!(evidence["task_input_sent"], task_input_sent);
        assert_eq!(
            evidence["actual"],
            json!({
                "creationPolicy": "GrokCreationPolicyV1",
                "catalog": {
                    "value_type": "array", "item_count": 11,
                    "expected_names": ["read_file", "search_tool", "use_tool"],
                    "known_name_counts": {
                        "read_file": 2, "write": 1, "search_replace": 1,
                        "search_tool": 1, "use_tool": 1, "grep": 0, "list_dir": 0
                    },
                    "unknown_string_count": 2, "non_string_count": 3,
                    "duplicate_string_count": 2
                }
            })
        );
        let serialized = evidence.to_string();
        assert!(!serialized.contains("OFFLINE_PRIVATE"));
        assert!(!serialized.contains("/private/secret"));
    }
}

#[test]
fn rejected_catalog_distinguishes_missing_null_and_non_array_without_content() {
    let profile = policy(None);
    for (tools, expected_type) in [
        (None, "missing"),
        (Some(json!(null)), "null"),
        (Some(json!(true)), "boolean"),
        (Some(json!(42)), "number"),
        (Some(json!("OFFLINE_PRIVATE_STRING")), "string"),
        (
            Some(json!({"OFFLINE_PRIVATE_KEY": "OFFLINE_PRIVATE_BODY"})),
            "object",
        ),
    ] {
        let error = profile.verify_catalog(tools.as_ref(), false).unwrap_err();
        let evidence = error.permission_ceiling_evidence().unwrap();
        let catalog = &evidence["actual"]["catalog"];
        assert_eq!(evidence["reason"], "grok_creation_catalog_missing");
        assert_eq!(evidence["task_input_sent"], false);
        assert_eq!(catalog["value_type"], expected_type);
        assert_eq!(catalog["item_count"], Value::Null);
        assert_eq!(catalog["unknown_string_count"], 0);
        assert_eq!(catalog["non_string_count"], 0);
        assert_eq!(catalog["duplicate_string_count"], 0);
        assert!(!evidence.to_string().contains("OFFLINE_PRIVATE"));
    }
    let error = profile.verify_catalog(Some(&json!([])), false).unwrap_err();
    let evidence = error.permission_ceiling_evidence().unwrap();
    assert_eq!(evidence["reason"], "grok_creation_catalog_changed");
    assert_eq!(evidence["actual"]["catalog"]["value_type"], "array");
    assert_eq!(evidence["actual"]["catalog"]["item_count"], 0);
}

#[test]
fn catalog_requires_exact_builtin_or_complete_served_local_union() {
    let profile = policy(permissions(true, true));
    let names: Vec<String> = ["inspect_local_tasks", "run_agents", "send_message_to_agent"]
        .iter()
        .map(|name| format!("infinishell-local-tasks__{name}"))
        .collect();
    let builtin = json!(["read_file", "search_tool", "use_tool"]);
    profile
        .verify_catalog_with_mcp(Some(&builtin), false, &names)
        .unwrap();
    let mut union = builtin.as_array().unwrap().clone();
    union.extend(names.iter().map(|name| json!(name)));
    profile
        .verify_catalog_with_mcp(Some(&json!(union)), true, &names)
        .unwrap();
    assert!(
        profile
            .verify_catalog_with_mcp(Some(&json!(union)), true, &[])
            .is_err()
    );
    for index in 0..union.len() {
        let mut missing = union.clone();
        missing.remove(index);
        assert!(
            profile
                .verify_catalog_with_mcp(Some(&json!(missing)), true, &names)
                .is_err()
        );
    }
    for unexpected in [
        "run_agents",
        "infinishell_local_tasks__run_agents",
        "other__run_agents",
        "unknown",
    ] {
        let mut expanded = union.clone();
        expanded.push(json!(unexpected));
        assert!(
            profile
                .verify_catalog_with_mcp(Some(&json!(expanded)), true, &names)
                .is_err()
        );
    }
    let mut duplicate = union.clone();
    duplicate[0] = duplicate[3].clone();
    assert!(
        profile
            .verify_catalog_with_mcp(Some(&json!(duplicate)), true, &names)
            .is_err()
    );
}

#[test]
fn current_fixed_scope_preserves_exact_binary_configuration_and_child_subset() {
    let directory = tempfile::tempdir().unwrap();
    for selected in [
        PermissionPolicy::GrokRestrictedReadV1,
        PermissionPolicy::GrokRestrictedFilesV1,
    ] {
        let parent = GrokCreationPolicyV1::compile(
            directory.path(),
            FIXED_SCOPE_VERSION,
            "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d".into(),
            format!("{:x}", Sha256::digest(fixed_configuration(true).as_bytes())),
            permissions(true, true),
            selected,
        )
        .unwrap();
        assert_eq!(
            parent.runtime_scope_verified(FIXED_SCOPE_VERSION, permissions(true, true), selected),
            cfg!(any(target_os = "macos", target_os = "linux", windows))
        );
        assert!(!parent.runtime_scope_verified(CURRENT_VERSION, permissions(true, true), selected));
        assert!(!parent.runtime_scope_verified(
            FIXED_SCOPE_VERSION,
            permissions(false, true),
            selected
        ));
        assert!(parent.fixed_configuration().contains("grok-4.7"));
        assert!(!parent.fixed_configuration().contains("grok-4.6-build"));
        let child = parent.derive_child(permissions(false, true)).unwrap();
        assert!(parent.validate_child(&child).is_ok());
        assert!(child.validate_child(&parent).is_err());
        assert!(child.permits_sdk_target("infinishell-local-tasks__send_message_to_agent"));
        assert!(!child.permits_sdk_target("infinishell-local-tasks__run_agents"));
    }
    assert!(fixed_configuration(false).contains("grok-4.6-build"));
}
