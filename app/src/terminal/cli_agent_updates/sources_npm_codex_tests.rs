#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::io::Cursor;

use super::super::CLIAgent;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use super::super::npm_release::NpmArchiveFile;
use super::*;

const WRAPPER: &[u8] = include_bytes!("fixtures/codex-0160/wrapper.json");
const PLATFORM: &[u8] = include_bytes!("fixtures/codex-0160/platform.json");

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn official_archives() -> (NpmRelease, VerifiedNpmArchive, VerifiedNpmArchive) {
    let release = NpmRelease::from_metadata(
        CLIAgent::Codex,
        "0.160.0",
        "darwin-arm64",
        WRAPPER,
        PLATFORM,
    )
    .unwrap();
    let wrapper = release
        .wrapper
        .verify_archive(&mut Cursor::new(include_bytes!(
            "fixtures/codex-0160/wrapper.tgz"
        )))
        .unwrap();
    // 大平台包的原件已逐字节归档；本夹具直接保存实读库存，不执行二进制。
    let files: BTreeMap<PathBuf, (u64, String, u32)> =
        serde_json::from_slice(include_bytes!("fixtures/codex-0160/platform-files.json")).unwrap();
    let platform = VerifiedNpmArchive {
        files: files
            .into_iter()
            .map(|(path, (length, digest, mode))| {
                (
                    path,
                    NpmArchiveFile {
                        length,
                        sha256: digest_bytes(&digest).unwrap(),
                        executable: mode & 0o111 != 0,
                        mode,
                    },
                )
            })
            .collect(),
        compressed_sha256: digest_bytes(
            "fc789bcd655d903f92e1a23c8dc5315ba38f43b3586eafb7bd3b195970b57466",
        )
        .unwrap(),
    };
    (release, wrapper, platform)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn official_current_macos_wrapper_and_complete_package_are_accepted() {
    assert_eq!(verify_metadata("0.160.0", WRAPPER, PLATFORM), Ok(()));
    let (release, wrapper, platform) = official_archives();
    assert_eq!(release.verify_entries(&wrapper, &platform), Ok(()));
    assert_eq!(
        verify_archives("0.160.0", &release, &wrapper, &platform),
        Ok(())
    );
    assert_eq!(wrapper.files.len(), 3);
    assert_eq!(platform.files.len(), 44);
    assert_eq!(release.public_entry, Path::new("bin/codex.js"));
    assert!(!release.materialize_native_entry);
    assert_eq!(
        hex::encode(wrapper.compressed_sha256),
        "373517768e912eeb5054024ae9215e2c90a1420957b66fe134ef745a00948d4a"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_macos_release_rejects_missing_or_modified_package_members() {
    let (release, wrapper, mut platform) = official_archives();
    let name = Path::new("vendor/aarch64-apple-darwin/bin/codex-code-mode-host");
    let original = platform.files.remove(name).unwrap();
    assert_eq!(
        verify_archives("0.160.0", &release, &wrapper, &platform),
        Err(Error::InvalidRelease)
    );
    platform.files.insert(name.to_owned(), original);
    platform.files.get_mut(name).unwrap().sha256[0] ^= 1;
    assert_eq!(
        verify_archives("0.160.0", &release, &wrapper, &platform),
        Err(Error::InvalidRelease)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_macos_release_rejects_mode_and_alias_metadata_changes() {
    let (release, wrapper, mut platform) = official_archives();
    let name = Path::new("vendor/aarch64-apple-darwin/bin/codex");
    platform.files.get_mut(name).unwrap().mode = 0o777;
    assert_eq!(
        verify_archives("0.160.0", &release, &wrapper, &platform),
        Err(Error::InvalidRelease)
    );
    platform.files.get_mut(name).unwrap().mode = 0o755;
    platform
        .files
        .get_mut(Path::new("package.json"))
        .unwrap()
        .sha256[0] ^= 1;
    assert_eq!(
        verify_archives("0.160.0", &release, &wrapper, &platform),
        Err(Error::InvalidRelease)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_release_cannot_reuse_historical_sri_or_wrapper_manifest() {
    let mut metadata: Value = serde_json::from_slice(WRAPPER).unwrap();
    metadata["dist"]["integrity"] = WRAPPER_INTEGRITY.into();
    assert_eq!(
        verify_metadata("0.160.0", &serde_json::to_vec(&metadata).unwrap(), PLATFORM),
        Err(Error::InvalidRelease)
    );
    let (release, mut wrapper, platform) = official_archives();
    wrapper
        .files
        .get_mut(Path::new("package.json"))
        .unwrap()
        .sha256 = digest_bytes(WRAPPER_FILES[1].2).unwrap();
    assert_eq!(
        verify_archives("0.160.0", &release, &wrapper, &platform),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn current_npm_contract_does_not_authorize_other_versions_or_targets() {
    assert_eq!(supports("0.160.1"), Err(Error::InvalidRelease));
    assert_eq!(supports("latest"), Err(Error::InvalidRelease));
    assert_eq!(supports("0.156.1"), Ok(()));
    assert!(
        NpmRelease::from_metadata(CLIAgent::Codex, "0.160.0", "linux-x64", WRAPPER, PLATFORM)
            .is_err()
    );
    assert!(
        NpmRelease::from_metadata(CLIAgent::Codex, "0.160.0", "win32-arm64", WRAPPER, PLATFORM)
            .is_err()
    );
    assert!(
        NpmRelease::from_metadata(CLIAgent::Codex, "0.160.0", "darwin-x64", WRAPPER, PLATFORM)
            .is_err()
    );
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    assert_eq!(supports("0.160.0"), Err(Error::InvalidRelease));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_metadata_cannot_add_lifecycle_scripts_or_float_the_alias() {
    let mut wrapper: Value = serde_json::from_slice(WRAPPER).unwrap();
    wrapper["scripts"] = serde_json::json!({"postinstall":"node install.js"});
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Codex,
            "0.160.0",
            "darwin-arm64",
            &serde_json::to_vec(&wrapper).unwrap(),
            PLATFORM
        )
        .is_err()
    );
    wrapper.as_object_mut().unwrap().remove("scripts");
    wrapper["optionalDependencies"]["@openai/codex-darwin-arm64"] = "latest".into();
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Codex,
            "0.160.0",
            "darwin-arm64",
            &serde_json::to_vec(&wrapper).unwrap(),
            PLATFORM
        )
        .is_err()
    );
}
