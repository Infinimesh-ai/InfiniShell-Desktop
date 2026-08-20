use std::ffi::OsString;

use clap::Parser;

use super::*;
use crate::agent::{AgentCommand, Harness};
use crate::memory_store::{MemoryCommand, MemoryStoreCommand};
// Zap Wave 7-2:`environment` CLI 随 cloud ambient agent 主体物理删。
// 上游的 artifact / federate / harness_support / integration / schedule / secret /
// task(`run message` / `finish-task` / `report-*`)子命令在本 fork 中不存在,
// 对应的解析测试与 `agent run-cloud` 相关测试一并删除。
// `agent create/update` 依赖已剥离的云端 named-agent CRUD,对应解析测试也不保留。

#[test]
fn identifies_worker_subcommands() {
    assert!(is_worker_invocation("minidump-server"));
    #[cfg(unix)]
    assert!(is_worker_invocation(&terminal_server_subcommand()));
    #[cfg(feature = "plugin_host")]
    assert!(is_worker_invocation("--plugin-host"));
    assert!(!is_worker_invocation("--prompt"));
}

#[test]
fn rust_ssh_broker_command_parses_a_loopback_endpoint() {
    let args = Args::try_parse_from([
        "warp",
        "rust-ssh-broker-command",
        "--endpoint",
        "127.0.0.1:49152",
        "--command",
        "true",
    ])
    .unwrap();
    let Some(Command::Worker(WorkerCommand::RustSshBrokerCommand(worker))) = args.command() else {
        panic!("期望 Rust SSH broker worker");
    };

    assert_eq!(worker.endpoint.as_deref(), Some("127.0.0.1:49152"));
    assert!(worker.control_path.is_none());
}

#[test]
fn rust_ssh_control_upload_parses_a_declared_stdin_size() {
    let args = Args::try_parse_from([
        "warp",
        "rust-ssh-broker-command",
        "--control-path",
        "/tmp/ssh-control",
        "--upload-path",
        "~/.infinishell/remote-server/archive.zip",
        "--stdin-size",
        "8192",
    ])
    .unwrap();
    let Some(Command::Worker(WorkerCommand::RustSshBrokerCommand(worker))) = args.command() else {
        panic!("期望 SSH ControlMaster upload worker");
    };

    assert_eq!(
        worker.control_path.as_deref(),
        Some(std::path::Path::new("/tmp/ssh-control"))
    );
    assert_eq!(worker.stdin_size, Some(8192));
}

#[test]
fn rust_ssh_worker_rejects_multiple_transports() {
    let result = Args::try_parse_from([
        "warp",
        "rust-ssh-broker-command",
        "--endpoint",
        "127.0.0.1:49152",
        "--control-path",
        "/tmp/ssh-control",
        "--command",
        "true",
    ]);

    assert!(result.is_err());
}

#[test]
#[serial_test::serial]
fn help_hides_api_key_env_value() {
    const API_KEY: &str = "warp-cli-test-api-key-NOT-REAL";

    let previous_api_key = set_env_var("WARP_API_KEY", API_KEY);

    let mut command = <Args as clap::CommandFactory>::command();
    let top_level_help = command.render_long_help().to_string();
    let runner_help = command
        .find_subcommand_mut("runner")
        .expect("runner subcommand exists")
        .render_long_help()
        .to_string();
    let args = Args::try_parse_from(["warp", "whoami"]).expect("API key env var should parse");

    restore_env_var("WARP_API_KEY", previous_api_key);

    for help in [&top_level_help, &runner_help] {
        assert!(
            help.contains("WARP_API_KEY"),
            "help should identify the API key environment variable:\n{help}"
        );
        assert!(
            !help.contains(API_KEY),
            "help should not reveal the API key environment value:\n{help}"
        );
    }
    assert_eq!(args.api_key().map(String::as_str), Some(API_KEY));
}

fn set_env_var(name: &str, value: &str) -> Option<OsString> {
    let previous = std::env::var_os(name);
    // Safety: tests that mutate process environment are marked `serial` so we
    // do not race with other environment readers/writers in this crate.
    unsafe { std::env::set_var(name, value) };
    previous
}

fn restore_env_var(name: &str, previous: Option<OsString>) {
    match previous {
        // Safety: tests that mutate process environment are marked `serial` so
        // we do not race with other environment readers/writers in this crate.
        Some(value) => unsafe { std::env::set_var(name, value) },
        // Safety: tests that mutate process environment are marked `serial` so
        // we do not race with other environment readers/writers in this crate.
        None => unsafe { std::env::remove_var(name) },
    }
}

