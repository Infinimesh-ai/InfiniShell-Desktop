use super::*;
use crate::ai::byop_readiness::{BlockedByopReadinessError, ReadinessCategory};

#[test]
fn byop_blocked_readiness_error_is_not_retryable() {
    let error = AIApiError::Other(
        BlockedByopReadinessError::new(ReadinessCategory::MissingResultWithoutRepairSource).into(),
    );

    assert!(!error.is_retryable());
}

#[test]
fn provider_protocol_error_is_not_retried_blindly() {
    let error = AIApiError::ProviderProtocol("response.incomplete".to_owned());

    assert!(!error.is_retryable());
}

#[test]
fn current_infinishell_quota_header_is_recognized() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        INFINISHELL_ERROR_CODE_HEADER,
        http::HeaderValue::from_static(WARP_ERROR_CODE_OUT_OF_CREDITS),
    );

    assert!(matches!(
        AIApiError::error_for_429(&headers),
        AIApiError::QuotaLimit
    ));
}

#[test]
fn legacy_zap_quota_header_is_recognized() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        LEGACY_ZAP_ERROR_CODE_HEADER,
        http::HeaderValue::from_static(WARP_ERROR_CODE_OUT_OF_CREDITS),
    );

    assert!(matches!(
        AIApiError::error_for_429(&headers),
        AIApiError::QuotaLimit
    ));
}
