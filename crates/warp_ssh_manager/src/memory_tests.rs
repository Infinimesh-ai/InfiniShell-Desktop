use chrono::{DateTime, Utc};
use diesel::connection::SimpleConnection;

use super::*;
use crate::repository::{setup_in_memory, SyncMetaRepository};

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn resolves_user_prefixed_host_case_insensitively() {
    assert_eq!(
        resolve_machine_key(Some("root@Web-01"), None).as_deref(),
        Some("web-01:22")
    );
}

#[test]
fn uses_default_port_when_missing_or_invalid() {
    assert_eq!(
        resolve_machine_key(Some("web-01"), None).as_deref(),
        Some("web-01:22")
    );
    assert_eq!(
        resolve_machine_key(Some("web-01"), Some("invalid")).as_deref(),
        Some("web-01:22")
    );
}

#[test]
fn uses_explicit_port() {
    assert_eq!(
        resolve_machine_key(Some("10.0.0.5"), Some("2222")).as_deref(),
        Some("10.0.0.5:2222")
    );
}

#[test]
fn rejects_missing_or_blank_host() {
    assert_eq!(resolve_machine_key(None, Some("2222")), None);
    assert_eq!(resolve_machine_key(Some("   "), None), None);
}

#[test]
fn upsert_creates_memory() {
    let mut conn = setup_in_memory();
    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "## System\nLinux").unwrap();

    let memory = MachineMemoryRepository::get(&mut conn, "web-01:22")
        .unwrap()
        .unwrap();
    assert_eq!(memory.machine_key, "web-01:22");
    assert_eq!(memory.content, "## System\nLinux");
    assert_eq!(memory.deleted_at, None);
}

#[test]
fn upsert_overwrites_existing_content() {
    let mut conn = setup_in_memory();
    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "old").unwrap();
    MachineMemoryRepository::set_hostname_alias(&mut conn, "web-01:22", "web-prod").unwrap();
    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "new").unwrap();

    let memory = MachineMemoryRepository::get(&mut conn, "web-01:22")
        .unwrap()
        .unwrap();
    assert_eq!(memory.content, "new");
    assert_eq!(memory.hostname_alias.as_deref(), Some("web-prod"));
}

#[test]
fn upsert_truncates_content_by_character_count() {
    let mut conn = setup_in_memory();
    let content = "机".repeat(MAX_MEMORY_CHARS + 1);
    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", &content).unwrap();

    let memory = MachineMemoryRepository::get(&mut conn, "web-01:22")
        .unwrap()
        .unwrap();
    assert_eq!(memory.content.chars().count(), MAX_MEMORY_CHARS);
    assert_eq!(memory.content, "机".repeat(MAX_MEMORY_CHARS));
}

#[test]
fn get_returns_none_when_memory_does_not_exist() {
    let mut conn = setup_in_memory();
    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "missing:22").unwrap(),
        None
    );
}

#[test]
fn updates_metadata_lists_and_deletes_memory() {
    let mut conn = setup_in_memory();
    let reviewed_at = Utc::now();
    MachineMemoryRepository::set_hostname_alias(&mut conn, "web-01:22", "web-prod").unwrap();
    MachineMemoryRepository::set_last_review_at(&mut conn, "web-01:22", reviewed_at).unwrap();

    let memories = MachineMemoryRepository::list_all(&mut conn).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].hostname_alias.as_deref(), Some("web-prod"));
    assert_eq!(memories[0].last_review_at, Some(reviewed_at));

    MachineMemoryRepository::delete(&mut conn, "web-01:22").unwrap();
    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "web-01:22").unwrap(),
        None
    );
    assert_eq!(
        MachineMemoryRepository::list_all(&mut conn).unwrap(),
        vec![]
    );

    let sync_memories = MachineMemoryRepository::list_all_for_sync(&mut conn).unwrap();
    assert_eq!(sync_memories.len(), 1);
    assert_eq!(sync_memories[0].content, "");
    assert_eq!(
        sync_memories[0].deleted_at,
        Some(sync_memories[0].updated_at)
    );
}

#[test]
fn delete_missing_memory_creates_sync_visible_tombstone() {
    let mut conn = setup_in_memory();

    MachineMemoryRepository::delete(&mut conn, "missing:22").unwrap();

    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "missing:22").unwrap(),
        None
    );
    let memories = MachineMemoryRepository::list_all_for_sync(&mut conn).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].machine_key, "missing:22");
    assert_eq!(memories[0].content, "");
    assert_eq!(memories[0].deleted_at, Some(memories[0].updated_at));
}

