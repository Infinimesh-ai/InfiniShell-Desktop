use super::*;

#[test]
fn local_orchestration_rejects_remote_fields_and_unverified_harnesses() {
    let mut args = json!({"summary":"并行检查","base_prompt":"检查代码","harness":"codex","agent_run_configs":[{"name":"review","prompt":"审查改动"}]});
    let api::message::tool_call::Tool::RunAgents(parsed) =
        run_from_args(&args.to_string()).unwrap()
    else {
        panic!("类型错误");
    };
    assert!(matches!(
        parsed.execution_mode,
        Some(api::run_agents::ExecutionModeOneOf::Local(_))
    ));
    args["harness"] = json!("grok");
    assert!(run_from_args(&args.to_string()).is_err());
    args["harness"] = json!("claude");
    args["worker_host"] = json!("remote");
    assert!(run_from_args(&args.to_string()).is_err());
}

#[test]
fn local_message_preserves_multiline_text_and_rejects_duplicate_recipients() {
    let mut args =
        json!({"addresses":["child"],"subject":"第二轮","message":"中文与 English\n'quote' `$()`"});
    let api::message::tool_call::Tool::SendMessageToAgent(parsed) =
        message_from_args(&args.to_string()).unwrap()
    else {
        panic!("类型错误");
    };
    assert_eq!(parsed.message, args["message"].as_str().unwrap());
    args["addresses"] = json!(["child", "child"]);
    assert!(message_from_args(&args.to_string()).is_err());
}

#[test]
fn unconfirmed_message_result_does_not_report_acknowledged() {
    let result =
        api::message::tool_call_result::Result::SendMessageToAgent(api::SendMessageToAgentResult {
            result: Some(api::send_message_to_agent_result::Result::Error(
                api::send_message_to_agent_result::Error {
                    message: "Delivery is unconfirmed".to_owned(),
                },
            )),
        });
    assert_eq!(
        message_result(&result).unwrap()["status"],
        json!("not_acknowledged")
    );
}

#[test]
fn local_run_keeps_skill_paths_for_native_launch_validation() {
    let args = json!({"summary":"审查","base_prompt":"检查修改","harness":"codex",
        "skills":["/project/.agents/skills/review/SKILL.md"],
        "agent_run_configs":[{"name":"review","prompt":"检查修改"}]});
    let api::message::tool_call::Tool::RunAgents(run) = run_from_args(&args.to_string()).unwrap()
    else {
        panic!("类型错误");
    };
    assert_eq!(run.skills.len(), 1);
    assert_eq!(
        run.skills[0].skill_reference,
        Some(api::skill_ref::SkillReference::Path(
            "/project/.agents/skills/review/SKILL.md".to_owned()
        ))
    );
}
