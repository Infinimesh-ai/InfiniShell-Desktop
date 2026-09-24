//! Linux 更新程序的密封内存执行；只接受已冻结的原生 ELF 单文件。

use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, RawFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{FileExt as _, MetadataExt as _};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::{ExpectedFileId, ExpectedFileIdentity, open_expected_file, sha256_file};

const MAX_NATIVE_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PROGRAM_HEADERS: u64 = 4_096;
const MAX_DYNAMIC_TABLE_BYTES: u64 = 1024 * 1024;
const REQUIRED_SEALS: libc::c_int =
    libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;

/// 已复制到匿名 memfd 且禁止写入、扩缩和继续增加 seal 的 ELF。
#[derive(Debug)]
pub(super) struct SealedExecutable {
    file: File,
    sha256: String,
    size: u64,
    system_closure: Option<glibc::SystemClosure>,
}

impl SealedExecutable {
    pub(super) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(super) fn size(&self) -> u64 {
        self.size
    }

    #[cfg(test)]
    fn seals(&self) -> io::Result<libc::c_int> {
        fcntl_get_seals(self.file.as_raw_fd())
    }

    #[cfg(test)]
    fn fd_flags(&self) -> io::Result<libc::c_int> {
        let flags = unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_GETFD) };
        if flags < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(flags)
        }
    }

    #[cfg(test)]
    fn try_write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        self.file.write_at(bytes, offset)
    }
}

/// 从预先冻结的文件身份制作不可变执行对象。
///
/// 路径只用于取得一个 no-follow 句柄；后续读取、摘要和执行均绑定该句柄。即使原路径在
/// `open` 后被替换，密封对象也不会重新读取 pathname。
pub(super) fn prepare(expected: &ExpectedFileIdentity) -> io::Result<SealedExecutable> {
    if !expected.canonical_path.is_absolute()
        || expected.sha256.len() != 64
        || !expected.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || expected.size == 0
        || expected.size > MAX_NATIVE_EXECUTABLE_BYTES
    {
        return Err(io::Error::other(
            "managed_process.linux_atomic_identity_invalid",
        ));
    }
    let mut source = open_expected_file(&expected.canonical_path)?;
    let before = source.metadata()?;
    if !before.is_file()
        || before.len() != expected.size
        || before.mode() & 0o111 == 0
        || expected.file_id
            != Some(ExpectedFileId {
                volume: before.dev(),
                index: before.ino(),
            })
    {
        return Err(io::Error::other(
            "managed_process.linux_atomic_source_changed",
        ));
    }

    let name = CString::new("infinishell-cli-update").expect("固定 memfd 名称没有 NUL");
    let fd =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut snapshot = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::fchmod(snapshot.as_raw_fd(), 0o500) } != 0 {
        return Err(io::Error::last_os_error());
    }

    source.rewind()?;
    let mut digest = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .filter(|size| *size <= MAX_NATIVE_EXECUTABLE_BYTES)
            .ok_or_else(|| io::Error::other("managed_process.linux_atomic_source_too_large"))?;
        digest.update(&buffer[..read]);
        snapshot.write_all(&buffer[..read])?;
    }
    let sha256 = format!("{:x}", digest.finalize());
    let after = source.metadata()?;
    if copied != expected.size
        || sha256 != expected.sha256
        || after.len() != before.len()
        || after.dev() != before.dev()
        || after.ino() != before.ino()
    {
        return Err(io::Error::other(
            "managed_process.linux_atomic_source_changed",
        ));
    }
    let system_closure = verify_elf(&snapshot, copied)?;
    snapshot.sync_all()?;
    if unsafe { libc::fcntl(snapshot.as_raw_fd(), libc::F_ADD_SEALS, REQUIRED_SEALS) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if fcntl_get_seals(snapshot.as_raw_fd())? & REQUIRED_SEALS != REQUIRED_SEALS {
        return Err(io::Error::other(
            "managed_process.linux_atomic_seal_incomplete",
        ));
    }
    if snapshot.metadata()?.len() != copied {
        return Err(io::Error::other(
            "managed_process.linux_atomic_snapshot_changed",
        ));
    }
    Ok(SealedExecutable {
        file: snapshot,
        sha256,
        size: copied,
        system_closure,
    })
}

