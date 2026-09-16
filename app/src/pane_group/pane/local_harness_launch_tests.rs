use std::ffi::OsString;
use std::fs;

#[cfg(feature = "local_fs")]
use futures::executor::block_on;
use tempfile::TempDir;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;

use super::{
    build_local_claude_child_command, build_local_codex_child_command,
    build_local_opencode_child_command, local_child_task_config, local_claude_child_prompt,
    normalize_local_child_harness, prepare_local_harness_child_launch,
    validate_local_harness_shell,
};
use crate::ai::agent_sdk::driver::OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV;
use crate::ai::ambient_agents::task::{HarnessConfig, normalize_orchestrator_agent_name};
use crate::ai::local_harness_setup::LOCAL_CODEX_HARNESS_DISABLED_MESSAGE;
#[cfg(feature = "local_fs")]
use crate::persistence::ModelEvent;
#[cfg(feature = "local_fs")]
use crate::persistence::local_cli_tasks::{LocalCliTask, LocalCliTaskState, load_task_generations};
use crate::terminal::shell::ShellType;

struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}
#[test]
fn local_claude_child_prompt_preserves_the_task_without_unavailable_mailbox_commands() {
    let prompt = "列出文件\nThen inspect changes";
    assert_eq!(local_claude_child_prompt(prompt), prompt);
}

