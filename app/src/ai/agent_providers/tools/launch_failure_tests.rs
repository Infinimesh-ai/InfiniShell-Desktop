use std::collections::HashMap;

use super::*;
use crate::ai::agent::AIAgentInput;
use crate::ai::agent::api::convert_conversation::restore_command_launch_failure;
use crate::ai::agent::task::TaskId;

fn failed_action() -> AIAgentActionResult {
    AIAgentActionResult {
        id: "launch-id".to_owned().into(),
        task_id: TaskId::new("task-id".to_owned()),
        result: AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::LaunchFailed {
                command: "grok".to_owned(),
                reason: "CLI 正在升级".to_owned(),
            },
        ),
    }
}

#[test]
fn launch_failure_uses_existing_byop_error_carrier_and_survives_restore() {
    let action = failed_action();
    let content = serialize_action_result(&action).unwrap();
    let value: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(value["status"], "error");
    assert_eq!(value["code"], "command_launch_failed");
    assert!(value.get("exit_code").is_none());
    assert!(value.get("command_id").is_none());
    let call = api::message::ToolCall {
        tool_call_id: action.id.to_string(),
        tool: Some(api::message::tool_call::Tool::RunShellCommand(
            api::message::tool_call::RunShellCommand {
                command: "grok".to_owned(),
                ..Default::default()
            },
        )),
    };
    let calls = HashMap::from([(call.tool_call_id.clone(), &call)]);
    let carrier = api::message::ToolCallResult {
        tool_call_id: action.id.to_string(),
        context: None,
        result: None,
    };
    let restored =
        restore_command_launch_failure(&action.task_id, &carrier, &content, &calls).unwrap();
    let AIAgentInput::ActionResult { result, .. } = restored else {
        panic!("应恢复启动失败记录");
    };
    assert_eq!(result.id, action.id);
    assert_eq!(result.result, action.result);
    assert!(result.result.is_failed());
    assert!(!result.result.is_cancelled());
}

#[test]
fn launch_failure_restore_requires_matching_shell_tool_and_exact_carrier_shape() {
    let action = failed_action();
    let content = serialize_action_result(&action).unwrap();
    let carrier = api::message::ToolCallResult {
        tool_call_id: "missing-call".to_owned(),
        context: None,
        result: None,
    };
    assert!(
        restore_command_launch_failure(&action.task_id, &carrier, &content, &HashMap::new())
            .is_none()
    );
    let call = api::message::ToolCall {
        tool_call_id: carrier.tool_call_id.clone(),
        tool: None,
    };
    let calls = HashMap::from([(call.tool_call_id.clone(), &call)]);
    assert!(restore_command_launch_failure(&action.task_id, &carrier, &content, &calls).is_none());
    for invalid in [
        r#"{"status":"completed","code":"command_launch_failed","command":"grok","error":"busy"}"#,
        r#"{"status":"error","code":"invalid_arguments","command":"grok","error":"busy"}"#,
        r#"{"status":"error","code":"command_launch_failed","command":[],"error":"busy"}"#,
        r#"{"status":"error","code":"command_launch_failed","command":"grok","error":"busy","exit_code":0}"#,
    ] {
        assert!(deserialize_command_launch_failure(invalid).is_none());
    }
}