/// 私有发布目录中的静态 ELF；保留 current_exe 的安装布局，并通过已打开 fd 执行。
#[derive(Debug)]
pub(super) struct LayoutExecutable {
    file: File,
    directory: File,
    path: PathBuf,
    identity: ExpectedFileId,
    sha256: String,
    size: u64,
}

impl LayoutExecutable {
    pub(super) fn verify_for_execution(&self) -> io::Result<()> {
        let directory = self.directory.metadata()?;
        let before = self.file.metadata()?;
        let path = fs::symlink_metadata(&self.path)?;
        if directory.uid() != unsafe { libc::geteuid() }
            || directory.mode() & 0o7777 != 0o500
            || !before.is_file()
            || before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o7777 != 0o500
            || before.nlink() != 1
            || before.len() != self.size
            || before.dev() != self.identity.volume
            || before.ino() != self.identity.index
            || path.file_type().is_symlink()
            || path.dev() != before.dev()
            || path.ino() != before.ino()
        {
            return Err(io::Error::other(
                "managed_process.linux_layout_snapshot_changed",
            ));
        }
        let mut file = self.file.try_clone()?;
        if sha256_file(&mut file)? != self.sha256 {
            return Err(io::Error::other(
                "managed_process.linux_layout_snapshot_changed",
            ));
        }
        Ok(())
    }
}

/// 只有需要保留 standalone 安装布局的官方更新器使用此分支；普通 ELF 仍走 sealed memfd。
/// 先密封并核验源字节，再发布到受控私有目录；新代次不能覆盖旧快照。
pub(super) fn prepare_layout_snapshot(
    releases: &Path,
    generation: uuid::Uuid,
    binding_digest: &str,
    expected: &ExpectedFileIdentity,
) -> io::Result<LayoutExecutable> {
    if binding_digest.len() != 64 || !binding_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::other(
            "managed_process.linux_layout_binding_invalid",
        ));
    }
    let mut sealed = prepare(expected)?;
    if sealed.system_closure.is_some() {
        return Err(io::Error::other(
            "managed_process.linux_atomic_dependency_closure_unbound",
        ));
    }
    let releases_directory = open_secure_directory(releases)?;
    let root_name = c"cli-agent-executable-snapshots";
    if unsafe { libc::mkdirat(releases_directory.as_raw_fd(), root_name.as_ptr(), 0o700) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    let root = open_at(
        releases_directory.as_raw_fd(),
        root_name,
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
    )?;
    let metadata = root.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7777 != 0o700 {
        return Err(io::Error::other(
            "managed_process.linux_layout_root_not_private",
        ));
    }
    let generation_name = CString::new(generation.to_string()).expect("UUID 不含 NUL");
    if unsafe { libc::mkdirat(root.as_raw_fd(), generation_name.as_ptr(), 0o700) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let directory = open_at(
        root.as_raw_fd(),
        &generation_name,
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
    )?;
    let mut output = open_at(
        directory.as_raw_fd(),
        c"program",
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )?;
    let metadata = output.metadata()?;
    let directory_metadata = directory.metadata()?;
    let summary = serde_json::to_vec(&serde_json::json!({
        "schema": 1,
        "generation": generation,
        "binding_digest": binding_digest,
        "directory_file_id": {"volume": directory_metadata.dev(), "index": directory_metadata.ino()},
        "source_file_id": expected.file_id,
        "snapshot_file_id": {"volume": metadata.dev(), "index": metadata.ino()},
        "size": expected.size,
        "sha256": expected.sha256,
        "program_relative_path": "program"
    }))
    .map_err(io::Error::other)?;
    let mut receipt = open_at(
        directory.as_raw_fd(),
        c"snapshot.json",
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o400,
    )?;
    receipt.write_all(&summary)?;
    receipt.sync_all()?;
    // 先持久化归属再复制大文件；中途失败仍可在整树退出后回收本代已绑定的部分副本。
    directory.sync_all()?;
    root.sync_all()?;
    releases_directory.sync_all()?;
    sealed.file.rewind()?;
    if io::copy(&mut sealed.file, &mut output)? != expected.size {
        return Err(io::Error::other(
            "managed_process.linux_layout_copy_incomplete",
        ));
    }
    if unsafe { libc::fchmod(output.as_raw_fd(), 0o500) } != 0 {
        return Err(io::Error::last_os_error());
    }
    output.sync_all()?;
    drop(output);
    let file = open_at(directory.as_raw_fd(), c"program", libc::O_RDONLY, 0)?;
    if unsafe { libc::fchmod(directory.as_raw_fd(), 0o500) } != 0 {
        return Err(io::Error::last_os_error());
    }
    directory.sync_all()?;
    root.sync_all()?;
    releases_directory.sync_all()?;
    let snapshot = LayoutExecutable {
        file,
        directory,
        path: releases
            .join("cli-agent-executable-snapshots")
            .join(generation.to_string())
            .join("program"),
        identity: ExpectedFileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        },
        sha256: expected.sha256.clone(),
        size: expected.size,
    };
    snapshot.verify_for_execution()?;
    Ok(snapshot)
}

