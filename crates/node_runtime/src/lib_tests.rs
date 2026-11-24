use std::fs;
use std::io::{Cursor, Write as _};

use futures::executor::block_on;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::extract_zip;

fn zip_files(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, contents) in files {
        writer.start_file(name, options).unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn extract_zip_preserves_nested_file_contents() {
    let tmp = tempdir().unwrap();
    let destination = tmp.path().join("node");
    let archive = zip_files(&[
        ("node.exe", b"\0node\xff"),
        ("node_modules/npm/说明.txt", "安装说明".as_bytes()),
    ]);

    block_on(extract_zip(
        &archive,
        &destination,
        None::<fn(&str) -> bool>,
    ))
    .unwrap();

    assert_eq!(
        fs::read(destination.join("node.exe")).unwrap(),
        b"\0node\xff"
    );
    assert_eq!(
        fs::read_to_string(destination.join("node_modules/npm/说明.txt")).unwrap(),
        "安装说明"
    );
}

#[test]
fn extract_zip_only_writes_matching_files() {
    let tmp = tempdir().unwrap();
    let destination = tmp.path().join("node");
    let archive = zip_files(&[("node.exe", b"node"), ("README.md", b"readme")]);

    block_on(extract_zip(
        &archive,
        &destination,
        Some(|name: &str| name == "node.exe"),
    ))
    .unwrap();

    assert_eq!(fs::read(destination.join("node.exe")).unwrap(), b"node");
    assert!(!destination.join("README.md").exists());
}

#[test]
fn extract_zip_skips_paths_outside_destination() {
    let tmp = tempdir().unwrap();
    let destination = tmp.path().join("node");
    let archive = zip_files(&[("../outside.txt", b"escape"), ("node.exe", b"node")]);

    block_on(extract_zip(
        &archive,
        &destination,
        None::<fn(&str) -> bool>,
    ))
    .unwrap();

    assert!(!tmp.path().join("outside.txt").exists());
    assert_eq!(fs::read(destination.join("node.exe")).unwrap(), b"node");
}

#[test]
fn extract_zip_rejects_unmatched_filter_without_writing() {
    let tmp = tempdir().unwrap();
    let destination = tmp.path().join("node");
    let archive = zip_files(&[("README.md", b"readme")]);

    let result = block_on(extract_zip(
        &archive,
        &destination,
        Some(|name: &str| name == "node.exe"),
    ));

    assert!(result.is_err());
    assert!(!destination.exists());
}

#[test]
fn extract_zip_rejects_conflicting_header_names_without_writing() {
    let tmp = tempdir().unwrap();
    let destination = tmp.path().join("node");
    let mut archive = zip_files(&[("node.exe", b"node")]);
    // 本地文件头的固定部分为 30 字节；只改其名称，保留中央目录中的原名称。
    assert_eq!(&archive[..4], b"PK\x03\x04");
    assert_eq!(&archive[30..38], b"node.exe");
    archive[30] = b'x';

    let result = block_on(extract_zip(
        &archive,
        &destination,
        None::<fn(&str) -> bool>,
    ));

    assert!(result.is_err());
    assert!(!destination.exists());
}
