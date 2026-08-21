//! infinishell_sftp — SFTP 协议层封装
//!
//! 基于 ssh2 crate 封装 SFTP 文件传输协议，提供会话管理、
//! 远程文件读写、目录操作等功能。
//! author: logic
//! date: 2026-05-31

pub mod dir;
pub mod error;
pub mod file;
pub mod session;
pub mod sftp;
pub mod types;

pub use dir::Dir;
pub use error::{SftpChannelError, SftpError};
pub use file::File;
pub use session::{AuthMethod, SftpSession};
pub use sftp::Sftp;
pub use types::*;
