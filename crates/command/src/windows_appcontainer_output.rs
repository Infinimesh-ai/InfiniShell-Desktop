//! 固定版本候选使用普通文件输出，避免把新登录身份缓存到宿主的标准输出管道。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, Write};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, IntoRawHandle as _};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_TYPE_DISK,
    GetFileInformationByHandle, GetFileType,
};

use super::{
    ProfileDirectoryIdentity, local_profile_path, open_profile_directory,
    profile_directory_identity,
};

// 与更新器现有 stdout 上限一致；stderr 没有新增长度限制。
const MAX_STDOUT: u64 = 1024 * 1024;
const MAX_CANCELLED_STDERR: u64 = 8192;
const CANCELLED_STDERR_RECEIPT: &str = "cancelled-candidate-stderr.safe.json";

struct OutputFile {
    file: File,
    path: PathBuf,
}

impl OutputFile {
    fn create(directory: &Path, name: &str) -> io::Result<Self> {
        let path = directory.join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(0)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_DELETE_ON_CLOSE).0)
            .open(&path)?;
        let result = Self { file, path };
        if result.identity()?.3 != 0 {
            return Err(io::Error::other("版本探针输出文件不是新空文件"));
        }
        Ok(result)
    }

    fn identity(&self) -> io::Result<(u32, u32, u32, u64)> {
        let handle = HANDLE(self.file.as_raw_handle());
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(io::Error::other("版本探针输出不是普通磁盘文件"));
        }
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(handle, &mut information) }
            .map_err(io::Error::other)?;
        if information.dwFileAttributes
            & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT).0
            != 0
            || information.nNumberOfLinks != 1
        {
            return Err(io::Error::other("版本探针输出文件类型或硬链接无效"));
        }
        Ok((
            information.dwVolumeSerialNumber,
            information.nFileIndexHigh,
            information.nFileIndexLow,
            (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow),
        ))
    }

    fn copy_to(
        &mut self,
        writer: &mut impl Write,
        maximum: Option<u64>,
        check_cancelled: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let before = self.identity()?;
        if maximum.is_some_and(|maximum| before.3 > maximum) {
            return Err(io::Error::other("版本探针 stdout 超出原有上限"));
        }
        self.file.rewind()?;
        let mut remaining = before.3;
        let mut buffer = [0; 8192];
        // 仅读取退出时的精确长度，不读到未知 EOF，也不把完整 stderr 放入内存。
        while remaining != 0 {
            check_cancelled()?;
            let length = remaining.min(buffer.len() as u64) as usize;
            self.file.read_exact(&mut buffer[..length])?;
            check_cancelled()?;
            writer.write_all(&buffer[..length])?;
            remaining -= length as u64;
        }
        check_cancelled()?;
        writer.flush()?;
        if self.identity()? != before {
            return Err(io::Error::other("版本探针退出后输出发生变化"));
        }
        Ok(())
    }

    fn close(self) -> io::Result<()> {
        unsafe { CloseHandle(HANDLE(self.file.into_raw_handle())) }.map_err(io::Error::other)?;
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
            Ok(_) => Err(io::Error::other("版本探针输出仍有未关闭的句柄或同名对象")),
        }
    }
}

pub(super) struct CapturedOutput {
    directory: PathBuf,
    directories: Vec<(File, ProfileDirectoryIdentity)>,
    stdout: OutputFile,
    stderr: OutputFile,
}

