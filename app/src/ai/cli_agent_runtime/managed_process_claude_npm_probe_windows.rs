//! Windows Claude npm 公共 cmd/PowerShell 入口的 AppContainer 版本探针。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::{ExpectedFileIdentity, Manifest};

#[path = "../../terminal/cli_agent_updates/sources_npm_claude_windows_contract.rs"]
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
    pub(crate) fn mode(&self) -> &str {
        self.arguments
            .first()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
    }
    pub(crate) fn stage(&self) -> &Path {
        self.arguments.get(1).map_or(Path::new(""), Path::new)
    }
    pub(crate) fn prefix(&self) -> &Path {
        self.arguments.get(2).map_or(Path::new(""), Path::new)
    }
    pub(super) fn files(&self) -> Vec<ExpectedFileIdentity> {
        self.files.clone()
    }
}

fn invalid() -> io::Error {
    io::Error::other("Claude Windows npm 探针闭包不匹配")
}

fn system_program(mode: &str) -> io::Result<PathBuf> {
    let mut buffer = [0u16; 32768];
    let count = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if count == 0 || count >= buffer.len() {
        return Err(invalid());
    }
    let relative = match mode {
        "cmd" => "cmd.exe",
        "powershell" => "WindowsPowerShell/v1.0/powershell.exe",
        _ => return Err(invalid()),
    };
    PathBuf::from(OsString::from_wide(&buffer[..count]))
        .join(relative)
        .canonicalize()
}

fn safe_path(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        && path.to_str().is_some_and(|value| {
            !value.chars().any(|c| {
                c.is_control() || matches!(c, '%' | '!' | '"' | '&' | '|' | '<' | '>' | '^')
            })
        })
}

