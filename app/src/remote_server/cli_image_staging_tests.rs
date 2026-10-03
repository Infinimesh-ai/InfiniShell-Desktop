use super::*;

fn private_parent() -> tempfile::TempDir {
    // 正例显式满足生产私有父目录合同，不依赖测试进程的 umask。
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn scope(connection_id: Uuid) -> RemoteImageScope {
    RemoteImageScope {
        terminal_epoch: Uuid::from_u128(3),
        host_id: HostId::new("daemon-a".into()),
        connection_id,
        terminal_session_id: SessionId::from(7),
        cli_session_id: "native-session-a".into(),
        input_generation: Uuid::new_v4(),
        submission_id: Uuid::new_v4(),
    }
}

fn spec(bytes: &[u8]) -> RemoteImageSpec {
    RemoteImageSpec {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

fn staging(parent: &Path, max_bytes: u64) -> RemoteImageStaging {
    RemoteImageStaging::new(HostId::new("daemon-a".into()), parent, max_bytes).unwrap()
}

#[test]
fn retained_references_keep_the_global_budget_across_daemon_restarts() {
    let parent = private_parent();
    let prior = scope(Uuid::new_v4());
    let transfer = Uuid::new_v4();
    let key = Uuid::new_v4();
    let mut old = staging(parent.path(), 16);
    old.activate_scope(prior.clone()).unwrap();
    let bytes = b"\x89PNG\r\n\x1a\n";
    old.begin(&prior, transfer, spec(bytes)).unwrap();
    old.write_chunk(&prior, transfer, 0, bytes).unwrap();
    let (image, _) = old.publish(&prior, transfer, key).unwrap();
    let owner = fs::metadata(&image).unwrap().uid();
    drop(old);

    let mut second = prior.clone();
    second.host_id = HostId::new("daemon-b".into());
    second.connection_id = Uuid::new_v4();
    let mut reopened = RemoteImageStaging::new(second.host_id.clone(), parent.path(), 16).unwrap();
    assert_eq!(reopened.reserved_bytes, 8);
    assert_eq!(fs::read(&image).unwrap(), bytes);
    assert_eq!(fs::metadata(&image).unwrap().uid(), owner);
    let record_path = parent
        .path()
        .join("cli-image-references-v1")
        .join(format!("reference-{transfer}.json"));
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(record_path).unwrap()).unwrap();
    assert_eq!(record["host"], prior.host_id.as_str());
    assert!(
        reopened
            .recover_reference(&second, transfer, key, false)
            .is_err()
    );
    reopened.activate_scope(second.clone()).unwrap();
    let second_transfer = Uuid::new_v4();
    let second_key = Uuid::new_v4();
    reopened.begin(&second, second_transfer, spec(bytes)).unwrap();
    reopened
        .write_chunk(&second, second_transfer, 0, bytes)
        .unwrap();
    let (second_image, _) = reopened
        .publish(&second, second_transfer, second_key)
        .unwrap();
    assert_eq!(reopened.reserved_bytes, 16);
    assert!(reopened.begin(&second, Uuid::new_v4(), spec(b"x")).is_err());
    drop(reopened);

    let mut current = second.clone();
    current.host_id = HostId::new("daemon-c".into());
    current.connection_id = Uuid::new_v4();
    let mut twice = RemoteImageStaging::new(current.host_id.clone(), parent.path(), 16).unwrap();
    twice.activate_scope(current.clone()).unwrap();
    assert_eq!(twice.reserved_bytes, 16);
    assert!(twice.begin(&current, Uuid::new_v4(), spec(b"x")).is_err());
    assert!(
        twice
            .recover_reference(&current, transfer, key, true)
            .is_err()
    );
    assert!(
        twice
            .recover_reference(&prior, transfer, Uuid::new_v4(), true)
            .is_err()
    );
    assert_eq!(twice.reserved_bytes, 16);
    assert_eq!(fs::read(&image).unwrap(), bytes);
    assert_eq!(fs::read(&second_image).unwrap(), bytes);
    twice
        .recover_reference(&prior, transfer, key, true)
        .unwrap();
    assert_eq!(twice.reserved_bytes, 8);
    assert!(!image.exists());
    twice.recover_reference(&prior, transfer, key, true).unwrap();
    assert_eq!(twice.reserved_bytes, 8);
    assert_eq!(fs::read(&second_image).unwrap(), bytes);
    twice.begin(&current, Uuid::new_v4(), spec(bytes)).unwrap();
    assert_eq!(twice.reserved_bytes, 16);
    assert!(twice.begin(&current, Uuid::new_v4(), spec(b"x")).is_err());
}

#[test]
fn damaged_other_host_reference_still_blocks_daemon_initialization() {
    let parent = private_parent();
    let prior = scope(Uuid::new_v4());
    let transfer = Uuid::new_v4();
    let mut old = staging(parent.path(), 8);
    old.activate_scope(prior.clone()).unwrap();
    let bytes = b"\x89PNG\r\n\x1a\n";
    old.begin(&prior, transfer, spec(bytes)).unwrap();
    old.write_chunk(&prior, transfer, 0, bytes).unwrap();
    let (image, _) = old.publish(&prior, transfer, Uuid::new_v4()).unwrap();
    drop(old);
    let path = parent
        .path()
        .join("cli-image-references-v1")
        .join(format!("reference-{transfer}.json"));
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (field, replacement) in [
        ("transfer_id", serde_json::json!(Uuid::new_v4())),
        ("directory", serde_json::json!("../unrelated")),
    ] {
        let mut damaged = original.clone();
        damaged[field] = replacement;
        fs::write(&path, serde_json::to_vec(&damaged).unwrap()).unwrap();
        assert!(RemoteImageStaging::new(HostId::new("daemon-b".into()), parent.path(), 8).is_err());
        assert_eq!(fs::read(&image).unwrap(), bytes);
    }
}

#[test]
fn exact_bytes_and_duplicate_chunks_produce_the_same_verified_receipt() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    let bytes = b"\x00\xff\x89PNG\r\n\x1a\n";
    let expected = spec(bytes);

    assert_eq!(
        store.begin(&binding, transfer, expected.clone()).unwrap(),
        0
    );
    assert_eq!(
        store
            .write_chunk(&binding, transfer, 0, &bytes[..4])
            .unwrap(),
        4
    );
    assert_eq!(
        store.begin(&binding, transfer, expected.clone()).unwrap(),
        4
    );
    assert_eq!(
        store
            .write_chunk(&binding, transfer, 0, &bytes[..4])
            .unwrap(),
        4
    );
    assert_eq!(
        store
            .write_chunk(&binding, transfer, 4, &bytes[4..])
            .unwrap(),
        10
    );
    let receipt = store.verify(&binding, transfer).unwrap();
    assert_eq!(receipt.spec, expected);
    assert_eq!(store.verify(&binding, transfer).unwrap(), receipt);
    assert_eq!(
        fs::read(store.entries[&transfer].file.path()).unwrap(),
        bytes
    );
}

#[test]
fn changed_duplicate_and_out_of_order_chunk_do_not_replace_original_bytes() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abcdef")).unwrap();
    store.write_chunk(&binding, transfer, 0, b"abc").unwrap();

    assert!(store.write_chunk(&binding, transfer, 0, b"xyz").is_err());
    assert!(store.write_chunk(&binding, transfer, 2, b"cde").is_err());
    assert!(store.write_chunk(&binding, transfer, 5, b"f").is_err());
    assert!(
        store
            .write_chunk(&binding, transfer, u64::MAX, b"x")
            .is_err()
    );
    assert!(store.write_chunk(&binding, transfer, 3, b"").is_err());
    assert!(store.verify(&binding, transfer).is_err());
    assert_eq!(
        fs::read(store.entries[&transfer].file.path()).unwrap(),
        b"abc"
    );
    store.write_chunk(&binding, transfer, 3, b"def").unwrap();
    store.verify(&binding, transfer).unwrap();
}

