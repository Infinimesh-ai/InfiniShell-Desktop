//! SFTP 远程文件句柄模块
//!
//! 封装 ssh2::File 提供远程文件的读写操作，
//! 支持 Read/Write trait 和流式传输。
//! author: logic
//! date: 2026-05-31

use std::io::{Read, Write};

use crate::error::SftpError;
use crate::types::OpenOptions;

/// SFTP 远程文件句柄
pub struct File {
    handle: ssh2::File,
}

impl File {
    /// 打开远程文件
    pub(crate) fn open(
        sftp: &ssh2::Sftp,
        path: &std::path::Path,
        options: &OpenOptions,
    ) -> Result<Self, SftpError> {
        let mut flags = ssh2::OpenFlags::empty();
        if options.read {
            flags |= ssh2::OpenFlags::READ;
        }
        if options.write.is_some() {
            flags |= ssh2::OpenFlags::WRITE;
        }
        if options.create && options.truncate {
            flags |= ssh2::OpenFlags::CREATE;
            flags |= ssh2::OpenFlags::TRUNCATE;
        } else if options.create {
            flags |= ssh2::OpenFlags::CREATE;
        }
        if matches!(options.write, Some(crate::types::WriteMode::Append)) {
            flags |= ssh2::OpenFlags::APPEND;
        }

        let handle = sftp.open_mode(
            path,
            flags,
            options.mode.unwrap_or(0o644) as i32,
            ssh2::OpenType::File,
        )?;
        Ok(File { handle })
    }

    /// 读取文件全部内容
    pub fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<u64, SftpError> {
        let n = self.handle.read_to_end(buf)?;
        Ok(n as u64)
    }

    /// 写入全部内容
    pub fn write_all(&mut self, buf: &[u8]) -> Result<(), SftpError> {
        self.handle.write_all(buf)?;
        Ok(())
    }

    /// 刷新写入缓冲
    pub fn flush(&mut self) -> Result<(), SftpError> {
        self.handle.flush()?;
        Ok(())
    }

    /// 获取文件元数据
    pub fn stat(&mut self) -> Result<crate::types::Metadata, SftpError> {
        let stat = self.handle.stat()?;
        Ok(crate::types::Metadata::from_ssh2(stat))
    }

    /// 读取一块数据到缓冲区
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, SftpError> {
        let n = self.handle.read(buf)?;
        Ok(n)
    }
}
