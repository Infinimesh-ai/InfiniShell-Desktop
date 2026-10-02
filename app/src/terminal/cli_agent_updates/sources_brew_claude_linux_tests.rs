#![cfg(target_arch = "x86_64")]

use super::*;

const CASK_285: &[u8] = include_bytes!("fixtures/claude-current-release/2.1.285-claude-code.rb");
const CASK_287: &[u8] =
    include_bytes!("fixtures/claude-current-release/2.1.287-claude-code@latest.rb");

#[test]
fn fixed_ruby_sources_match_official_linux_metadata_without_using_mac_images() {
    for (name, version, ruby, api) in [
        (
            "claude-code",
            "2.1.285",
            CASK_285,
            include_bytes!("fixtures/claude-current-release/cask-claude-code.json").as_slice(),
        ),
        (
            TOKEN,
            "2.1.287",
            CASK_287,
            include_bytes!("fixtures/claude-current-release/cask-claude-code@latest.json")
                .as_slice(),
        ),
    ] {
        let api: Value = serde_json::from_slice(api).unwrap();
        let (metadata, url, digest) = release(name, version, ruby).unwrap();
        for field in [
            "token",
            "version",
            "tap",
            "tap_git_head",
            "artifacts",
            "conflicts_with",
            "ruby_source_path",
            "ruby_source_checksum",
            "url_specs",
            "depends_on",
        ] {
            assert_eq!(metadata[field], api[field], "{name}: {field}");
        }
        assert_eq!(metadata["url"], api["variations"]["x86_64_linux"]["url"]);
        assert_eq!(
            metadata["sha256"],
            api["variations"]["x86_64_linux"]["sha256"]
        );
        assert_eq!(metadata["url"], url);
        assert_eq!(metadata["sha256"], hex::encode(digest));
        assert_ne!(metadata["sha256"], api["sha256"]);
        assert_eq!(
            metadata_url(name, version).unwrap(),
            format!(
                "https://raw.githubusercontent.com/Homebrew/homebrew-cask/{MIGRATION_TAP}/Casks/c/{name}.rb"
            )
        );
    }
}

#[test]
fn fixed_ruby_contract_rejects_edits_wrong_token_and_unreviewed_versions() {
    let mut modified = CASK_285.to_vec();
    modified.push(b'\n');
    assert!(release("claude-code", "2.1.285", &modified).is_err());
    for (name, version, ruby) in [
        (TOKEN, "2.1.285", CASK_285),
        ("claude-code", "2.1.287", CASK_287),
        ("claude-code", "2.1.285", CASK_287),
        (TOKEN, "2.1.280", CASK_287),
        (TOKEN, "2.1.288", CASK_287),
    ] {
        assert!(release(name, version, ruby).is_err());
    }
}

#[test]
fn migration_does_not_expand_ordinary_update_or_old_journal_recovery() {
    assert_eq!(metadata_url(TOKEN, "2.1.280").unwrap(), METADATA_URL);
    supports("2.1.280").unwrap();
    for version in ["2.1.278", "2.1.285", "2.1.287", "2.1.288"] {
        assert!(supports(version).is_err());
    }
    for (old, target, allowed) in [
        ("2.1.278", "2.1.280", true),
        ("2.1.280", "2.1.280", true),
        ("2.1.287", "2.1.285", true),
        ("2.1.285", "2.1.285", true),
        ("2.1.287", "2.1.287", true),
        ("2.1.280", "2.1.287", false),
        ("2.1.280", "2.1.285", false),
        ("2.1.285", "2.1.287", false),
        ("2.1.287", "2.1.280", false),
        ("2.1.288", "2.1.285", false),
    ] {
        assert_eq!(
            supports_transition(old, target).is_ok(),
            allowed,
            "{old} → {target}"
        );
    }
    verify_recovery(prefix().unwrap(), TOKEN, "2.1.278", "2.1.280").unwrap();
    assert!(verify_recovery(prefix().unwrap(), TOKEN, "2.1.287", "2.1.285").is_err());
    assert!(verify_recovery(prefix().unwrap(), TOKEN, "2.1.287", "2.1.280").is_err());
    assert!(verify_recovery(prefix().unwrap(), "claude-code", "2.1.278", "2.1.280").is_err());
}

fn receipt(version: &str) -> Value {
    json!({
        "homebrew_version":"7.0.4", "arch":"x86_64", "runtime_dependencies":{},
        "uninstall_flight_blocks":false, "uninstall_artifacts":artifacts(),
        "source":{
            "tap":"homebrew/cask", "version":version,
            "path":format!("/var/tmp/official-cask/{}.rb", token(version).unwrap()),
            "tap_git_head":"f29f0641e45b9eef3f66a10515239a083d5c6cf9"
        }
    })
}

#[test]
fn receipt_files_bind_token_arch_and_dependencies_without_pinning_install_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join(".metadata")).unwrap();
    let path = root.join(".metadata/INSTALL_RECEIPT.json");
    for version in ["2.1.285", "2.1.287"] {
        let original = receipt(version);
        fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        verify_receipt(&root, version).unwrap();
        for (pointer, changed) in [
            ("/source/path", json!("/other/unknown.rb")),
            (
                "/source/path",
                json!(format!("{}.rb", token(version).unwrap())),
            ),
            ("/source/tap_git_head", json!("unknown")),
            ("/source/tap", json!("other/cask")),
            ("/source/version", json!("2.1.288")),
            ("/arch", json!("arm64")),
            ("/homebrew_version", json!("7.0.5")),
            ("/runtime_dependencies", json!({"formula":["node"]})),
            ("/uninstall_flight_blocks", json!(true)),
        ] {
            let mut modified = original.clone();
            *modified.pointer_mut(pointer).unwrap() = changed;
            fs::write(&path, serde_json::to_vec(&modified).unwrap()).unwrap();
            assert!(
                verify_receipt(&root, version).is_err(),
                "{version}: {pointer}"
            );
        }
    }
    // 原278/280 tab的读取合同不因新增迁移而要求新的字段。
    let mut historical = receipt("2.1.278");
    historical["source"].as_object_mut().unwrap().remove("path");
    historical["source"]
        .as_object_mut()
        .unwrap()
        .remove("tap_git_head");
    fs::write(&path, serde_json::to_vec(&historical).unwrap()).unwrap();
    verify_receipt(&root, "2.1.278").unwrap();
}
