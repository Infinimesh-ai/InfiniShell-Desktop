use super::*;
use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::{MCPServer, MCPServerExt as _, TransportType};

// Zap:上游此处还有两个 Firebase 托管 token 的用例
// (`bearer_token_uses_a_valid_firebase_id_token` /
// `bearer_token_rejects_a_firebase_token_about_to_expire`)。本地化后
// `crate::auth::Credentials` 只剩 `ApiKey` / `Test`,没有 Firebase 托管 token
// 也没有过期竞态,`builtin_bearer_token` 里的过期检查已随之删除,用例一并删除。

#[test]
fn bearer_token_uses_api_keys() {
    let credentials = Credentials::ApiKey {
        key: "wk-test-key".to_string(),
        owner_type: None,
    };
    assert_eq!(
        builtin_bearer_token(&credentials),
        Some("wk-test-key".to_string())
    );
}

/// Zap:上游用 `Credentials::SessionCookie` 覆盖"无 bearer token 的凭据"这一路,
/// 本地化后该 variant 已删除,改用同样产出 `AuthToken::NoAuth` 的 `Credentials::Test`
/// 保住这条覆盖。
#[test]
fn bearer_token_rejects_credentials_without_a_token() {
    assert_eq!(builtin_bearer_token(&Credentials::Test), None);
}

#[test]
fn factory_mcp_url_joins_server_roots_with_and_without_trailing_slash() {
    assert_eq!(
        factory_mcp_url("https://app.warp.dev"),
        "https://app.warp.dev/api/v1/mcp/factory"
    );
    assert_eq!(
        factory_mcp_url("http://localhost:8080/"),
        "http://localhost:8080/api/v1/mcp/factory"
    );
}

#[test]
fn factory_installation_resolves_to_a_preauthenticated_http_server() {
    let installation =
        factory_mcp_installation_for_server_root("https://staging.warp.dev", "tok-123");
    assert_eq!(installation.uuid(), FACTORY_MCP_INSTALLATION_UUID);
    // Fully resolved: nothing for the variable-prompt UI to ask for, and
    // nothing for handlebars to substitute at spawn time.
    assert!(installation.template_variables().is_empty());

    // The resolved JSON must parse into a single HTTP server with the
    // pre-authenticated header, exactly as `spawn_server_impl` will see it.
    let resolved = resolve_json(&installation);
    let mut servers =
        MCPServer::from_user_json(&resolved).expect("built-in template must parse as MCP config");
    assert_eq!(servers.len(), 1);
    let server = servers.pop().expect("one server");
    assert_eq!(server.name, FACTORY_MCP_SERVER_NAME);
    match server.transport_type {
        TransportType::ServerSentEvents(sse) => {
            assert_eq!(sse.url, "https://staging.warp.dev/api/v1/mcp/factory");
            assert_eq!(sse.headers.len(), 1);
            assert_eq!(sse.headers[0].name, "Authorization");
            assert_eq!(sse.headers[0].value, "Bearer tok-123");
        }
        TransportType::CLIServer(_) => panic!("expected an HTTP transport"),
    }
}
