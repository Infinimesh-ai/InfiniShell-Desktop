use std::collections::HashMap;
use std::ffi::OsString;
#[cfg(feature = "local_fs")]
use std::future::Future;
use std::path::{Path, PathBuf};
#[cfg(feature = "local_fs")]
use std::sync::mpsc::SyncSender;

use shell_words::quote as shell_quote;
use uuid::Uuid;
use warp_cli::agent::Harness;

use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::agent_sdk::driver::harness::{
    harness_model_env_vars, remove_claude_externally_managed_listener_env_vars,
};
use crate::ai::agent_sdk::{
    ClaudeHarness, ThirdPartyHarness, task_env_vars, validate_cli_installed,
};
use crate::ai::ambient_agents::task::{
    HarnessConfig, HarnessModelConfig, normalize_orchestrator_agent_name,
};
use crate::ai::ambient_agents::{AgentConfigSnapshot, AmbientAgentTaskId};
#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::conversation_bridge::recorded_history_identity;
use crate::ai::local_harness_setup::local_harness_product_disabled_message;
#[cfg(feature = "local_fs")]
use crate::persistence::ModelEvent;
#[cfg(feature = "local_fs")]
use crate::persistence::local_cli_tasks::{
    LocalCliTask, LocalCliTaskState, checkpoint_task, load_tasks,
};
use crate::terminal::shell::ShellType;

#[derive(Clone)]
pub(super) struct PreparedLocalHarnessLaunch {
    pub command: String,
    pub env_vars: HashMap<OsString, OsString>,
    pub run_id: String,
    pub task_id: AmbientAgentTaskId,
    pub config: AgentConfigSnapshot,
    pub native_session_id: Option<String>,
}

pub(super) fn normalize_local_child_harness(harness_type: &str) -> Option<Harness> {
    Harness::parse_local_child_harness(harness_type)
}

pub(super) fn validate_local_harness_shell(shell_type: Option<ShellType>) -> Result<(), String> {
    match shell_type {
        Some(ShellType::Bash | ShellType::Zsh | ShellType::Fish | ShellType::PowerShell) => Ok(()),
        None => Err(crate::t!("ambient-agent-local-harness-shell-required")),
    }
}

pub(super) fn local_claude_child_prompt(task_prompt: &str) -> String {
    task_prompt.to_owned()
}

pub(super) fn build_local_claude_child_command(
    prompt: &str,
    session_id: Uuid,
    shell_type: ShellType,
) -> String {
    let quoted_prompt = quote_local_harness_argument(prompt, shell_type);
    // 继承用户的原生权限设置；调用方必须先显示终端，再启动交互进程。
    format!("claude --session-id {session_id} -- {quoted_prompt}")
}

pub(super) fn build_local_opencode_child_command(prompt: &str, shell_type: ShellType) -> String {
    let quoted_prompt = quote_local_harness_argument(prompt, shell_type);
    format!("opencode --prompt {quoted_prompt}")
}
pub(super) fn build_local_codex_child_command(prompt: &str, shell_type: ShellType) -> String {
    let quoted_prompt = quote_local_harness_argument(prompt, shell_type);
    // `--` 保证以短横线开头的任务文本不会被解释为 CLI 参数。
    format!("codex -- {quoted_prompt}")
}

fn quote_local_harness_argument(value: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::PowerShell => {
            // PowerShell 单引号字符串不展开变量或反引号，内部单引号通过双写保留。
            let quoted = value.replace('\'', "''");
            format!("'{quoted}'")
        }
        ShellType::Bash | ShellType::Zsh | ShellType::Fish => shell_quote(value).into_owned(),
    }
}