#[cfg(feature = "local_fs")]
#[test]
fn local_parent_new_user_exchange_creates_new_generation_without_rebinding_existing_children() {
    let directory = TempDir::new().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("local-parent.sqlite"))
            .unwrap();
    let mut parent = LocalCliTask {
        version: 1,
        task_id: "parent".to_owned(),
        parent_task_id: None,
        parent_generation: None,
        harness: "oz".to_owned(),
        working_directory: "/project".to_owned(),
        config_json: serde_json::json!({"execution_kind":"local_parent", "history_identity":{
            "root_task_id":"root-task", "user_exchange_id":"user-turn-one",
        }})
        .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let mut child = parent.clone();
    child.task_id = "first-child".to_owned();
    child.parent_task_id = Some(parent.task_id.clone());
    child.harness = "codex".to_owned();
    child.config_json = "{}".to_owned();
    let first = block_on(super::persist_local_harness_child_launch(
        &writer.sender,
        parent.clone(),
        child.clone(),
        || async { Ok(()) },
    ))
    .unwrap();
    assert_eq!(first.parent_generation, Some(1));
    child.task_id = "second-child".to_owned();
    let same_turn = block_on(super::persist_local_harness_child_launch(
        &writer.sender,
        parent.clone(),
        child.clone(),
        || async { Ok(()) },
    ))
    .unwrap();
    assert_eq!(same_turn.parent_generation, Some(1));
    parent.config_json = serde_json::json!({"execution_kind":"local_parent", "history_identity":{
        "root_task_id":"root-task", "user_exchange_id":"user-turn-two",
    }})
    .to_string();
    child.task_id = "next-turn-child".to_owned();
    let next = block_on(super::persist_local_harness_child_launch(
        &writer.sender,
        parent,
        child,
        || async { Ok(()) },
    ))
    .unwrap();
    assert_eq!(next.parent_generation, Some(2));
    let generations = block_on(load_task_generations(&writer.sender, "parent".to_owned()).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(generations.len(), 2);
    assert_eq!(generations[0].state, LocalCliTaskState::Disconnected);
    assert_eq!(generations[1].state, LocalCliTaskState::Running);
    let before_late_launch = generations.clone();
    let late_parent = generations[0].clone();
    let mut late_child = first.clone();
    late_child.task_id = "late-old-turn-child".to_owned();
    late_child.parent_generation = None;
    let late_sender = writer.sender.clone();
    let released = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release_old = released.clone();
    let late = std::thread::spawn(move || {
        release_old.wait();
        block_on(super::persist_local_harness_child_launch(
            &late_sender,
            late_parent,
            late_child,
            || async { Err("源历史已经进入新用户轮".to_owned()) },
        ))
    });
    released.wait();
    assert!(late.join().unwrap().is_err());
    let after_late_launch =
        block_on(load_task_generations(&writer.sender, "parent".to_owned()).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(after_late_launch, before_late_launch);
    assert!(
        block_on(load_task_generations(&writer.sender, "late-old-turn-child".to_owned()).unwrap())
            .unwrap()
            .unwrap()
            .is_empty()
    );
    let first_saved =
        block_on(load_task_generations(&writer.sender, "first-child".to_owned()).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(first_saved, [first]);
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[cfg(feature = "local_fs")]
#[test]
fn parallel_children_reuse_one_committed_parent_without_duplicate_generations() {
    let directory = TempDir::new().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("parallel-parent.sqlite"))
            .unwrap();
    let parent = LocalCliTask {
        version: 1,
        task_id: "parallel-parent".to_owned(),
        parent_task_id: None,
        parent_generation: None,
        harness: "oz".to_owned(),
        working_directory: "/project".to_owned(),
        config_json: serde_json::json!({"execution_kind":"local_parent", "history_identity":{
            "root_task_id":"root-task", "user_exchange_id":"same-user-turn",
        }})
        .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|index| {
            let parent = parent.clone();
            let barrier = barrier.clone();
            let sender = writer.sender.clone();
            std::thread::spawn(move || {
                let mut child = parent.clone();
                child.task_id = format!("parallel-child-{index}");
                child.parent_task_id = Some(parent.task_id.clone());
                child.harness = "codex".to_owned();
                child.config_json = "{}".to_owned();
                barrier.wait();
                block_on(super::persist_local_harness_child_launch(
                    &sender,
                    parent,
                    child,
                    || async { Ok(()) },
                ))
            })
        })
        .collect();
    for thread in threads {
        let child = thread.join().unwrap().unwrap();
        assert_eq!(child.parent_generation, Some(1));
    }
    let generations =
        block_on(load_task_generations(&writer.sender, "parallel-parent".to_owned()).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0].state, LocalCliTaskState::Running);
    let mut wrong_generation = generations[0].clone();
    wrong_generation.generation = 2;
    assert!(!super::parent_launch_can_reuse(
        &generations[0],
        &wrong_generation
    ));
    let mut wrong_marker = generations[0].clone();
    wrong_marker.config_json = "{}".to_owned();
    assert!(!super::parent_launch_can_reuse(
        &generations[0],
        &wrong_marker
    ));
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[cfg(feature = "local_fs")]
#[test]
fn explicit_parent_message_starts_only_the_new_parent_generation_and_preserves_sent() {
    use crate::persistence::local_cli_tasks::{
        acknowledge_message, checkpoint_task, enqueue_message, load_messages, load_tasks,
    };
    use crate::persistence::model::{LocalCliMessage, LocalCliMessageState};
    use std::cell::Cell;

    let directory = TempDir::new().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("follow-up.sqlite")).unwrap();
    block_on(async {
        let parent = LocalCliTask {
            version: 1,
            task_id: "parent".into(),
            parent_task_id: None,
            parent_generation: None,
            harness: "oz".into(),
            working_directory: directory.path().to_string_lossy().into_owned(),
            config_json: serde_json::json!({"execution_kind":"local_parent", "history_identity":{
                "root_task_id":"root", "user_exchange_id":"first",
            }})
            .to_string(),
            native_session_id: None,
            generation: 1,
            revision: 0,
            state: LocalCliTaskState::Queued,
            result: None,
            terminal_evidence: None,
        };
        let child = LocalCliTask {
            task_id: "existing-child".into(),
            parent_task_id: Some(parent.task_id.clone()),
            parent_generation: Some(1),
            harness: "codex".into(),
            config_json: "{}".into(),
            ..parent.clone()
        };
        let child = super::persist_local_harness_child_launch(
            &writer.sender,
            parent.clone(),
            child,
            || async { Ok(()) },
        )
        .await
        .unwrap();
        let old_message = LocalCliMessage {
            version: 1,
            message_id: "old-message".into(),
            sender_task_id: parent.task_id.clone(),
            recipient_task_id: child.task_id.clone(),
            sender_generation: 1,
            recipient_generation: 1,
            subject: "follow-up".into(),
            body: "已经派发，尚未确认".into(),
            state: LocalCliMessageState::Queued,
            receipt_kind: None,
        };
        enqueue_message(&writer.sender, old_message.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        acknowledge_message(
            &writer.sender,
            old_message.message_id.clone(),
            child.task_id.clone(),
            1,
            LocalCliMessageState::Sent,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        let mut next = parent;
        next.config_json = serde_json::json!({"execution_kind":"local_parent", "history_identity":{
            "root_task_id":"root", "user_exchange_id":"second",
        }})
        .to_string();
        let source = super::persist_local_harness_parent(&writer.sender, next, || async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(source.generation, 2);
        assert_eq!(source.state, LocalCliTaskState::Running);
        let tasks = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tasks.len(), 2, "追加指令不能生成替代子任务");
        assert_eq!(
            tasks.iter().find(|task| task.task_id == child.task_id),
            Some(&child)
        );
        let saved = load_messages(&writer.sender, child.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].message_id, old_message.message_id);
        assert_eq!(saved[0].sender_generation, 1);
        assert_eq!(saved[0].state, LocalCliMessageState::Sent);
        assert_eq!(saved[0].receipt_kind, None);
        let new_message = LocalCliMessage {
            message_id: "new-message".into(),
            sender_generation: 2,
            ..old_message
        };
        enqueue_message(&writer.sender, new_message)
            .unwrap()
            .await
            .unwrap()
            .unwrap();

        // 登记的最后一次 await 后仍要校验当前用户轮，不能返回已失效的发送资格。
        let checks = Cell::new(0);
        let rejected = super::persist_local_harness_parent(&writer.sender, source.clone(), || {
            checks.set(checks.get() + 1);
            let current = checks.get() == 1;
            async move {
                current
                    .then_some(())
                    .ok_or_else(|| "用户轮已切换".to_owned())
            }
        })
        .await;
        assert!(rejected.is_err());
        assert_eq!(checks.get(), 2);
        let mut disconnected = source;
        disconnected.state = LocalCliTaskState::Disconnected;
        disconnected.revision += 1;
        checkpoint_task(&writer.sender, disconnected.clone(), Some(2))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let same_turn =
            super::persist_local_harness_parent(&writer.sender, disconnected.clone(), || async {
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(same_turn, disconnected, "同一用户轮不能复活已断开的父任务");
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let original = std::env::var_os(key);
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(key, value.into()) };
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var(self.key, original) };
        } else {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

fn write_fake_cli(bin_dir: &std::path::Path, name: &str) {
    let executable_name = if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    };
    let executable_path = bin_dir.join(executable_name);
    let script = if cfg!(windows) {
        "@echo off\r\n"
    } else {
        "#!/bin/sh\n"
    };

    fs::write(&executable_path, script).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&executable_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable_path, permissions).unwrap();
    }
}

#[test]
fn normalize_local_child_harness_accepts_supported_aliases() {
    assert_eq!(
        normalize_local_child_harness("claude"),
        Some(Harness::Claude)
    );
    assert_eq!(
        normalize_local_child_harness("claude-code"),
        Some(Harness::Claude)
    );
    assert_eq!(
        normalize_local_child_harness("claude_code"),
        Some(Harness::Claude)
    );
    assert_eq!(
        normalize_local_child_harness("opencode"),
        Some(Harness::OpenCode)
    );
    assert_eq!(
        normalize_local_child_harness("open-code"),
        Some(Harness::OpenCode)
    );
    assert_eq!(
        normalize_local_child_harness("open_code"),
        Some(Harness::OpenCode)
    );
    assert_eq!(normalize_local_child_harness("codex"), Some(Harness::Codex));
}

#[test]
fn normalize_local_child_harness_rejects_unsupported_values() {
    assert_eq!(normalize_local_child_harness("oz"), None);
    assert_eq!(normalize_local_child_harness("gemini"), None);
    assert_eq!(normalize_local_child_harness(""), None);
}

#[test]
fn validate_local_harness_shell_accepts_supported_shells() {
    assert_eq!(validate_local_harness_shell(Some(ShellType::Bash)), Ok(()));
    assert_eq!(validate_local_harness_shell(Some(ShellType::Zsh)), Ok(()));
    assert_eq!(validate_local_harness_shell(Some(ShellType::Fish)), Ok(()));
    assert_eq!(
        validate_local_harness_shell(Some(ShellType::PowerShell)),
        Ok(())
    );
}

#[test]
fn validate_local_harness_shell_requires_a_detected_shell() {
    assert_eq!(
        validate_local_harness_shell(None),
        Err(crate::t!("ambient-agent-local-harness-shell-required"))
    );
}

#[test]
fn build_local_claude_child_command_quotes_the_prompt() {
    let command =
        build_local_claude_child_command("hello world", uuid::Uuid::new_v4(), ShellType::Zsh);

    assert!(command.starts_with("claude --session-id "));
    assert!(command.ends_with(" -- 'hello world'"));
}

#[test]
fn build_local_opencode_child_command_quotes_the_prompt() {
    assert_eq!(
        build_local_opencode_child_command("hello world", ShellType::Fish),
        "opencode --prompt 'hello world'"
    );
}

#[test]
fn build_local_codex_child_command_quotes_the_prompt() {
    assert_eq!(
        build_local_codex_child_command("hello world", ShellType::Bash),
        "codex -- 'hello world'"
    );
}

#[test]
fn local_claude_child_keeps_multiline_prompt_out_of_cli_options() {
    let prompt = "--permission-mode bypassPermissions\n中文 'quoted' $HOME `echo unsafe`";
    let command = build_local_claude_child_command(prompt, uuid::Uuid::new_v4(), ShellType::Zsh);
    let args = shell_words::split(&command).unwrap();

    assert_eq!(args.len(), 5);
    assert_eq!(args[0], "claude");
    assert_eq!(args[1], "--session-id");
    assert!(uuid::Uuid::parse_str(&args[2]).is_ok());
    assert_eq!(args[3], "--");
    assert_eq!(args[4], prompt);
}

#[test]
fn local_codex_child_keeps_multiline_prompt_out_of_cli_options() {
    let prompt = "--dangerously-bypass-approvals-and-sandbox\n中文 'quoted' $HOME `echo unsafe`";
    let command = build_local_codex_child_command(prompt, ShellType::Bash);

    assert_eq!(
        shell_words::split(&command).unwrap(),
        ["codex", "--", prompt]
    );
}

#[test]
fn powershell_launch_preserves_literal_multiline_prompt_and_special_characters() {
    let prompt = "中文 C:\\项目 空格\\main.rs\n'单引号' `反引号` $(Get-Item .) $env:HOME";
    assert_eq!(
        build_local_codex_child_command(prompt, ShellType::PowerShell),
        "codex -- '中文 C:\\项目 空格\\main.rs\n''单引号'' `反引号` $(Get-Item .) $env:HOME'"
    );
    let session_id = uuid::Uuid::nil();
    assert_eq!(
        build_local_claude_child_command(prompt, session_id, ShellType::PowerShell),
        "claude --session-id 00000000-0000-0000-0000-000000000000 -- '中文 C:\\项目 空格\\main.rs\n''单引号'' `反引号` $(Get-Item .) $env:HOME'"
    );
}

#[tokio::test]
#[serial_test::serial]
#[cfg(unix)]
async fn local_claude_child_preserves_existing_global_configuration() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let config_dir = fake_home.path().join("custom claude");
    fs::create_dir_all(config_dir.join("plugins")).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");
    let claude_config = "{\n  \"hasCompletedOnboarding\": false,\n  \"projects\": {}\n}\n";
    let settings = "{\"permissions\":{\"defaultMode\":\"plan\"},\"enabledPlugins\":{\"warp@claude-code-warp\":false}}\n";
    let plugins = "{\"plugins\":{\"warp@claude-code-warp\":[{\"version\":\"0.1.0\"}]}}\n";
    fs::write(fake_home.path().join(".claude.json"), claude_config).unwrap();
    fs::write(config_dir.join("settings.json"), settings).unwrap();
    fs::write(config_dir.join("plugins/installed_plugins.json"), plugins).unwrap();
    let _home = EnvVarGuard::set("HOME", fake_home.path());
    let _config_dir = EnvVarGuard::set("CLAUDE_CONFIG_DIR", &config_dir);
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path());

    prepare_local_harness_child_launch(
        "检查工作区".to_owned(),
        "claude".to_owned(),
        None,
        Some("parent-run".to_owned()),
        None,
        Some(ShellType::Zsh),
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        fs::read_to_string(fake_home.path().join(".claude.json")).unwrap(),
        claude_config
    );
    assert_eq!(
        fs::read_to_string(config_dir.join("settings.json")).unwrap(),
        settings
    );
    assert_eq!(
        fs::read_to_string(config_dir.join("plugins/installed_plugins.json")).unwrap(),
        plugins
    );
}

#[tokio::test]
#[serial_test::serial]
#[cfg(unix)]
async fn local_claude_child_does_not_create_global_configuration() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let config_dir = fake_home.path().join(".claude");
    write_fake_cli(fake_bin_dir.path(), "claude");
    let _home = EnvVarGuard::set("HOME", fake_home.path());
    let _config_dir = EnvVarGuard::set("CLAUDE_CONFIG_DIR", &config_dir);
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path());

    prepare_local_harness_child_launch(
        "检查工作区".to_owned(),
        "claude".to_owned(),
        None,
        None,
        None,
        Some(ShellType::Bash),
        None,
    )
    .await
    .unwrap();

    assert!(!fake_home.path().join(".claude.json").exists());
    assert!(!config_dir.exists());
}

