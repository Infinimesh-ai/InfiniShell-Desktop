use super::{
    AIAgentActionResultType, RequestCommandOutputResult, RunAgentsAgentOutcome,
    RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode, RunAgentsResult,
};

fn launched_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Launched {
            agent_id: format!("{name}-id"),
        },
        resolved_model_id: String::new(),
    }
}

fn failed_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Failed {
            error: "launch failed".to_string(),
        },
        resolved_model_id: String::new(),
    }
}

fn run_agents_result(agents: Vec<RunAgentsAgentOutcome>) -> AIAgentActionResultType {
    AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: RunAgentsLaunchedExecutionMode::Local,
        agents,
    })
}

#[test]
fn run_agents_is_successful_when_all_agents_launch() {
    let result = run_agents_result(vec![launched_agent("first"), launched_agent("second")]);

    assert!(result.is_successful());
    assert!(!result.is_failed());
}

#[test]
fn run_agents_is_successful_when_some_agents_launch() {
    let result = run_agents_result(vec![launched_agent("first"), failed_agent("second")]);
    assert!(result.is_successful());
    assert!(!result.is_failed());
}

#[test]
fn run_agents_is_failed_when_no_agents_launch() {
    let result = run_agents_result(vec![failed_agent("first"), failed_agent("second")]);

    assert!(!result.is_successful());
    assert!(result.is_failed());
}

#[test]
fn launch_failure_is_terminal_failure_and_never_cancellation_or_running_success() {
    let result =
        AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::LaunchFailed {
            command: "grok".to_owned(),
            reason: "CLI update is in progress".to_owned(),
        });
    assert!(result.is_failed());
    assert!(!result.is_successful());
    assert!(!result.is_cancelled());
    assert!(!result.triggers_server_subagent());
    assert!(result.should_trigger_request_upon_completion());
    assert_eq!(result.command_str(), Some("grok"));
}

#[test]
fn launch_failure_does_not_masquerade_as_a_supported_remote_proto_result() {
    let result = RequestCommandOutputResult::LaunchFailed {
        command: "claude".to_owned(),
        reason: "CLI update is in progress".to_owned(),
    };
    let converted: Result<warp_multi_agent_api::request::input::tool_call_result::Result, _> =
        result.try_into();
    assert!(matches!(
        converted,
        Err(crate::agent::convert::ConvertToAPITypeError::Other(_))
    ));
}