fn command_with_discovered_executable(
    command: &str,
    executable: &Path,
    shell_type: ShellType,
) -> Result<String, String> {
    let path = executable
        .to_str()
        .ok_or_else(|| crate::t!("cli-agent-message-recipient-unavailable"))?;
    let (_, arguments) = command
        .split_once(' ')
        .expect("本地 CLI 启动命令包含固定参数");
    let quoted = quote_local_harness_argument(path, shell_type);
    match shell_type {
        // PowerShell 需要调用运算符才能执行单引号包裹的程序路径。
        ShellType::PowerShell => Ok(format!("& {quoted} {arguments}")),
        ShellType::Bash | ShellType::Zsh | ShellType::Fish => Ok(format!("{quoted} {arguments}")),
    }
}

pub(super) fn local_child_task_config(
    harness: Harness,
    agent_name: Option<String>,
) -> Option<AgentConfigSnapshot> {
    let agent_name = agent_name
        .as_deref()
        .and_then(normalize_orchestrator_agent_name);
    match harness {
        Harness::Oz | Harness::Unknown => None,
        Harness::Claude | Harness::OpenCode | Harness::Gemini | Harness::Codex | Harness::Grok => {
            Some(AgentConfigSnapshot {
                name: agent_name,
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            })
        }
    }
}

pub(super) async fn prepare_local_harness_child_launch(
    prompt: String,
    harness_type: String,
    model_id: Option<String>,
    parent_run_id: Option<String>,
    agent_name: Option<String>,
    shell_type: Option<ShellType>,
    resolved_executable: Option<PathBuf>,
) -> Result<PreparedLocalHarnessLaunch, String> {
    let harness_model_config =
        model_id
            .filter(|id| !id.is_empty())
            .map(|model_id| HarnessModelConfig {
                model_id,
                reasoning_level: None,
            });
    let Some(harness) = normalize_local_child_harness(&harness_type) else {
        let harness_name = harness_type.trim();
        return Err(if harness_name.is_empty() {
            crate::t!("ambient-agent-local-harness-missing")
        } else {
            crate::t!(
                "ambient-agent-local-harness-unsupported",
                name = harness_name
            )
        });
    };
    if let Some(message) = local_harness_product_disabled_message(harness) {
        return Err(message.to_string());
    }
    validate_local_harness_shell(shell_type)?;
    let shell_type = shell_type.expect("本地 shell 已验证");
    if let Some(path) = &resolved_executable {
        if !path.is_absolute() || !path.is_file() {
            return Err(crate::t!("cli-agent-message-recipient-unavailable"));
        }
    }
    let native_session_id = (harness == Harness::Claude).then(Uuid::new_v4);
    let command = match harness {
        Harness::Oz => unreachable!("normalize_local_child_harness filters out Oz"),
        Harness::Unknown => unreachable!("normalize_local_child_harness filters out Unknown"),
        Harness::Claude => {
            if resolved_executable.is_none() {
                let claude_harness = ClaudeHarness;
                claude_harness
                    .validate()
                    .map_err(|error: AgentDriverError| error.to_string())?;
            }
            // 本地进程沿用用户配置与已安装插件。此处不执行云端环境准备或
            // 插件安装，避免改写全局引导、目录信任、权限和插件启用设置。

            build_local_claude_child_command(
                &local_claude_child_prompt(&prompt),
                native_session_id.expect("Claude 会话 ID 已生成"),
                shell_type,
            )
        }
        Harness::Codex => {
            // Zap:`harness_kind` 只覆盖 driver 支持的 harness,本地 Codex 子 pane
            // 直接校验 CLI 是否可用即可。
            if resolved_executable.is_none() {
                validate_cli_installed("codex", Some("https://developers.openai.com/codex/cli"))
                    .map_err(|error: AgentDriverError| error.to_string())?;
            }

            // Local Codex child panes must rely on the user's existing local
            // auth/session state. Do not run the shared Codex environment prep
            // here: it can seed OPENAI_API_KEY into ~/.codex/auth.json and
            // rewrite ~/.codex/config.toml for the whole machine.
            build_local_codex_child_command(&prompt, shell_type)
        }
        Harness::OpenCode => {
            if resolved_executable.is_none() {
                validate_cli_installed("opencode", Some("https://opencode.ai/docs"))
                    .map_err(|error: AgentDriverError| error.to_string())?;
            }
            build_local_opencode_child_command(&prompt, shell_type)
        }
        Harness::Gemini => unreachable!("normalize_local_child_harness filters out Gemini"),
        Harness::Grok => unreachable!("normalize_local_child_harness filters out Grok"),
    };
    let command = match resolved_executable {
        Some(executable) => command_with_discovered_executable(&command, &executable, shell_type)?,
        None => command,
    };

    let mut config = local_child_task_config(harness, agent_name).expect("本地 harness 已验证");
    if harness == Harness::Claude {
        config
            .harness
            .as_mut()
            .expect("本地 harness 配置已创建")
            .model_id = harness_model_config
            .as_ref()
            .map(|model| model.model_id.clone());
    }
    let task_id = AmbientAgentTaskId::new_local();

    let mut env_vars = task_env_vars(Some(&task_id), parent_run_id.as_deref(), harness);
    if harness == Harness::Claude {
        // Local Claude child panes are launched directly in hidden terminals,
        // not through AgentDriver's ClaudeHarnessRunner. Let the Claude plugin
        // manage its own listener instead of waiting for a non-existent
        // external MessageBridge.
        remove_claude_externally_managed_listener_env_vars(&mut env_vars);
    }
    // Propagate the selected model to Claude Code via ANTHROPIC_MODEL.
    // Codex local children never receive a model override — the UI
    // ensures model_id is empty for local Codex.
    env_vars.extend(harness_model_env_vars(
        harness,
        harness_model_config.as_ref(),
    ));

    Ok(PreparedLocalHarnessLaunch {
        command,
        env_vars,
        run_id: task_id.to_string(),
        task_id,
        config,
        native_session_id: native_session_id.map(|id| id.to_string()),
    })
}

