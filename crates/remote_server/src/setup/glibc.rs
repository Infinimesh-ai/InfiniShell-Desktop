//! Parsing helpers for the libc family + version reported by the
//! `preinstall_check.sh` script.
//!
//! This is split into its own submodule so the libc-specific logic
//! (version parsing, family classification) can evolve independently
//! from the rest of [`crate::setup`].
//!
//! 注意:remote-server 二进制现在是静态 musl 链接(见 `preinstall_check.sh`),
//! 不依赖宿主 libc。这里解析出的 libc family / version 已不再用于安装门禁,
//! 仅作为遥测/诊断信息保留。`RemoteLibc` 与 `GlibcVersion` 继续被
//! [`crate::setup::PreinstallCheckResult`] 和 `UnsupportedReason` 使用。

use std::fmt;

/// A glibc `(major, minor)` version pair, e.g. `2.31`.
///
/// Wraps a labelled struct rather than a raw `(u32, u32)` so the meaning
/// of each field is obvious at call sites and in event payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlibcVersion {
    pub major: u32,
    pub minor: u32,
}

impl GlibcVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Parses a `<major>.<minor>` (or `<major>.<minor>.<patch>`) string.
    /// Only the first two segments are consumed; trailing components
    /// (e.g. patch versions, distro suffixes) are ignored.
    ///
    /// Returns `None` if either segment is missing or non-numeric.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let (major, rest) = value.split_once('.')?;
        // Allow `2.31`, `2.31.0`, `2.31-foo`, etc.
        let minor = rest.split(|c: char| !c.is_ascii_digit()).next()?;
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }
}

impl fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Detected libc on the remote host, derived from the `libc_family` /
/// `libc_version` keys emitted by `preinstall_check.sh`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteLibc {
    Glibc(GlibcVersion),
    NonGlibc { name: String },
    Unknown,
}

/// Builds a [`RemoteLibc`] from the raw `libc_family` / `libc_version`
/// values pulled out of the script's `key=value` stdout.
pub(super) fn parse_libc(family: Option<&str>, version: Option<&str>) -> RemoteLibc {
    match family {
        Some("glibc") => match version.and_then(GlibcVersion::parse) {
            Some(v) => RemoteLibc::Glibc(v),
            None => RemoteLibc::Unknown,
        },
        Some(name) if !name.is_empty() && name != "unknown" => RemoteLibc::NonGlibc {
            name: name.to_string(),
        },
        _ => RemoteLibc::Unknown,
    }
}

#[cfg(test)]
#[path = "glibc_tests.rs"]
mod tests;
