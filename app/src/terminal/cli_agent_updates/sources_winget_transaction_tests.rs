use super::super::{Channel, ConfigDesired, ConfigKind, ConfigRestorePoint};
use super::*;

fn fixture(root: &Path) -> Journal {
    let id = Uuid::new_v4();
    let target = root.join("claude.exe");
    let candidate = root.join(format!(".infinishell-winget-{id}.exe"));
    fs::write(&target, b"original image").unwrap();
    fs::write(&candidate, b"candidate image").unwrap();
    let original = image(&target).unwrap();
    let prepared = image(&candidate).unwrap();
    Journal {
        schema: 1,
        id,
        owner: Owner {
            product: "Anthropic.ClaudeCode_Microsoft.Winget.Source_8wekyb3d8bbwe".into(),
            root: root.to_owned(),
            target,
            public: root.join("public-claude.exe"),
            version: "2.1.286".into(),
            sha256: hex::encode_upper(original.sha256),
        },
        target_version: "2.1.280".into(),
        target_sha256: hex::encode_upper(prepared.sha256),
        candidate: candidate.clone(),
        backup: root.join(format!(".infinishell-winget-old-{id}.exe")),
        original,
        prepared: prepared.clone(),
        probe: CandidateReceipt {
            generation: Uuid::new_v4(),
            binding_digest: "a".repeat(64),
            program: candidate.canonicalize().unwrap(),
            version: "2.1.280".into(),
            sha256: prepared.sha256,
        },
        phase: Phase::Prepared,
        intent: "legacy-latest".into(),
        downgrade: None,
        config: None,
        claude_policy: None,
    }
}

fn prepared_from(journal: &Journal) -> PreparedCandidate {
    PreparedCandidate {
        id: journal.id,
        owner: journal.owner.clone(),
        program: journal.probe.program.clone(),
        version: journal.target_version.clone(),
        sha256: journal.prepared.sha256,
        manifest_sha256: if journal.target_version == contract::TO {
            contract::manifest_sha256(contract::TO).unwrap()
        } else {
            [0; 32]
        },
        original: journal.original.clone(),
        image: Some(journal.prepared.clone()),
        probe: None,
        intent: journal.intent.clone(),
        downgrade: journal.downgrade,
        config: journal.config.clone(),
        claude_policy: journal.claude_policy.clone(),
    }
}

fn config(root: &Path) -> ConfigBackup {
    let before = br#"{"autoUpdatesChannel":"latest","userSetting":true}"#.to_vec();
    let path = root.join("settings.json");
    fs::write(&path, &before).unwrap();
    ConfigBackup {
        kind: ConfigKind::Claude,
        path,
        before: Some(before.clone()),
        after: Some(before.clone()),
        desired: Some(ConfigDesired {
            bytes: super::super::selected_channel_config(
                ConfigKind::Claude,
                Some(&before),
                Some(Channel::Stable),
            )
            .unwrap(),
        }),
        before_mode: None,
        restore_stage: None,
    }
}

// 此处只构造已审清单的持久字段；小型文件不会经过生产固定 PE 准入。
fn fixed_journal(root: &Path) -> Journal {
    let mut journal = fixture(root);
    let (length, digest) = contract::native(contract::FROM).unwrap();
    journal.original.length = length;
    journal.original.sha256 = digest;
    journal.owner.sha256 = hex::encode_upper(digest);
    let (length, digest) = contract::native(contract::TO).unwrap();
    journal.prepared.length = length;
    journal.prepared.sha256 = digest;
    journal.target_version = contract::TO.into();
    journal.target_sha256 = hex::encode_upper(digest);
    journal.probe.version = contract::TO.into();
    journal.probe.sha256 = digest;
    journal.downgrade = Some(claude_downgrade::Intent::ClaudeWingetStable21286To21285);
    let config = config(root);
    journal.claude_policy = Some(
        super::super::snapshot_claude_scope(
            &config,
            Uuid::new_v4(),
            root.join(".config.json"),
            contract::TO,
        )
        .unwrap(),
    );
    journal.config = Some(config);
    journal.probe.binding_digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&prepared_from(&journal)).unwrap(),)
    );
    journal
}

#[test]
fn legacy_records_round_trip_without_new_fields_or_new_plan_restrictions() {
    let root = tempfile::tempdir().unwrap();
    let journal = fixture(root.path());
    // 旧恢复不因新版已禁止发起的 286→280 计划而改变语义。
    validate_journal(&journal).unwrap();
    let bytes = serde_json::to_vec(&journal).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for field in ["downgrade", "config", "claude_policy"] {
        assert!(value.get(field).is_none());
    }
    let restored: Journal = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    validate_journal(&restored).unwrap();
    let candidate = prepared_from(&journal);
    let bytes = serde_json::to_vec(&candidate).unwrap();
    let restored: PreparedCandidate = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    validate_prepared(&restored).unwrap();
}

