use serde_json::Value;

use super::{REGISTRY, lookup};

fn contains_chinese(text: &str) -> bool {
    text.chars()
        .any(|character| ('\u{3400}'..='\u{9fff}').contains(&character))
}

fn check_schema_descriptions(schema: &Value, tool: &str) {
    if let Some(object) = schema.as_object() {
        for (key, value) in object {
            if key == "description" {
                let description = value.as_str().expect("工具描述必须是字符串");
                assert!(
                    !contains_chinese(description),
                    "{tool} 的模型描述仍含中文：{description}"
                );
            }
            check_schema_descriptions(value, tool);
        }
    } else if let Some(values) = schema.as_array() {
        for value in values {
            check_schema_descriptions(value, tool);
        }
    }
}

#[test]
fn model_tool_descriptions_do_not_force_chinese() {
    for tool in REGISTRY {
        assert!(
            !contains_chinese(tool.description),
            "{} 的模型描述仍含中文",
            tool.name
        );
        check_schema_descriptions(&(tool.parameters)(), tool.name);
    }
}

#[test]
fn user_facing_tool_fields_follow_the_users_language() {
    for (name, pointer) in [
        (
            "ask_user_question",
            "/properties/questions/items/properties/question/description",
        ),
        (
            "ask_user_question",
            "/properties/questions/items/properties/options/description",
        ),
        ("apply_file_diffs", "/properties/summary/description"),
        (
            "transfer_shell_command_control_to_user",
            "/properties/reason/description",
        ),
        ("suggest_prompt", "/properties/prompt/description"),
        ("suggest_prompt", "/properties/label/description"),
        (
            "create_documents",
            "/properties/new_documents/items/properties/title/description",
        ),
        (
            "create_documents",
            "/properties/new_documents/items/properties/content/description",
        ),
    ] {
        let tool = lookup(name).unwrap();
        let schema = (tool.parameters)();
        let description = schema.pointer(pointer).unwrap().as_str().unwrap();
        assert!(
            description.contains("same language as the user"),
            "{name} 的用户可见字段缺少跟随用户语言的要求：{pointer}"
        );
    }
}

#[test]
fn skill_schema_preserves_duplicate_name_path_guidance() {
    let schema = (lookup("read_skill").unwrap().parameters)();
    let description = schema["properties"]["name"]["description"]
        .as_str()
        .unwrap();
    assert!(description.contains("duplicate names"));
    assert!(description.contains("full skill_path"));
}
