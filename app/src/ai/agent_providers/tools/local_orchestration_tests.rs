use super::*;

#[test]
fn shared_orchestration_rejects_remote_fields_and_unverified_harnesses() {
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

#[test]
fn grok_local_request_preserves_identity_without_claiming_shared_api_support() {
    let args = json!({"summary":"并行审查","base_prompt":"中文\nReview changes",
        "harness":"grok","model_id":"fixed-model","plan_id":"plan-1",
        "skills":["/project/.agents/skills/review/SKILL.md"],
        "agent_run_configs":[{"name":"review","title":"审查","prompt":"保留\n两行"}]});
    let parsed = parse_local_run(&args.to_string()).unwrap();
    assert_eq!(parsed.harness, LocalHarness::Grok);
    assert_eq!(parsed.base_prompt, "中文\nReview changes");
    assert_eq!(parsed.agent_run_configs[0].prompt, "保留\n两行");
    assert_eq!(
        parsed.skills,
        vec!["/project/.agents/skills/review/SKILL.md"]
    );
    assert!(run_from_args(&args.to_string()).is_err());
    assert_eq!(
        local_run_parameters()["properties"]["harness"]["enum"],
        json!(["claude", "codex", "grok"])
    );
    assert_eq!(
        (RUN_AGENTS.parameters)()["properties"]["harness"]["enum"],
        json!(["claude", "codex"])
    );
}

#[test]
fn local_and_shared_parsers_preserve_claude_codex_launch_details() {
    for (harness, expected) in [
        ("claude", LocalHarness::Claude),
        ("codex", LocalHarness::Codex),
    ] {
        let args = json!({"summary":"review","base_prompt":"中文\nEnglish",
            "harness":harness,"model_id":"selected-model","plan_id":"plan-1",
            "skills":["/project/.agents/skills/review/SKILL.md"],
            "agent_run_configs":[{"name":"review","title":"标题","prompt":"检查修改"}]});
        let local = parse_local_run(&args.to_string()).unwrap();
        let api::message::tool_call::Tool::RunAgents(shared) =
            run_from_args(&args.to_string()).unwrap()
        else {
            panic!("共享接口返回类型错误");
        };
        assert_eq!(local.harness, expected);
        match expected {
            LocalHarness::Claude => assert!(matches!(
                shared.harness.as_ref().and_then(|h| h.variant.as_ref()),
                Some(api::harness::Variant::ClaudeCode(_))
            )),
            LocalHarness::Codex => assert!(matches!(
                shared.harness.as_ref().and_then(|h| h.variant.as_ref()),
                Some(api::harness::Variant::Codex(_))
            )),
            LocalHarness::Grok => panic!("Grok 不应进入共享接口兼容性测试"),
        }
        assert_eq!(shared.summary, local.summary);
        assert_eq!(shared.base_prompt, local.base_prompt);
        assert_eq!(shared.model_id, local.model_id);
        assert_eq!(shared.plan_id, local.plan_id);
        assert_eq!(
            shared.agent_run_configs[0].name,
            local.agent_run_configs[0].name
        );
        assert_eq!(
            shared.agent_run_configs[0].prompt,
            local.agent_run_configs[0].prompt
        );
        assert_eq!(
            shared.agent_run_configs[0].title,
            local.agent_run_configs[0].title
        );
        assert_eq!(
            shared.skills[0].skill_reference,
            Some(api::skill_ref::SkillReference::Path(
                local.skills[0].clone()
            ))
        );
        assert!(matches!(
            shared.execution_mode,
            Some(api::run_agents::ExecutionModeOneOf::Local(_))
        ));
    }
}

#[test]
fn local_harnesses_keep_child_limits_and_reject_identity_or_remote_overrides() {
    for harness in ["claude", "codex", "grok"] {
        let valid = json!({"summary":"审查","base_prompt":"检查","harness":harness,
            "agent_run_configs":[{"name":"review","prompt":"检查修改"}]});
        assert!(parse_local_run(&valid.to_string()).is_ok());
        let mut invalid = Vec::new();
        for children in [
            json!([]),
            json!(
                (0..9)
                    .map(|index| json!({"name":format!("child-{index}"),"prompt":"检查"}))
                    .collect::<Vec<_>>()
            ),
            json!([{"name":" ","prompt":"检查"}]),
            json!([{"name":" Review ","prompt":"检查"},{"name":"review","prompt":"复核"}]),
            json!([{"name":"review","prompt":"检查","harness":"codex"}]),
            json!([{"name":"review","prompt":"检查","execution_mode":"remote"}]),
        ] {
            let mut args = valid.clone();
            args["agent_run_configs"] = children;
            invalid.push(args);
        }
        for (field, value) in [
            ("parent_task_id", json!("claimed-parent")),
            ("worker_host", json!("remote")),
            ("skills", json!([{"cloud_id":"skill"}])),
            ("harness", json!("unknown")),
        ] {
            let mut args = valid.clone();
            args[field] = value;
            invalid.push(args);
        }
        for args in invalid {
            assert!(parse_local_run(&args.to_string()).is_err());
            if harness != "grok" {
                assert!(run_from_args(&args.to_string()).is_err());
            }
        }
    }
}
