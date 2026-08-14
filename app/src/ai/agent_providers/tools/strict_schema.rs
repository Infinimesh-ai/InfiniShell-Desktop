//! OpenAI Responses strict function schema 兼容处理。

use std::collections::HashSet;

use serde_json::{Map, Value};

/// 把内置工具的通用 JSON Schema 转成 Responses strict function schema。
pub fn normalize(mut schema: Value) -> Value {
    normalize_node(&mut schema);
    schema
}

/// 删除 strict schema 为“原本可选”字段补出的 `null`，让 serde 默认值继续生效。
pub fn omit_optional_nulls(args: &str, schema: &Value) -> Option<String> {
    let mut value = serde_json::from_str(args).ok()?;
    omit_optional_nulls_in_value(&mut value, schema);
    serde_json::to_string(&value).ok()
}

fn normalize_node(schema: &mut Value) {
    let Value::Object(object) = schema else {
        if let Value::Array(items) = schema {
            for item in items {
                normalize_node(item);
            }
        }
        return;
    };

    object.remove("default");
    replace_one_of(object);

    if object.get("type").is_some_and(is_object_type) {
        let originally_required = required_names(object);
        let property_names = object
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(Value::Object(properties)) = object.get_mut("properties") {
            for (name, property_schema) in properties {
                if !originally_required.contains(name) {
                    allow_null(property_schema);
                }
            }
        }
        object.insert("required".to_owned(), Value::from(property_names));
        object.insert("additionalProperties".to_owned(), Value::Bool(false));
    }

    for child in object.values_mut() {
        normalize_node(child);
    }
}

fn replace_one_of(schema: &mut Map<String, Value>) {
    let Some(one_of) = schema.remove("oneOf") else {
        return;
    };
    match (schema.get_mut("anyOf"), one_of) {
        (Some(Value::Array(any_of)), Value::Array(mut alternatives)) => {
            any_of.append(&mut alternatives);
        }
        (None, alternatives) => {
            schema.insert("anyOf".to_owned(), alternatives);
        }
        (Some(_), _) => {}
    }
}

fn allow_null(schema: &mut Value) {
    let Value::Object(object) = schema else {
        return;
    };
    if let Some(schema_type) = object.get_mut("type") {
        match schema_type {
            Value::String(value) if value != "null" => {
                *schema_type =
                    Value::Array(vec![Value::String(value.clone()), Value::from("null")]);
            }
            Value::Array(types) if !types.iter().any(|value| value == "null") => {
                types.push(Value::from("null"));
            }
            Value::String(_)
            | Value::Array(_)
            | Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Object(_) => {}
        }
        return;
    }
    if let Some(Value::Array(any_of)) = object.get_mut("anyOf") {
        any_of.push(Value::Object(Map::from_iter([(
            "type".to_owned(),
            Value::from("null"),
        )])));
        return;
    }

    let original = std::mem::replace(schema, Value::Object(Map::new()));
    *schema = Value::Object(Map::from_iter([(
        "anyOf".to_owned(),
        Value::Array(vec![
            original,
            Value::Object(Map::from_iter([("type".to_owned(), Value::from("null"))])),
        ]),
    )]));
}

fn omit_optional_nulls_in_value(value: &mut Value, schema: &Value) -> bool {
    let schema = select_union_alternative(value, schema);
    match (value, schema) {
        (Value::Object(arguments), Value::Object(schema)) => {
            let required = required_names(schema);
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return false;
            };
            let optional_nulls = arguments
                .iter()
                .filter(|(name, value)| {
                    value.is_null() && properties.contains_key(*name) && !required.contains(*name)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            let mut changed = !optional_nulls.is_empty();
            for name in optional_nulls {
                arguments.remove(&name);
            }
            for (name, value) in arguments {
                if let Some(property_schema) = properties.get(name) {
                    changed |= omit_optional_nulls_in_value(value, property_schema);
                }
            }
            changed
        }
        (Value::Array(arguments), Value::Object(schema)) => {
            let Some(item_schema) = schema.get("items") else {
                return false;
            };
            arguments.iter_mut().fold(false, |changed, value| {
                omit_optional_nulls_in_value(value, item_schema) || changed
            })
        }
        (
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_)
            | Value::Object(_),
        )
        | (
            Value::Object(_),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        )
        | (
            Value::Array(_),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        ) => false,
    }
}

fn select_union_alternative<'a>(value: &Value, schema: &'a Value) -> &'a Value {
    let Some(alternatives) = schema
        .as_object()
        .and_then(|object| object.get("anyOf").or_else(|| object.get("oneOf")))
        .and_then(Value::as_array)
    else {
        return schema;
    };
    alternatives
        .iter()
        .filter(|alternative| discriminator_matches(value, alternative))
        .max_by_key(|alternative| matching_property_count(value, alternative))
        .unwrap_or(schema)
}

fn discriminator_matches(value: &Value, schema: &Value) -> bool {
    let (Some(arguments), Some(properties)) = (
        value.as_object(),
        schema
            .as_object()
            .and_then(|object| object.get("properties"))
            .and_then(Value::as_object),
    ) else {
        return true;
    };
    properties.iter().all(|(name, property_schema)| {
        let Some(argument) = arguments.get(name) else {
            return true;
        };
        let Some(allowed) = property_schema.get("enum").and_then(Value::as_array) else {
            return true;
        };
        allowed.contains(argument)
    })
}

fn matching_property_count(value: &Value, schema: &Value) -> usize {
    let (Some(arguments), Some(properties)) = (
        value.as_object(),
        schema
            .as_object()
            .and_then(|object| object.get("properties"))
            .and_then(Value::as_object),
    ) else {
        return 0;
    };
    arguments
        .keys()
        .filter(|name| properties.contains_key(*name))
        .count()
}

fn required_names(schema: &Map<String, Value>) -> HashSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn is_object_type(schema_type: &Value) -> bool {
    match schema_type {
        Value::String(value) => value == "object",
        Value::Array(values) => values.iter().any(|value| value == "object"),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => false,
    }
}
