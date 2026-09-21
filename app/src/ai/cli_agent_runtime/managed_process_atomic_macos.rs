//! macOS 原生单文件更新程序的私有、不可变 generation 快照。
//!
//! 本模块只接受已经由来源层冻结身份的单个 Mach-O 文件。脚本、解释器入口、npm、
//! Homebrew、动态库或其他依赖闭包不属于这里的安全边界，调用方不得把它们降级为单文件。

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, RawFd};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};

use command::blocking::Command;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::{ExpectedFileId, ExpectedFileIdentity};

const SNAPSHOT_ROOT: &str = "cli-agent-executable-snapshots";
const PROGRAM_FILE: &str = "program";
const SUMMARY_FILE: &str = "snapshot.json";
const SUMMARY_VERSION: u32 = 1;
const MAX_SUMMARY_BYTES: usize = 64 * 1024;
const MAX_DEPENDENCY_REPORT_BYTES: usize = 1024 * 1024;
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PUBLISHED_DIRECTORY_MODE: u32 = 0o500;
const PUBLISHED_PROGRAM_MODE: u32 = 0o500;
const PUBLISHED_SUMMARY_MODE: u32 = 0o400;
const CODESIGN_REQUIREMENT: &str = "codesign --verify --strict --all-architectures";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PersistentFileId {
    pub(super) volume: u64,
    pub(super) index: u64,
}

