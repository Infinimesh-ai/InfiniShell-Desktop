use super::*;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::io::Cursor;

#[test]
fn unknown_consumer_versions_never_receive_a_package_contract() {
    assert!(supports("1.0.45").is_err());
    assert!(supports("1.0.47").is_err());
    assert!(native("1.0.47").is_err());
}

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(windows, target_arch = "x86_64")
))]
#[test]
fn old_target_and_recovery_inventory_remain_separate_from_current_release() {
    assert_eq!(supports_transition("1.0.40", "1.0.41"), Ok(()));
    let old = installed_files("1.0.41").unwrap();
    assert_eq!(
        old[Path::new("bin/postinstall.js")].sha256,
        "9debd63b905270da66d3e0f29d2a16fee1b52c9eddd8b270e05e855e53b2f69d"
    );
    assert!(
        contract(VERSION)
            .unwrap()
            .packages
            .contains_key("@xai-official/grok@1.0.40")
    );
    assert!(
        !contract(VERSION)
            .unwrap()
            .packages
            .contains_key("@xai-official/grok@1.0.46")
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn macos_current_update_does_not_add_an_unreviewed_downgrade() {
    assert_eq!(supports_transition("1.0.40", "1.0.46"), Ok(()));
    assert_eq!(supports_transition("1.0.41", "1.0.46"), Ok(()));
    assert_eq!(supports_transition("1.0.46", "1.0.46"), Ok(()));
    assert_eq!(
        supports_transition("1.0.46", "1.0.41"),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        supports_transition("1.0.45", "1.0.46"),
        Err(Error::InvalidRelease)
    );
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn macos_current_inventory_cannot_enable_other_platforms() {
    assert!(supports("1.0.46").is_err());
    assert!(native("1.0.46").is_err());
    assert!(installed_files("1.0.46").is_err());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn wrapper() -> (Package, VerifiedNpmArchive) {
    let package = contract(MACOS_CURRENT_VERSION)
        .unwrap()
        .packages
        .remove("@xai-official/grok@1.0.46")
        .unwrap();
    let artifact = NpmArtifact::from_pinned_metadata(
        include_bytes!("fixtures/grok-current-release/1.0.46-wrapper.json"),
        package.manifest.clone(),
        &package.integrity,
    )
    .unwrap();
    let archive = artifact
        .verify_archive(&mut Cursor::new(include_bytes!(
            "fixtures/grok-current-release/1.0.46-wrapper.tgz"
        )))
        .unwrap();
    (package, archive)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn official_current_wrapper_binds_full_archive_and_member_modes() {
    let (package, archive) = wrapper();
    assert_eq!(package.verify_archive(&archive), Ok(()));
    assert_eq!(archive.files.len(), 5);
    assert_eq!(archive.files[Path::new("bin/grok")].mode, 0o755);
    assert_eq!(archive.files[Path::new("bin/postinstall.js")].mode, 0o644);
    let installed = installed_files(MACOS_CURRENT_VERSION).unwrap();
    assert!(!installed.contains_key(Path::new("bin/grok")));
    assert_eq!(installed[Path::new("bin/grok-native")].length, 150374256);
    assert_eq!(
        installed[Path::new("bin/grok-native")].sha256,
        "e8daa302364c9c3b6a5546d511cfbd1ab5e5d407a9b04282f660665ea405f9f3"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn official_toml_tar_permissions_are_checked_as_source_metadata() {
    let package = contract(MACOS_CURRENT_VERSION)
        .unwrap()
        .packages
        .remove("@iarna/toml@3.0.0")
        .unwrap();
    let artifact = NpmArtifact::from_pinned_metadata(
        include_bytes!("fixtures/grok-current-release/1.0.46-toml.json"),
        package.manifest.clone(),
        &package.integrity,
    )
    .unwrap();
    let archive = artifact
        .verify_archive(&mut Cursor::new(include_bytes!(
            "fixtures/grok-current-release/1.0.46-toml.tgz"
        )))
        .unwrap();
    assert_eq!(package.verify_archive(&archive), Ok(()));
    assert_eq!(archive.files.len(), 20);
    assert_eq!(archive.files[Path::new("toml.js")].mode, 0o777);
    // 本测试只证明原 tar 完整性；事务仍沿用 apply_grok_permissions，不使用原始 mode 写盘。
    assert_eq!(package.files[Path::new("toml.js")].mode, Some(0o777));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_archive_rejects_changed_content_mode_members_and_compressed_digest() {
    let (package, mut archive) = wrapper();
    archive.files.get_mut(Path::new("bin/grok")).unwrap().mode = 0o775;
    assert_eq!(package.verify_archive(&archive), Err(Error::InvalidRelease));
    let (package, mut archive) = wrapper();
    archive.files.get_mut(Path::new("bin/grok")).unwrap().sha256[0] ^= 1;
    assert_eq!(package.verify_archive(&archive), Err(Error::InvalidRelease));
    let (package, mut archive) = wrapper();
    archive.files.remove(Path::new("README.md"));
    assert_eq!(package.verify_archive(&archive), Err(Error::InvalidRelease));
    let (package, mut archive) = wrapper();
    archive.compressed_sha256[0] ^= 1;
    assert_eq!(package.verify_archive(&archive), Err(Error::InvalidRelease));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_metadata_rejects_changed_dependency_or_platform_version() {
    let package = contract(MACOS_CURRENT_VERSION)
        .unwrap()
        .packages
        .remove("@xai-official/grok@1.0.46")
        .unwrap();
    let mut metadata: Value = serde_json::from_slice(include_bytes!(
        "fixtures/grok-current-release/1.0.46-wrapper.json"
    ))
    .unwrap();
    metadata["dependencies"]["@iarna/toml"] = serde_json::json!("^4.0.0");
    assert!(
        NpmArtifact::from_pinned_metadata(
            &serde_json::to_vec(&metadata).unwrap(),
            package.manifest,
            &package.integrity
        )
        .is_err()
    );
    let package = contract(MACOS_CURRENT_VERSION)
        .unwrap()
        .packages
        .remove("@xai-official/grok-darwin-arm64@1.0.46")
        .unwrap();
    assert!(
        NpmArtifact::from_pinned_metadata(
            include_bytes!("fixtures/grok-current-release/1.0.46-platform.json"),
            package.manifest.clone(),
            &package.integrity
        )
        .is_ok()
    );
    let mut metadata: Value = serde_json::from_slice(include_bytes!(
        "fixtures/grok-current-release/1.0.46-platform.json"
    ))
    .unwrap();
    metadata["version"] = serde_json::json!("1.0.41");
    assert!(
        NpmArtifact::from_pinned_metadata(
            &serde_json::to_vec(&metadata).unwrap(),
            package.manifest,
            &package.integrity
        )
        .is_err()
    );
}
