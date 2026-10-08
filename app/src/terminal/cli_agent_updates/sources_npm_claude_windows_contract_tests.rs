use std::io::Read as _;

use sha2::{Digest as _, Sha256};

use super::*;

const WRAPPER_285: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.285-wrapper.json");
const PLATFORM_285: &[u8] =
    include_bytes!("fixtures/claude-windows-current-release/2.1.285-platform.json");
const WRAPPER_287: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.287-wrapper.json");
const PLATFORM_287: &[u8] =
    include_bytes!("fixtures/claude-windows-current-release/2.1.287-platform.json");

#[test]
fn official_windows_metadata_rejects_mixed_version_platform_and_integrity() {
    verify_metadata("2.1.285", WRAPPER_285, PLATFORM_285).unwrap();
    verify_metadata("2.1.287", WRAPPER_287, PLATFORM_287).unwrap();
    assert!(verify_metadata("2.1.285", WRAPPER_287, PLATFORM_285).is_err());
    assert!(verify_metadata("2.1.285", WRAPPER_285, PLATFORM_287).is_err());
    assert!(verify_metadata("2.1.288", WRAPPER_285, PLATFORM_285).is_err());
    for (pointer, value) in [
        (
            "/name",
            Value::from("@anthropic-ai/claude-code-win32-arm64"),
        ),
        ("/dist/integrity", Value::from(PLATFORM_INTEGRITY)),
        ("/dist/unpackedSize", Value::from(1)),
        ("/dist/fileCount", Value::from(3)),
        (
            "/dist/tarball",
            Value::from("https://example.invalid/claude.tgz"),
        ),
    ] {
        let mut platform: Value = serde_json::from_slice(PLATFORM_285).unwrap();
        *platform.pointer_mut(pointer).unwrap() = value;
        assert!(
            verify_metadata(
                "2.1.285",
                WRAPPER_285,
                &serde_json::to_vec(&platform).unwrap()
            )
            .is_err()
        );
    }
}

#[test]
fn official_wrapper_archives_bind_all_members_modes_and_materialized_windows_native() {
    for (version, bytes) in [
        (
            "2.1.285",
            include_bytes!("fixtures/claude-current-release/2.1.285-wrapper.tgz").as_slice(),
        ),
        (
            "2.1.287",
            include_bytes!("fixtures/claude-current-release/2.1.287-wrapper.tgz").as_slice(),
        ),
    ] {
        let release = release(version).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            release.wrapper.sha256
        );
        let mut expected = release.wrapper.files;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
        for member in archive.entries().unwrap() {
            let mut member = member.unwrap();
            let path = member
                .path()
                .unwrap()
                .strip_prefix("package")
                .unwrap()
                .to_owned();
            let file = expected.remove(&path).unwrap();
            assert!(member.header().entry_type().is_file());
            assert_eq!(member.header().mode().unwrap(), file.mode);
            let mut contents = Vec::new();
            member.read_to_end(&mut contents).unwrap();
            assert_eq!(contents.len() as u64, file.length);
            assert_eq!(format!("{:x}", Sha256::digest(&contents)), file.sha256);
        }
        assert!(expected.is_empty());
        let archived = archive_files_for(version).unwrap();
        let installed = files_for(version).unwrap();
        assert_eq!(archived.len(), 11);
        assert_eq!(installed.len(), 11);
        assert_eq!(
            installed[&PathBuf::from(PUBLIC)],
            installed[&PathBuf::from(NATIVE)]
        );
        assert_ne!(
            archived[&PathBuf::from(PUBLIC)],
            installed[&PathBuf::from(PUBLIC)]
        );
        assert_eq!(archive_modes_for(version).unwrap().len(), 11);
    }
    assert_eq!(archive_files_for(VERSION).unwrap(), archive_files());
    assert_eq!(files_for(VERSION).unwrap(), files());
    assert!(files_for("2.1.288").is_none());
}
