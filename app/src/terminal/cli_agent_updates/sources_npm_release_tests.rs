use std::io::{self, Cursor, SeekFrom, Write as _};

use flate2::{Compression, write::GzEncoder};
use tar::{Builder, EntryType, Header};

use super::*;

fn wrapper_manifest(agent: CLIAgent) -> Value {
    let (package, version, bin, targets) = if agent == CLIAgent::Codex {
        (
            "@openai/codex",
            "0.156.1",
            json!({"codex":"bin/codex.js"}),
            CODEX_TARGETS.as_slice(),
        )
    } else {
        (
            "@anthropic-ai/claude-code",
            "2.1.280",
            json!({"claude":"bin/claude.exe"}),
            CLAUDE_TARGETS.as_slice(),
        )
    };
    let optional: BTreeMap<_, _> = targets
        .iter()
        .map(|target| {
            (
                format!("{package}-{target}"),
                if agent == CLIAgent::Codex {
                    format!("npm:@openai/codex@0.156.1-{target}")
                } else {
                    version.to_owned()
                },
            )
        })
        .collect();
    let mut value =
        json!({"name":package,"version":version,"bin":bin,"optionalDependencies":optional});
    if agent == CLIAgent::Claude {
        value["scripts"] = json!({"prepare":CLAUDE_PREPARE,"postinstall":"node install.cjs"});
        value["dependencies"] = json!({});
    }
    value
}

fn registry(mut manifest: Value, integrity: &[u8], size: u64, count: u64) -> Value {
    let name = manifest["name"].as_str().unwrap();
    let version = manifest["version"].as_str().unwrap();
    let leaf = name.rsplit('/').next().unwrap();
    manifest["dist"] = json!({
        "tarball":format!("https://registry.npmjs.org/{name}/-/{leaf}-{version}.tgz"),
        "integrity":format!("sha512-{}",STANDARD.encode(integrity)),
        "unpackedSize":size,"fileCount":count,
    });
    manifest
}

fn release(agent: CLIAgent, wrapper: Value, platform: Value) -> Result<NpmRelease, Error> {
    NpmRelease::from_metadata(
        agent,
        if agent == CLIAgent::Codex {
            "0.156.1"
        } else {
            "2.1.280"
        },
        "darwin-arm64",
        &serde_json::to_vec(&wrapper).unwrap(),
        &serde_json::to_vec(&platform).unwrap(),
    )
}

fn metadata_pair(agent: CLIAgent) -> (Value, Value) {
    let wrapper = registry(wrapper_manifest(agent), &[0; 64], 100, 3);
    let platform = if agent == CLIAgent::Codex {
        json!({"name":"@openai/codex","version":"0.156.1-darwin-arm64","os":["darwin"],"cpu":["arm64"]})
    } else {
        json!({"name":"@anthropic-ai/claude-code-darwin-arm64","version":"2.1.280","os":["darwin"],"cpu":["arm64"]})
    };
    (wrapper, registry(platform, &[1; 64], 1024, 4))
}

#[test]
fn codex_alias_directory_keeps_official_manifest_identity() {
    let (wrapper, platform) = metadata_pair(CLIAgent::Codex);
    let accepted = release(CLIAgent::Codex, wrapper.clone(), platform.clone()).unwrap();
    assert_eq!(LAYOUT_VERSION, 1);
    assert_eq!(
        accepted.dependency_directory,
        PathBuf::from("node_modules/@openai/codex-darwin-arm64")
    );
    assert_eq!(
        accepted.native_entry,
        PathBuf::from("vendor/aarch64-apple-darwin/bin/codex")
    );
    assert!(!accepted.materialize_native_entry);
    let mut wrong = platform;
    wrong["name"] = json!("@openai/codex-darwin-arm64");
    assert!(release(CLIAgent::Codex, wrapper, wrong).is_err());
}

