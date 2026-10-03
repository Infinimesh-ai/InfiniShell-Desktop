use std::ffi::{CString, c_void};
use std::fs;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{PermissionsExt as _, symlink};

use super::*;

fn prefix() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let prefix = temporary.path().canonicalize().unwrap().join("brew");
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(prefix.join("bin/brew"), b"unit-test manager identity").unwrap();
    (temporary, prefix)
}

#[test]
fn selected_public_entry_can_bind_a_safe_private_prefix() {
    let (_temporary, prefix) = prefix();

    assert_eq!(
        prefix_from_entry(CLIAgent::Codex, &prefix.join("bin/codex")),
        Ok(prefix.clone())
    );
    assert_eq!(
        prefix_from_entry(CLIAgent::Claude, &prefix.join("bin/claude")),
        Ok(prefix.clone())
    );
    assert_eq!(
        prefix_from_entry(CLIAgent::Grok, &prefix.join("bin/grok")),
        Ok(prefix.clone())
    );
    assert!(prefix_from_entry(CLIAgent::Codex, &prefix.join("other/codex")).is_err());
    assert!(prefix_from_entry(CLIAgent::Codex, &prefix.join("bin/claude")).is_err());
    assert_eq!(
        manager_stamp(&prefix).unwrap(),
        stamp(&prefix.join("bin/brew")).unwrap()
    );
}

#[test]
fn private_prefix_rejects_symlink_and_writable_ancestor() {
    let (temporary, prefix) = prefix();
    let alias = temporary.path().canonicalize().unwrap().join("alias");
    symlink(&prefix, &alias).unwrap();
    assert!(validate_prefix(&alias).is_err());

    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o770)).unwrap();
    let result = validate_prefix(&prefix);
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Err(Error::UnsupportedSource));
    assert_eq!(validate_prefix(&prefix), Ok(()));
}

#[test]
fn manager_public_link_cannot_escape_the_selected_prefix() {
    let (temporary, prefix) = prefix();
    let outside = temporary.path().join("outside-brew");
    fs::write(&outside, b"another manager").unwrap();
    fs::remove_file(prefix.join("bin/brew")).unwrap();
    symlink(outside, prefix.join("bin/brew")).unwrap();

    assert_eq!(manager_stamp(&prefix), Err(Error::UnsupportedSource));
}

unsafe extern "C" {
    fn acl_init(count: libc::c_int) -> *mut c_void;
    fn acl_create_entry(acl: *mut *mut c_void, entry: *mut *mut c_void) -> libc::c_int;
    fn acl_set_tag_type(entry: *mut c_void, tag: libc::c_int) -> libc::c_int;
    fn acl_set_qualifier(entry: *mut c_void, qualifier: *const c_void) -> libc::c_int;
    fn acl_get_permset(entry: *mut c_void, permissions: *mut *mut c_void) -> libc::c_int;
    fn acl_add_perm(permissions: *mut c_void, permission: libc::c_int) -> libc::c_int;
    fn acl_set_file(path: *const libc::c_char, kind: libc::c_int, acl: *mut c_void) -> libc::c_int;
    fn acl_free(acl: *mut c_void) -> libc::c_int;
    fn mbr_uid_to_uuid(uid: libc::uid_t, uuid: *mut u8) -> libc::c_int;
}

struct AncestorAcl(CString);

impl AncestorAcl {
    fn set(path: &Path, tag: i32, permission: i32) -> Self {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut uuid = [0; 16];
        assert_eq!(
            unsafe { mbr_uid_to_uuid(libc::geteuid(), uuid.as_mut_ptr()) },
            0
        );
        let mut acl = unsafe { acl_init(1) };
        assert!(!acl.is_null());
        let mut entry = std::ptr::null_mut();
        assert_eq!(unsafe { acl_create_entry(&mut acl, &mut entry) }, 0);
        assert_eq!(unsafe { acl_set_tag_type(entry, tag) }, 0);
        assert_eq!(unsafe { acl_set_qualifier(entry, uuid.as_ptr().cast()) }, 0);
        let mut permissions = std::ptr::null_mut();
        assert_eq!(unsafe { acl_get_permset(entry, &mut permissions) }, 0);
        assert_eq!(unsafe { acl_add_perm(permissions, permission) }, 0);
        assert_eq!(unsafe { acl_set_file(path.as_ptr(), 0x100, acl) }, 0);
        unsafe { acl_free(acl) };
        Self(path)
    }
}

impl Drop for AncestorAcl {
    fn drop(&mut self) {
        let acl = unsafe { acl_init(0) };
        assert!(!acl.is_null());
        assert_eq!(unsafe { acl_set_file(self.0.as_ptr(), 0x100, acl) }, 0);
        unsafe { acl_free(acl) };
    }
}

#[test]
fn ancestor_allow_write_acl_is_rejected_without_changing_permissions() {
    let (temporary, prefix) = prefix();
    let _acl = AncestorAcl::set(temporary.path(), 1, 1 << 2);

    assert_eq!(validate_prefix(&prefix), Err(Error::UnsupportedSource));
}

#[test]
fn ancestor_deny_delete_and_allow_read_acl_remain_usable() {
    let (temporary, prefix) = prefix();
    let acl = AncestorAcl::set(temporary.path(), 2, 1 << 4);
    assert_eq!(validate_prefix(&prefix), Ok(()));
    drop(acl);

    let _acl = AncestorAcl::set(temporary.path(), 1, 1 << 1);
    assert_eq!(validate_prefix(&prefix), Ok(()));
}
#[test]
fn claude_current_casks_bind_official_metadata_to_reviewed_native_images() {
    for (token, version, bytes, length) in [
        (
            "claude-code",
            "2.1.285",
            include_bytes!("fixtures/claude-current-release/cask-claude-code.json").as_slice(),
            223_821_616,
        ),
        (
            "claude-code@latest",
            "2.1.287",
            include_bytes!("fixtures/claude-current-release/cask-claude-code@latest.json")
                .as_slice(),
            227_827_120,
        ),
    ] {
        let (value, url, digest) = release(token, version, bytes).unwrap();
        assert_eq!(value["version"], version);
        assert_eq!(
            url,
            format!(
                "https://downloads.claude.ai/claude-code-releases/{version}/darwin-arm64/claude"
            )
        );
        assert_eq!(
            super::super::claude_current_release::native(version, "darwin-arm64"),
            Ok((length, digest))
        );
        let mut changed = value.clone();
        changed["sha256"] = serde_json::json!("11".repeat(32));
        assert!(release(token, version, &serde_json::to_vec(&changed).unwrap()).is_err());
        let mut changed = value.clone();
        changed["artifacts"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"preflight":["unknown-script"]}));
        assert!(release(token, version, &serde_json::to_vec(&changed).unwrap()).is_err());
        assert!(release(token, "2.1.286", bytes).is_err());
        assert!(release("codex", version, bytes).is_err());
    }
}
