use super::workflow_description;

#[test]
fn workflow_description_truncates_at_utf8_boundary() {
    let content = format!("{}🚀-tail", "a".repeat(195));

    assert_eq!(
        workflow_description(&content),
        Some(format!("{}...", "a".repeat(195)))
    );
}
