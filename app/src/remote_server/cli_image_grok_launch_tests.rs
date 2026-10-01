use super::*;
use std::os::unix::fs::PermissionsExt;

fn legacy_reservation(directory: &Path, ticket: &Ticket, scope: &Scope) -> Reservation {
    Reservation {
        version: 1,
        id: ticket.id,
        key_sha256: digest(ticket.key.as_bytes()),
        host: scope.host.clone(),
        terminal_session: scope.terminal_session,
        cwd: directory.to_owned(),
        app_executable: directory.join("worker"),
        app_sha256: "unexecuted-test-worker".into(),
        notifications: None,
    }
}

#[test]
fn legacy_remote_ticket_remains_queryable_but_cannot_claim_new_launch() {
    let parent = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = parent.path().canonicalize().unwrap();
    let store = TicketStore::new(&root).unwrap();
    let ticket = Ticket {
        id: Uuid::new_v4(),
        key: Uuid::new_v4(),
    };
    let scope = Scope {
        host: "test-host".into(),
        terminal_session: 1,
        terminal_epoch: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    let directory = store.directory(&ticket).unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let reservation = legacy_reservation(&directory, &ticket, &scope);
    write_new(&directory.join("ticket.json"), &reservation).unwrap();
    let serialized = serde_json::to_value(&reservation).unwrap();
    assert!(serialized.get("notifications").is_none());
    assert!(reservation.supported_version());
    assert!(reservation.entry_notifications(&directory).is_err());
    let guard = store.lock(&scope, &ticket).unwrap();
    assert!(guard.launch_for_input().is_err());
    assert!(
        matches!(guard.status(&ticket).unwrap(), Reply::Launch { phase, .. } if phase == "reserved")
    );
    assert!(
        matches!(guard.cancel(&ticket).unwrap(), Reply::Launch { phase, .. } if phase == "cancelled")
    );
    assert!(!directory.join("entered.json").exists());
}

#[test]
fn remote_ticket_claim_revalidates_frozen_notifications() {
    let parent = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = parent.path().canonicalize().unwrap();
    let ticket = Ticket {
        id: Uuid::new_v4(),
        key: Uuid::new_v4(),
    };
    let scope = Scope {
        host: "test-host".into(),
        terminal_session: 1,
        terminal_epoch: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    let mut reservation = legacy_reservation(&root, &ticket, &scope);
    fs::write(&reservation.app_executable, b"private test worker").unwrap();
    fs::set_permissions(
        &reservation.app_executable,
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    reservation.version = 2;
    reservation.notifications = Some(
        NotificationPlan::create(&root, Uuid::new_v4(), &root, &reservation.app_executable)
            .unwrap(),
    );
    assert!(reservation.supported_version());
    reservation.entry_notifications(&root).unwrap();
    let path = root.join("notifications/plugin/hooks/notify.cjs");
    fs::write(path, b"changed notification").unwrap();
    assert!(reservation.entry_notifications(&root).is_err());
}

#[test]
fn remote_ticket_versions_cannot_upgrade_legacy_credentials() {
    let ticket = Ticket {
        id: Uuid::new_v4(),
        key: Uuid::new_v4(),
    };
    let scope = Scope {
        host: "test-host".into(),
        terminal_session: 1,
        terminal_epoch: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    let mut reservation = legacy_reservation(Path::new("/unused"), &ticket, &scope);
    reservation.version = 2;
    assert!(!reservation.supported_version());
    assert!(
        reservation
            .entry_notifications(Path::new("/unused"))
            .is_err()
    );
}
