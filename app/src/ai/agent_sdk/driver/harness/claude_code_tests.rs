use std::collections::HashMap;
use std::fs;
use std::path::Path;

use std::ffi::OsString;
use tempfile::TempDir;

use serde_json::json;

use super::super::ManagedSecretValue;
use super::*;
use crate::ai::agent_events::MessageHydrator;

fn sample_parent_bridge_message(
    sequence: i64,
    message_id: &str,
    subject: &str,
    body: &str,
) -> MessageBridgeMessageRecord {
    MessageBridgeMessageRecord {
        sequence,
        message_id: message_id.to_string(),
        sender_run_id: "parent-run-456".to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        occurred_at: "2026-04-17T15:46:00Z".to_string(),
    }
}

fn sample_staged_parent_bridge_message(
    sequence: i64,
    message_id: &str,
) -> MessageBridgeMessageRecord {
    MessageBridgeMessageRecord {
        sequence,
        message_id: message_id.to_string(),
        sender_run_id: String::new(),
        subject: String::new(),
        body: String::new(),
        occurred_at: "2026-04-17T15:46:00Z".to_string(),
    }
}

fn write_surfaced_parent_bridge_message(state_dir: &Path, record: &MessageBridgeMessageRecord) {
    fs::write(
        parent_bridge_surfaced_message_path(state_dir, record.sequence, &record.message_id),
        serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

#[test]
#[serial_test::serial]
fn parent_bridge_root_prefers_environment_override() {
    let tmp = TempDir::new().unwrap();
    // edition 2024:`set_var` / `remove_var` 是 unsafe fn(多线程下改环境变量不安全)。
    // 这里由 `#[serial_test::serial]` 保证测试串行执行,不存在并发读写环境变量的情况。
    unsafe {
        std::env::set_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV, tmp.path());
    }
    let root = parent_bridge_root().unwrap();
    unsafe {
        std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV);
    }

    assert_eq!(root, tmp.path());
}

#[test]
fn stage_parent_bridge_message_writes_message_record() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();
    let record = sample_staged_parent_bridge_message(42, "msg-123");

    stage_parent_bridge_message(&state_dir, &record).unwrap();

    let staged_path = parent_bridge_staged_message_path(&state_dir, 42, "msg-123");
    let staged_record: MessageBridgeMessageRecord =
        serde_json::from_slice(&fs::read(&staged_path).unwrap()).unwrap();
    assert_eq!(staged_record.sequence, 42);
    assert_eq!(staged_record.message_id, "msg-123");
    assert!(staged_record.sender_run_id.is_empty());
}

#[tokio::test]
async fn prepare_parent_bridge_hook_output_moves_selected_messages_to_surfaced_dir() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let first = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    stage_parent_bridge_message(&state_dir, &first).unwrap();
    stage_parent_bridge_message(
        &state_dir,
        &sample_staged_parent_bridge_message(43, "msg-456"),
    )
    .unwrap();

    let hydrator = MessageHydrator::new();

    let max_context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE)
        + parent_bridge_char_count(&render_parent_bridge_message_block(&first));
    prepare_parent_bridge_hook_output(&hydrator, &state_dir, max_context_chars)
        .await
        .unwrap();

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert_eq!(hook_output.remaining_staged_count, 1);
    assert!(hook_output.additional_context.contains("Please pivot"));
    assert!(!hook_output.additional_context.contains("Second update"));
    let surfaced_path = parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123");
    assert!(surfaced_path.exists());
    assert!(parent_bridge_staged_message_path(&state_dir, 43, "msg-456").exists());
    assert!(!parent_bridge_staged_message_path(&state_dir, 42, "msg-123").exists());
    let surfaced_record: MessageBridgeMessageRecord =
        serde_json::from_slice(&fs::read(&surfaced_path).unwrap()).unwrap();
    assert_eq!(surfaced_record.subject, first.subject);
    assert_eq!(surfaced_record.body, first.body);
}

#[tokio::test]
async fn acknowledge_parent_bridge_hook_output_marks_messages_delivered_and_clears_state() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let record = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &record);
    fs::write(
        parent_bridge_hook_output_file(&state_dir),
        serde_json::to_vec(&MessageBridgeHookOutput {
            additional_context: "context".to_string(),
            remaining_staged_count: 0,
            surfaced_count: 1,
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(parent_bridge_hook_output_ack_file(&state_dir), "").unwrap();

    let hydrator = MessageHydrator::new();

    acknowledge_parent_bridge_hook_output(&hydrator, &state_dir)
        .await
        .unwrap();

    assert!(!parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123").exists());
    assert!(!parent_bridge_hook_output_file(&state_dir).exists());
    assert!(!parent_bridge_hook_output_ack_file(&state_dir).exists());
}

#[tokio::test]
async fn prepare_parent_bridge_hook_output_reuses_surfaced_records_without_rehydrating() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let record = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &record);

    let hydrator = MessageHydrator::new();

    let max_context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE)
        + parent_bridge_char_count(&render_parent_bridge_message_block(&record));
    prepare_parent_bridge_hook_output(&hydrator, &state_dir, max_context_chars)
        .await
        .unwrap();

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert_eq!(hook_output.remaining_staged_count, 0);
    assert!(hook_output.additional_context.contains(&record.subject));
}

