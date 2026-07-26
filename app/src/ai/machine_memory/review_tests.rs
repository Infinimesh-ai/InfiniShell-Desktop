use std::collections::HashMap;

use warp_multi_agent_api as api;

use super::*;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};

fn message(id: &str, request_id: &str, message: api::message::Message) -> api::Message {
    api::Message {
        id: id.to_owned(),
        task_id: "root-task".to_owned(),
        request_id: request_id.to_owned(),
        timestamp: None,
        server_message_data: String::new(),
        citations: Vec::new(),
        message: Some(message),
    }
}

fn user_query(id: &str, request_id: &str, query: &str) -> api::Message {
    message(
        id,
        request_id,
        api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_owned(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        }),
    )
}

fn agent_output(id: &str, request_id: &str, text: &str) -> api::Message {
    message(
        id,
        request_id,
        api::message::Message::AgentOutput(api::message::AgentOutput {
            text: text.to_owned(),
        }),
    )
}

fn command_result(id: &str, request_id: &str, command: &str, output: &str) -> api::Message {
    message(
        id,
        request_id,
        api::message::Message::ToolCallResult(api::message::ToolCallResult {
            tool_call_id: format!("call-{id}"),
            context: None,
            result: Some(api::message::tool_call_result::Result::RunShellCommand(
                #[allow(deprecated)]
                api::RunShellCommandResult {
                    command: command.to_owned(),
                    output: String::new(),
                    exit_code: 0,
                    result: Some(api::run_shell_command_result::Result::CommandFinished(
                        api::ShellCommandFinished {
                            command_id: format!("command-{id}"),
                            output: output.to_owned(),
                            exit_code: 0,
                        },
                    )),
                },
            )),
        }),
    )
}

fn conversation(messages: Vec<api::Message>) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_owned(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .unwrap()
}

#[test]
fn digest_joins_multiple_rounds_and_truncates_each_command_output() {
    let long_output = "界".repeat(COMMAND_OUTPUT_MAX_CHARS + 1);
    let conversation = conversation(vec![
        user_query("user-1", "request-1", "Inspect nginx"),
        command_result("result-1", "request-1", "nginx -V", &long_output),
        agent_output("agent-1", "request-1", "Nginx is installed under /opt."),
        user_query("user-2", "request-2", "Restart it"),
        command_result(
            "result-2",
            "request-2",
            "sudo /opt/nginx/sbin/nginx -s reload",
            "ok",
        ),
        agent_output("agent-2", "request-2", "Reload completed."),
    ]);

    let digest = build_session_digest([&conversation]);

    assert!(digest.contains("User:\nInspect nginx"));
    assert!(digest.contains("$ nginx -V"));
    assert!(digest.contains(&"界".repeat(COMMAND_OUTPUT_MAX_CHARS)));
    assert!(!digest.contains(&"界".repeat(COMMAND_OUTPUT_MAX_CHARS + 1)));
    assert!(digest.contains("Assistant:\nNginx is installed under /opt."));
    assert!(digest.contains("User:\nRestart it"));
    assert!(digest.contains("$ sudo /opt/nginx/sbin/nginx -s reload"));
    assert!(digest.contains("Assistant:\nReload completed."));
}

#[test]
fn digest_total_limit_preserves_unicode_boundaries() {
    let conversation = conversation(vec![user_query(
        "user-1",
        "request-1",
        &"机".repeat(DIGEST_MAX_CHARS + 1),
    )]);

    let digest = build_session_digest([&conversation]);

    assert_eq!(digest.chars().count(), DIGEST_MAX_CHARS);
    assert!(digest.starts_with("User:\n机"));
}

#[test]
fn parses_changed_review_response() {
    assert_eq!(
        parse_review_response(r###"{"changed":true,"memory":"## System Profile\nUbuntu"}"###),
        ParsedReviewResponse::Changed("## System Profile\nUbuntu".to_owned())
    );
}

#[test]
fn rejects_invalid_review_response() {
    assert_eq!(
        parse_review_response("```json\n{\"changed\":true}\n```"),
        ParsedReviewResponse::Invalid
    );
}

#[test]
fn skips_unchanged_review_response() {
    assert_eq!(
        parse_review_response(r#"{"changed":false,"memory":"unchanged"}"#),
        ParsedReviewResponse::Unchanged
    );
}
