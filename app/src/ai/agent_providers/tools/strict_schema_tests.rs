use serde_json::{Value, json};

use super::{REGISTRY, lookup, strict_schema};

#[test]
fn run_shell_command_schema满足responses_strict要求() {
    let tool = lookup("run_shell_command").expect("应注册 run_shell_command");
    let schema = strict_schema::normalize((tool.parameters)());

    assert_strict_schema(&schema);
    assert_eq!(
        schema["required"],
        json!([
            "command",
            "is_read_only",
            "is_risky",
            "uses_pager",
            "wait_until_complete"
        ])
    );
    assert_eq!(
        schema["properties"]["is_read_only"]["type"],
        json!(["boolean", "null"])
    );
    assert!(
        schema["properties"]["is_read_only"]
            .get("default")
            .is_none()
    );
}

#[test]
fn 所有内置工具schema满足responses_strict要求() {
    for tool in REGISTRY {
        let schema = strict_schema::normalize((tool.parameters)());
        assert_strict_schema(&schema);
    }
}

#[test]
fn strict_schema把one_of转换为受支持的any_of() {
    let tool = lookup("apply_file_diffs").expect("应注册 apply_file_diffs");
    let schema = strict_schema::normalize((tool.parameters)());

    assert!(!contains_key(&schema, "oneOf"));
    assert!(contains_key(&schema, "anyOf"));
}

#[test]
fn strict参数中的可选null会恢复为字段缺省() {
    let tool = lookup("run_shell_command").expect("应注册 run_shell_command");
    let schema = (tool.parameters)();
    let args = json!({
        "command": "pwd",
        "is_read_only": null,
        "uses_pager": null,
        "is_risky": null,
        "wait_until_complete": null
    });

    let normalized =
        strict_schema::omit_optional_nulls(&args.to_string(), &schema).expect("应删除可选 null");
    assert_eq!(normalized, json!({"command": "pwd"}).to_string());
    (tool.from_args)(&normalized).expect("删除 null 后 serde 默认值应正常生效");
}

fn assert_strict_schema(schema: &Value) {
    match schema {
        Value::Object(object) => {
            assert!(
                !object.contains_key("default"),
                "strict schema 不应包含 default"
            );
            assert!(
                !object.contains_key("oneOf"),
                "strict schema 不应包含 oneOf"
            );
            if object.get("type").is_some_and(is_object_type) {
                assert_eq!(
                    object.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "每个 object 都必须禁止额外字段: {schema}"
                );
                let property_names = object
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                assert_eq!(
                    object.get("required"),
                    Some(&json!(property_names)),
                    "required 必须包含 properties 的全部字段: {schema}"
                );
            }
            for child in object.values() {
                assert_strict_schema(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_strict_schema(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_object_type(schema_type: &Value) -> bool {
    match schema_type {
        Value::String(value) => value == "object",
        Value::Array(values) => values.iter().any(|value| value == "object"),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => false,
    }
}

fn contains_key(value: &Value, target: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(target) || object.values().any(|child| contains_key(child, target))
        }
        Value::Array(items) => items.iter().any(|item| contains_key(item, target)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}
