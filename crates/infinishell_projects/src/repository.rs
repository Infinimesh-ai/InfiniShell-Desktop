//! Diesel CRUD over `infinishell_projects`、项目仓库及服务器关联表。返回的全部是
//! `crate::types` 里的 plain 数据结构,把 ORM 细节挡在 crate 边界内。
//!
//! 与 `warp_ssh_manager::SshRepository` 同款约定:每个方法接受
//! `&mut SqliteConnection`,事务边界由调用方决定。

use std::collections::HashSet;

use chrono::Utc;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel::sqlite::SqliteConnection;
use persistence::model::{
    InfiniShellProjectRepositoryRow, InfiniShellProjectRow, NewInfiniShellProject,
    NewInfiniShellProjectRepository, NewInfiniShellProjectRepositoryServer,
    NewInfiniShellProjectServer,
};
use persistence::schema::{
    infinishell_project_repositories, infinishell_project_repository_servers,
    infinishell_project_servers, infinishell_projects,
};
use thiserror::Error;
use uuid::Uuid;

use crate::types::{Project, ProjectGitRepository};

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
        let mut projects = Vec::with_capacity(rows.len());
        for row in rows {
            let repositories = Self::repositories_for_project(conn, &row.id)?;
            projects.push(project_from_row(row, repositories));
        }
        Ok(projects)
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
        let Some(row) = row else {
            return Ok(None);
        };
        let repositories = Self::repositories_for_project(conn, &row.id)?;
        Ok(Some(project_from_row(row, repositories)))
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

    /// 整体更新项目字段与 Git 仓库映射,updated_at 取当前时间。
    pub fn update(
        conn: &mut SqliteConnection,
        project: &Project,
    ) -> Result<(), ProjectRepositoryError> {
        conn.transaction::<_, ProjectRepositoryError, _>(|conn| {
            let now = Utc::now().naive_utc();
            let legacy_git_url = project
                .repositories
                .first()
                .map(|repository| repository.git_url.as_str());
            let affected = diesel::update(
                infinishell_projects::table
                    .find(&project.id)
                    .filter(infinishell_projects::deleted_at.is_null()),
            )
            .set((
                infinishell_projects::name.eq(&project.name),
                infinishell_projects::git_url.eq(legacy_git_url),
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
            replace_repositories(conn, &project.id, &project.repositories)
        })
    }

    /// 软删项目并清空其服务器/仓库关联(关联无 tombstone 需求,直接硬删)。
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
        delete_repositories(conn, project_id)?;
        diesel::delete(
            infinishell_project_servers::table
                .filter(infinishell_project_servers::project_id.eq(project_id)),
        )
        .execute(conn)?;
        Ok(())
    }

    /// 覆盖式设置项目仓库及其服务器映射。仓库和映射顺序均按切片顺序保存。
    pub fn set_repositories(
        conn: &mut SqliteConnection,
        project_id: &str,
        repositories: &[ProjectGitRepository],
    ) -> Result<(), ProjectRepositoryError> {
        conn.transaction::<_, ProjectRepositoryError, _>(|conn| {
            if !project_exists(conn, project_id)? {
                return Err(ProjectRepositoryError::NotFound(project_id.to_string()));
            }
            replace_repositories(conn, project_id, repositories)?;
            let legacy_git_url = repositories
                .first()
                .map(|repository| repository.git_url.as_str());
            diesel::update(infinishell_projects::table.find(project_id))
                .set(infinishell_projects::git_url.eq(legacy_git_url))
                .execute(conn)?;
            Ok(())
        })
    }

    /// 项目的 Git 仓库列表及每个仓库的服务器映射。
    pub fn repositories_for_project(
        conn: &mut SqliteConnection,
        project_id: &str,
    ) -> Result<Vec<ProjectGitRepository>, ProjectRepositoryError> {
        let rows: Vec<InfiniShellProjectRepositoryRow> = infinishell_project_repositories::table
            .filter(infinishell_project_repositories::project_id.eq(project_id))
            .order(infinishell_project_repositories::sort_order.asc())
            .load(conn)?;
        let mut repositories = Vec::with_capacity(rows.len());
        for row in rows {
            let server_node_ids = infinishell_project_repository_servers::table
                .filter(infinishell_project_repository_servers::repository_id.eq(&row.id))
                .order(infinishell_project_repository_servers::sort_order.asc())
                .select(infinishell_project_repository_servers::node_id)
                .load(conn)?;
            repositories.push(ProjectGitRepository {
                id: row.id,
                git_url: row.git_url,
                server_node_ids,
            });
        }
        Ok(repositories)
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

fn project_from_row(
    row: InfiniShellProjectRow,
    repositories: Vec<ProjectGitRepository>,
) -> Project {
    Project {
        id: row.id,
        name: row.name,
        repositories,
        root_path: row.root_path,
        rules: row.rules,
        notes: row.notes,
        default_profile_id: row.default_profile_id,
        sort_order: row.sort_order,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn project_exists(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<bool, ProjectRepositoryError> {
    Ok(infinishell_projects::table
        .find(project_id)
        .filter(infinishell_projects::deleted_at.is_null())
        .select(infinishell_projects::id)
        .first::<String>(conn)
        .optional()?
        .is_some())
}

fn replace_repositories(
    conn: &mut SqliteConnection,
    project_id: &str,
    repositories: &[ProjectGitRepository],
) -> Result<(), ProjectRepositoryError> {
    delete_repositories(conn, project_id)?;

    let repository_values = repositories
        .iter()
        .enumerate()
        .map(|(index, repository)| NewInfiniShellProjectRepository {
            id: &repository.id,
            project_id,
            git_url: &repository.git_url,
            sort_order: index as i32,
        })
        .collect::<Vec<_>>();
    diesel::insert_into(infinishell_project_repositories::table)
        .values(&repository_values)
        .execute(conn)?;

    let mut mapping_values = Vec::new();
    for repository in repositories {
        let mut seen = HashSet::new();
        for node_id in &repository.server_node_ids {
            if seen.insert(node_id) {
                mapping_values.push(NewInfiniShellProjectRepositoryServer {
                    repository_id: &repository.id,
                    node_id,
                    sort_order: (seen.len() - 1) as i32,
                });
            }
        }
    }
    diesel::insert_into(infinishell_project_repository_servers::table)
        .values(&mapping_values)
        .execute(conn)?;
    Ok(())
}

fn delete_repositories(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<(), ProjectRepositoryError> {
    let repository_ids = infinishell_project_repositories::table
        .filter(infinishell_project_repositories::project_id.eq(project_id))
        .select(infinishell_project_repositories::id)
        .load::<String>(conn)?;
    if !repository_ids.is_empty() {
        diesel::delete(
            infinishell_project_repository_servers::table.filter(
                infinishell_project_repository_servers::repository_id.eq_any(&repository_ids),
            ),
        )
        .execute(conn)?;
    }
    diesel::delete(
        infinishell_project_repositories::table
            .filter(infinishell_project_repositories::project_id.eq(project_id)),
    )
    .execute(conn)?;
    Ok(())
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
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-08-26-000000_add_infinishell_project_repositories/up.sql"
    ))
    .unwrap();
    conn
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
