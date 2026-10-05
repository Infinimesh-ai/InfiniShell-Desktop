//! Windows Codex npm 公共 cmd/PowerShell 入口的 AppContainer 版本探针。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::atomic_windows::{WindowsDirectoryLease, capture_directory_id, prepare_directory};
use super::probe_control::CancellationListener;
use super::{ExpectedFileIdentity, Manifest};

#[path = "../../terminal/cli_agent_updates/sources_npm_codex_windows_contract.rs"]
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
    pub(crate) fn node(&self) -> &Path {
        self.arguments.get(2).map_or(Path::new(""), Path::new)
    }
    pub(crate) fn prefix(&self) -> &Path {
        self.arguments.get(3).map_or(Path::new(""), Path::new)
    }
    pub(super) fn files(&self) -> Vec<ExpectedFileIdentity> {
        self.files.clone()
    }

    fn shim_template(&self) -> io::Result<contract::ShimTemplate> {
        let mut digests = BTreeMap::new();
        for file in &self.files {
            for name in contract::SHIM_NAMES {
                if file.path == self.prefix().join(name)
                    && digests
                        .insert(name.to_owned(), file.sha256.clone())
                        .is_some()
                {
                    return Err(invalid());
                }
            }
        }
        contract::identify_shims(&digests).ok_or_else(invalid)
    }

    pub(crate) fn shim_template_id(&self) -> io::Result<&'static str> {
        self.shim_template().map(contract::ShimTemplate::id)
    }
}

fn invalid() -> io::Error {
    io::Error::other("Codex Windows npm 探针闭包不匹配")
}

