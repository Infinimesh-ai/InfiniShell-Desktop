//! infinishell_sftp::error 模块单元测试
//!
//! author: logic
//! date: 2026/05/26

use infinishell_sftp::error::{SftpChannelError, SftpError};

// ============================================================
// SftpError Display 测试
// ============================================================

/// 验证 ConnectionFailed 格式化输出
#[test]
fn test_sftp_error_connection_failed() {
    let err = SftpError::ConnectionFailed("host unreachable".to_string());
    assert_eq!(format!("{err}"), "连接失败: host unreachable");
}

/// 验证 AuthFailed 格式化输出
#[test]
fn test_sftp_error_auth_failed() {
    let err = SftpError::AuthFailed("bad password".to_string());
    assert_eq!(format!("{err}"), "认证失败: bad password");
}

/// 验证 Timeout 格式化输出
#[test]
fn test_sftp_error_timeout() {
    let err = SftpError::Timeout;
    assert_eq!(format!("{err}"), "操作超时");
}

/// 验证 NoSuchFile 格式化输出
#[test]
fn test_sftp_error_no_such_file() {
    let err = SftpError::NoSuchFile("/tmp/missing.txt".to_string());
    assert_eq!(format!("{err}"), "文件未找到: /tmp/missing.txt");
}

/// 验证 PermissionDenied 格式化输出
#[test]
fn test_sftp_error_permission_denied() {
    let err = SftpError::PermissionDenied("/root/secret".to_string());
    assert_eq!(format!("{err}"), "权限不足: /root/secret");
}

/// 验证 General 格式化输出
#[test]
fn test_sftp_error_general() {
    let err = SftpError::General("something went wrong".to_string());
    assert_eq!(format!("{err}"), "操作失败: something went wrong");
}

// ============================================================
// SftpChannelError Display 测试
// ============================================================

/// 验证 SendFailed 格式化输出
#[test]
fn test_sftp_channel_error_send_failed() {
    let err = SftpChannelError::SendFailed("channel closed".to_string());
    assert_eq!(format!("{err}"), "发送请求失败: channel closed");
}

/// 验证 RecvFailed 格式化输出
#[test]
fn test_sftp_channel_error_recv_failed() {
    let err = SftpChannelError::RecvFailed("timeout".to_string());
    assert_eq!(format!("{err}"), "接收响应失败: timeout");
}

// ============================================================
// From<SftpError> for SftpChannelError 测试
// ============================================================

/// 验证 SftpError 可转换为 SftpChannelError::Sftp
#[test]
fn test_sftp_channel_error_from_sftp_error() {
    let sftp_err = SftpError::General("inner error".to_string());
    let channel_err: SftpChannelError = sftp_err.into();
    match channel_err {
        SftpChannelError::Sftp(inner) => {
            assert_eq!(format!("{inner}"), "操作失败: inner error");
        }
        _ => panic!("期望 SftpChannelError::Sftp 变体"),
    }
}