#[test]
fn claude_declared_placeholder_is_materialized_without_postinstall() {
    let (wrapper, platform) = metadata_pair(CLIAgent::Claude);
    let accepted = release(CLIAgent::Claude, wrapper.clone(), platform.clone()).unwrap();
    assert_eq!(accepted.public_entry, PathBuf::from("bin/claude.exe"));
    assert_eq!(accepted.native_entry, PathBuf::from("claude"));
    assert!(accepted.materialize_native_entry);
    let mut changed = wrapper;
    changed["scripts"]["postinstall"] = json!("node install.cjs && arbitrary-command");
    assert!(release(CLIAgent::Claude, changed, platform).is_err());
}

#[test]
fn undeclared_dependencies_and_lifecycle_scripts_are_rejected() {
    for key in [
        "dependencies",
        "peerDependencies",
        "bundledDependencies",
        "bundleDependencies",
        "scripts",
    ] {
        let (mut wrapper, platform) = metadata_pair(CLIAgent::Codex);
        wrapper[key] = json!({"unexpected":"1.0.0"});
        assert!(
            release(CLIAgent::Codex, wrapper, platform).is_err(),
            "{key}"
        );
    }
    let (wrapper, mut platform) = metadata_pair(CLIAgent::Codex);
    platform["optionalDependencies"] = json!({"unexpected":"1.0.0"});
    assert!(release(CLIAgent::Codex, wrapper, platform).is_err());
}

#[test]
fn dependency_alias_and_platform_version_cannot_float() {
    let (mut wrapper, platform) = metadata_pair(CLIAgent::Codex);
    wrapper["optionalDependencies"]["@openai/codex-darwin-arm64"] = json!("latest");
    assert!(release(CLIAgent::Codex, wrapper, platform).is_err());
    let (wrapper, mut platform) = metadata_pair(CLIAgent::Claude);
    platform["version"] = json!("2.1.281");
    assert!(release(CLIAgent::Claude, wrapper, platform).is_err());
}

#[test]
fn metadata_does_not_authorize_a_new_release_or_platform() {
    let (wrapper, platform) = metadata_pair(CLIAgent::Codex);
    let wrapper = serde_json::to_vec(&wrapper).unwrap();
    let platform = serde_json::to_vec(&platform).unwrap();
    for version in ["0.156.0", "0.156.2", "latest"] {
        assert!(
            NpmRelease::from_metadata(
                CLIAgent::Codex,
                version,
                "darwin-arm64",
                &wrapper,
                &platform
            )
            .is_err()
        );
    }
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Codex,
            "0.156.1",
            "freebsd-x64",
            &wrapper,
            &platform
        )
        .is_err()
    );
    assert!(
        NpmRelease::from_metadata(
            CLIAgent::Grok,
            "1.0.41",
            "darwin-arm64",
            &wrapper,
            &platform
        )
        .is_err()
    );
}

#[test]
fn tarball_must_have_exact_official_url_and_single_sha512() {
    for url in [
        "http://registry.npmjs.org/@openai/codex/-/codex-0.156.1.tgz",
        "https://registry.npmjs.org.evil.test/codex.tgz",
        "https://registry.npmjs.org/@openai/codex/-/codex-0.156.1.tgz?token=example",
        "https://user@registry.npmjs.org/@openai/codex/-/codex-0.156.1.tgz",
    ] {
        let (mut wrapper, platform) = metadata_pair(CLIAgent::Codex);
        wrapper["dist"]["tarball"] = json!(url);
        assert!(release(CLIAgent::Codex, wrapper, platform).is_err());
    }
    for sri in [
        format!("sha256-{}", STANDARD.encode([0; 32])),
        "sha512-not-base64".to_owned(),
        format!(
            "sha512-{} sha512-{}",
            STANDARD.encode([0; 64]),
            STANDARD.encode([1; 64])
        ),
    ] {
        let (mut wrapper, platform) = metadata_pair(CLIAgent::Codex);
        wrapper["dist"]["integrity"] = json!(sri);
        assert!(release(CLIAgent::Codex, wrapper, platform).is_err());
    }
}

