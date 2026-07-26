//! 每台 SSH 机器的 AI 记忆数据层。

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sqlite::SqliteConnection;
use persistence::model::{NewSshMachineMemory, SshMachineMemoryRow};
use persistence::schema::ssh_machine_memories;
use thiserror::Error;

pub const MAX_MEMORY_CHARS: usize = 16_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineMemory {
    pub machine_key: String,
    pub content: String,
    pub hostname_alias: Option<String>,
    pub ssh_node_id: Option<String>,
    pub last_review_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum MachineMemoryRepositoryError {
    #[error("database error: {0}")]
    Db(#[from] DieselError),
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
            .find(machine_key)
            .select(SshMachineMemoryRow::as_select())
            .first(conn)
            .optional()?;
        row.map(memory_from_row).transpose()
    }

    /// 不存在则插入；存在则只更新 content 与 updated_at。
    pub fn upsert_content(
        conn: &mut SqliteConnection,
        machine_key: &str,
        content: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        let content = truncate_content(content);
        let now = Utc::now().to_rfc3339();
        let row = NewSshMachineMemory {
            machine_key,
            content: &content,
            hostname_alias: None,
            ssh_node_id: None,
            last_review_at: None,
            created_at: &now,
            updated_at: &now,
        };
        diesel::insert_into(ssh_machine_memories::table)
            .values(&row)
            .on_conflict(ssh_machine_memories::machine_key)
            .do_update()
            .set((
                ssh_machine_memories::content.eq(&content),
                ssh_machine_memories::updated_at.eq(&now),
            ))
            .execute(conn)?;
        Ok(())
    }

    pub fn set_hostname_alias(
        conn: &mut SqliteConnection,
        machine_key: &str,
        alias: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        Self::ensure_exists(conn, machine_key)?;
        diesel::update(ssh_machine_memories::table.find(machine_key))
            .set((
                ssh_machine_memories::hostname_alias.eq(alias),
                ssh_machine_memories::updated_at.eq(Utc::now().to_rfc3339()),
            ))
            .execute(conn)?;
        Ok(())
    }

    pub fn set_last_review_at(
        conn: &mut SqliteConnection,
        machine_key: &str,
        at: DateTime<Utc>,
    ) -> Result<(), MachineMemoryRepositoryError> {
        Self::ensure_exists(conn, machine_key)?;
        diesel::update(ssh_machine_memories::table.find(machine_key))
            .set((
                ssh_machine_memories::last_review_at.eq(at.to_rfc3339()),
                ssh_machine_memories::updated_at.eq(Utc::now().to_rfc3339()),
            ))
            .execute(conn)?;
        Ok(())
    }

    pub fn list_all(
        conn: &mut SqliteConnection,
    ) -> Result<Vec<MachineMemory>, MachineMemoryRepositoryError> {
        let rows = ssh_machine_memories::table
            .select(SshMachineMemoryRow::as_select())
            .load(conn)?;
        let mut memories = rows
            .into_iter()
            .map(memory_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(memories)
    }

    pub fn delete(
        conn: &mut SqliteConnection,
        machine_key: &str,
    ) -> Result<(), MachineMemoryRepositoryError> {
        diesel::delete(ssh_machine_memories::table.find(machine_key)).execute(conn)?;
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

fn memory_from_row(
    row: SshMachineMemoryRow,
) -> Result<MachineMemory, MachineMemoryRepositoryError> {
    let last_review_at = row
        .last_review_at
        .as_deref()
        .map(|value| parse_timestamp("last_review_at", value))
        .transpose()?;
    let updated_at = parse_timestamp("updated_at", &row.updated_at)?;
    Ok(MachineMemory {
        machine_key: row.machine_key,
        content: row.content,
        hostname_alias: row.hostname_alias,
        ssh_node_id: row.ssh_node_id,
        last_review_at,
        updated_at,
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