#[test]
fn upsert_content_explicitly_resurrects_tombstone() {
    let mut conn = setup_in_memory();
    MachineMemoryRepository::delete(&mut conn, "web-01:22").unwrap();

    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "new memory").unwrap();

    let memory = MachineMemoryRepository::get(&mut conn, "web-01:22")
        .unwrap()
        .unwrap();
    assert_eq!(memory.content, "new memory");
    assert_eq!(memory.deleted_at, None);
}

#[test]
fn metadata_updates_do_not_resurrect_tombstone() {
    let mut conn = setup_in_memory();
    let reviewed_at = timestamp("2026-07-26T12:00:00Z");
    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "old").unwrap();
    MachineMemoryRepository::delete(&mut conn, "web-01:22").unwrap();

    MachineMemoryRepository::set_hostname_alias(&mut conn, "web-01:22", "web-prod").unwrap();
    MachineMemoryRepository::set_last_review_at(&mut conn, "web-01:22", reviewed_at).unwrap();

    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "web-01:22").unwrap(),
        None
    );
    let memory = MachineMemoryRepository::list_all_for_sync(&mut conn)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(memory.hostname_alias.as_deref(), Some("web-prod"));
    assert_eq!(memory.last_review_at, Some(reviewed_at));
    assert_ne!(memory.deleted_at, None);
}

#[test]
fn local_mutations_increment_sync_version() {
    let mut conn = setup_in_memory();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 0);

    MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "memory").unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 1);

    MachineMemoryRepository::set_hostname_alias(&mut conn, "web-01:22", "web-prod").unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 2);

    MachineMemoryRepository::set_last_review_at(
        &mut conn,
        "web-01:22",
        timestamp("2026-07-26T12:00:00Z"),
    )
    .unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 3);

    MachineMemoryRepository::delete(&mut conn, "web-01:22").unwrap();
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 4);
}

#[test]
fn mutation_rolls_back_when_sync_version_cannot_increment() {
    let mut conn = setup_in_memory();
    conn.batch_execute("DROP TABLE sync_meta;").unwrap();

    assert!(MachineMemoryRepository::upsert_content(&mut conn, "web-01:22", "memory").is_err());
    assert_eq!(
        MachineMemoryRepository::list_all_for_sync(&mut conn).unwrap(),
        vec![]
    );
}

#[test]
fn sync_upsert_preserves_fields_and_does_not_increment_version() {
    let mut conn = setup_in_memory();
    let reviewed_at = timestamp("2026-07-25T10:00:00Z");
    let updated_at = timestamp("2026-07-26T10:00:00Z");
    let deleted_at = timestamp("2026-07-26T09:00:00Z");
    let memory = MachineMemory {
        machine_key: "web-01:22".to_string(),
        content: String::new(),
        hostname_alias: Some("web-prod".to_string()),
        ssh_node_id: Some("node-1".to_string()),
        last_review_at: Some(reviewed_at),
        updated_at,
        deleted_at: Some(deleted_at),
    };

    MachineMemoryRepository::upsert_from_sync(&mut conn, &memory).unwrap();

    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 0);
    assert_eq!(
        MachineMemoryRepository::list_all_for_sync(&mut conn).unwrap(),
        vec![memory]
    );
    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "web-01:22").unwrap(),
        None
    );
}

#[test]
fn sync_upsert_truncates_content_by_character_count() {
    let mut conn = setup_in_memory();
    let memory = MachineMemory {
        machine_key: "web-01:22".to_string(),
        content: "机".repeat(MAX_MEMORY_CHARS + 1),
        hostname_alias: None,
        ssh_node_id: None,
        last_review_at: None,
        updated_at: timestamp("2026-07-26T10:00:00Z"),
        deleted_at: None,
    };

    MachineMemoryRepository::upsert_from_sync(&mut conn, &memory).unwrap();

    let stored = MachineMemoryRepository::get(&mut conn, "web-01:22")
        .unwrap()
        .unwrap();
    assert_eq!(stored.content.chars().count(), MAX_MEMORY_CHARS);
    assert_eq!(stored.content, "机".repeat(MAX_MEMORY_CHARS));
    assert_eq!(SyncMetaRepository::get_sync_version(&mut conn).unwrap(), 0);
}

#[test]
fn migration_round_trips_down_and_up() {
    let mut conn = setup_in_memory();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-07-26-010000_add_ssh_machine_memory_deleted_at/down.sql"
    ))
    .unwrap();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-07-26-000000_add_ssh_machine_memories/down.sql"
    ))
    .unwrap();
    assert!(MachineMemoryRepository::get(&mut conn, "web-01:22").is_err());

    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-07-26-000000_add_ssh_machine_memories/up.sql"
    ))
    .unwrap();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-07-26-010000_add_ssh_machine_memory_deleted_at/up.sql"
    ))
    .unwrap();
    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "web-01:22").unwrap(),
        None
    );
}