#[test]
fn agent_run_accepts_model() {
    let args = Args::try_parse_from([
        "warp", "agent", "run", "--prompt", "hello", "--model", "gpt-4o",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.model.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn agent_run_accepts_hidden_bedrock_inference_role_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--bedrock-inference-role",
        "arn:aws:iam::123456789012:role/test",
        "--bedrock-role-region",
        "us-east-1",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.bedrock_inference_role.as_deref(),
        Some("arn:aws:iam::123456789012:role/test")
    );
    assert_eq!(run_args.bedrock_role_region.as_deref(), Some("us-east-1"));
}

#[test]
fn agent_run_rejects_bedrock_inference_role_without_region() {
    let err = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--bedrock-inference-role",
        "arn:aws:iam::123456789012:role/test",
    ])
    .expect_err("--bedrock-inference-role must require --bedrock-role-region");
    assert!(
        err.to_string().contains("--bedrock-role-region"),
        "expected error to reference --bedrock-role-region, got: {err}"
    );
}

#[test]
fn agent_run_rejects_bedrock_role_region_without_role() {
    let err = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--bedrock-role-region",
        "us-east-1",
    ])
    .expect_err("--bedrock-role-region must require --bedrock-inference-role");
    assert!(
        err.to_string().contains("--bedrock-inference-role"),
        "expected error to reference --bedrock-inference-role, got: {err}"
    );
}

