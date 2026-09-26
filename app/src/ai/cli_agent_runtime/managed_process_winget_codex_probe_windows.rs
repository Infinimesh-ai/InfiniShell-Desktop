//! Windows Codex WinGet 完整便携目录的独立 AppContainer 版本探针。

use super::{ExpectedFileIdentity, Manifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, GetFileInformationByHandle,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
#[path = "../../terminal/cli_agent_updates/sources_winget_codex_contract.rs"]
mod contract;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeInputs {
    program: PathBuf,
    arguments: Vec<OsString>,
    files: Vec<ExpectedFileIdentity>,
}
impl ProbeInputs {
    pub(crate) fn program(&self) -> &Path {
        &self.program
    }
    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }
    pub(crate) fn stage(&self) -> &Path {
        self.arguments.first().map_or(Path::new(""), Path::new)
    }
    pub(crate) fn prefix(&self) -> &Path {
        self.arguments.get(1).map_or(Path::new(""), Path::new)
    }
    fn rg(&self) -> &Path {
        self.arguments.get(2).map_or(Path::new(""), Path::new)
    }
    pub(super) fn files(&self) -> Vec<ExpectedFileIdentity> {
        self.files.clone()
    }
}
fn invalid() -> io::Error {
    io::Error::other("Codex WinGet 探针闭包不匹配")
}
fn safe(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|s| !s.chars().any(|c| c.is_control() || c == '\"'))
        && !path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}
