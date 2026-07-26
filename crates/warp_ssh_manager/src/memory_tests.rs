use chrono::Utc;
use diesel::connection::SimpleConnection;

use super::*;
use crate::repository::setup_in_memory;

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
}

#[test]
fn migration_round_trips_down_and_up() {
    let mut conn = setup_in_memory();
    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-07-26-000000_add_ssh_machine_memories/down.sql"
    ))
    .unwrap();
    assert!(MachineMemoryRepository::get(&mut conn, "web-01:22").is_err());

    conn.batch_execute(include_str!(
        "../../persistence/migrations/2026-07-26-000000_add_ssh_machine_memories/up.sql"
    ))
    .unwrap();
    assert_eq!(
        MachineMemoryRepository::get(&mut conn, "web-01:22").unwrap(),
        None
    );
}
