//! Grok 创建输入的固定策略；本模块不把配置相等或 ACP 回显当作原生生效证明。

use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use super::local_tools::{LocalToolPermissions, MCP_SERVER_NAME, tool_definitions};
use super::{PermissionPolicy, RuntimeError, SessionTarget};

const LEGACY_VERIFIED_VERSION: &str = "1.0.30";
const P0_VERIFIED_VERSION: &str = "1.0.34";
const CURRENT_VERSION: &str = "1.0.40";
const FIXED_SCOPE_VERSION: &str = "1.0.41";
// 这里只绑定各平台的固定官方文件摘要；权限仍需启动材料、目录和实际工具证据。
const VERIFIED_EXECUTABLES: &[(&str, &str, &str, &str)] = &[
    (
        "linux",
        "x86_64",
        FIXED_SCOPE_VERSION,
        "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407",
    ),
    (
        "windows",
        "x86_64",
        FIXED_SCOPE_VERSION,
        "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
    ),
    (
        "macos",
        "aarch64",
        FIXED_SCOPE_VERSION,
        "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d",
    ),
    (
        "macos",
        "aarch64",
        LEGACY_VERIFIED_VERSION,
        "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb",
    ),
    (
        "macos",
        "aarch64",
        P0_VERIFIED_VERSION,
        "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e",
    ),
    (
        "macos",
        "aarch64",
        CURRENT_VERSION,
        "3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2",
    ),
    (
        "linux",
        "x86_64",
        LEGACY_VERIFIED_VERSION,
        "504dd6546ab991b75d36698242875ce461489cd1f8cd84285873cb55bd5c7d54",
    ),
    (
        "linux",
        "x86_64",
        P0_VERIFIED_VERSION,
        "be5905e107d2b8b5f3c142d21ecfe4c8fd32a913d2fd551b788707930c4dc80d",
    ),
    (
        "linux",
        "x86_64",
        CURRENT_VERSION,
        "92c997dfd109c0672d40d5ae6fbd15835d53ffaf12cf9ea124d22aaef3ff23fc",
    ),
    (
        "windows",
        "x86_64",
        LEGACY_VERIFIED_VERSION,
        "ca24ea63272ba7881261f4a52498d1f5bd884b01da25845990422a10dd315266",
    ),
    (
        "windows",
        "x86_64",
        P0_VERIFIED_VERSION,
        "021d8f7f6bdf9db48b6c87e799cd99130a76c463e6e6a3161510839aed016d94",
    ),
    (
        "windows",
        "x86_64",
        CURRENT_VERSION,
        "034c883fa3962ab6ca409c2d3c7501c642166535dd39fa936ecffe1ac2cad92e",
    ),
];
const PROFILE_NAME: &str = "infinishell-managed-grok-v1";
const READ_TOOL: &str = "GrokBuild:read_file";
const WRITE_TOOL: &str = "OpenCode:write";
const EDIT_TOOL: &str = "GrokBuild:search_replace";
const SEARCH_TOOL: &str = "GrokBuild:search_tool";
const USE_TOOL: &str = "GrokBuild:use_tool";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum GrokToolSet {
    #[default]
    Read,
    Files,
}

impl GrokToolSet {
    fn is_read(&self) -> bool {
        *self == Self::Read
    }
}

/// 固定父子创建输入与应用 SDK 能力，不声明文件系统沙箱或原生目录已经核验。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GrokCreationPolicyV1 {
    version: u32,
    storage_id: Uuid,
    canonical_working_directory: PathBuf,
    executable_sha256: String,
    config_sha256: String,
    // 旧记录缺少该字段时仍为只读，序列化继续省略，不能在升级时扩大旧任务权限。
    #[serde(default, skip_serializing_if = "GrokToolSet::is_read")]
    tool_set: GrokToolSet,
    #[serde(deserialize_with = "deserialize_local_tools")]
    local_tools: Option<LocalToolPermissions>,
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