fn key(path: &Path) -> io::Result<String> {
    let value = path.to_str().ok_or_else(invalid)?;
    Ok(value
        .strip_prefix(r"\\?\")
        .unwrap_or(value)
        .replace('/', "\\")
        .to_ascii_lowercase())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicLink {
    path: PathBuf,
    target: PathBuf,
    volume: u32,
    id: u64,
}
impl PublicLink {
    fn capture(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(path)?;
        Self::from_handle(path, &file)
    }
    fn from_handle(path: &Path, file: &File) -> io::Result<Self> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(io::Error::other)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            || info.nNumberOfLinks != 1
            || !fs::symlink_metadata(path)?.file_type().is_symlink()
        {
            return Err(invalid());
        }
        Ok(Self {
            path: path.to_owned(),
            target: fs::read_link(path)?,
            volume: info.dwVolumeSerialNumber,
            id: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }
    fn hold(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(&self.path)?;
        if Self::from_handle(&self.path, &file)? != *self {
            return Err(invalid());
        }
        Ok(file)
    }
}
fn links(input: &ProbeInputs) -> io::Result<Vec<PublicLink>> {
    serde_json::from_str(
        input
            .arguments
            .get(3)
            .and_then(|s| s.to_str())
            .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())
}
fn system() -> io::Result<PathBuf> {
    let mut buffer = [0u16; 32768];
    let count = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if count == 0 || count >= buffer.len() {
        return Err(invalid());
    }
    PathBuf::from(OsString::from_wide(&buffer[..count])).canonicalize()
}

pub(super) fn valid_entry(program: &Path, arguments: &[OsString]) -> bool {
    if !cfg!(target_arch = "x86_64") || arguments.len() != 4 {
        return false;
    }
    let stage = Path::new(&arguments[0]);
    let prefix = Path::new(&arguments[1]);
    let rg = Path::new(&arguments[2]);
    [program, stage, prefix, rg].into_iter().all(safe)
        && program == stage.join("codex.exe")
        && stage.parent() == Some(prefix.join("Packages").as_path())
        && stage
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|s| s.strip_prefix(".infinishell-winget-"))
            .and_then(|s| Uuid::parse_str(s).ok())
            .is_some_and(|id| !id.is_nil())
        && rg.starts_with(
            prefix
                .join("Packages")
                .join(format!("BurntSushi.ripgrep.MSVC_{}", contract::SOURCE)),
        )
        && rg.file_name().is_some_and(|name| name == "rg.exe")
}
pub(super) fn validate_manifest(manifest: &Manifest) -> io::Result<()> {
    validate(&ProbeInputs {
        program: manifest.executable.clone(),
        arguments: manifest.arguments.clone(),
        files: manifest.expected_files.clone(),
    })
}
pub(super) fn validate(input: &ProbeInputs) -> io::Result<()> {
    if !valid_entry(&input.program, &input.arguments)
        || input
            .files
            .first()
            .is_none_or(|file| file.path != input.program)
    {
        return Err(invalid());
    }
    super::validate_expected_files_contract(&input.program, &input.files)?;
    let mut required = contract::files(contract::VERSION).ok_or_else(invalid)?;
    required.insert(
        "codex.exe".into(),
        *required
            .get(Path::new(contract::NATIVE))
            .ok_or_else(invalid)?,
    );
    let mut external: BTreeSet<_> = ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"]
        .into_iter()
        .map(|name| system().and_then(|path| path.join(name).canonicalize()))
        .collect::<io::Result<_>>()?;
    external.insert(input.rg().to_owned());
    for file in &input.files {
        if file.path != file.canonical_path || !safe(&file.path) || file.file_id.is_none() {
            return Err(invalid());
        }
        if external.remove(&file.path) {
            continue;
        }
        let relative = file
            .path
            .strip_prefix(input.stage())
            .map_err(|_| invalid())?;
        let (length, sha) = required.remove(relative).ok_or_else(invalid)?;
        if file.size != length || file.sha256 != sha {
            return Err(invalid());
        }
    }
    let public = links(input)?;
    if public.len() != 3 || !required.is_empty() || !external.is_empty() {
        return Err(invalid());
    }
    for (link, (name, target)) in public.iter().zip(contract::LINKS) {
        if key(&link.path)? != key(&input.prefix().join("Links").join(name))?
            || key(&link.target)?
                != key(&input
                    .prefix()
                    .join("Packages")
                    .join(contract::PRODUCT)
                    .join(target))?
            || link.id == 0
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub(super) fn verify_external(input: &ProbeInputs) -> io::Result<()> {
    validate(input)?;
    for link in links(input)? {
        if PublicLink::capture(&link.path)? != link {
            return Err(invalid());
        }
    }
    super::verify_expected_files(
        &input
            .files
            .iter()
            .filter(|file| !file.path.starts_with(input.stage()))
            .cloned()
            .collect::<Vec<_>>(),
    )
}
pub(crate) fn capture(
    stage: &Path,
    prefix: &Path,
    rg: &Path,
    runtime: &[ExpectedFileIdentity],
) -> io::Result<ProbeInputs> {
    let public = contract::LINKS
        .into_iter()
        .map(|(name, _)| PublicLink::capture(&prefix.join("Links").join(name)))
        .collect::<io::Result<Vec<_>>>()?;
    let arguments = vec![
        stage.as_os_str().to_owned(),
        prefix.as_os_str().to_owned(),
        rg.as_os_str().to_owned(),
        serde_json::to_string(&public)
            .map_err(|_| invalid())?
            .into(),
    ];
    let program = stage.join("codex.exe");
    let mut files = vec![ExpectedFileIdentity::capture(&program)?];
    for path in contract::files(contract::VERSION)
        .ok_or_else(invalid)?
        .into_keys()
    {
        files.push(ExpectedFileIdentity::capture(&stage.join(path))?);
    }
    files.push(ExpectedFileIdentity::capture(rg)?);
    files.extend_from_slice(runtime);
    let input = ProbeInputs {
        program,
        arguments,
        files,
    };
    validate(&input)?;
    Ok(input)
}

fn copy(expected: &ExpectedFileIdentity, path: &Path) -> io::Result<File> {
    let mut source = super::open_expected_file(&expected.path)?;
    if ExpectedFileIdentity::capture_opened(&expected.path, expected.path.clone(), &mut source)?
        != *expected
    {
        return Err(invalid());
    }
    source.rewind()?;
    let mut destination = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > expected.size {
            return Err(invalid());
        }
        hash.update(&buffer[..count]);
        destination.write_all(&buffer[..count])?;
    }
    if bytes != expected.size
        || format!("{:x}", hash.finalize()) != expected.sha256
        || ExpectedFileIdentity::capture_opened(&expected.path, expected.path.clone(), &mut source)?
            != *expected
    {
        return Err(invalid());
    }
    destination.sync_all()?;
    drop(destination);
    // 这些句柄直到 Job 清空才释放，AppContainer 不能重写或替换被测脚本/镜像。
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

pub(super) fn execute(manifest: &Manifest, record_directory: &Path) -> io::Result<()> {
    let input = ProbeInputs {
        program: manifest.executable.clone(),
        arguments: manifest.arguments.clone(),
        files: manifest.expected_files.clone(),
    };
    validate(&input)?;
    super::verify_expected_files(&input.files)?;
    let mut handles: Vec<File> = links(&input)?
        .iter()
        .map(PublicLink::hold)
        .collect::<io::Result<_>>()?;
    let install = manifest.cwd.join("install");
    fs::create_dir(&install)?;
    let dependency = manifest.cwd.join("dependency");
    fs::create_dir(&dependency)?;
    let mut directories = BTreeSet::from([install.clone(), dependency.clone()]);
    let mut images = Vec::new();
    let mut readonly = Vec::new();
    for expected in &input.files {
        let destination = if let Ok(relative) = expected.path.strip_prefix(input.stage()) {
            install.join(relative)
        } else if expected.path == input.rg() {
            dependency.join("rg.exe")
        } else {
            continue;
        };
        let relative = destination
            .strip_prefix(&manifest.cwd)
            .map_err(|_| invalid())?;
        let mut path = manifest.cwd.clone();
        for part in relative.parent().ok_or_else(invalid)?.components() {
            let Component::Normal(name) = part else {
                return Err(invalid());
            };
            path.push(name);
            if directories.insert(path.clone()) {
                fs::create_dir(&path)?;
            }
        }
        handles.push(copy(expected, &destination)?);
        if destination.extension().is_some_and(|suffix| {
            suffix.as_encoded_bytes().eq_ignore_ascii_case(b"exe")
                || suffix.as_encoded_bytes().eq_ignore_ascii_case(b"dll")
        }) {
            images.push(ExpectedFileIdentity::capture(&destination)?);
        }
        readonly.push(destination);
    }
    // ZIP 中的空 bin 目录也必须保持，不能依赖当前机器恰好存在的其他资源。
    for relative in contract::directories() {
        if relative.as_os_str().is_empty() {
            continue;
        }
        let directory = install.join(relative);
        if directories.insert(directory.clone()) {
            fs::create_dir(&directory)?;
        }
    }
    readonly.extend(directories);
    let program = ExpectedFileIdentity::capture(&install.join("codex.exe"))?;
    images.retain(|image| image.path != program.path);
    let mut executable = super::atomic_windows::prepare(&program)?;
    executable.set_package_images(images)?;
    let cwd = super::atomic_windows::prepare_directory(
        manifest.atomic_cwd.as_ref().ok_or_else(invalid)?,
    )?;
    let mut environment = super::version_probe::resolved_environment(&manifest.cwd)?;
    environment.retain(|(name, _)| name != "PATH");
    environment.push((
        "PATH".into(),
        std::env::join_paths([dependency, system()?]).map_err(|_| invalid())?,
    ));
    environment.push((
        "CODEX_HOME".into(),
        manifest.cwd.join("config").into_os_string(),
    ));
    verify_external(&input)?;
    executable.verify_for_spawn()?;
    cwd.verify_for_spawn()?;
    let mut process = command::windows::AppContainerProbe::spawn_package_suspended(
        executable.execution_path(),
        "--version".as_ref(),
        cwd.execution_path(),
        &environment,
        &format!("InfiniShell.Version.{}", manifest.generation),
        &readonly,
    )?;
    process.resume()?;
    let mut debugger = executable.begin_image_debug_session(process.id())?;
    debugger.drain_until_exit()?;
    let code = process.exit_code()?;
    drop(cwd);
    process.write_cleanup_receipt(&record_directory.join("appcontainer-cleanup-v1"))?;
    drop(handles);
    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code as i32);
    }
}
