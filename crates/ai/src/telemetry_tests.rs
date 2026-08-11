use serde_json::json;

use super::*;

/// Zap 已删除遥测上报,事件壳只剩分类元数据。这里锁定三个枚举的
/// 线格式(snake_case),确保壳里永远只出现分类值、不会混入 UGC。
#[test]
fn provider_credential_metadata_serializes_as_classification_only() {
    assert_eq!(
        serde_json::to_value(ProviderCredentialTelemetryProvider::Anthropic).unwrap(),
        json!("anthropic")
    );
    assert_eq!(
        serde_json::to_value(ProviderCredentialTelemetryKind::PastedKey).unwrap(),
        json!("pasted_key")
    );
    assert_eq!(
        serde_json::to_value(ProviderCredentialTelemetryAction::Added).unwrap(),
        json!("added")
    );
}
