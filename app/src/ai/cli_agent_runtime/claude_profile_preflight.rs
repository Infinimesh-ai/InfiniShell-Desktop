//! 固定策略在真实任务进程启动前生成；预检不发送用户输入、不注册工具或 hooks。

use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use command::Stdio;
use command::r#async::Command;
use futures::io::{AsyncBufReadExt, AsyncWrite, BufReader};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::{MAX_LINE_BYTES, write_message};
use crate::ai::cli_agent_runtime::claude_profile::{
    ClaudeRestrictedFilesV1, FIXED_PERMISSION_MODE_ARGUMENT, ISOLATED_SETTINGS_ARGUMENTS, reject,
};
use crate::ai::cli_agent_runtime::{RuntimeError, SessionOptions, SessionTarget};

pub(super) async fn prepare(
    options: &SessionOptions,
) -> Result<ClaudeRestrictedFilesV1, RuntimeError> {
    if !options.selected_skills.is_empty() {
        return Err(reject("claude_profile_skills_unsupported"));
    }
    if options.target != SessionTarget::New && options.claude_profile.is_none() {
        return Err(reject("claude_profile_resume_missing"));
    }
    #[cfg(any(test, feature = "claude_21280_test_candidate"))]
    let candidate = super::test_candidate_enabled(options);
    #[cfg(not(any(test, feature = "claude_21280_test_candidate")))]
    let candidate = false;
    let digest = executable_digest(&options.executable, candidate)?;
    let arguments = [
        "--print",
        "--input-format=stream-json",
        "--output-format=stream-json",
        "--verbose",
        "--permission-prompt-tool=stdio",
        "--permission-prompts=host",
        "--tools=",
        "--disallowedTools=*",
        "--strict-mcp-config",
        "--mcp-config={\"mcpServers\":{}}",
        "--no-session-persistence",
    ]
    .into_iter()
    .chain(ISOLATED_SETTINGS_ARGUMENTS)
    .chain([FIXED_PERMISSION_MODE_ARGUMENT]);
    let mut child = Command::new(&options.executable)
        .args(arguments)
        .current_dir(&options.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| reject("claude_profile_preflight_stdin"))?;
    let mut stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| reject("claude_profile_preflight_stdout"))?,
    );
    let result = async {
        let initialize = query(&mut stdin, &mut stdout, "initialize").await?;
        let pid = initialize["pid"]
            .as_u64()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| reject("claude_profile_preflight_identity"))?;
        if initialize["session_state"] != "idle" {
            return Err(reject("claude_profile_preflight_busy"));
        }
        let settings = query(&mut stdin, &mut stdout, "get_settings").await?;
        let rules = query(&mut stdin, &mut stdout, "list_permission_rules").await?;
        let hooks = query(&mut stdin, &mut stdout, "get_hooks_listing").await?;
        let again = query(&mut stdin, &mut stdout, "initialize").await?;
        if again["pid"].as_u64() != Some(pid)
            || again["session_state"] != "idle"
            || again["current_permission_mode"] != initialize["current_permission_mode"]
        {
            return Err(reject("claude_profile_preflight_identity"));
        }
        let profile = ClaudeRestrictedFilesV1::compile(
            &options.cwd,
            digest,
            options.local_tools,
            &settings,
            &rules,
            &hooks,
        )?;
        if let Some(saved) = &options.claude_profile {
            saved.verify_source(&profile)?;
        }
        if let Some(ceiling) = &options.permission_ceiling {
            ceiling
                .claude_profile()
                .ok_or_else(|| reject("claude_profile_wrong_parent"))?
                .verify_source(&profile)?;
        }
        Ok(profile)
    }
    .await;
    drop(stdin);
    drop(stdout);
    let status = child
        .status()
        .with_timeout(Duration::from_secs(5))
        .await
        .map_err(|_| reject("claude_profile_preflight_exit_unconfirmed"))??;
    if !status.success() {
        return Err(reject("claude_profile_preflight_failed"));
    }
    result
}

