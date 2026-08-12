use anyhow::anyhow;
use chrono::NaiveDateTime;

use super::*;

fn ssh_info() -> InteractiveSshCommand {
    InteractiveSshCommand {
        host: Some("root@Web-01".to_owned()),
        port: None,
    }
}

fn project(id: &str, name: &str) -> Project {
    Project {
        id: id.to_owned(),
        name: name.to_owned(),
        git_url: None,
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
fn disabled_or_non_legacy_session_skips_loading() {
    let info = ssh_info();
    for (enabled, is_legacy_ssh) in [(false, true), (true, false)] {
        assert_eq!(
            load_with(
                enabled,
                is_legacy_ssh,
                Some(&info),
                |_host, _port| -> anyhow::Result<Option<String>> {
                    panic!("gated sessions must not access the database")
                },
                |_node_id| -> anyhow::Result<Vec<ProjectRecord>> {
                    panic!("gated sessions must not access the database")
                },
                |_node_ids| panic!("gated sessions must not access the database"),
            ),
            None
        );
    }
}

#[test]
fn missing_connection_info_or_unmatched_node_returns_none() {
    assert_eq!(
        load_with(
            true,
            true,
            None,
            |_host, _port| -> anyhow::Result<Option<String>> {
                panic!("missing connection info must not access the database")
            },
            |_node_id| -> anyhow::Result<Vec<ProjectRecord>> {
                panic!("missing connection info must not access the database")
            },
            |_node_ids| Vec::new(),
        ),
        None
    );

    let info = ssh_info();
    assert_eq!(
        load_with(
            true,
            true,
            Some(&info),
            |_host, _port| Ok::<_, anyhow::Error>(None),
            |_node_id| -> anyhow::Result<Vec<ProjectRecord>> {
                panic!("unmatched host must not load projects")
            },
            |_node_ids| Vec::new(),
        ),
        None
    );
}

#[test]
fn node_lookup_normalizes_host_and_defaults_port() {
    let info = ssh_info();
    let mut seen = None;
    load_with(
        true,
        true,
        Some(&info),
        |host, port| {
            seen = Some((host.to_owned(), port));
            Ok::<_, anyhow::Error>(None)
        },
        |_node_id| Ok::<_, anyhow::Error>(Vec::new()),
        |_node_ids| Vec::new(),
    );
    assert_eq!(seen, Some(("web-01".to_owned(), 22)));
}

#[test]
fn database_errors_degrade_to_none() {
    let info = ssh_info();
    assert_eq!(
        load_with(
            true,
            true,
            Some(&info),
            |_host, _port| Err::<Option<String>, _>(anyhow!("no such table: ssh_servers")),
            |_node_id| Ok::<_, anyhow::Error>(Vec::new()),
            |_node_ids| Vec::new(),
        ),
        None
    );
    assert_eq!(
        load_with(
            true,
            true,
            Some(&info),
            |_host, _port| Ok::<_, anyhow::Error>(Some("node-1".to_owned())),
            |_node_id| Err::<Vec<ProjectRecord>, _>(anyhow!("no such table: zap_projects")),
            |_node_ids| Vec::new(),
        ),
        None
    );
}

#[test]
fn no_projects_for_node_returns_none() {
    let info = ssh_info();
    assert_eq!(
        load_with(
            true,
            true,
            Some(&info),
            |_host, _port| Ok::<_, anyhow::Error>(Some("node-1".to_owned())),
            |_node_id| Ok::<_, anyhow::Error>(Vec::new()),
            |_node_ids| Vec::new(),
        ),
        None
    );
}

#[test]
fn loads_projects_with_current_host_node_id() {
    let info = ssh_info();
    let context = load_with(
        true,
        true,
        Some(&info),
        |_host, _port| Ok::<_, anyhow::Error>(Some("node-1".to_owned())),
        |node_id| {
            assert_eq!(node_id, "node-1");
            Ok::<_, anyhow::Error>(vec![record("p1", &["node-1", "node-2"])])
        },
        |node_ids| hosts_from_lookup(node_ids, |node_id| Some(host_info(node_id))),
    )
    .unwrap();

    assert_eq!(context.current_host_node_id, "node-1");
    assert_eq!(context.projects.len(), 1);
    let entry = &context.projects[0];
    assert_eq!(entry.project_id, "p1");
    assert_eq!(
        entry
            .hosts
            .iter()
            .map(|host| host.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["node-1", "node-2"]
    );
}

#[test]
fn projects_are_capped() {
    let info = ssh_info();
    let records = (0..PROJECTS_MAX + 2)
        .map(|index| record(&format!("p{index}"), &[]))
        .collect::<Vec<_>>();
    let context = load_with(
        true,
        true,
        Some(&info),
        |_host, _port| Ok::<_, anyhow::Error>(Some("node-1".to_owned())),
        |_node_id| Ok::<_, anyhow::Error>(records),
        |_node_ids| Vec::new(),
    )
    .unwrap();
    assert_eq!(context.projects.len(), PROJECTS_MAX);
    assert_eq!(context.projects[0].project_id, "p0");
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

#[test]
fn normalize_host_port_parses_port_and_defaults_to_22() {
    assert_eq!(
        normalize_host_port(&InteractiveSshCommand {
            host: Some("root@Web-01".to_owned()),
            port: Some("2222".to_owned()),
        }),
        Some(("web-01".to_owned(), 2222))
    );
    assert_eq!(
        normalize_host_port(&ssh_info()),
        Some(("web-01".to_owned(), 22))
    );
    assert_eq!(
        normalize_host_port(&InteractiveSshCommand {
            host: None,
            port: Some("2222".to_owned()),
        }),
        None
    );
}
