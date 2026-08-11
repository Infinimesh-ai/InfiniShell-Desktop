use ai::agent::action::AskUserQuestionType;

use super::*;
use crate::ai::agent::AIAgentOutputMessageType;

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
        fetched_memories: vec![],
    }
}

fn conversion_params(task_id: &TaskId) -> ConversionParams<'_> {
    ConversionParams {
        task_id,
        current_todo_list: None,
        active_code_review: None,
        skill_path_origin: &SkillPathOrigin::Local,
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

fn file_artifact_created_message(filepath: &str, description: &str) -> api::Message {
    api::Message {
        id: "message-id".to_string(),
        task_id: "task-id".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ArtifactEvent(
            api::message::ArtifactEvent {
                event: Some(api::message::artifact_event::Event::Created(
                    api::message::artifact_event::ArtifactCreated {
                        artifact: Some(
                            api::message::artifact_event::artifact_created::Artifact::File(
                                api::message::artifact_event::FileArtifact {
                                    artifact_uid: "artifact-uid".to_string(),
                                    filepath: filepath.to_string(),
                                    mime_type: "text/plain".to_string(),
                                    size_bytes: 42,
                                    description: description.to_string(),
                                },
                            ),
                        ),
                    },
                )),
            },
        )),
        request_id: "request-id".to_string(),
        timestamp: None,
        fetched_memories: vec![],
    }
}

fn build_multiple_choice_question(
    recommended_option_index: i32,
) -> api::ask_user_question::Question {
    api::ask_user_question::Question {
        question_id: "q1".to_string(),
        question: "Which option should we prefer?".to_string(),
        question_type: Some(
            api::ask_user_question::question::QuestionType::MultipleChoice(
                api::ask_user_question::MultipleChoice {
                    is_multiselect: false,
                    options: vec![
                        api::ask_user_question::Option {
                            label: "First".to_string(),
                        },
                        api::ask_user_question::Option {
                            label: "Second".to_string(),
                        },
                    ],
                    recommended_option_index,
                    supports_other: false,
                },
            ),
        ),
    }
}

#[test]
fn convert_api_question_ignores_negative_recommended_index() {
    let converted = convert_api_question(build_multiple_choice_question(-1))
        .expect("multiple choice questions should convert");

    let AskUserQuestionType::MultipleChoice { options, .. } = converted.question_type;
    assert_eq!(options.len(), 2);
    assert!(options.iter().all(|option| !option.recommended));
}

#[test]
fn convert_api_question_uses_zero_based_recommended_index_when_present() {
    let converted = convert_api_question(build_multiple_choice_question(0))
        .expect("multiple choice questions should convert");

    let AskUserQuestionType::MultipleChoice { options, .. } = converted.question_type;
    assert_eq!(options.len(), 2);
    assert!(options[0].recommended);
    assert!(!options[1].recommended);
}

fn extract_file_artifact_created(
    output: MaybeAIAgentOutputMessage,
) -> (String, String, Option<String>, i64) {
    let MaybeAIAgentOutputMessage::Message(output_message) = output else {
        panic!("expected output message");
    };
    let AIAgentOutputMessageType::ArtifactCreated(artifact) = output_message.message else {
        panic!("expected artifact created output message");
    };
    let crate::ai::agent::ArtifactCreatedData::File {
        filepath,
        filename,
        description,
        size_bytes,
        ..
    } = artifact
    else {
        panic!("expected file artifact created output message");
    };
    (filepath, filename, description, size_bytes)
}

#[test]
fn converts_file_artifact_created_message_with_filename() {
    let task_id = TaskId::new("task-id".to_string());
    let message =
        file_artifact_created_message("outputs/report.txt", "Build output for the latest run");

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
            skill_path_origin: &SkillPathOrigin::Local,
        })
        .expect("conversion should succeed");

    let (filepath, filename, description, size_bytes) = extract_file_artifact_created(output);

    assert_eq!(filepath, "outputs/report.txt");
    assert_eq!(filename, "report.txt");
    assert_eq!(
        description.as_deref(),
        Some("Build output for the latest run")
    );
    assert_eq!(size_bytes, 42);
}

#[test]
fn transfer_control_tool_call_converts_to_action_message() {
    let task_id = TaskId::new("task".to_string());
    let reason = "Please finish the interactive flow".to_string();
    let message = api::Message {
        id: "message".to_string(),
        task_id: "task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: "tool_call".to_string(),
            tool: Some(
                api::message::tool_call::Tool::TransferShellCommandControlToUser(
                    api::message::tool_call::TransferShellCommandControlToUser {
                        reason: reason.clone(),
                    },
                ),
            ),
        })),
        request_id: "req".to_string(),
        timestamp: None,
        fetched_memories: vec![],
    };

    let converted = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
            skill_path_origin: &SkillPathOrigin::Local,
        })
        .expect("transfer-control conversion should succeed");

    match converted {
        MaybeAIAgentOutputMessage::Message(output) => match output.message {
            AIAgentOutputMessageType::Action(action) => {
                assert_eq!(action.task_id, task_id);
                assert_eq!(
                    action.action,
                    AIAgentActionType::TransferShellCommandControlToUser { reason }
                );
                assert!(action.requires_result);
            }
            other => panic!("Expected action message, got {other:?}"),
        },
        MaybeAIAgentOutputMessage::NoClientRepresentation => {
            panic!("Expected transfer-control tool call to produce a client action")
        }
    }
}