/// 调用者必须先验证本代整棵进程树退出；仅删除与 manifest 及启动摘要绑定的副本。
/// 已删除的代次可幂等重试；不扫描或回收其他代次，更不删除整个快照根。
pub(super) fn cleanup_layout_snapshot(
    releases: &Path,
    generation: uuid::Uuid,
    binding_digest: &str,
    expected: &ExpectedFileIdentity,
) -> io::Result<()> {
    let releases_directory = open_secure_directory(releases)?;
    let root = match open_at(
        releases_directory.as_raw_fd(),
        c"cli-agent-executable-snapshots",
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
    ) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let root_metadata = root.metadata()?;
    if root_metadata.uid() != unsafe { libc::geteuid() } || root_metadata.mode() & 0o7777 != 0o700 {
        return Err(io::Error::other(
            "managed_process.linux_layout_root_not_private",
        ));
    }
    let generation_name = CString::new(generation.to_string()).expect("UUID 不含 NUL");
    let directory = match open_at(
        root.as_raw_fd(),
        &generation_name,
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
    ) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let metadata = directory.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || !matches!(metadata.mode() & 0o7777, 0o500 | 0o700)
    {
        return Err(io::Error::other(
            "managed_process.linux_layout_cleanup_identity",
        ));
    }
    // read_dir 绑定已打开目录；未知文件一律保留并拒绝清理。
    let names = fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    if names
        .iter()
        .any(|name| name != "program" && name != "snapshot.json")
    {
        return Err(io::Error::other(
            "managed_process.linux_layout_cleanup_unknown_file",
        ));
    }
    if !names.is_empty() {
        let receipt = open_at(
            directory.as_raw_fd(),
            c"snapshot.json",
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        )?;
        let info = receipt.metadata()?;
        if !info.is_file()
            || info.uid() != unsafe { libc::geteuid() }
            || info.nlink() != 1
            || info.mode() & 0o7777 != 0o400
            || info.len() > 16 * 1024
        {
            return Err(io::Error::other(
                "managed_process.linux_layout_cleanup_identity",
            ));
        }
        let summary: serde_json::Value =
            serde_json::from_reader(receipt).map_err(io::Error::other)?;
        if summary["schema"] != 1
            || summary["generation"] != generation.to_string()
            || summary["binding_digest"] != binding_digest
            || summary["source_file_id"]
                != serde_json::to_value(expected.file_id).map_err(io::Error::other)?
            || summary["directory_file_id"]
                != serde_json::json!({"volume": metadata.dev(), "index": metadata.ino()})
            || summary["size"] != expected.size
            || summary["sha256"] != expected.sha256
            || summary["program_relative_path"] != "program"
        {
            return Err(io::Error::other(
                "managed_process.linux_layout_cleanup_binding",
            ));
        }
        match open_at(
            directory.as_raw_fd(),
            c"program",
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        ) {
            Ok(program) => {
                let info = program.metadata()?;
                if !info.is_file()
                    || info.uid() != unsafe { libc::geteuid() }
                    || info.nlink() != 1
                    || !matches!(info.mode() & 0o7777, 0o500 | 0o600)
                    || info.len() > expected.size
                    || summary["snapshot_file_id"]
                        != serde_json::json!({"volume": info.dev(), "index": info.ino()})
                {
                    return Err(io::Error::other(
                        "managed_process.linux_layout_cleanup_identity",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // 先删程序，最后删归属记录；崩溃后仍可按相同代次恢复，空目录只执行 rmdir。
    for name in [c"program", c"snapshot.json"] {
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error);
            }
        }
    }
    directory.sync_all()?;
    if unsafe {
        libc::unlinkat(
            root.as_raw_fd(),
            generation_name.as_ptr(),
            libc::AT_REMOVEDIR,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    root.sync_all()
}

fn open_at(parent: RawFd, name: &CStr, flags: libc::c_int, mode: libc::mode_t) -> io::Result<File> {
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn open_secure_directory(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::other(
            "managed_process.linux_layout_path_not_absolute",
        ));
    }
    let mut directory = open_at(libc::AT_FDCWD, c"/", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    for component in path.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => CString::new(name.as_bytes()).map_err(io::Error::other)?,
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::other(
                    "managed_process.linux_layout_path_not_normalized",
                ));
            }
        };
        directory = open_at(
            directory.as_raw_fd(),
            &name,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        let metadata = directory.metadata()?;
        if metadata.mode() & 0o022 != 0
            || metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::other(
                "managed_process.linux_layout_ancestor_unsafe",
            ));
        }
    }
    Ok(directory)
}

