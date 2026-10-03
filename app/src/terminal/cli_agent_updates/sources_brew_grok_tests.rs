use super::*;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CURRENT_CASK: &[u8] = include_bytes!("fixtures/grok-current-release/1.0.46-grok-build.rb");

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
))]
#[test]
fn historical_cask_url_and_update_edge_remain_bound_to_41() {
    assert_eq!(
        metadata_url("1.0.41").unwrap(),
        "https://raw.githubusercontent.com/Homebrew/homebrew-cask/52a97ac96ae1af1a764eaefe0bdcad4296a12541/Casks/g/grok-build.rb"
    );
    assert_eq!(supports_transition("1.0.40", "1.0.41"), Ok(()));
    assert!(native("1.0.40").is_ok());
    assert!(native("1.0.41").is_ok());
    assert!(supports("1.0.45").is_err());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_cask_source_binds_native_and_unchanged_alias_completion_contract() {
    let (metadata, url, digest) = release("1.0.46", CURRENT_CASK).unwrap();
    assert_eq!(
        metadata["tap_git_head"],
        "8d9df9ae501d586458789ecbf01115d91fe838c0"
    );
    assert_eq!(url, "https://x.ai/cli/grok-1.0.46-macos-aarch64");
    assert_eq!(
        digest,
        brew::decode_sha256("e8daa302364c9c3b6a5546d511cfbd1ab5e5d407a9b04282f660665ea405f9f3")
            .unwrap()
    );
    assert_eq!(native("1.0.46").unwrap().0, 150374256);
    assert_eq!(
        metadata["artifacts"],
        json!([
            {"binary":["grok-1.0.46-macos-aarch64",{"target":"grok"}]},
            {"binary":["grok-1.0.46-macos-aarch64",{"target":"agent"}]},
            {"generate_completions_from_executable":["grok-1.0.46-macos-aarch64","completions",{"base_name":"grok","shell_parameter_format":null,"shells":["bash","zsh","fish"]}]},
            {"zap":[{"rmdir":"~/.grok"}]}
        ])
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_cask_rejects_source_edits_and_version_substitution() {
    assert!(release("1.0.41", CURRENT_CASK).is_err());
    let mut changed = CURRENT_CASK.to_vec();
    changed.push(b'\n');
    assert!(release("1.0.46", &changed).is_err());
    assert_eq!(supports_transition("1.0.41", "1.0.46"), Ok(()));
    assert_eq!(
        supports_transition("1.0.46", "1.0.41"),
        Err(Error::InvalidRelease)
    );
    assert!(supports_transition("1.0.45", "1.0.46").is_err());
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn current_macos_cask_does_not_expand_other_platforms() {
    assert!(supports("1.0.46").is_err());
    assert!(metadata_url("1.0.46").is_err());
    assert!(native("1.0.46").is_err());
}
