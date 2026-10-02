use std::io::Cursor;
use std::path::Path;

use super::super::CLIAgent;
use super::super::npm_release::{NpmArchiveFile, NpmRelease, VerifiedNpmArchive};
use super::*;

const WRAPPER_285: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.285-wrapper.json");
const PLATFORM_285: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.285-platform.json");
const WRAPPER_287: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.287-wrapper.json");
const PLATFORM_287: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.287-platform.json");

fn archives_285() -> (VerifiedNpmArchive, VerifiedNpmArchive) {
    let parsed = NpmRelease::from_metadata(
        CLIAgent::Claude,
        V285,
        "darwin-arm64",
        WRAPPER_285,
        PLATFORM_285,
    )
    .unwrap();
    let wrapper = parsed
        .wrapper
        .verify_archive(&mut Cursor::new(include_bytes!(
            "fixtures/claude-current-release/2.1.285-wrapper.tgz"
        )))
        .unwrap();
    (wrapper, native_inventory(V285))
}

// 平台大二进制的逐字节校验由真实发行审计承担；这里仅验证库存拒绝条件，不执行原件。
fn native_inventory(version: &str) -> VerifiedNpmArchive {
    let artifact = release(version, "darwin-arm64").unwrap().platform;
    VerifiedNpmArchive {
        files: artifact
            .files
            .into_iter()
            .map(|(path, file)| {
                (
                    path,
                    NpmArchiveFile {
                        length: file.length,
                        sha256: hex::decode(file.sha256).unwrap().try_into().unwrap(),
                        executable: file.mode & 0o111 != 0,
                        mode: file.mode,
                    },
                )
            })
            .collect(),
        compressed_sha256: hex::decode(artifact.sha256).unwrap().try_into().unwrap(),
    }
}

#[test]
fn official_285_wrapper_bytes_match_the_fixed_complete_inventory() {
    let (wrapper, native) = archives_285();
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Ok(())
    );
    assert_eq!(wrapper.files.len(), 7);
    assert_eq!(wrapper.files[Path::new("bin/claude.exe")].mode, 0o644);
    assert_eq!(wrapper.files[Path::new("sdk-tools.d.ts")].length, 169648);
}

#[test]
fn official_287_wrapper_bytes_match_the_fixed_complete_inventory() {
    let parsed = NpmRelease::from_metadata(
        CLIAgent::Claude,
        V287,
        "darwin-arm64",
        WRAPPER_287,
        PLATFORM_287,
    )
    .unwrap();
    let wrapper = parsed
        .wrapper
        .verify_archive(&mut Cursor::new(include_bytes!(
            "fixtures/claude-current-release/2.1.287-wrapper.tgz"
        )))
        .unwrap();
    assert_eq!(
        verify_archives(V287, "darwin-arm64", &wrapper, &native_inventory(V287)),
        Ok(())
    );
    assert_eq!(wrapper.files.len(), 7);
    assert_eq!(
        hex::encode(wrapper.compressed_sha256),
        "b829179017c73acbc4084b764ca3a8cd9d0f3554ec2107519a272c8ded975efb"
    );
}

#[test]
fn fixed_release_rejects_other_platforms_and_unknown_versions() {
    assert_eq!(
        supports(V285, "darwin-x64"),
        Err(Error::UnsupportedPlatform)
    );
    assert_eq!(supports(V287, "linux-x64"), Err(Error::UnsupportedPlatform));
    assert_eq!(
        supports(V287, "win32-arm64"),
        Err(Error::UnsupportedPlatform)
    );
    assert_eq!(
        supports("2.1.288", "darwin-arm64"),
        Err(Error::InvalidRelease)
    );
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Claude,
            V285,
            "linux-x64",
            WRAPPER_285,
            PLATFORM_285
        )
        .is_err()
    );
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Claude,
            "2.1.288",
            "darwin-arm64",
            WRAPPER_287,
            PLATFORM_287
        )
        .is_err()
    );
}

