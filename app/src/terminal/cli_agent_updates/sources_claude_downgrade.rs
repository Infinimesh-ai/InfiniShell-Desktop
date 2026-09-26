//! Claude 消费者 Stable 的固定主动降级合同，独立于开发验收基线。
//! 当前官方指针必须再次查询；存在这个合同不表示 Stable 正在发布该版本。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{CLIAgent, Channel, ConfigBackup, Error, Source};

pub(super) const FROM: &str = "2.1.280";
pub(super) const TO: &str = "2.1.278";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Intent {
    ClaudeNpmStable21280To21278,
}

pub(super) fn platform() -> Result<&'static str, Error> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        _ => Err(Error::UnsupportedPlatform),
    }
}

pub(super) fn select(
    agent: CLIAgent,
    source: Source,
    installed: &str,
    target: &str,
    selected: Channel,
) -> Result<Option<Intent>, Error> {
    if agent != CLIAgent::Claude || source != Source::Npm || target != TO {
        return Ok(None);
    }
    platform()?;
    if installed == target {
        return Ok(None);
    }
    if installed != FROM {
        return Err(Error::InvalidRelease);
    }
    if selected != Channel::Stable {
        return Err(Error::ChannelMismatch);
    }
    Ok(Some(Intent::ClaudeNpmStable21280To21278))
}

pub(super) fn validate(
    intent: Option<Intent>,
    installed: &str,
    target: &str,
    config: &Option<ConfigBackup>,
) -> Result<(), Error> {
    if target != TO {
        return if intent.is_none() {
            Ok(())
        } else {
            Err(Error::RecoveryRequired)
        };
    }
    platform()?;
    if installed == target && intent.is_none() {
        return Ok(());
    }
    if installed != FROM || intent != Some(Intent::ClaudeNpmStable21280To21278) {
        return Err(Error::RecoveryRequired);
    }
    let desired: Value = config
        .as_ref()
        .and_then(|config| config.publication_bytes(true))
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .ok_or(Error::ChannelMismatch)?;
    if desired["autoUpdatesChannel"] != "stable" {
        return Err(Error::ChannelMismatch);
    }
    Ok(())
}

pub(super) async fn revalidate(intent: Option<Intent>) -> Result<(), Error> {
    if intent.is_some()
        && super::latest(
            CLIAgent::Claude,
            Channel::Stable,
            &http_client::Client::new(),
        )
        .await?
            != TO
    {
        return Err(Error::ChannelMismatch);
    }
    Ok(())
}

