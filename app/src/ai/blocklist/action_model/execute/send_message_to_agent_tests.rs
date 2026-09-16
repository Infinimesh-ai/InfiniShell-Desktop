use super::*;

#[test]
fn local_message_ids_are_stable_and_separate_each_recipient_and_sender() {
    let original = stable_message_id("parent", "action", "child");
    assert_eq!(original, stable_message_id("parent", "action", "child"));
    assert_ne!(original, stable_message_id("parent", "action", "other"));
    assert_ne!(original, stable_message_id("other", "action", "child"));
    assert_ne!(
        stable_message_id("a", "bc", "d"),
        stable_message_id("ab", "c", "d")
    );
}