async fn query(
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl futures::io::AsyncBufRead + Unpin),
    subtype: &str,
) -> Result<Value, RuntimeError> {
    let request_id = Uuid::new_v4().to_string();
    write_message(
        stdin,
        &json!({"type":"control_request","request_id":request_id,"request":{"subtype":subtype}}),
    )
    .await?;
    async {
        for _ in 0..64 {
            let mut line = String::new();
            if stdout.read_line(&mut line).await? == 0 || line.len() > MAX_LINE_BYTES {
                return Err(reject("claude_profile_preflight_stream"));
            }
            let message: Value =
                serde_json::from_str(&line).map_err(|_| reject("claude_profile_preflight_json"))?;
            if message["type"] == "system" {
                continue;
            }
            if message["type"] != "control_response"
                || message["response"]["request_id"] != request_id
                || message["response"]["subtype"] != "success"
            {
                return Err(reject("claude_profile_preflight_response"));
            }
            return Ok(message["response"]["response"].clone());
        }
        Err(reject("claude_profile_preflight_flood"))
    }
    .with_timeout(Duration::from_secs(5))
    .await
    .map_err(|_| reject("claude_profile_preflight_timeout"))?
}

fn executable_digest(path: &Path, candidate: bool) -> Result<String, RuntimeError> {
    let expected = expected_executable_digests(std::env::consts::OS, std::env::consts::ARCH)?;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    let actual = format!("{:x}", digest.finalize());
    #[cfg(any(test, feature = "claude_21280_test_candidate"))]
    let candidate_matches = candidate
        && test_candidate_executable_digest(std::env::consts::OS, std::env::consts::ARCH)
            == Some(actual.as_str());
    #[cfg(not(any(test, feature = "claude_21280_test_candidate")))]
    let candidate_matches = {
        let _ = candidate;
        false
    };
    if !expected.contains(&actual.as_str()) && !candidate_matches {
        return Err(reject("claude_profile_executable_unverified"));
    }
    Ok(actual)
}

#[cfg(any(test, feature = "claude_21280_test_candidate"))]
pub(super) fn test_candidate_executable_digest(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => {
            Some("387a5c5dcdbb815085edf0baf79591f9d8894efe922bceaf3d75b1b08055229d")
        }
        ("macos", "x86_64") => {
            Some("c1d32d87630482250633208ab77855429b24010ae3086a7ff7539b57b93168d4")
        }
        ("linux", "x86_64") => {
            Some("1e08503dbdf3c2cb0d706d32f3408277388d1c76ef108673e8fe42c1b322925b")
        }
        ("windows", "x86_64") => {
            Some("0e4195524b73eb77efbdf3e2b36de5322a29f0ca575dfd2d9b4f946b1d425469")
        }
        _ => None,
    }
}

fn expected_executable_digests(os: &str, arch: &str) -> Result<[&'static str; 2], RuntimeError> {
    // 每个平台仅绑定已核对官方清单的 273/278；不代表各平台任务链已验收。
    Ok(match (os, arch) {
        ("macos", "aarch64") => [
            "953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb",
            "bd245662fb8a0e321b3bf133e930371d6563c387527885f30b2613aef3ba14d6",
        ],
        ("macos", "x86_64") => [
            "2030ecf911e301e778b3c5a49068d6751384c830e48ea14f11eb61dd23622cee",
            "c522425e3d42275d2ac2238757ef8ba7f80d165a934044ec5a7a5fd7d7b9950b",
        ],
        ("linux", "x86_64") => [
            "6c752e2cc7c110c9df15f26d8d134d438c5ae95dbd610efc1a308bf7f9c5f6c1",
            "5c4735937844e84f8a93306e841a5b0e12252909b07870f789b190468da147ab",
        ],
        ("windows", "x86_64") => [
            "19654006672b6da7c945115eea99ca10051796016df563a65b3f0c7d72720ef0",
            "006ea5c8638f67f10a5ae66bb232fd267c9f6af294e3f03f4cfcf1fd3f2cced8",
        ],
        _ => return Err(reject("claude_profile_platform_unverified")),
    })
}

#[cfg(test)]
#[path = "claude_profile_preflight_tests.rs"]
mod tests;