pub(super) fn execute_layout(
    executable: &LayoutExecutable,
    argv0: &Path,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
) -> io::Error {
    if let Err(error) = executable.verify_for_execution() {
        return error;
    }
    execute_file(&executable.file, argv0, arguments, environment)
}

fn verify_elf(file: &File, size: u64) -> io::Result<Option<glibc::SystemClosure>> {
    if size < 64 {
        return Err(io::Error::other("managed_process.linux_atomic_not_elf"));
    }
    let mut header = [0_u8; 64];
    file.read_exact_at(&mut header, 0)?;
    let class = header[4];
    let endian = header[5];
    let class_valid = matches!(class, 1 | 2);
    let endian_valid = matches!(endian, 1 | 2);
    let version_valid = header[6] == 1;
    let executable_type = read_u16(&header[16..18], endian)?;
    if header[..4] != *b"\x7fELF"
        || !class_valid
        || !endian_valid
        || !version_valid
        || !matches!(executable_type, 2 | 3)
    {
        return Err(io::Error::other("managed_process.linux_atomic_not_elf"));
    }

    let (program_offset, entry_size, entry_count) = match class {
        1 => (
            u64::from(read_u32(&header[28..32], endian)?),
            u64::from(read_u16(&header[42..44], endian)?),
            u64::from(read_u16(&header[44..46], endian)?),
        ),
        2 => (
            read_u64(&header[32..40], endian)?,
            u64::from(read_u16(&header[54..56], endian)?),
            u64::from(read_u16(&header[56..58], endian)?),
        ),
        _ => unreachable!("ELF class 已在上方穷尽验证"),
    };
    let minimum_entry_size = if class == 1 { 32 } else { 56 };
    let table_size = entry_size
        .checked_mul(entry_count)
        .ok_or_else(|| io::Error::other("managed_process.linux_atomic_program_table_invalid"))?;
    let table_end = program_offset
        .checked_add(table_size)
        .ok_or_else(|| io::Error::other("managed_process.linux_atomic_program_table_invalid"))?;
    if entry_count == 0
        || entry_count > MAX_PROGRAM_HEADERS
        || entry_size != minimum_entry_size
        || table_end > size
        || table_size > usize::MAX as u64
    {
        return Err(io::Error::other(
            "managed_process.linux_atomic_program_table_invalid",
        ));
    }

    let mut table = vec![0_u8; table_size as usize];
    file.read_exact_at(&mut table, program_offset)?;
    if table
        .chunks_exact(entry_size as usize)
        .any(|entry| read_u32(&entry[..4], endian).is_ok_and(|segment| segment == 3))
    {
        return glibc::prepare(file, size).map(Some);
    }
    for entry in table.chunks_exact(entry_size as usize) {
        let segment_type = read_u32(&entry[..4], endian)?;
        if segment_type == 3 {
            return Err(io::Error::other(
                "managed_process.linux_atomic_dependency_closure_unbound",
            ));
        }
        if segment_type == 2 {
            verify_static_dynamic_segment(file, size, class, endian, entry)?;
        }
        if segment_type == 0x6474_e551 {
            let flags = if class == 1 {
                read_u32(&entry[24..28], endian)?
            } else {
                read_u32(&entry[4..8], endian)?
            };
            if flags & 1 != 0 {
                return Err(io::Error::other(
                    "managed_process.linux_atomic_executable_stack",
                ));
            }
        }
    }
    Ok(None)
}

