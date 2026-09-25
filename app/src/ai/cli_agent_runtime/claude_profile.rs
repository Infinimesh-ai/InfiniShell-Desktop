//! 创建时固定的 Claude 文件工具策略；不把模式或观察快照称为操作系统沙箱。

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::RuntimeError;
use super::local_tools::{LocalToolPermissions, MCP_SERVER_NAME, tool_definitions};

const BLOCKED_TOOLS: [&str; 10] = [
    "Bash",
    "Agent",
    "EnterPlanMode",
    "ExitPlanMode",
    "Skill",
    "Write",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
    "Computer",
];

// 受限模式保留默认账户认证；空设置来源隔离用户、项目与本地设置，管理员策略仍由 CLI 强制应用。
pub(super) const ISOLATED_SETTINGS_ARGUMENTS: [&str; 2] = ["--restricted", "--setting-sources="];
pub(super) const FIXED_PERMISSION_MODE_ARGUMENT: &str = "--permission-mode=manual";
pub(super) const FIXED_PROTOCOL_PERMISSION_MODE: &str = "default";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClaudeRestrictedFilesV1 {
    version: u32,
    working_directory: PathBuf,
    canonical_working_directory: PathBuf,
    executable_sha256: String,
    deny_rules: Vec<String>,
    source_rules: Vec<SourceRule>,
    #[serde(deserialize_with = "deserialize_local_tools")]
    local_tools: Option<LocalToolPermissions>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SourceRule {
    source: String,
    behavior: String,
    rule: String,
}

fn deserialize_local_tools<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<LocalToolPermissions>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| D::Error::custom("invalid local tool permissions"))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "allow_spawn" | "allow_message"))
    {
        return Err(D::Error::custom("unknown local tool permission"));
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(D::Error::custom)
}

pub(super) fn reject(reason: &str) -> RuntimeError {
    let mut error = super::permissions::rejected(
        None,
        &json!({"profile":"ClaudeRestrictedFilesV1"}),
        reason,
        false,
    );
    if let RuntimeError::PermissionCeilingRejected { message, details } = &mut error {
        *message = crate::t!("cli-agent-claude-file-policy-unverified");
        details["task_input_sent"] = Value::Null;
    }
    error
}

impl ClaudeRestrictedFilesV1 {
    pub(super) fn compile(
        cwd: &Path,
        executable_sha256: String,
        local_tools: Option<LocalToolPermissions>,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
    ) -> Result<Self, RuntimeError> {
        validate_settings(settings, false)?;
        validate_hooks(hooks)?;
        let canonical_working_directory = std::fs::canonicalize(cwd)
            .map_err(|_| reject("claude_profile_directory_unavailable"))?;
        validate_rule_scope(rules, cwd, &canonical_working_directory)?;
        let mut source_rules = Vec::new();
        let mut deny_rules = BTreeSet::new();
        for row in rules["state"]["rules"]
            .as_array()
            .ok_or_else(|| reject("claude_profile_rules_shape"))?
        {
            let source = row["source"]
                .as_str()
                .ok_or_else(|| reject("claude_profile_rule_source"))?;
            if source == "toolsNarrowing"
                || (source == "cliArg" && row["behavior"] == "deny" && row["rule"] == "*")
            {
                continue;
            }
            if !matches!(source, "userSettings" | "projectSettings" | "localSettings")
                || row["notInEffect"] == true
            {
                return Err(reject("claude_profile_rule_source_unverified"));
            }
            let behavior = row["behavior"]
                .as_str()
                .ok_or_else(|| reject("claude_profile_rule_behavior"))?;
            let rule = row["rule"]
                .as_str()
                .ok_or_else(|| reject("claude_profile_rule_shape"))?;
            let tool = rule.split('(').next().unwrap_or(rule);
            if !matches!(behavior, "allow" | "deny" | "ask") {
                return Err(reject("claude_profile_rule_behavior"));
            }
            if behavior == "ask"
                && (tool == "Read"
                    || (tool.contains(['*', '?', '[', ']', '!']) && !tool.starts_with("mcp__")))
            {
                return Err(reject("claude_profile_read_ask_unsupported"));
            }
            if behavior == "deny"
                && tool != "*"
                && !tool.starts_with("mcp__")
                && tool.contains(['*', '?', '[', ']', '!'])
            {
                return Err(reject("claude_profile_tool_pattern_unsupported"));
            }
            if behavior == "deny"
                && (matches!(tool, "*" | "Read" | "Edit") || tool.starts_with("mcp__"))
            {
                validate_deny(rule)?;
                deny_rules.insert(rule.to_owned());
            }
            // 被移除的工具不会因忽略其允许规则而重新出现；原规则仍保存用于漂移核验。
            source_rules.push(SourceRule {
                source: source.into(),
                behavior: behavior.into(),
                rule: rule.into(),
            });
        }
        let mut expected_rules = BTreeSet::new();
        for source in settings["sources"]
            .as_array()
            .ok_or_else(|| reject("claude_profile_settings_shape"))?
        {
            let name = source["source"]
                .as_str()
                .ok_or_else(|| reject("claude_profile_settings_source"))?;
            for behavior in ["allow", "deny", "ask"] {
                if let Some(rows) = source["settings"]["permissions"][behavior].as_array() {
                    for rule in rows {
                        expected_rules.insert(SourceRule {
                            source: name.into(),
                            behavior: behavior.into(),
                            rule: rule
                                .as_str()
                                .ok_or_else(|| reject("claude_profile_rule_shape"))?
                                .into(),
                        });
                    }
                }
            }
        }
        if source_rules.iter().cloned().collect::<BTreeSet<_>>() != expected_rules {
            return Err(reject("claude_profile_sources_live_mismatch"));
        }
        source_rules.sort();
        source_rules.dedup();
        let profile = Self {
            version: 1,
            working_directory: cwd.to_owned(),
            canonical_working_directory,
            executable_sha256,
            deny_rules: deny_rules.into_iter().collect(),
            source_rules,
            local_tools,
        };
        profile.validate(cwd)?;
        Ok(profile)
    }

