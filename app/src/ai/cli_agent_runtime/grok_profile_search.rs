//! 固定 Grok 搜索工具的审批参数；不会把检索工具转换成任意 shell 命令。

use serde_json::Value;
use std::path::Path;

use crate::ai::cli_agent_runtime::file_search_scope::{
    bounded_pattern, relative_glob, resolve_search_path,
};

pub(super) fn approval_allowed(root: &Path, call: &Value) -> bool {
    let input = &call["rawInput"];
    let Some(fields) = input.as_object() else {
        return false;
    };
    let name = call["_meta"]["x.ai/tool"]["name"].as_str();
    match (name, input["variant"].as_str()) {
        (Some("list_dir"), Some("ListDir")) => {
            call["kind"] == "read"
                && fields.len() == 2
                && input["target_directory"]
                    .as_str()
                    .and_then(|path| resolve_search_path(root, Some(path)))
                    .is_some_and(|path| path.is_dir())
        }
        (Some("grep"), Some("GrepSearch")) => {
            if call["kind"] != "search"
                || !input["pattern"].as_str().is_some_and(bounded_pattern)
                || !input["path"]
                    .as_str()
                    .and_then(|path| resolve_search_path(root, Some(path)))
                    .is_some_and(|path| path.is_file())
            {
                return false;
            }
            fields.iter().all(|(key, value)| match key.as_str() {
                "variant" | "pattern" | "path" => value.is_string(),
                "glob" => value.is_null() || value.as_str().is_some_and(relative_glob),
                "type" => {
                    value.is_null()
                        || value.as_str().is_some_and(|kind| {
                            !kind.is_empty()
                                && kind.len() <= 64
                                && kind.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                                })
                        })
                }
                "output_mode" => {
                    value.is_null()
                        || matches!(
                            value.as_str(),
                            Some("content" | "files_with_matches" | "count")
                        )
                }
                "-i" | "-n" | "multiline" => value.is_null() || value.is_boolean(),
                "-A" | "-B" | "-C" | "head_limit" | "max_chars_per_line" => {
                    value.is_null() || value.as_u64().is_some_and(|count| count <= 100_000)
                }
                _ => false,
            })
        }
        _ => false,
    }
}
