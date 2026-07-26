use anyhow::anyhow;
use chrono::{TimeZone as _, Utc};

use super::*;

fn ssh_info() -> InteractiveSshCommand {
    InteractiveSshCommand {
        host: Some("root@Web-01".to_owned()),
        port: None,
    }
}

#[test]
fn disabled_or_non_legacy_session_skips_loading() {
    let info = ssh_info();
    assert_eq!(
        load_with(false, true, Some(&info), |_| -> anyhow::Result<_> {
            panic!("disabled setting must not access the database")
        }),
        None
    );
    assert_eq!(
        load_with(true, false, Some(&info), |_| -> anyhow::Result<_> {
            panic!("local sessions must not access the database")
        }),
        None
    );
}

#[test]
fn missing_memory_keeps_machine_identity_with_empty_content() {
    let info = ssh_info();
    let context = load_with(true, true, Some(&info), |_| Ok::<_, anyhow::Error>(None)).unwrap();
    assert_eq!(
        context,
        MachineMemoryContext {
            machine_key: "web-01:22".to_owned(),
            content: String::new(),
        }
    );
}

#[test]
fn loaded_memory_is_truncated_on_unicode_boundaries() {
    let info = ssh_info();
    let content = "机".repeat(INJECT_MAX_CHARS + 1);
    let context = load_with(true, true, Some(&info), |_| {
        Ok::<_, anyhow::Error>(Some(content))
    })
    .unwrap();
    assert_eq!(context.content.chars().count(), INJECT_MAX_CHARS);
    assert_eq!(context.content, "机".repeat(INJECT_MAX_CHARS));
}

#[test]
fn database_error_degrades_to_no_memory_context() {
    let info = ssh_info();
    let context = load_with(true, true, Some(&info), |_| {
        Err::<Option<String>, _>(anyhow!("no such table: ssh_machine_memories"))
    });
    assert_eq!(context, None);
}

fn memory_at(
    machine_key: impl Into<String>,
    content: impl Into<String>,
    updated_at: i64,
) -> MachineMemory {
    MachineMemory {
        machine_key: machine_key.into(),
        content: content.into(),
        hostname_alias: None,
        ssh_node_id: None,
        last_review_at: None,
        updated_at: Utc.timestamp_opt(updated_at, 0).single().unwrap(),
        deleted_at: None,
    }
}

#[test]
fn machine_index_skips_disabled_legacy_and_warpified_sessions() {
    for (enabled, is_legacy_ssh, is_warpified_ssh) in [
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        assert_eq!(
            load_index_with(
                enabled,
                is_legacy_ssh,
                is_warpified_ssh,
                || -> anyhow::Result<Vec<MachineMemory>> {
                    panic!("gated sessions must not access the database")
                }
            ),
            None
        );
    }
}

#[test]
fn machine_index_empty_table_and_database_error_degrade_to_none() {
    assert_eq!(
        load_index_with(true, false, false, || Ok::<_, anyhow::Error>(Vec::new())),
        None
    );
    assert_eq!(
        load_index_with(true, false, false, || {
            Err::<Vec<MachineMemory>, _>(anyhow!("database unavailable"))
        }),
        None
    );
}

#[test]
fn machine_index_loads_for_enabled_local_session() {
    assert_eq!(
        load_index_with(true, false, false, || {
            Ok::<_, anyhow::Error>(vec![memory_at("web-01:22", "Linux", 1)])
        }),
        Some("- web-01:22: Linux".to_owned())
    );
}

#[test]
fn machine_index_uses_first_non_empty_line_and_unicode_summary_limit() {
    let memories = vec![
        memory_at("older:2222", "\n\n  nginx lives in /opt/nginx  \nmore", 1),
        memory_at(
            "newest:22",
            format!(
                "\n  {}  \nignored",
                "机".repeat(INDEX_SUMMARY_MAX_CHARS + 1)
            ),
            2,
        ),
    ];

    assert_eq!(
        build_machine_index(&memories).unwrap(),
        format!(
            "- newest:22: {}\n- older:2222: nginx lives in /opt/nginx",
            "机".repeat(INDEX_SUMMARY_MAX_CHARS)
        )
    );
}

#[test]
fn machine_index_limits_machine_count() {
    let memories = (0..INDEX_MAX_MACHINES + 1)
        .map(|index| memory_at(format!("machine-{index}:22"), "Linux", index as i64))
        .collect::<Vec<_>>();

    let index = build_machine_index(&memories).unwrap();
    assert_eq!(index.lines().count(), INDEX_MAX_MACHINES);
    assert!(index.starts_with("- machine-30:22: Linux"));
    assert!(index.contains("- machine-1:22: Linux"));
    assert!(!index.contains("machine-0:22"));
}

#[test]
fn machine_index_total_limit_preserves_unicode_boundaries() {
    let memories = (0..INDEX_MAX_MACHINES)
        .map(|index| {
            memory_at(
                format!("machine-{index}:22"),
                "机".repeat(INDEX_SUMMARY_MAX_CHARS),
                index as i64,
            )
        })
        .collect::<Vec<_>>();

    let index = build_machine_index(&memories).unwrap();
    assert_eq!(index.chars().count(), INDEX_MAX_CHARS);
}
