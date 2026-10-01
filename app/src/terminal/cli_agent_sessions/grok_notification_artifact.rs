//! 远端通知能力使用独立原生工件，不能以官方版本或旧终端桥工件替代。

use std::io;

pub(crate) const VERSION: &str = "1.0.41+infinishell.session-notifications.4";
pub(crate) const BUILD_CONTRACT: &str = "infinishell-terminal-bridge-v1+session-notifications-v1";

// 原生构建及来源审计完成后绑定真实摘要；未绑定期间不能启动此入口。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SHA256: Option<&str> =
    Some("b196c3a073a37a109af57ddb4d12d45ca40eaa2a99d6b08c5c5a9c88ddab2ad3");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SHA256: Option<&str> =
    Some("d128a7b8f624368f8ae16cba1c40f11d16962b0ce5eee0b37450b889087fd8c6");

pub(crate) fn sha256() -> io::Result<&'static str> {
    SHA256.ok_or_else(|| io::Error::other("Grok 会话通知原生工件尚未绑定"))
}

// 完整 --version 包含来源提交；与摘要一起在原生构建后绑定。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const VERSION_OUTPUT: Option<&str> =
    Some("grok 1.0.41+infinishell.session-notifications.4 (ba8ce6d346aa)");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const VERSION_OUTPUT: Option<&str> =
    Some("grok 1.0.41+infinishell.session-notifications.4 (07e35a3dfeed)");

pub(crate) fn version_output() -> io::Result<&'static str> {
    VERSION_OUTPUT.ok_or_else(|| io::Error::other("Grok 会话通知原生版本输出尚未绑定"))
}