impl CapturedOutput {
    pub(super) fn create(directory: &Path) -> io::Result<Self> {
        if !local_profile_path(directory) {
            return Err(io::Error::other("版本探针输出目录不是本地绝对目录"));
        }
        let mut directories = Vec::new();
        // 沿原代次记录目录逐级锁住祖先，拒绝重解析点，不修改任何目录 ACL。
        for ancestor in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let file = open_profile_directory(ancestor)?;
            let identity = profile_directory_identity(&file)?;
            directories.push((file, identity));
        }
        let stdout = OutputFile::create(directory, "candidate.stdout")?;
        let stderr = OutputFile::create(directory, "candidate.stderr")?;
        Ok(Self {
            directory: directory.to_owned(),
            directories,
            stdout,
            stderr,
        })
    }

    pub(super) fn stdout_handle(&self) -> HANDLE {
        HANDLE(self.stdout.file.as_raw_handle())
    }

    pub(super) fn stderr_handle(&self) -> HANDLE {
        HANDLE(self.stderr.file.as_raw_handle())
    }

    fn preserve_cancelled_stderr(&mut self) -> io::Result<()> {
        for (file, identity) in &self.directories {
            if profile_directory_identity(file)? != *identity {
                return Err(io::Error::other("版本探针取消输出目录身份变化"));
            }
        }
        let before = self.stderr.identity()?;
        let mut prefix = vec![0; before.3.min(MAX_CANCELLED_STDERR) as usize];
        self.stderr.file.rewind()?;
        self.stderr.file.read_exact(&mut prefix)?;
        if self.stderr.identity()? != before {
            return Err(io::Error::other("版本探针取消后 stderr 发生变化"));
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "cancelled": true,
            "total_bytes": before.3,
            "captured_bytes": prefix.len(),
            "truncated": before.3 > prefix.len() as u64,
            "prefix_bytes": prefix,
        }))
        .map_err(io::Error::other)?;
        // 只写锁定代次目录的新文件；候选从未继承此句柄，原始字节不进入日志或调用方管道。
        let mut receipt = OutputFile {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .share_mode(0)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(self.directory.join(CANCELLED_STDERR_RECEIPT))?,
            path: self.directory.join(CANCELLED_STDERR_RECEIPT),
        };
        let created = receipt.identity()?;
        if created.3 != 0 {
            return Err(io::Error::other("版本探针取消收据不是新空文件"));
        }
        receipt.file.write_all(&bytes)?;
        receipt.file.flush()?;
        if receipt.identity()? != (created.0, created.1, created.2, bytes.len() as u64)
            || self.stderr.identity()? != before
        {
            return Err(io::Error::other("版本探针取消收据或 stderr 身份变化"));
        }
        for (file, identity) in &self.directories {
            if profile_directory_identity(file)? != *identity {
                return Err(io::Error::other("版本探针取消输出目录身份变化"));
            }
        }
        Ok(())
    }

    pub(super) fn seal(
        mut self,
        check_cancelled: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<ReplayOutput> {
        for (file, identity) in &self.directories {
            if profile_directory_identity(file)? != *identity {
                return Err(io::Error::other("版本探针输出目录身份变化"));
            }
        }
        let mut stdout = OutputFile::create(&self.directory, "worker-output.stdout")?;
        let mut stderr = OutputFile::create(&self.directory, "worker-output.stderr")?;
        let copied_stdout =
            self.stdout
                .copy_to(&mut stdout.file, Some(MAX_STDOUT), check_cancelled);
        // stdout 拒绝或写失败也不能静默吞掉本轮 stderr。
        let copied_stderr = self.stderr.copy_to(&mut stderr.file, None, check_cancelled);
        let cancelled = [copied_stdout.as_ref(), copied_stderr.as_ref()]
            .iter()
            .any(|result| result.is_err_and(|error| error.kind() == io::ErrorKind::Interrupted));
        if cancelled {
            // 取消仍是原失败；仅保留有界诊断，取证错误不能覆盖它或阻断原句柄释放。
            if let Err(error) = self.preserve_cancelled_stderr() {
                let kind = error.kind();
                log::debug!("版本探针取消 stderr 收据未完成: {kind:?}");
            }
        }
        let closed_stdout = self.stdout.close();
        let closed_stderr = self.stderr.close();
        let failure = copied_stdout
            .and(copied_stderr)
            .and(closed_stdout)
            .and(closed_stderr)
            .err();
        Ok(ReplayOutput {
            stdout,
            stderr,
            directories: self.directories,
            failure,
            cancelled,
        })
    }
}

pub(super) struct ReplayOutput {
    stdout: OutputFile,
    stderr: OutputFile,
    // 保留原目录身份到 worker-only 文件已关闭；候选从未得到这些文件句柄。
    directories: Vec<(File, ProfileDirectoryIdentity)>,
    failure: Option<io::Error>,
    cancelled: bool,
}

impl ReplayOutput {
    pub(super) fn replay(
        mut self,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
        check_cancelled: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if self.cancelled {
            let failure = self.failure.take().unwrap();
            let _ = self.stdout.close();
            let _ = self.stderr.close();
            drop(self.directories);
            return Err(failure);
        }
        let out = self
            .stdout
            .copy_to(stdout, Some(MAX_STDOUT), check_cancelled);
        let err = self.stderr.copy_to(stderr, None, check_cancelled);
        let close_out = self.stdout.close();
        let close_err = self.stderr.close();
        drop(self.directories);
        self.failure
            .map_or(Ok(()), Err)
            .and(out)
            .and(err)
            .and(close_out)
            .and(close_err)
    }
}

#[cfg(test)]
#[path = "windows_appcontainer_output_tests.rs"]
mod tests;
