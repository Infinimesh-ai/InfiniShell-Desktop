use std::fs;

use super::*;

fn pending_file(path: std::path::PathBuf) -> PendingFile {
    PendingFile {
        file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
        file_path: path,
        mime_type: "application/octet-stream".to_owned(),
    }
}

#[test]
fn preserves_chinese_spaces_quotes_and_binary_path_without_inlining_bytes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("中文 'quoted' file.bin");
    fs::write(&path, b"\0private attachment bytes\xff").unwrap();
    let expected = serde_json::to_string(&vec![path.to_str().unwrap()]).unwrap();

    let prompt = async_io::block_on(prepare_file_attachments(
        "查看附件".to_owned(),
        vec![pending_file(path)],
    ))
    .unwrap();

    assert!(prompt.starts_with("查看附件\n\n"));
    assert!(prompt.contains(&expected));
    assert!(!prompt.contains("private attachment bytes"));
}

#[test]
fn accepts_file_only_input_and_preserves_attachment_order() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.txt");
    let second = root.path().join("第二份.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    let expected =
        serde_json::to_string(&vec![first.to_str().unwrap(), second.to_str().unwrap()]).unwrap();

    let prompt = async_io::block_on(prepare_file_attachments(
        String::new(),
        vec![pending_file(first), pending_file(second)],
    ))
    .unwrap();

    assert!(prompt.contains(&expected));
}

#[test]
fn missing_file_rejects_the_entire_batch() {
    let root = tempfile::tempdir().unwrap();
    let valid = root.path().join("valid.txt");
    fs::write(&valid, "valid").unwrap();

    let result = async_io::block_on(prepare_file_attachments(
        "原草稿".to_owned(),
        vec![
            pending_file(valid),
            pending_file(root.path().join("missing.txt")),
        ],
    ));

    assert!(result.is_err());
}

#[test]
fn directories_and_relative_paths_are_not_file_attachments() {
    let root = tempfile::tempdir().unwrap();

    assert!(
        async_io::block_on(prepare_file_attachments(
            "草稿".to_owned(),
            vec![pending_file(root.path().to_owned())],
        ))
        .is_err()
    );
    assert!(
        async_io::block_on(prepare_file_attachments(
            "草稿".to_owned(),
            vec![pending_file("relative.txt".into())],
        ))
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn unreadable_file_is_rejected_without_changing_its_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("unreadable.txt");
    fs::write(&path, "protected").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let result = async_io::block_on(prepare_file_attachments(
        "保留草稿".to_owned(),
        vec![pending_file(path.clone())],
    ));

    assert!(result.is_err());
    assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0);
}
