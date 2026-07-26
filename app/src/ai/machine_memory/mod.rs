//! Legacy SSH 会话的每机器 AI 记忆加载。

pub(crate) mod review;

use std::fmt::Display;

use warp_ssh_manager::{resolve_machine_key, MachineMemory, MachineMemoryRepository};
use warpui::{AppContext, SingletonEntity as _};

use crate::ai::blocklist::SessionContext;
use crate::settings::AISettings;
use crate::terminal::ssh::util::InteractiveSshCommand;

pub const INJECT_MAX_CHARS: usize = 6_000;
pub const INDEX_MAX_MACHINES: usize = 30;
pub const INDEX_SUMMARY_MAX_CHARS: usize = 120;
pub const INDEX_MAX_CHARS: usize = 3_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineMemoryContext {
    pub machine_key: String,
    /// 注入 prompt 的内容，已按 Unicode 字符截断。
    pub content: String,
}

/// 仅为可定位机器的 legacy SSH 会话加载记忆。
pub fn load_for_session(
    session_context: &SessionContext,
    ctx: &AppContext,
) -> Option<MachineMemoryContext> {
    load_with(
        AISettings::as_ref(ctx).is_ssh_machine_memory_enabled(ctx),
        session_context.is_legacy_ssh(),
        session_context.ssh_connection_info(),
        |machine_key| {
            warp_ssh_manager::with_conn(|conn| {
                Ok(MachineMemoryRepository::get(conn, machine_key)?.map(|memory| memory.content))
            })
        },
    )
}

/// 仅为本地非 SSH 会话加载已知机器索引。
pub fn load_index_for_session(
    session_context: &SessionContext,
    ctx: &AppContext,
) -> Option<String> {
    load_index_with(
        AISettings::as_ref(ctx).is_ssh_machine_memory_enabled(ctx),
        session_context.is_legacy_ssh(),
        session_context.is_remote(),
        || warp_ssh_manager::with_conn(|conn| Ok(MachineMemoryRepository::list_all(conn)?)),
    )
}

fn load_with<E>(
    enabled: bool,
    is_legacy_ssh: bool,
    ssh_connection_info: Option<&InteractiveSshCommand>,
    load_content: impl FnOnce(&str) -> Result<Option<String>, E>,
) -> Option<MachineMemoryContext>
where
    E: Display,
{
    if !enabled || !is_legacy_ssh {
        return None;
    }

    let info = ssh_connection_info?;
    let machine_key = resolve_machine_key(info.host.as_deref(), info.port.as_deref())?;
    let content = match load_content(&machine_key) {
        Ok(Some(content)) => truncate_for_injection(&content),
        Ok(None) => String::new(),
        Err(err) => {
            log::warn!("machine memory load failed for {machine_key}: {err}");
            return None;
        }
    };

    Some(MachineMemoryContext {
        machine_key,
        content,
    })
}

fn truncate_for_injection(content: &str) -> String {
    content.chars().take(INJECT_MAX_CHARS).collect()
}

fn load_index_with<E>(
    enabled: bool,
    is_legacy_ssh: bool,
    is_warpified_ssh: bool,
    load_memories: impl FnOnce() -> Result<Vec<MachineMemory>, E>,
) -> Option<String>
where
    E: Display,
{
    if !enabled || is_legacy_ssh || is_warpified_ssh {
        return None;
    }

    let memories = match load_memories() {
        Ok(memories) => memories,
        Err(err) => {
            log::warn!("machine memory index load failed: {err}");
            return None;
        }
    };
    build_machine_index(&memories)
}

fn build_machine_index(memories: &[MachineMemory]) -> Option<String> {
    if memories.is_empty() {
        return None;
    }

    let mut memories = memories.iter().collect::<Vec<_>>();
    memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let index = memories
        .into_iter()
        .take(INDEX_MAX_MACHINES)
        .map(|memory| {
            let summary = memory
                .content
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .chars()
                .take(INDEX_SUMMARY_MAX_CHARS)
                .collect::<String>();
            format!("- {}: {summary}", memory.machine_key)
        })
        .collect::<Vec<_>>()
        .join("\n");

    Some(index.chars().take(INDEX_MAX_CHARS).collect())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
