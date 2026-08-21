//! Diesel CRUD over `infinishell_projects` + `infinishell_project_servers`。返回的全部是
//! `crate::types` 里的 plain 数据结构,把 ORM 细节挡在 crate 边界内。
//!
//! 与 `warp_ssh_manager::SshRepository` 同款约定:每个方法接受
//! `&mut SqliteConnection`,事务边界由调用方决定。

use chrono::Utc;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sqlite::SqliteConnection;
use persistence::model::{
    InfiniShellProjectRow, NewInfiniShellProject, NewInfiniShellProjectServer,
};
use persistence::schema::{infinishell_project_servers, infinishell_projects};
use thiserror::Error;
use uuid::Uuid;

use crate::types::Project;

#[derive(Debug, Error)]
pub enum ProjectRepositoryError {
    #[error("database error: {0}")]
    Db(#[from] DieselError),
    #[error("project not found: {0}")]
    NotFound(String),
}

/// 数据访问层。软删语义:`deleted_at` 非空的项目对所有读接口不可见。
pub struct ProjectRepository;

impl ProjectRepository {
    /// 列出全部未删除项目,按 sort_order、created_at 排序。
    pub fn list(conn: &mut SqliteConnection) -> Result<Vec<Project>, ProjectRepositoryError> {
        let rows: Vec<InfiniShellProjectRow> = infinishell_projects::table
            .filter(infinishell_projects::deleted_at.is_null())
            .order((
                infinishell_projects::sort_order.asc(),
                infinishell_projects::created_at.asc(),
            ))
            .load(conn)?;
        Ok(rows.into_iter().map(project_from_row).collect())
    }

    pub fn get(
        conn: &mut SqliteConnection,
        project_id: &str,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        let row: Option<InfiniShellProjectRow> = infinishell_projects::table
            .find(project_id)
            .filter(infinishell_projects::deleted_at.is_null())
            .first(conn)
            .optional()?;
        Ok(row.map(project_from_row))
    }

    /// 新建项目,sort_order 追加到当前最大 +1。
    pub fn create(
        conn: &mut SqliteConnection,
        name: &str,
    ) -> Result<Project, ProjectRepositoryError> {
        let id = Uuid::new_v4().to_string();
        let sort = next_sort_order(conn)?;
        diesel::insert_into(infinishell_projects::table)
            .values(NewInfiniShellProject {
                id: &id,
                name,
                git_url: None,
                root_path: None,
                rules: "",
                notes: "",
                default_profile_id: None,
                sort_order: sort,
            })
            .execute(conn)?;
        Self::get(conn, &id)?.ok_or_else(|| ProjectRepositoryError::NotFound(id))
    }

    /// 整体更新项目字段(不含 sort_order 之外的排序语义),updated_at 取当前时间。
    pub fn update(
        conn: &mut SqliteConnection,
        project: &Project,
    ) -> Result<(), ProjectRepositoryError> {
        let now = Utc::now().naive_utc();
        let affected = diesel::update(
            infinishell_projects::table
                .find(&project.id)
                .filter(infinishell_projects::deleted_at.is_null()),
        )
        .set((
            infinishell_projects::name.eq(&project.name),
            infinishell_projects::git_url.eq(project.git_url.as_deref()),
            infinishell_projects::root_path.eq(project.root_path.as_deref()),
            infinishell_projects::rules.eq(&project.rules),
            infinishell_projects::notes.eq(&project.notes),
            infinishell_projects::default_profile_id.eq(project.default_profile_id.as_deref()),
            infinishell_projects::sort_order.eq(project.sort_order),
            infinishell_projects::updated_at.eq(now),
        ))
        .execute(conn)?;
        if affected == 0 {
            return Err(ProjectRepositoryError::NotFound(project.id.clone()));
        }
        Ok(())
    }

