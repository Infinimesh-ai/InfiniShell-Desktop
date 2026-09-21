use super::*;
use serde_json::json;
use tempfile::TempDir;

fn private_root() -> (TempDir, PathBuf) {
    let directory = TempDir::new().unwrap();
    let root = directory.path().canonicalize().unwrap();
    (directory, root)
}

#[cfg(unix)]
fn executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = root.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn grok_before() -> Vec<u8> {
    b"# preserve comment\n[cli]\ninstaller = \"internal\"\nchannel = \"stable\"\nauto_update = true\n[ui]\nyolo = false\ncompact_mode = true\n".to_vec()
}

fn grok_after() -> Vec<u8> {
    b"[cli]\ninstaller = \"internal\"\nchannel = \"stable\"\nauto_update = true\n[ui]\nyolo = false\ncompact_mode = true\nmax_thoughts_width = 120\nfork_secondary_model = \"grok-4.6\"\n".to_vec()
}

#[test]
fn alpha_versions_sort_below_same_core_release_and_compare_numeric_components() {
    assert_eq!(
        compare_versions("0.156.0-alpha.7", "0.155.1"),
        Ok(std::cmp::Ordering::Greater)
    );
    assert_eq!(
        compare_versions("0.156.0-alpha.7", "0.156.0"),
        Ok(std::cmp::Ordering::Less)
    );
    assert_eq!(
        compare_versions("0.156.0-alpha.9", "0.156.0-alpha.10"),
        Ok(std::cmp::Ordering::Less)
    );
    for invalid in [
        "latest",
        "1.0",
        "1.0.0;echo",
        "1.0.0\n",
        "1.0.0-alpha.-1",
        "01.0.0",
        "1.0.0-alpha.1.2.3",
        "1.0.0+metadata",
        "18446744073709551616.0.0",
    ] {
        assert_eq!(
            parse_version(invalid),
            Err(Error::InvalidRelease),
            "{invalid}"
        );
    }
}

#[test]
fn channels_are_agent_specific() {
    assert!(channel_supported(CLIAgent::Codex, Channel::Alpha));
    assert!(!channel_supported(CLIAgent::Codex, Channel::Stable));
    assert!(channel_supported(CLIAgent::Claude, Channel::Stable));
    assert!(!channel_supported(CLIAgent::Claude, Channel::Alpha));
    assert!(channel_supported(CLIAgent::Grok, Channel::Alpha));
    assert!(!channel_supported(CLIAgent::Grok, Channel::Latest));
    assert!(!channel_supported(CLIAgent::Unknown, Channel::Latest));
}

#[test]
fn updater_uses_each_adapter_exact_version_contract() {
    assert!(adapter_supports_version(CLIAgent::Codex, "0.155.1"));
    assert!(!adapter_supports_version(CLIAgent::Codex, "0.155.2"));
    assert!(adapter_supports_version(CLIAgent::Claude, "2.1.278"));
    assert!(!adapter_supports_version(CLIAgent::Claude, "2.1.279"));
    assert!(adapter_supports_version(CLIAgent::Grok, "1.0.40"));
    assert!(!adapter_supports_version(CLIAgent::Grok, "1.0.41"));
    assert!(!adapter_supports_version(CLIAgent::Gemini, "1.0.40"));
}

fn compatible_plugins() -> PluginCompatibility {
    PluginCompatibility {
        installed: true,
        disabled: false,
        needs_update: false,
        platform_installed: true,
        platform_needs_update: false,
    }
}

#[test]
fn missing_plugin_fails_compatibility_verification() {
    assert!(
        !PluginCompatibility {
            installed: false,
            ..compatible_plugins()
        }
        .verified()
    );
}

#[test]
fn disabled_plugin_fails_compatibility_verification() {
    assert!(
        !PluginCompatibility {
            disabled: true,
            ..compatible_plugins()
        }
        .verified()
    );
}

#[test]
fn outdated_plugin_fails_compatibility_verification() {
    assert!(
        !PluginCompatibility {
            needs_update: true,
            ..compatible_plugins()
        }
        .verified()
    );
}

#[test]
fn missing_or_outdated_platform_plugin_fails_compatibility_verification() {
    assert!(
        !PluginCompatibility {
            platform_installed: false,
            ..compatible_plugins()
        }
        .verified()
    );
    assert!(
        !PluginCompatibility {
            platform_needs_update: true,
            ..compatible_plugins()
        }
        .verified()
    );
}

#[test]
fn complete_plugin_installation_passes_compatibility_verification() {
    assert!(compatible_plugins().verified());
}

#[cfg(unix)]
#[tokio::test]
async fn updater_version_probe_parses_the_supervised_command_output() {
    let (_directory, root) = private_root();
    let executable = executable_script(&root, "claude", "printf '2.1.278 (Claude Code)\\n'");

    let detected = version(CLIAgent::Claude, &executable).await;

    assert_eq!(detected, Ok("2.1.278".to_owned()));
}

#[cfg(unix)]
#[tokio::test]
async fn updater_version_probe_reports_its_bounded_timeout() {
    let (_directory, root) = private_root();
    let executable = executable_script(&root, "claude", "exec sleep 3600");

    let detected =
        version_with_timeout(CLIAgent::Claude, &executable, Duration::from_millis(20)).await;

    assert_eq!(detected, Err(Error::TimedOut));
}

#[tokio::test]
async fn verification_progress_without_ack_still_converges() {
    let (entered_tx, entered_rx) = async_channel::bounded(1);
    let (_resume_tx, resume_rx) = async_channel::bounded(1);
    let progress = VerificationProgress::new(entered_tx, resume_rx);

    tokio::time::timeout(
        Duration::from_secs(1),
        progress.enter_with_timeout(Duration::from_millis(20)),
    )
    .await
    .unwrap();

    assert_eq!(entered_rx.try_recv(), Ok(()));
}

#[test]
fn claude_follows_existing_channel_but_rejects_unrecognized_settings() {
    let (_directory, root) = private_root();
    let config = root.join("settings.json");
    assert_eq!(claude_channel(&config), Ok(Channel::Latest));
    fs::write(
        &config,
        br#"{"autoUpdatesChannel":"stable","permissions":{"defaultMode":"default"}}"#,
    )
    .unwrap();
    assert_eq!(claude_channel(&config), Ok(Channel::Stable));
    fs::write(&config, br#"{"autoUpdatesChannel":[]}"#).unwrap();
    assert_eq!(claude_channel(&config), Err(Error::ChannelMismatch));
    fs::write(&config, b"[]").unwrap();
    assert_eq!(claude_channel(&config), Err(Error::ProbeFailed));
}

#[test]
fn grok_read_only_check_must_match_entry_version_and_native_installer() {
    let mut value = json!({"currentVersion":"1.0.30","latestVersion":"1.0.34","installer":"internal","channel":"stable","error":null});
    assert_eq!(
        validate_grok_check(&serde_json::to_vec(&value).unwrap(), "1.0.30"),
        Ok(Channel::Stable)
    );
    value["currentVersion"] = json!("1.0.29");
    assert_eq!(
        validate_grok_check(&serde_json::to_vec(&value).unwrap(), "1.0.30"),
        Err(Error::UnsupportedSource)
    );
    value["currentVersion"] = json!("1.0.30");
    value["installer"] = json!("homebrew");
    assert_eq!(
        validate_grok_check(&serde_json::to_vec(&value).unwrap(), "1.0.30"),
        Err(Error::UnsupportedSource)
    );
    value["installer"] = json!("internal");
    value["channel"] = json!("unknown");
    assert_eq!(
        validate_grok_check(&serde_json::to_vec(&value).unwrap(), "1.0.30"),
        Err(Error::ChannelMismatch)
    );
}

#[test]
fn known_grok_serialization_changes_preserve_permissions_and_preferences() {
    assert!(grok_config_delta_allowed(
        &grok_before(),
        &grok_after(),
        "stable"
    ));
    for altered in [
        String::from_utf8(grok_after())
            .unwrap()
            .replace("auto_update = true", "auto_update = false"),
        String::from_utf8(grok_after())
            .unwrap()
            .replace("yolo = false", "yolo = true"),
        String::from_utf8(grok_after())
            .unwrap()
            .replace("compact_mode = true", "compact_mode = false"),
        String::from_utf8(grok_after())
            .unwrap()
            .replace("grok-4.6", "unknown-model"),
        format!(
            "{}new_field = true\n",
            String::from_utf8(grok_after()).unwrap()
        ),
    ] {
        assert!(!grok_config_delta_allowed(
            &grok_before(),
            altered.as_bytes(),
            "stable"
        ));
    }
}

#[test]
fn channel_override_only_accepts_selected_native_output() {
    let after = String::from_utf8(grok_after())
        .unwrap()
        .replace("channel = \"stable\"", "channel = \"alpha\"");
    assert!(grok_config_delta_allowed(
        &grok_before(),
        after.as_bytes(),
        "alpha"
    ));
    assert!(!grok_config_delta_allowed(
        &grok_before(),
        after.as_bytes(),
        "stable"
    ));
}

#[test]
fn byte_restore_keeps_comments_and_rejects_concurrent_user_changes() {
    let (_directory, root) = private_root();
    let path = root.join("config.toml");
    let backup = ConfigBackup {
        desired: None,
        before_mode: None,
        restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
        kind: ConfigKind::Grok,
        path: path.clone(),
        before: Some(grok_before()),
        after: Some(grok_after()),
    };
    fs::write(&path, grok_after()).unwrap();
    restore_config(&backup).unwrap();
    assert_eq!(fs::read(&path).unwrap(), grok_before());
    fs::write(&path, b"# newer user edit\n").unwrap();
    assert_eq!(restore_config(&backup), Err(Error::RecoveryRequired));
    assert_eq!(fs::read(&path).unwrap(), b"# newer user edit\n");
}

#[test]
fn restore_of_originally_absent_config_only_removes_recorded_native_file() {
    let (_directory, root) = private_root();
    let path = root.join("config.toml");
    let backup = ConfigBackup {
        desired: None,
        before_mode: None,
        restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
        kind: ConfigKind::Grok,
        path: path.clone(),
        before: None,
        after: Some(grok_after()),
    };
    fs::write(&path, grok_after()).unwrap();
    restore_config(&backup).unwrap();
    assert!(!path.exists());
    fs::write(&path, b"# new user file").unwrap();
    assert_eq!(restore_config(&backup), Err(Error::RecoveryRequired));
    assert_eq!(fs::read(&path).unwrap(), b"# new user file");
}

fn journal(root: &Path, phase: &str) -> Journal {
    Journal {
        claude_update: None,
        binding_digest: None,
        binding_kind: None,
        launch: Some(JournalLaunch {
            program: root.join("grok"),
            args: vec![OsString::from("update")],
            cwd: root.to_owned(),
        }),
        publish_desired: false,
        command_failed: false,
        intent: None,
        generation: None,
        old_stamp: None,
        schema: 2,
        agent: "grok".to_owned(),
        entry: root.join("grok"),
        old_version: "1.0.30".to_owned(),
        target_version: "1.0.34".to_owned(),
        phase: phase.to_owned(),
        config: Some(ConfigBackup {
            desired: None,
            before_mode: None,
            restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
            kind: ConfigKind::Grok,
            path: root.join("config.toml"),
            before: Some(grok_before()),
            after: Some(grok_after()),
        }),
        channel: "stable".to_owned(),
    }
}

#[cfg(feature = "local_fs")]
fn bind_journal(record: &mut Journal) -> managed_process::PreparedLaunchBinding {
    let binding = managed_process::PreparedLaunchBinding::new("a".repeat(64)).unwrap();
    record.binding_digest = Some(binding.digest().to_owned());
    record.binding_kind = binding.kind_name().map(ToOwned::to_owned);
    binding
}

#[cfg(feature = "local_fs")]
fn record_bound_not_started(root: &Path, record: &Journal) {
    let binding = journal_binding(record).unwrap();
    managed_process::record_not_started_with_binding(
        root,
        record.generation.unwrap(),
        &record.entry,
        &[],
        root,
        &binding,
    )
    .unwrap();
}

#[test]
fn claude_update_metadata_keeps_installation_policy_without_account_or_history() {
    let bytes = br#"{"installMethod":"native","autoUpdates":false,"autoUpdatesProtectedForNative":true,"autoUpdaterStatus":"disabled","oauthAccount":{"synthetic":"private"},"projects":{"synthetic":"private"},"machineID":"private","bypassPermissionsModeAccepted":true}"#;
    let projected = claude_update_metadata(Some(bytes)).unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&projected).unwrap(),
        json!({"installMethod":"native","autoUpdates":false,"autoUpdatesProtectedForNative":true,"autoUpdaterStatus":"disabled"})
    );
    assert_eq!(claude_update_metadata(None).unwrap(), None);
}

