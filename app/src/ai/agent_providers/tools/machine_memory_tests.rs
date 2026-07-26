use super::*;

fn assert_serialized_sentinel(value: &Value) {
    assert_eq!(value["_byop_intercepted"], true);
    let serialized = serde_json::to_string(value).unwrap();
    assert!(serialized.contains(r#""_byop_intercepted":true"#));
}

#[test]
fn parses_complete_memory_document() {
    let args = parse_args(r###"{"content":"## System\nUbuntu 24.04"}"###).unwrap();
    assert_eq!(
        args,
        Args {
            content: "## System\nUbuntu 24.04".to_owned(),
        }
    );
}

#[test]
fn rejects_missing_or_non_string_content() {
    assert!(parse_args("{}").is_err());
    assert!(parse_args(r#"{"content":42}"#).is_err());
}

#[test]
fn truncates_content_by_unicode_characters() {
    let input = format!("{}tail", "机".repeat(MAX_MEMORY_CHARS));
    let truncated = truncate_content(&input);

    assert_eq!(truncated.chars().count(), MAX_MEMORY_CHARS);
    assert_eq!(truncated, "机".repeat(MAX_MEMORY_CHARS));
}

#[test]
fn intercepted_result_payloads_include_auto_resume_sentinel() {
    let success = success_result_to_json(123);
    assert_eq!(success["status"], "ok");
    assert_eq!(success["stored_chars"], 123);
    assert_serialized_sentinel(&success);

    let missing_key = missing_machine_key_result_to_json();
    assert_eq!(missing_key["status"], "error");
    assert_eq!(
        missing_key["message"],
        "not in an ssh session with machine identity"
    );
    assert_serialized_sentinel(&missing_key);

    let error = error_result_to_json("database unavailable");
    assert_eq!(error["status"], "error");
    assert_eq!(error["message"], "database unavailable");
    assert_serialized_sentinel(&error);

    let invalid = invalid_arguments_result_to_json("missing field `content`");
    assert_eq!(invalid["status"], "error");
    assert_eq!(
        invalid["message"],
        "invalid arguments: missing field `content`"
    );
    assert_serialized_sentinel(&invalid);
}