#[test]
fn local_child_task_config_records_supported_third_party_harnesses() {
    for harness in [Harness::Claude, Harness::OpenCode, Harness::Codex] {
        assert_eq!(
            local_child_task_config(harness, None),
            Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            }),
        );
    }
}

#[test]
fn local_child_task_config_stamps_orchestrator_name() {
    for harness in [Harness::Claude, Harness::OpenCode, Harness::Codex] {
        assert_eq!(
            local_child_task_config(harness, Some("frontend-tests".to_string())),
            Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
                name: Some("frontend-tests".to_string()),
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            }),
        );
    }
}

#[test]
fn local_child_task_config_trims_whitespace_only_name() {
    assert_eq!(
        local_child_task_config(Harness::Claude, Some("  frontend-tests  ".to_string())),
        Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
            name: Some("frontend-tests".to_string()),
            harness: Some(HarnessConfig::from_harness_type(Harness::Claude)),
            ..Default::default()
        }),
    );
    assert_eq!(
        local_child_task_config(Harness::Claude, Some("   ".to_string())),
        Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
            name: None,
            harness: Some(HarnessConfig::from_harness_type(Harness::Claude)),
            ..Default::default()
        }),
    );
}

#[test]
fn local_child_task_config_returns_none_for_oz_and_unknown() {
    assert!(local_child_task_config(Harness::Oz, Some("name".to_string())).is_none());
    assert!(local_child_task_config(Harness::Unknown, Some("name".to_string())).is_none());
}

