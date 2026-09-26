//! Windows 固定 Grok 普通终端：真实 ConPTY 父进程、原子专属 Job 与一次派发收据。

#[path = "grok_owned_windows_files.rs"]
mod files;

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use command::blocking::Command;
use command::managed::{WindowsProcessIdentity, WindowsProcessLease, windows_process_exited};
use command::owned_console_windows::OwnedConsoleChild;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::runtime::Builder;
use uuid::Uuid;
use warpui::{AppContext, EntityId, SingletonEntity};
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::System::Console::{
    AttachConsole, CONSOLE_MODE, CTRL_BREAK_EVENT, CTRL_C_EVENT, GetConsoleMode,
    GetConsoleProcessList, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    SetConsoleCtrlHandler,
};
use windows::core::BOOL;

use super::*;
use crate::terminal::cli_agent_sessions::grok_owned_history_source_windows::NpmGrokSource;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::GrokPermissionObservation;
use crate::terminal::cli_agent_sessions::grok_leader_input::{
    GrokLeaderInputError, GrokLeaderTarget, windows_leader_pipe_name,
};
use crate::terminal::cli_agent_updates::CliAgentUpdatesModel;
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

const CLI_SHA256: &str = "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25";
const CLI_VERSION_OUTPUT: &str = "grok 1.0.41 (4220f3b224a6)";
const MODEL: &str = "grok-4.7";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessIdentity {
    pid: u32,
    created_at: u64,
    logon_low: u32,
    logon_high: i32,
    session_id: u32,
}
impl From<WindowsProcessIdentity> for ProcessIdentity {
    fn from(identity: WindowsProcessIdentity) -> Self {
        Self {
            pid: identity.pid,
            created_at: identity.created_at,
            logon_low: identity.logon_low,
            logon_high: identity.logon_high,
            session_id: identity.session_id,
        }
    }
}
impl From<ProcessIdentity> for WindowsProcessIdentity {
    fn from(identity: ProcessIdentity) -> Self {
        Self {
            pid: identity.pid,
            created_at: identity.created_at,
            logon_low: identity.logon_low,
            logon_high: identity.logon_high,
            session_id: identity.session_id,
        }
    }
}