pub(super) fn valid_entry(program: &Path, arguments: &[OsString]) -> bool {
    if !cfg!(target_arch = "x86_64") || arguments.len() != 3 {
        return false;
    }
    let mode = arguments[0].to_str().unwrap_or_default();
    let stage = Path::new(&arguments[1]);
    let prefix = Path::new(&arguments[2]);
    [program, stage, prefix].into_iter().all(safe_path)
        && system_program(mode).is_ok_and(|expected| expected == program)
        && stage
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|name| name.strip_prefix(".infinishell-npm-"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some_and(|id| !id.is_nil())
        && stage.parent() == Some(prefix.join("node_modules/@anthropic-ai").as_path())
}

pub(super) fn validate_manifest(manifest: &Manifest) -> io::Result<()> {
    validate(&ProbeInputs {
        program: manifest.executable.clone(),
        arguments: manifest.arguments.clone(),
        files: manifest.expected_files.clone(),
    })
}

pub(super) fn validate(input: &ProbeInputs) -> io::Result<()> {
    if !valid_entry(&input.program, &input.arguments) {
        return Err(invalid());
    }
    super::validate_expected_files_contract(&input.program, &input.files)?;
    let mut required = contract::files();
    let mut shims: BTreeMap<_, _> = contract::shims()
        .into_iter()
        .map(|(name, text)| (input.prefix().join(name), text))
        .collect();
    let mut external = BTreeSet::from([input.program.clone()]);
    if input.files.len() > 64
        || input
            .files
            .first()
            .is_none_or(|file| file.path != input.program)
    {
        return Err(invalid());
    }
    for file in &input.files {
        if file.path != file.canonical_path || file.file_id.is_none() || !safe_path(&file.path) {
            return Err(invalid());
        }
        if external.remove(&file.path) {
            continue;
        }
        if let Some(contents) = shims.remove(&file.path) {
            if file.size != contents.len() as u64
                || file.sha256 != format!("{:x}", Sha256::digest(contents.as_bytes()))
            {
                return Err(invalid());
            }
            continue;
        }
        let relative = file
            .path
            .strip_prefix(input.stage())
            .map_err(|_| invalid())?;
        if let Some((length, digest, _mode)) = required.remove(relative) {
            if file.size != length || file.sha256 != digest {
                return Err(invalid());
            }
        } else {
            return Err(invalid());
        }
    }
    if !required.is_empty() || !shims.is_empty() || !external.is_empty() {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn verify_external(input: &ProbeInputs) -> io::Result<()> {
    validate(input)?;
    let files: Vec<_> = input
        .files
        .iter()
        .filter(|file| !file.path.starts_with(input.stage()))
        .cloned()
        .collect();
    super::verify_expected_files(&files)
}

pub(crate) fn capture(mode: &str, stage: &Path, prefix: &Path) -> io::Result<ProbeInputs> {
    let program = system_program(mode)?;
    let arguments = vec![
        mode.into(),
        stage.as_os_str().to_owned(),
        prefix.as_os_str().to_owned(),
    ];
    if !valid_entry(&program, &arguments) {
        return Err(invalid());
    }
    let mut files = vec![ExpectedFileIdentity::capture(&program)?];
    for (name, _contents) in contract::shims() {
        files.push(ExpectedFileIdentity::capture(&prefix.join(name))?);
    }
    for path in contract::files().into_keys() {
        files.push(ExpectedFileIdentity::capture(&stage.join(path))?);
    }
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

fn dos_path(path: &Path) -> io::Result<String> {
    let value = path.to_str().ok_or_else(invalid)?;
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    if value.as_bytes().get(1) != Some(&b':') || !safe_path(path) {
        return Err(invalid());
    }
    Ok(value.to_owned())
}

pub(super) fn execute(manifest: &Manifest, record_directory: &Path) -> io::Result<()> {
    let input = ProbeInputs {
        program: manifest.executable.clone(),
        arguments: manifest.arguments.clone(),
        files: manifest.expected_files.clone(),
    };
    validate(&input)?;
    super::verify_expected_files(&input.files)?;
    let install = manifest.cwd.join("install");
    fs::create_dir(&install)?;
    let package = install.join("node_modules/@anthropic-ai/claude-code");
    let mut directories = BTreeSet::from([manifest.cwd.clone(), install.clone()]);
    let mut handles = Vec::new();
    let mut readonly = Vec::new();
    let mut images = Vec::new();
    for expected in &input.files {
        if expected.path == input.program {
            continue;
        }
        let destination = if expected.path.parent() == Some(input.prefix()) {
            install.join(expected.path.file_name().ok_or_else(invalid)?)
        } else {
            package.join(
                expected
                    .path
                    .strip_prefix(input.stage())
                    .map_err(|_| invalid())?,
            )
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
    readonly.extend(directories.into_iter().filter(|path| path != &manifest.cwd));
    let entry = dos_path(&install.join(if input.mode() == "cmd" {
        "claude.cmd"
    } else {
        "claude.ps1"
    }))?;
    let arguments = match input.mode() {
        "cmd" => format!("/d /v:off /s /c \"\"{entry}\" --version\""),
        "powershell" => format!("-NoLogo -NoProfile -NonInteractive -File \"{entry}\" --version"),
        _ => return Err(invalid()),
    };
    let mut environment = super::version_probe::resolved_environment(&manifest.cwd)?;
    environment.push((
        "CLAUDE_CONFIG_DIR".into(),
        manifest.cwd.join("config").into_os_string(),
    ));
    environment.push(("COMSPEC".into(), system_program("cmd")?.into_os_string()));
    let mut executable = super::atomic_windows::prepare(&input.files[0])?;
    executable.set_package_images(images)?;
    let cwd = super::atomic_windows::prepare_directory(
        manifest.atomic_cwd.as_ref().ok_or_else(invalid)?,
    )?;
    executable.verify_for_spawn()?;
    cwd.verify_for_spawn()?;
    let mut process = command::windows::AppContainerProbe::spawn_package_suspended(
        executable.execution_path(),
        arguments.as_ref(),
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
