//! 子任务继承已验证的原生权限或创建时固定的工具策略，不将 CLI 模式名称映射成沙箱。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::RuntimeError;
pub use super::claude_profile::ClaudeRestrictedFilesV1;
#[cfg(feature = "local_fs")]
use crate::persistence::model::LocalCliTask;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentPermissionCeiling {
    parent_task_id: String,
    parent_generation: i64,
    parent_native_session_id: String,
    working_directory: PathBuf,
    permissions: NativePermissions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum NativePermissions {
    Codex(CodexPermissions),
    Claude(ClaudePermissionProof),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClaudePermissionProof {
    claude_restricted_files_v1: ClaudeRestrictedFilesV1,
}

impl ParentPermissionCeiling {
    pub(crate) fn claude_profile(&self) -> Option<&ClaudeRestrictedFilesV1> {
        match &self.permissions {
            NativePermissions::Claude(proof) => Some(&proof.claude_restricted_files_v1),
            NativePermissions::Codex(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CodexPermissions {
    approval_policy: String,
    approvals_reviewer: String,
    sandbox: CodexSandbox,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields, rename_all = "camelCase")]
enum CodexSandbox {
    ReadOnly {
        #[serde(rename = "networkAccess")]
        network_access: bool,
    },
    WorkspaceWrite {
        #[serde(rename = "writableRoots")]
        writable_roots: Vec<PathBuf>,
        #[serde(rename = "networkAccess")]
        network_access: bool,
        #[serde(rename = "excludeTmpdirEnvVar")]
        exclude_tmpdir_env_var: bool,
        #[serde(rename = "excludeSlashTmp")]
        exclude_slash_tmp: bool,
    },
}

fn parse_permissions(value: &Value) -> Option<CodexPermissions> {
    let permissions: CodexPermissions = serde_json::from_value(value.clone()).ok()?;
    if !matches!(
        permissions.approval_policy.as_str(),
        "never" | "untrusted" | "on-request"
    ) || permissions.approvals_reviewer != "user"
        || matches!(&permissions.sandbox, CodexSandbox::WorkspaceWrite { writable_roots, .. }
            if writable_roots.iter().any(|path| !path.is_absolute()))
    {
        return None;
    }
    Some(permissions)
}

/// 调用方必须传入 SQLite 已提交的父记录；恢复时读取创建子任务时的固定父代。
#[cfg(feature = "local_fs")]
pub(crate) fn ceiling_from_parent(
    parent: &LocalCliTask,
    child_harness: &str,
) -> Result<ParentPermissionCeiling, RuntimeError> {
    let config: Value = serde_json::from_str(&parent.config_json).unwrap_or(Value::Null);
    let observed = config
        .get("effective_permissions")
        .cloned()
        .unwrap_or(Value::Null);
    let reject = || rejected(None, &observed, "parent_permissions_unverifiable", false);
    if parent.harness != child_harness
        || !matches!(parent.harness.as_str(), "codex" | "claude")
        || parent.generation < 1
        || parent.task_id.is_empty()
        || !Path::new(&parent.working_directory).is_absolute()
    {
        return Err(reject());
    }
    let native_id = parent
        .native_session_id
        .as_ref()
        .filter(|id| !id.is_empty())
        .ok_or_else(reject)?;
    let permissions = match parent.harness.as_str() {
        "codex" if config.get("cli_version").and_then(Value::as_str) == Some("0.147.0") => {
            NativePermissions::Codex(parse_permissions(&observed).ok_or_else(reject)?)
        }
        "claude"
            if config.get("cli_version").and_then(Value::as_str) == Some("2.1.273")
                && config["permission_policy"] == "ClaudeRestrictedFilesV1"
                && observed["fixedProfileVerified"] == true
                && observed["permissionMode"] == "plan"
                && config["claude_profile"] == observed["claudeRestrictedFilesV1"] =>
        {
            let profile: ClaudeRestrictedFilesV1 =
                serde_json::from_value(observed["claudeRestrictedFilesV1"].clone())
                    .map_err(|_| reject())?;
            profile.validate(Path::new(&parent.working_directory))?;
            NativePermissions::Claude(ClaudePermissionProof {
                claude_restricted_files_v1: profile,
            })
        }
        _ => return Err(reject()),
    };
    Ok(ParentPermissionCeiling {
        parent_task_id: parent.task_id.clone(),
        parent_generation: parent.generation,
        parent_native_session_id: native_id.clone(),
        working_directory: parent.working_directory.clone().into(),
        permissions,
    })
}

#[cfg(feature = "local_fs")]
pub(crate) fn verify_parent_binding(
    ceiling: Option<&ParentPermissionCeiling>,
    parent: &LocalCliTask,
    child: &LocalCliTask,
) -> Result<(), RuntimeError> {
    let expected = ceiling_from_parent(parent, &child.harness)?;
    if child.parent_task_id.as_deref() != Some(parent.task_id.as_str())
        || child.parent_generation != Some(parent.generation)
        || ceiling != Some(&expected)
    {
        return Err(rejected(
            ceiling,
            &json!(expected),
            "parent_binding_mismatch",
            false,
        ));
    }
    Ok(())
}

/// 在 SessionReady 前核验；尚未发送初始输入，配置漂移不会触发模型或工具执行。
pub(crate) fn verify_effective_permissions(
    ceiling: Option<&ParentPermissionCeiling>,
    harness: &str,
    cwd: &Path,
    actual: &Value,
) -> Result<(), RuntimeError> {
    let Some(ceiling) = ceiling else {
        return Ok(());
    };
    if cwd != ceiling.working_directory {
        return Err(rejected(
            Some(ceiling),
            actual,
            "harness_or_directory_mismatch",
            false,
        ));
    }
    match &ceiling.permissions {
        NativePermissions::Codex(expected) if harness == "codex" => {
            let actual_permissions = parse_permissions(actual).ok_or_else(|| {
                rejected(
                    Some(ceiling),
                    actual,
                    "child_permissions_unverifiable",
                    false,
                )
            })?;
            if actual_permissions != *expected {
                return Err(rejected(
                    Some(ceiling),
                    actual,
                    "effective_permissions_changed",
                    true,
                ));
            }
        }
        NativePermissions::Claude(expected) if harness == "claude" => {
            let profile: ClaudeRestrictedFilesV1 = serde_json::from_value(
                actual["claudeRestrictedFilesV1"].clone(),
            )
            .map_err(|_| {
                rejected(
                    Some(ceiling),
                    actual,
                    "child_permissions_unverifiable",
                    false,
                )
            })?;
            if actual["fixedProfileVerified"] != true
                || actual["permissionMode"] != "plan"
                || !expected.claude_restricted_files_v1.same_scope(&profile)
            {
                return Err(rejected(
                    Some(ceiling),
                    actual,
                    "effective_permissions_changed",
                    true,
                ));
            }
        }
        NativePermissions::Codex(_) | NativePermissions::Claude(_) => {
            return Err(rejected(
                Some(ceiling),
                actual,
                "harness_or_directory_mismatch",
                false,
            ));
        }
    }
    Ok(())
}

pub(crate) fn rejected(
    ceiling: Option<&ParentPermissionCeiling>,
    actual: &Value,
    reason: &str,
    mismatch: bool,
) -> RuntimeError {
    RuntimeError::PermissionCeilingRejected {
        message: if mismatch {
            crate::t!("cli-agent-task-permission-ceiling-mismatch")
        } else {
            crate::t!("cli-agent-task-permission-ceiling-unavailable")
        },
        details: json!({"source":"parent_permission_ceiling","reason":reason,
            "expected":ceiling,"actual":actual,"task_input_sent":false}),
    }
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
