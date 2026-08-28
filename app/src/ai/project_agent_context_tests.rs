use chrono::NaiveDateTime;

use super::*;

fn project(id: &str, name: &str) -> Project {
    Project {
        id: id.to_owned(),
        name: name.to_owned(),
        repositories: Vec::new(),
        root_path: None,
        rules: String::new(),
        notes: String::new(),
        default_profile_id: None,
        sort_order: 0,
        created_at: NaiveDateTime::default(),
        updated_at: NaiveDateTime::default(),
    }
}

fn host_info(node_id: &str) -> ProjectHostInfo {
    ProjectHostInfo {
        node_id: node_id.to_owned(),
        name: format!("{node_id}-name"),
        host: format!("{node_id}.example.com"),
        port: 22,
        username: "root".to_owned(),
    }
}

fn record(id: &str, host_node_ids: &[&str]) -> ProjectRecord {
    ProjectRecord {
        project: project(id, id),
        host_node_ids: host_node_ids.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn conversation_without_explicit_project_binding_returns_none() {
    assert_eq!(load_for_conversation(None), None);
}

#[test]
fn project_context_keeps_all_hosts() {
    let context = context_from_record(record("p1", &["node-1", "node-2"]), |node_ids| {
        hosts_from_lookup(node_ids, |node_id| Some(host_info(node_id)))
    });

    assert_eq!(
        context.projects[0]
            .hosts
            .iter()
            .map(|host| host.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["node-1", "node-2"]
    );
}

#[test]
fn dangling_host_references_are_filtered() {
    let node_ids = [
        "live".to_owned(),
        "dangling".to_owned(),
        "live-2".to_owned(),
    ];
    let hosts = hosts_from_lookup(&node_ids, |node_id| {
        (node_id != "dangling").then(|| host_info(node_id))
    });
    assert_eq!(
        hosts
            .iter()
            .map(|host| host.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["live", "live-2"]
    );
}

#[test]
fn repository_mappings_are_filtered_to_resolved_project_hosts() {
    let mut project = project("p1", "demo");
    project.repositories = vec![infinishell_projects::ProjectGitRepository {
        id: "repo-1".to_owned(),
        git_url: "git@example.com:demo.git".to_owned(),
        server_node_ids: vec!["node-1".to_owned(), "dangling".to_owned()],
    }];

    let entry = build_entry(project, vec![host_info("node-1")]);
    assert_eq!(
        entry.repositories,
        vec![ProjectRepositoryInfo {
            git_url: "git@example.com:demo.git".to_owned(),
            server_node_ids: vec!["node-1".to_owned()],
        }]
    );
}

#[test]
fn rules_and_notes_are_truncated_with_marker_on_unicode_boundaries() {
    let mut long_project = project("p1", "长文项目");
    long_project.rules = "规".repeat(RULES_MAX_CHARS + 1);
    long_project.notes = "注".repeat(NOTES_MAX_CHARS + 1);
    let entry = build_entry(long_project, Vec::new());

    assert_eq!(
        entry.rules,
        format!("{}…(截断)", "规".repeat(RULES_MAX_CHARS))
    );
    assert_eq!(
        entry.notes,
        format!("{}…(截断)", "注".repeat(NOTES_MAX_CHARS))
    );

    // 恰好达到上限时不追加标记。
    let mut exact_project = project("p2", "临界项目");
    exact_project.rules = "规".repeat(RULES_MAX_CHARS);
    exact_project.notes = "注".repeat(NOTES_MAX_CHARS);
    let exact_entry = build_entry(exact_project, Vec::new());
    assert_eq!(exact_entry.rules, "规".repeat(RULES_MAX_CHARS));
    assert_eq!(exact_entry.notes, "注".repeat(NOTES_MAX_CHARS));
}

#[test]
fn hosts_are_capped_per_project() {
    let hosts = (0..HOSTS_MAX_PER_PROJECT + 3)
        .map(|index| host_info(&format!("node-{index}")))
        .collect::<Vec<_>>();
    let entry = build_entry(project("p1", "p1"), hosts);
    assert_eq!(entry.hosts.len(), HOSTS_MAX_PER_PROJECT);
    assert_eq!(entry.hosts[0].node_id, "node-0");
}
