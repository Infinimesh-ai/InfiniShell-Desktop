use std::io::Write as _;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use super::{executable_digest, expected_executable_digests};

const RELEASE_273: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-release-manifest.json"
);
const RELEASE_278: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.278-release-manifest.json"
);

fn assert_platform_matches_official_releases(os: &str, arch: &str, platform: &str) {
    let old: Value = serde_json::from_str(RELEASE_273).unwrap();
    let latest: Value = serde_json::from_str(RELEASE_278).unwrap();
    let expected = expected_executable_digests(os, arch).unwrap();

    assert_eq!(
        expected[0],
        old["platforms"][platform]["checksum"].as_str().unwrap()
    );
    assert_eq!(
        expected[1],
        latest["platforms"][platform]["checksum"].as_str().unwrap()
    );
    assert_ne!(expected[0], expected[1]);
}

#[test]
fn fixed_release_manifests_keep_the_verified_official_digests() {
    assert_eq!(
        format!("{:x}", Sha256::digest(RELEASE_273.as_bytes())),
        "02aa2311fd5d9a4cc9a5aea017b89f5067eec78013bd340f62450719a5393fca"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(RELEASE_278.as_bytes())),
        "d1bf63d94621d6aa6fb84297b235ddb8f5aaadc9252cef8020d170ef661b2f28"
    );
    let old: Value = serde_json::from_str(RELEASE_273).unwrap();
    let latest: Value = serde_json::from_str(RELEASE_278).unwrap();
    assert_eq!(old["version"], "2.1.273");
    assert_eq!(latest["version"], "2.1.278");
}

#[test]
fn macos_arm64_accepts_only_the_two_fixed_release_digests() {
    assert_platform_matches_official_releases("macos", "aarch64", "darwin-arm64");
}

#[test]
fn macos_x64_accepts_only_the_two_fixed_release_digests() {
    assert_platform_matches_official_releases("macos", "x86_64", "darwin-x64");
}

#[test]
fn linux_x64_accepts_only_the_two_fixed_release_digests() {
    assert_platform_matches_official_releases("linux", "x86_64", "linux-x64");
}

#[test]
fn windows_x64_accepts_only_the_two_fixed_release_digests() {
    assert_platform_matches_official_releases("windows", "x86_64", "win32-x64");
}

#[test]
fn known_digest_for_another_platform_is_not_accepted() {
    let macos = expected_executable_digests("macos", "aarch64").unwrap();
    let intel_macos = expected_executable_digests("macos", "x86_64").unwrap();
    let linux = expected_executable_digests("linux", "x86_64").unwrap();
    let windows = expected_executable_digests("windows", "x86_64").unwrap();

    assert!(!macos.contains(&intel_macos[0]));
    assert!(!macos.contains(&intel_macos[1]));
    assert!(!intel_macos.contains(&linux[1]));
    assert!(!linux.contains(&windows[1]));
    assert!(!windows.contains(&macos[1]));
    assert!(!macos.contains(&"0000000000000000000000000000000000000000000000000000000000000000"));
}

#[test]
fn unsupported_platforms_do_not_inherit_another_platform_digest() {
    assert!(expected_executable_digests("linux", "aarch64").is_err());
    assert!(expected_executable_digests("windows", "aarch64").is_err());
    assert!(expected_executable_digests("macos", "arm64").is_err());
    assert!(expected_executable_digests("unknown", "x86_64").is_err());
}

#[test]
fn executable_contents_must_match_a_fixed_release_digest() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"synthetic executable with an untrusted digest")
        .unwrap();

    assert!(executable_digest(file.path()).is_err());
}