#[test]
fn normalize_orchestrator_agent_name_trims_and_drops_empty() {
    assert_eq!(
        normalize_orchestrator_agent_name("frontend-tests"),
        Some("frontend-tests".to_string())
    );
    assert_eq!(
        normalize_orchestrator_agent_name("  frontend-tests  "),
        Some("frontend-tests".to_string())
    );
    assert_eq!(normalize_orchestrator_agent_name(""), None);
    assert_eq!(normalize_orchestrator_agent_name("   "), None);
    assert_eq!(normalize_orchestrator_agent_name("\t\n  "), None);
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_codex_child_launch_rejects_without_rewriting_global_codex_state() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "codex");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let result = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "codex".to_string(),
        None,
        Some("parent-run".to_string()),
        None,
        Some(ShellType::Zsh),
        None,
    )
    .await;

    match result {
        Ok(_) => panic!("disabled local codex should be rejected"),
        Err(err) => assert_eq!(err, LOCAL_CODEX_HARNESS_DISABLED_MESSAGE),
    }
    assert!(!fake_home.path().join(".codex").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_codex_child_launch_succeeds_when_testing_flag_is_enabled() {
    let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "codex");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let prepared = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "codex".to_string(),
        Some("ignored-model".to_string()),
        Some("parent-run".to_string()),
        None,
        Some(ShellType::Zsh),
        None,
    )
    .await
    .unwrap();

    assert_eq!(prepared.command, "codex -- 'hello world'");
    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from("ANTHROPIC_MODEL"))
    );
    // Zap:task_id 由本地生成,只校验它是一个非空 UUID。
    assert!(uuid::Uuid::parse_str(&prepared.run_id).is_ok());
    assert!(!fake_home.path().join(".codex").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_claude_child_merges_anthropic_model_env_var() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _claude_home = EnvVarGuard::set(
        "CLAUDE_HOME",
        fake_home.path().join(".claude").as_os_str().to_os_string(),
    );
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let prepared = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "claude".to_string(),
        Some("opus".to_string()),
        Some("parent-run".to_string()),
        None,
        Some(ShellType::Zsh),
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        prepared.env_vars.get(&OsString::from("ANTHROPIC_MODEL")),
        Some(&OsString::from("opus"))
    );
    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV))
    );
    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from("OZ_PARENT_LISTENER_MANAGED_EXTERNALLY"))
    );
    assert!(
        !prepared
            .command
            .contains("run message send --sender-run-id")
    );
    assert!(!prepared.command.contains("OZ_PARENT_RUN_ID"));
    assert_eq!(
        prepared.native_session_id.as_deref(),
        prepared.command.split_whitespace().nth(2)
    );
    assert_eq!(
        prepared
            .config
            .harness
            .as_ref()
            .unwrap()
            .model_id
            .as_deref(),
        Some("opus")
    );
    assert!(prepared.config.harness_auth_secrets.is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_claude_child_no_anthropic_model_when_empty() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _claude_home = EnvVarGuard::set(
        "CLAUDE_HOME",
        fake_home.path().join(".claude").as_os_str().to_os_string(),
    );
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let prepared = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "claude".to_string(),
        None,
        Some("parent-run".to_string()),
        None,
        Some(ShellType::Zsh),
        None,
    )
    .await
    .unwrap();

    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from("ANTHROPIC_MODEL"))
    );
}

