use diesel::connection::SimpleConnection;

use super::*;
use crate::types::{Project, ProjectGitRepository};

fn sample(conn: &mut diesel::SqliteConnection, name: &str) -> Project {
    ProjectRepository::create(conn, name).unwrap()
}

#[test]
fn migration_preserves_legacy_projects_and_server_links() {
    let mut conn = diesel::SqliteConnection::establish(":memory:").unwrap();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-08-11-000000_add_zap_projects/up.sql"
    ))
    .unwrap();
    conn.batch_execute(
        "INSERT INTO zap_projects (id, name, git_url, rules, notes, sort_order) \
         VALUES ('project-1', '生产集群', 'git@example.com:legacy/repo.git', '先冒烟', '保留备注', 3); \
         INSERT INTO zap_project_servers (project_id, node_id, sort_order) \
         VALUES ('project-1', 'node-1', 0);",
    )
    .unwrap();

    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-08-20-000000_rename_zap_projects_to_infinishell/up.sql"
    ))
    .unwrap();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-08-26-000000_add_infinishell_project_repositories/up.sql"
    ))
    .unwrap();

    let project = ProjectRepository::get(&mut conn, "project-1")
        .unwrap()
        .unwrap();
    assert_eq!(project.name, "生产集群");
    assert_eq!(project.rules, "先冒烟");
    assert_eq!(project.notes, "保留备注");
    assert_eq!(project.sort_order, 3);
    assert_eq!(
        project.repositories,
        vec![ProjectGitRepository {
            id: "project-1:legacy-repository".to_owned(),
            git_url: "git@example.com:legacy/repo.git".to_owned(),
            server_node_ids: Vec::new(),
        }]
    );
    assert_eq!(
        ProjectRepository::servers_for_project(&mut conn, "project-1").unwrap(),
        vec!["node-1"]
    );
}

#[test]
fn create_and_list_projects() {
    let mut conn = setup_in_memory();
    let a = sample(&mut conn, "生产集群");
    let b = sample(&mut conn, "预发环境");
    assert_eq!(a.sort_order, 0);
    assert_eq!(b.sort_order, 1);

    let all = ProjectRepository::list(&mut conn).unwrap();
    assert_eq!(
        all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        vec!["生产集群", "预发环境"]
    );
}

#[test]
fn update_roundtrips_all_fields() {
    let mut conn = setup_in_memory();
    let mut project = sample(&mut conn, "demo");
    project.name = "改名".into();
    project.repositories = vec![
        ProjectGitRepository {
            id: "repo-1".into(),
            git_url: "git@github.com:org/repo.git".into(),
            server_node_ids: vec!["node-a".into(), "node-b".into()],
        },
        ProjectGitRepository {
            id: "repo-2".into(),
            git_url: "https://github.com/org/docs.git".into(),
            server_node_ids: vec!["node-b".into()],
        },
    ];
    project.root_path = Some("/srv/app".into());
    project.rules = "发布前先跑冒烟".into();
    project.notes = "备注".into();
    project.default_profile_id = Some("profile-1".into());
    ProjectRepository::update(&mut conn, &project).unwrap();

    let loaded = ProjectRepository::get(&mut conn, &project.id)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.name, "改名");
    assert_eq!(loaded.repositories, project.repositories);
    assert_eq!(loaded.root_path.as_deref(), Some("/srv/app"));
    assert_eq!(loaded.rules, "发布前先跑冒烟");
    assert_eq!(loaded.notes, "备注");
    assert_eq!(loaded.default_profile_id.as_deref(), Some("profile-1"));
}