#[cfg(feature = "local_fs")]
pub(super) fn compatible_history<'a>(
    tasks: impl Iterator<Item = &'a crate::persistence::model::LocalCliTask>,
) -> bool {
    use crate::ai::cli_agent_runtime::permissions::ClaudeFileProfile;
    use crate::ai::cli_agent_runtime::{PermissionPolicy, coordinator};
    use crate::persistence::model::LocalCliTaskState;

    tasks.filter(|task| task.harness == "claude").all(|task| {
        // 未创建原生会话的失败任务没有可继续历史；未知或仍活跃记录由忙碌门禁保留。
        if task.state.is_active() || task.state == LocalCliTaskState::Unknown {
            return false;
        }
        if task.native_session_id.is_none() {
            return task.state.is_terminal();
        }
        if task
            .native_session_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_none_or(|id| id.is_nil())
        {
            return false;
        }
        let Ok(config) = serde_json::from_str::<Value>(&task.config_json) else {
            return false;
        };
        let generation = config["runtime_generation"]
            .as_str()
            .and_then(|text| Uuid::parse_str(text).ok());
        if config["cli_version"] != TO
            || generation.is_none_or(|id| id.is_nil())
            || config["cli_version_runtime_generation"] != config["runtime_generation"]
            || coordinator::verify_recovery_identity(task).is_err()
            || !config["claude_pending_input"].is_null()
            || config.get("selected_skills").is_some_and(|skills| {
                !skills.is_null() && !skills.as_array().is_some_and(Vec::is_empty)
            })
        {
            return false;
        }
        let Ok(policy) =
            serde_json::from_value::<PermissionPolicy>(config["permission_policy"].clone())
        else {
            return false;
        };
        match policy {
            PermissionPolicy::Inherit => [
                "claude_profile",
                "grok_profile",
                "permission_ceiling",
                "local_tools",
            ]
            .iter()
            .all(|key| config[*key].is_null()),
            PermissionPolicy::ClaudeRestrictedFilesV1 => {
                let Ok(profile) =
                    serde_json::from_value::<ClaudeFileProfile>(config["claude_profile"].clone())
                else {
                    return false;
                };
                profile.policy() == PermissionPolicy::ClaudeRestrictedFilesV1
                    && profile.validate(Path::new(&task.working_directory)).is_ok()
                    && native_digest()
                        .is_ok_and(|digest| config["claude_profile"]["executableSha256"] == digest)
            }
            PermissionPolicy::ReadOnly
            | PermissionPolicy::WorkspaceWrite
            | PermissionPolicy::ClaudeRestrictedFilesV2
            | PermissionPolicy::ClaudeRestrictedFilesV3
            | PermissionPolicy::ClaudeRestrictedSkillsV1
            | PermissionPolicy::ClaudeReviewedCommandsV1
            | PermissionPolicy::ClaudeReviewedCommandsSkillsV1
            | PermissionPolicy::GrokRestrictedReadV1
            | PermissionPolicy::GrokRestrictedFilesV1
            | PermissionPolicy::GrokRestrictedFilesV2
            | PermissionPolicy::GrokRestrictedSkillsV1
            | PermissionPolicy::GrokReviewedCommandsV1
            | PermissionPolicy::GrokReviewedCommandsSkillsV1 => false,
        }
    })
}

fn native_digest() -> Result<&'static str, Error> {
    files(platform()?)?
        .get(Path::new("claude"))
        .map(|(_, sha, _)| *sha)
        .ok_or(Error::InvalidRelease)
}

#[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
pub(super) fn verify_metadata(platform: &str, wrapper: &[u8], native: &[u8]) -> Result<(), Error> {
    let integrity = match platform {
        "darwin-arm64" => {
            "sha512-Jgl//CpT1KR1N8uxAN0CmkFu3eESU7GYzPOwdGH1pCENysJqYgo2LwsIrTol+EequNMrTG3kkrNW9mIoS8eWLg=="
        }
        "linux-x64" => {
            "sha512-q3r+5aLGAet1MGMkCH2xPsuIW9A40ws4zftURxhYwDenheCKvXc7Gr1jBwvEhYG8uQwgY3YsfNwnvIsh1Bjmeg=="
        }
        _ => return Err(Error::UnsupportedPlatform),
    };
    for (bytes, name, expected) in [
        (
            wrapper,
            "@anthropic-ai/claude-code".to_owned(),
            "sha512-mfNRqC0GaEXqmP97NiwJBeYBmRuqe2VzgLUreUUaEhyJxJWx2Z6ClW1tBOncGNNXdhj6EY4LPvUIWP+oq311CA==",
        ),
        (
            native,
            format!("@anthropic-ai/claude-code-{platform}"),
            integrity,
        ),
    ] {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidRelease)?;
        let short = name
            .strip_prefix("@anthropic-ai/")
            .ok_or(Error::InvalidRelease)?;
        if value["name"] != name
            || value["version"] != TO
            || value["dist"]["integrity"] != expected
            || value["dist"]["tarball"]
                != format!("https://registry.npmjs.org/{name}/-/{short}-{TO}.tgz")
        {
            return Err(Error::InvalidRelease);
        }
    }
    Ok(())
}

