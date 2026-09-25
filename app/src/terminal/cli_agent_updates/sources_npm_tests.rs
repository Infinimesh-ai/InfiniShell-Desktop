use super::*;
use crate::terminal::cli_agent_updates::{CliAgentUpdateChannel, CliAgentUpdateSource};
use serde_json::json;
use tempfile::TempDir;

const CODEX_CMD: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n";
const CLAUDE_CMD: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n";

fn installation(entry: PathBuf) -> Installation {
    Installation {
        stamp: stamp(&entry).unwrap(),
        entry,
        source: CliAgentUpdateSource::Unknown,
        manager: None,
        helper: None,
        registration: None,
        invocation: None,
        channel: CliAgentUpdateChannel::Latest,
        config: None,
        error: None,
        source_target: None,
    }
}

fn package(prefix: &Path, windows_layout: bool, agent: CLIAgent, version: &str) -> PathBuf {
    let root = prefix
        .join(if windows_layout {
            "node_modules"
        } else {
            "lib/node_modules"
        })
        .join(package_name(agent).unwrap());
    fs::create_dir_all(root.join("bin")).unwrap();
    let bin = if agent == CLIAgent::Codex {
        "bin/codex.js"
    } else {
        "bin/claude.exe"
    };
    fs::write(
        root.join("package.json"),
        serde_json::to_vec(&json!({
            "name": package_name(agent).unwrap(),
            "version": version,
            "bin": { agent.command_prefix(): bin },
        }))
        .unwrap(),
    )
    .unwrap();
    let bytes: &[u8] = if agent == CLIAgent::Codex {
        b"#!/usr/bin/env node\n"
    } else {
        b"MZnative"
    };
    fs::write(root.join(bin), bytes).unwrap();
    root.join(bin)
}

