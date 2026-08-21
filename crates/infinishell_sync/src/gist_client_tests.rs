use super::*;

#[test]
fn builds_github_auth_header() {
    let header = GistClient::auth_header(SyncPlatform::GitHub, "mytoken");

    assert_eq!(header, "Bearer mytoken");
}

#[test]
fn builds_gitee_auth_header() {
    let header = GistClient::auth_header(SyncPlatform::Gitee, "mytoken");

    assert_eq!(header, "token mytoken");
}

#[test]
fn reads_legacy_zap_config_file_when_current_file_is_absent() {
    let detail = serde_json::json!({
        "files": {
            "zap_config.json": { "content": "legacy" }
        }
    });

    assert_eq!(sync_file(&detail).unwrap()["content"], "legacy");
}

#[test]
fn prefers_current_config_file_when_both_names_exist() {
    let detail = serde_json::json!({
        "files": {
            "infinishell_config.json": { "content": "current" },
            "zap_config.json": { "content": "legacy" }
        }
    });

    assert_eq!(sync_file(&detail).unwrap()["content"], "current");
}

#[tokio::test]
async fn rejects_empty_tokens_before_sending_requests() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let client = GistClient::new();

    assert!(matches!(
        client.validate_token(SyncPlatform::GitHub, "").await,
        Err(GistClientError::NoToken)
    ));
    assert!(matches!(
        client.find_gist(SyncPlatform::GitHub, "").await,
        Err(GistClientError::NoToken)
    ));
    assert!(matches!(
        client.create_gist(SyncPlatform::GitHub, "", "{}").await,
        Err(GistClientError::NoToken)
    ));
    assert!(matches!(
        client
            .update_gist(SyncPlatform::GitHub, "", "x", "{}")
            .await,
        Err(GistClientError::NoToken)
    ));
    assert!(matches!(
        client.get_gist_content(SyncPlatform::GitHub, "", "x").await,
        Err(GistClientError::NoToken)
    ));
}