fn archive(entries: &[(&str, &[u8], EntryType, u32)]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    for (path, bytes, kind, mode) in entries {
        let mut header = Header::new_ustar();
        header.set_entry_type(*kind);
        header.set_mode(*mode);
        header.set_size(bytes.len() as u64);
        // 构造 tar 库不会主动生成的恶意原始名称，校验生产路径过滤器。
        assert!(path.len() < 100);
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        header.set_cksum();
        builder.append(&header, *bytes).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
    gzip.write_all(&tar).unwrap();
    gzip.finish().unwrap()
}

fn archive_fixture(extra: &[(&str, &[u8], EntryType, u32)]) -> (NpmArtifact, Vec<u8>) {
    let manifest = wrapper_manifest(CLIAgent::Codex);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let mut files = vec![(
        "package/package.json",
        bytes.as_slice(),
        EntryType::Regular,
        0o644,
    )];
    files.extend_from_slice(extra);
    let gzip = archive(&files);
    let count = files.len() as u64;
    let size = files.iter().map(|entry| entry.1.len() as u64).sum();
    let metadata = registry(manifest.clone(), &Sha512::digest(&gzip), size, count);
    (
        NpmArtifact::from_metadata(
            &serde_json::to_vec(&metadata).unwrap(),
            contract(&manifest).unwrap(),
        )
        .unwrap(),
        gzip,
    )
}

#[test]
fn archive_hashes_every_file_and_never_extracts() {
    let (artifact, gzip) = archive_fixture(&[(
        "package/bin/codex.js",
        b"native launcher",
        EntryType::Regular,
        0o755,
    )]);
    let accepted = artifact.verify_archive(&mut Cursor::new(&gzip)).unwrap();
    assert_eq!(
        accepted.compressed_sha256,
        <[u8; 32]>::from(Sha256::digest(&gzip))
    );
    let bin = &accepted.files[&PathBuf::from("bin/codex.js")];
    assert_eq!(
        bin.sha256,
        <[u8; 32]>::from(Sha256::digest(b"native launcher"))
    );
    assert_eq!(bin.length, 15);
    assert!(bin.executable);
}

#[test]
fn compressed_corruption_and_registry_size_mismatch_are_rejected() {
    let (mut artifact, mut gzip) = archive_fixture(&[]);
    gzip[12] ^= 1;
    assert!(artifact.verify_archive(&mut Cursor::new(gzip)).is_err());
    let (_, clean) = archive_fixture(&[]);
    artifact.file_count += 1;
    assert!(artifact.verify_archive(&mut Cursor::new(&clean)).is_err());
    artifact.file_count -= 1;
    artifact.unpacked_size += 1;
    assert!(artifact.verify_archive(&mut Cursor::new(&clean)).is_err());
}

#[test]
fn links_devices_sparse_and_extension_headers_are_rejected() {
    for kind in [
        EntryType::Symlink,
        EntryType::Link,
        EntryType::Char,
        EntryType::Block,
        EntryType::Fifo,
        EntryType::GNUSparse,
        EntryType::GNULongName,
        EntryType::XHeader,
        EntryType::XGlobalHeader,
    ] {
        let (artifact, gzip) = archive_fixture(&[("package/unsafe", b"", kind, 0o644)]);
        assert!(
            artifact.verify_archive(&mut Cursor::new(gzip)).is_err(),
            "{kind:?}"
        );
    }
}

#[test]
fn archive_paths_reject_both_platforms_escape_and_alias_forms() {
    for path in [
        "/absolute",
        "package/../outside",
        "package/a/./b",
        "package/a//b",
        "package/C:/escape",
        "package/a\\b",
        "package/CON.txt",
        "package/a/nul",
        "package/a/COM1",
        "package/a.",
        "package/a ",
        "package/CONIN$",
        "package/longfi~1",
        "package/é",
        "package/",
    ] {
        let (artifact, gzip) = archive_fixture(&[(path, b"x", EntryType::Regular, 0o644)]);
        assert!(
            artifact.verify_archive(&mut Cursor::new(gzip)).is_err(),
            "{path}"
        );
    }
}

#[test]
fn duplicate_case_and_file_directory_collisions_are_rejected() {
    for paths in [
        ["package/A", "package/a"],
        ["package/a", "package/a"],
        ["package/A", "package/a/child"],
        ["package/A/first", "package/a/second"],
    ] {
        let files = paths.map(|path| (path, b"x".as_slice(), EntryType::Regular, 0o644));
        let (artifact, gzip) = archive_fixture(&files);
        assert!(artifact.verify_archive(&mut Cursor::new(gzip)).is_err());
    }
}

#[test]
fn embedded_manifest_must_match_registry_contract() {
    let (mut artifact, gzip) = archive_fixture(&[]);
    artifact.manifest_contract["version"] = json!("different");
    assert!(artifact.verify_archive(&mut Cursor::new(gzip)).is_err());
    let (artifact, gzip) = archive_fixture(&[("package/setuid", b"x", EntryType::Regular, 0o4755)]);
    assert!(artifact.verify_archive(&mut Cursor::new(gzip)).is_err());
}

#[test]
fn declared_expand_and_file_count_limits_are_hard_bounds() {
    for (key, value) in [
        ("unpackedSize", 0),
        ("unpackedSize", MAX_UNPACKED + 1),
        ("fileCount", 0),
        ("fileCount", MAX_FILES + 1),
    ] {
        let (mut wrapper, platform) = metadata_pair(CLIAgent::Codex);
        wrapper["dist"][key] = json!(value);
        assert!(release(CLIAgent::Codex, wrapper, platform).is_err());
    }
}

struct ReplacedReader {
    before: Cursor<Vec<u8>>,
    after: Cursor<Vec<u8>>,
    starts: usize,
}

impl Read for ReplacedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.starts < 2 {
            self.before.read(bytes)
        } else {
            self.after.read(bytes)
        }
    }
}

