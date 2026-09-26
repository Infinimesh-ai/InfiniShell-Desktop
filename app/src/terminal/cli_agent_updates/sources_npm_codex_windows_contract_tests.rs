use super::{ShimTemplate, cmd_shim, identify_shims, powershell_shim};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

// npm 10.1.0 原始 cmd-shim 6.0.1 模板，代入官方 Codex 的 Node shebang 与相对入口。
// 原始 index.js SHA256：1d0be05b092b015de8cbba5997282670d682fa8b66f2f87825d123748677d752。
fn npm_10_shims() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "codex.cmd".into(),
            "c54db6755e710c39703f7c37512f9e35ed41042d8080558d2b84b8d2694323c3".into(),
        ),
        (
            "codex.ps1".into(),
            "0c149db80ed0bf442c810146b0ad0163b74982fe4542d673f56c354d7b8229cb".into(),
        ),
        (
            "codex".into(),
            "948f713a5ac6c0c81ac1fbc134c1094d6c6ff3684b1b518b573f6355652afa84".into(),
        ),
    ])
}

#[test]
fn accepts_the_complete_official_npm_10_shim_group() {
    assert_eq!(
        identify_shims(&npm_10_shims()),
        Some(ShimTemplate::CmdShim601)
    );
}

#[test]
fn preserves_the_existing_cmd_shim_8_group() {
    let mut shims = npm_10_shims();
    shims.insert(
        "codex".into(),
        "6ee176b43f96b2294c8e2265d599f829e161b2c2ad34cb2b8fbf5f836fd34b8c".into(),
    );

    assert_eq!(identify_shims(&shims), Some(ShimTemplate::CmdShim8));
}

#[test]
fn changing_to_another_known_template_changes_the_bound_identity() {
    let saved = npm_10_shims();
    let mut current = saved.clone();
    current.insert(
        "codex".into(),
        "6ee176b43f96b2294c8e2265d599f829e161b2c2ad34cb2b8fbf5f836fd34b8c".into(),
    );

    assert_ne!(identify_shims(&saved), identify_shims(&current));
    assert_ne!(
        identify_shims(&saved).unwrap().id(),
        identify_shims(&current).unwrap().id()
    );
}

#[test]
fn rejects_a_missing_member_of_a_known_group() {
    let mut shims = npm_10_shims();
    shims.remove("codex.ps1");

    assert_eq!(identify_shims(&shims), None);
}

#[test]
fn rejects_an_extra_member_of_a_known_group() {
    let mut shims = npm_10_shims();
    shims.insert("codex.bat".into(), shims["codex.cmd"].clone());

    assert_eq!(identify_shims(&shims), None);
}

#[test]
fn rejects_normalized_cmd_line_endings() {
    let mut shims = npm_10_shims();
    shims.insert(
        "codex.cmd".into(),
        format!("{:x}", Sha256::digest(cmd_shim().replace("\r\n", "\n"))),
    );

    assert_eq!(identify_shims(&shims), None);
}

#[test]
fn rejects_a_changed_powershell_target_path() {
    let mut shims = npm_10_shims();
    shims.insert(
        "codex.ps1".into(),
        format!(
            "{:x}",
            Sha256::digest(powershell_shim().replace("bin/codex.js", "bin/other.js"))
        ),
    );

    assert_eq!(identify_shims(&shims), None);
}

#[test]
fn rejects_a_changed_shell_shebang() {
    let mut shims = npm_10_shims();
    shims.insert(
        "codex".into(),
        format!(
            "{:x}",
            Sha256::digest(
                ShimTemplate::CmdShim601.shims()[2]
                    .1
                    .replace("#!/bin/sh", "#!/bin/bash")
            )
        ),
    );

    assert_eq!(identify_shims(&shims), None);
}

#[test]
fn rejects_an_unknown_shell_in_an_otherwise_known_group() {
    let mut shims = npm_10_shims();
    shims.insert(
        "codex".into(),
        format!("{:x}", Sha256::digest(b"#!/bin/sh\n")),
    );

    assert_eq!(identify_shims(&shims), None);
}