    pub(super) fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        if self.version != 1
            || self.working_directory != cwd
            || !cwd.is_absolute()
            || !self.canonical_working_directory.is_absolute()
            || std::fs::canonicalize(cwd).ok().as_ref() != Some(&self.canonical_working_directory)
            || self.executable_sha256.len() != 64
            || !self
                .executable_sha256
                .bytes()
                .all(|value| value.is_ascii_hexdigit())
        {
            return Err(reject("claude_profile_identity_invalid"));
        }
        if !self.deny_rules.is_empty() || !self.source_rules.is_empty() {
            return Err(reject("claude_profile_legacy_settings_source"));
        }
        for rule in &self.deny_rules {
            validate_deny(rule)?;
        }
        for row in &self.source_rules {
            let tool = row.rule.split('(').next().unwrap_or(&row.rule);
            if !matches!(
                row.source.as_str(),
                "userSettings" | "projectSettings" | "localSettings"
            ) || !matches!(row.behavior.as_str(), "allow" | "deny" | "ask")
                || row.rule.is_empty()
                || row.rule.chars().any(char::is_control)
                || (row.behavior == "ask"
                    && (tool == "Read"
                        || (tool.contains(['*', '?', '[', ']', '!'])
                            && !tool.starts_with("mcp__"))))
                || (row.behavior == "deny"
                    && tool != "*"
                    && !tool.starts_with("mcp__")
                    && tool.contains(['*', '?', '[', ']', '!']))
            {
                return Err(reject("claude_profile_saved_source_unsupported"));
            }
        }
        let expected = self
            .source_rules
            .iter()
            .filter(|row| {
                row.behavior == "deny"
                    && (matches!(row.rule.split('(').next(), Some("*" | "Read" | "Edit"))
                        || row.rule.starts_with("mcp__"))
            })
            .map(|row| row.rule.clone())
            .collect::<BTreeSet<_>>();
        if expected != self.deny_rules.iter().cloned().collect::<BTreeSet<_>>() {
            return Err(reject("claude_profile_deny_source_mismatch"));
        }
        Ok(())
    }

    pub(super) fn verify_source(&self, observed: &Self) -> Result<(), RuntimeError> {
        if self != observed {
            return Err(reject("claude_profile_source_changed"));
        }
        Ok(())
    }

    fn fixed_settings(&self) -> Value {
        let mut ask = vec!["Edit".to_owned()];
        if let Some(permissions) = self.local_tools {
            for tool in tool_definitions(permissions.allow_spawn, permissions.allow_message) {
                if let Some(name) = tool["name"].as_str() {
                    ask.push(format!("mcp__{MCP_SERVER_NAME}__{name}"));
                }
            }
        }
        json!({"permissions":{"ask":ask},"sandbox":{"enabled":false}})
    }

    pub(super) fn arguments(&self) -> Vec<String> {
        self.arguments_for("Read,Edit", &BLOCKED_TOOLS, self.fixed_settings())
    }

    fn arguments_for(&self, tools: &str, blocked: &[&str], settings: Value) -> Vec<String> {
        let mut arguments = ISOLATED_SETTINGS_ARGUMENTS
            .into_iter()
            .map(str::to_owned)
            .chain([
                FIXED_PERMISSION_MODE_ARGUMENT.into(),
                format!("--tools={tools}"),
                "--strict-mcp-config".into(),
                "--disable-slash-commands".into(),
                format!("--settings={settings}"),
            ])
            .collect::<Vec<_>>();
        // 该变长参数只出现一次，避免后一个选项覆盖前一个拒绝列表。
        arguments.push("--disallowedTools".into());
        arguments.extend(blocked.iter().map(|tool| (*tool).to_owned()));
        arguments.extend(self.deny_rules.iter().cloned());
        if self.local_tools.is_none() {
            arguments.push("mcp__*".into());
            arguments.push("--mcp-config={\"mcpServers\":{}}".into());
        }
        arguments
    }

    pub(super) fn verify_live(
        &self,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
        mcp: &Value,
    ) -> Result<(), RuntimeError> {
        self.verify_live_for(
            settings,
            rules,
            hooks,
            mcp,
            &self.fixed_settings(),
            &BLOCKED_TOOLS,
        )
    }

    fn verify_live_for(
        &self,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
        mcp: &Value,
        expected_settings: &Value,
        blocked: &[&str],
    ) -> Result<(), RuntimeError> {
        validate_settings(settings, true)?;
        if settings["effective"] != *expected_settings {
            return Err(reject("claude_profile_fixed_settings_changed"));
        }
        validate_hooks(hooks)?;
        validate_rule_scope(
            rules,
            &self.working_directory,
            &self.canonical_working_directory,
        )?;
        let rows = rules["state"]["rules"]
            .as_array()
            .ok_or_else(|| reject("claude_profile_rules_shape"))?;
        for expected in blocked
            .iter()
            .copied()
            .chain(self.deny_rules.iter().map(String::as_str))
            .chain(self.local_tools.is_none().then_some("mcp__*"))
        {
            if !rows.iter().any(|row| {
                row["source"] == "cliArg"
                    && row["behavior"] == "deny"
                    && row["rule"] == expected
                    && row["notInEffect"] != true
            }) {
                return Err(reject("claude_profile_cli_deny_missing"));
            }
        }
        let servers = mcp["mcpServers"]
            .as_array()
            .ok_or_else(|| reject("claude_profile_mcp_shape"))?;
        match self.local_tools {
            None if servers.is_empty() => Ok(()),
            Some(permissions) if servers.len() == 1 => {
                let server = &servers[0];
                if server["name"] != MCP_SERVER_NAME
                    || server["status"] != "connected"
                    || server["scope"] != "dynamic"
                    || server["serverInfo"]["name"] != MCP_SERVER_NAME
                    || server["serverInfo"]["version"] != "0.1.0"
                {
                    return Err(reject("claude_profile_mcp_identity"));
                }
                let actual = server["tools"]
                    .as_array()
                    .ok_or_else(|| reject("claude_profile_mcp_tools"))?
                    .iter()
                    .filter_map(|tool| tool["name"].as_str())
                    .collect::<BTreeSet<_>>();
                let definitions =
                    tool_definitions(permissions.allow_spawn, permissions.allow_message);
                let expected = definitions
                    .iter()
                    .filter_map(|tool| tool["name"].as_str())
                    .collect::<BTreeSet<_>>();
                if actual != expected
                    || server["tools"]
                        .as_array()
                        .is_none_or(|tools| tools.len() != actual.len())
                {
                    return Err(reject("claude_profile_mcp_tools"));
                }
                Ok(())
            }
            None | Some(_) => Err(reject("claude_profile_mcp_unverified")),
        }
    }

    pub(super) fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        if self.deny_rules.iter().any(|rule| rule == "*") {
            return false;
        }
        if let Some(permissions) = self.local_tools {
            let allowed = tool_definitions(permissions.allow_spawn, permissions.allow_message)
                .iter()
                .any(|definition| {
                    definition["name"]
                        .as_str()
                        .is_some_and(|name| tool == format!("mcp__{MCP_SERVER_NAME}__{name}"))
                });
            if allowed {
                return !self.deny_rules.iter().any(|rule| mcp_denied(rule, tool));
            }
        }
        if !matches!(tool, "Read" | "Edit") {
            return false;
        }
        let Some(value) = input["file_path"].as_str() else {
            return false;
        };
        let path = Path::new(value);
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return false;
        }
        let resolved = match std::fs::canonicalize(path) {
            Ok(path) => path,
            Err(_) => {
                let Some(parent) = path
                    .parent()
                    .and_then(|parent| std::fs::canonicalize(parent).ok())
                else {
                    return false;
                };
                let Some(name) = path.file_name() else {
                    return false;
                };
                parent.join(name)
            }
        };
        // 创建时保存的规范目录处理 Windows 扩展前缀和目录别名，不在审批时移动授权根。
        if !resolved.starts_with(&self.canonical_working_directory) {
            return false;
        }
        // 配置、会话与版本控制目录不能由模型改写，否则会改变下一进程的权限输入。
        if resolved
            .strip_prefix(&self.canonical_working_directory)
            .ok()
            .is_some_and(|relative| {
                relative.components().any(|part| {
                    part.as_os_str().to_str().is_some_and(|name| {
                        [".claude", ".git", ".mcp.json"]
                            .iter()
                            .any(|protected| name.eq_ignore_ascii_case(protected))
                    })
                })
            })
        {
            return false;
        }
        !self.deny_rules.iter().any(|rule| {
            rule == tool
                || rule == &format!("{tool}(/{})", path.display())
                || rule == &format!("{tool}(/{})", resolved.display())
        })
    }

    pub(super) fn verify_system_init(&self, message: &Value) -> Result<(), RuntimeError> {
        self.verify_system_init_for(message, &["Read", "Edit", "EndConversation"])
    }

    fn verify_system_init_for(&self, message: &Value, tools: &[&str]) -> Result<(), RuntimeError> {
        if message["permissionMode"] != FIXED_PROTOCOL_PERMISSION_MODE {
            return Err(reject("claude_profile_mode_changed"));
        }
        let mut allowed = tools
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect::<BTreeSet<_>>();
        if let Some(permissions) = self.local_tools {
            for tool in tool_definitions(permissions.allow_spawn, permissions.allow_message) {
                let name = tool["name"]
                    .as_str()
                    .ok_or_else(|| reject("claude_profile_mcp_tools"))?;
                allowed.insert(format!("mcp__{MCP_SERVER_NAME}__{name}"));
            }
        }
        if !message["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .all(|tool| tool.as_str().is_some_and(|name| allowed.contains(name)))
        }) {
            return Err(reject("claude_profile_native_tools_changed"));
        }
        Ok(())
    }

    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        self == other
    }
    pub(super) fn digest(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("fixed profile is serializable"))
        )
    }
}

