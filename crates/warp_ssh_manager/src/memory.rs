//! 每台 SSH 机器的 AI 记忆数据层。

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sqlite::SqliteConnection;
use persistence::model::{NewSshMachineMemory, SshMachineMemoryRow};
use persistence::schema::ssh_machine_memories;
use thiserror::Error;

use crate::repository::{SshRepositoryError, SyncMetaRepository};

pub const MAX_MEMORY_CHARS: usize = 16_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineMemory {
    pub machine_key: String,
    pub content: String,
    pub hostname_alias: Option<String>,
    pub ssh_node_id: Option<String>,
    pub last_review_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum MachineMemoryRepositoryError {
    #[error("database error: {0}")]
    Db(#[from] DieselError),
    #[error("sync metadata error: {0}")]
    SyncMeta(#[from] SshRepositoryError),
    #[error("invalid RFC3339 timestamp in db column `{column}`: {value}")]
    InvalidTimestamp { column: &'static str, value: String },
}

pub struct MachineMemoryRepository;

impl MachineMemoryRepository {
    pub fn get(
        conn: &mut SqliteConnection,
        machine_key: &str,
    ) -> Result<Option<MachineMemory>, MachineMemoryRepositoryError> {
        let row = ssh_machine_memories::table
            .filter(ssh_machine_memories::machine_key.eq(machine_key))
            .filter(ssh_machine_memories::deleted_at.is_null())
            .select(SshMachineMemoryRow::as_select())
            .first(conn)
            .optional()?;
        row.map(memory_from_row).transpose()
    }

    /// 不存在则插入；存在则更新 content 与 updated_at，并明确复活 tombstone。
    pub fn upsert_content(
        conn: &mut SqliteConnection,
        machine_key: &str,
        content: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        let content = truncate_content(content);
        conn.transaction::<_, MachineMemoryRepositoryError, _>(|conn| {
            let now = Utc::now().to_rfc3339();
            let not_deleted: Option<&str> = None;
            let row = NewSshMachineMemory {
                machine_key,
                content: &content,
                hostname_alias: None,
                ssh_node_id: None,
                last_review_at: None,
                created_at: &now,
                updated_at: &now,
                deleted_at: not_deleted,
            };
            diesel::insert_into(ssh_machine_memories::table)
                .values(&row)
                .on_conflict(ssh_machine_memories::machine_key)
                .do_update()
                .set((
                    ssh_machine_memories::content.eq(&content),
                    ssh_machine_memories::updated_at.eq(&now),
                    ssh_machine_memories::deleted_at.eq(not_deleted),
                ))
                .execute(conn)?;
            SyncMetaRepository::increment_sync_version(conn)?;
            Ok(())
        })
    }

    pub fn set_hostname_alias(
        conn: &mut SqliteConnection,
        machine_key: &str,
        alias: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        conn.transaction::<_, MachineMemoryRepositoryError, _>(|conn| {
            Self::ensure_exists(conn, machine_key)?;
            diesel::update(ssh_machine_memories::table.find(machine_key))
                .set((
                    ssh_machine_memories::hostname_alias.eq(alias),
                    ssh_machine_memories::updated_at.eq(Utc::now().to_rfc3339()),
                ))
                .execute(conn)?;
            SyncMetaRepository::increment_sync_version(conn)?;
            Ok(())
        })
    }

    pub fn set_last_review_at(
        conn: &mut SqliteConnection,
        machine_key: &str,
        at: DateTime<Utc>,
    ) -> Result<(), MachineMemoryRepositoryError> {
        conn.transaction::<_, MachineMemoryRepositoryError, _>(|conn| {
            Self::ensure_exists(conn, machine_key)?;
            diesel::update(ssh_machine_memories::table.find(machine_key))
                .set((
                    ssh_machine_memories::last_review_at.eq(at.to_rfc3339()),
                    ssh_machine_memories::updated_at.eq(Utc::now().to_rfc3339()),
                ))
                .execute(conn)?;
            SyncMetaRepository::increment_sync_version(conn)?;
            Ok(())
        })
    }

    pub fn list_all(
        conn: &mut SqliteConnection,
    ) -> Result<Vec<MachineMemory>, MachineMemoryRepositoryError> {
        let rows = ssh_machine_memories::table
            .filter(ssh_machine_memories::deleted_at.is_null())
            .select(SshMachineMemoryRow::as_select())
            .load(conn)?;
        memories_from_rows(rows)
    }

    /// 同步专用列表，包含 tombstone，避免旧设备把已删除记忆重新带回。
    pub fn list_all_for_sync(
        conn: &mut SqliteConnection,
    ) -> Result<Vec<MachineMemory>, MachineMemoryRepositoryError> {
        let rows = ssh_machine_memories::table
            .select(SshMachineMemoryRow::as_select())
            .load(conn)?;
        memories_from_rows(rows)
    }

    /// 写入 tombstone；即使本地缺行也创建删除记录，供其他设备合并。
    pub fn delete(
        conn: &mut SqliteConnection,
        machine_key: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        conn.transaction::<_, MachineMemoryRepositoryError, _>(|conn| {
            let now = Utc::now().to_rfc3339();
            let deleted_at = Some(now.as_str());
            diesel::insert_into(ssh_machine_memories::table)
                .values(NewSshMachineMemory {
                    machine_key,
                    content: "",
                    hostname_alias: None,
                    ssh_node_id: None,
                    last_review_at: None,
                    created_at: &now,
                    updated_at: &now,
                    deleted_at,
                })
                .on_conflict(ssh_machine_memories::machine_key)
                .do_update()
                .set((
                    ssh_machine_memories::content.eq(""),
                    ssh_machine_memories::updated_at.eq(&now),
                    ssh_machine_memories::deleted_at.eq(deleted_at),
                ))
                .execute(conn)?;
            SyncMetaRepository::increment_sync_version(conn)?;
            Ok(())
        })
    }

    /// 同步合并结果的原样写入，不修改本地 sync_version。
    pub fn upsert_from_sync(
        conn: &mut SqliteConnection,
        memory: &MachineMemory,
    ) -> Result<(), MachineMemoryRepositoryError> {
        let content = truncate_content(&memory.content);
        let now = Utc::now().to_rfc3339();
        let last_review_at = memory.last_review_at.as_ref().map(DateTime::to_rfc3339);
        let updated_at = memory.updated_at.to_rfc3339();
        let deleted_at = memory.deleted_at.as_ref().map(DateTime::to_rfc3339);
        diesel::insert_into(ssh_machine_memories::table)
            .values(NewSshMachineMemory {
                machine_key: &memory.machine_key,
                content: &content,
                hostname_alias: memory.hostname_alias.as_deref(),
                ssh_node_id: memory.ssh_node_id.as_deref(),
                last_review_at: last_review_at.as_deref(),
                created_at: &now,
                updated_at: &updated_at,
                deleted_at: deleted_at.as_deref(),
            })
            .on_conflict(ssh_machine_memories::machine_key)
            .do_update()
            .set((
                ssh_machine_memories::content.eq(&content),
                ssh_machine_memories::hostname_alias.eq(memory.hostname_alias.as_deref()),
                ssh_machine_memories::ssh_node_id.eq(memory.ssh_node_id.as_deref()),
                ssh_machine_memories::last_review_at.eq(last_review_at.as_deref()),
                ssh_machine_memories::updated_at.eq(&updated_at),
                ssh_machine_memories::deleted_at.eq(deleted_at.as_deref()),
            ))
            .execute(conn)?;
        Ok(())
    }

    fn ensure_exists(
        conn: &mut SqliteConnection,
        machine_key: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        let now = Utc::now().to_rfc3339();
        diesel::insert_into(ssh_machine_memories::table)
            .values(NewSshMachineMemory {
                machine_key,
                content: "",
                hostname_alias: None,
                ssh_node_id: None,
                last_review_at: None,
                created_at: &now,
                updated_at: &now,
                deleted_at: None,
            })
            .on_conflict(ssh_machine_memories::machine_key)
            .do_nothing()
            .execute(conn)?;
        Ok(())
    }
}

/// 将 SSH 命令里的原始 host 与 port 归一化为稳定的 `host:port` key。
pub fn resolve_machine_key(host: Option<&str>, port: Option<&str>) -> Option<String> {
    let host = host?.rsplit('@').next()?.trim().to_lowercase();
    if host.is_empty() {
        return None;
    }
    let port = port
        .and_then(|port| port.trim().parse::<u16>().ok())
        .unwrap_or(22);
    Some(format!("{host}:{port}"))
}

fn truncate_content(content: &str) -> String {
    content.chars().take(MAX_MEMORY_CHARS).collect()
}

fn memories_from_rows(
    rows: Vec<SshMachineMemoryRow>,
) -> Result<Vec<MachineMemory>, MachineMemoryRepositoryError> {
    let mut memories = rows
        .into_iter()
        .map(memory_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(memories)
}

fn memory_from_row(
    row: SshMachineMemoryRow,
) -> Result<MachineMemory, MachineMemoryRepositoryError> {
    let last_review_at = row
        .last_review_at
        .as_deref()
        .map(|value| parse_timestamp("last_review_at", value))
        .transpose()?;
    let updated_at = parse_timestamp("updated_at", &row.updated_at)?;
    let deleted_at = row
        .deleted_at
        .as_deref()
        .map(|value| parse_timestamp("deleted_at", value))
        .transpose()?;
    Ok(MachineMemory {
        machine_key: row.machine_key,
        content: row.content,
        hostname_alias: row.hostname_alias,
        ssh_node_id: row.ssh_node_id,
        last_review_at,
        updated_at,
        deleted_at,
    })
}

fn parse_timestamp(
    column: &'static str,
    value: &str,
) -> Result<DateTime<Utc>, MachineMemoryRepositoryError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| MachineMemoryRepositoryError::InvalidTimestamp {
            column,
            value: value.to_string(),
        })
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
