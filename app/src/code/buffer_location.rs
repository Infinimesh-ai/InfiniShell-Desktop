use warp_util::content_version::ContentVersion;
// Re-export from warp_util so existing app-level imports continue to work.
pub use warp_util::local_or_remote_path::LocalOrRemotePath;
pub use warp_util::remote_path::RemotePath;

/// Tracks sync state between client and server for a single remote buffer.
///
/// Uses a version vector with two components:
/// - `server_version`: bumped by the server when the file changes on disk.
/// - `client_version`: bumped by the client when the user edits the buffer.
///
/// Conflict detection:
/// - Server pushes `{S_new, C_expected}`. Client checks `C_expected == local client_version`.
///   Match → accept. Mismatch → conflict.
/// - Client sends `{S_expected, C_new}`. Server checks `S_expected == local server_version`.
///   Match → accept. Mismatch → reject (server pushes its current state).
///
/// Both fields use `ContentVersion` internally. At the wire boundary (proto
/// encode/decode), convert via `ContentVersion::as_u64()` and
/// `ContentVersion::from_raw()`.
#[derive(Clone, Debug)]
pub struct SyncClock {
    /// Last version acknowledged from the server (file-watcher side).
    pub server_version: ContentVersion,
    /// Last version acknowledged from the client (user-edit side).
    pub client_version: ContentVersion,
}

impl SyncClock {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn new() -> Self {
        Self {
            server_version: ContentVersion::from_raw(0),
            client_version: ContentVersion::from_raw(0),
        }
    }

    /// Reconstruct a `SyncClock` from wire values (proto deserialization).
    /// 用 `from_wire_u64` 饱和而不是 `as usize`,避免 32-bit 平台上隐式截断。
    pub fn from_wire(server_version: u64, client_version: u64) -> Self {
        Self {
            server_version: ContentVersion::from_wire_u64(server_version),
            client_version: ContentVersion::from_wire_u64(client_version),
        }
    }

    /// Bump the server version after a file-watcher change.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn bump_server(&mut self) -> ContentVersion {
        self.server_version = ContentVersion::new();
        self.server_version
    }

    /// Check whether a server push's expected client version matches our local state.
    pub fn server_push_matches(&self, expected_client_version: ContentVersion) -> bool {
        self.client_version == expected_client_version
    }

    /// Check whether a client edit's expected server version matches our local state.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn client_edit_matches(&self, expected_server_version: ContentVersion) -> bool {
        self.server_version == expected_server_version
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use warp_util::file_type::is_markdown_file;
    use warp_util::host_id::HostId;
    use warp_util::standardized_path::StandardizedPath;

    use super::*;

    fn remote(path: &str) -> LocalOrRemotePath {
        LocalOrRemotePath::Remote(RemotePath::new(
            HostId::new("test-host".to_string()),
            StandardizedPath::try_new(path).unwrap(),
        ))
    }

    /// 远端路径转成仅用于后缀识别的 `PathBuf`(不做文件系统访问)。
    fn language_path(location: &LocalOrRemotePath) -> PathBuf {
        match location {
            LocalOrRemotePath::Local(path) => path.clone(),
            LocalOrRemotePath::Remote(remote) => PathBuf::from(remote.path.as_str()),
        }
    }

    #[test]
    fn remote_markdown_detected_via_language_path() {
        // 远端文件没有本地路径,Markdown 识别必须只取后缀。
        assert!(is_markdown_file(language_path(&remote(
            "/home/user/notes/README.md"
        ))));
        assert!(is_markdown_file(language_path(&remote(
            "/home/user/doc.markdown"
        ))));
        assert!(is_markdown_file(language_path(&remote("/srv/CHANGELOG"))));
        assert!(!is_markdown_file(language_path(&remote(
            "/home/user/src/main.rs"
        ))));
        assert!(!is_markdown_file(language_path(&remote(
            "/home/user/data.json"
        ))));
    }
}