fn verify_static_dynamic_segment(
    file: &File,
    file_size: u64,
    class: u8,
    endian: u8,
    program_header: &[u8],
) -> io::Result<()> {
    let (offset, size, entry_size) = if class == 1 {
        (
            u64::from(read_u32(&program_header[4..8], endian)?),
            u64::from(read_u32(&program_header[16..20], endian)?),
            8_u64,
        )
    } else {
        (
            read_u64(&program_header[8..16], endian)?,
            read_u64(&program_header[32..40], endian)?,
            16_u64,
        )
    };
    if size == 0
        || size % entry_size != 0
        || offset.checked_add(size).is_none_or(|end| end > file_size)
        || size > MAX_DYNAMIC_TABLE_BYTES
        || size > usize::MAX as u64
    {
        return Err(io::Error::other(
            "managed_process.linux_atomic_dynamic_table_invalid",
        ));
    }
    let mut table = vec![0_u8; size as usize];
    file.read_exact_at(&mut table, offset)?;
    let mut terminated = false;
    for entry in table.chunks_exact(entry_size as usize) {
        let tag = if class == 1 {
            u64::from(read_u32(&entry[..4], endian)?)
        } else {
            read_u64(&entry[..8], endian)?
        };
        if tag == 0 {
            terminated = true;
            break;
        }
        // 静态 PIE 可以带 PT_DYNAMIC 做自重定位；只有会请求外部对象的 tag 才越出
        // 当前单文件冻结闭包。运行时显式 dlopen 仍须由具体 CLI 的真实验收覆盖。
        if matches!(
            tag,
            1 | 0x6fff_fefb | 0x6fff_fefc | 0x7fff_fffd | 0x7fff_ffff
        ) {
            return Err(io::Error::other(
                "managed_process.linux_atomic_dependency_closure_unbound",
            ));
        }
    }
    if !terminated {
        return Err(io::Error::other(
            "managed_process.linux_atomic_dynamic_table_invalid",
        ));
    }
    Ok(())
}

