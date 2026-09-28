use super::*;

fn ticket() -> Ticket {
    Ticket {
        id: Uuid::new_v4(),
        key: Uuid::new_v4(),
    }
}

#[test]
fn same_ticket_from_old_ssh_client_cannot_finish_current_pending() {
    let ticket = ticket();
    let attempt = Uuid::new_v4();
    let old_client = Arc::new(1u8);
    let current_client = Arc::new(2u8);
    let generation = attempt.to_string();

    assert!(!same_remote_grok_attempt(
        &ticket,
        attempt,
        &old_client,
        &ticket,
        Some(attempt),
        Some((&current_client, generation.as_str())),
    ));
    assert!(same_remote_grok_attempt(
        &ticket,
        attempt,
        &current_client,
        &ticket,
        Some(attempt),
        Some((&current_client, generation.as_str())),
    ));
}

#[test]
fn old_attempt_cannot_finish_new_pending_on_same_client_and_ticket() {
    let ticket = ticket();
    let client = Arc::new(1u8);
    let old_attempt = Uuid::new_v4();
    let current_attempt = Uuid::new_v4();
    let generation = current_attempt.to_string();

    assert!(!same_remote_grok_attempt(
        &ticket,
        old_attempt,
        &client,
        &ticket,
        Some(current_attempt),
        Some((&client, generation.as_str())),
    ));
    assert!(same_remote_grok_attempt(
        &ticket,
        current_attempt,
        &client,
        &ticket,
        Some(current_attempt),
        Some((&client, generation.as_str())),
    ));
}