#[test]
fn model_list_parses() {
    let args = Args::try_parse_from(["warp", "model", "list"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp model list` command");
    };
    let CliCommand::Model(model_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp model` command");
    };

    assert!(matches!(model_cmd, crate::model::ModelCommand::List));
}

#[test]
fn memory_store_list_parses() {
    let args = Args::try_parse_from(["warp", "memory-store", "list"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory-store list` command");
    };
    let CliCommand::MemoryStore(memory_store_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory-store` command");
    };

    assert!(matches!(memory_store_cmd, MemoryStoreCommand::List));
}

#[test]
fn memory_stores_alias_parses() {
    let args = Args::try_parse_from(["warp", "memory-stores", "list"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory-stores list` command");
    };
    let CliCommand::MemoryStore(memory_store_cmd) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory-stores` alias to parse as memory-store command");
    };

    assert!(matches!(memory_store_cmd, MemoryStoreCommand::List));
}

#[test]
fn memory_list_parses() {
    let args = Args::try_parse_from(["warp", "memory", "list", "store-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory list` command");
    };
    let CliCommand::Memory(MemoryCommand::List(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory list` command");
    };

    assert_eq!(args.store_uid, "store-123");
}

#[test]
fn memory_store_get_parses() {
    let args = Args::try_parse_from(["warp", "memory-store", "get", "store-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory-store get` command");
    };
    let CliCommand::MemoryStore(MemoryStoreCommand::Get(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory-store get` command");
    };

    assert_eq!(args.store_uid, "store-123");
}

#[test]
fn memory_store_get_store_alias_parses() {
    let args = Args::try_parse_from(["warp", "memory-store", "get-store", "store-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory-store get-store` command");
    };
    let CliCommand::MemoryStore(MemoryStoreCommand::Get(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory-store get-store` alias to parse as get command");
    };

    assert_eq!(args.store_uid, "store-123");
}

#[test]
fn memory_store_update_parses() {
    let args = Args::try_parse_from([
        "warp",
        "memory-store",
        "update",
        "store-123",
        "--description",
        "team memory store",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory-store update` command");
    };
    let CliCommand::MemoryStore(MemoryStoreCommand::Update(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory-store update` command");
    };

    assert_eq!(args.store_uid, "store-123");
    assert_eq!(args.description.as_deref(), Some("team memory store"));
}

#[test]
fn memory_store_update_store_alias_parses() {
    let args = Args::try_parse_from([
        "warp",
        "memory-store",
        "update-store",
        "store-123",
        "--description",
        "team memory store",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory-store update-store` command");
    };
    let CliCommand::MemoryStore(MemoryStoreCommand::Update(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory-store update-store` alias to parse as update command");
    };

    assert_eq!(args.store_uid, "store-123");
    assert_eq!(args.description.as_deref(), Some("team memory store"));
}

#[test]
fn memory_create_parses() {
    let args = Args::try_parse_from([
        "warp",
        "memory",
        "create",
        "store-123",
        "--content",
        "remember this",
        "--reason",
        "manual note",
        "--version",
        "v1",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory create` command");
    };
    let CliCommand::Memory(MemoryCommand::Create(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory create` command");
    };

    assert_eq!(args.store_uid, "store-123");
    assert_eq!(args.content, "remember this");
    assert_eq!(args.reason, "manual note");
    assert_eq!(args.version.as_deref(), Some("v1"));
}

#[test]
fn memory_update_parses() {
    let args = Args::try_parse_from([
        "warp",
        "memory",
        "update",
        "memory-123",
        "--store",
        "store-123",
        "--content",
        "updated memory",
        "--reason",
        "manual edit",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory update` command");
    };
    let CliCommand::Memory(MemoryCommand::Update(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory update` command");
    };

    assert_eq!(args.memory_uid, "memory-123");
    assert_eq!(args.store_uid, "store-123");
    assert_eq!(args.content, "updated memory");
    assert_eq!(args.reason, "manual edit");
}

#[test]
fn memory_delete_parses() {
    let args = Args::try_parse_from([
        "warp",
        "memory",
        "delete",
        "memory-123",
        "--store",
        "store-123",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory delete` command");
    };
    let CliCommand::Memory(MemoryCommand::Delete(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory delete` command");
    };

    assert_eq!(args.memory_uid, "memory-123");
    assert_eq!(args.store_uid, "store-123");
}

#[test]
fn memory_versions_parses() {
    let args = Args::try_parse_from([
        "warp",
        "memory",
        "versions",
        "memory-123",
        "--store",
        "store-123",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp memory versions` command");
    };
    let CliCommand::Memory(MemoryCommand::Versions(args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp memory versions` command");
    };

    assert_eq!(args.memory_uid, "memory-123");
    assert_eq!(args.store_uid, "store-123");
}

#[test]
fn legacy_memory_store_memory_commands_are_rejected() {
    for command in [
        "list-memories",
        "memories",
        "create-memory",
        "add-memory",
        "update-memory",
        "edit-memory",
        "delete-memory",
        "remove-memory",
        "list-versions",
        "versions",
    ] {
        let err = Args::try_parse_from(["warp", "memory-store", command, "memory-123"])
            .expect_err("legacy memory-store memory command should not parse");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}

#[test]
fn api_key_before_subcommand_parses() {
    // Regression test: `warp --api-key KEY <subcommand>` should work.
    // Previously the top-level [URLS] positional would swallow the subcommand
    // when --api-key preceded it.
    let args = Args::try_parse_from(["warp", "--api-key", "test-key", "whoami"]).unwrap();

    assert_eq!(args.api_key(), Some(&"test-key".to_string()));
    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp whoami` command");
    };
    assert!(matches!(boxed_cmd.as_ref(), CliCommand::Whoami));
}

#[test]
fn debug_before_subcommand_parses() {
    // Regression test: `warp --debug <subcommand>` should work.
    // Global flags like --debug must not prevent subcommand detection.
    let args = Args::try_parse_from(["warp", "--debug", "whoami"]).unwrap();

    assert!(args.debug());
    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp whoami` command");
    };
    assert!(matches!(boxed_cmd.as_ref(), CliCommand::Whoami));
}

#[test]
fn multiple_global_flags_before_subcommand_parse() {
    // Both --api-key and --debug before the subcommand should work.
    let args =
        Args::try_parse_from(["warp", "--api-key", "test-key", "--debug", "whoami"]).unwrap();

    assert_eq!(args.api_key(), Some(&"test-key".to_string()));
    assert!(args.debug());
    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp whoami` command");
    };
    assert!(matches!(boxed_cmd.as_ref(), CliCommand::Whoami));
}

#[test]
fn agent_run_accepts_file() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--file",
        "config.yaml",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.config_file.file.as_ref().and_then(|p| p.to_str()),
        Some("config.yaml")
    );
}

#[test]
fn agent_run_accepts_idle_on_complete_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--idle-on-complete",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.idle_on_complete,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            45 * 60
        )))
    );
}

#[test]
fn agent_run_accepts_idle_on_complete_duration() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--idle-on-complete",
        "10m",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(
        run_args.idle_on_complete,
        Some(humantime::Duration::from(std::time::Duration::from_secs(
            10 * 60
        )))
    );
}

#[test]
fn agent_run_rejects_without_prompt_or_skill() {
    let result = Args::try_parse_from(["warp", "agent", "run", "--model", "gpt-4o"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("prompt_group") || err_str.contains("required"));
}

#[test]
fn agent_run_accepts_prompt_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.prompt_arg.prompt.as_deref(), Some("hello"));
    assert!(run_args.prompt_arg.saved_prompt.is_none());
    assert!(run_args.skill.is_none());
}

#[test]
fn agent_run_accepts_saved_prompt_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--saved-prompt", "sp-123"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.prompt_arg.prompt.is_none());
    assert_eq!(run_args.prompt_arg.saved_prompt.as_deref(), Some("sp-123"));
    assert!(run_args.skill.is_none());
}

#[test]
fn agent_run_accepts_skill_only() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--skill", "my-skill"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.prompt_arg.prompt.is_none());
    assert!(run_args.skill.is_some());
}