pub(crate) struct GrokOwnedPty {
    identity: LocalPtyIdentity,
}
impl GrokOwnedPty {
    pub(crate) fn from_local_pty(identity: LocalPtyIdentity) -> io::Result<Self> {
        identity.current_shell()?;
        Ok(Self { identity })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchManifest {
    version: u32,
    launch_id: Uuid,
    session_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history: Option<HistorySource>,
    executable: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    npm_source: Option<NpmGrokSource>,
    app_executable: PathBuf,
    cwd: PathBuf,
    socket_path: PathBuf,
    directory: files::FileIdentity,
    pty_generation: Uuid,
    shell: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WrapperReceipt {
    version: u32,
    launch_id: Uuid,
    manifest_sha256: String,
    process: ProcessIdentity,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnReceipt {
    wrapper: WrapperReceipt,
    tui: ProcessIdentity,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundProcesses {
    spawn: SpawnReceipt,
    leader: ProcessIdentity,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitReceipt {
    spawn: SpawnReceipt,
    job_empty: bool,
    exit_code: Option<u32>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartFailure {
    wrapper: WrapperReceipt,
    no_native_started: bool,
    os_error: Option<i32>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetiredLaunch {
    version: u32,
    launch_id: Uuid,
    manifest_sha256: String,
    dispatched: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LaunchPhase {
    Prepared,
    Reserved,
    Dispatched,
    RecoveredUnsent,
    Released,
}

pub(crate) struct GrokOwnedLaunch {
    directory: PathBuf,
    manifest: LaunchManifest,
    manifest_sha256: String,
    reservation: Option<EntityId>,
    phase: LaunchPhase,
    processes: Option<BoundProcesses>,
    leases: Vec<WindowsProcessLease>,
}

impl GrokOwnedLaunch {
    pub(crate) fn prepare(
        executable: &Path,
        app_executable: &Path,
        cwd: &Path,
        state_directory: &Path,
        pty: GrokOwnedPty,
    ) -> io::Result<Self> {
        Self::prepare_with_history(executable, app_executable, cwd, state_directory, pty, None)
    }

    pub(crate) fn prepare_history(
        executable: &Path,
        app_executable: &Path,
        cwd: &Path,
        state_directory: &Path,
        pty: GrokOwnedPty,
        history: HistorySource,
    ) -> io::Result<Self> {
        history.validate_target(cwd, history.session_id)?;
        history.verify_exited()?;
        Self::prepare_with_history(executable, app_executable, cwd, state_directory, pty, Some(history))
    }

    fn prepare_with_history(
        executable: &Path,
        app_executable: &Path,
        cwd: &Path,
        state_directory: &Path,
        pty: GrokOwnedPty,
        history: Option<HistorySource>,
    ) -> io::Result<Self> {
        let npm_source = NpmGrokSource::capture(executable)?;
        let executable = npm_source.as_ref().map_or_else(|| executable.canonicalize(), |source| Ok(source.executable.clone()))?;
        let (_binary, _source_files) = verify_binary(&executable, npm_source.as_ref())?;
        let mut command = Command::new(&executable);
        command.arg("--version").env("GROK_DISABLE_AUTOUPDATER", "1");
        if let Some(source) = &npm_source { command.env("GROK_MANAGED_BY_NPM", "1").env("GROK_HOME", &source.grok_home); }
        let output = command.output()?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stdout).trim() != CLI_VERSION_OUTPUT
        {
            return Err(io::Error::other("Grok 固定版本不匹配"));
        }
        let cwd = dunce::canonicalize(cwd)?;
        files::plain_path(&cwd)?;
        if !cwd.is_dir() {
            return Err(io::Error::other("Grok 工作目录无效"));
        }
        let app_executable = app_executable.canonicalize()?;
        files::plain_path(&app_executable)?;
        let shell = pty.identity.current_shell()?.into();
        let (directory, identity) = files::create_launch_directory(state_directory)?;
        let manifest = LaunchManifest {
            version: 4,
            launch_id: Uuid::new_v4(),
            session_id: history.as_ref().map_or_else(Uuid::new_v4, |source| source.session_id),
            history,
            executable,
            npm_source,
            app_executable,
            cwd,
            socket_path: directory.join("leader.sock"),
            directory: identity,
            pty_generation: pty.identity.generation(),
            shell,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
        files::write_new(&directory.join("launch.json"), &bytes)?;
        Ok(Self {
            directory,
            manifest,
            manifest_sha256: digest(&bytes),
            reservation: None,
            phase: LaunchPhase::Prepared,
            processes: None,
            leases: Vec::new(),
        })
    }

    /// 只关联清单与原进程；恢复路径永远不能重新领取 argv。
    pub(crate) fn restore(path: &Path, expected_sha256: &str) -> io::Result<Self> {
        let bytes = files::read_private(path)?;
        if digest(&bytes) != expected_sha256 {
            return Err(io::Error::other("owned Grok 清单摘要改变"));
        }
        let manifest: LaunchManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        validate_manifest(path, &manifest)?;
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("启动清单缺少目录"))?
            .to_owned();
        let phase = match files::read_optional(&directory.join("dispatched"))? {
            Some(bytes) if bytes == expected_sha256.as_bytes() => LaunchPhase::Dispatched,
            Some(_) => return Err(io::Error::other("owned Grok 派发记录不匹配")),
            None => LaunchPhase::RecoveredUnsent,
        };
        let mut launch = Self {
            directory,
            manifest,
            manifest_sha256: expected_sha256.to_owned(),
            reservation: None,
            phase,
            processes: None,
            leases: Vec::new(),
        };
        if let Some(bytes) = files::read_optional(&launch.directory.join("retired.json"))? {
            let record: RetiredLaunch = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if record.version != 1
                || record.launch_id != launch.launch_id()
                || record.manifest_sha256 != expected_sha256
                || record.dispatched != (phase == LaunchPhase::Dispatched)
            {
                return Err(io::Error::other("owned Grok 退出收据不匹配"));
            }
            launch.phase = LaunchPhase::Released;
        } else if phase == LaunchPhase::Dispatched {
            launch.refresh_bound_processes()?;
        }
        Ok(launch)
    }

    pub(crate) fn reserve_launch(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase != LaunchPhase::Prepared {
            return Err(io::Error::other("owned Grok 启动已领取"));
        }
        self.reserve(ctx)?;
        self.phase = LaunchPhase::Reserved;
        Ok(())
    }
    pub(crate) fn reserve_for_recovery(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched || self.reservation.is_some() {
            return Err(io::Error::other("owned Grok 恢复占用状态无效"));
        }
        self.reserve(ctx)
    }
    fn reserve(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        let reservation = EntityId::new();
        if !CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
            model.reserve_launch(CLIAgent::Grok, reservation, ctx)
        }) {
            return Err(io::Error::other("Grok 正在升级，启动未派发"));
        }
        self.reservation = Some(reservation);
        Ok(())
    }
    pub(crate) fn take_launch_argv(&mut self) -> io::Result<Vec<OsString>> {
        if self.phase != LaunchPhase::Reserved {
            return Err(io::Error::other("owned Grok 启动尚未占用或已派发"));
        }
        let (_binary, _source_files) = verify_binary(
            &self.manifest.executable,
            self.manifest.npm_source.as_ref(),
        )?;
        files::write_new(
            &self.directory.join("dispatched"),
            self.manifest_sha256.as_bytes(),
        )?;
        self.phase = LaunchPhase::Dispatched;
        Ok(vec![
            self.manifest.app_executable.as_os_str().to_owned(),
            OWNED_GROK_COMMAND.into(),
            self.manifest_path().into_os_string(),
        ])
    }
    pub(crate) fn launch_id(&self) -> Uuid {
        self.manifest.launch_id
    }
    pub(crate) fn session_id(&self) -> Uuid {
        self.manifest.session_id
    }
    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.directory.join("launch.json")
    }
    pub(crate) fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    pub(crate) fn confirm_history_exit(&mut self) -> io::Result<()> {
        if self.phase == LaunchPhase::Released && files::read_optional(&self.directory.join("dispatched"))?.is_none() {
            for name in ["wrapper.json", "spawn.json", "bound.json", "start-failed.json"] { if files::read_optional(&self.directory.join(name))?.is_some() { return Err(io::Error::other("历史启动仍有原生执行记录")); } }
            return Ok(());
        }
        let previous_phase = self.phase;
        if self.phase == LaunchPhase::Released { self.phase = LaunchPhase::Dispatched; }
        let refreshed = self.refresh_bound_processes();
        self.phase = previous_phase;
        refreshed?;
        self.confirm_exit()
    }

    pub(crate) fn matches_history_pty(&self, pty: &GrokOwnedPty) -> io::Result<bool> {
        Ok(self.manifest.shell == ProcessIdentity::from(pty.identity.current_shell()?) && self.manifest.pty_generation == pty.identity.generation())
    }

    pub(crate) fn working_directory(&self) -> &Path {
        &self.manifest.cwd
    }
    pub(crate) fn is_retired(&self) -> bool {
        self.phase == LaunchPhase::Released
    }
    pub(crate) fn was_dispatched(&self) -> bool {
        self.phase == LaunchPhase::Dispatched
    }

    fn spawn_receipt(&self) -> io::Result<SpawnReceipt> {
        let receipt: SpawnReceipt =
            serde_json::from_slice(&files::read_private(&self.directory.join("spawn.json"))?)
                .map_err(io::Error::other)?;
        validate_spawn(&receipt, &self.manifest, &self.manifest_sha256)?;
        Ok(receipt)
    }
    pub(crate) fn refresh_bound_processes(&mut self) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched || self.processes.is_some() {
            return Ok(());
        }
        if let Some(bytes) = files::read_optional(&self.directory.join("bound.json"))? {
            let bound: BoundProcesses = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if bound.spawn != self.spawn_receipt()?
                || bound.leader.pid == bound.spawn.tui.pid
                || bound.leader.created_at == 0
            {
                return Err(io::Error::other("owned Grok 进程归属记录不匹配"));
            }
            let mut leases = Vec::new();
            for expected in [bound.spawn.wrapper.process, bound.spawn.tui, bound.leader] {
                if !windows_process_exited(expected.into())? {
                    let lease = WindowsProcessLease::capture(expected.pid)?;
                    if lease.identity() != expected.into() {
                        return Err(io::Error::other("owned Grok 进程身份已变化"));
                    }
                    leases.push(lease);
                }
            }
            self.leases = leases;
            self.processes = Some(bound);
        }
        Ok(())
    }
    pub(crate) fn capture_owned_processes(&mut self) -> io::Result<()> {
        self.refresh_bound_processes()
    }

    pub(crate) fn bind(
        &mut self,
        binding_id: Uuid,
        permission_revision: Uuid,
        observation: &GrokPermissionObservation,
    ) -> Result<GrokOwnedBinding, GrokLeaderInputError> {
        if self.phase != LaunchPhase::Dispatched
            || binding_id.is_nil()
            || permission_revision.is_nil()
            || observation.session_id != self.session_id().to_string()
            || dunce::canonicalize(&observation.cwd).ok().as_deref()
                != Some(self.manifest.cwd.as_path())
            || observation.mode != "default"
            || observation.session_start_event_id.is_empty()
        {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        self.refresh_bound_processes()?;
        let bound = self
            .processes
            .as_ref()
            .ok_or(GrokLeaderInputError::InvalidTarget)?;
        let target = GrokLeaderTarget::capture(
            binding_id,
            self.session_id(),
            &self.manifest.cwd,
            &self.manifest.socket_path,
            &self.manifest.executable,
            bound.spawn.tui.into(),
            bound.leader.into(),
            &observation.mode,
        )?;
        Ok(GrokOwnedBinding {
            target,
            launch_id: self.launch_id(),
            binding_id,
            session_id: self.session_id(),
            permission_revision,
            working_directory: self.manifest.cwd.clone(),
        })
    }

    /// Windows wrapper 持有本次原子 Job 并在 TUI 退出后清理；GUI 不终止管道枚举出的进程。
    pub(crate) fn stop_leader_after_tui_exit(&self) -> io::Result<()> {
        if let Some(bound) = &self.processes {
            if windows_process_exited(bound.spawn.wrapper.process.into())?
                && !windows_process_exited(bound.leader.into())?
            {
                return Err(io::Error::other(
                    "owned Grok wrapper 已退出，专属 Job 清理仍未确认",
                ));
            }
        }
        Ok(())
    }
    fn confirm_exit(&self) -> io::Result<()> {
        if let Some(bytes) = files::read_optional(&self.directory.join("exit.json"))? {
            let exit: ExitReceipt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if !exit.job_empty
                || exit.spawn != self.spawn_receipt()?
                || !windows_process_exited(exit.spawn.tui.into())?
                || !windows_process_exited(exit.spawn.wrapper.process.into())?
            {
                return Err(io::Error::other("owned Grok 原生 Job 退出尚未确认"));
            }
            return Ok(());
        }
        if let Some(bound) = &self.processes {
            if [bound.spawn.wrapper.process, bound.spawn.tui, bound.leader]
                .into_iter()
                .map(|identity| windows_process_exited(identity.into()))
                .collect::<io::Result<Vec<_>>>()?
                .iter()
                .all(|exited| *exited)
            {
                return Ok(());
            }
        }
        if let Some(bytes) = files::read_optional(&self.directory.join("start-failed.json"))? {
            let failure: StartFailure = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            validate_wrapper(&failure.wrapper, &self.manifest, &self.manifest_sha256)?;
            if failure.no_native_started
                && files::read_optional(&self.directory.join("spawn.json"))?.is_none()
                && windows_process_exited(failure.wrapper.process.into())?
            {
                return Ok(());
            }
        }
        Err(io::Error::other(
            "owned Grok 真实退出证据不足，保留启动占用",
        ))
    }
    fn retire(&mut self, dispatched: bool, ctx: &mut AppContext) -> io::Result<()> {
        let record = RetiredLaunch {
            version: 1,
            launch_id: self.launch_id(),
            manifest_sha256: self.manifest_sha256.clone(),
            dispatched,
        };
        files::write_new(
            &self.directory.join("retired.json"),
            &serde_json::to_vec(&record).map_err(io::Error::other)?,
        )?;
        if let Some(reservation) = self.reservation.take() {
            CliAgentUpdatesModel::handle(ctx)
                .update(ctx, |model, ctx| model.release_launch(reservation, ctx));
        }
        self.phase = LaunchPhase::Released;
        self.leases.clear();
        Ok(())
    }
    pub(crate) fn release_after_exit(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched {
            return Err(io::Error::other("owned Grok 尚未派发或已释放"));
        }
        self.confirm_exit()?;
        self.retire(true, ctx)
    }
    pub(crate) fn cancel_before_dispatch(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase == LaunchPhase::Released {
            return Ok(());
        }
        if self.phase == LaunchPhase::Dispatched {
            return Err(io::Error::other("owned Grok 已派发，不能重新领取"));
        }
        for name in [
            "dispatched",
            "wrapper.json",
            "spawn.json",
            "bound.json",
            "exit.json",
            "start-failed.json",
        ] {
            if files::read_optional(&self.directory.join(name))?.is_some() {
                return Err(io::Error::other("owned Grok 已有执行记录"));
            }
        }
        self.retire(false, ctx)
    }
}

pub(crate) struct GrokOwnedBinding {
    target: GrokLeaderTarget,
    pub(crate) launch_id: Uuid,
    pub(crate) binding_id: Uuid,
    pub(crate) session_id: Uuid,
    pub(crate) permission_revision: Uuid,
    pub(crate) working_directory: PathBuf,
}
impl GrokOwnedBinding {
    pub(crate) fn into_target(self) -> GrokLeaderTarget {
        self.target
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn validate_manifest(path: &Path, manifest: &LaunchManifest) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("启动清单缺少目录"))?;
    if manifest.version != 4
        || path.file_name() != Some(OsStr::new("launch.json"))
        || manifest.launch_id.is_nil()
        || manifest.session_id.is_nil()
        || manifest.pty_generation.is_nil()
        || manifest.shell.pid == 0
        || manifest.shell.created_at == 0
        || !manifest.cwd.is_absolute()
        || !manifest.executable.is_absolute()
        || !manifest.app_executable.is_absolute()
        || manifest.socket_path != parent.join("leader.sock")
        || files::file_identity(&files::directory(parent)?)? != manifest.directory
    {
        return Err(io::Error::other("Windows owned Grok 清单或目录身份无效"));
    }
    Ok(())
}
fn validate_wrapper(
    receipt: &WrapperReceipt,
    manifest: &LaunchManifest,
    sha256: &str,
) -> io::Result<()> {
    if receipt.version != 1
        || receipt.launch_id != manifest.launch_id
        || receipt.manifest_sha256 != sha256
        || receipt.process.pid == 0
        || receipt.process.created_at == 0
        || receipt.process.pid == manifest.shell.pid
    {
        return Err(io::Error::other("owned Grok wrapper 记录不匹配"));
    }
    Ok(())
}
fn validate_spawn(
    receipt: &SpawnReceipt,
    manifest: &LaunchManifest,
    sha256: &str,
) -> io::Result<()> {
    validate_wrapper(&receipt.wrapper, manifest, sha256)?;
    if receipt.tui.pid == 0
        || receipt.tui.created_at == 0
        || receipt.tui.pid == receipt.wrapper.process.pid
        || receipt.tui.pid == manifest.shell.pid
    {
        return Err(io::Error::other("owned Grok 原始派生记录不匹配"));
    }
    Ok(())
}
fn verify_binary(path: &Path, source: Option<&NpmGrokSource>) -> io::Result<(File, Vec<File>)> {
    // 来源只在最初准备时捕获；后续 shell 环境不能替换清单中的 npm 安装。
    let source_files = match source {
        Some(source) => {
            if source.executable != path {
                return Err(io::Error::other("npm 启动映像不匹配"));
            }
            source.validate_and_hold()?
        }
        None => Vec::new(),
    };
    files::plain_path(path)?;
    let mut file = File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)?;
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 65536];
    loop {
        let length = file.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        hash.update(&chunk[..length]);
    }
    let digest = format!("{:x}", hash.finalize());
    let expected = match source {
        Some(_) => "6db593f5aeb1e12b1f4f36fac7fad34c2dfe8f9fcb853bff5fc01928c95c53fa",
        None => CLI_SHA256,
    };
    if !file.metadata()?.is_file() || digest != expected {
        return Err(io::Error::other("Grok 固定 Windows 映像摘要不匹配"));
    }
    file.seek(SeekFrom::Start(0))?;
    Ok((file, source_files))
}
fn native_arguments(manifest: &LaunchManifest) -> Vec<OsString> {
    vec![
        "--leader".into(),
        "--minimal".into(),
        "--no-alt-screen".into(),
        "--cwd".into(),
        manifest.cwd.as_os_str().to_owned(),
        "--leader-socket".into(),
        manifest.socket_path.as_os_str().to_owned(),
        if manifest.history.is_some() { "--resume".into() } else { "--session-id".into() },
        manifest.session_id.to_string().into(),
        "--model".into(),
        MODEL.into(),
    ]
}

fn console_contains(shell: &WindowsProcessLease) -> io::Result<()> {
    shell.validate()?;
    let mut clients = vec![0u32; 16];
    for _ in 0..3 {
        let length = unsafe { GetConsoleProcessList(&mut clients) } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length > 4096 {
            return Err(io::Error::other("ConPTY 进程列表超过上限"));
        }
        if length > clients.len() {
            clients.resize(length, 0);
            continue;
        }
        if !clients[..length].contains(&shell.identity().pid)
            || !clients[..length].contains(&std::process::id())
        {
            return Err(io::Error::other("wrapper 不属于原 ConPTY 控制台"));
        }
        return shell.validate();
    }
    Err(io::Error::other("ConPTY 进程列表变化过快"))
}
unsafe extern "system" fn keep_wrapper_for_native_ctrl_c(control: u32) -> BOOL {
    BOOL::from(control == CTRL_C_EVENT || control == CTRL_BREAK_EVENT)
}
fn attach_owned_console(manifest: &LaunchManifest) -> io::Result<WindowsProcessLease> {
    let shell = WindowsProcessLease::capture(manifest.shell.pid)?;
    if shell.identity() != manifest.shell.into() {
        return Err(io::Error::other("原 ConPTY shell 身份已变化"));
    }
    let own_pid = Pid::from_u32(std::process::id());
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[own_pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    if system.process(own_pid).and_then(|process| process.parent())
        != Some(Pid::from_u32(manifest.shell.pid))
    {
        return Err(io::Error::other(
            "owned Grok wrapper 不是原 ConPTY shell 的直接子进程",
        ));
    }
    let mut clients = [0u32; 1];
    if unsafe { GetConsoleProcessList(&mut clients) } == 0 {
        unsafe { AttachConsole(manifest.shell.pid) }.map_err(io::Error::from)?;
    }
    console_contains(&shell)?;
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        let handle = unsafe { GetStdHandle(kind) }.map_err(io::Error::from)?;
        let mut mode = CONSOLE_MODE::default();
        unsafe { GetConsoleMode(handle, &mut mode) }.map_err(io::Error::from)?;
    }
    // 本 wrapper 只等待/收尾；自定义 handler 不会像全局忽略标志那样让子进程继承忽略 Ctrl-C。
    unsafe { SetConsoleCtrlHandler(Some(keep_wrapper_for_native_ctrl_c), true) }
        .map_err(io::Error::from)?;
    Ok(shell)
}

/// Windows 无 POSIX exec；wrapper 始终保留原始创建句柄和专属 Job，CLI 继承同一控制台。
pub(super) fn exec_owned(path: &Path) -> io::Result<()> {
    let bytes = files::read_private(path)?;
    let manifest: LaunchManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    validate_manifest(path, &manifest)?;
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("清单缺少目录"))?;
    let sha256 = digest(&bytes);
    if std::env::current_exe()?.canonicalize()? != manifest.app_executable
        || dunce::canonicalize(&manifest.cwd)? != manifest.cwd
        || files::read_private(&directory.join("dispatched"))? != sha256.as_bytes()
    {
        return Err(io::Error::other("owned Grok 派发身份改变"));
    }
    let shell = attach_owned_console(&manifest)?;
    let wrapper = WindowsProcessLease::capture(std::process::id())?;
    let receipt = WrapperReceipt {
        version: 1,
        launch_id: manifest.launch_id,
        manifest_sha256: sha256,
        process: wrapper.identity().into(),
    };
    files::write_new(
        &directory.join("wrapper.json"),
        &serde_json::to_vec(&receipt).map_err(io::Error::other)?,
    )?;
    let spawned = (|| {
        if let Some(history) = &manifest.history {
            history.validate_target(&manifest.cwd, manifest.session_id)?;
            history.verify_exited()?;
        }
        let (binary, source_files) =
            verify_binary(&manifest.executable, manifest.npm_source.as_ref())?;
        console_contains(&shell)?;
        let mut environment = vec![(OsStr::new("GROK_DISABLE_AUTOUPDATER"), OsStr::new("1"))];
        if let Some(source) = &manifest.npm_source {
            environment.push((OsStr::new("GROK_MANAGED_BY_NPM"), OsStr::new("1")));
            environment.push((OsStr::new("GROK_HOME"), source.grok_home.as_os_str()));
        }
        let child = OwnedConsoleChild::spawn_suspended(
            &manifest.executable,
            &native_arguments(&manifest),
            &manifest.cwd,
            &environment,
        )?;
        let tui = WindowsProcessLease::from_process_handle(&child)?;
        if tui.image_path().canonicalize()? != manifest.executable {
            return Err(io::Error::other("Grok 原始进程映像不匹配"));
        }
        Ok::<_, io::Error>((child, tui, binary, source_files))
    })();
    let (mut child, tui, _binary, _source_files) = match spawned {
        Ok(result) => result,
        Err(error) => {
            let failure = StartFailure {
                wrapper: receipt,
                no_native_started: true,
                os_error: error.raw_os_error(),
            };
            files::write_new(
                &directory.join("start-failed.json"),
                &serde_json::to_vec(&failure).map_err(io::Error::other)?,
            )?;
            return Err(error);
        }
    };
    let spawn = SpawnReceipt {
        wrapper: receipt,
        tui: tui.identity().into(),
    };
    files::write_new(
        &directory.join("spawn.json"),
        &serde_json::to_vec(&spawn).map_err(io::Error::other)?,
    )?;
    let execution = (|| {
        console_contains(&shell)?;
        child.resume()?;
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        let mut leader: Option<WindowsProcessLease> = None;
        let mut last_lookup = Instant::now() - Duration::from_secs(1);
        loop {
            if let Some(code) = child.root_exit_code()? {
                return Ok::<_, io::Error>(code);
            }
            console_contains(&shell)?;
            if leader.is_none() && last_lookup.elapsed() >= Duration::from_millis(250) {
                last_lookup = Instant::now();
                let _entered = runtime.enter();
                if let Ok(pipe) =
                    ClientOptions::new().open(windows_leader_pipe_name(&manifest.socket_path))
                {
                    let peer = WindowsProcessLease::from_named_pipe_peer(&pipe)?;
                    if !child.owns_process(&peer)?
                        || peer.image_path().canonicalize()? != manifest.executable
                        || peer.identity() == tui.identity()
                    {
                        return Err(io::Error::other("Grok leader 不属于本次专属 Job"));
                    }
                    peer.validate_named_pipe_peer(&pipe)?;
                    let bound = BoundProcesses {
                        spawn: spawn.clone(),
                        leader: peer.identity().into(),
                    };
                    files::write_new(
                        &directory.join("bound.json"),
                        &serde_json::to_vec(&bound).map_err(io::Error::other)?,
                    )?;
                    leader = Some(peer);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    child.terminate_and_confirm(Duration::from_secs(5))?;
    let exit = ExitReceipt {
        spawn,
        job_empty: true,
        exit_code: execution.as_ref().ok().copied(),
    };
    files::write_new(
        &directory.join("exit.json"),
        &serde_json::to_vec(&exit).map_err(io::Error::other)?,
    )?;
    execution.map(|_| ())
}

/// PowerShell 对 GUI 子系统程序必须明确等待，否则外层 shell 会提前显示新提示符。
pub(crate) fn windows_launch_command(argv: &[OsString]) -> io::Result<String> {
    if argv.len() != 3 || argv[1] != OsStr::new(OWNED_GROK_COMMAND) {
        return Err(io::Error::other("owned Grok Windows argv 无效"));
    }
    let program = argv[0]
        .to_str()
        .ok_or_else(|| io::Error::other("应用路径不是有效 Unicode"))?;
    let manifest = argv[2]
        .to_str()
        .ok_or_else(|| io::Error::other("清单路径不是有效 Unicode"))?;
    if !Path::new(program).is_absolute()
        || !Path::new(manifest).is_absolute()
        || manifest.contains('"')
    {
        return Err(io::Error::other("owned Grok Windows 路径无效"));
    }
    let arguments = format!("{OWNED_GROK_COMMAND} \"{manifest}\"");
    Ok(format!(
        "Start-Process -FilePath '{}' -ArgumentList '{}' -NoNewWindow -Wait",
        program.replace('\'', "''"),
        arguments.replace('\'', "''")
    ))
}
