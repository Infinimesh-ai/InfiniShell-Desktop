use std::sync::Arc;

use super::{BlockType, Event};
use crate::terminal::model::block::SerializedBlock;

#[test]
fn pluggable_notification_debug_redacts_content() {
    let event = Event::PluggableNotification {
        title: Some("private title".to_owned()),
        body: "private body".to_owned(),
    };

    assert_eq!(format!("{event:?}"), "PluggableNotification");
}

#[test]
fn visible_bootstrap_blocks_are_not_restored() {
    let block = Arc::new(SerializedBlock::new_for_test(
        b"Welcome to Ubuntu".to_vec(),
        Vec::new(),
    ));

    assert!(
        BlockType::BootstrapVisible(block.clone())
            .session_restoration_data()
            .is_none()
    );
    assert!(
        BlockType::Background(block)
            .session_restoration_data()
            .is_some()
    );
}
