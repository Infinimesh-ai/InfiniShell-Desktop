use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use i18n_embed::LanguageLoader;
use i18n_embed::fluent::fluent_language_loader;

use super::*;

fn message_id(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) || line.starts_with('#') || line.starts_with('-') {
        return None;
    }

    let (id, _) = line.split_once('=')?;
    let id = id.trim();
    let is_valid = id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        });
    is_valid.then_some(id)
}

fn insert_message(
    messages: &mut BTreeMap<String, String>,
    id: String,
    value: String,
    resource_path: &str,
) {
    assert!(
        messages.insert(id.clone(), value).is_none(),
        "{resource_path} 中存在重复的 Fluent 消息键：{id}"
    );
}

fn messages_for_locale(locale: &str) -> BTreeMap<String, String> {
    let prefix = format!("{locale}/");
    let mut messages = BTreeMap::new();

    for resource_path in Localizations::iter() {
        let resource_path: &str = resource_path.as_ref();
        if !resource_path.starts_with(&prefix) || !resource_path.ends_with(".ftl") {
            continue;
        }

        let resource = Localizations::get(resource_path)
            .unwrap_or_else(|| panic!("无法读取嵌入的本地化资源：{resource_path}"));
        let source = std::str::from_utf8(resource.data.as_ref())
            .unwrap_or_else(|error| panic!("本地化资源不是有效 UTF-8：{resource_path}: {error}"));
        let mut current_id = None;
        let mut current_value = String::new();

        for line in source.lines() {
            if let Some(id) = message_id(line) {
                if let Some(previous_id) = current_id.replace(id.to_string()) {
                    insert_message(
                        &mut messages,
                        previous_id,
                        std::mem::take(&mut current_value),
                        resource_path,
                    );
                }
                current_value.push_str(line.split_once('=').unwrap().1);
            } else if current_id.is_some()
                && (line.is_empty() || line.starts_with(char::is_whitespace))
            {
                current_value.push('\n');
                current_value.push_str(line);
            } else if let Some(previous_id) = current_id.take() {
                insert_message(
                    &mut messages,
                    previous_id,
                    std::mem::take(&mut current_value),
                    resource_path,
                );
            }
        }

        if let Some(previous_id) = current_id {
            insert_message(&mut messages, previous_id, current_value, resource_path);
        }
    }

    messages
}

fn message_variables(value: &str) -> BTreeSet<String> {
    let bytes = value.as_bytes();
    let mut variables = BTreeSet::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }

        let start = index + 1;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
        {
            end += 1;
        }
        if end > start {
            variables.insert(value[start..end].to_string());
        }
        index = end.max(index + 1);
    }

    variables
}

fn rust_source_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut directories = vec![directory.to_path_buf()];

    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("无法读取源码目录 {}：{error}", directory.display()))
        {
            let entry = entry.unwrap_or_else(|error| {
                panic!("无法读取源码目录项 {}：{error}", directory.display())
            });
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let file_stem = path
                    .file_stem()
                    .and_then(|file_stem| file_stem.to_str())
                    .unwrap_or_default();
                if file_stem.ends_with("_test")
                    || file_stem.ends_with("_tests")
                    || matches!(file_stem, "test" | "tests" | "mod_test")
                {
                    continue;
                }
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

fn literal_translation_keys(source: &str) -> Vec<(usize, String)> {
    let bytes = source.as_bytes();
    let mut keys = Vec::new();
    let mut index = 0;

    while let Some(relative_bang) = bytes[index..].iter().position(|byte| *byte == b'!') {
        let bang = index + relative_bang;
        let mut identifier_start = bang;
        while identifier_start > 0
            && (bytes[identifier_start - 1].is_ascii_alphanumeric()
                || bytes[identifier_start - 1] == b'_')
        {
            identifier_start -= 1;
        }
        let identifier = &source[identifier_start..bang];
        if !matches!(identifier, "t" | "t_static") {
            index = bang + 1;
            continue;
        }
        let line_start = source[..identifier_start]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        if source[line_start..identifier_start].contains("//") {
            index = bang + 1;
            continue;
        }

        let mut cursor = bang + 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'(') {
            index = bang + 1;
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'"') {
            index = bang + 1;
            continue;
        }

        let key_start = cursor + 1;
        let Some(relative_quote) = bytes[key_start..].iter().position(|byte| *byte == b'"') else {
            break;
        };
        let key_end = key_start + relative_quote;
        let key = &source[key_start..key_end];
        if key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            let line = source[..identifier_start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            keys.push((line, key.to_string()));
        }
        index = key_end + 1;
    }

    keys
}

