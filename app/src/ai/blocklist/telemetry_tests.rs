use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest};
use ai::agent::action_result::{
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
};

use super::*;

fn request() -> RunAgentsRequest {
    RunAgentsRequest {
        summary: "summary".to_string(),
        base_prompt: "base".to_string(),
        skills: Vec::new(),
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: RunAgentsExecutionMode::Local,
        agent_run_configs: ["one", "two", "three"]
            .into_iter()
            .map(|name| RunAgentsAgentRunConfig {
                name: name.to_string(),
                prompt: String::new(),
                title: String::new(),
                agent_identity_uid: String::new(),
                model_id: String::new(),
            })
            .collect(),
        plan_id: "plan-id".to_string(),
        harness_auth_secret_name: None,
    }
}

#[test]
fn run_agents_completed_counts_actual_launches() {
    let request = request();
    let result = RunAgentsResult::Launched {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: RunAgentsLaunchedExecutionMode::Local,
        agents: vec![
            RunAgentsAgentOutcome {
                name: "one".to_string(),
                kind: RunAgentsAgentOutcomeKind::Launched {
                    agent_id: "agent-one".to_string(),
                },
                resolved_model_id: String::new(),
            },
            RunAgentsAgentOutcome {
                name: "two".to_string(),
                kind: RunAgentsAgentOutcomeKind::Failed {
                    error: "failed".to_string(),
                },
                resolved_model_id: String::new(),
            },
            RunAgentsAgentOutcome {
                name: "three".to_string(),
                kind: RunAgentsAgentOutcomeKind::Launched {
                    agent_id: "agent-three".to_string(),
                },
                resolved_model_id: String::new(),
            },
        ],
    };

    // Zap:遥测上报已删除,这里改为直接断言事件结构体字段
    // (上游断言的是 `TelemetryEvent::payload()` 的 JSON 形状)。
    let event = run_agents_completed_event(AIConversationId::new(), &request, &result);

    assert_eq!(event.plan_id.as_deref(), Some("plan-id"));
    assert_eq!(event.requested_agent_count, 3);
    assert_eq!(event.launched_agent_count, 2);
    assert_eq!(event.failed_agent_count, 1);
    assert!(matches!(event.result, RunAgentsResultKind::Launched));
    assert!(matches!(event.harness, OrchestrationHarnessKind::Oz));
    assert!(matches!(
        event.execution_mode,
        OrchestrationExecutionModeKind::Local
    ));
}

#[test]
fn run_agents_failure_counts_every_requested_agent_as_failed() {
    let request = request();
    let event = run_agents_completed_event(
        AIConversationId::new(),
        &request,
        &RunAgentsResult::Failure {
            error: "invalid".to_string(),
        },
    );

    assert_eq!(event.requested_agent_count, 3);
    assert_eq!(event.launched_agent_count, 0);
    assert_eq!(event.failed_agent_count, 3);
    assert!(matches!(event.result, RunAgentsResultKind::Failure));
}