#[test]
fn set_repositories_replaces_rows_mappings_and_legacy_url() {
    use persistence::schema::infinishell_projects;

    let mut conn = setup_in_memory();
    let project = sample(&mut conn, "demo");
    let repositories = vec![
        ProjectGitRepository {
            id: "repo-a".into(),
            git_url: "git@example.com:a.git".into(),
            server_node_ids: vec!["node-b".into(), "node-a".into(), "node-b".into()],
        },
        ProjectGitRepository {
            id: "repo-b".into(),
            git_url: "git@example.com:b.git".into(),
            server_node_ids: Vec::new(),
        },
    ];
    ProjectRepository::set_repositories(&mut conn, &project.id, &repositories).unwrap();

    let loaded = ProjectRepository::repositories_for_project(&mut conn, &project.id).unwrap();
    assert_eq!(loaded[0].server_node_ids, vec!["node-b", "node-a"]);
    assert_eq!(loaded[1], repositories[1]);
    let legacy_url = infinishell_projects::table
        .find(&project.id)
        .select(infinishell_projects::git_url)
        .first::<Option<String>>(&mut conn)
        .unwrap();
    assert_eq!(legacy_url.as_deref(), Some("git@example.com:a.git"));

    ProjectRepository::set_repositories(&mut conn, &project.id, &repositories[1..]).unwrap();
    assert_eq!(
        ProjectRepository::repositories_for_project(&mut conn, &project.id).unwrap(),
        repositories[1..]
    );
}

#[test]
fn update_missing_project_errors() {
    let mut conn = setup_in_memory();
    let mut ghost = sample(&mut conn, "ghost");
    ProjectRepository::soft_delete(&mut conn, &ghost.id).unwrap();
    ghost.name = "无效".into();
    assert!(matches!(
        ProjectRepository::update(&mut conn, &ghost),
        Err(ProjectRepositoryError::NotFound(_))
    ));
}

#[test]
fn soft_delete_hides_project_and_clears_links() {
    let mut conn = setup_in_memory();
    let project = sample(&mut conn, "demo");
    ProjectRepository::set_servers(&mut conn, &project.id, &["node-a".into(), "node-b".into()])
        .unwrap();
    ProjectRepository::set_repositories(
        &mut conn,
        &project.id,
        &[ProjectGitRepository {
            id: "repo-a".into(),
            git_url: "git@example.com:a.git".into(),
            server_node_ids: vec!["node-a".into()],
        }],
    )
    .unwrap();
    ProjectRepository::soft_delete(&mut conn, &project.id).unwrap();

    assert!(ProjectRepository::list(&mut conn).unwrap().is_empty());
    assert!(
        ProjectRepository::get(&mut conn, &project.id)
            .unwrap()
            .is_none()
    );
    assert!(
        ProjectRepository::servers_for_project(&mut conn, &project.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        ProjectRepository::repositories_for_project(&mut conn, &project.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn set_servers_replaces_and_keeps_order() {
    let mut conn = setup_in_memory();
    let project = sample(&mut conn, "demo");
    ProjectRepository::set_servers(&mut conn, &project.id, &["b".into(), "a".into()]).unwrap();
    assert_eq!(
        ProjectRepository::servers_for_project(&mut conn, &project.id).unwrap(),
        vec!["b", "a"]
    );

    ProjectRepository::set_servers(&mut conn, &project.id, &["c".into()]).unwrap();
    assert_eq!(
        ProjectRepository::servers_for_project(&mut conn, &project.id).unwrap(),
        vec!["c"]
    );
}

#[test]
fn set_servers_on_missing_project_errors() {
    let mut conn = setup_in_memory();
    assert!(matches!(
        ProjectRepository::set_servers(&mut conn, "missing", &[]),
        Err(ProjectRepositoryError::NotFound(_))
    ));
}

#[test]
fn projects_for_node_excludes_deleted() {
    let mut conn = setup_in_memory();
    let a = sample(&mut conn, "A");
    let b = sample(&mut conn, "B");
    ProjectRepository::set_servers(&mut conn, &a.id, &["shared".into()]).unwrap();
    ProjectRepository::set_servers(&mut conn, &b.id, &["shared".into()]).unwrap();

    let mut owners = ProjectRepository::projects_for_node(&mut conn, "shared").unwrap();
    owners.sort();
    let mut expected = vec![a.id.clone(), b.id.clone()];
    expected.sort();
    assert_eq!(owners, expected);

    ProjectRepository::soft_delete(&mut conn, &b.id).unwrap();
    assert_eq!(
        ProjectRepository::projects_for_node(&mut conn, "shared").unwrap(),
        vec![a.id]
    );
}