impl From<ExpectedFileId> for PersistentFileId {
    fn from(value: ExpectedFileId) -> Self {
        Self {
            volume: value.volume,
            index: value.index,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MacosAtomicExecutionSummary {
    pub(super) version: u32,
    pub(super) generation: Uuid,
    pub(super) binding_digest: String,
    pub(super) source_path: PathBuf,
    pub(super) source_file_id: PersistentFileId,
    pub(super) snapshot_file_id: PersistentFileId,
    pub(super) program_relative_path: PathBuf,
    pub(super) size: u64,
    pub(super) source_mode: u32,
    pub(super) snapshot_mode: u32,
    pub(super) sha256: String,
    pub(super) extended_attributes_sha256: String,
    pub(super) signature_requirement: String,
}

/// 已经无覆盖发布的只读执行对象；持久化时同时保存 summary 与其字节摘要。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PublishedMacosExecutable {
    execution_path: PathBuf,
    summary: MacosAtomicExecutionSummary,
    summary_sha256: String,
}

impl PublishedMacosExecutable {
    pub(super) fn execution_path(&self) -> &Path {
        &self.execution_path
    }

    pub(super) fn summary(&self) -> &MacosAtomicExecutionSummary {
        &self.summary
    }

    pub(super) fn summary_sha256(&self) -> &str {
        &self.summary_sha256
    }

    /// worker 在派生前调用；任何清单、inode、内容、mode、xattr 或签名变化都拒绝执行。
    pub(super) fn verify_for_execution(&self) -> io::Result<()> {
        let generation_directory = self
            .execution_path
            .parent()
            .ok_or_else(|| error("managed_process.atomic_macos_execution_path_invalid"))?;
        validate_private_generation_directory(generation_directory)?;

        let summary_path = generation_directory.join(SUMMARY_FILE);
        let summary_bytes = read_private_file(
            &summary_path,
            PUBLISHED_SUMMARY_MODE,
            MAX_SUMMARY_BYTES as u64,
        )?;
        if sha256_bytes(&summary_bytes) != self.summary_sha256 {
            return Err(error("managed_process.atomic_macos_summary_changed"));
        }
        let persisted: MacosAtomicExecutionSummary =
            serde_json::from_slice(&summary_bytes).map_err(io::Error::other)?;
        if persisted != self.summary
            || persisted.version != SUMMARY_VERSION
            || generation_directory.file_name()
                != Some(std::ffi::OsStr::new(&persisted.generation.to_string()))
            || persisted.program_relative_path != Path::new(PROGRAM_FILE)
            || persisted.signature_requirement != CODESIGN_REQUIREMENT
        {
            return Err(error("managed_process.atomic_macos_summary_mismatch"));
        }

        let mut program = open_plain_file(&self.execution_path, false)?;
        let before = inspect_open_file(&program)?;
        validate_published_program(&before, &persisted)?;
        let sha256 = sha256_file(&mut program)?;
        let xattrs = extended_attributes_sha256(&program)?;
        if sha256 != persisted.sha256 || xattrs != persisted.extended_attributes_sha256 {
            return Err(error("managed_process.atomic_macos_snapshot_changed"));
        }
        verify_complete_macho_signature(&self.execution_path)?;
        verify_system_dependency_closure(&self.execution_path)?;
        let after = inspect_open_file(&program)?;
        if after != before || inspect_path_file(&self.execution_path)? != after {
            return Err(error("managed_process.atomic_macos_snapshot_raced"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InspectedFile {
    id: PersistentFileId,
    size: u64,
    mode: u32,
    links: u64,
    owner: u32,
}

/// 从同一个已打开、已核验的源句柄复制；路径只用于确认来源没有被并发替换。
pub(super) fn prepare_native_executable(
    state_dir: &Path,
    generation: Uuid,
    binding_digest: &str,
    expected: &ExpectedFileIdentity,
) -> io::Result<PublishedMacosExecutable> {
    prepare_native_executable_inner(
        state_dir,
        generation,
        binding_digest,
        expected,
        || Ok(()),
        || Ok(()),
    )
}

fn prepare_native_executable_inner(
    state_dir: &Path,
    generation: Uuid,
    binding_digest: &str,
    expected: &ExpectedFileIdentity,
    after_source_verified: impl FnOnce() -> io::Result<()>,
    after_published: impl FnOnce() -> io::Result<()>,
) -> io::Result<PublishedMacosExecutable> {
    validate_digest(binding_digest)?;
    validate_source_path(expected)?;
    validate_secure_ancestors(&expected.path, false)?;
    validate_secure_ancestors(state_dir, true)?;

    let mut source = open_plain_file(&expected.path, false)?;
    let source_before = inspect_open_file(&source)?;
    validate_source_identity(&mut source, &source_before, expected)?;
    let source_sha256 = sha256_file(&mut source)?;
    let source_xattrs = extended_attributes_sha256(&source)?;
    if source_sha256 != expected.sha256 {
        return Err(error("managed_process.atomic_macos_source_digest_changed"));
    }
    require_macho(&mut source)?;
    if inspect_path_file(&expected.path)? != source_before {
        return Err(error("managed_process.atomic_macos_source_path_changed"));
    }

    after_source_verified()?;

    let (root_path, root) = open_snapshot_root(state_dir)?;
    let generation_name = generation.to_string();
    let scratch_name = format!(".scratch-{generation}-{}", Uuid::new_v4());
    create_private_directory_at(root.as_raw_fd(), &scratch_name)?;
    let scratch_path = root_path.join(&scratch_name);
    let scratch = match open_directory_at(root.as_raw_fd(), &scratch_name) {
        Ok(scratch) => scratch,
        Err(error) => {
            let _ = fs::remove_dir(&scratch_path);
            return Err(error);
        }
    };
    let scratch_id = inspect_open_file(&scratch)?.id;
    let mut published = false;
    let result = (|| {
        let program_path = scratch_path.join(PROGRAM_FILE);
        let mut snapshot = create_file_at(scratch.as_raw_fd(), PROGRAM_FILE, 0o600, libc::O_RDWR)?;
        copy_open_file(&mut source, &mut snapshot)?;
        snapshot.sync_all()?;

        let source_after_copy = inspect_open_file(&source)?;
        if source_after_copy != source_before || inspect_path_file(&expected.path)? != source_before
        {
            return Err(error("managed_process.atomic_macos_source_raced"));
        }
        let snapshot_before = inspect_open_file(&snapshot)?;
        if snapshot_before.links != 1 || snapshot_before.id == source_before.id {
            return Err(error(
                "managed_process.atomic_macos_snapshot_identity_invalid",
            ));
        }
        let snapshot_sha256 = sha256_file(&mut snapshot)?;
        let snapshot_xattrs = extended_attributes_sha256(&snapshot)?;
        if snapshot_sha256 != source_sha256 || snapshot_xattrs != source_xattrs {
            return Err(error("managed_process.atomic_macos_snapshot_copy_mismatch"));
        }

        snapshot.set_permissions(fs::Permissions::from_mode(PUBLISHED_PROGRAM_MODE))?;
        snapshot.sync_all()?;
        let snapshot_published = inspect_open_file(&snapshot)?;
        if snapshot_published.mode != PUBLISHED_PROGRAM_MODE
            || snapshot_published.id != snapshot_before.id
            || snapshot_published.size != source_before.size
        {
            return Err(error("managed_process.atomic_macos_snapshot_mode_invalid"));
        }
        verify_complete_macho_signature(&program_path)?;
        verify_system_dependency_closure(&program_path)?;
        if inspect_open_file(&snapshot)? != snapshot_published
            || inspect_path_file(&program_path)? != snapshot_published
        {
            return Err(error("managed_process.atomic_macos_signature_check_raced"));
        }

        let summary = MacosAtomicExecutionSummary {
            version: SUMMARY_VERSION,
            generation,
            binding_digest: binding_digest.to_owned(),
            source_path: expected.canonical_path.clone(),
            source_file_id: source_before.id,
            snapshot_file_id: snapshot_published.id,
            program_relative_path: PathBuf::from(PROGRAM_FILE),
            size: source_before.size,
            source_mode: source_before.mode,
            snapshot_mode: snapshot_published.mode,
            sha256: source_sha256.clone(),
            extended_attributes_sha256: source_xattrs.clone(),
            signature_requirement: CODESIGN_REQUIREMENT.to_owned(),
        };
        let summary_bytes = serde_json::to_vec(&summary).map_err(io::Error::other)?;
        if summary_bytes.len() > MAX_SUMMARY_BYTES {
            return Err(error("managed_process.atomic_macos_summary_too_large"));
        }
        let mut summary_file = create_file_at(
            scratch.as_raw_fd(),
            SUMMARY_FILE,
            PUBLISHED_SUMMARY_MODE,
            libc::O_WRONLY,
        )?;
        summary_file.write_all(&summary_bytes)?;
        summary_file.sync_all()?;
        if inspect_open_file(&summary_file)?.mode != PUBLISHED_SUMMARY_MODE {
            return Err(error("managed_process.atomic_macos_summary_mode_invalid"));
        }

        set_directory_mode(&scratch, PUBLISHED_DIRECTORY_MODE)?;
        scratch.sync_all()?;
        publish_noclobber(root.as_raw_fd(), &scratch_name, &generation_name)?;
        published = true;
        root.sync_all()?;
        after_published()?;

        let generation_path = root_path.join(&generation_name);
        let execution_path = generation_path.join(PROGRAM_FILE);
        let published = PublishedMacosExecutable {
            execution_path,
            summary,
            summary_sha256: sha256_bytes(&summary_bytes),
        };
        published.verify_for_execution()?;
        Ok(published)
    })();
    if result.is_err() {
        // 只按仍打开的 inode 撤回本次对象，绝不递归删除同名但身份不同的并发内容。
        let _ = set_directory_mode(&scratch, PRIVATE_DIRECTORY_MODE);
        if published
            && inspect_path_file(&root_path.join(&generation_name))
                .is_ok_and(|information| information.id == scratch_id)
            && publish_noclobber(root.as_raw_fd(), &generation_name, &scratch_name).is_ok()
        {
            published = false;
            let _ = root.sync_all();
        }
        if !published
            && inspect_path_file(&scratch_path)
                .is_ok_and(|information| information.id == scratch_id)
        {
            let _ = set_path_mode(&scratch_path, PRIVATE_DIRECTORY_MODE);
            let _ = fs::remove_dir_all(&scratch_path);
        }
    }
    result
}

fn validate_digest(digest: &str) -> io::Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(error("managed_process.atomic_macos_binding_digest_invalid"));
    }
    Ok(())
}

fn validate_source_path(expected: &ExpectedFileIdentity) -> io::Result<()> {
    if !expected.path.is_absolute()
        || expected.path != expected.canonical_path
        || expected.file_id.is_none()
        || expected.sha256.len() != 64
        || !expected.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(error(
            "managed_process.atomic_macos_source_identity_invalid",
        ));
    }
    Ok(())
}

fn validate_source_identity(
    source: &mut File,
    inspected: &InspectedFile,
    expected: &ExpectedFileIdentity,
) -> io::Result<()> {
    let expected_id = expected
        .file_id
        .ok_or_else(|| error("managed_process.atomic_macos_source_file_id_missing"))?;
    if inspected.id != expected_id.into()
        || inspected.size != expected.size
        || inspected.links != 1
        || inspected.owner != unsafe { libc::geteuid() } && inspected.owner != 0
        || inspected.mode & 0o111 == 0
        || inspected.mode & 0o7022 != 0
    {
        return Err(error("managed_process.atomic_macos_source_file_unsafe"));
    }
    let digest = sha256_file(source)?;
    if digest != expected.sha256 {
        return Err(error(
            "managed_process.atomic_macos_source_identity_changed",
        ));
    }
    Ok(())
}

fn validate_published_program(
    inspected: &InspectedFile,
    summary: &MacosAtomicExecutionSummary,
) -> io::Result<()> {
    if inspected.id != summary.snapshot_file_id
        || inspected.size != summary.size
        || inspected.mode != summary.snapshot_mode
        || inspected.mode != PUBLISHED_PROGRAM_MODE
        || inspected.links != 1
        || inspected.owner != unsafe { libc::geteuid() }
    {
        return Err(error(
            "managed_process.atomic_macos_snapshot_identity_changed",
        ));
    }
    Ok(())
}

fn require_macho(file: &mut File) -> io::Result<()> {
    file.rewind()?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)?;
    file.rewind()?;
    if !matches!(
        magic,
        [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xce]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbf, 0xba, 0xfe, 0xca]
    ) {
        return Err(error("managed_process.atomic_macos_program_not_macho"));
    }
    Ok(())
}

fn verify_complete_macho_signature(path: &Path) -> io::Result<()> {
    let status = Command::new("/usr/bin/codesign")
        .args([
            std::ffi::OsStr::new("--verify"),
            std::ffi::OsStr::new("--strict"),
            std::ffi::OsStr::new("--all-architectures"),
            std::ffi::OsStr::new("--"),
            path.as_os_str(),
        ])
        .status()?;
    if !status.success() {
        return Err(error("managed_process.atomic_macos_signature_invalid"));
    }
    Ok(())
}

fn verify_system_dependency_closure(path: &Path) -> io::Result<()> {
    let mut libraries = Command::new("/usr/bin/otool");
    libraries.args([std::ffi::OsStr::new("-L"), path.as_os_str()]);
    let libraries = libraries.output()?;
    if !libraries.status.success()
        || libraries.stdout.len() > MAX_DEPENDENCY_REPORT_BYTES
        || !libraries.stderr.is_empty()
    {
        return Err(error(
            "managed_process.atomic_macos_dependency_report_invalid",
        ));
    }
    let libraries = std::str::from_utf8(&libraries.stdout).map_err(io::Error::other)?;
    let mut lines = libraries.lines();
    let _program = lines
        .next()
        .ok_or_else(|| error("managed_process.atomic_macos_dependency_report_invalid"))?;
    for line in lines {
        let dependency = line
            .trim()
            .split_ascii_whitespace()
            .next()
            .ok_or_else(|| error("managed_process.atomic_macos_dependency_report_invalid"))?;
        if !dependency.starts_with("/usr/lib/")
            && !dependency.starts_with("/System/Library/Frameworks/")
            && !dependency.starts_with("/System/Library/PrivateFrameworks/")
        {
            return Err(error(
                "managed_process.atomic_macos_dependency_closure_unbound",
            ));
        }
    }

    let mut commands = Command::new("/usr/bin/otool");
    commands.args([std::ffi::OsStr::new("-l"), path.as_os_str()]);
    let commands = commands.output()?;
    if !commands.status.success()
        || commands.stdout.len() > MAX_DEPENDENCY_REPORT_BYTES
        || !commands.stderr.is_empty()
    {
        return Err(error(
            "managed_process.atomic_macos_dependency_report_invalid",
        ));
    }
    let commands = std::str::from_utf8(&commands.stdout).map_err(io::Error::other)?;
    if commands.lines().any(|line| line.trim() == "cmd LC_RPATH") {
        return Err(error(
            "managed_process.atomic_macos_dependency_closure_unbound",
        ));
    }
    Ok(())
}

fn open_snapshot_root(state_dir: &Path) -> io::Result<(PathBuf, File)> {
    let state = open_directory(state_dir)?;
    let state_before = inspect_open_file(&state)?;
    if state_before.owner != unsafe { libc::geteuid() } || state_before.mode & 0o022 != 0 {
        return Err(error("managed_process.atomic_macos_state_directory_unsafe"));
    }
    match create_private_directory_at(state.as_raw_fd(), SNAPSHOT_ROOT) {
        Ok(()) => state.sync_all()?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let root = open_directory_at(state.as_raw_fd(), SNAPSHOT_ROOT)?;
    let root_information = inspect_open_file(&root)?;
    if root_information.owner != unsafe { libc::geteuid() }
        || root_information.mode != PRIVATE_DIRECTORY_MODE
    {
        return Err(error("managed_process.atomic_macos_snapshot_root_unsafe"));
    }
    let state_after = inspect_open_file(&state)?;
    let state_path = inspect_path_file(state_dir)?;
    if state_after.id != state_before.id
        || state_path.id != state_before.id
        || state_after.owner != state_before.owner
        || state_path.owner != state_before.owner
        || state_after.mode != state_before.mode
        || state_path.mode != state_before.mode
    {
        return Err(error("managed_process.atomic_macos_state_directory_raced"));
    }
    Ok((state_dir.join(SNAPSHOT_ROOT), root))
}

fn validate_private_generation_directory(path: &Path) -> io::Result<()> {
    validate_secure_ancestors(path, true)?;
    let directory = open_directory(path)?;
    let information = inspect_open_file(&directory)?;
    if information.owner != unsafe { libc::geteuid() }
        || information.mode != PUBLISHED_DIRECTORY_MODE
        || inspect_path_file(path)? != information
    {
        return Err(error(
            "managed_process.atomic_macos_generation_directory_unsafe",
        ));
    }
    Ok(())
}

fn validate_secure_ancestors(path: &Path, include_leaf: bool) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(error("managed_process.atomic_macos_path_not_absolute"));
    }
    let mut current = PathBuf::from("/");
    let components = path.components().collect::<Vec<_>>();
    let end = if include_leaf {
        components.len()
    } else {
        components.len().saturating_sub(1)
    };
    for component in components.into_iter().take(end) {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => current.push(name),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(error("managed_process.atomic_macos_path_not_normalized"));
            }
        }
        let metadata = fs::symlink_metadata(&current)?;
        if !metadata.file_type().is_dir()
            || metadata.mode() & 0o022 != 0
            || metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(error("managed_process.atomic_macos_ancestor_unsafe"));
        }
    }
    Ok(())
}

