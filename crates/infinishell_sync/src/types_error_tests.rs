use std::error::Error;

use super::*;

#[derive(Debug, thiserror::Error)]
#[error("typed provider diagnostic")]
struct ProviderDiagnostic;

#[test]
fn gist_errors_keep_categories_through_the_sync_engine_boundary() {
    for error in [
        GistClientError::NoToken,
        GistClientError::NotFound,
        GistClientError::MissingLogin,
    ] {
        let category = std::mem::discriminant(&error);
        let engine_error = SyncEngineError::from(error);
        assert!(
            engine_error
                .source()
                .unwrap()
                .downcast_ref::<GistClientError>()
                .is_some()
        );
        let SyncEngineError::Gist(source) = engine_error else {
            panic!("Gist 类型不能退化为字符串");
        };
        assert_eq!(std::mem::discriminant(&source), category);
    }
}

#[test]
fn crypto_errors_keep_the_operation_for_ui_localization() {
    let error = SyncEngineError::from(CryptoError::Decrypt("ciphertext-marker".into()));
    let Some(CryptoError::Decrypt(detail)) = error.source().unwrap().downcast_ref() else {
        panic!("同步边界必须保留解密错误及诊断来源");
    };
    assert_eq!(detail, "ciphertext-marker");
}

#[test]
fn provider_errors_keep_business_types_and_diagnostic_sources() {
    let error = SyncEngineError::Provider(anyhow::Error::new(ProviderDiagnostic));
    let SyncEngineError::Provider(source) = error else {
        panic!("provider 边界必须保留原始错误");
    };
    assert!(source.downcast_ref::<ProviderDiagnostic>().is_some());
    assert_eq!(source.to_string(), "typed provider diagnostic");
}
