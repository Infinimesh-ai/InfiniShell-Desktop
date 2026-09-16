//! 本地 CLI 子任务与消息工具复用既有动作权限和持久化执行链路。

use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use warp_multi_agent_api as api;

use super::OpenAiTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    summary: String,
    base_prompt: String,
    harness: String,
    #[serde(default)]
    model_id: String,
    agent_run_configs: Vec<ChildArgs>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    plan_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildArgs {
    name: String,
    prompt: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageArgs {
    addresses: Vec<String>,
    subject: String,
    message: String,
}

fn run_parameters() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "summary": {"type":"string"},
            "base_prompt": {"type":"string"},
            "harness": {"type":"string", "enum":["claude", "codex"]},
            "model_id": {"type":"string"},
            "plan_id": {"type":"string"},
            "skills": {"type":"array", "items":{"type":"string"}, "description":"Local SKILL.md paths for this task."},
            "agent_run_configs": {"type":"array", "minItems":1, "maxItems":8, "items":{
                "type":"object", "additionalProperties":false,
                "properties":{"name":{"type":"string"},"prompt":{"type":"string"},"title":{"type":"string"}},
                "required":["name","prompt"]
            }}
        }, "required":["summary","base_prompt","harness","agent_run_configs"]
    })
}

fn run_from_args(args: &str) -> Result<api::message::tool_call::Tool> {
    let args: RunArgs = serde_json::from_str(args)?;
    if args.agent_run_configs.is_empty() || args.agent_run_configs.len() > 8 {
        bail!("run_agents requires between 1 and 8 local children");
    }
    let mut names = HashSet::new();
    for child in &args.agent_run_configs {
        if child.name.trim().is_empty() || !names.insert(child.name.trim().to_lowercase()) {
            bail!("child names must be nonempty and unique");
        }
    }
    let variant = match args.harness.as_str() {
        "claude" => api::harness::Variant::ClaudeCode(api::harness::ClaudeCode {}),
        "codex" => api::harness::Variant::Codex(api::harness::Codex {}),
        _ => bail!("only verified local Claude and Codex adapters are available"),
    };
    Ok(api::message::tool_call::Tool::RunAgents(api::RunAgents {
        summary: args.summary,
        base_prompt: args.base_prompt,
        skills: args
            .skills
            .into_iter()
            .map(|path| api::SkillRef {
                skill_reference: Some(api::skill_ref::SkillReference::Path(path)),
            })
            .collect(),
        model_id: args.model_id,
        harness: Some(api::Harness {
            variant: Some(variant),
        }),
        agent_run_configs: args
            .agent_run_configs
            .into_iter()
            .map(|child| api::run_agents::AgentRunConfig {
                name: child.name,
                prompt: child.prompt,
                title: child.title,
                agent_identity_uid: String::new(),
                model_id: String::new(),
                harness: None,
                execution_mode: None,
            })
            .collect(),
        plan_id: args.plan_id,
        execution_mode: Some(api::run_agents::ExecutionModeOneOf::Local(
            api::run_agents::Local {},
        )),
    }))
}

fn message_parameters() -> Value {
    json!({"type":"object", "additionalProperties":false,
        "properties":{
            "addresses":{"type":"array","minItems":1,"maxItems":16,"uniqueItems":true,"items":{"type":"string"}},
            "subject":{"type":"string"},"message":{"type":"string"}
        },"required":["addresses","subject","message"]})
}

fn message_from_args(args: &str) -> Result<api::message::tool_call::Tool> {
    let args: MessageArgs = serde_json::from_str(args)?;
    let unique: HashSet<_> = args.addresses.iter().collect();
    if args.addresses.is_empty()
        || args.addresses.len() > 16
        || unique.len() != args.addresses.len()
        || args
            .addresses
            .iter()
            .any(|address| address.trim().is_empty())
    {
        bail!("message recipients must be between 1 and 16 unique local task IDs");
    }
    Ok(api::message::tool_call::Tool::SendMessageToAgent(
        api::SendMessageToAgent {
            addresses: args.addresses,
            subject: args.subject,
            message: args.message,
        },
    ))
}

fn run_result(result: &api::message::tool_call_result::Result) -> Option<Value> {
    let api::message::tool_call_result::Result::RunAgentsResult(result) = result else {
        return None;
    };
    Some(match &result.outcome {
        Some(api::run_agents_result::Outcome::Launched(launched)) => json!({
            "status":"launched", "agents":launched.agents.iter().map(|agent| match &agent.result {
                Some(api::run_agents_result::agent_outcome::Result::Launched(child)) => json!({"name":agent.name,"agent_id":child.agent_id,"status":"launched"}),
                Some(api::run_agents_result::agent_outcome::Result::Failed(failed)) => json!({"name":agent.name,"status":"failed","error":failed.error}),
                None => json!({"name":agent.name,"status":"unknown"}),
            }).collect::<Vec<_>>()
        }),
        Some(api::run_agents_result::Outcome::Denied(denied)) => {
            json!({"status":"denied","reason":denied.reason})
        }
        Some(api::run_agents_result::Outcome::Failure(failure)) => {
            json!({"status":"failed","error":failure.error})
        }
        None => json!({"status":"cancelled"}),
    })
}

fn message_result(result: &api::message::tool_call_result::Result) -> Option<Value> {
    let api::message::tool_call_result::Result::SendMessageToAgent(result) = result else {
        return None;
    };
    Some(match &result.result {
        Some(api::send_message_to_agent_result::Result::Success(success)) => {
            json!({"status":"acknowledged","message_id":success.message_id})
        }
        Some(api::send_message_to_agent_result::Result::Error(error)) => {
            json!({"status":"not_acknowledged","error":error.message,"retry":"inspect_saved_status_before_retrying"})
        }
        None => json!({"status":"cancelled"}),
    })
}

pub static RUN_AGENTS: OpenAiTool = OpenAiTool {
    name: "run_agents",
    description: "Start local Claude Code or Codex child tasks using the approved orchestration policy. Assign distinct scopes and names. A launched task is not a completed task. Reuse returned agent_id values for follow-up messages; do not spawn duplicate children. No cloud or remote execution is available through this tool.",
    parameters: run_parameters,
    from_args: run_from_args,
    result_to_json: run_result,
};

pub static SEND_MESSAGE: OpenAiTool = OpenAiTool {
    name: "send_message_to_agent",
    description: "Send messages to an active managed local parent or child task. A native_protocol receipt confirms native acceptance; an application_history receipt confirms the Oz parent saved the message for its next normal request, not that its model has read or processed it. Do not resend unconfirmed messages or create replacement children. Unrelated tasks and cloud recipients are rejected.",
    parameters: message_parameters,
    from_args: message_from_args,
    result_to_json: message_result,
};

#[cfg(test)]
#[path = "local_orchestration_tests.rs"]
mod tests;