fn reject(reason: &str) -> RuntimeError {
    super::permissions::rejected(
        None,
        &json!({"creationPolicy":"GrokCreationPolicyV1"}),
        reason,
        false,
    )
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl GrokCreationPolicyV1 {
    /// 调用方从独占进程的启动材料计算摘要；不接受模型参数提供这些值。
    pub(super) fn compile(
        cwd: &Path,
        cli_version: &str,
        executable_sha256: String,
        config_sha256: String,
        local_tools: Option<LocalToolPermissions>,
        permission_policy: PermissionPolicy,
    ) -> Result<Self, RuntimeError> {
        if !matches!(
            cli_version,
            LEGACY_VERIFIED_VERSION | P0_VERIFIED_VERSION | CURRENT_VERSION | FIXED_SCOPE_VERSION
        ) || !cwd.is_absolute()
        {
            return Err(reject("grok_creation_policy_version_or_directory"));
        }
        let tool_set = match permission_policy {
            PermissionPolicy::GrokRestrictedReadV1 => GrokToolSet::Read,
            PermissionPolicy::GrokRestrictedFilesV1 => GrokToolSet::Files,
            PermissionPolicy::Inherit
            | PermissionPolicy::ReadOnly
            | PermissionPolicy::WorkspaceWrite
            | PermissionPolicy::ClaudeRestrictedFilesV1
            | PermissionPolicy::ClaudeRestrictedFilesV2 => {
                return Err(reject("grok_creation_policy_not_selected"));
            }
        };
        let policy = Self {
            version: 1,
            storage_id: Uuid::new_v4(),
            canonical_working_directory: std::fs::canonicalize(cwd)
                .map_err(|_| reject("grok_creation_policy_directory_unavailable"))?,
            executable_sha256,
            config_sha256,
            tool_set,
            local_tools,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        if self.version != 1
            || self.storage_id.is_nil()
            || !self.canonical_working_directory.is_absolute()
            // 原生目录信任对文件系统根自动放行，固定项目策略不能使用该例外。
            || self.canonical_working_directory.parent().is_none()
            || self
                .canonical_working_directory
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
            || !valid_sha256(&self.executable_sha256)
            || !valid_sha256(&self.config_sha256)
        {
            return Err(reject("grok_creation_policy_invalid"));
        }
        Ok(())
    }

    /// 恢复已有任务时重新检查启动材料，不能把新配置覆盖到已保存的父权限上限。
    pub(super) fn validate_launch(
        &self,
        cwd: &Path,
        cli_version: &str,
        executable_sha256: String,
        config_sha256: String,
        permission_policy: PermissionPolicy,
    ) -> Result<(), RuntimeError> {
        if !self.matches_cli_version(cli_version) {
            return Err(reject("grok_creation_policy_cli_version_changed"));
        }
        let current = Self::compile(
            cwd,
            cli_version,
            executable_sha256,
            config_sha256,
            self.local_tools,
            permission_policy,
        )?;
        self.validate_child(&current)?;
        Ok(())
    }

    pub(super) fn local_tools(&self) -> Option<LocalToolPermissions> {
        self.local_tools
    }

    pub(super) fn permission_policy(&self) -> PermissionPolicy {
        match self.tool_set {
            GrokToolSet::Read => PermissionPolicy::GrokRestrictedReadV1,
            GrokToolSet::Files => PermissionPolicy::GrokRestrictedFilesV1,
        }
    }

    pub(super) fn matches_cli_version(&self, cli_version: &str) -> bool {
        VERIFIED_EXECUTABLES.iter().any(|(_, _, version, digest)| {
            *version == cli_version && *digest == self.executable_sha256.as_str()
        })
    }

    /// 这里只描述已取得真实收据的生产启动范围；保存记录能够反序列化不等于该版本可执行。
    pub(super) fn runtime_scope_verified(
        &self,
        cli_version: &str,
        local_tools: Option<LocalToolPermissions>,
        permission_policy: PermissionPolicy,
    ) -> bool {
        self.validate().is_ok()
            && self.matches_cli_version(cli_version)
            && self.local_tools == local_tools
            && self.permission_policy() == permission_policy
            && (cli_version == LEGACY_VERIFIED_VERSION
                || super::grok::current_fixed_scope_supported_version(cli_version)
                || (cli_version == P0_VERIFIED_VERSION
                    && local_tools.is_none()
                    && permission_policy == PermissionPolicy::GrokRestrictedReadV1))
    }

    pub(super) fn native_tool_ids(&self) -> Vec<&'static str> {
        let mut tools = vec![READ_TOOL];
        if self.tool_set == GrokToolSet::Files {
            tools.extend([WRITE_TOOL, EDIT_TOOL]);
        }
        if self.local_tools.is_some() {
            tools.extend([SEARCH_TOOL, USE_TOOL]);
        }
        tools
    }

    /// JSON 是合法 YAML；保留固定前置元数据，避免把空工具列表解释成继承默认工具。
    pub(super) fn profile_document(&self) -> Result<String, RuntimeError> {
        self.validate()?;
        let tools: Vec<Value> = self
            .native_tool_ids()
            .into_iter()
            .map(|id| json!({"id":id}))
            .collect();
        let (name, description) = match self.tool_set {
            GrokToolSet::Read => (
                PROFILE_NAME,
                "Application-managed read tools and local task tools.",
            ),
            GrokToolSet::Files => (
                "infinishell-managed-grok-files-v1",
                "Application-managed file tools and local task tools.",
            ),
        };
        let profile = json!({
            "name":name,
            "description":description,
            "permissionMode":"default",
            "discoverSkills":false,
            "inheritSkills":false,
            "agentsMd":false,
            "injectDefaultTools":false,
            "toolConfig":{"tools":tools},
            "tools":self.native_tool_ids(),
            "skills":[],
            "mcpServers":[],
            "mcpInheritance":"none",
            "hooks":{},
        });
        Ok(format!(
            "---\n{profile}\n---\nUse the application local-task tools for managed child tasks.\n"
        ))
    }

    /// 只用于独占冷进程的创建输入；这些 false 不是 warm load 降权确认。
    pub(super) fn creation_meta() -> Value {
        json!({"yoloMode":false,"autoMode":false})
    }

    /// 子任务可移除应用能力，不能改变父代的执行文件、配置来源或工作目录。
    pub(super) fn derive_child(
        &self,
        local_tools: Option<LocalToolPermissions>,
    ) -> Result<Self, RuntimeError> {
        self.validate()?;
        let mut child = self.clone();
        child.local_tools = local_tools;
        child.storage_id = Uuid::new_v4();
        self.validate_child(&child)?;
        Ok(child)
    }

    pub(super) fn validate_child(&self, child: &Self) -> Result<(), RuntimeError> {
        self.validate()?;
        child.validate()?;
        if self.canonical_working_directory != child.canonical_working_directory
            || self.executable_sha256 != child.executable_sha256
            || self.config_sha256 != child.config_sha256
            || self.tool_set != child.tool_set
        {
            return Err(reject("grok_child_creation_identity_changed"));
        }
        let subset = match (self.local_tools, child.local_tools) {
            (None, None) | (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(parent), Some(child)) => {
                (!child.allow_spawn || parent.allow_spawn)
                    && (!child.allow_message || parent.allow_message)
            }
        };
        if !subset {
            return Err(reject("grok_child_local_permissions_increased"));
        }
        Ok(())
    }

    /// 必须在写出单次允许前检查完整目标，不能用应用 server 前缀代替工具白名单。
    pub(super) fn permits_sdk_target(&self, qualified_target: &str) -> bool {
        let Some(permissions) = self.local_tools else {
            return false;
        };
        if self.validate().is_err() {
            return false;
        }
        let Some(tool_name) = qualified_target.strip_prefix(&format!("{MCP_SERVER_NAME}__")) else {
            return false;
        };
        tool_definitions(permissions.allow_spawn, permissions.allow_message)
            .iter()
            .any(|tool| tool["name"].as_str() == Some(tool_name))
    }
}

const FIXED_CONFIGURATION: &str = r#"[cli]
use_leader = false
auto_update = false
[folder_trust]
enabled = true
[models]
default = "grok-4.6-build"
session_summary = "grok-4.6-build"
web_search = "grok-4.6-build"
image_description = "grok-4.6-build"
[features]
turn_summary = false
title_refresh = false
support_permission = true
[[permission.rules]]
action = "ask"
tool = "any"
[compat.claude]
hooks = false
mcps = false
[marketplace]
default_skills_installs_purged = true
official_marketplace_auto_installed = true
[[marketplace.sources]]
name = "xAI Official"
git = "https://github.com/xai-org/plugin-marketplace.git"
"#;

fn fixed_configuration(current: bool) -> String {
    // 已保存旧任务保留原模型配置摘要；当前固定版本使用本轮实链固定的模型。
    if current {
        FIXED_CONFIGURATION.replace("grok-4.6-build", "grok-4.7")
    } else {
        FIXED_CONFIGURATION.to_owned()
    }
}

pub(super) struct GrokLaunch {
    pub policy: GrokCreationPolicyV1,
    pub home: PathBuf,
    pub profile_path: PathBuf,
}

impl Drop for GrokLaunch {
    fn drop(&mut self) {
        // 正常监督器先清理；启动失败或 future 被取消时也不遗留本端认证副本。
        let _ = std::fs::remove_file(self.home.join("grok/auth.json"));
    }
}

impl GrokCreationPolicyV1 {
    pub(super) fn working_directory(&self) -> &Path {
        &self.canonical_working_directory
    }

    pub(super) fn verify_catalog(
        &self,
        tools: Option<&Value>,
        task_input_sent: bool,
    ) -> Result<(), RuntimeError> {
        let actual = tools.and_then(Value::as_array).ok_or_else(|| {
            self.reject_catalog(tools, "grok_creation_catalog_missing", task_input_sent)
        })?;
        let mut names = std::collections::HashSet::new();
        let expected: Vec<_> = self
            .native_tool_ids()
            .into_iter()
            .map(|id| id.rsplit(':').next().expect("固定工具均具有名称"))
            .collect();
        if actual.len() != expected.len()
            || actual.iter().any(|name| {
                name.as_str()
                    .is_none_or(|name| !expected.contains(&name) || !names.insert(name))
            })
        {
            return Err(self.reject_catalog(
                tools,
                "grok_creation_catalog_changed",
                task_input_sent,
            ));
        }
        Ok(())
    }

    pub(super) fn verify_catalog_with_mcp(
        &self,
        tools: Option<&Value>,
        task_input_sent: bool,
        served_names: &[String],
    ) -> Result<(), RuntimeError> {
        // 原生初始化先公布纯 builtin；异步注册完成后只接受整个已发送本地目录的并集。
        if served_names.is_empty() || self.verify_catalog(tools, task_input_sent).is_ok() {
            return self.verify_catalog(tools, task_input_sent);
        }
        let builtin: Vec<_> = self
            .native_tool_ids()
            .into_iter()
            .map(|id| id.rsplit(':').next().expect("固定工具均具有名称"))
            .collect();
        let mut actual_names = std::collections::HashSet::new();
        if !tools.and_then(Value::as_array).is_some_and(|actual| {
            actual.len() == builtin.len() + served_names.len()
                && actual.iter().all(|value| {
                    value.as_str().is_some_and(|name| {
                        (builtin.contains(&name)
                            || served_names.iter().any(|expected| expected == name))
                            && actual_names.insert(name)
                    })
                })
                && builtin.iter().all(|name| actual_names.contains(name))
                && served_names
                    .iter()
                    .all(|name| actual_names.contains(name.as_str()))
        }) {
            return Err(self.reject_catalog(
                tools,
                "grok_creation_catalog_changed",
                task_input_sent,
            ));
        }
        Ok(())
    }

    fn reject_catalog(
        &self,
        tools: Option<&Value>,
        reason: &str,
        task_input_sent: bool,
    ) -> RuntimeError {
        // 只输出固定原生名称的计数；未知文本、路径、对象和散列都不进入错误证据。
        let known = [
            "read_file",
            "write",
            "search_replace",
            "search_tool",
            "use_tool",
        ];
        let mut known_counts = [0usize; 5];
        let mut unknown_strings = 0;
        let mut non_strings = 0;
        let mut duplicate_strings = 0;
        let mut seen = std::collections::HashSet::new();
        if let Some(items) = tools.and_then(Value::as_array) {
            for item in items {
                if let Some(name) = item.as_str() {
                    if !seen.insert(name) {
                        duplicate_strings += 1;
                    }
                    if let Some(index) = known.iter().position(|known| *known == name) {
                        known_counts[index] += 1;
                    } else {
                        unknown_strings += 1;
                    }
                } else {
                    non_strings += 1;
                }
            }
        }
        let value_type = match tools {
            None => "missing",
            Some(Value::Null) => "null",
            Some(Value::Bool(_)) => "boolean",
            Some(Value::Number(_)) => "number",
            Some(Value::String(_)) => "string",
            Some(Value::Array(_)) => "array",
            Some(Value::Object(_)) => "object",
        };
        let expected: Vec<_> = self
            .native_tool_ids()
            .into_iter()
            .map(|id| id.rsplit(':').next().expect("固定工具均具有名称"))
            .collect();
        let counts: serde_json::Map<String, Value> = known
            .into_iter()
            .zip(known_counts)
            .map(|(name, count)| (name.to_owned(), json!(count)))
            .collect();
        let actual = json!({"creationPolicy":"GrokCreationPolicyV1","catalog":{
            "value_type":value_type,"item_count":tools.and_then(Value::as_array).map(Vec::len),
            "expected_names":expected,"known_name_counts":counts,
            "unknown_string_count":unknown_strings,"non_string_count":non_strings,
            "duplicate_string_count":duplicate_strings}});
        let mut error = super::permissions::rejected(None, &actual, reason, false);
        if let RuntimeError::PermissionCeilingRejected { details, .. } = &mut error {
            // 此字段是成功写入证明，不是原生接收证明，也不包含恢复的历史输入。
            details["task_input_sent"] = json!(task_input_sent);
        }
        error
    }

    pub(super) fn verify_files(&self, state: &Path) -> Result<(), RuntimeError> {
        let home = state
            .canonicalize()?
            .join("grok-managed")
            .join(self.storage_id.to_string());
        let config = home.join("grok/config.toml");
        let profile = home.join("profile.md");
        if !config.is_file() || !profile.is_file() {
            return Err(reject("grok_creation_files_missing"));
        }
        immutable_file(&config, self.fixed_configuration().as_bytes())?;
        immutable_file(&profile, self.profile_document()?.as_bytes())
    }

    pub(super) fn permits_native_request(&self, tool_call: &Value) -> bool {
        let input = &tool_call["rawInput"];
        if let Some(target) = input["tool_name"].as_str() {
            return input["variant"] == "UseTool"
                && self.permits_sdk_target(target)
                && input["tool_input"].is_object();
        }
        if input["variant"] == "ReadFile"
            || (input["variant"] == "SearchTool" && self.local_tools.is_some())
        {
            return input.is_object();
        }
        if self.tool_set != GrokToolSet::Files || self.validate().is_err() {
            return false;
        }
        let Some(fields) = input.as_object() else {
            return false;
        };
        let name = tool_call["_meta"]["x.ai/tool"]["name"].as_str();
        // 精确目录之外还核对审批名称与输入结构；未知版本不能借 AllowOnce 扩大工具集合。
        if tool_call["kind"] != "edit" {
            return false;
        }
        match (name, input["variant"].as_str()) {
            (Some("write"), Some("Write")) => {
                fields.len() == 3
                    && input["file_path"]
                        .as_str()
                        .is_some_and(|path| Path::new(path).is_absolute())
                    && input["content"].is_string()
            }
            (Some("search_replace"), Some("SearchReplace")) => {
                fields.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "variant" | "file_path" | "old_string" | "new_string" | "replace_all"
                    )
                }) && input["file_path"]
                    .as_str()
                    .is_some_and(|path| !path.is_empty())
                    && input["old_string"].is_string()
                    && input["new_string"].is_string()
                    && input.get("replace_all").is_none_or(Value::is_boolean)
            }
            _ => false,
        }
    }

    fn fixed_configuration(&self) -> String {
        fixed_configuration(self.matches_cli_version(FIXED_SCOPE_VERSION))
    }

    fn prepare_storage(
        &self,
        state: &Path,
        target: &SessionTarget,
    ) -> Result<PathBuf, RuntimeError> {
        if !state.is_absolute() {
            return Err(reject("grok_managed_directory_invalid"));
        }
        std::fs::create_dir_all(state)?;
        let parent = state.canonicalize()?.join("grok-managed");
        create_private_directory(&parent)?;
        let home = parent.join(self.storage_id.to_string());
        if *target != SessionTarget::New && !home.is_dir() {
            return Err(reject("grok_resume_creation_storage_missing"));
        }
        create_private_directory(&home)?;
        Ok(home)
    }

    pub(super) fn prepare(
        options: &super::SessionOptions,
        profile_state_dir: &Path,
    ) -> Result<GrokLaunch, RuntimeError> {
        let (cli_version, digest) = verified_executable_digest(&options.executable)?;
        let configuration = fixed_configuration(cli_version == FIXED_SCOPE_VERSION);
        let config_digest = format!("{:x}", Sha256::digest(configuration.as_bytes()));
        let mut policy = Self::compile(
            &options.cwd,
            cli_version,
            digest,
            config_digest,
            options.local_tools,
            options.permission_policy,
        )?;
        if !policy.runtime_scope_verified(
            cli_version,
            options.local_tools,
            options.permission_policy,
        ) {
            // 必须早于目录、固定配置及认证副本写入；仅版本摘要匹配不能放行未验工具集。
            return Err(reject("grok_creation_policy_scope_unverified"));
        }
        if let Some(saved) = &options.grok_profile {
            saved.validate_launch(
                &options.cwd,
                cli_version,
                policy.executable_sha256.clone(),
                policy.config_sha256.clone(),
                options.permission_policy,
            )?;
            if saved.local_tools != options.local_tools {
                return Err(reject("grok_saved_local_permissions_changed"));
            }
            policy = saved.clone();
        } else if options.target != super::SessionTarget::New {
            return Err(reject("grok_resume_creation_policy_missing"));
        }
        if let Some(parent) = &options.permission_ceiling {
            parent
                .grok_profile()
                .ok_or_else(|| reject("grok_creation_parent_unknown"))?
                .validate_child(&policy)?;
        }
        // 持久会话存储属于应用作用域，不能随监督宿主的进程代次更换。
        let home = policy.prepare_storage(profile_state_dir, &options.target)?;
        for path in [
            "grok",
            "home",
            "home/.config",
            "home/.local/share",
            "home/.cache",
            "home/.claude",
            "home/.codex",
            "home/AppData/Roaming",
            "home/AppData/Local",
            "tmp",
            "startup",
        ] {
            create_private_directory(&home.join(path))?;
        }
        let source_home = std::env::var_os("GROK_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".grok")))
            .ok_or_else(|| reject("grok_auth_directory_unavailable"))?;
        // 必须早于清理及 Drop guard；来源别名不能被当成可删除的认证副本。
        ensure_distinct_auth_source(&source_home, &home.join("grok"))?;
        let source = source_home.join("auth.json");
        reset_project_trust(&home)?;
        let launch = GrokLaunch {
            policy,
            profile_path: home.join("profile.md"),
            home,
        };
        immutable_file(
            &launch.home.join("grok/config.toml"),
            configuration.as_bytes(),
        )?;
        immutable_file(
            &launch.profile_path,
            launch.policy.profile_document()?.as_bytes(),
        )?;
        // 只复制原生认证缓存，不导入用户配置、插件、hooks 或历史会话。
        let target = launch.home.join("grok/auth.json");
        // 先清除旧运行遗留的副本；用户退出登录后不能沿用旧缓存。
        if target.exists() {
            std::fs::remove_file(&target)?;
        }
        if source.exists() {
            let metadata = std::fs::symlink_metadata(&source)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > 4 * 1024 * 1024
            {
                return Err(reject("grok_auth_cache_invalid"));
            }
            let mut source = std::fs::File::open(source)?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut target = options.open(target)?;
            std::io::copy(&mut source, &mut target)?;
            target.sync_all()?;
        }
        Ok(launch)
    }
}