#[tokio::test]
async fn prepare_parent_bridge_hook_output_truncates_single_large_message() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();
    let long_body = "x".repeat(200);
    stage_parent_bridge_message(
        &state_dir,
        &sample_parent_bridge_message(42, "msg-123", "Please pivot", &long_body),
    )
    .unwrap();
    let hydrator = MessageHydrator::new();

    let max_context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE) + 48;
    prepare_parent_bridge_hook_output(&hydrator, &state_dir, max_context_chars)
        .await
        .unwrap();

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert!(
        hook_output.additional_context.ends_with("..."),
        "expected truncated context, got: {}",
        hook_output.additional_context
    );
    assert!(parent_bridge_char_count(&hook_output.additional_context) <= max_context_chars);
}

#[tokio::test]
async fn acknowledge_parent_bridge_hook_output_ignores_missing_ack_marker() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let record = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &record);

    let hydrator = MessageHydrator::new();

    acknowledge_parent_bridge_hook_output(&hydrator, &state_dir)
        .await
        .unwrap();

    assert!(parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123").exists());
}

// 测试只在临时 HOME 与配置目录内放置合成配置，退出时还原原环境。
struct IsolatedClaudeEnvironment {
    previous: [(&'static str, Option<OsString>); 3],
}

impl IsolatedClaudeEnvironment {
    fn new(home: &Path, config: &Path) -> Self {
        let previous = [
            ("HOME", std::env::var_os("HOME")),
            ("USERPROFILE", std::env::var_os("USERPROFILE")),
            ("CLAUDE_CONFIG_DIR", std::env::var_os("CLAUDE_CONFIG_DIR")),
        ];
        // 本组测试使用 serial_test，与同组环境修改串行执行。
        unsafe {
            std::env::set_var("HOME", home);
            std::env::set_var("USERPROFILE", home);
            std::env::set_var("CLAUDE_CONFIG_DIR", config);
        }
        Self { previous }
    }
}

impl Drop for IsolatedClaudeEnvironment {
    fn drop(&mut self) {
        for (name, previous) in &self.previous {
            // 与构造时相同的串行测试约束，包含断言失败后的环境恢复。
            unsafe {
                match previous {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

#[test]
#[serial_test::serial]
fn claude_environment_preparation_preserves_user_configuration_bytes() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let config = temp.path().join("custom-claude");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&config).unwrap();
    let _environment = IsolatedClaudeEnvironment::new(&home, &config);
    let global = home.join(".claude.json");
    let settings = config.join("settings.json");
    let global_bytes = serde_json::to_vec_pretty(&json!({
        "hasCompletedOnboarding":false,"lspRecommendationDisabled":false,
        "projects":{"project":{"hasTrustDialogAccepted":false}},
        "customApiKeyResponses":{"approved":[],"rejected":["合成拒绝记录"]},
        "unknownUserConfig":{"enabled":true}
    }))
    .unwrap();
    let settings_bytes = serde_json::to_vec_pretty(&json!({
        "skipDangerousModePermissionPrompt":false,
        "enabledPlugins":{"warp@claude-code-warp":false,"user-plugin":true},
        "hooks":{"UserPromptSubmit":[{"matcher":"","hooks":[{"type":"command","command":"user-hook"}]}]},
        "permissions":{"defaultMode":"default","deny":["Bash(*)"]}
    })).unwrap();
    fs::write(&global, &global_bytes).unwrap();
    fs::write(&settings, &settings_bytes).unwrap();
    let secrets = HashMap::from([(
        "ANTHROPIC_API_KEY".to_string(),
        ManagedSecretValue::raw_value("synthetic-unit-test-only-key"),
    )]);

    ClaudeHarness
        .prepare_environment_config(&home.join("project"), Some("合成系统提示"), &secrets)
        .unwrap();

    assert_eq!(fs::read(global).unwrap(), global_bytes);
    assert_eq!(fs::read(settings).unwrap(), settings_bytes);
    assert!(!home.join(".claude").exists());
}

#[test]
#[serial_test::serial]
fn claude_environment_preparation_does_not_create_global_configuration() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let config = temp.path().join("custom-claude");
    fs::create_dir_all(&home).unwrap();
    let _environment = IsolatedClaudeEnvironment::new(&home, &config);

    ClaudeHarness
        .prepare_environment_config(&home.join("project"), None, &HashMap::new())
        .unwrap();

    assert!(!home.join(".claude.json").exists());
    assert!(!home.join(".claude").exists());
    assert!(!config.exists());
    assert!(!home.join("project").exists());
}