fn read_u16(bytes: &[u8], endian: u8) -> io::Result<u16> {
    match endian {
        1 => Ok(u16::from_le_bytes(bytes.try_into().expect("固定 u16 切片"))),
        2 => Ok(u16::from_be_bytes(bytes.try_into().expect("固定 u16 切片"))),
        _ => Err(io::Error::other("managed_process.linux_atomic_not_elf")),
    }
}

fn read_u32(bytes: &[u8], endian: u8) -> io::Result<u32> {
    match endian {
        1 => Ok(u32::from_le_bytes(bytes.try_into().expect("固定 u32 切片"))),
        2 => Ok(u32::from_be_bytes(bytes.try_into().expect("固定 u32 切片"))),
        _ => Err(io::Error::other("managed_process.linux_atomic_not_elf")),
    }
}

fn read_u64(bytes: &[u8], endian: u8) -> io::Result<u64> {
    match endian {
        1 => Ok(u64::from_le_bytes(bytes.try_into().expect("固定 u64 切片"))),
        2 => Ok(u64::from_be_bytes(bytes.try_into().expect("固定 u64 切片"))),
        _ => Err(io::Error::other("managed_process.linux_atomic_not_elf")),
    }
}

fn fcntl_get_seals(fd: RawFd) -> io::Result<libc::c_int> {
    let seals = unsafe { libc::fcntl(fd, libc::F_GET_SEALS) };
    if seals < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(seals)
    }
}

/// 用 `execveat(AT_EMPTY_PATH)` 执行密封对象；任何错误都直接返回，不回退 pathname。
pub(super) fn execute(
    executable: &SealedExecutable,
    argv0: &Path,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
) -> io::Error {
    if let Some(closure) = &executable.system_closure {
        if let Err(error) = closure.verify() {
            return error;
        }
    }
    execute_file(&executable.file, argv0, arguments, environment)
}

fn execute_file(
    file: &File,
    argv0: &Path,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
) -> io::Error {
    let argv = match cstring_vector(
        std::iter::once(argv0.as_os_str()).chain(arguments.iter().map(OsString::as_os_str)),
        "managed_process.linux_atomic_argument_invalid",
    ) {
        Ok(argv) => argv,
        Err(error) => return error,
    };
    let env = match environment
        .iter()
        .map(|(name, value)| {
            if name.as_bytes().contains(&b'=') {
                return Err(io::Error::other(
                    "managed_process.linux_atomic_environment_invalid",
                ));
            }
            let mut bytes = Vec::with_capacity(name.as_bytes().len() + value.as_bytes().len() + 1);
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(b'=');
            bytes.extend_from_slice(value.as_bytes());
            CString::new(bytes)
                .map_err(|_| io::Error::other("managed_process.linux_atomic_environment_invalid"))
        })
        .collect::<io::Result<Vec<_>>>()
    {
        Ok(env) => env,
        Err(error) => return error,
    };
    let argv_ptrs = argv
        .iter()
        .map(|value| value.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect::<Vec<_>>();
    let env_ptrs = env
        .iter()
        .map(|value| value.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect::<Vec<_>>();
    let empty = c"";
    // libc 在 musl 目标不公开 `execveat` 包装，但仓库正式 Linux 产物使用 musl；
    // 直接调用同一内核系统调用，仍然禁止任何 pathname 回退。
    let result = unsafe {
        libc::syscall(
            libc::SYS_execveat,
            file.as_raw_fd(),
            empty.as_ptr(),
            argv_ptrs.as_ptr(),
            env_ptrs.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    };
    debug_assert_eq!(result, -1);
    io::Error::last_os_error()
}

fn cstring_vector<'a>(
    values: impl Iterator<Item = &'a OsStr>,
    error: &'static str,
) -> io::Result<Vec<CString>> {
    values
        .map(|value| CString::new(value.as_bytes()).map_err(|_| io::Error::other(error)))
        .collect()
}

#[cfg(test)]
#[path = "managed_process_atomic_linux_tests.rs"]
mod tests;

#[path = "managed_process_atomic_linux_glibc.rs"]
mod glibc;
