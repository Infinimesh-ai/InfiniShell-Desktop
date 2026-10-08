use super::*;

const STABLE: &[u8] =
    include_bytes!("fixtures/claude-winget-current-release/2.1.285-installer.yaml");
const SOURCE: &[u8] =
    include_bytes!("fixtures/claude-winget-current-release/2.1.286-installer.yaml");

#[test]
fn official_manifests_bind_complete_bytes_and_x64_images() {
    for (version, bytes, length) in [(TO, STABLE, 243_751_072), (FROM, SOURCE, 245_092_000)] {
        let value = contract(version).unwrap();
        assert_eq!(bytes.len() as u64, value.winget_installer_bytes);
        assert_eq!(
            <[u8; 32]>::from(Sha256::digest(bytes)),
            manifest_sha256(version).unwrap()
        );
        assert_eq!(
            serde_yaml::from_slice::<Value>(bytes).unwrap(),
            value.expected_complete_manifest
        );
        let x64 = &value.expected_complete_manifest["Installers"][0];
        assert_eq!(x64["Architecture"], "x64");
        assert_eq!(x64["InstallerUrl"], value.native_url);
        assert_eq!(native(version).unwrap().0, length);
        assert_eq!(
            native(version).unwrap().1,
            brew::decode_sha256(x64["InstallerSha256"].as_str().unwrap()).unwrap()
        );
    }
    let (url, digest) = release(TO, STABLE).unwrap();
    assert!(url.ends_with("/2.1.285/win32-x64/claude.exe"));
    assert_eq!(digest, native(TO).unwrap().1);
    assert!(
        metadata_url(TO)
            .unwrap()
            .contains("/f0fb65e5263ef3f223bb0a750b6d1246189c5956/")
    );
}

#[test]
fn changed_manifest_or_unimplemented_target_is_rejected() {
    let mut altered = STABLE.to_vec();
    altered.push(b'\n');
    assert!(release(TO, &altered).is_err());
    assert!(release(TO, SOURCE).is_err());
    for version in ["2.1.280", FROM, "2.1.287", "2.1.285-arm64"] {
        assert!(release(version, STABLE).is_err());
        assert!(metadata_url(version).is_err());
    }
    assert!(native("2.1.287").is_err());
    assert!(native("2.1.285-arm64").is_err());
}