#[test]
fn exact_manifest_version_binds_the_registered_entry() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    let bin = package(&prefix, false, CLIAgent::Codex, "0.156.1");
    let entry = installation(bin);

    let registration = registered_installation(CLIAgent::Codex, &entry, "0.156.1", &prefix, false)
        .unwrap()
        .unwrap();
    assert_eq!(registration.prefix, prefix);
    assert_eq!(
        registration.package_root,
        prefix.join("lib/node_modules/@openai/codex")
    );
    assert!(
        registered_installation(CLIAgent::Codex, &entry, "0.155.1", &prefix, false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn another_global_prefix_does_not_own_the_active_entry() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let first_prefix = first.path().canonicalize().unwrap();
    let second_prefix = second.path().canonicalize().unwrap();
    let bin = package(&first_prefix, false, CLIAgent::Codex, "0.156.1");
    package(&second_prefix, false, CLIAgent::Codex, "0.156.1");

    assert!(
        registered_installation(
            CLIAgent::Codex,
            &installation(bin),
            "0.156.1",
            &second_prefix,
            false
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn absent_npm_prefix_leaves_other_installation_sources_discoverable() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    let bin = package(&prefix, false, CLIAgent::Codex, "0.156.1");
    assert!(
        registered_installation(
            CLIAgent::Codex,
            &installation(bin),
            "0.156.1",
            &prefix.join("absent-prefix"),
            false
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn official_windows_node_shim_binds_to_its_package() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    package(&prefix, true, CLIAgent::Codex, "0.156.1");
    let shim = prefix.join("codex.cmd");
    fs::write(&shim, CODEX_CMD).unwrap();

    assert!(
        registered_installation(
            CLIAgent::Codex,
            &installation(shim),
            "0.156.1",
            &prefix,
            true
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn official_windows_native_shim_binds_claude_2280() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    package(&prefix, true, CLIAgent::Claude, "2.1.280");
    let shim = prefix.join("claude.cmd");
    fs::write(&shim, CLAUDE_CMD).unwrap();

    assert!(
        registered_installation(
            CLIAgent::Claude,
            &installation(shim),
            "2.1.280",
            &prefix,
            true
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn extra_commands_in_a_windows_shim_do_not_establish_ownership() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    package(&prefix, true, CLIAgent::Codex, "0.156.1");
    let shim = prefix.join("codex.cmd");
    fs::write(&shim, format!("{CODEX_CMD}echo unexpected\r\n")).unwrap();

    assert!(
        registered_installation(
            CLIAgent::Codex,
            &installation(shim),
            "0.156.1",
            &prefix,
            true
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn copied_windows_shim_outside_the_prefix_is_not_owned() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    package(&prefix, true, CLIAgent::Codex, "0.156.1");
    fs::create_dir(prefix.join("other")).unwrap();
    let shim = prefix.join("other/codex.cmd");
    fs::write(&shim, CODEX_CMD).unwrap();

    assert!(
        registered_installation(
            CLIAgent::Codex,
            &installation(shim),
            "0.156.1",
            &prefix,
            true
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn package_bin_rejects_cross_platform_escape_paths() {
    assert_eq!(
        relative_bin(&json!({"bin": {"codex": "../../outside"}}), "codex"),
        None
    );
    assert_eq!(
        relative_bin(&json!({"bin": {"codex": "/outside"}}), "codex"),
        None
    );
    assert_eq!(
        relative_bin(&json!({"bin": {"codex": "C:\\outside"}}), "codex"),
        None
    );
    assert_eq!(
        relative_bin(&json!({"bin": {"codex": "bin\\..\\outside"}}), "codex"),
        None
    );
    assert_eq!(
        relative_bin(&json!({"bin": {"codex": "bin/%OTHER%"}}), "codex"),
        None
    );
    assert_eq!(
        relative_bin(&json!({"bin": {"codex": "./bin/codex.js"}}), "codex"),
        Some(PathBuf::from("bin/codex.js"))
    );
}

#[test]
fn same_named_npm_script_requires_its_manager_registration() {
    let directory = TempDir::new().unwrap();
    let root = directory.path().canonicalize().unwrap();
    fs::create_dir(root.join("bin")).unwrap();
    let cli = root.join("bin/npm-cli.js");
    fs::write(&cli, "#!/usr/bin/env node\n").unwrap();
    fs::write(
        root.join("package.json"),
        br#"{"name":"custom-manager","version":"11.12.1","bin":{"npm":"bin/npm-cli.js"}}"#,
    )
    .unwrap();
    assert_eq!(registered_manager(&cli), Ok(false));

    fs::write(
        root.join("package.json"),
        br#"{"name":"npm","version":"11.12.1","bin":{"npm":"bin/npm-cli.js"}}"#,
    )
    .unwrap();
    assert_eq!(registered_manager(&cli), Ok(true));
}

#[cfg(unix)]
#[test]
fn npm_link_outside_prefix_does_not_establish_global_ownership() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().unwrap();
    let prefix = directory.path().canonicalize().unwrap();
    let external = TempDir::new().unwrap();
    let bin = package(
        &external.path().canonicalize().unwrap(),
        false,
        CLIAgent::Codex,
        "0.156.1",
    );
    fs::create_dir_all(prefix.join("lib/node_modules/@openai")).unwrap();
    symlink(
        bin.parent().unwrap().parent().unwrap(),
        prefix.join("lib/node_modules/@openai/codex"),
    )
    .unwrap();

    assert!(
        registered_installation(
            CLIAgent::Codex,
            &installation(bin),
            "0.156.1",
            &prefix,
            false
        )
        .unwrap()
        .is_none()
    );
}

#[test]
#[ignore = "需要显式指定隔离 npm 前缀和独立收据路径，不安装或升级用户 CLI"]
fn real_npm_prefix_registration() {
    let prefix = PathBuf::from(std::env::var_os("INFINISHELL_NPM_PROBE_PREFIX").unwrap());
    let npm = PathBuf::from(std::env::var_os("INFINISHELL_NPM_PROBE_MANAGER").unwrap());
    let receipt = PathBuf::from(std::env::var_os("INFINISHELL_NPM_PROBE_RECEIPT").unwrap());
    assert_eq!(registered_manager(&npm), Ok(true));
    let mut cases = Vec::new();
    for (agent, version) in [(CLIAgent::Codex, "0.156.1"), (CLIAgent::Claude, "2.1.280")] {
        let name = agent.command_prefix();
        let entry = if cfg!(windows) {
            prefix.join(format!("{name}.cmd"))
        } else {
            prefix.join("bin").join(name)
        };
        let registration =
            registered_installation(agent, &installation(entry), version, &prefix, cfg!(windows))
                .unwrap()
                .unwrap();
        cases.push(json!({"agent": name, "version": version, "source": "npm", "manifest_sha256": format!("{:x}", Sha256::digest(fs::read(registration.manifest).unwrap())), "registered": true}));
    }
    fs::write(receipt, serde_json::to_vec_pretty(&json!({
        "schema": 1,
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "scope": "actual isolated npm prefix source registration only; no automatic update acceptance",
        "cases": cases,
        "source_sha256": format!("{:x}", Sha256::digest(include_bytes!("sources_npm.rs"))),
    })).unwrap()).unwrap();
}
