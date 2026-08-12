use remote_server::setup::{RemoteArch, RemoteOs};

use super::*;

fn linux_x86_64_platform() -> RemotePlatform {
    RemotePlatform {
        os: RemoteOs::Linux,
        arch: RemoteArch::X86_64,
    }
}

#[test]
fn bundled_manifest_selects_expected_platform_artifact() {
    let manifest: BundledRemoteServerManifest = serde_json::from_str(
        r#"{
            "version": "v1.2.3",
            "artifacts": [{
                "os": "linux",
                "arch": "x86_64",
                "file": "infinishell-linux-x86_64.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }]
        }"#,
    )
    .unwrap();

    let artifact = select_bundled_artifact(&manifest, &linux_x86_64_platform(), "v1.2.3").unwrap();

    assert_eq!(artifact.file, "infinishell-linux-x86_64.tar.gz");
}

#[test]
fn bundled_manifest_rejects_version_mismatch() {
    let manifest: BundledRemoteServerManifest = serde_json::from_str(
        r#"{
            "version": "v1.2.2",
            "artifacts": []
        }"#,
    )
    .unwrap();

    let error = select_bundled_artifact(&manifest, &linux_x86_64_platform(), "v1.2.3").unwrap_err();

    assert!(error.to_string().contains("version mismatch"));
}

#[tokio::test]
async fn bundled_tarball_sha256_is_verified() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tarball_path = temp_dir.path().join("infinishell-linux-x86_64.tar.gz");
    std::fs::write(&tarball_path, b"bundled remote server").unwrap();

    verify_tarball_sha256(
        &tarball_path,
        "8a8063bc8bb79d2ce3beb44264452cdf9fc9a902170e2900205e7d2b9a3e558b",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn bundled_tarball_sha256_mismatch_is_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tarball_path = temp_dir.path().join("infinishell-linux-x86_64.tar.gz");
    std::fs::write(&tarball_path, b"tampered").unwrap();

    let error = verify_tarball_sha256(
        &tarball_path,
        "6e585fd4a24e5b2c9b3bf53cd0d34273e619087dcd83cb2117ee5ea2a7f51924",
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("SHA-256 mismatch"));
}