#[test]
fn claude_update_metadata_rejects_malformed_policy_instead_of_using_defaults() {
    assert!(claude_update_metadata(Some(br#"{"autoUpdates":"false"}"#)).is_err());
    assert!(claude_update_metadata(Some(br#"{"autoUpdatesProtectedForNative":1}"#)).is_err());
    assert!(claude_update_metadata(Some(br#"{"installMethod":null}"#)).is_err());
    assert!(claude_update_metadata(Some(br#"{"autoUpdaterStatus":"unknown"}"#)).is_err());
    assert!(claude_update_metadata(Some(b"[]")).is_err());
}

#[test]
fn startup_recognizes_pending_journal_before_any_network_or_cli_call() {
    let (_directory, root) = private_root();
    assert!(!recovery_pending_in(&root, CLIAgent::Grok));
    fs::write(root.join("grok.json"), b"incomplete journal").unwrap();
    assert!(recovery_pending_in(&root, CLIAgent::Grok));
    assert!(!recovery_pending_in(&root, CLIAgent::Claude));
}

#[test]
fn restart_finishes_only_recorded_success_and_preserves_unknown_prepared_transaction() {
    let (_directory, root) = private_root();
    let path = root.join("grok.json");
    let mut pending = journal(&root, "prepared");
    fs::write(root.join("config.toml"), grok_after()).unwrap();
    save_journal(&path, &pending).unwrap();
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.34", &root),
        Err(Error::RecoveryRequired)
    );
    assert!(path.exists());
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), grok_after());
    pending.phase = "command_returned".to_owned();
    save_journal(&path, &pending).unwrap();
    reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.34", &root).unwrap();
    assert!(!path.exists());
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), grok_before());
}

#[test]
fn recovery_rejects_wrong_version_entry_and_concurrent_configuration() {
    let (_directory, root) = private_root();
    let path = root.join("grok.json");
    let pending = journal(&root, "command_returned");
    fs::write(root.join("config.toml"), grok_after()).unwrap();
    save_journal(&path, &pending).unwrap();
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.30", &root),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &root.join("another-cli"), "1.0.34", &root),
        Err(Error::RecoveryRequired)
    );
    fs::write(root.join("config.toml"), b"# changed after native update").unwrap();
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.34", &root),
        Err(Error::RecoveryRequired)
    );
    assert!(path.exists());
}

#[test]
fn file_identity_detects_replacement_without_size_change() {
    let (_directory, root) = private_root();
    let path = root.join("launcher");
    fs::write(&path, b"source-one").unwrap();
    let before = stamp(&path).unwrap();
    fs::write(&path, b"source-two").unwrap();
    assert_ne!(stamp(&path).unwrap(), before);
}

#[test]
fn structured_roles_arguments_and_target_version_define_binding_digest() {
    let (_directory, root) = private_root();
    let node = root.join("node");
    let npm = root.join("npm-cli.js");
    let prefix = root.join("prefix");
    let mut invocation = BoundInvocation {
        artifacts: [
            (ArtifactRole::Program, node.clone()),
            (ArtifactRole::Helper, npm.clone()),
            (ArtifactRole::InstallRoot, prefix.clone()),
        ]
        .into_iter()
        .collect(),
        program: ArtifactRole::Program,
        arguments: vec![
            ArgumentRef::ArtifactPath {
                role: ArtifactRole::Helper,
                relative: PathBuf::new(),
            },
            ArgumentRef::Literal("install".into()),
            ArgumentRef::ArtifactPath {
                role: ArtifactRole::InstallRoot,
                relative: PathBuf::new(),
            },
            ArgumentRef::PackageVersion {
                package: "@openai/codex".to_owned(),
            },
        ],
        environment: vec![("CODEX_RELEASE".into(), ArgumentRef::TargetVersion)],
        env_remove: Vec::new(),
        binding: LaunchBindingSpec::NativeFile {
            program: ArtifactRole::Program,
        },
    };

    let first = invocation.prepare("0.155.1").unwrap();
    let second = invocation.prepare("0.156.0-alpha.7").unwrap();
    assert_eq!(first.invocation.program, node);
    assert_eq!(
        first.invocation.args,
        [
            npm.into_os_string(),
            OsString::from("install"),
            prefix.into_os_string(),
            OsString::from("@openai/codex@0.155.1"),
        ]
    );
    assert_eq!(
        first.invocation.env,
        [(OsString::from("CODEX_RELEASE"), OsString::from("0.155.1"))]
    );
    assert_eq!(first.binding.kind_name(), Some("native_file"));
    assert_ne!(first.binding.digest(), second.binding.digest());

    invocation
        .artifacts
        .insert(ArtifactRole::InstallRoot, root.join("another-prefix"));
    assert_ne!(
        invocation.prepare("0.155.1").unwrap().binding.digest(),
        first.binding.digest()
    );
}

#[test]
fn journal_round_trip_preserves_atomic_execution_kind() {
    let (_directory, root) = private_root();
    let mut record = journal(&root, "prepared");
    let binding = managed_process::PreparedLaunchBinding::native_file("a".repeat(64)).unwrap();
    record.binding_digest = Some(binding.digest().to_owned());
    record.binding_kind = binding.kind_name().map(ToOwned::to_owned);
    let decoded: Journal = serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();

    assert_eq!(journal_binding(&decoded).unwrap(), binding);
}

#[test]
fn manual_only_binding_never_materializes_a_pathname_invocation() {
    let (_directory, root) = private_root();
    let invocation = BoundInvocation::manual_only(
        [(ArtifactRole::Program, root.join("brew"))],
        ArtifactRole::Program,
        vec![ArgumentRef::Literal("upgrade".into())],
        "Homebrew 依赖闭包未绑定",
    );

    assert!(matches!(
        invocation.prepare("1.0.0"),
        Err(Error::UnsupportedSource)
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn journal_digest_mismatch_rejects_an_otherwise_confirmed_receipt() {
    let (_directory, root) = private_root();
    let mut record = journal(&root, "command_returned");
    fs::write(&record.entry, b"unchanged installation").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.generation = Some(Uuid::new_v4());
    bind_journal(&mut record);
    record.config = None;
    record.binding_digest = Some("a".repeat(64));
    save_journal(&root.join("grok.json"), &record).unwrap();
    let other = managed_process::PreparedLaunchBinding::new("b".repeat(64)).unwrap();
    managed_process::record_not_started_with_binding(
        &root,
        record.generation.unwrap(),
        &record.entry,
        &[],
        &root,
        &other,
    )
    .unwrap();

    assert_eq!(
        preflight_recovery(CLIAgent::Grok, &record.entry, &root),
        Err(Error::RecoveryRequired)
    );
}

#[test]
fn supervised_dependencies_include_every_controlled_file_and_reject_replacement() {
    let (_directory, root) = private_root();
    let entry = root.join("agent");
    let manager_path = root.join("manager");
    let helper_path = root.join("helper");
    let registration_path = root.join("package.json");
    let unrelated = root.join("unrelated");
    fs::write(&entry, b"agent-one").unwrap();
    fs::write(&manager_path, b"manager-one").unwrap();
    fs::write(&helper_path, b"helper-one").unwrap();
    fs::write(&registration_path, b"registration-one").unwrap();
    fs::write(&unrelated, b"unrelated").unwrap();
    let manager = stamp(&manager_path).unwrap();
    let helper = stamp(&helper_path).unwrap();
    let registration = stamp(&registration_path).unwrap();
    let installation = Installation {
        source: Source::Npm,
        entry: entry.clone(),
        stamp: stamp(&entry).unwrap(),
        manager: Some(manager.clone()),
        helper: Some(helper),
        registration: Some((registration_path.clone(), registration)),
        invocation: None,
        channel: Channel::Latest,
        config: None,
        error: None,
        source_target: None,
    };
    let invocation = Invocation::new(&manager_path, ["update"]);
    let binding = managed_process::PreparedLaunchBinding::new("a".repeat(64)).unwrap();

    assert_eq!(verify_installation_identity(&installation), Ok(()));
    assert_eq!(
        expected_invocation_stamp(&installation, &invocation),
        Ok(&manager)
    );
    assert_eq!(
        supervised_dependency_paths(&installation, &invocation),
        Ok(vec![
            manager_path.clone(),
            entry.clone(),
            helper_path.clone(),
            registration_path.clone(),
        ])
    );
    assert_eq!(
        capture_supervised_dependencies(&installation, &invocation, &binding)
            .unwrap()
            .1
            .len(),
        4
    );
    assert_eq!(
        expected_invocation_stamp(&installation, &Invocation::new(&unrelated, ["update"])),
        Err(Error::SourceChanged)
    );

    fs::write(&helper_path, b"helper-two").unwrap();
    assert_eq!(
        verify_installation_identity(&installation),
        Err(Error::SourceChanged)
    );
    fs::write(&helper_path, b"helper-one").unwrap();
    fs::write(&registration_path, b"registration-two").unwrap();
    assert_eq!(
        capture_supervised_dependencies(&installation, &invocation, &binding).map(|_| ()),
        Err(Error::SourceChanged)
    );
    fs::write(&registration_path, b"registration-one").unwrap();
    fs::remove_file(&helper_path).unwrap();
    assert_eq!(
        capture_supervised_dependencies(&installation, &invocation, &binding).map(|_| ()),
        Err(Error::SourceChanged)
    );
}

#[test]
fn supervised_dependencies_deduplicate_canonical_paths_but_keep_executable() {
    let (_directory, root) = private_root();
    let entry = root.join("agent");
    let manager_path = root.join("manager");
    fs::write(&entry, b"agent-one").unwrap();
    fs::write(&manager_path, b"manager-one").unwrap();
    let manager = stamp(&manager_path).unwrap();
    let installation = Installation {
        source: Source::Npm,
        entry: entry.clone(),
        stamp: stamp(&entry).unwrap(),
        manager: Some(manager.clone()),
        helper: Some(manager.clone()),
        registration: Some((manager_path.clone(), manager)),
        invocation: None,
        channel: Channel::Latest,
        config: None,
        error: None,
        source_target: None,
    };
    let invocation = Invocation::new(&manager_path, ["update"]);
    let binding = managed_process::PreparedLaunchBinding::new("b".repeat(64)).unwrap();

    assert_eq!(
        supervised_dependency_paths(&installation, &invocation),
        Ok(vec![manager_path.clone(), entry])
    );
    assert_eq!(
        capture_supervised_dependencies(&installation, &invocation, &binding)
            .unwrap()
            .1
            .len(),
        2
    );
}

#[test]
#[cfg(unix)]
fn native_file_capture_freezes_only_the_canonical_program_object() {
    let (_directory, root) = private_root();
    let program = root.join("native-program");
    let entry = root.join("entry");
    fs::write(&program, b"native-program").unwrap();
    std::os::unix::fs::symlink(&program, &entry).unwrap();
    let installation = Installation {
        source: Source::Native,
        entry: entry.clone(),
        stamp: stamp(&entry).unwrap(),
        manager: None,
        helper: None,
        registration: None,
        invocation: None,
        channel: Channel::Latest,
        config: None,
        error: None,
        source_target: None,
    };
    let invocation = Invocation::new(program.canonicalize().unwrap(), ["update"]);
    let binding = managed_process::PreparedLaunchBinding::native_file("c".repeat(64)).unwrap();

    let (_, expected) =
        capture_supervised_dependencies(&installation, &invocation, &binding).unwrap();

    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].path(), program.canonicalize().unwrap());
    assert!(binding.is_native_file());
}

#[test]
fn windows_reparse_attribute_is_never_plain() {
    assert!(windows_file_attributes_are_plain(0));
    assert!(windows_file_attributes_are_plain(0x20));
    assert!(!windows_file_attributes_are_plain(0x400));
    assert!(!windows_file_attributes_are_plain(0x420));
}

#[test]
fn file_size_limit_and_unknown_journal_fields_fail_closed() {
    let (_directory, root) = private_root();
    let path = root.join("data");
    fs::write(&path, b"four").unwrap();
    assert_eq!(read_limited(&path, 3), Err(Error::SourceChanged));
    let mut value = serde_json::to_value(journal(&root, "verified")).unwrap();
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<Journal>(value).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_recovery_configuration_is_never_overwritten() {
    use std::os::unix::fs::symlink;
    let (_directory, root) = private_root();
    let target = root.join("user-data");
    let path = root.join("config.toml");
    fs::write(&target, grok_after()).unwrap();
    symlink(&target, &path).unwrap();
    let backup = ConfigBackup {
        desired: None,
        before_mode: None,
        restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
        kind: ConfigKind::Grok,
        path,
        before: Some(grok_before()),
        after: Some(grok_after()),
    };
    assert!(restore_config(&backup).is_err());
    assert_eq!(fs::read(target).unwrap(), grok_after());
}

#[test]
fn grok_missing_channel_is_restored_as_missing_after_selected_override() {
    let before = b"[cli]\ninstaller = \"internal\"\nauto_update = true\n";
    let after = b"[cli]\ninstaller = \"internal\"\nauto_update = true\nchannel = \"alpha\"\n";
    assert!(grok_config_delta_allowed(before, after, "alpha"));
    assert!(!grok_config_delta_allowed(before, after, "stable"));
    assert!(grok_config_delta_allowed(b"", b"[cli]\ninstaller = \"internal\"\nchannel = \"stable\"\n[ui]\nyolo = false\ncompact_mode = false\nmax_thoughts_width = 120\nfork_secondary_model = \"grok-4.6\"\n", "stable"));
}

#[test]
fn failed_codex_update_restores_previous_native_marker_bytes() {
    let (_directory, root) = private_root();
    let path = root.join("auto-update-version");
    let backup = ConfigBackup {
        desired: None,
        before_mode: None,
        restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
        kind: ConfigKind::CodexUpdateMarker,
        path: path.clone(),
        before: Some(b"0.155.1\n".to_vec()),
        after: None,
    };
    assert!(config_delta_allowed(&backup, None, "latest"));
    assert!(!config_delta_allowed(
        &backup,
        Some(b"unknown replacement"),
        "latest"
    ));
    restore_config(&backup).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"0.155.1\n");
    fs::write(&path, b"concurrent user marker").unwrap();
    assert_eq!(restore_config(&backup), Err(Error::RecoveryRequired));
    assert_eq!(fs::read(&path).unwrap(), b"concurrent user marker");
}

#[test]
fn no_spawn_failure_can_remove_journal_only_when_original_installation_is_unchanged() {
    let (_directory, root) = private_root();
    let mut pending = journal(&root, "prepared");
    fs::write(&pending.entry, b"original binary").unwrap();
    fs::write(root.join("config.toml"), grok_before()).unwrap();
    let identity = stamp(&pending.entry).unwrap();
    let path = root.join("grok.json");
    save_journal(&path, &pending).unwrap();
    rollback_unstarted(&path, &pending, &identity).unwrap();
    assert!(!path.exists());
    save_journal(&path, &pending).unwrap();
    fs::write(&pending.entry, b"external replacement").unwrap();
    assert_eq!(
        rollback_unstarted(&path, &pending, &identity),
        Err(Error::RecoveryRequired)
    );
    assert!(path.exists());
    pending.config = None;
    assert_eq!(
        rollback_unstarted(&path, &pending, &identity),
        Err(Error::RecoveryRequired)
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn confirmed_unstarted_update_recovers_old_installation_and_persists_retry_suppression() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut record = journal(&root, "prepared");
    fs::write(&record.entry, b"unchanged installation").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.generation = Some(Uuid::new_v4());
    bind_journal(&mut record);
    record.config = Some(ConfigBackup {
        desired: None,
        before_mode: None,
        restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
        kind: ConfigKind::Grok,
        path: root.join("missing-config.toml"),
        before: None,
        after: None,
    });
    let path = root.join("grok.json");
    save_journal(&path, &record).unwrap();
    let old = record.old_version.clone();
    assert_eq!(
        preflight_recovery(CLIAgent::Grok, &record.entry, &root),
        Ok(true)
    );
    let receipt = managed_process::confirmed_exit_with_binding(
        &root,
        record.generation.unwrap(),
        &journal_binding(&record).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(receipt.containment, "not_started");
    reconcile_confirmed_update(&path, &mut record, CLIAgent::Grok, &root, &old).unwrap();
    assert!(!path.exists());
    assert!(!recovery_pending_in(&root, CLIAgent::Grok));
    assert_eq!(
        previous_failure(&root, CLIAgent::Grok, &record.entry).unwrap(),
        Some(record.target_version)
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn preflight_closes_every_supported_updater_generation_that_never_started() {
    for (agent, config) in [
        (CLIAgent::Codex, None),
        (
            CLIAgent::Claude,
            Some((
                ConfigKind::Claude,
                br#"{"autoUpdatesChannel":"latest"}"#.to_vec(),
            )),
        ),
        (CLIAgent::Grok, Some((ConfigKind::Grok, grok_before()))),
    ] {
        let (_directory, root) = private_root();
        let mut record = journal(&root, "prepared");
        record.agent = agent.command_prefix().to_owned();
        record.entry = root.join(agent.command_prefix());
        record.launch = Some(JournalLaunch {
            program: record.entry.clone(),
            args: vec![OsString::from("update")],
            cwd: root.clone(),
        });
        match agent {
            CLIAgent::Codex => {
                record.old_version = "0.154.0".to_owned();
                record.target_version = "0.155.1".to_owned();
                record.channel = "latest".to_owned();
            }
            CLIAgent::Claude => {
                record.old_version = "2.1.267".to_owned();
                record.target_version = "2.1.278".to_owned();
                record.channel = "latest".to_owned();
            }
            CLIAgent::Grok => {}
            _ => unreachable!(),
        }
        fs::write(&record.entry, b"unchanged installation").unwrap();
        record.old_stamp = Some(stamp(&record.entry).unwrap());
        record.generation = Some(Uuid::new_v4());
        bind_journal(&mut record);
        record.config = config.map(|(kind, before)| {
            let path = root.join(if matches!(kind, ConfigKind::Claude) {
                "settings.json"
            } else {
                "config.toml"
            });
            fs::write(&path, &before).unwrap();
            ConfigBackup {
                kind,
                path,
                before: Some(before.clone()),
                after: None,
                desired: Some(ConfigDesired {
                    bytes: Some(before),
                }),
                before_mode: None,
                restore_stage: None,
            }
        });
        if agent == CLIAgent::Claude {
            record.claude_update = Some(
                snapshot_claude_scope(
                    record.config.as_ref().unwrap(),
                    record.generation.unwrap(),
                    root.join(".claude.json"),
                    &record.target_version,
                )
                .unwrap(),
            );
        }
        let journal_path = root.join(format!("{}.json", agent.command_prefix()));
        save_journal(&journal_path, &record).unwrap();

        assert_eq!(preflight_recovery(agent, &record.entry, &root), Ok(true));
        let receipt = managed_process::confirmed_exit_with_binding(
            &root,
            record.generation.unwrap(),
            &journal_binding(&record).unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(receipt.containment, "not_started");
        assert_eq!(receipt.exit_code, None);
        let old_version = record.old_version.clone();
        reconcile_confirmed_update(&journal_path, &mut record, agent, &root, &old_version).unwrap();
        assert!(!journal_path.exists());
        assert_eq!(
            previous_failure(&root, agent, &record.entry).unwrap(),
            Some(record.target_version.clone())
        );
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn missing_generation_after_command_returned_is_not_rewritten_as_not_started() {
    let (_directory, root) = private_root();
    let mut record = journal(&root, "command_returned");
    fs::write(&record.entry, b"unchanged installation").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.generation = Some(Uuid::new_v4());
    record.config = None;
    save_journal(&root.join("grok.json"), &record).unwrap();

    assert_eq!(
        preflight_recovery(CLIAgent::Grok, &record.entry, &root),
        Err(Error::RecoveryRequired)
    );
    assert!(
        managed_process::confirmed_exit(&root, record.generation.unwrap())
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn pre_spawn_source_failure_receipt_is_idempotent() {
    let (_directory, root) = private_root();
    let generation = Uuid::new_v4();
    let program = root.join("agent");
    let args = [OsString::from("update")];
    let binding = managed_process::PreparedLaunchBinding::new("a".repeat(64)).unwrap();

    assert_eq!(
        record_not_started_if_missing(&root, generation, &program, &args, &binding),
        Ok(true)
    );
    assert_eq!(
        record_not_started_if_missing(&root, generation, &program, &args, &binding),
        Ok(false)
    );
    let receipt = managed_process::confirmed_exit_with_binding(&root, generation, &binding)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.containment, "not_started");
    assert_eq!(receipt.exit_code, None);
}

#[cfg(feature = "local_fs")]
#[test]
fn preflight_not_started_receipt_uses_the_persisted_updater_command() {
    let (_directory, root) = private_root();
    let mut record = journal(&root, "prepared");
    let generation = Uuid::new_v4();
    let manager = root.join("manager");
    record.generation = Some(generation);
    record.launch = Some(JournalLaunch {
        program: manager.clone(),
        args: vec![OsString::from("install"), OsString::from("grok@1.0.34")],
        cwd: root.clone(),
    });
    bind_journal(&mut record);
    fs::write(&record.entry, b"unchanged installation").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    save_journal(&root.join("grok.json"), &record).unwrap();

    assert_eq!(
        preflight_recovery(CLIAgent::Grok, &record.entry, &root),
        Ok(true)
    );
    let manifest: Value = serde_json::from_slice(
        &fs::read(
            root.join("cli-agent-processes")
                .join(generation.to_string())
                .join("manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["executable"], json!(manager));
    let arguments: Vec<OsString> = serde_json::from_value(manifest["arguments"].clone()).unwrap();
    assert_eq!(
        arguments,
        [OsString::from("install"), OsString::from("grok@1.0.34")]
    );
    assert_eq!(manifest["cwd"], json!(root));
}

#[cfg(feature = "local_fs")]
#[test]
fn preflight_rejects_old_or_malformed_pre_spawn_journals_without_a_receipt() {
    for malformed_claude_phase in [false, true] {
        let (_directory, root) = private_root();
        let mut record = journal(
            &root,
            if malformed_claude_phase {
                "claude_config_preparing"
            } else {
                "prepared"
            },
        );
        let generation = Uuid::new_v4();
        record.generation = Some(generation);
        if malformed_claude_phase {
            record.agent = "grok".to_owned();
        } else {
            record.launch = None;
        }
        fs::write(&record.entry, b"unchanged installation").unwrap();
        record.old_stamp = Some(stamp(&record.entry).unwrap());
        save_journal(&root.join("grok.json"), &record).unwrap();

        assert_eq!(
            preflight_recovery(CLIAgent::Grok, &record.entry, &root),
            Err(Error::RecoveryRequired)
        );
        assert!(
            managed_process::confirmed_exit(&root, generation)
                .unwrap()
                .is_none()
        );
    }

    let (_directory, root) = private_root();
    let mut record = journal(&root, "claude_config_preparing");
    let generation = Uuid::new_v4();
    record.agent = "claude".to_owned();
    record.entry = root.join("claude");
    record.launch = Some(JournalLaunch {
        program: record.entry.clone(),
        args: vec![OsString::from("update")],
        cwd: root.clone(),
    });
    record.generation = Some(generation);
    fs::write(&record.entry, b"unchanged installation").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    save_journal(&root.join("claude.json"), &record).unwrap();

    assert_eq!(
        preflight_recovery(CLIAgent::Claude, &record.entry, &root),
        Err(Error::RecoveryRequired)
    );
    assert!(
        managed_process::confirmed_exit(&root, generation)
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn confirmed_update_recovery_rejects_changed_old_installation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut record = journal(&root, "prepared");
    fs::write(&record.entry, b"original").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.generation = Some(Uuid::new_v4());
    bind_journal(&mut record);
    record.config = None;
    let path = root.join("grok.json");
    save_journal(&path, &record).unwrap();
    record_bound_not_started(&root, &record);
    fs::write(&record.entry, b"changed concurrently").unwrap();
    let old = record.old_version.clone();
    assert_eq!(
        reconcile_confirmed_update(&path, &mut record, CLIAgent::Grok, &root, &old),
        Err(Error::RecoveryRequired)
    );
    assert!(path.exists());
}

#[cfg(feature = "local_fs")]
#[test]
fn verified_update_recovery_accepts_already_restored_channel_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for original in [None, Some(b"[cli]\nchannel = \"alpha\"\n".to_vec())] {
        let mut record = journal(&root, "verified");
        record.generation = Some(Uuid::new_v4());
        bind_journal(&mut record);
        let config_path = root.join("config.toml");
        if let Some(bytes) = &original {
            fs::write(&config_path, bytes).unwrap();
        }
        record.config = Some(ConfigBackup {
            desired: None,
            before_mode: None,
            restore_stage: Some(root.join(format!(".infinishell-cli-update-{}", Uuid::new_v4()))),
            kind: ConfigKind::Grok,
            path: config_path,
            before: original.clone(),
            after: original,
        });
        let path = root.join("grok.json");
        save_journal(&path, &record).unwrap();
        record_bound_not_started(&root, &record);
        let target = record.target_version.clone();
        reconcile_confirmed_update(&path, &mut record, CLIAgent::Grok, &root, &target).unwrap();
        assert!(!path.exists());
    }
}

fn atomic_user_save(path: &Path, bytes: &[u8]) {
    let mut file = NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    file.write_all(bytes).unwrap();
    file.as_file().sync_all().unwrap();
    file.persist(path).unwrap();
}

#[test]
fn config_save_after_check_is_preserved_in_claim_and_returned_without_overwrite() {
    let (_directory, root) = private_root();
    let config = journal(&root, "command_returned").config.unwrap();
    fs::write(&config.path, grok_after()).unwrap();
    let user = b"# user saved after final comparison\n";
    let result = restore_config_with_hook(&config, |point| {
        if point == ConfigRestorePoint::BeforeClaim {
            atomic_user_save(&config.path, user);
        }
        Ok(())
    });
    assert_eq!(result, Err(Error::RecoveryRequired));
    assert_eq!(fs::read(&config.path).unwrap(), user);
    assert_eq!(
        fs::read(config.restore_stage.as_ref().unwrap().join("claimed")).unwrap(),
        user
    );
    assert_eq!(restore_config(&config), Err(Error::RecoveryRequired));
    assert_eq!(
        cleanup_config_restore(&config),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(fs::read(&config.path).unwrap(), user);
}

#[test]
fn user_save_after_claim_blocks_restore_and_preserves_both_files() {
    for point in [
        ConfigRestorePoint::AfterClaim,
        ConfigRestorePoint::BeforePublish,
    ] {
        let (_directory, root) = private_root();
        let config = journal(&root, "command_returned").config.unwrap();
        fs::write(&config.path, grok_after()).unwrap();
        let user = b"# newer file owns the original path\n";
        let result = restore_config_with_hook(&config, |at| {
            if at == point {
                atomic_user_save(&config.path, user);
            }
            Ok(())
        });
        assert_eq!(result, Err(Error::RecoveryRequired));
        assert_eq!(fs::read(&config.path).unwrap(), user);
        assert_eq!(
            fs::read(config.restore_stage.as_ref().unwrap().join("claimed")).unwrap(),
            grok_after()
        );
        assert_eq!(restore_config(&config), Err(Error::RecoveryRequired));
        assert_eq!(fs::read(&config.path).unwrap(), user);
    }
}

#[test]
fn originally_absent_configuration_does_not_delete_new_save_after_claim() {
    let (_directory, root) = private_root();
    let mut config = journal(&root, "command_returned").config.unwrap();
    config.before = None;
    fs::write(&config.path, grok_after()).unwrap();
    let user = b"# newly created user config\n";
    assert_eq!(
        restore_config_with_hook(&config, |point| {
            if point == ConfigRestorePoint::AfterClaim {
                atomic_user_save(&config.path, user);
            }
            Ok(())
        }),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(fs::read(&config.path).unwrap(), user);
    assert_eq!(
        fs::read(config.restore_stage.as_ref().unwrap().join("claimed")).unwrap(),
        grok_after()
    );
}

#[test]
fn config_restore_resumes_persisted_claim_at_each_crash_boundary() {
    for stop in [
        ConfigRestorePoint::BeforeClaim,
        ConfigRestorePoint::AfterClaim,
        ConfigRestorePoint::BeforePublish,
        ConfigRestorePoint::AfterPublish,
    ] {
        let (_directory, root) = private_root();
        let pending = journal(&root, "command_returned");
        let path = root.join("grok.json");
        save_journal(&path, &pending).unwrap();
        let config = pending.config.as_ref().unwrap();
        fs::write(&config.path, grok_after()).unwrap();
        assert_eq!(
            restore_config_with_hook(config, |point| {
                if point == stop {
                    Err(Error::RecoveryRequired)
                } else {
                    Ok(())
                }
            }),
            Err(Error::RecoveryRequired)
        );
        assert!(path.exists());
        reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.34", &root).unwrap();
        assert_eq!(fs::read(&config.path).unwrap(), grok_before());
        assert!(!path.exists());
        assert!(!config.restore_stage.as_ref().unwrap().exists());
    }
}

#[test]
fn restart_keeps_conflicting_claim_even_when_original_path_matches_before() {
    let (_directory, root) = private_root();
    let pending = journal(&root, "command_returned");
    let path = root.join("grok.json");
    let config = pending.config.as_ref().unwrap();
    save_journal(&path, &pending).unwrap();
    fs::write(&config.path, grok_after()).unwrap();
    assert_eq!(
        restore_config_with_hook(config, |point| {
            if point == ConfigRestorePoint::AfterClaim {
                return Err(Error::RecoveryRequired);
            }
            Ok(())
        }),
        Err(Error::RecoveryRequired)
    );
    let claimed = config.restore_stage.as_ref().unwrap().join("claimed");
    fs::write(&claimed, b"unknown saved config").unwrap();
    atomic_user_save(&config.path, &grok_before());
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.34", &root),
        Err(Error::RecoveryRequired)
    );
    assert!(path.exists());
    assert_eq!(fs::read(claimed).unwrap(), b"unknown saved config");
    assert_eq!(fs::read(&config.path).unwrap(), grok_before());
}

#[test]
fn verified_journal_cleans_claim_idempotently_after_restore() {
    let (_directory, root) = private_root();
    let mut pending = journal(&root, "command_returned");
    let path = root.join("grok.json");
    let config = pending.config.as_ref().unwrap();
    fs::write(&config.path, grok_after()).unwrap();
    restore_config(config).unwrap();
    pending.phase = "verified".to_owned();
    save_journal(&path, &pending).unwrap();
    let config = pending.config.as_ref().unwrap();
    cleanup_config_restore(config).unwrap();
    // 清理暂存后、删除 journal 前崩溃，重启只能核对原字节，不再次认领用户文件。
    reconcile_journal(CLIAgent::Grok, &pending.entry, "1.0.34", &root).unwrap();
    assert_eq!(fs::read(&config.path).unwrap(), grok_before());
    assert!(!path.exists());
}

#[test]
fn config_restore_rejects_stage_outside_same_parent() {
    let (_directory, root) = private_root();
    let mut config = journal(&root, "command_returned").config.unwrap();
    config.restore_stage = Some(
        root.join("other")
            .join(format!(".infinishell-cli-update-{}", Uuid::new_v4())),
    );
    fs::write(&config.path, grok_after()).unwrap();
    assert_eq!(restore_config(&config), Err(Error::RecoveryRequired));
    assert_eq!(fs::read(&config.path).unwrap(), grok_after());
}

fn selected_backup(
    root: &Path,
    kind: ConfigKind,
    before: Option<Vec<u8>>,
    channel: Channel,
) -> ConfigBackup {
    let desired = selected_channel_config(kind, before.as_deref(), Some(channel)).unwrap();
    let mut config = ConfigBackup {
        kind,
        path: root.join(if matches!(kind, ConfigKind::Claude) {
            "settings.json"
        } else {
            "config.toml"
        }),
        after: before.clone(),
        before,
        desired: Some(ConfigDesired { bytes: desired }),
        before_mode: None,
        restore_stage: None,
    };
    plan_config_publish(&mut config, true).unwrap();
    config
}

#[test]
fn explicit_grok_channel_preserves_comments_and_all_other_semantics() {
    let before = grok_before();
    let desired = selected_channel_config(ConfigKind::Grok, Some(&before), Some(Channel::Alpha))
        .unwrap()
        .unwrap();
    let text = std::str::from_utf8(&desired).unwrap();
    assert!(text.starts_with("# preserve comment\n"));
    let mut expected = std::str::from_utf8(&before)
        .unwrap()
        .parse::<toml::Value>()
        .unwrap();
    expected["cli"]["channel"] = toml::Value::String("alpha".to_owned());
    assert_eq!(text.parse::<toml::Value>().unwrap(), expected);
    assert_eq!(
        selected_channel_config(ConfigKind::Grok, Some(&desired), Some(Channel::Alpha)).unwrap(),
        Some(desired)
    );
}

#[test]
fn explicit_claude_channel_preserves_permission_and_unknown_settings() {
    let before = br#"{"autoUpdatesChannel":"latest","permissions":{"defaultMode":"default","deny":["Bash(rm:*)"]},"env":{"SYNTHETIC":"kept"},"unknown":{"value":3}}"#;
    let desired = selected_channel_config(ConfigKind::Claude, Some(before), Some(Channel::Stable))
        .unwrap()
        .unwrap();
    let mut expected: Value = serde_json::from_slice(before).unwrap();
    expected["autoUpdatesChannel"] = json!("stable");
    assert_eq!(serde_json::from_slice::<Value>(&desired).unwrap(), expected);
    assert!(
        !String::from_utf8(desired)
            .unwrap()
            .contains("bypassPermissions")
    );
}

#[test]
fn follow_and_matching_defaults_do_not_create_or_reformat_configuration() {
    for (kind, default) in [
        (ConfigKind::Grok, Channel::Stable),
        (ConfigKind::Claude, Channel::Latest),
    ] {
        assert_eq!(selected_channel_config(kind, None, None).unwrap(), None);
        assert_eq!(
            selected_channel_config(kind, None, Some(default)).unwrap(),
            None
        );
    }
    let bytes = b"# keep exact bytes\n[cli]\nchannel = 'stable'\n";
    assert_eq!(
        selected_channel_config(ConfigKind::Grok, Some(bytes), None).unwrap(),
        Some(bytes.to_vec())
    );
    assert_eq!(
        selected_channel_config(ConfigKind::Grok, Some(bytes), Some(Channel::Stable)).unwrap(),
        Some(bytes.to_vec())
    );
}

#[test]
fn missing_configuration_gets_only_the_explicit_channel_and_malformed_input_is_rejected() {
    let grok = selected_channel_config(ConfigKind::Grok, None, Some(Channel::Alpha))
        .unwrap()
        .unwrap();
    let parsed = std::str::from_utf8(&grok)
        .unwrap()
        .parse::<toml::Value>()
        .unwrap();
    assert_eq!(
        parsed,
        "[cli]\nchannel='alpha'\n".parse::<toml::Value>().unwrap()
    );
    let claude = selected_channel_config(ConfigKind::Claude, None, Some(Channel::Stable))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&claude).unwrap(),
        json!({"autoUpdatesChannel":"stable"})
    );
    for bytes in [
        b"[]".as_slice(),
        br#"{"autoUpdatesChannel":false}"#,
        br#"{"autoUpdatesChannel":"unknown"}"#,
    ] {
        assert!(
            selected_channel_config(ConfigKind::Claude, Some(bytes), Some(Channel::Stable))
                .is_err()
        );
    }
    for bytes in [
        b"cli=false".as_slice(),
        b"[cli]\nchannel=3",
        b"[cli]\nchannel='unknown'",
    ] {
        assert!(
            selected_channel_config(ConfigKind::Grok, Some(bytes), Some(Channel::Alpha)).is_err()
        );
    }
}

#[test]
fn same_version_channel_sync_recovers_without_a_supervisor_or_updater() {
    for (kind, channel, before) in [
        (ConfigKind::Grok, Channel::Alpha, grok_before()),
        (
            ConfigKind::Claude,
            Channel::Stable,
            br#"{"autoUpdatesChannel":"latest","permissions":{"defaultMode":"default"}}"#.to_vec(),
        ),
    ] {
        let (_directory, root) = private_root();
        let mut record = journal(&root, "config_prepared");
        fs::write(&record.entry, b"same version binary").unwrap();
        record.old_stamp = Some(stamp(&record.entry).unwrap());
        record.old_version = record.target_version.clone();
        record.config = Some(selected_backup(&root, kind, Some(before.clone()), channel));
        record.channel = channel_name(channel).unwrap().to_owned();
        let config = record.config.as_ref().unwrap();
        fs::write(&config.path, &before).unwrap();
        let desired = config.publication_bytes(true).cloned().unwrap();
        let config_path = config.path.clone();
        let path = root.join("grok.json");
        save_journal(&path, &record).unwrap();
        assert_eq!(
            preflight_recovery(CLIAgent::Grok, &record.entry, &root),
            Ok(true)
        );
        reconcile_journal(CLIAgent::Grok, &record.entry, &record.target_version, &root).unwrap();
        assert_eq!(fs::read(config_path).unwrap(), desired);
        assert!(!path.exists());
        assert!(!root.join("cli-agent-processes").exists());
        assert_eq!(fs::read(&record.entry).unwrap(), b"same version binary");
    }
}

#[test]
fn same_version_channel_sync_rejects_changed_binary_and_user_configuration() {
    let (_directory, root) = private_root();
    let mut record = journal(&root, "config_prepared");
    fs::write(&record.entry, b"old binary").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.old_version = record.target_version.clone();
    record.config = Some(selected_backup(
        &root,
        ConfigKind::Grok,
        Some(grok_before()),
        Channel::Alpha,
    ));
    let config = record.config.as_ref().unwrap();
    fs::write(&config.path, grok_before()).unwrap();
    let path = root.join("grok.json");
    save_journal(&path, &record).unwrap();
    fs::write(&record.entry, b"new binary").unwrap();
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &record.entry, &record.target_version, &root),
        Err(Error::RecoveryRequired)
    );
    fs::write(&record.entry, b"old binary").unwrap();
    fs::write(&config.path, b"# user replacement\n").unwrap();
    assert_eq!(
        reconcile_journal(CLIAgent::Grok, &record.entry, &record.target_version, &root),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(fs::read(&config.path).unwrap(), b"# user replacement\n");
    assert!(path.exists());
}

#[test]
fn desired_channel_resumes_each_crash_boundary_without_reverting_to_before() {
    for stop in [
        ConfigRestorePoint::BeforeClaim,
        ConfigRestorePoint::AfterClaim,
        ConfigRestorePoint::BeforePublish,
        ConfigRestorePoint::AfterPublish,
    ] {
        let (_directory, root) = private_root();
        let config = selected_backup(&root, ConfigKind::Grok, Some(grok_before()), Channel::Alpha);
        fs::write(&config.path, grok_before()).unwrap();
        assert_eq!(
            publish_config_with_hook(&config, true, |point| if point == stop {
                Err(Error::RecoveryRequired)
            } else {
                Ok(())
            }),
            Err(Error::RecoveryRequired)
        );
        publish_config(&config, true).unwrap();
        assert_eq!(
            fs::read(&config.path).unwrap(),
            *config.publication_bytes(true).unwrap()
        );
        cleanup_config_restore(&config).unwrap();
        assert_eq!(config.before, Some(grok_before()));
        assert_eq!(config.after, Some(grok_before()));
    }
}

#[test]
fn desired_channel_cas_keeps_concurrent_user_save_at_both_claim_boundaries() {
    for stop in [
        ConfigRestorePoint::BeforeClaim,
        ConfigRestorePoint::AfterClaim,
        ConfigRestorePoint::BeforePublish,
    ] {
        let (_directory, root) = private_root();
        let config = selected_backup(&root, ConfigKind::Grok, Some(grok_before()), Channel::Alpha);
        fs::write(&config.path, grok_before()).unwrap();
        let result = publish_config_with_hook(&config, true, |point| {
            if point == stop {
                atomic_user_save(&config.path, b"# concurrent user save\n");
            }
            Ok(())
        });
        assert_eq!(result, Err(Error::RecoveryRequired));
        assert_eq!(fs::read(&config.path).unwrap(), b"# concurrent user save\n");
        assert_eq!(publish_config(&config, true), Err(Error::RecoveryRequired));
    }
}

#[test]
fn failed_channel_transaction_restores_before_and_keeps_desired_separate() {
    let (_directory, root) = private_root();
    let mut config = selected_backup(&root, ConfigKind::Grok, Some(grok_before()), Channel::Alpha);
    config.after = Some(
        String::from_utf8(grok_after())
            .unwrap()
            .replace("channel = \"stable\"", "channel = \"alpha\"")
            .into_bytes(),
    );
    fs::write(&config.path, config.after.as_ref().unwrap()).unwrap();
    assert!(config_delta_allowed(
        &config,
        config.after.as_deref(),
        "alpha"
    ));
    publish_config(&config, false).unwrap();
    assert_eq!(fs::read(&config.path).unwrap(), grok_before());
    assert_ne!(config.publication_bytes(true), config.before.as_ref());
}

#[test]
fn new_claude_settings_directory_is_created_only_when_publishing_explicit_channel() {
    let (_directory, root) = private_root();
    let parent = root.join("missing-settings");
    let config = selected_backup(&parent, ConfigKind::Claude, None, Channel::Stable);
    assert!(!parent.exists());
    publish_config(&config, true).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&config.path).unwrap()).unwrap(),
        json!({"autoUpdatesChannel":"stable"})
    );
    cleanup_config_restore(&config).unwrap();
}

#[cfg(unix)]
fn codex_marker_fixture(root: &Path, version: &str) -> (PathBuf, PathBuf, String) {
    use std::os::unix::fs::symlink;
    let standalone = root.join("standalone");
    let release = format!("{version}-aarch64-apple-darwin");
    let directory = standalone.join("releases").join(&release);
    fs::create_dir_all(directory.join("bin")).unwrap();
    let executable = directory.join("bin/codex");
    fs::write(&executable, b"synthetic codex").unwrap();
    symlink(&directory, standalone.join("current")).unwrap();
    (standalone.join("auto-update-version"), executable, release)
}

#[cfg(unix)]
#[test]
fn codex_valid_stable_marker_follows_new_release_but_invalid_marker_does_not_enable() {
    let (_directory, root) = private_root();
    let (path, executable, old) = codex_marker_fixture(&root, "0.147.0");
    let target = b"0.155.1-aarch64-apple-darwin";
    assert_eq!(
        codex_marker_config(
            &path,
            Some(old.as_bytes()),
            &executable,
            "0.147.0",
            "0.155.1"
        )
        .unwrap(),
        Some(target.to_vec())
    );
    for before in [
        None,
        Some(b"disabled".as_slice()),
        Some(b"wrong-platform".as_slice()),
    ] {
        assert_eq!(
            codex_marker_config(&path, before, &executable, "0.147.0", "0.155.1").unwrap(),
            before.map(<[u8]>::to_vec)
        );
    }
    assert_eq!(
        codex_marker_config(&path, Some(target), &executable, "0.147.0", "0.155.1").unwrap(),
        None
    );
}

#[cfg(unix)]
#[test]
fn codex_alpha_round_trip_never_silently_reenables_native_updater() {
    let (_directory, root) = private_root();
    let (path, executable, stable) = codex_marker_fixture(&root.join("stable"), "0.155.1");
    assert_eq!(
        codex_marker_config(
            &path,
            Some(stable.as_bytes()),
            &executable,
            "0.155.1",
            "0.156.0-alpha.7"
        )
        .unwrap(),
        None
    );
    let (path, executable, alpha) = codex_marker_fixture(&root.join("alpha"), "0.156.0-alpha.7");
    for before in [None, Some(alpha.as_bytes()), Some(stable.as_bytes())] {
        let desired =
            codex_marker_config(&path, before, &executable, "0.156.0-alpha.7", "0.155.1").unwrap();
        assert_ne!(desired.as_deref(), Some(stable.as_bytes()));
    }
}

#[cfg(unix)]
#[test]
fn codex_marker_requires_exact_current_directory_entry_and_version() {
    let (_directory, root) = private_root();
    let (path, executable, current) = codex_marker_fixture(&root, "0.155.1");
    assert_eq!(
        codex_marker_config(
            &path,
            Some(current.as_bytes()),
            &root.join("other"),
            "0.155.1",
            "0.155.2"
        ),
        Err(Error::SourceChanged)
    );
    assert_eq!(
        codex_marker_config(
            &path,
            Some(current.as_bytes()),
            &executable,
            "0.147.0",
            "0.155.2"
        ),
        Err(Error::SourceChanged)
    );
    let newline = format!("{current}\n");
    assert_eq!(
        codex_marker_config(
            &path,
            Some(newline.as_bytes()),
            &executable,
            "0.155.1",
            "0.155.2"
        )
        .unwrap(),
        Some(newline.into_bytes())
    );
}

#[test]
fn retry_suppression_is_bound_to_channel_target_and_desired_configuration() {
    let (_directory, root) = private_root();
    let entry = root.join("grok");
    let alpha = selected_backup(&root, ConfigKind::Grok, Some(grok_before()), Channel::Alpha);
    let alpha_intent = update_intent(Channel::Alpha, "1.0.38", Some(&alpha)).unwrap();
    let stable_intent = update_intent(Channel::Stable, "1.0.38", Some(&alpha)).unwrap();
    save_failure_with_intent(
        &root,
        CLIAgent::Grok,
        &entry,
        "1.0.38",
        Some(alpha_intent.clone()),
    )
    .unwrap();
    assert_eq!(
        previous_failure_for_intent(&root, CLIAgent::Grok, &entry, &alpha_intent, false).unwrap(),
        Some("1.0.38".to_owned())
    );
    assert_eq!(
        previous_failure_for_intent(&root, CLIAgent::Grok, &entry, &stable_intent, false).unwrap(),
        None
    );
    let changed = selected_backup(
        &root,
        ConfigKind::Grok,
        Some(b"# changed preference\n[cli]\nchannel='stable'\n".to_vec()),
        Channel::Alpha,
    );
    let changed_intent = update_intent(Channel::Alpha, "1.0.38", Some(&changed)).unwrap();
    assert_ne!(alpha_intent, changed_intent);
    assert_eq!(
        previous_failure_for_intent(&root, CLIAgent::Grok, &entry, &changed_intent, false).unwrap(),
        None
    );
    save_failure_with_intent(&root, CLIAgent::Grok, &entry, "1.0.38", None).unwrap();
    assert!(
        previous_failure_for_intent(&root, CLIAgent::Grok, &entry, &alpha_intent, true)
            .unwrap()
            .is_some()
    );
    assert!(
        previous_failure_for_intent(&root, CLIAgent::Grok, &entry, &alpha_intent, false)
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn confirmed_failed_update_never_publishes_explicit_channel() {
    let (_directory, root) = private_root();
    let mut record = journal(&root, "prepared");
    fs::write(&record.entry, b"old unchanged binary").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.generation = Some(Uuid::new_v4());
    bind_journal(&mut record);
    record.config = Some(selected_backup(
        &root,
        ConfigKind::Grok,
        Some(grok_before()),
        Channel::Alpha,
    ));
    record.config.as_mut().unwrap().restore_stage = None;
    let config_path = record.config.as_ref().unwrap().path.clone();
    fs::write(&config_path, grok_before()).unwrap();
    record.channel = "alpha".to_owned();
    record.intent = Some(
        update_intent(
            Channel::Alpha,
            &record.target_version,
            record.config.as_ref(),
        )
        .unwrap(),
    );
    let journal_path = root.join("grok.json");
    save_journal(&journal_path, &record).unwrap();
    record_bound_not_started(&root, &record);
    let old_version = record.old_version.clone();
    reconcile_confirmed_update(
        &journal_path,
        &mut record,
        CLIAgent::Grok,
        &root,
        &old_version,
    )
    .unwrap();
    assert!(!record.publish_desired);
    assert_eq!(fs::read(config_path).unwrap(), grok_before());
    assert!(
        previous_failure_for_intent(
            &root,
            CLIAgent::Grok,
            &record.entry,
            record.intent.as_deref().unwrap(),
            false
        )
        .unwrap()
        .is_some()
    );
    assert!(!journal_path.exists());
}

#[test]
fn legacy_journal_without_channel_publication_fields_keeps_rollback_semantics() {
    let (_directory, root) = private_root();
    let mut value = serde_json::to_value(journal(&root, "command_returned")).unwrap();
    let object = value.as_object_mut().unwrap();
    for key in ["publish_desired", "command_failed", "intent"] {
        object.remove(key);
    }
    let config = object.get_mut("config").unwrap().as_object_mut().unwrap();
    config.remove("desired");
    config.remove("before_mode");
    let restored: Journal = serde_json::from_value(value).unwrap();
    assert!(!restored.publish_desired);
    assert_eq!(
        restored.config.as_ref().unwrap().publication_bytes(true),
        Some(&grok_before())
    );
    assert!(restored.intent.is_none());
}

#[cfg(unix)]
#[test]
fn codex_target_marker_is_published_only_after_actual_current_directory_matches() {
    use std::os::unix::fs::symlink;
    let (_directory, root) = private_root();
    let (marker, executable, old) = codex_marker_fixture(&root, "0.147.0");
    let mut record = journal(&root, "command_returned");
    record.target_version = "0.155.1".to_owned();
    record.entry = marker.parent().unwrap().join("current/bin/codex");
    record.publish_desired = true;
    record.config = Some(ConfigBackup {
        kind: ConfigKind::CodexUpdateMarker,
        desired: Some(ConfigDesired {
            bytes: codex_marker_config(
                &marker,
                Some(old.as_bytes()),
                &executable,
                "0.147.0",
                "0.155.1",
            )
            .unwrap(),
        }),
        before: Some(old.into_bytes()),
        after: None,
        path: marker.clone(),
        restore_stage: None,
        before_mode: None,
    });
    assert_eq!(
        validate_codex_publication(&record),
        Err(Error::RecoveryRequired)
    );
    let target = marker
        .parent()
        .unwrap()
        .join("releases/0.155.1-aarch64-apple-darwin");
    fs::create_dir_all(target.join("bin")).unwrap();
    fs::write(target.join("bin/codex"), b"target binary").unwrap();
    let current = marker.parent().unwrap().join("current");
    fs::remove_file(&current).unwrap();
    symlink(&target, current).unwrap();
    validate_codex_publication(&record).unwrap();
}

#[cfg(unix)]
#[test]
fn publication_preserves_original_mode_when_native_updater_removed_the_file() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_directory, root) = private_root();
    let mut config = selected_backup(
        &root,
        ConfigKind::Claude,
        Some(br#"{"autoUpdatesChannel":"latest"}"#.to_vec()),
        Channel::Stable,
    );
    config.after = None;
    config.before_mode = Some(0o640);
    publish_config(&config, true).unwrap();
    assert_eq!(
        fs::metadata(&config.path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::read(&config.path).unwrap(),
        *config.publication_bytes(true).unwrap()
    );
    config.before_mode = Some(0o4640);
    assert_eq!(publish_config(&config, true), Err(Error::RecoveryRequired));
}

fn claude_scope_fixture(root: &Path) -> Journal {
    let user = root.join("user");
    fs::create_dir(&user).unwrap();
    let config = selected_backup(
        &user,
        ConfigKind::Claude,
        Some(br#"{"autoUpdatesChannel":"latest","minimumVersion":"2.1.200","permissions":{"deny":["Read(secret)"]},"env":{"DISABLE_UPDATES":"false","SYNTHETIC":"kept"}}"#.to_vec()),
        Channel::Stable,
    );
    fs::write(&config.path, config.before.as_ref().unwrap()).unwrap();
    let metadata = user.join(".claude.json");
    fs::write(&metadata, br#"{"installMethod":"native","autoUpdates":true,"oauthAccount":{"synthetic":"private"},"projects":{"synthetic":"private"}}"#).unwrap();
    fs::write(user.join(".credentials.json"), b"synthetic-do-not-copy").unwrap();
    fs::write(user.join("remote-settings.json"), br#"{"minimumVersion":"2.1.200","requiredMaximumVersion":"2.1.300","env":{"DISABLE_UPDATES":"false"}}"#).unwrap();
    fs::write(
        user.join("remote-settings.json.signature.json"),
        b"synthetic-policy-receipt",
    )
    .unwrap();
    let generation = Uuid::new_v4();
    let scope = snapshot_claude_scope(&config, generation, metadata, "2.1.267").unwrap();
    let mut record = journal(root, "prepared");
    record.agent = "claude".to_owned();
    record.entry = root.join("claude");
    record.launch = Some(JournalLaunch {
        program: record.entry.clone(),
        args: vec![OsString::from("update")],
        cwd: root.to_owned(),
    });
    fs::write(&record.entry, b"synthetic unchanged native executable").unwrap();
    record.old_stamp = Some(stamp(&record.entry).unwrap());
    record.old_version = "2.1.278".to_owned();
    record.target_version = "2.1.267".to_owned();
    record.config = Some(config);
    record.generation = Some(generation);
    bind_journal(&mut record);
    record.claude_update = Some(scope);
    record
}

#[test]
fn claude_shadow_preserves_settings_and_policy_bytes_without_copying_credentials() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap();
    let config = record.config.as_ref().unwrap();
    let shadow = prepare_claude_scope(&root, scope, config).unwrap();

    assert_eq!(
        fs::read(shadow.join("settings.json")).unwrap(),
        *config.before.as_ref().unwrap()
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(shadow.join(".claude.json")).unwrap()).unwrap(),
        json!({"installMethod":"native","autoUpdates":true})
    );
    assert_eq!(
        fs::read(shadow.join("remote-settings.json")).unwrap(),
        fs::read(root.join("user/remote-settings.json")).unwrap()
    );
    assert_eq!(
        fs::read(shadow.join("remote-settings.json.signature.json")).unwrap(),
        b"synthetic-policy-receipt"
    );
    assert!(!shadow.join(".credentials.json").exists());
    fs::write(
        shadow.join(".claude.json"),
        br#"{"firstStartTime":"synthetic","machineID":"synthetic"}"#,
    )
    .unwrap();
    verify_claude_originals(scope).unwrap();
    cleanup_claude_scope(&root, scope).unwrap();
    cleanup_claude_scope(&root, scope).unwrap();
    assert!(!shadow.exists());
    assert!(shadow.parent().unwrap().join("owner").is_file());
    assert_eq!(
        fs::read(root.join("user/.credentials.json")).unwrap(),
        b"synthetic-do-not-copy"
    );
}

#[test]
fn claude_cached_constraints_only_restrict_the_selected_update_target() {
    assert!(claude_cached_update_constraints(None, "2.1.278").is_ok());
    assert!(
        claude_cached_update_constraints(
            Some(br#"{"theme":"dark","permissions":{"deny":["Read(secret)"]}}"#),
            "2.1.278"
        )
        .is_ok()
    );
    assert!(
        claude_cached_update_constraints(
            Some(br#"{"minimumVersion":"2.1.267","requiredMaximumVersion":"2.1.278"}"#),
            "2.1.278"
        )
        .is_ok()
    );
    assert_eq!(
        claude_cached_update_constraints(Some(br#"{"minimumVersion":"2.1.278"}"#), "2.1.267"),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        claude_cached_update_constraints(
            Some(br#"{"requiredMaximumVersion":"2.1.267"}"#),
            "2.1.278"
        ),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        claude_cached_update_constraints(
            Some(br#"{"requiredMinimumVersion":"2.1.278"}"#),
            "2.1.267"
        ),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        claude_cached_update_constraints(Some(br#"{"env":{"DISABLE_UPDATES":"1"}}"#), "2.1.278"),
        Err(Error::PermissionDenied)
    );
    assert!(
        claude_cached_update_constraints(Some(br#"{"minimumVersion":false}"#), "2.1.278").is_err()
    );
    assert_eq!(
        claude_cached_update_constraints(Some(br#"{"policyHelpers":{"update":{}}}"#), "2.1.278"),
        Err(Error::UnsupportedSource)
    );
}

#[test]
fn claude_shadow_rejects_configuration_redirects_without_removing_policy() {
    for key in [
        "CLAUDE_CONFIG_DIR",
        "claude_config_dir",
        "HOME",
        "USERPROFILE",
    ] {
        let bytes = serde_json::to_vec(&json!({"env":{key:"synthetic-other-directory"}})).unwrap();
        assert_eq!(
            claude_cached_update_constraints(Some(&bytes), "2.1.278"),
            Err(Error::UnsupportedSource)
        );
    }
    for value in [json!(true), json!("true"), json!("unexpected"), json!(null)] {
        let bytes = serde_json::to_vec(&json!({"env":{"DISABLE_UPDATES":value}})).unwrap();
        assert_eq!(
            claude_cached_update_constraints(Some(&bytes), "2.1.278"),
            Err(Error::PermissionDenied)
        );
    }
}

#[test]
fn claude_shadow_rejects_another_configuration_or_journal_generation() {
    let (_directory, root) = private_root();
    let mut record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap().clone();
    let mut other_config = record.config.as_ref().unwrap().clone();
    other_config.path = root.join("other/settings.json");
    assert_eq!(
        prepare_claude_scope(&root, &scope, &other_config),
        Err(Error::RecoveryRequired)
    );
    other_config.path = scope.settings_path.clone();
    other_config.kind = ConfigKind::Grok;
    assert_eq!(
        prepare_claude_scope(&root, &scope, &other_config),
        Err(Error::RecoveryRequired)
    );
    record.generation = Some(Uuid::new_v4());
    assert_eq!(
        validate_claude_journal(&record),
        Err(Error::RecoveryRequired)
    );
    let mut other_scope = scope.clone();
    other_scope.originals[0].path = root.join("unrelated/.claude.json");
    assert_eq!(
        validate_claude_scope(&other_scope),
        Err(Error::RecoveryRequired)
    );
}

#[test]
fn claude_policy_same_bytes_replacement_is_a_different_source_identity() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap();
    let path = root.join("user/remote-settings.json");
    let replacement = root.join("user/replacement.json");
    fs::write(&replacement, fs::read(&path).unwrap()).unwrap();
    fs::remove_file(&path).unwrap();
    fs::rename(replacement, &path).unwrap();
    assert_eq!(verify_claude_originals(scope), Err(Error::SourceChanged));
}

#[test]
fn claude_changed_policy_or_metadata_is_not_copied_from_a_stale_snapshot() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap();
    fs::write(
        root.join("user/remote-settings.json"),
        br#"{"env":{"DISABLE_UPDATES":"1"}}"#,
    )
    .unwrap();
    assert_eq!(
        prepare_claude_scope(&root, scope, record.config.as_ref().unwrap()),
        Err(Error::SourceChanged)
    );
    assert!(!claude_scope_directory(&root, scope).unwrap().exists());
}

#[test]
fn claude_scope_never_adopts_an_existing_directory_or_different_owner() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap();
    let directory = claude_scope_directory(&root, scope).unwrap();
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("owner"), b"other owner").unwrap();
    fs::write(directory.join("keep"), b"other data").unwrap();
    assert!(prepare_claude_scope(&root, scope, record.config.as_ref().unwrap()).is_err());
    assert!(cleanup_claude_scope(&root, scope).is_err());
    assert_eq!(fs::read(directory.join("keep")).unwrap(), b"other data");
}

#[test]
fn claude_scope_rejects_authentication_payloads_and_escaped_policy_names() {
    let (_directory, root) = private_root();
    let mut record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_mut().unwrap();
    scope.metadata =
        Some(br#"{"installMethod":"native","oauthAccount":{"synthetic":"private"}}"#.to_vec());
    assert!(validate_claude_scope(scope).is_err());
    scope.metadata = Some(br#"{"installMethod":"native"}"#.to_vec());
    assert_eq!(verify_claude_originals(scope), Err(Error::SourceChanged));
    scope.originals[1].name = "../.credentials.json".to_owned();
    assert!(validate_claude_scope(scope).is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn claude_unstarted_checkpoint_is_permanently_closed_before_recovery_cleanup() {
    for phase in ["claude_config_preparing", "prepared"] {
        let (_directory, root) = private_root();
        let mut record = claude_scope_fixture(&root);
        record.phase = phase.to_owned();
        let shadow = prepare_claude_scope(
            &root,
            record.claude_update.as_ref().unwrap(),
            record.config.as_ref().unwrap(),
        )
        .unwrap();
        let path = root.join("claude.json");
        save_journal(&path, &record).unwrap();
        assert_eq!(
            preflight_recovery(CLIAgent::Claude, &record.entry, &root),
            Ok(true)
        );
        assert!(!shadow.exists());
        let receipt = managed_process::confirmed_exit(&root, record.generation.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(receipt.containment, "not_started");
        assert!(
            managed_process::record_not_started(
                &root,
                record.generation.unwrap(),
                &record.entry,
                &[],
                &root
            )
            .is_err()
        );
        reconcile_journal(CLIAgent::Claude, &record.entry, &record.old_version, &root).unwrap();
        assert_eq!(
            fs::read(record.config.as_ref().unwrap().path.clone()).unwrap(),
            *record.config.as_ref().unwrap().before.as_ref().unwrap()
        );
        assert!(!path.exists());
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn claude_unconfirmed_process_does_not_release_its_private_configuration() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let shadow = prepare_claude_scope(
        &root,
        record.claude_update.as_ref().unwrap(),
        record.config.as_ref().unwrap(),
    )
    .unwrap();
    fs::create_dir_all(
        root.join("cli-agent-processes")
            .join(record.generation.unwrap().to_string()),
    )
    .unwrap();
    save_journal(&root.join("claude.json"), &record).unwrap();
    assert_eq!(
        preflight_recovery(CLIAgent::Claude, &record.entry, &root),
        Err(Error::RecoveryRequired)
    );
    assert!(shadow.join("settings.json").exists());
}

#[cfg(feature = "local_fs")]
#[test]
fn claude_cas_conflict_cleans_the_finished_process_shadow_and_preserves_user_changes() {
    let (_directory, root) = private_root();
    let mut record = claude_scope_fixture(&root);
    let shadow = prepare_claude_scope(
        &root,
        record.claude_update.as_ref().unwrap(),
        record.config.as_ref().unwrap(),
    )
    .unwrap();
    let path = root.join("claude.json");
    save_journal(&path, &record).unwrap();
    record_bound_not_started(&root, &record);
    let config_path = record.config.as_ref().unwrap().path.clone();
    fs::write(&config_path, b"new user configuration").unwrap();
    let old = record.old_version.clone();
    assert_eq!(
        reconcile_confirmed_update(&path, &mut record, CLIAgent::Claude, &root, &old),
        Err(Error::RecoveryRequired)
    );
    assert!(!shadow.exists());
    assert_eq!(fs::read(config_path).unwrap(), b"new user configuration");
    assert!(path.exists());
}

#[cfg(feature = "local_fs")]
#[test]
fn claude_metadata_conflict_cleans_only_the_shadow_after_confirmed_exit() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let shadow = prepare_claude_scope(
        &root,
        record.claude_update.as_ref().unwrap(),
        record.config.as_ref().unwrap(),
    )
    .unwrap();
    record_bound_not_started(&root, &record);
    fs::write(root.join("user/.claude.json"), b"new user metadata").unwrap();
    assert_eq!(
        finish_claude_scope(&root, &record),
        Err(Error::RecoveryRequired)
    );
    assert!(!shadow.exists());
    assert_eq!(
        fs::read(root.join("user/.claude.json")).unwrap(),
        b"new user metadata"
    );
}

#[cfg(unix)]
#[test]
fn claude_private_copy_has_private_modes_and_does_not_follow_payload_links_on_cleanup() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap();
    let shadow = prepare_claude_scope(&root, scope, record.config.as_ref().unwrap()).unwrap();
    assert_eq!(
        fs::metadata(&shadow).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(shadow.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(shadow.join("remote-settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let external = root.join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("keep"), b"external data").unwrap();
    std::os::unix::fs::symlink(&external, shadow.join("linked")).unwrap();
    cleanup_claude_scope(&root, scope).unwrap();
    assert_eq!(fs::read(external.join("keep")).unwrap(), b"external data");
}

#[cfg(unix)]
#[test]
fn claude_shadow_rejects_replaced_owner_links_and_keeps_foreign_files() {
    for hard_link in [false, true] {
        let (_directory, root) = private_root();
        let record = claude_scope_fixture(&root);
        let scope = record.claude_update.as_ref().unwrap();
        let shadow = prepare_claude_scope(&root, scope, record.config.as_ref().unwrap()).unwrap();
        let owner = shadow.parent().unwrap().join("owner");
        let foreign = root.join("foreign-owner");
        fs::rename(&owner, &foreign).unwrap();
        if hard_link {
            fs::hard_link(&foreign, &owner).unwrap();
        } else {
            std::os::unix::fs::symlink(&foreign, &owner).unwrap();
        }
        assert!(cleanup_claude_scope(&root, scope).is_err());
        assert_eq!(fs::read(&foreign).unwrap(), claude_scope_owner(scope));
        assert!(shadow.join("settings.json").exists());
    }
}

#[cfg(unix)]
#[test]
fn claude_original_policy_rejects_links_before_reading_contents() {
    let (_directory, root) = private_root();
    let record = claude_scope_fixture(&root);
    let scope = record.claude_update.as_ref().unwrap();
    let policy = root.join("user/remote-settings.json");
    let target = root.join("unrelated");
    fs::write(&target, b"unrelated file").unwrap();
    fs::remove_file(&policy).unwrap();
    std::os::unix::fs::symlink(&target, &policy).unwrap();
    assert!(verify_claude_originals(scope).is_err());
    assert_eq!(fs::read(target).unwrap(), b"unrelated file");
}