#[tokio::test]
async fn prepare_local_harness_child_launch_rejects_disabled_codex_before_shell_validation() {
    let result = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "codex".to_string(),
        None,
        Some("parent-run".to_string()),
        None,
        None,
        None,
    )
    .await;

    match result {
        Ok(_) => panic!("disabled local codex should be rejected"),
        Err(err) => assert_eq!(err, LOCAL_CODEX_HARNESS_DISABLED_MESSAGE),
    }
}

#[tokio::test]
async fn app_dispatched_children_use_the_scanned_binary_outside_the_process_path() {
    let _codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let directory = TempDir::new().unwrap();
    let binary_dir = directory.path().join("中文 path's `$() folder");
    fs::create_dir(&binary_dir).unwrap();
    let prompt = "中文\nEnglish 'quoted' `$()`";
    for harness in ["claude", "codex"] {
        write_fake_cli(&binary_dir, harness);
        let executable = binary_dir.join(if cfg!(windows) {
            format!("{harness}.cmd")
        } else {
            harness.to_owned()
        });
        let launch = prepare_local_harness_child_launch(
            prompt.to_owned(),
            harness.to_owned(),
            None,
            None,
            None,
            Some(ShellType::Bash),
            Some(executable.clone()),
        )
        .await
        .unwrap();
        let arguments = shell_words::split(&launch.command).unwrap();
        assert_eq!(arguments[0], executable.to_str().unwrap());
        assert_eq!(arguments.last().unwrap(), prompt);
        let powershell = prepare_local_harness_child_launch(
            prompt.to_owned(),
            harness.to_owned(),
            None,
            None,
            None,
            Some(ShellType::PowerShell),
            Some(executable.clone()),
        )
        .await
        .unwrap();
        let expected_prefix = format!("& '{}' ", executable.to_str().unwrap().replace('\'', "''"));
        assert!(powershell.command.starts_with(&expected_prefix));
        assert!(
            powershell
                .command
                .ends_with("'中文\nEnglish ''quoted'' `$()`'")
        );
    }
}

#[tokio::test]
async fn missing_scanned_executable_does_not_fall_back_to_another_cli() {
    let directory = TempDir::new().unwrap();
    let result = prepare_local_harness_child_launch(
        "保留输入".to_owned(),
        "claude".to_owned(),
        None,
        None,
        None,
        Some(ShellType::Bash),
        Some(directory.path().join("removed-claude")),
    )
    .await;
    assert!(result.is_err());
}
