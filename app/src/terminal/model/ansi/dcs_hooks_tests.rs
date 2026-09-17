use serde_json::{Value, json};

use super::{BootstrappedValue, DProtoHook, PendingHook};
use crate::terminal::model::session::SessionInfo;
use crate::terminal::shell::ShellType;

fn bootstrap_json() -> Value {
    json!({
        "shell": "bash",
        "histfile": "",
        "home_dir": "/fixture/home",
        "path": "/fixture/bin",
        "aliases": "",
        "abbreviations": "",
        "function_names": "",
        "env_var_names": "",
        "builtins": "",
        "keywords": "",
        "shell_version": ""
    })
}

fn session_from_json(value: Value) -> SessionInfo {
    let bootstrap = serde_json::from_value::<BootstrappedValue>(value).unwrap();
    SessionInfo::new_for_test()
        .with_shell_type(ShellType::Bash)
        .merge_from_bootstrapped_value(bootstrap)
}

#[test]
fn json_explicit_empty_command_lists_are_complete_empty_snapshots() {
    let bootstrap = serde_json::from_value::<BootstrappedValue>(bootstrap_json()).unwrap();
    assert_eq!(bootstrap.aliases.as_deref(), Some(""));
    assert_eq!(bootstrap.function_names.as_deref(), Some(""));
    // 其它空字符串字段保留原有语义。
    assert_eq!(bootstrap.histfile, None);
    assert_eq!(bootstrap.abbreviations, None);
    let info = session_from_json(bootstrap_json());
    assert!(info.command_snapshot_complete);
    assert!(info.aliases.is_empty());
    assert!(info.function_names.is_empty());
}

#[test]
fn json_missing_command_list_is_unknown_even_when_the_other_list_is_empty() {
    for field in ["aliases", "function_names"] {
        let mut value = bootstrap_json();
        value.as_object_mut().unwrap().remove(field);
        let bootstrap = serde_json::from_value::<BootstrappedValue>(value.clone()).unwrap();
        assert!(!(bootstrap.aliases.is_some() && bootstrap.function_names.is_some()));
        assert!(!session_from_json(value).command_snapshot_complete);
    }
}

#[test]
fn json_null_command_list_is_unknown() {
    for field in ["aliases", "function_names"] {
        let mut value = bootstrap_json();
        value[field] = Value::Null;
        let bootstrap = serde_json::from_value::<BootstrappedValue>(value.clone()).unwrap();
        assert!(!(bootstrap.aliases.is_some() && bootstrap.function_names.is_some()));
        assert!(!session_from_json(value).command_snapshot_complete);
    }
}

#[test]
fn json_nonempty_alias_and_function_lists_keep_existing_parsing() {
    let mut value = bootstrap_json();
    value["aliases"] = json!("alias grok='custom-grok --profile user'");
    value["function_names"] = json!("claude\ncodex");
    let info = session_from_json(value);
    assert!(info.command_snapshot_complete);
    assert_eq!(
        info.aliases.get("grok").map(String::as_str),
        Some("custom-grok --profile user")
    );
    assert!(info.function_names.contains("claude"));
    assert!(info.function_names.contains("codex"));
}

fn kv_bootstrap(fields: &[(&str, &str)]) -> BootstrappedValue {
    let mut hook = PendingHook::create("Bootstrapped").unwrap();
    hook.update("shell".into(), "bash".into());
    for (key, value) in fields {
        hook.update((*key).into(), (*value).into());
    }
    let DProtoHook::Bootstrapped { value } = hook.finish().unwrap() else {
        panic!("夹具必须生成 Bootstrapped hook");
    };
    *value
}

#[test]
fn kv_explicit_empty_command_lists_are_complete_empty_snapshots() {
    let bootstrap = kv_bootstrap(&[("aliases", ""), ("function_names", "")]);
    assert_eq!(bootstrap.aliases.as_deref(), Some(""));
    assert_eq!(bootstrap.function_names.as_deref(), Some(""));
    let info = SessionInfo::new_for_test()
        .with_shell_type(ShellType::Bash)
        .merge_from_bootstrapped_value(bootstrap);
    assert!(info.command_snapshot_complete);
    assert!(info.aliases.is_empty());
    assert!(info.function_names.is_empty());
}

#[test]
fn kv_missing_command_list_is_unknown() {
    for fields in [vec![], vec![("aliases", "")], vec![("function_names", "")]] {
        let bootstrap = kv_bootstrap(&fields);
        let info = SessionInfo::new_for_test()
            .with_shell_type(ShellType::Bash)
            .merge_from_bootstrapped_value(bootstrap);
        assert!(!info.command_snapshot_complete);
    }
}

#[test]
fn kv_text_null_is_plain_text_and_does_not_claim_a_json_null() {
    // KV 传输没有 JSON null 类型，不将字面量文本改写为缺失值。
    let bootstrap = kv_bootstrap(&[("aliases", "null"), ("function_names", "null")]);
    assert_eq!(bootstrap.aliases.as_deref(), Some("null"));
    assert_eq!(bootstrap.function_names.as_deref(), Some("null"));
}

#[test]
fn json_command_lists_keep_existing_whitespace_and_nul_trimming() {
    let mut value = bootstrap_json();
    value["aliases"] = json!("  alias grok='custom-grok'\n\u{0}");
    value["function_names"] = json!("\nclaude\ncodex\n\u{0}");
    let bootstrap = serde_json::from_value::<BootstrappedValue>(value).unwrap();
    assert_eq!(
        bootstrap.aliases.as_deref(),
        Some("alias grok='custom-grok'\n")
    );
    assert_eq!(bootstrap.function_names.as_deref(), Some("claude\ncodex\n"));
}
