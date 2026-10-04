//! 官方小型 metadata 只用于供应链合同回归；这里不执行平台二进制。

use super::super::{
    CLIAgent,
    npm_release::{NpmArchiveFile, NpmRelease, VerifiedNpmArchive},
};
use super::*;

fn metadata(version: &str) -> (Value, Value) {
    let value: Value =
        serde_json::from_slice(include_bytes!("sources_claude_musl_metadata.fixture.json"))
            .unwrap();
    (
        value[version]["wrapper"].clone(),
        value[version]["platform"].clone(),
    )
}

fn parse(version: &str, wrapper: &Value, platform: &Value) -> Result<NpmRelease, Error> {
    NpmRelease::from_metadata(
        CLIAgent::Claude,
        version,
        "linux-x64-musl",
        &serde_json::to_vec(wrapper).unwrap(),
        &serde_json::to_vec(platform).unwrap(),
    )
}

fn accepts_fixed_native(version: &str, length: u64, digest: &str) {
    let (wrapper, platform) = metadata(version);
    let release = parse(version, &wrapper, &platform).unwrap();
    assert_eq!(release.native_entry, PathBuf::from("claude"));
    assert_eq!(release.public_entry, PathBuf::from("bin/claude.exe"));
    assert_eq!(
        release.dependency_directory,
        PathBuf::from("node_modules/@anthropic-ai/claude-code-linux-x64-musl")
    );
    let image = native(version, "linux-x64-musl").unwrap();
    assert_eq!(image.0, length);
    assert_eq!(hex::encode(image.1), digest);
}

#[test]
fn official_musl_278_keeps_the_original_downgrade_target() {
    accepts_fixed_native(
        "2.1.278",
        228_036_568,
        "e21d4818a7282c1b18f7949af863a58e640f017a8ffa4053f5ae927ac95d954a",
    );
    let wrapper = release("2.1.278", "linux-x64-musl").unwrap().wrapper;
    for (path, (length, digest, mode)) in super::super::claude_downgrade::files("wrapper").unwrap()
    {
        let file = &wrapper.files[&path];
        assert_eq!(
            (file.length, file.sha256.as_str(), file.mode),
            (length, digest, mode)
        );
    }
}

#[test]
fn official_musl_280_keeps_the_original_upgrade_target() {
    accepts_fixed_native(
        "2.1.280",
        227_467_360,
        "8d25ffbf600882d7b985706cc10ea45995060637ed009b5700b65e162bb00b96",
    );
}

#[test]
fn official_musl_285_is_a_distinct_consumer_native() {
    accepts_fixed_native(
        V285,
        234_078_296,
        "7b4414af1bc06eb6d91730759bd01c4c02a4976d0b8526237e9a6d8c33eaf102",
    );
}

#[test]
fn official_musl_287_is_a_distinct_consumer_native() {
    accepts_fixed_native(
        V287,
        238_063_704,
        "ba22ce4b5744c6016e3076ccbd692a059fc4e9f1592f21361859a73938f4e722",
    );
}

#[test]
fn musl_metadata_rejects_libc_version_and_sri_substitution() {
    let (wrapper, platform) = metadata(V285);
    let mut glibc = platform.clone();
    glibc["libc"] = serde_json::json!(["glibc"]);
    assert!(parse(V285, &wrapper, &glibc).is_err());
    let (_, other_version) = metadata(V287);
    assert!(parse(V285, &wrapper, &other_version).is_err());
    let mut different_sri = platform;
    different_sri["dist"]["integrity"] = other_version["dist"]["integrity"].clone();
    assert!(parse(V285, &wrapper, &different_sri).is_err());
    assert_eq!(
        supports("2.1.288", "linux-x64-musl"),
        Err(Error::InvalidRelease)
    );
    assert_eq!(
        supports(V285, "linux-arm64-musl"),
        Err(Error::UnsupportedPlatform)
    );
}

fn inventory(artifact: Artifact) -> VerifiedNpmArchive {
    VerifiedNpmArchive {
        compressed_sha256: hex::decode(artifact.sha256).unwrap().try_into().unwrap(),
        files: artifact
            .files
            .into_iter()
            .map(|(path, file)| {
                (
                    path,
                    NpmArchiveFile {
                        length: file.length,
                        sha256: hex::decode(file.sha256).unwrap().try_into().unwrap(),
                        mode: file.mode,
                        executable: file.mode & 0o111 != 0,
                    },
                )
            })
            .collect(),
    }
}

#[test]
fn musl_archive_contract_rejects_member_mode_hash_and_extra_file() {
    let fixed = release(V285, "linux-x64-musl").unwrap();
    let wrapper = inventory(fixed.wrapper);
    let mut platform = inventory(fixed.platform);
    assert_eq!(
        verify_archives(V285, "linux-x64-musl", &wrapper, &platform),
        Ok(())
    );
    platform
        .files
        .get_mut(&PathBuf::from("claude"))
        .unwrap()
        .mode = 0o644;
    assert!(verify_archives(V285, "linux-x64-musl", &wrapper, &platform).is_err());
    platform
        .files
        .get_mut(&PathBuf::from("claude"))
        .unwrap()
        .mode = 0o755;
    platform
        .files
        .get_mut(&PathBuf::from("claude"))
        .unwrap()
        .sha256 = [0; 32];
    assert!(verify_archives(V285, "linux-x64-musl", &wrapper, &platform).is_err());
    let mut platform = inventory(release(V285, "linux-x64-musl").unwrap().platform);
    platform.files.insert(
        PathBuf::from("unreviewed"),
        NpmArchiveFile {
            length: 0,
            sha256: [0; 32],
            mode: 0o644,
            executable: false,
        },
    );
    assert!(verify_archives(V285, "linux-x64-musl", &wrapper, &platform).is_err());
}
