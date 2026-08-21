use super::EditorChoice;

#[test]
fn serializes_the_current_infinishell_editor_name() {
    assert_eq!(
        serde_json::to_string(&EditorChoice::InfiniShell).unwrap(),
        r#""InfiniShell""#
    );
}

#[test]
fn deserializes_the_legacy_zap_editor_name() {
    assert_eq!(
        serde_json::from_str::<EditorChoice>(r#""Zap""#).unwrap(),
        EditorChoice::InfiniShell
    );
}