    /// 软删项目并清空其服务器关联(关联无 tombstone 需求,直接硬删)。
    pub fn soft_delete(
        conn: &mut SqliteConnection,
        project_id: &str,
    ) -> Result<(), ProjectRepositoryError> {
        let now = Utc::now().naive_utc();
        let affected = diesel::update(
            infinishell_projects::table
                .find(project_id)
                .filter(infinishell_projects::deleted_at.is_null()),
        )
        .set(infinishell_projects::deleted_at.eq(now))
        .execute(conn)?;
        if affected == 0 {
            return Err(ProjectRepositoryError::NotFound(project_id.to_string()));
        }
        diesel::delete(
            infinishell_project_servers::table
                .filter(infinishell_project_servers::project_id.eq(project_id)),
        )
        .execute(conn)?;
        Ok(())
    }

    /// 覆盖式设置项目的服务器关联,顺序即 node_ids 的顺序。
    pub fn set_servers(
        conn: &mut SqliteConnection,
        project_id: &str,
        node_ids: &[String],
    ) -> Result<(), ProjectRepositoryError> {
        if Self::get(conn, project_id)?.is_none() {
            return Err(ProjectRepositoryError::NotFound(project_id.to_string()));
        }
        diesel::delete(
            infinishell_project_servers::table
                .filter(infinishell_project_servers::project_id.eq(project_id)),
        )
        .execute(conn)?;
        let values: Vec<NewInfiniShellProjectServer> = node_ids
            .iter()
            .enumerate()
            .map(|(index, node_id)| NewInfiniShellProjectServer {
                project_id,
                node_id,
                sort_order: index as i32,
            })
            .collect();
        diesel::insert_into(infinishell_project_servers::table)
            .values(&values)
            .execute(conn)?;
        Ok(())
    }

    /// 项目关联的服务器 node_id 列表,按 sort_order。
    /// 悬挂引用(ssh 节点已删)由调用方对照 `SshRepository::list_nodes` 过滤。
    pub fn servers_for_project(
        conn: &mut SqliteConnection,
        project_id: &str,
    ) -> Result<Vec<String>, ProjectRepositoryError> {
        let ids: Vec<String> = infinishell_project_servers::table
            .filter(infinishell_project_servers::project_id.eq(project_id))
            .order(infinishell_project_servers::sort_order.asc())
            .select(infinishell_project_servers::node_id)
            .load(conn)?;
        Ok(ids)
    }

    /// 包含某台服务器的全部未删除项目 id。
    pub fn projects_for_node(
        conn: &mut SqliteConnection,
        node_id: &str,
    ) -> Result<Vec<String>, ProjectRepositoryError> {
        let ids: Vec<String> = infinishell_project_servers::table
            .inner_join(infinishell_projects::table)
            .filter(infinishell_project_servers::node_id.eq(node_id))
            .filter(infinishell_projects::deleted_at.is_null())
            .order(infinishell_projects::sort_order.asc())
            .select(infinishell_projects::id)
            .load(conn)?;
        Ok(ids)
    }
}

fn project_from_row(row: InfiniShellProjectRow) -> Project {
    Project {
        id: row.id,
        name: row.name,
        git_url: row.git_url,
        root_path: row.root_path,
        rules: row.rules,
        notes: row.notes,
        default_profile_id: row.default_profile_id,
        sort_order: row.sort_order,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn next_sort_order(conn: &mut SqliteConnection) -> Result<i32, ProjectRepositoryError> {
    use diesel::dsl::max;
    let current: Option<i32> = infinishell_projects::table
        .select(max(infinishell_projects::sort_order))
        .first(conn)?;
    Ok(current.map_or(0, |value| value + 1))
}

#[cfg(test)]
pub(crate) fn setup_in_memory() -> SqliteConnection {
    use diesel::connection::SimpleConnection;
    let mut conn = SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute("PRAGMA foreign_keys = ON;").unwrap();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-08-11-000000_add_zap_projects/up.sql"
    ))
    .unwrap();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-08-20-000000_rename_zap_projects_to_infinishell/up.sql"
    ))
    .unwrap();
    conn
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
