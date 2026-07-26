use super::*;

fn carrier_message(name: &str) -> api::Message {
    api::Message {
        id: "message-1".to_owned(),
        task_id: "task-1".to_owned(),
        server_message_data: format!("{name}\n{{\"content\":\"memory\"}}"),
        citations: Vec::new(),
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: "call-1".to_owned(),
            tool: None,
        })),
        request_id: "request-1".to_owned(),
        timestamp: None,
    }
}

fn conversion_params(task_id: &TaskId) -> ConversionParams<'_> {
    ConversionParams {
        task_id,
        current_todo_list: None,
        active_code_review: None,
    }
}

#[test]
fn machine_memory_carrier_has_visible_neutral_message() {
    let task_id = TaskId::new("task-1".to_owned());
    let output = carrier_message("update_machine_memory")
        .to_client_output_message(conversion_params(&task_id))
        .unwrap();

    let MaybeAIAgentOutputMessage::Message(output) = output else {
        panic!("machine memory carrier must have a client representation");
    };
    assert_eq!(output.to_string(), "Updating machine memory");
}

#[test]
fn unrelated_empty_tool_carrier_remains_hidden() {
    let task_id = TaskId::new("task-1".to_owned());
    let output = carrier_message("some_other_local_tool")
        .to_client_output_message(conversion_params(&task_id))
        .unwrap();

    assert!(matches!(
        output,
        MaybeAIAgentOutputMessage::NoClientRepresentation
    ));
}
