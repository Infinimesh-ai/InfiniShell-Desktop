use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};

fn create_test_action(id: AIAgentActionId) -> AIAgentAction {
    AIAgentAction {
        id,
        task_id: TaskId::new("fake-task".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: "test".to_string(),
            is_read_only: None,
            is_risky: None,
            rationale: None,
            uses_pager: None,
            wait_until_completion: true,
            citations: vec![],
        },
        requires_result: false,
    }
}

fn generate_new_action_id() -> AIAgentActionId {
    AIAgentActionId::from(uuid::Uuid::new_v4().to_string())
}

#[test]
fn test_single_batch_done() {
    let mut actions = PendingPreprocessedActions::default();
    let action_id = generate_new_action_id();

    let test_action = create_test_action(action_id.clone());
    let preprocess_id = actions.insert_preprocess_action_batch(vec![test_action.clone()]);
    let result_actions = vec![test_action.clone()];

    let queued_actions = actions.handle_preprocess_actions_result(preprocess_id, result_actions);

    // Verify the action is returned
    assert_eq!(queued_actions.len(), 1);
    assert_eq!(queued_actions[0].id, action_id);

    // Verify the batch is removed
    assert_eq!(actions.0.len(), 0);
}

#[test]
fn test_multiple_batches() {
    let mut actions = PendingPreprocessedActions::default();

    // Insert three batches.
    let action_id1 = generate_new_action_id();
    let preprocess_id1 =
        actions.insert_preprocess_action_batch(vec![create_test_action(action_id1.clone())]);

    let action_id2 = generate_new_action_id();
    let preprocess_id2 =
        actions.insert_preprocess_action_batch(vec![create_test_action(action_id2.clone())]);

    let action_id3 = generate_new_action_id();
    let preprocess_id3 =
        actions.insert_preprocess_action_batch(vec![create_test_action(action_id3.clone())]);

    // Process the last batch. Should return nothing since the batch is not done.
    let test_action3 = create_test_action(action_id3.clone());
    let result_actions3 = vec![test_action3.clone()];
    let queued_actions3 = actions.handle_preprocess_actions_result(preprocess_id3, result_actions3);

    // Nothing should be returned: the first two batches are not done.
    assert_eq!(queued_actions3.len(), 0);
    assert_eq!(actions.0.len(), 3);

    // Process the second-to-last-batch.
    let test_action2 = create_test_action(action_id2.clone());
    let result_actions2 = vec![test_action2.clone()];

    let queued_actions = actions.handle_preprocess_actions_result(preprocess_id2, result_actions2);

    // Nothing should be returned: the first batch is not done.
    assert_eq!(queued_actions.len(), 0);
    assert_eq!(actions.0.len(), 3);

    // Process the first batch.
    let test_action1 = create_test_action(action_id1.clone());
    let result_actions1 = vec![test_action1.clone()];
    let queued_actions = actions.handle_preprocess_actions_result(preprocess_id1, result_actions1);

    // All of the batches are done--they should all be returned and the internal queue should be empty.
    assert_eq!(queued_actions.len(), 3);
    assert_eq!(actions.0.len(), 0);
    assert_eq!(queued_actions[0].id, action_id1);
    assert_eq!(queued_actions[1].id, action_id2);
    assert_eq!(queued_actions[2].id, action_id3);
}

#[test]
fn cancellation_retains_pending_and_completed_batches_in_order() {
    let mut batches = PendingPreprocessedActions::default();
    let first = create_test_action(AIAgentActionId::from("pending".to_owned()));
    let second = create_test_action(AIAgentActionId::from("completed".to_owned()));
    batches.insert_preprocess_action_batch(vec![first.clone()]);
    let second_batch = batches.insert_preprocess_action_batch(vec![second.clone()]);
    assert!(
        batches
            .handle_preprocess_actions_result(second_batch, vec![second.clone()])
            .is_empty()
    );

    assert_eq!(
        batches.into_actions().collect::<Vec<_>>(),
        vec![first, second]
    );
}