/// 先登记已有父会话，再确认子任务检查点；此函数不会执行命令或安装插件。
#[cfg(feature = "local_fs")]
pub(super) async fn persist_local_harness_child_launch<F, Fut>(
    sender: &SyncSender<ModelEvent>,
    parent: LocalCliTask,
    mut child: LocalCliTask,
    validate_parent: F,
) -> Result<LocalCliTask, String>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let parent = persist_local_harness_parent(sender, parent, &validate_parent).await?;
    child.parent_generation = Some(parent.generation);
    validate_parent().await?;
    checkpoint_task(sender, child.clone(), None)?
        .await
        .map_err(|_| "子任务检查点通道已关闭".to_owned())??;
    Ok(child)
}

/// 复用当前用户轮的父记录；显式追加消息不会派生子任务或改写既有父子关系。
#[cfg(feature = "local_fs")]
pub(crate) async fn persist_local_harness_parent<F, Fut>(
    sender: &SyncSender<ModelEvent>,
    mut parent: LocalCliTask,
    validate_parent: F,
) -> Result<LocalCliTask, String>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let tasks = load_tasks(sender, false)?
        .await
        .map_err(|_| "本地任务读取通道已关闭".to_owned())??;
    if let Some(existing) = tasks.iter().find(|task| task.task_id == parent.task_id) {
        if existing.version != 1
            || existing.harness != parent.harness
            || existing.parent_task_id != parent.parent_task_id
            || existing.state == LocalCliTaskState::Unknown
        {
            return Err("父任务类型不一致或记录版本不兼容".to_owned());
        }
    } else {
        if parent.harness != "oz" {
            return Err("原生 CLI 父任务必须由托管协调器登记".to_owned());
        }
        if let Some(grandparent_id) = parent.parent_task_id.as_ref() {
            parent.parent_generation = Some(
                tasks
                    .iter()
                    .find(|task| &task.task_id == grandparent_id)
                    .ok_or_else(|| "本地上级父任务尚未持久化".to_owned())?
                    .generation,
            );
        }
        validate_parent().await?;
        let result = checkpoint_task(sender, parent.clone(), None)?
            .await
            .map_err(|_| "父任务检查点通道已关闭".to_owned())?;
        if let Err(error) = result {
            // 同一个父任务可能并行派发多个子任务；只复用已经提交的相同父记录。
            let tasks = load_tasks(sender, false)?
                .await
                .map_err(|_| "本地任务读取通道已关闭".to_owned())??;
            if !tasks.iter().any(|task| {
                task.task_id == parent.task_id
                    && task.version == 1
                    && task.harness == parent.harness
                    && task.parent_task_id == parent.parent_task_id
            }) {
                return Err(error);
            }
        }
    }
    let tasks = load_tasks(sender, false)?
        .await
        .map_err(|_| "本地任务读取通道已关闭".to_owned())??;
    let mut existing = tasks
        .into_iter()
        .find(|task| task.task_id == parent.task_id)
        .ok_or_else(|| "本地父任务记录不存在".to_owned())?;
    validate_parent().await?;
    if let Some(identity) = recorded_history_identity(&parent) {
        let previous_identity = recorded_history_identity(&existing)
            .ok_or_else(|| "父会话缺少用户轮标识，不能推断恢复".to_owned())?;
        if identity != previous_identity {
            // 只有明确的新用户 exchange 才开始新父代，工具结果续流不会改变此标识。
            if existing.state.is_active() {
                existing.state = LocalCliTaskState::Disconnected;
                existing.revision = existing
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| "父运行修订号已耗尽".to_owned())?;
                checkpoint_task(sender, existing.clone(), Some(existing.generation))?
                    .await
                    .map_err(|_| "父运行断开确认已关闭".to_owned())??;
            }
            let previous_generation = existing.generation;
            existing.generation = previous_generation
                .checked_add(1)
                .ok_or_else(|| "父运行代数已耗尽".to_owned())?;
            existing.revision = 0;
            existing.state = LocalCliTaskState::Queued;
            existing.result = None;
            existing.terminal_evidence = None;
            existing.config_json = parent.config_json.clone();
            checkpoint_task(sender, existing.clone(), Some(previous_generation))?
                .await
                .map_err(|_| "父新运行确认已关闭".to_owned())??;
        }
    }
    if existing.state == LocalCliTaskState::Queued
        && serde_json::from_str::<serde_json::Value>(&existing.config_json)
            .ok()
            .and_then(|config| config.get("execution_kind").cloned())
            == Some(serde_json::Value::String("local_parent".to_owned()))
    {
        // 这个父记录描述已经执行派发动作的本地会话，不会再启动一个父进程。
        existing.state = LocalCliTaskState::Running;
        existing.revision = existing
            .revision
            .checked_add(1)
            .ok_or_else(|| "父任务 revision 溢出".to_owned())?;
        let generation = existing.generation;
        let expected_parent = existing.clone();
        let result = checkpoint_task(sender, existing.clone(), Some(generation))?
            .await
            .map_err(|_| "父任务检查点通道已关闭".to_owned())?;
        if let Err(error) = result {
            // 其他子任务可能已经推进同一父记录；仅复用同代、同来源且活动的提交结果。
            let tasks = load_tasks(sender, false)?
                .await
                .map_err(|_| "并行父任务读取确认已关闭".to_owned())??;
            existing = tasks
                .into_iter()
                .find(|task| parent_launch_can_reuse(&expected_parent, task))
                .ok_or(error)?;
        }
    }
    validate_parent().await?;
    Ok(existing)
}

#[cfg(feature = "local_fs")]
fn parent_launch_can_reuse(expected: &LocalCliTask, actual: &LocalCliTask) -> bool {
    actual.version == expected.version
        && actual.task_id == expected.task_id
        && actual.harness == expected.harness
        && actual.parent_task_id == expected.parent_task_id
        && actual.parent_generation == expected.parent_generation
        && actual.generation == expected.generation
        && actual.state.is_active()
        && actual.config_json == expected.config_json
        && recorded_history_identity(actual) == recorded_history_identity(expected)
}

#[cfg(test)]
#[path = "local_harness_launch_tests.rs"]
mod tests;
