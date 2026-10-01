//! 远端通知能力使用独立原生工件，不能以官方版本或旧终端桥工件替代。

use std::io;

pub(crate) const VERSION: &str = "1.0.41+infinishell.session-notifications.1";
pub(crate) const BUILD_CONTRACT: &str = "infinishell-terminal-bridge-v1+session-notifications-v1";

// 原生构建及来源审计完成后绑定真实摘要；未绑定期间不能启动此入口。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SHA256: Option<&str> =
    Some("0f3d4aa695e1d596c120be296dc68d1942dacc8c2a6320dc887cda127d0ea3f0");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SHA256: Option<&str> = None;

pub(crate) fn sha256() -> io::Result<&'static str> {
    SHA256.ok_or_else(|| io::Error::other("Grok 会话通知原生工件尚未绑定"))
}

// 完整 --version 包含来源提交；与摘要一起在原生构建后绑定。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const VERSION_OUTPUT: Option<&str> =
    Some("grok 1.0.41+infinishell.session-notifications.1 (13a09e440e29)");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const VERSION_OUTPUT: Option<&str> = None;

pub(crate) fn version_output() -> io::Result<&'static str> {
    VERSION_OUTPUT.ok_or_else(|| io::Error::other("Grok 会话通知原生版本输出尚未绑定"))
}
