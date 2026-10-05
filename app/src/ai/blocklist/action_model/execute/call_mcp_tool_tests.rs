//! Unit tests for the `coerce_integer_args` helper.

use serde_json::json;
#[cfg(not(target_family = "wasm"))]
use warpui::App;

use super::*;

fn obj(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(m) => m,
        _ => panic!("expected a JSON object"),
    }
}

#[test]
fn whole_float_is_coerced_when_schema_declares_integer() {
    let mut args = obj(json!({ "line": 5.0 }));
    let schema = obj(json!({
        "properties": { "line": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    // Serialized as "5", not "5.0", and round-trips as i64.
    assert_eq!(serde_json::to_string(&args["line"]).unwrap(), "5");
    assert_eq!(args["line"].as_i64(), Some(5));
}

#[test]
fn no_coercion_when_not_typed_as_integer() {
    // Three scenarios that should all preserve the original float value:
    //   * schema declares `"type": "number"` (explicit float)
    //   * schema has no `properties` at all
    //   * schema property lacks a `"type"` key
    let cases = [
        json!({ "properties": { "x": { "type": "number" } } }),
        json!({}),
        json!({ "properties": { "x": { "description": "no type" } } }),
    ];

    for schema_value in cases {
        let mut args = obj(json!({ "x": 1.0 }));
        let schema = obj(schema_value);

        coerce_integer_args(&mut args, &schema);

        assert_eq!(args["x"].as_f64(), Some(1.0));
        assert_eq!(serde_json::to_string(&args["x"]).unwrap(), "1.0");
    }
}

#[test]
fn nested_object_integer_is_coerced() {
    let mut args = obj(json!({ "outer": { "inner": 5.0 } }));
    let schema = obj(json!({
        "properties": {
            "outer": {
                "type": "object",
                "properties": { "inner": { "type": "integer" } }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["outer"]["inner"].as_i64(), Some(5));
    assert_eq!(serde_json::to_string(&args["outer"]["inner"]).unwrap(), "5");
}

#[test]
fn array_items_integer_is_coerced() {
    let mut args = obj(json!({ "ids": [1.0, 2.0, 3.5] }));
    let schema = obj(json!({
        "properties": {
            "ids": { "type": "array", "items": { "type": "integer" } }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    // Whole-number floats become i64; fractional values are left alone.
    assert_eq!(args["ids"][0].as_i64(), Some(1));
    assert_eq!(args["ids"][1].as_i64(), Some(2));
    assert_eq!(args["ids"][2].as_f64(), Some(3.5));
}

#[test]
fn nullable_integer_type_array_is_coerced() {
    let mut args = obj(json!({ "n": 7.0, "m": null }));
    let schema = obj(json!({
        "properties": {
            "n": { "type": ["integer", "null"] },
            "m": { "type": ["integer", "null"] }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["n"].as_i64(), Some(7));
    assert!(args["m"].is_null());
}

#[test]
fn one_of_branch_with_integer_is_coerced() {
    // Mirrors the schema shape from issue #10596: a `filters` array whose items
    // are a oneOf where one branch has `value: integer` (a millisecond timestamp).
    let mut args = obj(json!({
        "filters": [
            { "value": ["a", "b"] },
            { "value": 1730419200000.0 }
        ]
    }));
    let schema = obj(json!({
        "properties": {
            "filters": {
                "type": "array",
                "items": {
                    "oneOf": [
                        { "properties": { "value": { "type": "array", "items": { "type": "string" } } } },
                        { "properties": { "value": { "type": "integer" } } }
                    ]
                }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["filters"][1]["value"].as_i64(), Some(1730419200000));
    assert_eq!(
        serde_json::to_string(&args["filters"][1]["value"]).unwrap(),
        "1730419200000"
    );
    // The string-array branch value is untouched.
    assert_eq!(args["filters"][0]["value"][0].as_str(), Some("a"));
}

#[test]
fn any_of_at_property_level_is_coerced() {
    let mut args = obj(json!({ "x": 9.0 }));
    let schema = obj(json!({
        "properties": {
            "x": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "integer" }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(9));
}

#[test]
fn all_of_with_integer_branch_is_coerced() {
    let mut args = obj(json!({ "x": 4.0 }));
    let schema = obj(json!({
        "properties": {
            "x": {
                "allOf": [
                    { "type": "integer" },
                    { "minimum": 0 }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(4));
}

#[test]
fn additional_properties_schema_is_applied() {
    let mut args = obj(json!({ "meta": { "a": 1.0, "b": 2.0 } }));
    let schema = obj(json!({
        "properties": {
            "meta": {
                "type": "object",
                "additionalProperties": { "type": "integer" }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["meta"]["a"].as_i64(), Some(1));
    assert_eq!(args["meta"]["b"].as_i64(), Some(2));
}

#[test]
fn fractional_value_is_not_coerced_to_int() {
    let mut args = obj(json!({ "x": 2.5 }));
    let schema = obj(json!({
        "properties": { "x": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    // Schema mismatch (server will reject) but we must not silently truncate.
    assert_eq!(args["x"].as_f64(), Some(2.5));
}

#[test]
fn root_level_one_of_branch_with_integer_is_coerced() {
    // When the root schema uses a combinator instead of (or alongside)
    // `properties`, the entrypoint must walk every branch, not stop at the
    // first one.
    let mut args = obj(json!({ "x": 6.0 }));
    let schema = obj(json!({
        "oneOf": [
            { "properties": { "x": { "type": "string" } } },
            { "properties": { "x": { "type": "integer" } } }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(6));
}

#[test]
fn root_level_additional_properties_is_applied() {
    let mut args = obj(json!({ "anything": 8.0, "more": 9.0 }));
    let schema = obj(json!({ "additionalProperties": { "type": "integer" } }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["anything"].as_i64(), Some(8));
    assert_eq!(args["more"].as_i64(), Some(9));
}

#[test]
fn negative_whole_float_is_coerced() {
    let mut args = obj(json!({ "x": -42.0 }));
    let schema = obj(json!({
        "properties": { "x": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(-42));
    assert_eq!(serde_json::to_string(&args["x"]).unwrap(), "-42");
}

#[test]
fn tuple_style_items_coerces_positional_schemas() {
    let mut args = obj(json!({ "pair": [1.0, "hi", 2.0] }));
    let schema = obj(json!({
        "properties": {
            "pair": {
                "type": "array",
                "items": [
                    { "type": "integer" },
                    { "type": "string" },
                    { "type": "integer" }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["pair"][0].as_i64(), Some(1));
    assert_eq!(args["pair"][1].as_str(), Some("hi"));
    assert_eq!(args["pair"][2].as_i64(), Some(2));
}

#[test]
fn multiple_combinators_at_same_level_are_all_traversed() {
    // A schema may declare more than one of {oneOf, anyOf, allOf} at the
    // same level, and every combinator must be walked — not just the first
    // present key.
    let mut args = obj(json!({ "a": 1.0, "b": 2.0, "c": 3.0 }));
    let schema = obj(json!({
        "oneOf": [{ "properties": { "a": { "type": "integer" } } }],
        "anyOf": [{ "properties": { "b": { "type": "integer" } } }],
        "allOf": [{ "properties": { "c": { "type": "integer" } } }]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["a"].as_i64(), Some(1));
    assert_eq!(args["b"].as_i64(), Some(2));
    assert_eq!(args["c"].as_i64(), Some(3));
}

#[test]
fn one_of_with_multiple_branches_all_visited() {
    // Every branch under a combinator must be visited, not just the first
    // match. The same property name across branches with different types
    // should still get the integer branch's coercion when applicable.
    let mut args = obj(json!({ "v": 11.0 }));
    let schema = obj(json!({
        "oneOf": [
            { "properties": { "v": { "type": "string" } } },
            { "properties": { "v": { "type": "number" } } },
            { "properties": { "v": { "type": "integer" } } }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["v"].as_i64(), Some(11));
}

#[test]
fn already_integer_value_is_unchanged() {
    let mut args = obj(json!({ "x": 5 }));
    let schema = obj(json!({
        "properties": { "x": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(5));
    assert_eq!(serde_json::to_string(&args["x"]).unwrap(), "5");
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn transport_failure_after_dispatch_surfaces_localized_unknown_outcome() {
    App::test((), |app| async move {
        crate::i18n::init(None);
        let expected = crate::t!("ai-mcp-tool-outcome-unknown");
        assert_ne!(expected, "ai-mcp-tool-outcome-unknown");

        let result = app.read(|ctx| {
            handle_call_tool_result(
                Err(ToolCallError::OutcomeUnknown(
                    rmcp::ServiceError::TransportClosed,
                )),
                None,
                "test".to_owned(),
                ctx,
            )
        });

        let AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(message)) = result else {
            panic!("传输失败应返回 MCP 工具错误");
        };
        assert_eq!(message, expected);
        assert_ne!(message, "Transport closed");
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn protocol_error_preserves_the_confirmed_server_error() {
    App::test((), |app| async move {
        let error = rmcp::ServiceError::McpError(rmcp::model::ErrorData::invalid_params(
            "test protocol error",
            None,
        ));
        let expected = error.to_string();

        let result = app.read(|ctx| {
            handle_call_tool_result(
                Err(ToolCallError::Service(error)),
                None,
                "test".to_owned(),
                ctx,
            )
        });

        let AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(message)) = result else {
            panic!("协议错误应返回 MCP 工具错误");
        };
        assert_eq!(message, expected);
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn deadline_before_dispatch_does_not_claim_the_tool_may_have_executed() {
    App::test((), |app| async move {
        crate::i18n::init(None);
        let expected = crate::t!("ai-mcp-tool-timeout-before-dispatch", seconds = 1800);
        assert_ne!(expected, "ai-mcp-tool-timeout-before-dispatch");

        let result = app.read(|ctx| {
            handle_call_tool_result(
                Err(ToolCallError::DeadlineExceeded {
                    may_have_executed: false,
                }),
                None,
                "test".to_owned(),
                ctx,
            )
        });

        let AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(message)) = result else {
            panic!("连接超时应返回 MCP 工具错误");
        };
        assert_eq!(message, expected);
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn deadline_after_dispatch_surfaces_localized_unknown_outcome() {
    App::test((), |app| async move {
        crate::i18n::init(None);
        let expected = crate::t!("ai-mcp-tool-timeout-after-dispatch", seconds = 1800);
        assert_ne!(expected, "ai-mcp-tool-timeout-after-dispatch");

        let result = app.read(|ctx| {
            handle_call_tool_result(
                Err(ToolCallError::DeadlineExceeded {
                    may_have_executed: true,
                }),
                None,
                "test".to_owned(),
                ctx,
            )
        });

        let AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(message)) = result else {
            panic!("派发后超时应返回 MCP 工具错误");
        };
        assert_eq!(message, expected);
    });
}