#[test]
fn agent_run_accepts_prompt_and_skill() {
    let args = Args::try_parse_from([
        "warp", "agent", "run", "--prompt", "do stuff", "--skill", "my-skill",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.prompt_arg.prompt.as_deref(), Some("do stuff"));
    assert!(run_args.skill.is_some());
}

#[test]
fn agent_run_accepts_saved_prompt_and_skill() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--saved-prompt",
        "sp-1",
        "--skill",
        "my-skill",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert_eq!(run_args.prompt_arg.saved_prompt.as_deref(), Some("sp-1"));
    assert!(run_args.skill.is_some());
}

#[test]
fn agent_run_rejects_prompt_and_saved_prompt() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--saved-prompt",
        "sp-1",
    ]);
    assert!(result.is_err());
}

#[test]
fn run_command_is_removed() {
    let result = Args::try_parse_from(["warp", "run", "message"]);
    assert!(result.is_err());
}

// Zap Wave 7-2:environment_image_list_parses / environment_create_accepts_description /
// environment_create_description_max_length / environment_update_accepts_description /
// environment_update_accepts_remove_description 随 cloud ambient agent 主体子系统物理删。
// 上游的 schedule / artifact / integration 解析测试同随其子命令一并删除。

#[test]
fn agent_run_accepts_computer_use_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--computer-use",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(run_args.computer_use.computer_use);
    assert!(!run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), Some(true));
}

#[test]
fn agent_run_accepts_no_computer_use_flag() {
    let args = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--no-computer-use",
    ])
    .unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(!run_args.computer_use.computer_use);
    assert!(run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), Some(false));
}

#[test]
fn agent_run_rejects_both_computer_use_flags() {
    let result = Args::try_parse_from([
        "warp",
        "agent",
        "run",
        "--prompt",
        "hello",
        "--computer-use",
        "--no-computer-use",
    ]);

    assert!(result.is_err());
}

#[test]
fn agent_run_defaults_to_no_computer_use_override() {
    let args = Args::try_parse_from(["warp", "agent", "run", "--prompt", "hello"]).unwrap();

    let Some(Command::CommandLine(boxed_cmd)) = args.command else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = boxed_cmd.as_ref() else {
        panic!("Expected `warp agent run` command");
    };

    assert!(!run_args.computer_use.computer_use);
    assert!(!run_args.computer_use.no_computer_use);
    assert_eq!(run_args.computer_use.computer_use_override(), None);
}

#[test]
fn harness_parse_orchestration_harness_accepts_aliases() {
    assert_eq!(
        Harness::parse_orchestration_harness("claude-code"),
        Some(Harness::Claude)
    );
    assert_eq!(
        Harness::parse_orchestration_harness("open_code"),
        Some(Harness::OpenCode)
    );
}

#[test]
fn harness_parse_local_child_harness_rejects_oz() {
    assert_eq!(Harness::parse_local_child_harness("oz"), None);
    assert_eq!(
        Harness::parse_local_child_harness("opencode"),
        Some(Harness::OpenCode)
    );
}

#[test]
fn harness_parse_orchestration_harness_accepts_codex() {
    assert_eq!(
        Harness::parse_orchestration_harness("codex"),
        Some(Harness::Codex)
    );
}

#[test]
fn harness_parse_local_child_harness_accepts_codex() {
    assert_eq!(
        Harness::parse_local_child_harness("codex"),
        Some(Harness::Codex)
    );
}