fn inspect_open_file(file: &File) -> io::Result<InspectedFile> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() && !metadata.file_type().is_dir() {
        return Err(error("managed_process.atomic_macos_special_file_rejected"));
    }
    Ok(InspectedFile {
        id: PersistentFileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        },
        size: metadata.len(),
        mode: metadata.mode() & 0o7777,
        links: metadata.nlink(),
        owner: metadata.uid(),
    })
}

fn inspect_path_file(path: &Path) -> io::Result<InspectedFile> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() && !metadata.file_type().is_dir() {
        return Err(error("managed_process.atomic_macos_path_not_plain"));
    }
    Ok(InspectedFile {
        id: PersistentFileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        },
        size: metadata.len(),
        mode: metadata.mode() & 0o7777,
        links: metadata.nlink(),
        owner: metadata.uid(),
    })
}

fn open_plain_file(path: &Path, writable: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(writable);
    options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(error("managed_process.atomic_macos_program_not_regular"));
    }
    Ok(file)
}

fn open_directory(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY);
    options.open(path)
}

fn open_directory_at(parent: RawFd, name: &str) -> io::Result<File> {
    let name = c_name(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
        )
    };
    file_from_fd(fd)
}

fn create_file_at(parent: RawFd, name: &str, mode: u32, flags: i32) -> io::Result<File> {
    let name = c_name(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode as libc::c_uint,
        )
    };
    file_from_fd(fd)
}

