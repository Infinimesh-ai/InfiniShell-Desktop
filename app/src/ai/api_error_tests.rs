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
