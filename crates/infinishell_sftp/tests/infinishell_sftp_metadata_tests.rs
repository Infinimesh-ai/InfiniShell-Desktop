//! infinishell_sftp::types::Metadata::from_ssh2 模块单元测试
//!
//! 验证从 ssh2::FileStat 创建 Metadata 的逻辑，
//! 重点覆盖符号链接检测修复和各字段 Some/None 回退。
//! author: logic
//! date: 2026-05-27

use std::time::{Duration, SystemTime};

use infinishell_sftp::types::*;

/// 构造所有字段为 None 的空 ssh2::FileStat
fn empty_stat() -> ssh2::FileStat {
    ssh2::FileStat {
        size: None,
        uid: None,
        gid: None,
        perm: None,
        atime: None,
        mtime: None,
    }
}

// ============================================================
// Metadata::from_ssh2 — 文件类型检测
// ============================================================

/// 验证 perm 包含目录 mode 位时 file_type 为 Dir
#[test]
fn test_metadata_from_ssh2_dir() {
    let stat = ssh2::FileStat {
        perm: Some(0o040755),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::Dir);
}

/// 验证 perm 包含常规文件 mode 位时 file_type 为 File
#[test]
fn test_metadata_from_ssh2_file() {
    let stat = ssh2::FileStat {
        perm: Some(0o100644),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::File);
}

/// 验证 perm 包含符号链接 mode 位时 file_type 为 Symlink（修复验证）
#[test]
fn test_metadata_from_ssh2_symlink() {
    let stat = ssh2::FileStat {
        perm: Some(0o120755),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::Symlink);
}

/// 验证 perm 为 None 时 file_type 回退为 Other
#[test]
fn test_metadata_from_ssh2_perm_none() {
    let stat = ssh2::FileStat {
        perm: None,
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::Other);
}

/// 验证未知 mode 位组合时 file_type 为 Other
#[test]
fn test_metadata_from_ssh2_unknown_mode() {
    let stat = ssh2::FileStat {
        perm: Some(0o050000),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::Other);
}

// ============================================================
// Metadata::from_ssh2 — 权限字段
// ============================================================

/// 验证权限位正确解析
#[test]
fn test_metadata_from_ssh2_permissions() {
    let stat = ssh2::FileStat {
        perm: Some(0o100755),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert!(meta.permissions.owner_read);
    assert!(meta.permissions.owner_write);
    assert!(meta.permissions.owner_exec);
    assert!(meta.permissions.group_read);
    assert!(!meta.permissions.group_write);
    assert!(meta.permissions.group_exec);
}

/// 验证 perm 为 None 时权限全部为 false
#[test]
fn test_metadata_from_ssh2_permissions_none() {
    let stat = ssh2::FileStat {
        perm: None,
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert!(!meta.permissions.owner_read);
    assert!(!meta.permissions.owner_write);
}

// ============================================================
// Metadata::from_ssh2 — 数值字段回退
// ============================================================

/// 验证 size/uid/gid 正常值
#[test]
fn test_metadata_from_ssh2_fields_present() {
    let stat = ssh2::FileStat {
        perm: Some(0o100644),
        size: Some(4096),
        uid: Some(1000),
        gid: Some(100),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.size, 4096);
    assert_eq!(meta.uid, 1000);
    assert_eq!(meta.gid, 100);
}

/// 验证 size/uid/gid 为 None 时回退为 0
#[test]
fn test_metadata_from_ssh2_fields_absent() {
    let stat = ssh2::FileStat {
        perm: Some(0o100644),
        size: None,
        uid: None,
        gid: None,
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.size, 0);
    assert_eq!(meta.uid, 0);
    assert_eq!(meta.gid, 0);
}

// ============================================================
// Metadata::from_ssh2 — 时间戳
// ============================================================

/// 验证 atime/mtime 正确转换为 SystemTime
#[test]
fn test_metadata_from_ssh2_timestamps_present() {
    let stat = ssh2::FileStat {
        perm: Some(0o100644),
        atime: Some(1609459200), // 2021-01-01 00:00:00 UTC
        mtime: Some(1609545600), // 2021-01-02 00:00:00 UTC
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    let expected_atime = SystemTime::UNIX_EPOCH + Duration::from_secs(1609459200);
    let expected_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1609545600);
    assert_eq!(meta.accessed, Some(expected_atime));
    assert_eq!(meta.modified, Some(expected_mtime));
}

/// 验证 atime/mtime 为 None 时 accessed/modified 为 None
#[test]
fn test_metadata_from_ssh2_timestamps_absent() {
    let stat = ssh2::FileStat {
        perm: Some(0o100644),
        atime: None,
        mtime: None,
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert!(meta.accessed.is_none());
    assert!(meta.modified.is_none());
}

// ============================================================
// Metadata::from_ssh2 — 完整字段组合
// ============================================================

/// 验证所有字段同时设置的完整场景
#[test]
fn test_metadata_from_ssh2_full_stat() {
    let stat = ssh2::FileStat {
        perm: Some(0o120777), // symlink + 777
        size: Some(11),
        uid: Some(501),
        gid: Some(20),
        atime: Some(1000000),
        mtime: Some(2000000),
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::Symlink);
    assert_eq!(meta.size, 11);
    assert_eq!(meta.uid, 501);
    assert_eq!(meta.gid, 20);
    assert!(meta.permissions.other_exec);
    assert!(meta.accessed.is_some());
    assert!(meta.modified.is_some());
}