#[test]
fn fixed_metadata_rejects_changed_sri_and_mixed_versions() {
    let mut metadata: Value = serde_json::from_slice(WRAPPER_285).unwrap();
    metadata["dist"]["integrity"] = serde_json::json!("sha512-unreviewed");
    assert_eq!(
        verify_metadata(
            V285,
            "darwin-arm64",
            &serde_json::to_vec(&metadata).unwrap(),
            PLATFORM_285
        ),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        verify_metadata(V285, "darwin-arm64", WRAPPER_285, PLATFORM_287),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn fixed_metadata_rejects_archive_redirect_and_changed_member_count() {
    let mut metadata: Value = serde_json::from_slice(WRAPPER_287).unwrap();
    metadata["dist"]["tarball"] = serde_json::json!("https://example.invalid/claude.tgz");
    assert_eq!(
        verify_metadata(
            V287,
            "darwin-arm64",
            &serde_json::to_vec(&metadata).unwrap(),
            PLATFORM_287
        ),
        Err(Error::InvalidRelease)
    );
    let mut metadata: Value = serde_json::from_slice(PLATFORM_287).unwrap();
    metadata["dist"]["fileCount"] = serde_json::json!(5);
    assert_eq!(
        verify_metadata(
            V287,
            "darwin-arm64",
            WRAPPER_287,
            &serde_json::to_vec(&metadata).unwrap()
        ),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn fixed_release_keeps_exact_dependencies_and_postinstall_contract() {
    let mut wrapper: Value = serde_json::from_slice(WRAPPER_285).unwrap();
    wrapper["optionalDependencies"]["@anthropic-ai/claude-code-darwin-arm64"] =
        serde_json::json!("^2.1.285");
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Claude,
            V285,
            "darwin-arm64",
            &serde_json::to_vec(&wrapper).unwrap(),
            PLATFORM_285
        )
        .is_err()
    );
    let mut wrapper: Value = serde_json::from_slice(WRAPPER_285).unwrap();
    wrapper["scripts"]["postinstall"] = serde_json::json!("node unknown.cjs");
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Claude,
            V285,
            "darwin-arm64",
            &serde_json::to_vec(&wrapper).unwrap(),
            PLATFORM_285
        )
        .is_err()
    );
}

#[test]
fn fixed_inventory_rejects_changed_archive_digest() {
    let (mut wrapper, native) = archives_285();
    wrapper.compressed_sha256[0] ^= 1;
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn fixed_inventory_rejects_writable_mode_even_when_executability_is_unchanged() {
    let (mut wrapper, native) = archives_285();
    wrapper
        .files
        .get_mut(Path::new("install.cjs"))
        .unwrap()
        .mode = 0o664;
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn fixed_inventory_rejects_changed_native_bytes_and_permissions() {
    let (wrapper, mut native) = archives_285();
    native.files.get_mut(Path::new("claude")).unwrap().sha256[0] ^= 1;
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
    let mut native = native_inventory(V285);
    native.files.get_mut(Path::new("claude")).unwrap().mode = 0o775;
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn fixed_inventory_rejects_missing_and_extra_members() {
    let (mut wrapper, native) = archives_285();
    let file = wrapper.files.remove(Path::new("sdk-tools.d.ts")).unwrap();
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
    wrapper.files.insert(PathBuf::from("unexpected.d.ts"), file);
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
    wrapper.files.insert(
        PathBuf::from("sdk-tools.d.ts"),
        NpmArchiveFile {
            length: 1,
            sha256: [0; 32],
            executable: false,
            mode: 0o644,
        },
    );
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn fixed_inventory_rejects_cross_version_platform_substitution() {
    let (wrapper, _) = archives_285();
    assert_eq!(
        verify_archives(V285, "darwin-arm64", &wrapper, &native_inventory(V287)),
        Err(Error::InvalidRelease)
    );
}