#[path = "claude_profile_v2.rs"]
mod v2;
pub use v2::{ClaudeFileProfile, ClaudeRestrictedFilesV2};

fn mcp_denied(rule: &str, tool: &str) -> bool {
    rule == "mcp__*" || rule == tool
}

fn validate_deny(rule: &str) -> Result<(), RuntimeError> {
    if matches!(rule, "*" | "Read" | "Edit" | "mcp__*") {
        return Ok(());
    }
    if rule.starts_with("mcp__")
        && rule
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-'))
    {
        return Ok(());
    }
    let Some(path) = rule
        .strip_prefix("Read(//")
        .or_else(|| rule.strip_prefix("Edit(//"))
        .and_then(|path| path.strip_suffix(')'))
    else {
        return Err(reject("claude_profile_deny_anchor_unsupported"));
    };
    if path.is_empty()
        || path.chars().any(|value| {
            matches!(value, '*' | '?' | '[' | ']' | '!' | '(' | ')' | '\\') || value.is_control()
        })
        || path.split('/').any(|part| matches!(part, ".." | "." | ""))
    {
        return Err(reject("claude_profile_deny_pattern_unsupported"));
    }
    Ok(())
}

fn validate_settings(value: &Value, fixed: bool) -> Result<(), RuntimeError> {
    if value
        .get("errors")
        .is_some_and(|value| !value.is_null() && value != &json!([]))
    {
        return Err(reject("claude_profile_settings_errors"));
    }
    let sources = value["sources"]
        .as_array()
        .ok_or_else(|| reject("claude_profile_settings_shape"))?;
    for source in sources {
        let name = source["source"]
            .as_str()
            .ok_or_else(|| reject("claude_profile_settings_source"))?;
        let settings = source["settings"]
            .as_object()
            .ok_or_else(|| reject("claude_profile_settings_shape"))?;
        if name == "policySettings" && !settings.is_empty() {
            return Err(reject("claude_profile_admin_policy_unverified"));
        }
        if (fixed && !matches!(name, "flagSettings" | "policySettings"))
            || (!fixed && name != "policySettings")
        {
            return Err(reject("claude_profile_unexpected_settings_source"));
        }
        if !matches!(
            name,
            "userSettings"
                | "projectSettings"
                | "localSettings"
                | "flagSettings"
                | "policySettings"
        ) || settings
            .keys()
            .any(|key| !matches!(key.as_str(), "permissions" | "sandbox" | "$schema"))
        {
            return Err(reject("claude_profile_settings_unsupported"));
        }
        if let Some(sandbox) = settings.get("sandbox") {
            if sandbox != &json!({"enabled":false}) {
                return Err(reject("claude_profile_sandbox_unsupported"));
            }
        }
        if let Some(permissions) = settings.get("permissions") {
            let permissions = permissions
                .as_object()
                .ok_or_else(|| reject("claude_profile_permissions_shape"))?;
            if permissions.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "allow"
                        | "deny"
                        | "ask"
                        | "defaultMode"
                        | "disableBypassPermissionsMode"
                        | "additionalDirectories"
                )
            }) || permissions
                .get("additionalDirectories")
                .is_some_and(|value| value != &json!([]))
            {
                return Err(reject("claude_profile_permissions_unsupported"));
            }
            for key in ["allow", "deny", "ask"] {
                if permissions.get(key).is_some_and(|value| {
                    !value
                        .as_array()
                        .is_some_and(|rows| rows.iter().all(Value::is_string))
                }) {
                    return Err(reject("claude_profile_permissions_shape"));
                }
            }
        }
    }
    Ok(())
}

fn validate_rule_scope(
    value: &Value,
    cwd: &Path,
    canonical_cwd: &Path,
) -> Result<(), RuntimeError> {
    let state = &value["state"];
    // 原生 getcwd 可展开符号链接；只接受创建时保存的两种目录表示。
    if !state["originalCwd"]
        .as_str()
        .map(Path::new)
        .is_some_and(|path| path == cwd || path == canonical_cwd)
        || state["managedOnly"] != false
        || state["workspaceDirectories"] != json!([])
        || state
            .get("errors")
            .is_some_and(|value| !value.is_null() && value != &json!([]))
    {
        return Err(reject("claude_profile_rule_scope"));
    }
    Ok(())
}

fn validate_hooks(value: &Value) -> Result<(), RuntimeError> {
    if value["hooks"] != json!([])
        || value["policy"]["policyHookCount"] != 0
        || value["policy"]["policyUnreadable"] == true
        || value
            .get("errors")
            .is_some_and(|value| !value.is_null() && value != &json!([]))
    {
        return Err(reject("claude_profile_hooks_unsupported"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "claude_profile_tests.rs"]
mod tests;