fn ensure_distinct_auth_source(source_home: &Path, target_home: &Path) -> Result<(), RuntimeError> {
    match source_home.canonicalize() {
        Ok(source) if source == target_home.canonicalize()? => {
            Err(reject("grok_auth_source_matches_managed_copy"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reset_project_trust(home: &Path) -> Result<(), RuntimeError> {
    // 每次冷创建/恢复都不沿用应用专属域内的项目授权；系统管理员策略仍由原生 CLI 应用。
    for name in ["trusted_folders.toml", "trusted-hook-projects"] {
        match std::fs::remove_file(home.join("grok").join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), RuntimeError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = std::fs::metadata(path)?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(reject("grok_managed_directory_permissions"));
        }
    }
    if path.canonicalize()? != path || !path.is_dir() {
        return Err(reject("grok_managed_directory_invalid"));
    }
    Ok(())
}

fn immutable_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != contents.len() as u64
            || std::fs::read(path)? != contents
        {
            return Err(reject("grok_creation_file_changed"));
        }
        return Ok(());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o400);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn verified_executable_digest(path: &Path) -> Result<(&'static str, String), RuntimeError> {
    if !VERIFIED_EXECUTABLES
        .iter()
        .any(|(os, arch, _, _)| *os == std::env::consts::OS && *arch == std::env::consts::ARCH)
    {
        return Err(reject("grok_creation_platform_unverified"));
    }
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
    let digest = format!("{:x}", digest.finalize());
    let version = VERIFIED_EXECUTABLES
        .iter()
        .find_map(|(os, arch, version, expected)| {
            (*os == std::env::consts::OS && *arch == std::env::consts::ARCH && digest == *expected)
                .then_some(*version)
        })
        .ok_or_else(|| reject("grok_creation_executable_unverified"))?;
    Ok((version, digest))
}

#[cfg(test)]
#[path = "grok_profile_tests.rs"]
mod tests;