fn station_bootstrap_executable() -> io::Result<PathBuf> {
    // 只使用本版监督程序同目录的已打包引导器，不从用户 PATH 查找。
    let supervisor = super::supervisor_executable()?.canonicalize()?;
    let path = supervisor
        .parent()
        .ok_or_else(invalid)?
        .join("infinishell-station-bootstrap.exe");
    if path.canonicalize()? != path || !safe_path(&path) {
        return Err(invalid());
    }
    Ok(path)
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

fn candidate_environment(mode: &str, root: &Path) -> io::Result<Vec<(OsString, OsString)>> {
    let mut environment = super::version_probe::resolved_environment(root)?;
    if mode == "cmd" {
        // 仅本次受管 CMD 候选使用标准堆，调试事件与隔离约束保持原样。
        environment.push(("_NO_DEBUG_HEAP".into(), "1".into()));
    }
    Ok(environment)
}

fn mapped_command(
    mode: &str,
    root: &Path,
    node_alongside_shim: bool,
) -> io::Result<(OsString, Vec<(OsString, OsString)>)> {
    let value = root.to_str().ok_or_else(invalid)?.as_bytes();
    if value.len() != 3 || !(b'D'..=b'Z').contains(&value[0]) || &value[1..] != b":\\" {
        return Err(invalid());
    }
    let install = root.join("install");
    let entry = dos_path(&install.join(match mode {
        "cmd" => "codex.cmd",
        "powershell" => "codex.ps1",
        _ => return Err(invalid()),
    }))?;
    let arguments = match mode {
        "cmd" => format!("/d /v:off /s /c \"\"{entry}\" --version\""),
        "powershell" => {
            // 空输入和流式输出让原 shim 请求独立管道，不保证禁止系统启动回退。
            // 非零初值避免未取得原生退出码时沿用成功状态。
            format!(
                r#"-NoLogo -NoProfile -NonInteractive -Command "$global:LASTEXITCODE=1; @() | & '{entry}' --version | & {{ process {{ $_ }} }}; exit $global:LASTEXITCODE""#
            )
        }
        _ => return Err(invalid()),
    };
    let mut environment = candidate_environment(mode, root)?;
    if !node_alongside_shim {
        let system_path = environment
            .iter_mut()
            .find(|(name, _)| name == "PATH")
            .ok_or_else(invalid)?;
        // 只在固定 shim 原本使用 PATH 时加入同一私有树的固定 Node，保留官方选路。
        system_path.1 = std::env::join_paths(
            std::iter::once(root.join("runtime")).chain(std::env::split_paths(&system_path.1)),
        )
        .map_err(|_| invalid())?;
    }
    environment.push(("CODEX_HOME".into(), root.join("config").into_os_string()));
    environment.push(("NODE_DISABLE_COMPILE_CACHE".into(), "1".into()));
    environment.push(("COMSPEC".into(), system_program("cmd")?.into_os_string()));
    Ok((arguments.into(), environment))
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
    if !cfg!(target_arch = "x86_64") || arguments.len() != 4 {
        return false;
    }
    let mode = arguments[0].to_str().unwrap_or_default();
    let stage = Path::new(&arguments[1]);
    let node = Path::new(&arguments[2]);
    let prefix = Path::new(&arguments[3]);
    [program, stage, node, prefix].into_iter().all(safe_path)
        && system_program(mode).is_ok_and(|expected| expected == program)
        && stage
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|name| name.strip_prefix(".infinishell-npm-"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some_and(|id| !id.is_nil())
        && node
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes().eq_ignore_ascii_case(b"node.exe"))
        && stage.parent() == Some(prefix.join("node_modules/@openai").as_path())
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
    let mut required = contract::files()?;
    // 三份文件摘要已进入启动摘要与退出收据；监督者只接受同一个完整官方模板。
    let mut shims: BTreeMap<_, _> = input
        .shim_template()?
        .shims()
        .into_iter()
        .map(|(name, text)| (input.prefix().join(name), text))
        .collect();
    let mut external = BTreeSet::from([
        input.program.clone(),
        input.node().to_owned(),
        station_bootstrap_executable()?,
    ]);
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

pub(crate) fn capture(
    mode: &str,
    node: &Path,
    stage: &Path,
    prefix: &Path,
) -> io::Result<ProbeInputs> {
    let program = system_program(mode)?;
    let arguments = vec![
        mode.into(),
        stage.as_os_str().to_owned(),
        node.as_os_str().to_owned(),
        prefix.as_os_str().to_owned(),
    ];
    if !valid_entry(&program, &arguments) {
        return Err(invalid());
    }
    let mut files = vec![
        ExpectedFileIdentity::capture(&program)?,
        ExpectedFileIdentity::capture(node)?,
        ExpectedFileIdentity::capture(&station_bootstrap_executable()?)?,
    ];
    for name in contract::SHIM_NAMES {
        files.push(ExpectedFileIdentity::capture(&prefix.join(name))?);
    }
    for path in contract::files()?.into_keys() {
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

fn dos_bound_path(path: &Path) -> io::Result<PathBuf> {
    // Win32 会重新解释这些名称；不能把 verbatim 对象静默换成另一个 DOS 对象。
    for part in path.components() {
        if let Component::Normal(name) = part {
            let name = name.to_str().ok_or_else(invalid)?;
            let stem = name.split('.').next().unwrap_or_default();
            let upper = stem.to_ascii_uppercase();
            if name.ends_with(['.', ' '])
                || matches!(
                    upper.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
                )
                || upper
                    .strip_prefix("COM")
                    .or_else(|| upper.strip_prefix("LPT"))
                    .is_some_and(|suffix| {
                        matches!(
                            suffix,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
            {
                return Err(invalid());
            }
        }
    }
    let normalized = PathBuf::from(dos_path(path)?);
    if normalized.canonicalize()? != path {
        return Err(invalid());
    }
    Ok(normalized)
}

fn dos_directory_path(lease: &WindowsDirectoryLease) -> io::Result<PathBuf> {
    lease.verify_for_spawn()?;
    let original = lease.execution_path();
    let normalized = dos_bound_path(original)?;
    if capture_directory_id(&normalized)? != capture_directory_id(original)? {
        return Err(invalid());
    }
    Ok(normalized)
}

pub(super) fn execute(
    manifest: &Manifest,
    record_directory: &Path,
    control: &CancellationListener,
) -> io::Result<()> {
    let input = ProbeInputs {
        program: manifest.executable.clone(),
        arguments: manifest.arguments.clone(),
        files: manifest.expected_files.clone(),
    };
    validate(&input)?;
    super::verify_expected_files(&input.files)?;
    let bootstrap_path = station_bootstrap_executable()?;
    let bootstrap_identity = input
        .files
        .iter()
        .find(|file| file.path == bootstrap_path)
        .ok_or_else(invalid)?;
    // 同一文件身份进入事务摘要，并在引导器接管前锁住原文件，封住路径替换窗口。
    let mut bootstrap_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&bootstrap_path)?;
    if ExpectedFileIdentity::capture_opened(
        &bootstrap_path,
        bootstrap_path.clone(),
        &mut bootstrap_handle,
    )? != *bootstrap_identity
    {
        return Err(invalid());
    }
    let bootstrap = command::windows::StationBootstrapImage::capture(
        &bootstrap_path,
        bootstrap_identity.size,
        &bootstrap_identity.sha256,
    )?;
    let install = manifest.cwd.join("install");
    fs::create_dir(&install)?;
    // 保留官方 shim 原先的 Node 选路；PATH 布局不能在镜像中变成本地 Node 分支。
    let node_directory = if input.node().parent() == Some(input.prefix()) {
        install.clone()
    } else {
        manifest.cwd.join("runtime")
    };
    let package = install.join("node_modules/@openai/codex");
    let mut directories = BTreeSet::from([manifest.cwd.clone(), install.clone()]);
    let mut handles = Vec::new();
    let mut readonly = Vec::new();
    let mut images = Vec::new();
    for expected in &input.files {
        if expected.path == input.program || expected.path == bootstrap_path {
            continue;
        }
        let destination = if expected.path == input.node() {
            node_directory.join("node.exe")
        } else if expected.path.parent() == Some(input.prefix()) {
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
    let mut executable = super::atomic_windows::prepare(&input.files[0])?;
    executable.set_package_images(images)?;
    executable.enable_npm_console_host()?;
    let cwd = prepare_directory(manifest.atomic_cwd.as_ref().ok_or_else(invalid)?)?;
    let execution_cwd = dos_directory_path(&cwd)?;
    executable.verify_for_spawn()?;
    cwd.verify_for_spawn()?;
    let mut debugger = executable.prepare_image_debug_session()?;
    debugger.bind_cancellation(control.token());
    debugger.bind_package_diagnostics(manifest.generation);
    #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
    let reader = super::read_native_witness_reader(record_directory, manifest)?;
    let spawn_started = Instant::now();
    let spawned = command::windows::AppContainerProbe::spawn_package_suspended_with_station(
        executable.execution_path(),
        cwd.execution_path(),
        &execution_cwd,
        &format!("InfiniShell.Version.{}", manifest.generation),
        &readonly,
        &bootstrap,
        record_directory,
        |operation, process| control.authorize_process(operation, process),
        |root| {
            let (arguments, environment) =
                mapped_command(input.mode(), root, node_directory == install)?;
            #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
            debugger.bind_native_witness(
                manifest.generation,
                input.mode(),
                &environment,
                &cwd,
                root,
                record_directory,
                reader.as_ref(),
            )?;
            Ok((arguments, environment))
        },
    );
    debugger.record_package_spawn_result(spawned.as_ref().map(|_| ()), spawn_started.elapsed());
    if let Err(failure) = &spawned {
        debugger.record_package_probe_result(Err(failure));
    }
    let mut process = spawned?;
    let started = Instant::now();
    let result = debugger
        .bind_station_debugger(process.station_debugger())
        .and_then(|()| run_package_probe(&mut process, &mut debugger, started));
    // 终止与清理会继续消费原调试事件；先封存本次真实 generation 的正常阶段边界。
    debugger.record_package_probe_result(result.as_ref().map(|code| *code));
    if let Err(failure) = &result {
        record_phase("probe_failed", started, Some(failure));
    }
    record_phase("cleanup_before", started, None);
    // 派生后任何失败都保留原事件状态；先请求终止精确 Job，才允许继续未验证事件。
    #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
    if result.is_err() {
        debugger.note_native_witness_cancel();
    }
    let termination = match &result {
        Ok(_) => Ok(()),
        Err(_) => terminate_package_probe(&process, &mut debugger),
    };
    drop(cwd);
    let cleanup = termination.and_then(|()| {
        process.write_cleanup_receipt_with_output(
            &record_directory.join("appcontainer-cleanup-v1"),
            &mut io::stdout(),
            &mut io::stderr(),
            || {
                if control.token().load(Ordering::Acquire) {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "managed_process.atomic_windows_probe_cancelled",
                    ))
                } else {
                    Ok(())
                }
            },
        )
    });
    match &cleanup {
        Ok(()) => record_phase("cleanup_complete", started, None),
        Err(failure) => record_phase("cleanup_failed", started, Some(failure)),
    }
    #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
    debugger.record_native_witness_result();
    drop(handles);
    drop(bootstrap);
    drop(bootstrap_handle);
    let code = match result {
        Ok(code) => {
            cleanup?;
            code
        }
        Err(failure) => {
            if let Err(cleanup_failure) = cleanup {
                let kind = cleanup_failure.kind();
                let os_code = cleanup_failure.raw_os_error();
                // 保留原映像拒绝错误；额外清理错误只输出固定类型，不含路径或控制材料。
                warp_core::safe_eprintln!(
                    safe: ("managed_process.windows_npm_cleanup_unconfirmed kind={kind:?} os_code={os_code:?}"),
                    full: ("managed_process.windows_npm_cleanup_unconfirmed kind={kind:?} os_code={os_code:?}")
                );
            }
            return Err(failure);
        }
    };
    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code as i32);
    }
}

fn run_package_probe(
    process: &mut command::windows::AppContainerProbe,
    debugger: &mut super::atomic_windows::WindowsImageDebugSession,
    started: Instant,
) -> io::Result<u32> {
    record_phase("resume_before", started, None);
    process.resume()?;
    record_phase("initial_image_before", started, None);
    debugger.verify_package_initial_image_in_container(process)?;
    record_phase("drain_before", started, None);
    debugger.drain_package_in_container_until_exit(process)?;
    record_phase("exit_code_before", started, None);
    process.exit_code()
}

fn record_phase(phase: &'static str, started: Instant, failure: Option<&io::Error>) {
    let elapsed_ms = started.elapsed().as_millis();
    let kind = failure.map(io::Error::kind);
    let os_code = failure.and_then(io::Error::raw_os_error);
    // 两个日志分支只含固定阶段与非敏感诊断值，不输出错误正文。
    warp_core::safe_eprintln!(
        safe: ("managed_process.windows_npm_probe phase={phase} elapsed_ms={elapsed_ms} kind={kind:?} os_code={os_code:?}"),
        full: ("managed_process.windows_npm_probe phase={phase} elapsed_ms={elapsed_ms} kind={kind:?} os_code={os_code:?}")
    );
}

fn terminate_package_probe(
    process: &command::windows::AppContainerProbe,
    debugger: &mut super::atomic_windows::WindowsImageDebugSession,
) -> io::Result<()> {
    debugger.record_package_termination_request();
    process.terminate_job()?;
    // 显式传入已持有的根进程身份，包含 resume 自身失败、尚无首事件的路径。
    debugger.drain_terminated_package_in_container(process)
}

#[cfg(test)]
#[path = "managed_process_npm_probe_windows_tests.rs"]
mod tests;
