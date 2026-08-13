//! 工具参数容错层。
//!
//! 部分 BYOP 模型(尤其 DeepSeek reasoner、某些 OSS 模型)在 tool_calls 的
//! `arguments` 里会把 boolean 写成 `"true"`/`"false"`、把数字写成字符串、把
//! array/object 整个 JSON.stringify 一次。`from_args` 用 serde 严格解,这类
//! 输入会直接 reject,UI 端表现为"工具偶发故障"。
//!
//! 本模块只在 `from_args` 第一次失败后才被调用:读 `parameters()` schema,
//! 按 schema 声明的类型,把 JSON Value 里的 string 强转回目标类型。覆盖:
//!
//! | schema type | 模型返回 | 修正为 |
//! |---|---|---|
//! | boolean | "true"/"True"/"1"/"yes" | true |
//! | boolean | "false"/"False"/"0"/"no" | false |
//! | integer | "42" / 42.0 | 42 |
//! | number | "3.14" | 3.14 |
//! | string | 42 / true | "42" / "true" |
//! | array | "[\"a\"]"(JSON 字符串) | ["a"] |
//! | object | "{\"k\":1}"(JSON 字符串) | {"k":1} |
//!
//! 不能 coerce 的字段保留原值,让原始解析错误透出。

use serde_json::{Number, Value};

/// 尝试根据 schema 修正 args JSON。返回 `Some(coerced_string)` 表示至少做了一次
/// 类型转换;返回 `None` 表示输入根本解不出 JSON 或没有任何字段需要 coerce。
pub fn coerce_args_against_schema(args_str: &str, schema: &Value) -> Option<String> {
    let mut value: Value = serde_json::from_str(args_str).ok()?;
    let mut changed = false;
    coerce_value(&mut value, schema, &mut changed);
    if !changed {
        return None;
    }
    serde_json::to_string(&value).ok()
}

fn coerce_value(value: &mut Value, schema: &Value, changed: &mut bool) {
    let Some(ty) = schema.get("type").and_then(|t| t.as_str()) else {
        // schema 没标 type:对象类型尝试递归 properties,否则放弃。
        if let Some(props) = schema.get("properties") {
            coerce_object(value, props, schema, changed);
        }
        return;
    };

    match ty {
        "object" => {
            // 模型把整个 object 字符串化的情况:解一层后再继续。
            if let Some(s) = value.as_str() {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    if parsed.is_object() {
                        *value = parsed;
                        *changed = true;
                    }
                }
            }
            if let Some(props) = schema.get("properties") {
                coerce_object(value, props, schema, changed);
            }
        }
        "array" => {
            if let Some(s) = value.as_str() {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    if parsed.is_array() {
                        *value = parsed;
                        *changed = true;
                    }
                }
            }
            if let (Some(arr), Some(items_schema)) = (value.as_array_mut(), schema.get("items")) {
                for item in arr {
                    coerce_value(item, items_schema, changed);
                }
            }
        }
        "boolean" => {
            if let Some(s) = value.as_str() {
                match s.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => {
                        *value = Value::Bool(true);
                        *changed = true;
                    }
                    "false" | "0" | "no" => {
                        *value = Value::Bool(false);
                        *changed = true;
                    }
                    _ => {}
                }
            }
        }
        "integer" => {
            if let Some(s) = value.as_str() {
                if let Ok(n) = s.parse::<i64>() {
                    *value = Value::Number(n.into());
                    *changed = true;
                } else if let Ok(f) = s.parse::<f64>() {
                    if f.fract() == 0.0 && f.is_finite() {
                        if let Some(num) = Number::from_f64(f).and_then(|n| n.as_i64()) {
                            *value = Value::Number(num.into());
                            *changed = true;
                        }
                    }
                }
            } else if let Some(f) = value.as_f64() {
                if f.fract() == 0.0 && f.is_finite() {
                    let n = f as i64;
                    *value = Value::Number(n.into());
                    *changed = true;
                }
            }
        }
        "number" => {
            if let Some(s) = value.as_str() {
                if let Ok(f) = s.parse::<f64>() {
                    if let Some(num) = Number::from_f64(f) {
                        *value = Value::Number(num);
                        *changed = true;
                    }
                }
            }
        }
        "string" => match value {
            Value::Number(n) => {
                let s = n.to_string();
                *value = Value::String(s);
                *changed = true;
            }
            Value::Bool(b) => {
                *value = Value::String(b.to_string());
                *changed = true;
            }
            _ => {}
        },
        _ => {}
    }
}

