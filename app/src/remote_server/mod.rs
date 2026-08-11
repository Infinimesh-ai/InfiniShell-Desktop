// Re-export everything from the `remote_server` crate so existing
// `crate::remote_server::*` imports in `app` continue to work.
pub use remote_server::*;

#[cfg(not(target_family = "wasm"))]
pub mod auth_context;
#[cfg(not(target_family = "wasm"))]
pub mod codebase_index_model;
#[cfg(not(target_family = "wasm"))]
mod codebase_index_status;
pub mod diff_state_proto;
#[cfg(not(target_family = "wasm"))]
pub mod diff_state_tracker;
pub mod git_status_proto;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod handoff_snapshot;
#[cfg(not(target_family = "wasm"))]
mod ripgrep_search;
#[cfg(not(target_family = "wasm"))]
pub mod server_buffer_tracker;
#[cfg(not(target_family = "wasm"))]
pub mod server_model;
#[cfg(not(target_family = "wasm"))]
pub mod ssh_transport;
#[cfg(unix)]
pub mod unix;

/// Run the `remote-server-proxy` subcommand.
#[cfg(unix)]
pub fn run_proxy(identity_key: String) -> anyhow::Result<()> {
    unix::proxy::run(&identity_key)
}

#[cfg(not(unix))]
pub fn run_proxy(_identity_key: String) -> anyhow::Result<()> {
    anyhow::bail!("remote-server-proxy is not supported on this platform")
}

/// Run the `remote-server-daemon` subcommand.
#[cfg(unix)]
pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    unix::run_daemon(identity_key)
}

#[cfg(not(unix))]
pub fn run_daemon(_identity_key: String) -> anyhow::Result<()> {
    anyhow::bail!("remote-server-daemon is not supported on this platform")
}

// 上游把 daemon 启动改成走统一的 `run_internal` / `launch()` 路径
// (`unix::launch_daemon`,见 `lib.rs` 的 `LaunchMode::RemoteServerDaemon` 分支),
// 我方原先的 `run_daemon_app` headless AppBuilder 封装随之删除:
// DirectoryWatcher / DetectedRepositories / RepoMetadataModel / FileModel /
// GlobalBufferModel 的注册与顺序约束现在由 `initialize_app` 统一负责。

// Zap Wave 6-1:`wire_auth_token_rotation` 函数物理删 — 原订阅 server API
// token rotation 事件并转发到 `RemoteServerManager::rotate_auth_token`。Wave 3-1
// 删 auth 子系统后该事件 0 emit 点,Wave 6-1 同步删事件 + 本订阅函数 + `lib.rs`
// 中的调用点。`RemoteServerManager::rotate_auth_token` 函数本体暂保留。
//
// 上游在本次合并里给该函数追加了 crash-reporting 偏好 / codebase-index limits
// 的转发(`current_codebase_index_limits`),依赖 `warp_server_client::auth` 与
// `crate::server::server_api::ServerApiProvider`,两者均已从本 fork 剥离,故一并不引入。
