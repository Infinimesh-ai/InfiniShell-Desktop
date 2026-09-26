use super::*;

fn journal() -> (tempfile::TempDir, Journal) {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let lock = safe_options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(directory.path().join("lock"))
        .unwrap();
    let journal = Journal {
        directory: directory.path().to_owned(),
        lock,
    };
    (directory, journal)
}

fn scope(host: &str, native: &str) -> CliImageStagingScope {
    CliImageStagingScope {
        host_id: host.into(),
        terminal_session_id: 7,
        cli_session_id: native.into(),
        input_generation: Uuid::new_v4().to_string(),
        submission_id: Uuid::new_v4().to_string(),
        terminal_epoch: Uuid::new_v4().to_string(),
    }
}

fn persist_image(journal: &Journal, scope: &CliImageStagingScope) -> Intent {
    let intent = Intent::new(
        scope,
        Uuid::new_v4(),
        &CliImageStagingSpec {
            byte_len: 12,
            sha256: vec![7; 32],
        },
    )
    .unwrap();
    journal.persist_intent(&intent).unwrap();
    intent
}

#[test]
fn claude_event_recovery_selects_only_original_claimed_images_in_this_native_session() {
    let (_directory, journal) = journal();
    let original = scope("host-a", "claude-a");
    let image = persist_image(&journal, &original);
    journal.prepare_queue(&original, [1; 32], true).unwrap();

    let another_cli = scope("host-a", "claude-a");
    persist_image(&journal, &another_cli);
    journal.prepare_queue(&another_cli, [2; 32], false).unwrap();
    let another_session = scope("host-a", "claude-b");
    persist_image(&journal, &another_session);
    journal
        .prepare_queue(&another_session, [3; 32], true)
        .unwrap();
    let another_host = scope("host-b", "claude-a");
    persist_image(&journal, &another_host);
    journal.prepare_queue(&another_host, [4; 32], true).unwrap();
    let uploading = scope("host-a", "claude-a");
    persist_image(&journal, &uploading);
    let _lease = journal
        .lease(Uuid::parse_str(&uploading.submission_id).unwrap())
        .unwrap();

    // 编辑器代次已改变，恢复请求仍只引用旧图片意图，不新建或替换原提交。
    let current = scope("host-a", "claude-a");
    let work = journal.recovery_work(&current, true).unwrap();
    assert_eq!(work.len(), 1);
    assert_eq!(work[0].0.submission_id, image.submission_id);
    assert_eq!(work[0].0.transfer_id, image.transfer_id);
    assert_eq!(work[0].0.recovery_key, image.recovery_key);
    assert!(!work[0].1);
    assert!(work[0].2.is_none());
    assert!(
        journal
            .read::<bool>(&journal.path("release", image.submission_id))
            .unwrap()
            .is_none()
    );
}

#[test]
fn delayed_claude_consumption_releases_only_after_exact_terminal_receipt() {
    let (_directory, journal) = journal();
    let original = scope("host-a", "claude-a");
    let image = persist_image(&journal, &original);
    let queue = journal.prepare_queue(&original, [1; 32], true).unwrap();
    let mut result = CliImageCodexQueueResult {
        submission_id: original.submission_id.clone(),
        status: "unknown".into(),
        subject_sha256: vec![1; 32],
        native_queue_id: String::new(),
    };
    assert!(!journal.record_queue_result(&queue, &result).unwrap());
    assert!(!journal.recovery_work(&original, true).unwrap()[0].1);
    result.status = "confirmed".into();
    assert!(journal.record_queue_result(&queue, &result).is_err());
    result.native_queue_id = "精确图片读取回执".into();
    result.subject_sha256 = vec![2; 32];
    assert!(journal.record_queue_result(&queue, &result).is_err());
    result.subject_sha256 = vec![1; 32];
    result.submission_id = Uuid::new_v4().to_string();
    assert!(journal.record_queue_result(&queue, &result).is_err());
    assert!(
        journal
            .read::<bool>(&journal.path("release", image.submission_id))
            .unwrap()
            .is_none()
    );
    result.submission_id = original.submission_id;
    assert!(journal.record_queue_result(&queue, &result).unwrap());
    let current = scope("host-a", "claude-a");
    let work = journal.recovery_work(&current, true).unwrap();
    assert_eq!(work.len(), 1);
    assert!(work[0].1);
    assert_eq!(work[0].0.transfer_id, image.transfer_id);
}

#[test]
fn connection_recovery_keeps_unclaimed_images_while_upload_lease_is_held() {
    let (_directory, journal) = journal();
    let original = scope("host-a", "claude-a");
    let image = persist_image(&journal, &original);
    let lease = journal.lease(image.submission_id).unwrap();
    assert!(journal.recovery_work(&original, false).unwrap().is_empty());
    drop(lease);
    let work = journal.recovery_work(&original, false).unwrap();
    assert_eq!(work.len(), 1);
    assert!(work[0].1);
    assert!(work[0].2.is_some());
}