fn coerce_object(value: &mut Value, props: &Value, parent_schema: &Value, changed: &mut bool) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(props_map) = props.as_object() else {
        return;
    };
    for (key, prop_schema) in props_map {
        if let Some(field) = obj.get_mut(key) {
            coerce_value(field, prop_schema, changed);
        }
    }
    // additionalProperties: schema 也有可能描述未列在 properties 中的字段。
    if let Some(additional) = parent_schema
        .get("additionalProperties")
        .filter(|v| v.is_object())
    {
        let known: std::collections::HashSet<&String> = props_map.keys().collect();
        // SAFETY: keys collected before mutating values. Walk via owned copy of
        // the keys to avoid double borrow.
        let extra_keys: Vec<String> = obj.keys().filter(|k| !known.contains(k)).cloned().collect();
        for k in extra_keys {
            if let Some(field) = obj.get_mut(&k) {
                coerce_value(field, additional, changed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn shell_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "is_read_only": {"type": "boolean"},
                "uses_pager": {"type": "boolean"},
                "is_risky": {"type": "boolean"},
                "wait_until_complete": {"type": "boolean"}
            },
            "required": ["command"]
        })
    }

    #[test]
    fn boolean_strings_coerced() {
        let args =
            r#"{"command":"echo b","is_read_only":"true","is_risky":"False","uses_pager":"0"}"#;
        let out = coerce_args_against_schema(args, &shell_schema()).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["is_read_only"], json!(true));
        assert_eq!(v["is_risky"], json!(false));
        assert_eq!(v["uses_pager"], json!(false));
    }

    #[test]
    fn no_change_returns_none() {
        let args = r#"{"command":"echo b","is_read_only":true}"#;
        assert!(coerce_args_against_schema(args, &shell_schema()).is_none());
    }

    #[test]
    fn malformed_json_returns_none() {
        let args = r#"{not json"#;
        assert!(coerce_args_against_schema(args, &shell_schema()).is_none());
    }

    fn grep_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "queries": {"type": "array", "items": {"type": "string"}},
                "path": {"type": "string"}
            }
        })
    }

    #[test]
    fn array_string_coerced_to_array() {
        let args = r#"{"queries":"[\"mod menu\",\"foo\"]","path":"app/src/lib.rs"}"#;
        let out = coerce_args_against_schema(args, &grep_schema()).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["queries"], json!(["mod menu", "foo"]));
    }

    #[test]
    fn integer_string_coerced() {
        let schema = json!({
            "type": "object",
            "properties": {"count": {"type": "integer"}}
        });
        let args = r#"{"count":"42"}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["count"], json!(42));
    }

    #[test]
    fn nested_array_items_coerced() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"flag": {"type": "boolean"}}
                    }
                }
            }
        });
        let args = r#"{"items":[{"flag":"true"},{"flag":"false"}]}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["items"][0]["flag"], json!(true));
        assert_eq!(v["items"][1]["flag"], json!(false));
    }

    #[test]
    fn number_to_string_field() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}}
        });
        let args = r#"{"path":42}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["path"], json!("42"));
    }

    #[test]
    fn stringified_object_coerced() {
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {"enabled": {"type": "boolean"}}
                }
            }
        });
        let args = r#"{"config":"{\"enabled\":\"true\"}"}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["config"]["enabled"], json!(true));
    }
}
