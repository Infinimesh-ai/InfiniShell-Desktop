use std::sync::Arc;

use remote_server::auth::RemoteServerAuthContext;
use warpui::r#async::BoxFuture;

use crate::auth::AuthState;

/// 构造给 remote-server 模块使用的 auth context。
///
/// Zap Wave 3-1:`AuthClient` trait 已物理删。Bearer token 来源改为直接读取
/// `AuthState::get_access_token_ignoring_validity()`(在 Zap 路径下仅在用户挂了
/// BYOP API key 时返回 `Some`,其余永远 `None`)。
pub fn server_api_auth_context(auth_state: Arc<AuthState>) -> RemoteServerAuthContext {
    let token_auth_state = auth_state.clone();
    let identity_auth_state = auth_state;

    RemoteServerAuthContext::new(
        move || -> BoxFuture<'static, Option<String>> {
            let token = token_auth_state.get_access_token_ignoring_validity();
            Box::pin(async move { token })
        },
        move || remote_server_identity_key(&identity_auth_state),
    )
}

fn remote_server_identity_key(auth_state: &AuthState) -> String {
    // Zap 不再区分匿名 / 已登录身份,统一用 `user_id()`(本地测试 UID)。
    auth_state
        .user_id()
        .map(|uid| uid.as_string())
        .unwrap_or_else(|| auth_state.anonymous_id())
}
