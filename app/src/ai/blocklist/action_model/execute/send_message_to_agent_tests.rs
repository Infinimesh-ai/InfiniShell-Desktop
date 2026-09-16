use super::*;

#[cfg(feature = "local_fs")]
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
#[cfg(feature = "local_fs")]
use crate::ai::agent::{
    AIAgentAction, AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutput,
    AIAgentOutputMessage, AIAgentOutputStatus, Shared,
};
#[cfg(feature = "local_fs")]
use crate::ai::llms::LLMId;

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

#[cfg(feature = "local_fs")]
#[test]
fn parent_message_action_must_belong_to_current_user_turn_including_tool_continuations() {
    let mut conversation = AIConversation::new(false, false);
    let action = AIAgentAction {
        id: "message-action".to_owned().into(),
        task_id: conversation.get_root_task_id().clone(),
        action: AIAgentActionType::SendMessageToAgent {
            addresses: vec!["child".into()],
            subject: "update".into(),
            message: "追加指令".into(),
        },
        requires_result: true,
    };
    let exchange = |input, output| AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![input],
        output_status: AIAgentOutputStatus::Streaming { output },
        added_message_ids: Default::default(),
        start_time: chrono::Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-model"),
        cli_agent_model_id: LLMId::from("test-model"),
        computer_use_model_id: LLMId::from("test-model"),
        response_initiator: None,
    };
    conversation.append_root_exchange_for_test(exchange(
        AIAgentInput::ResumeConversation {
            context: Vec::new().into(),
        },
        None,
    ));
    let expected = local_parent_history_identity(&conversation).unwrap();
    conversation.append_root_exchange_for_test(exchange(
        AIAgentInput::MessagesReceivedFromAgents {
            messages: Vec::new(),
        },
        Some(Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage::action(
                "action-message".to_owned().into(),
                action.clone(),
            )],
            ..Default::default()
        })),
    ));
    assert_eq!(
        current_action_identity(&conversation, &action.id),
        Some(expected)
    );
    conversation.append_root_exchange_for_test(exchange(
        AIAgentInput::ResumeConversation {
            context: Vec::new().into(),
        },
        None,
    ));
    assert_eq!(current_action_identity(&conversation, &action.id), None);
    assert_eq!(
        current_action_identity(&conversation, &"unknown-action".to_owned().into()),
        None
    );
    conversation.set_status_for_test(ConversationStatus::Cancelled);
    assert_eq!(current_action_identity(&conversation, &action.id), None);
}
