use std::collections::HashMap;

use chrono::TimeZone as _;
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

/// 给消息补上 proto 时间戳,restored exchange 的 start_time 由此推导。
fn at(mut message: api::Message, seconds: i64) -> api::Message {
    message.timestamp = Some(prost_types::Timestamp { seconds, nanos: 0 });
    message
}

fn local_time(seconds: i64) -> DateTime<Local> {
    Local.timestamp_opt(seconds, 0).unwrap()
}

/// 不做窗口过滤的全量消息 id 集合,供只关注 digest 格式的测试使用。
fn all_message_ids(conversation: &AIConversation) -> HashSet<&str> {
    conversation
        .all_linearized_messages()
        .into_iter()
        .map(|message| message.id.as_str())
        .collect()
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

    let digest = build_session_digest([&conversation], &all_message_ids(&conversation));

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

    let digest = build_session_digest([&conversation], &all_message_ids(&conversation));

    assert_eq!(digest.chars().count(), DIGEST_MAX_CHARS);
    assert!(digest.starts_with("User:\n机"));
}

/// 本地交互(t=1000)在前、SSH 会话内交互(t=2000)在后的混合会话。
fn mixed_window_conversation() -> AIConversation {
    conversation(vec![
        at(user_query("user-1", "request-1", "Check local disk"), 1000),
        at(
            command_result("result-1", "request-1", "df -h", "local output"),
            1000,
        ),
        at(
            agent_output("agent-1", "request-1", "Local disk is fine."),
            1000,
        ),
        at(
            user_query("user-2", "request-2", "Inspect remote nginx"),
            2000,
        ),
        at(
            command_result("result-2", "request-2", "nginx -V", "remote output"),
            2000,
        ),
        at(
            agent_output("agent-2", "request-2", "Remote nginx is installed."),
            2000,
        ),
    ])
}

#[test]
fn window_gating_and_digest_exclude_exchanges_before_ssh_session() {
    let conversation = mixed_window_conversation();

    // SSH 会话从 t=1500 开始:只有 request-2 的交互落在窗口内。
    let session_message_ids =
        collect_session_scoped_message_ids(&[&conversation], local_time(1500))
            .expect("窗口内存在完整交互,应放行");
    let digest = build_session_digest([&conversation], &session_message_ids);

    assert!(digest.contains("User:\nInspect remote nginx"));
    assert!(digest.contains("$ nginx -V"));
    assert!(digest.contains("Assistant:\nRemote nginx is installed."));
    assert!(!digest.contains("Check local disk"));
    assert!(!digest.contains("df -h"));
    assert!(!digest.contains("Local disk is fine."));
}

#[test]
fn pure_manual_ssh_session_yields_no_review_gate() {
    let conversation = mixed_window_conversation();

    // SSH 会话从 t=3000 开始:期间没有任何 Agent 交互,不应放行(即不发 LLM 请求)。
    assert_eq!(
        collect_session_scoped_message_ids(&[&conversation], local_time(3000)),
        None
    );
}

#[test]
fn unfinished_exchange_in_window_does_not_open_gate() {
    // request-1(t=1000)是完整交互但在窗口前;request-2(t=2000)在窗口内,
    // 但只有用户输入、没有输出,restore 后按取消处理,不构成完整交互。
    let conversation = conversation(vec![
        at(user_query("user-1", "request-1", "Check local disk"), 1000),
        at(
            agent_output("agent-1", "request-1", "Local disk is fine."),
            1000,
        ),
        at(
            user_query("user-2", "request-2", "Inspect remote nginx"),
            2000,
        ),
    ]);

    assert_eq!(
        collect_session_scoped_message_ids(&[&conversation], local_time(1500)),
        None
    );
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
