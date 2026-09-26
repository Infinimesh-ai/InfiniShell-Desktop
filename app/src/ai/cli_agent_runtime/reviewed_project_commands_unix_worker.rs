//! 每条受审 Unix 命令独占一个既有监督代；只派生固定绑定，不执行原生 Bash。

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read as _, Seek as _};
use std::os::unix::fs::MetadataExt as _;

use super::super::reviewed_project_commands_unix::UnixCommandBinding;
use super::{ExpectedFileIdentity, Manifest, open_expected_file};

fn invalid() -> io::Error {
    io::Error::other("受审 Unix 项目命令启动合同无效")
}

fn binding(manifest: &Manifest) -> io::Result<UnixCommandBinding> {
    let [encoded] = manifest.arguments.as_slice() else {
        return Err(invalid());
    };
    let text = encoded
        .to_str()
        .filter(|value| value.len() <= 128 * 1024)
        .ok_or_else(invalid)?;
    serde_json::from_str(text).map_err(io::Error::other)
}

pub(super) fn validate(manifest: &Manifest) -> io::Result<()> {
    if !cfg!(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    )) || manifest.environment.is_some()
        || manifest.isolated_home.is_some()
        || manifest.isolated_state_dir.is_some()
    {
        return Err(invalid());
    }
    let command = binding(manifest)?;
    command
        .validate()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let cwd = manifest.atomic_cwd.as_ref().ok_or_else(invalid)?;
    if manifest.executable != command.executable
        || manifest.expected_files != command.files
        || manifest.cwd != command.directory.path
        || cwd.path != command.directory.path
        || cwd.canonical_path != command.directory.path
        || cwd.file_id.volume != command.directory.device
        || cwd.file_id.index != command.directory.inode
    {
        return Err(invalid());
    }
    super::validate_expected_files_contract(&manifest.executable, &manifest.expected_files)
}

fn native(file: &mut File) -> io::Result<()> {
    file.rewind()?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header)?;
    #[cfg(target_os = "linux")]
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != 62
        || !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
    {
        return Err(io::Error::other("受审命令必须是 Linux x64 原生 ELF"));
    }
    #[cfg(target_os = "macos")]
    if !matches!(
        u32::from_be_bytes(header[..4].try_into().expect("固定四字节")),
        0xcffa_edfe | 0xcafe_babe | 0xcafe_babf
    ) {
        return Err(io::Error::other("受审命令必须是 macOS 原生 Mach-O"));
    }
    file.rewind()?;
    Ok(())
}

fn verify_files(expected: &[ExpectedFileIdentity], files: &mut [File]) -> io::Result<()> {
    for (expected, file) in expected.iter().zip(files) {
        if ExpectedFileIdentity::capture_opened(
            &expected.path,
            expected.canonical_path.clone(),
            file,
        )? != *expected
            || ExpectedFileIdentity::capture(&expected.path)? != *expected
        {
            return Err(io::Error::other("受审 Unix 命令源在审批后改变"));
        }
    }
    Ok(())
}

pub(super) fn execute(manifest: &Manifest) -> io::Result<()> {
    validate(manifest)?;
    let command = binding(manifest)?;
    command
        .verify_entry()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let directory = manifest
        .atomic_cwd
        .as_ref()
        .ok_or_else(invalid)?
        .open_verified()?;
    let actual = directory.metadata()?;
    if actual.dev() != command.directory.device || actual.ino() != command.directory.inode {
        return Err(invalid());
    }
    let mut files = command
        .files
        .iter()
        .map(|expected| open_expected_file(&expected.canonical_path))
        .collect::<io::Result<Vec<_>>>()?;
    verify_files(&command.files, &mut files)?;
    let program = files.first_mut().ok_or_else(invalid)?;
    if program.metadata()?.mode() & 0o111 == 0 {
        return Err(invalid());
    }
    native(program)?;
    let image = program.try_clone()?;
    let arguments = command
        .arguments
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    // 继承用户账号运行环境，只去除既有动态加载器注入变量；不声称项目代码受沙箱限制。
    let environment = super::resolved_atomic_environment(manifest)?;
    command::reviewed_unix::execute(
        &command.executable,
        &image,
        &directory,
        OsStr::new(&command.argv0),
        &arguments,
        &environment,
        || {
            command
                .verify_entry()
                .map_err(|error| io::Error::other(error.to_string()))?;
            verify_files(&command.files, &mut files)
        },
    )
}