impl Seek for ReplacedReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if position == SeekFrom::Start(0) {
            self.starts += 1;
        }
        if self.starts < 2 {
            self.before.seek(position)
        } else {
            self.after.seek(position)
        }
    }
}

#[test]
fn archive_replacement_between_integrity_check_and_parse_is_rejected() {
    let (artifact, before) = archive_fixture(&[(
        "package/bin/codex.js",
        b"approved",
        EntryType::Regular,
        0o755,
    )]);
    let (_, after) = archive_fixture(&[(
        "package/bin/codex.js",
        b"replaced",
        EntryType::Regular,
        0o755,
    )]);
    let mut replaced = ReplacedReader {
        before: Cursor::new(before),
        after: Cursor::new(after),
        starts: 0,
    };
    assert!(artifact.verify_archive(&mut replaced).is_err());
}

#[test]
fn extraction_validates_integrity_before_any_member_is_written() {
    let (artifact, mut gzip) = archive_fixture(&[]);
    gzip[12] ^= 1;
    let mut writes = 0;

    assert!(
        artifact
            .extract_verified(&mut Cursor::new(gzip), |_, _, _| {
                writes += 1;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(writes, 0);
}

#[test]
fn extraction_passes_only_verified_relative_members() {
    let (artifact, gzip) = archive_fixture(&[(
        "package/bin/codex.js",
        b"verified",
        EntryType::Regular,
        0o755,
    )]);
    let mut contents = BTreeMap::new();

    artifact
        .extract_verified(&mut Cursor::new(gzip), |path, expected, reader| {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes.len() as u64, expected.length);
            assert_eq!(<[u8; 32]>::from(Sha256::digest(&bytes)), expected.sha256);
            contents.insert(path.to_owned(), bytes);
            Ok(())
        })
        .unwrap();

    assert_eq!(contents.len(), 2);
    assert_eq!(contents[&PathBuf::from("bin/codex.js")], b"verified");
}

#[test]
fn extraction_cannot_accept_an_unconsumed_member() {
    let (artifact, gzip) = archive_fixture(&[]);

    assert!(
        artifact
            .extract_verified(&mut Cursor::new(gzip), |_, _, _| Ok(()))
            .is_err()
    );
}
