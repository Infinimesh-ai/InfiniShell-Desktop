use super::super::ExpectedFileId;
use super::*;

const ROOT: &str = "/fixture/.infinishell-npm-70fb67a3-fdd5-4cc9-bb12-a4380b1bb213";
const NODE: &str = "/fixture/node";

fn identity(path: PathBuf, size: u64, sha256: String) -> ExpectedFileIdentity {
    ExpectedFileIdentity {
        canonical_path: path.clone(),
        path,
        size,
        sha256,
        file_id: Some(ExpectedFileId {
            volume: 1,
            index: 2,
        }),
    }
}

fn historical_closure() -> Vec<ExpectedFileIdentity> {
    let (target, package, triple) = platform().unwrap();
    let root = Path::new(ROOT);
    let manifest: serde_json::Value = serde_json::from_slice(PACKAGE_MANIFEST).unwrap();
    let files: BTreeMap<PathBuf, (u64, String, u32)> =
        serde_json::from_value(manifest["packages"][package]["files"].clone()).unwrap();
    let dependency = root.join(format!("node_modules/@openai/codex-{target}"));
    let mut result = vec![
        identity(NODE.into(), 1, "0".repeat(64)),
        identity(
            root.join("bin/codex.js"),
            8790,
            "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70".into(),
        ),
        identity(
            root.join("package.json"),
            1082,
            "c3f16464dca0fe1269b17d02fe0997d1ca3a241c3a61da3d89ec13def0a66c6e".into(),
        ),
        identity(
            root.join("README.md"),
            3334,
            "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7".into(),
        ),
        identity(dependency.join("package.json"), 517, "1".repeat(64)),
        identity(
            dependency.join("README.md"),
            3334,
            "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7".into(),
        ),
    ];
    result.extend(files.into_iter().map(|(path, (size, sha256, _mode))| {
        identity(
            dependency.join("vendor").join(triple).join(path),
            size,
            sha256,
        )
    }));
    result
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn current_closure() -> Vec<ExpectedFileIdentity> {
    let root = Path::new(ROOT);
    let mut result = vec![
        identity(NODE.into(), 1, "0".repeat(64)),
        identity(
            root.join("bin/codex.js"),
            8790,
            "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70".into(),
        ),
        identity(
            root.join("package.json"),
            1082,
            "29c350dfcd8d33749852c16e2f5dcde528409d7e1d3f5d916fe3c576f3914820".into(),
        ),
        identity(
            root.join("README.md"),
            3334,
            "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7".into(),
        ),
    ];
    // 直接使用官方平台 tar 原件的独立库存；此测试只验证 worker 的输入闭包。
    let files: BTreeMap<PathBuf, (u64, String, u32)> = serde_json::from_slice(include_bytes!(
        "../../terminal/cli_agent_updates/fixtures/codex-0160/platform-files.json"
    ))
    .unwrap();
    result.extend(files.into_iter().map(|(path, (size, sha256, _mode))| {
        identity(
            root.join("node_modules/@openai/codex-darwin-arm64")
                .join(path),
            size,
            sha256,
        )
    }));
    result
}

fn arguments() -> [OsString; 2] {
    [
        Path::new(ROOT).join("bin/codex.js").into_os_string(),
        "--version".into(),
    ]
}

#[test]
fn historical_npm_worker_closure_remains_valid() {
    assert!(validate_contract(Path::new(NODE), &arguments(), &historical_closure()).is_ok());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_macos_npm_worker_accepts_the_official_complete_inventory() {
    assert!(validate_contract(Path::new(NODE), &arguments(), &current_closure()).is_ok());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_worker_cannot_mix_historical_native_files_with_new_wrapper() {
    let mut files = current_closure();
    let old_native = historical_closure()
        .into_iter()
        .find(|file| file.path.ends_with("vendor/aarch64-apple-darwin/bin/codex"))
        .unwrap();
    let current_native = files
        .iter_mut()
        .find(|file| file.path == old_native.path)
        .unwrap();
    *current_native = old_native;
    assert!(validate_contract(Path::new(NODE), &arguments(), &files).is_err());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_worker_rejects_a_missing_package_asset() {
    let mut files = current_closure();
    files.retain(|file| !file.path.ends_with("bin/codex-code-mode-host"));
    assert!(validate_contract(Path::new(NODE), &arguments(), &files).is_err());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_worker_rejects_changed_platform_alias_metadata() {
    let mut files = current_closure();
    let file = files
        .iter_mut()
        .find(|file| {
            file.path
                .ends_with("node_modules/@openai/codex-darwin-arm64/package.json")
        })
        .unwrap();
    file.sha256 = "0".repeat(64);
    assert!(validate_contract(Path::new(NODE), &arguments(), &files).is_err());
}

#[test]
fn unknown_wrapper_digest_never_selects_a_native_inventory() {
    let mut files = historical_closure();
    files[2].sha256 = "0".repeat(64);
    assert!(validate_contract(Path::new(NODE), &arguments(), &files).is_err());
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn new_macos_wrapper_does_not_expand_other_platform_workers() {
    let mut files = historical_closure();
    files[2].sha256 = "29c350dfcd8d33749852c16e2f5dcde528409d7e1d3f5d916fe3c576f3914820".into();
    assert!(validate_contract(Path::new(NODE), &arguments(), &files).is_err());
}