#[test]
fn matching_length_with_wrong_digest_does_not_verify() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();
    store.write_chunk(&binding, transfer, 0, b"xyz").unwrap();

    assert!(store.verify(&binding, transfer).is_err());
    assert_eq!(
        fs::read(store.entries[&transfer].file.path()).unwrap(),
        b"xyz"
    );
}

#[test]
fn changed_native_session_or_generation_rejects_old_callbacks_and_removes_unpublished_bytes() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let original = scope(Uuid::new_v4());
    store.activate_scope(original.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&original, transfer, spec(b"abc")).unwrap();
    let path = store.entries[&transfer].file.path().to_owned();
    let mut replacement = original.clone();
    replacement.cli_session_id = "native-session-b".into();
    replacement.input_generation = Uuid::new_v4();
    store.activate_scope(replacement.clone()).unwrap();

    assert!(!path.exists());
    assert!(store.write_chunk(&original, transfer, 0, b"abc").is_err());
    assert!(store.begin(&original, transfer, spec(b"abc")).is_err());
    assert!(store.cancel(&original, transfer).is_err());
    store
        .begin(&replacement, Uuid::new_v4(), spec(b"abc"))
        .unwrap();
}

#[test]
fn sibling_connection_with_equal_terminal_number_cannot_access_or_cancel_other_bytes() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let first = scope(Uuid::new_v4());
    let second = scope(Uuid::new_v4());
    store.activate_scope(first.clone()).unwrap();
    store.activate_scope(second.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&first, transfer, spec(b"abc")).unwrap();
    store.write_chunk(&first, transfer, 0, b"abc").unwrap();

    assert!(store.begin(&second, transfer, spec(b"abc")).is_err());
    assert!(store.verify(&second, transfer).is_err());
    assert!(store.cancel(&second, transfer).is_err());
    store.disconnect(second.connection_id).unwrap();
    store.verify(&first, transfer).unwrap();
}

