use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

// 菜单描述符也读取元数据，避免每次枚举技能时重复编译正则。
static FRONT_MATTER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?ms)\A\s*---[ \t]*\r?\n(.*?)\r?\n---[ \t]*\r?\n").unwrap());

/// 保留 YAML 类型，调用方按实际 CLI 的规则决定人工菜单可见性。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkillUserInvocable {
    #[default]
    Unspecified,
    Boolean(bool),
    String(String),
    Invalid,
}

impl SkillUserInvocable {
    fn from_yaml(value: Option<&Value>) -> Self {
        match value {
            None => Self::Unspecified,
            Some(Value::Bool(value)) => Self::Boolean(*value),
            Some(Value::String(value)) => Self::String(value.clone()),
            Some(Value::Null | Value::Number(_) | Value::Sequence(_) | Value::Mapping(_)) => {
                Self::Invalid
            }
        }
    }
}

/// Represents a parsed markdown file with YAML front matter
#[derive(Debug)]
#[allow(dead_code)]
pub struct ParsedMarkdown {
    /// The YAML front matter parsed as a map
    /// For Skills, the front matter is always a single-level map with string keys and string values
    pub front_matter: HashMap<String, String>,
    /// 人工技能调用元数据，不改变模型侧技能清单。
    pub user_invocable: SkillUserInvocable,
    /// The entire content of the file
    pub content: String,
    /// The line range where the markdown content (without front matter) is located (1-indexed)
    /// None if there is no front matter (content is the entire file)
    pub line_range: Option<Range<usize>>,
}

/// Parse markdown content with YAML front matter
#[allow(dead_code)]
pub(crate) fn parse_markdown_content(content: &str) -> Result<ParsedMarkdown> {
    // Regex to match YAML front matter at the start of the file
    // Handles both LF (\n) and CRLF (\r\n) line endings
    // Allows leading whitespace (spaces, tabs, newlines) before the opening ---
    // Allows trailing spaces/tabs after --- markers
    // Pattern: (optional whitespace) --- (optional spaces/tabs) (line ending) (content) (line ending) --- (optional spaces/tabs) (line ending)
    let captures = FRONT_MATTER_REGEX.captures(content);

    if let Some(captures) = captures {
        // Extract the YAML section (first capture group) and trim to handle extra blank lines
        let yaml_str = captures.get(1).unwrap().as_str().trim();

        // Parse the YAML into a map (empty front matter is valid — just yields no keys)
        let mut user_invocable = SkillUserInvocable::Unspecified;
        let front_matter = if yaml_str.is_empty() {
            HashMap::new()
        } else {
            let yaml_value: Value =
                serde_yaml::from_str(yaml_str).context("Failed to parse YAML front matter")?;
            match yaml_value {
                Value::Mapping(map) => {
                    user_invocable = SkillUserInvocable::from_yaml(
                        map.get(&Value::String("user-invocable".to_owned())),
                    );
                    map.iter()
                        .filter_map(|(key, value)| {
                            if let Value::String(key_str) = key
                                && let Value::String(value_str) = value
                            {
                                return Some((key_str.clone(), value_str.clone()));
                            }

                            None
                        })
                        .collect()
                }
                _ => HashMap::new(),
            }
        };

        // Get the content after the front matter
        let content_start = captures.get(0).unwrap().end();

        // Calculate line range for the markdown content (without front matter)
        // Line numbers are 1-indexed, so we add 1
        let lines_before_content = content[..content_start].lines().count();
        let total_lines = content.lines().count();
        let line_range = Some((lines_before_content + 1)..(total_lines + 1));

        Ok(ParsedMarkdown {
            front_matter,
            user_invocable,
            content: content.to_string(),
            line_range,
        })
    } else {
        // No front matter found - content is the entire file, so line_range is None
        Ok(ParsedMarkdown {
            front_matter: HashMap::new(),
            user_invocable: SkillUserInvocable::Unspecified,
            content: content.to_string(),
            line_range: None,
        })
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_test;
