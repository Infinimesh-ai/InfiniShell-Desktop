//! 宿主受审命令的 Windows 派生入口；完整进程树仍由外层严格 Job 监督。

use std::fs::OpenOptions;
use std::io::{self, Read as _, Seek as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsHandle as _;
use std::path::Path;
use std::process::Stdio;

use command::managed::WindowsProcessLease;
use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

use super::{AtomicDirectoryIdentity, ExpectedFileIdentity, Manifest, atomic_windows};

pub(super) fn validate(manifest: &Manifest) -> io::Result<()> {
    #[cfg(target_arch = "x86_64")]
    if manifest.grok_npm_source.is_some() {
        return Err(io::Error::other("受审项目命令不能套用 CLI npm 来源"));
    }
    if !cfg!(target_arch = "x86_64")
        || manifest.environment.is_some()
        || manifest.isolated_home.is_some()
        || manifest.child_image.is_some()
        || manifest
            .expected_files
            .first()
            .is_none_or(|file| file.path() != manifest.executable)
        || manifest
            .atomic_cwd
            .as_ref()
            .is_none_or(|directory| directory.path != manifest.cwd)
    {
        return Err(io::Error::other("Windows 受审命令启动合同无效"));
    }
    let args = manifest
        .arguments
        .iter()
        .map(|arg| arg.to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| io::Error::other("Windows 受审命令参数必须为 UTF-8"))?;
    let name = manifest
        .executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("Windows 受审命令映像名无效"))?;
    let exact = if name.eq_ignore_ascii_case("cargo.exe") {
        manifest.expected_files.len() == 1
            && matches!(args.as_slice(), ["check"] | ["test"] | ["fmt", "--check"])
    } else if name.eq_ignore_ascii_case("python.exe") {
        manifest.expected_files.len() == 1 && args == ["-m", "pytest"]
    } else if name.eq_ignore_ascii_case("node.exe") {
        let root = manifest
            .executable
            .parent()
            .ok_or_else(|| io::Error::other("Node 目录无效"))?;
        let cli = root.join("node_modules/npm/bin/npm-cli.js");
        manifest.expected_files.len() == 4
            && args.len() == 3
            && Path::new(args[0]) == cli
            && args[1] == "run"
            && !args[2].is_empty()
            && args[2].len() <= 64
            && args[2].as_bytes()[0].is_ascii_alphanumeric()
            && args[2].bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.')
            })
            && manifest.expected_files[1].path() == cli
            && manifest.expected_files[2].path() == root.join("node_modules/npm/package.json")
            && manifest.expected_files[3].path() == manifest.cwd.join("package.json")
    } else {
        false
    };
    if !exact {
        return Err(io::Error::other("Windows 受审命令不得扩大固定 argv 集合"));
    }
    Ok(())
}

pub(super) fn execute(manifest: &Manifest) -> io::Result<()> {
    validate(manifest)?;
    let cwd =
        atomic_windows::prepare_directory(manifest.atomic_cwd.as_ref().expect("合同已核对 cwd"))?;
    let mut directory_leases = Vec::new();
    let mut file_leases = Vec::new();
    for expected in &manifest.expected_files {
        let parent = expected
            .path()
            .parent()
            .ok_or_else(|| io::Error::other("命令源缺少父目录"))?;
        directory_leases.push(atomic_windows::prepare_directory(
            &AtomicDirectoryIdentity::capture(parent)?,
        )?);
        let mut options = OpenOptions::new();
        options
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
        let file = options.open(expected.path())?;
        // 文件不共享写入/删除，祖先不共享删除；捕获与 CreateProcess 之间保持同一对象。
        if ExpectedFileIdentity::capture(expected.path())? != *expected {
            return Err(io::Error::other("Windows 受审命令源在审批后改变"));
        }
        file_leases.push(file);
    }
    let program = file_leases.first_mut().expect("合同包含真实执行映像");
    let mut dos = [0_u8; 64];
    program.read_exact(&mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err(io::Error::other("项目命令不是原生 PE"));
    }
    let pe = u32::from_le_bytes(dos[60..64].try_into().expect("固定四字节"));
    if pe > 16 * 1024 * 1024 {
        return Err(io::Error::other("PE 头偏移无效"));
    }
    program.seek(io::SeekFrom::Start(u64::from(pe)))?;
    let mut header = [0_u8; 6];
    program.read_exact(&mut header)?;
    if &header[..4] != b"PE\0\0" || u16::from_le_bytes([header[4], header[5]]) != 0x8664 {
        return Err(io::Error::other("项目命令必须是 Windows x64 PE"));
    }
    let mut command = command::blocking::Command::new(&manifest.executable);
    command
        .args(&manifest.arguments)
        .current_dir(cwd.execution_path())
        .env_remove(super::EXEC_CONTROL_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::from(io::stdout().as_handle().try_clone_to_owned()?))
        .inherit_managed_job();
    cwd.verify_for_spawn()?;
    let suspended = command
        .spawn_suspended_with_child_on_error()
        .map_err(|(error, child)| {
            if let Some(mut child) = child {
                let _ = child.kill();
            }
            error
        })?;
    let image = WindowsProcessLease::from_process_handle(&suspended);
    if image.as_ref().ok().is_none_or(|image| {
        dunce::canonicalize(image.image_path()).ok().as_ref()
            != dunce::canonicalize(&manifest.executable).ok().as_ref()
    }) {
        // 不恢复疑似 IFEO 重定向进程；严格 Job 在 worker 返回时确认整树清理。
        drop(suspended);
        return Err(io::Error::other("Windows 实际派生映像与审批不一致"));
    }
    let mut child = suspended
        .resume_with_child_on_error()
        .map_err(|(error, mut child)| {
            let _ = child.kill();
            error
        })?;
    // 这里有意不把项目代码的后代限制成安装探针映像白名单；UI 明确显示当前账号权限。
    let status = child.wait()?;
    drop(file_leases);
    drop(directory_leases);
    drop(cwd);
    if status.success() {
        Ok(())
    } else {
        std::process::exit(
            status
                .code()
                .ok_or_else(|| io::Error::other("项目命令缺少退出码"))?,
        );
    }
}