#[test]
fn disconnected_connection_cannot_reuse_old_transfer_on_new_connection() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let original = scope(Uuid::new_v4());
    store.activate_scope(original.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&original, transfer, spec(b"abc")).unwrap();
    let path = store.entries[&transfer].file.path().to_owned();
    store.disconnect(original.connection_id).unwrap();
    let mut resumed = original.clone();
    resumed.connection_id = Uuid::new_v4();
    store.activate_scope(resumed.clone()).unwrap();

    assert!(!path.exists());
    assert!(store.verify(&original, transfer).is_err());
    assert!(store.verify(&resumed, transfer).is_err());
    assert!(store.begin(&original, transfer, spec(b"abc")).is_err());
}

#[test]
fn canceled_transfer_is_not_resurrected_by_late_begin() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();
    store.cancel(&binding, transfer).unwrap();
    store.cancel(&binding, transfer).unwrap();

    assert!(store.begin(&binding, transfer, spec(b"abc")).is_err());
    assert!(store.write_chunk(&binding, transfer, 0, b"abc").is_err());
}

#[test]
fn total_reservation_is_enforced_before_writing_bytes() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 5);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let first = Uuid::new_v4();
    store.begin(&binding, first, spec(b"abc")).unwrap();
    let second = Uuid::new_v4();

    assert!(store.begin(&binding, second, spec(b"def")).is_err());
    store.cancel(&binding, first).unwrap();
    store.begin(&binding, second, spec(b"def")).unwrap();
}