fn file_from_fd(fd: RawFd) -> io::Result<File> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn create_private_directory_at(parent: RawFd, name: &str) -> io::Result<()> {
    let name = c_name(name)?;
    if unsafe {
        libc::mkdirat(
            parent,
            name.as_ptr(),
            PRIVATE_DIRECTORY_MODE as libc::mode_t,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn publish_noclobber(parent: RawFd, source: &str, destination: &str) -> io::Result<()> {
    let source = c_name(source)?;
    let destination = c_name(destination)?;
    if unsafe {
        libc::renameatx_np(
            parent,
            source.as_ptr(),
            parent,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_directory_mode(directory: &File, mode: u32) -> io::Result<()> {
    if unsafe { libc::fchmod(directory.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_path_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

fn copy_open_file(source: &mut File, destination: &mut File) -> io::Result<()> {
    source.rewind()?;
    destination.rewind()?;
    let flags = libc::COPYFILE_DATA | libc::COPYFILE_XATTR;
    if unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            flags,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    mirror_extended_attributes(source, destination)?;
    source.rewind()?;
    destination.rewind()?;
    Ok(())
}

fn mirror_extended_attributes(source: &File, destination: &File) -> io::Result<()> {
    let source_attributes = extended_attributes(source)?;
    let destination_attributes = extended_attributes(destination)?;

    for (name, _) in destination_attributes.iter().filter(|(name, _)| {
        !source_attributes
            .iter()
            .any(|(source_name, _)| source_name == name)
    }) {
        let name = CString::new(name.as_slice())
            .map_err(|_| error("managed_process.atomic_macos_xattr_name_invalid"))?;
        if unsafe { libc::fremovexattr(destination.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }

    for (name, value) in source_attributes {
        let already_equal =
            destination_attributes
                .iter()
                .any(|(destination_name, destination_value)| {
                    destination_name == &name && destination_value == &value
                });
        if already_equal {
            continue;
        }
        let name = CString::new(name)
            .map_err(|_| error("managed_process.atomic_macos_xattr_name_invalid"))?;
        if unsafe {
            libc::fsetxattr(
                destination.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn extended_attributes(file: &File) -> io::Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let needed = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    if needed < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut names = vec![0_u8; needed as usize];
    if needed > 0 {
        let actual = unsafe {
            libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len(), 0)
        };
        if actual < 0 {
            return Err(io::Error::last_os_error());
        }
        if actual as usize != names.len() {
            return Err(error("managed_process.atomic_macos_xattrs_changed"));
        }
    }
    let mut names = names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    names.sort();

    names
        .into_iter()
        .map(|name| {
            let name = CString::new(name)
                .map_err(|_| error("managed_process.atomic_macos_xattr_name_invalid"))?;
            let needed = unsafe {
                libc::fgetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                )
            };
            if needed < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut value = vec![0_u8; needed as usize];
            if needed > 0 {
                let actual = unsafe {
                    libc::fgetxattr(
                        file.as_raw_fd(),
                        name.as_ptr(),
                        value.as_mut_ptr().cast(),
                        value.len(),
                        0,
                        0,
                    )
                };
                if actual < 0 {
                    return Err(io::Error::last_os_error());
                }
                if actual as usize != value.len() {
                    return Err(error("managed_process.atomic_macos_xattrs_changed"));
                }
            }
            Ok((name.into_bytes(), value))
        })
        .collect()
}

fn extended_attributes_sha256(file: &File) -> io::Result<String> {
    let mut digest = Sha256::new();
    for (name, value) in extended_attributes(file)? {
        let name = name.as_slice();
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name);
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_file(file: &mut File) -> io::Result<String> {
    file.rewind()?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    file.rewind()?;
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_private_file(path: &Path, mode: u32, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = open_plain_file(path, false)?;
    let information = inspect_open_file(&file)?;
    if information.mode != mode || information.links != 1 || information.size > limit {
        return Err(error("managed_process.atomic_macos_private_file_invalid"));
    }
    let mut bytes = Vec::with_capacity(information.size as usize);
    file.read_to_end(&mut bytes)?;
    if inspect_open_file(&file)? != information || inspect_path_file(path)? != information {
        return Err(error("managed_process.atomic_macos_private_file_raced"));
    }
    Ok(bytes)
}

fn c_name(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| error("managed_process.atomic_macos_name_invalid"))
}

fn error(message: &'static str) -> io::Error {
    io::Error::other(message)
}

#[cfg(test)]
#[path = "managed_process_atomic_macos_tests.rs"]
mod tests;