#[test]
fn init_is_idempotent() {
    init(Some("en"));
    init(Some("en"));
    assert!(loader().is_some());
}

#[test]
fn fallback_chain_works() {
    let loader = fluent_language_loader!();
    loader.load_fallback_language(&MergedLocalizations).unwrap();
    let languages = ["zh-CN".parse().unwrap()];
    i18n_embed::select(&loader, &MergedLocalizations, &languages).unwrap();
    // common-ok 已提供简体中文翻译。
    assert_eq!(loader.get("common-ok"), "确定");
    // 不存在的键会以原键或带标记的文本返回。
    let missing = loader.get("definitely-does-not-exist");
    assert!(missing.contains("definitely-does-not-exist"));
}

#[test]
fn shared_runtime_loader_includes_tui_resources() {
    let loader = fluent_language_loader!();
    loader.load_fallback_language(&MergedLocalizations).unwrap();

    assert_eq!(loader.get("tui-hint-shortcuts"), "? for shortcuts");
}

#[test]
fn requested_languages_keep_preferred_order() {
    let languages = ["zh-CN", "en"]
        .into_iter()
        .filter_map(parse_language_identifier)
        .collect();

    let languages = languages_or_fallback(languages);

    assert_eq!(languages[0].to_string(), "zh-CN");
    assert_eq!(languages[1].to_string(), "en");
}

#[test]
fn requested_languages_fall_back_to_english_when_empty() {
    let languages = languages_or_fallback(Vec::new());

    assert_eq!(languages.len(), 1);
    assert_eq!(languages[0].to_string(), "en");
}

#[test]
fn literal_translation_key_scanner_ignores_docs_and_finds_calls() {
    let source = r#"
//! 文档示例：t!("ignored-doc-key")
let first = crate::t!("first-key");
let second = warp::t_static!(
    "second-key",
);
let unrelated = format!("not-a-translation-key");
"#;

    assert_eq!(
        literal_translation_keys(source),
        vec![(3, "first-key".to_string()), (4, "second-key".to_string())]
    );
}

#[test]
fn english_and_simplified_chinese_have_the_same_message_ids() {
    let english = messages_for_locale("en");
    let simplified_chinese = messages_for_locale("zh-CN");
    let missing_in_chinese: Vec<_> = english
        .keys()
        .filter(|id| !simplified_chinese.contains_key(*id))
        .collect();
    let extra_in_chinese: Vec<_> = simplified_chinese
        .keys()
        .filter(|id| !english.contains_key(*id))
        .collect();

    assert!(
        missing_in_chinese.is_empty() && extra_in_chinese.is_empty(),
        "中英文 Fluent 消息键不一致；中文缺失：{missing_in_chinese:?}；中文多余：{extra_in_chinese:?}"
    );
}

#[test]
fn english_and_simplified_chinese_use_the_same_variables() {
    let english = messages_for_locale("en");
    let simplified_chinese = messages_for_locale("zh-CN");
    let mut mismatches = Vec::new();

    for (id, english_value) in &english {
        let Some(chinese_value) = simplified_chinese.get(id) else {
            continue;
        };
        let english_variables = message_variables(english_value);
        let chinese_variables = message_variables(chinese_value);
        if english_variables != chinese_variables {
            mismatches.push((id, english_variables, chinese_variables));
        }
    }

    assert!(
        mismatches.is_empty(),
        "中英文 Fluent 插值变量不一致：{mismatches:?}"
    );
}

#[test]
fn production_literal_translation_keys_exist_in_english_resources() {
    let english = messages_for_locale("en");
    let app_directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = app_directory
        .parent()
        .expect("app crate 应位于仓库根目录下");
    let source_roots = [
        app_directory.join("src"),
        repository.join("crates/warp_tui/src"),
    ];
    let mut missing = Vec::new();

    for source_root in source_roots {
        for path in rust_source_files(&source_root) {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("无法读取 Rust 源码 {}：{error}", path.display()));
            for (line, key) in literal_translation_keys(&source) {
                if !english.contains_key(&key) {
                    let relative_path = path.strip_prefix(repository).unwrap_or(&path);
                    missing.push(format!("{}:{line}: {key}", relative_path.display()));
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "生产代码引用了英文资源中不存在的 Fluent 消息键：\n{}",
        missing.join("\n")
    );
}