#[test]
#[cfg(target_arch = "x86_64")]
fn new_records_bind_source_target_intent_config_and_same_probe() {
    let root = tempfile::tempdir().unwrap();
    let journal = fixed_journal(root.path());
    validate_journal(&journal).unwrap();
    let encoded = serde_json::to_value(&journal).unwrap();
    for field in ["downgrade", "config", "claude_policy"] {
        let mut changed = encoded.clone();
        changed.as_object_mut().unwrap().remove(field);
        assert!(validate_journal(&serde_json::from_value(changed).unwrap()).is_err());
    }
    for (field, value) in [
        ("version", serde_json::json!("2.1.280")),
        ("binding_digest", serde_json::json!("b".repeat(64))),
    ] {
        let mut changed = encoded.clone();
        changed["probe"][field] = value;
        assert!(validate_journal(&serde_json::from_value(changed).unwrap()).is_err());
    }
    let mut changed: Journal = serde_json::from_value(encoded.clone()).unwrap();
    changed.prepared.length -= 1;
    assert!(validate_journal(&changed).is_err());
    let mut changed: Journal = serde_json::from_value(encoded.clone()).unwrap();
    changed.original.sha256[0] ^= 1;
    assert!(validate_journal(&changed).is_err());
    let mut changed: Journal = serde_json::from_value(encoded).unwrap();
    changed
        .config
        .as_mut()
        .unwrap()
        .desired
        .as_mut()
        .unwrap()
        .bytes = Some(br#"{"autoUpdatesChannel":"stable","userSetting":false}"#.to_vec());
    assert!(validate_journal(&changed).is_err());
    // 准入本身仍拒绝小字节映像，未增加测试专用 PE 绕过。
    assert!(validate_target(contract::TO, &image(&journal.candidate).unwrap()).is_err());
}

#[test]
fn rollback_file_boundaries_use_actual_file_identity_and_reject_replacement() {
    let root = tempfile::tempdir().unwrap();
    let journal = fixture(root.path());
    verify_rollback_images(&journal).unwrap();
    let original = hold_file(&journal.owner.target, &journal.original).unwrap();
    rename_owned(&original, &journal.backup).unwrap();
    drop(original);
    verify_rollback_images(&journal).unwrap();
    let candidate = hold_file(&journal.candidate, &journal.prepared).unwrap();
    rename_owned(&candidate, &journal.owner.target).unwrap();
    drop(candidate);
    verify_rollback_images(&journal).unwrap();
    replace(
        &journal.owner.target,
        &journal.backup,
        &journal.candidate,
        &journal.prepared,
        &journal.original,
    )
    .unwrap();
    verify_rollback_images(&journal).unwrap();
    fs::write(&journal.candidate, b"external candidate").unwrap();
    assert!(verify_rollback_images(&journal).is_err());
    assert_eq!(image(&journal.owner.target).unwrap(), journal.original);
}

#[test]
fn rename_owned_preserves_images_at_all_utf16_alignments_with_chinese_and_spaces() {
    let root = tempfile::tempdir().unwrap();
    for remainder in 0..4 {
        let directory = root.path().join(format!("中文 空格 {remainder}"));
        fs::create_dir(&directory).unwrap();
        let mut journal = fixture(&directory);
        let base_length = directory
            .join("备份 file.exe")
            .as_os_str()
            .encode_wide()
            .count();
        let padding = (remainder + 4 - base_length % 4) % 4;
        journal.backup = directory.join(format!("备份 file{}.exe", "a".repeat(padding)));
        // 按实际绝对路径的 UTF-16 码元数覆盖四种余数，不依赖临时目录的随机长度。
        assert_eq!(
            journal.backup.as_os_str().encode_wide().count() % 4,
            remainder
        );
        verify_rollback_images(&journal).unwrap();

        let original = hold_file(&journal.owner.target, &journal.original).unwrap();
        rename_owned(&original, &journal.backup).unwrap();
        drop(original);
        assert_eq!(optional_image(&journal.owner.target).unwrap(), None);
        assert_eq!(image(&journal.backup).unwrap(), journal.original);
        assert_eq!(image(&journal.candidate).unwrap(), journal.prepared);
        verify_rollback_images(&journal).unwrap();

        let candidate = hold_file(&journal.candidate, &journal.prepared).unwrap();
        rename_owned(&candidate, &journal.owner.target).unwrap();
        drop(candidate);
        assert_eq!(optional_image(&journal.candidate).unwrap(), None);
        assert_eq!(image(&journal.owner.target).unwrap(), journal.prepared);
        assert_eq!(image(&journal.backup).unwrap(), journal.original);
        verify_rollback_images(&journal).unwrap();

        replace(
            &journal.owner.target,
            &journal.backup,
            &journal.candidate,
            &journal.prepared,
            &journal.original,
        )
        .unwrap();
        assert_eq!(image(&journal.owner.target).unwrap(), journal.original);
        assert_eq!(optional_image(&journal.backup).unwrap(), None);
        assert_eq!(image(&journal.candidate).unwrap(), journal.prepared);
        verify_rollback_images(&journal).unwrap();
    }
}

#[test]
fn arp_partial_write_accepts_only_bound_fields_without_touching_registry() {
    let root = tempfile::tempdir().unwrap();
    let journal = fixture(root.path());
    let mut arp = journal.owner.clone();
    verify_arp_fields(&journal, &arp).unwrap();
    arp.sha256 = journal.target_sha256.clone();
    verify_arp_fields(&journal, &arp).unwrap();
    arp.version = journal.target_version.clone();
    verify_arp_fields(&journal, &arp).unwrap();
    arp.target = root.path().join("external.exe");
    assert!(verify_arp_fields(&journal, &arp).is_err());
    arp = journal.owner.clone();
    arp.version = "2.1.287".into();
    assert!(verify_arp_fields(&journal, &arp).is_err());
}

#[test]
#[cfg(target_arch = "x86_64")]
fn configuration_commit_recovers_each_real_file_publication_boundary() {
    for point in [
        ConfigRestorePoint::BeforeClaim,
        ConfigRestorePoint::AfterClaim,
        ConfigRestorePoint::BeforePublish,
        ConfigRestorePoint::AfterPublish,
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut journal = fixed_journal(root.path());
        verify_config(&journal.config, &journal.claude_policy, false).unwrap();
        super::super::plan_config_publish(journal.config.as_mut().unwrap(), true).unwrap();
        journal.phase = Phase::ConfigPublishing;
        let bytes = serde_json::to_vec(&journal).unwrap();
        let config = journal.config.as_ref().unwrap();
        assert!(
            super::super::publish_config_with_hook(config, true, |observed| {
                if observed == point {
                    Err(Error::RecoveryRequired)
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        let restored: Journal = serde_json::from_slice(&bytes).unwrap();
        validate_journal(&restored).unwrap();
        super::super::publish_config(restored.config.as_ref().unwrap(), true).unwrap();
        verify_config(&restored.config, &restored.claude_policy, true).unwrap();
        super::super::cleanup_config_restore(restored.config.as_ref().unwrap()).unwrap();
        assert_eq!(
            super::super::read_optional_config(&config.path)
                .unwrap()
                .as_ref(),
            config.publication_bytes(true)
        );
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn external_settings_or_policy_change_blocks_both_recovery_directions() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = fixed_journal(root.path());
    let config_path = journal.config.as_ref().unwrap().path.clone();
    fs::write(&config_path, br#"{"external":true}"#).unwrap();
    assert!(verify_config(&journal.config, &journal.claude_policy, false).is_err());
    super::super::plan_config_publish(journal.config.as_mut().unwrap(), true).unwrap();
    assert!(super::super::publish_config(journal.config.as_ref().unwrap(), true).is_err());
    assert_eq!(fs::read(&config_path).unwrap(), br#"{"external":true}"#);
    fs::write(
        &config_path,
        journal.config.as_ref().unwrap().before.as_ref().unwrap(),
    )
    .unwrap();
    fs::write(root.path().join("remote-settings.json"), b"{}").unwrap();
    assert!(verify_config(&journal.config, &journal.claude_policy, false).is_err());
    assert!(verify_config(&journal.config, &journal.claude_policy, true).is_err());
}

#[test]
fn completed_new_roll_forward_is_reported_as_success_without_changing_legacy_result() {
    assert_eq!(
        publication_result(
            Err(Error::PersistenceFailed),
            Some("2.1.285".into()),
            "2.1.285"
        ),
        Ok("2.1.285".into())
    );
    assert_eq!(
        publication_result(Err(Error::PersistenceFailed), None, "2.1.285"),
        Err(Error::PersistenceFailed)
    );
    assert_eq!(
        publication_result(
            Err(Error::PersistenceFailed),
            Some("2.1.280".into()),
            "2.1.280"
        ),
        Err(Error::PersistenceFailed)
    );
    assert_eq!(
        publication_result(
            Err(Error::PersistenceFailed),
            Some("2.1.280".into()),
            "2.1.285"
        ),
        Err(Error::RecoveryRequired)
    );
}

#[test]
fn successful_new_publication_clears_old_failure_record_and_allows_absence() {
    let root = tempfile::tempdir().unwrap();
    let path = super::super::failure_path(root.path(), CLIAgent::Claude);
    fs::write(&path, br#"{"target":"2.1.285","failure":"prior attempt"}"#).unwrap();
    clear_failure(root.path()).unwrap();
    assert!(!path.exists());
    clear_failure(root.path()).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(clear_failure(root.path()).is_err());
    assert!(path.is_dir());
}