#[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
pub(super) fn verify_archives(
    platform: &str,
    wrapper: &super::npm_release::VerifiedNpmArchive,
    native: &super::npm_release::VerifiedNpmArchive,
) -> Result<(), Error> {
    for (archive, contract) in [(wrapper, files("wrapper")?), (native, files(platform)?)] {
        if archive.files.len() != contract.len() {
            return Err(Error::InvalidRelease);
        }
        for (path, (length, sha, mode)) in contract {
            let file = archive.files.get(&path).ok_or(Error::InvalidRelease)?;
            if file.length != length
                || file.sha256 != super::brew::decode_sha256(sha)?
                || file.executable != (mode & 0o111 != 0)
            {
                return Err(Error::InvalidRelease);
            }
        }
    }
    Ok(())
}

fn files(platform: &str) -> Result<BTreeMap<PathBuf, (u64, &'static str, u32)>, Error> {
    let files: &[(&str, u64, &str, u32)] = match platform {
        "wrapper" => &[
            (
                "cli-wrapper.cjs",
                4997,
                "61ad63033d9c8155d5e60a29f45dc4665afa07631c0b108e62cc83bf45ba490e",
                0o644,
            ),
            (
                "install.cjs",
                7196,
                "5cbab1670597f492cd4eeb946f3c344ebcb1fbd43c623ba192c9b33744461b85",
                0o644,
            ),
            (
                "bin/claude.exe",
                500,
                "6d7abae055d3b598281300a6c835086dec81bf3048f8a2294c5d3e50c8830d7b",
                0o644,
            ),
            (
                "package.json",
                1476,
                "0ddbbc1481de3a7437762c9b7414fde846f2e1b17cd1d291f3551b42660039dc",
                0o644,
            ),
            (
                "LICENSE.md",
                147,
                "8ce94b9478bb9868f9641f818e06cd722fbe55d4c22e2d2ed11971b20146173a",
                0o644,
            ),
            (
                "README.md",
                2037,
                "da7cf15ce4e35bad6a107acd6a36b4fe052083068bb9e564b1aa3f2078b5d0ce",
                0o644,
            ),
            (
                "sdk-tools.d.ts",
                167766,
                "4fdbd64f76e190800f48e93ddf4edcf6f3c87187396b586a75de808a78d8c1c4",
                0o644,
            ),
        ],
        "darwin-arm64" => &[
            (
                "claude",
                217695408,
                "bd245662fb8a0e321b3bf133e930371d6563c387527885f30b2613aef3ba14d6",
                0o755,
            ),
            (
                "package.json",
                277,
                "da56572a9cfa0bc1a9e62c5cacbfca47449c0ccc17a1a17ef3b84a09f318eb30",
                0o644,
            ),
            (
                "LICENSE.md",
                147,
                "8ce94b9478bb9868f9641f818e06cd722fbe55d4c22e2d2ed11971b20146173a",
                0o644,
            ),
            (
                "README.md",
                153,
                "26170f86550171a90850a39d867b0487d0d218e100ab5552e51ba4f1159b9a0c",
                0o644,
            ),
        ],
        "linux-x64" => &[
            (
                "claude",
                234119480,
                "5c4735937844e84f8a93306e841a5b0e12252909b07870f789b190468da147ab",
                0o755,
            ),
            (
                "package.json",
                289,
                "582dd41846f79c046e96d018872a38f07fc3c20443a7f4e35ec78458359f494f",
                0o644,
            ),
            (
                "LICENSE.md",
                147,
                "8ce94b9478bb9868f9641f818e06cd722fbe55d4c22e2d2ed11971b20146173a",
                0o644,
            ),
            (
                "README.md",
                150,
                "ab4b9d72c3aa1ac7357e8eeb8b77c62e308e9af01ac0ecdb181f3bf1e57e25d5",
                0o644,
            ),
        ],
        _ => return Err(Error::UnsupportedPlatform),
    };
    Ok(files
        .iter()
        .map(|(path, size, sha, mode)| (PathBuf::from(path), (*size, *sha, *mode)))
        .collect())
}
