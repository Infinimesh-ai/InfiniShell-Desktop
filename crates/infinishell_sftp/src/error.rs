//! SFTP 协议层错误类型定义
//!
//! 定义 SftpError 和 SftpChannelError 两种错误枚举，
//! 覆盖连接、认证、超时、权限等错误场景。
//! author: logic
//! date: 2026-05-31

use thiserror::Error;

/// SFTP 协议级错误
#[derive(Debug, Error)]
pub enum SftpError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("SSH2 错误: {0}")]
    Ssh2(#[from] ssh2::Error),

    #[error("连接失败: {0}")]
    ConnectionFailed(String),

    #[error("认证失败: {0}")]
    AuthFailed(String),

    #[error("操作超时")]
    Timeout,

    #[error("文件未找到: {0}")]
    NoSuchFile(String),

    #[error("权限不足: {0}")]
    PermissionDenied(String),

    #[error("操作失败: {0}")]
    General(String),
}

/// SFTP 通道错误
#[derive(Debug, Error)]
pub enum SftpChannelError {
    #[error("SFTP 错误: {0}")]
    Sftp(#[from] SftpError),

    #[error("发送请求失败: {0}")]
    SendFailed(String),

    #[error("接收响应失败: {0}")]
    RecvFailed(String),
}

impl From<ssh2::Error> for SftpChannelError {
    fn from(e: ssh2::Error) -> Self {
        SftpChannelError::Sftp(SftpError::Ssh2(e))
    }
}