#[test]
fn staging_uses_private_directory_and_files_without_user_paths() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();
    let path = store.entries[&transfer].file.path();

    assert_eq!(
        fs::metadata(store.root.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(path.parent(), Some(store.root.path()));
}

#[test]
fn non_private_or_symlink_parent_is_rejected_without_changing_permissions() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let public = parent.path().join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    let alias = parent.path().join("alias");
    symlink(parent.path(), &alias).unwrap();

    assert!(RemoteImageStaging::new(HostId::new("daemon-a".into()), &public, 32).is_err());
    assert!(RemoteImageStaging::new(HostId::new("daemon-a".into()), &alias, 32).is_err());
    assert_eq!(
        fs::metadata(&public).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn unknown_host_cannot_register_an_image_scope() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let mut binding = scope(Uuid::new_v4());
    binding.host_id = HostId::new("daemon-b".into());

    assert!(store.activate_scope(binding.clone()).is_err());
    assert!(store.begin(&binding, Uuid::new_v4(), spec(b"abc")).is_err());
}

#[test]
fn cleanup_refuses_to_delete_a_replacement_symlink() {
    use std::os::unix::fs::symlink;

    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();
    let path = store.entries[&transfer].file.path().to_owned();
    let unrelated = parent.path().join("unrelated");
    fs::write(&unrelated, b"preserve").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&unrelated, &path).unwrap();

    assert!(store.cancel(&binding, transfer).is_err());
    assert!(store.disconnect(binding.connection_id).is_err());
    // 只有析构之后仍保留，才能证明临时文件库没有绕过显式身份检查。
    drop(store);
    assert_eq!(fs::read(&unrelated).unwrap(), b"preserve");
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn new_submission_in_same_cli_generation_rejects_previous_batch_callbacks() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let original = scope(Uuid::new_v4());
    store.activate_scope(original.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&original, transfer, spec(b"abc")).unwrap();
    let mut next = original.clone();
    next.submission_id = Uuid::new_v4();
    store.activate_scope(next.clone()).unwrap();

    assert!(store.write_chunk(&original, transfer, 0, b"abc").is_err());
    store.begin(&next, Uuid::new_v4(), spec(b"def")).unwrap();
}

#[test]
fn changed_duplicate_spec_cannot_overwrite_the_existing_transfer() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();

    assert!(store.begin(&binding, transfer, spec(b"xyz")).is_err());
    store.write_chunk(&binding, transfer, 0, b"abc").unwrap();
    store.verify(&binding, transfer).unwrap();
}

#[test]
fn cancel_before_begin_keeps_a_tombstone_for_late_upload() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();

    store.cancel(&binding, transfer).unwrap();
    store.cancel(&binding, transfer).unwrap();

    assert!(store.begin(&binding, transfer, spec(b"abc")).is_err());
    assert!(store.write_chunk(&binding, transfer, 0, b"abc").is_err());
    assert_eq!(store.reserved_bytes, 0);
}

#[test]
fn successful_drop_removes_only_owned_staging_files() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    store.begin(&binding, Uuid::new_v4(), spec(b"abc")).unwrap();
    let root = store.root.path().to_owned();

    drop(store);

    assert!(!root.exists());
}

#[test]
fn drop_preserves_a_replacement_regular_file_after_cleanup_rejection() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();
    let path = store.entries[&transfer].file.path().to_owned();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"foreign replacement").unwrap();

    assert!(store.cancel(&binding, transfer).is_err());
    drop(store);

    assert_eq!(fs::read(&path).unwrap(), b"foreign replacement");
}

#[test]
fn drop_preserves_an_unregistered_file_in_the_staging_directory() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let transfer = Uuid::new_v4();
    store.begin(&binding, transfer, spec(b"abc")).unwrap();
    let owned_path = store.entries[&transfer].file.path().to_owned();
    let foreign_path = store.root.path().join("foreign");
    fs::write(&foreign_path, b"preserve").unwrap();

    drop(store);

    assert!(!owned_path.exists());
    assert_eq!(fs::read(&foreign_path).unwrap(), b"preserve");
}

#[test]
fn replaced_directory_rejects_new_uploads_and_survives_drop() {
    let parent = private_parent();
    let mut store = staging(parent.path(), 32);
    let binding = scope(Uuid::new_v4());
    store.activate_scope(binding.clone()).unwrap();
    let root = store.root.path().to_owned();
    let moved = parent.path().join("moved-original");
    fs::rename(&root, &moved).unwrap();
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let foreign_path = root.join("foreign");
    fs::write(&foreign_path, b"preserve").unwrap();

    assert!(store.begin(&binding, Uuid::new_v4(), spec(b"abc")).is_err());
    drop(store);

    assert_eq!(fs::read(&foreign_path).unwrap(), b"preserve");
    assert!(moved.is_dir());
}
