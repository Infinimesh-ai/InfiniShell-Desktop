//! Linux x86_64 固定 Grok 1.0.41 后端；内核身份不满足时拒绝，不回退到裸 PID。

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{
    DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _,
    PermissionsExt as _,
};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use command::blocking::Command;
use command::managed::{
    LinuxProcessHandle, LinuxProcessIdentity, linux_boot_session, linux_peer_identity,
    linux_process_exited, linux_process_identity, linux_signal_owned_process,
    linux_verify_identity_support,
};
use command::unix::CommandExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use warpui::{AppContext, EntityId, SingletonEntity};

use super::*;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::GrokPermissionObservation;
use crate::terminal::cli_agent_sessions::grok_leader_input::{
    GrokLeaderInputError, GrokLeaderTarget,
};
use crate::terminal::cli_agent_updates::CliAgentUpdatesModel;
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

const CLI_SHA256: &str = "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407";
const CLI_VERSION_OUTPUT: &str = "grok 1.0.41 (4220f3b224a6)";
const MODEL: &str = "grok-4.7";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessIdentity {
    pid: i32,
    start_time_ticks: u64,
    proc_inode: u64,
    pid_namespace_device: u64,
    pid_namespace_inode: u64,
    uid: u32,
    executable_device: u64,
    executable_inode: u64,
}

impl From<LinuxProcessIdentity> for ProcessIdentity {
    fn from(value: LinuxProcessIdentity) -> Self {
        Self {
            pid: value.pid,
            start_time_ticks: value.start_time_ticks,
            proc_inode: value.proc_inode,
            pid_namespace_device: value.pid_namespace_device,
            pid_namespace_inode: value.pid_namespace_inode,
            uid: value.uid,
            executable_device: value.executable_device,
            executable_inode: value.executable_inode,
        }
    }
}

/// slave 必须来自当前本地活动 PTY，不能从 shell PID 猜测终端设备。
pub(crate) struct GrokOwnedPty {
    shell: ProcessIdentity,
    device: u64,
}

impl GrokOwnedPty {
    pub(crate) fn from_local_pty(identity: LocalPtyIdentity) -> io::Result<Self> {
        Ok(Self {
            shell: identity.current_shell()?.into(),
            device: identity.slave_device(),
        })
    }

    pub(crate) fn from_current_terminal() -> io::Result<Self> {
        let (parent, device) = current_remote_terminal()?;
        let handle = LinuxProcessHandle::capture(parent)?;
        let shell = handle.snapshot()?;
        if shell.identity.uid != unsafe { libc::geteuid() }
            || shell.tty_device != device
            || shell.session != unsafe { libc::getsid(0) }
            || handle.snapshot()?.identity != shell.identity
            || current_remote_terminal()? != (parent, device)
        {
            return Err(io::Error::other("远端 Grok 父 shell 与控制终端不匹配"));
        }
        Ok(Self {
            shell: shell.identity.into(),
            device,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchManifest {
    version: u32,
    launch_id: Uuid,
    session_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history: Option<HistorySource>,
    executable: PathBuf,
    app_executable: PathBuf,
    cwd: PathBuf,
    socket_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    socket_directory: Option<SocketDirectoryIdentity>,
    boot_session: String,
    shell: ProcessIdentity,
    tty_device: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SocketDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecReceipt {
    version: u32,
    launch_id: Uuid,
    manifest_sha256: String,
    process: ProcessIdentity,
    process_group: i32,
    tty_device: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundProcesses {
    version: u32,
    launch_id: Uuid,
    manifest_sha256: String,
    tui: ProcessIdentity,
    leader: ProcessIdentity,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecFailure {
    receipt: ExecReceipt,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchPhase {
    Prepared,
    Reserved,
    Dispatched,
    RecoveredUnsent,
    Released,
}

/// 保留目录至拥有的 TUI 和 leader 真实退出；侧车断开不能释放更新占用或删除 socket。
pub(crate) struct GrokOwnedLaunch {
    directory: PathBuf,
    manifest: LaunchManifest,
    manifest_sha256: String,
    reservation: Option<EntityId>,
    phase: LaunchPhase,
    processes: Option<(LinuxProcessIdentity, LinuxProcessIdentity)>,
}

impl GrokOwnedLaunch {
    /// 只重建已知清单，不派生或派发；未派发的旧清单也不能重新领取启动。
    pub(crate) fn restore(path: &Path, expected_sha256: &str) -> io::Result<Self> {
        let bytes = read_private(path)?;
        if digest(&bytes) != expected_sha256 {
            return Err(io::Error::other("owned Grok 启动清单摘要已变化"));
        }
        let manifest: LaunchManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("启动清单缺少目录"))?;
        validate_manifest(path, &manifest)?;
        let phase = match read_optional_private(&directory.join("dispatched"))? {
            Some(record) if record == expected_sha256.as_bytes() => LaunchPhase::Dispatched,
            Some(_) => return Err(io::Error::other("owned Grok 派发记录不匹配")),
            None => LaunchPhase::RecoveredUnsent,
        };
        let mut launch = Self {
            directory: directory.to_owned(),
            manifest,
            manifest_sha256: expected_sha256.to_owned(),
            reservation: None,
            phase,
            processes: None,
        };
        if let Some(bytes) = read_optional_private(&directory.join("bound.json"))? {
            if phase != LaunchPhase::Dispatched {
                return Err(io::Error::other("未派发清单出现进程绑定"));
            }
            let bound: BoundProcesses = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            let receipt = launch.exec_receipt()?;
            validate_bound_processes(&bound, &receipt)?;
            launch.processes = Some((kernel_identity(&bound.tui), kernel_identity(&bound.leader)));
        }
        if let Some(bytes) = read_optional_private(&directory.join("retired.json"))? {
            let retired: RetiredLaunch =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if retired.version != 1
                || retired.launch_id != launch.launch_id()
                || retired.manifest_sha256 != expected_sha256
                || retired.dispatched != (phase == LaunchPhase::Dispatched)
            {
                return Err(io::Error::other("owned Grok 退出归档与启动不匹配"));
            }
            // 归档仍保留原生身份；损坏或尚存活的进程不能借此放行升级。
            if retired.dispatched {
                launch.confirm_exit()?;
            } else {
                launch.confirm_not_dispatched()?;
            }
            launch.phase = LaunchPhase::Released;
        }
        Ok(launch)
    }

    /// 在后台线程准备，不持有 TerminalModel 锁；不发送用户输入，不复制认证材料。
    pub(crate) fn prepare(
        executable: &Path,
        app_executable: &Path,
        cwd: &Path,
        state_directory: &Path,
        pty: GrokOwnedPty,
    ) -> io::Result<Self> {
        linux_verify_identity_support()?;
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
        linux_verify_identity_support()?;
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
        linux_verify_identity_support()?;
        let executable = executable.canonicalize()?;
        let binary = verify_binary(&executable)?;
        // 版本探针也执行刚核验的描述符，避免路径替换后先执行再发现摘要改变。
        let output = Command::new(format!("/proc/self/fd/{}", binary.as_raw_fd()))
            .arg0(&executable)
            .arg("--version")
            .env("GROK_DISABLE_AUTOUPDATER", "1")
            .output()?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stdout).trim() != CLI_VERSION_OUTPUT
        {
            return Err(io::Error::other("Grok 固定版本不匹配"));
        }
        verify_binary(&executable)?;
        let cwd = cwd.canonicalize()?;
        if !cwd.is_dir() {
            return Err(io::Error::other("Grok 工作目录无效"));
        }
        let (directory, socket_directory) = create_launch_directories(state_directory)?;
        let root = directory.path().canonicalize()?;
        let socket_root = socket_directory.path().canonicalize()?;
        let socket_metadata = fs::symlink_metadata(&socket_root)?;
        let manifest = LaunchManifest {
            version: 3,
            launch_id: Uuid::new_v4(),
            session_id: history.as_ref().map_or_else(Uuid::new_v4, |source| source.session_id),
            history,
            executable,
            app_executable: app_executable.canonicalize()?,
            cwd,
            socket_path: socket_root.join("leader.sock"),
            socket_directory: Some(SocketDirectoryIdentity {
                device: socket_metadata.dev(),
                inode: socket_metadata.ino(),
            }),
            boot_session: linux_boot_session()?,
            shell: pty.shell,
            tty_device: pty.device,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
        write_new(&root.join("launch.json"), &bytes)?;
        let _ = directory.keep();
        let _ = socket_directory.keep();
        Ok(Self {
            directory: root,
            manifest,
            manifest_sha256: digest(&bytes),
            reservation: None,
            phase: LaunchPhase::Prepared,
            processes: None,
        })
    }

    /// 独立 reservation 不被普通 shell Preexec 的 release_launch(view_id) 提前释放。
    pub(crate) fn reserve_launch(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase != LaunchPhase::Prepared {
            return Err(io::Error::other("owned Grok 启动已领取"));
        }
        self.reserve(ctx)?;
        self.phase = LaunchPhase::Reserved;
        Ok(())
    }

    /// 恢复占用必须先于自动升级；这条路径永远不能产生可再次派发的 Reserved 状态。
    pub(crate) fn reserve_for_recovery(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched || self.reservation.is_some() {
            return Err(io::Error::other("owned Grok 恢复占用状态无效"));
        }
        self.reserve(ctx)
    }

    fn reserve(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        let reservation = EntityId::new();
        let reserved = CliAgentUpdatesModel::handle(ctx).update(ctx, |model, ctx| {
            model.reserve_launch(CLIAgent::Grok, reservation, ctx)
        });
        if !reserved {
            return Err(io::Error::other("Grok 正在升级，启动未派发"));
        }
        self.reservation = Some(reservation);
        Ok(())
    }

    /// 仅供未编辑的显式 owned 启动意图；返回 argv，禁止拼接到用户自定义 grok 命令。
    /// 调用方按当前已验证 shell 原生转义后交给同一 PTY；返回一次后不能自动重派。
    pub(crate) fn take_launch_argv(&mut self) -> io::Result<Vec<OsString>> {
        if self.phase != LaunchPhase::Reserved {
            return Err(io::Error::other("owned Grok 启动尚未占用或已派发"));
        }
        verify_binary(&self.manifest.executable)?;
        write_new(
            &self.directory.join("dispatched"),
            self.manifest_sha256.as_bytes(),
        )?;
        self.phase = LaunchPhase::Dispatched;
        Ok(vec![
            self.manifest.app_executable.as_os_str().to_owned(),
            OWNED_GROK_COMMAND.into(),
            self.directory.join("launch.json").into_os_string(),
        ])
    }

    /// 调用方先持有远端 ticket 排他锁，并持久化本清单及 Unknown 派发记录。
    /// 远端占用独立于本机更新模型；恢复对象永远不能取得此一次派发资格。
    pub(crate) fn dispatch_remote_reserved(&mut self) -> io::Result<PathBuf> {
        if self.phase != LaunchPhase::Prepared || self.reservation.is_some() {
            return Err(io::Error::other("远端 Grok 启动已领取或属于本地占用"));
        }
        verify_binary(&self.manifest.executable)?;
        write_new(
            &self.directory.join("dispatched"),
            self.manifest_sha256.as_bytes(),
        )?;
        self.phase = LaunchPhase::Dispatched;
        Ok(self.manifest_path())
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

    fn exec_receipt(&self) -> io::Result<ExecReceipt> {
        let receipt: ExecReceipt =
            serde_json::from_slice(&read_private(&self.directory.join("exec.json"))?)
                .map_err(io::Error::other)?;
        if receipt.version != 1
            || receipt.launch_id != self.manifest.launch_id
            || receipt.manifest_sha256 != self.manifest_sha256
            || receipt.tty_device != self.manifest.tty_device
            || receipt.process.pid <= 0
            || receipt.process.pid == self.manifest.shell.pid
            || receipt.process.start_time_ticks == 0
            || receipt.process.pid_namespace_inode == 0
            || receipt.process_group <= 0
        {
            return Err(io::Error::other("owned Grok exec 记录不匹配"));
        }
        Ok(receipt)
    }

    /// 新 SessionStart 权限证据、输入代际或活动 PTY 改变时，调用方须废弃旧 binding。
    pub(crate) fn bind(
        &mut self,
        binding_id: Uuid,
        permission_revision: Uuid,
        observation: &GrokPermissionObservation,
    ) -> Result<GrokOwnedBinding, GrokLeaderInputError> {
        if self.phase != LaunchPhase::Dispatched
            || binding_id.is_nil()
            || permission_revision.is_nil()
            || observation.session_id != self.manifest.session_id.to_string()
            || observation.cwd != self.manifest.cwd.to_string_lossy()
            || observation.mode != "default"
            || observation.session_start_event_id.is_empty()
        {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        let receipt = self.exec_receipt()?;
        let tui_handle = LinuxProcessHandle::capture(receipt.process.pid)?;
        let snapshot = tui_handle.snapshot()?;
        let actual = snapshot.identity;
        if receipt.version != 1
            || receipt.launch_id != self.manifest.launch_id
            || receipt.manifest_sha256 != self.manifest_sha256
            || receipt.tty_device != self.manifest.tty_device
            || receipt.process.pid == self.manifest.shell.pid
            || !exec_identity_matches(&receipt.process, actual)
            || snapshot.process_group != receipt.process_group
            || snapshot.tty_device != receipt.tty_device
            || linux_boot_session()? != self.manifest.boot_session
        {
            return Err(GrokLeaderInputError::IdentityChanged);
        }
        // PID 来自本次入口的真实 exec 进程；leader 来自本次私有 socket 的内核对端。
        validate_socket_directory(&self.manifest)?;
        let peer = UnixStream::connect(&self.manifest.socket_path)?;
        let leader = linux_peer_identity(&peer)?;
        drop(peer);
        let target = GrokLeaderTarget::capture(
            binding_id,
            self.manifest.session_id,
            &self.manifest.cwd,
            &self.manifest.socket_path,
            &self.manifest.executable,
            actual.pid,
            leader.pid,
            &observation.mode,
        )?;
        target.verify_owned(actual, leader, receipt.tty_device)?;
        let bound = BoundProcesses {
            version: 1,
            launch_id: self.manifest.launch_id,
            manifest_sha256: self.manifest_sha256.clone(),
            tui: actual.into(),
            leader: leader.into(),
        };
        validate_bound_processes(&bound, &receipt)?;
        self.record_owned_processes(&bound, &receipt)?;
        self.processes = Some((actual, leader));
        Ok(GrokOwnedBinding {
            target,
            launch_id: self.manifest.launch_id,
            binding_id,
            session_id: self.manifest.session_id,
            permission_revision,
            working_directory: self.manifest.cwd.clone(),
        })
    }

    fn record_owned_processes(
        &self,
        bound: &BoundProcesses,
        receipt: &ExecReceipt,
    ) -> io::Result<()> {
        validate_bound_processes(bound, receipt)?;
        let path = self.directory.join("bound.json");
        match write_new(&path, &serde_json::to_vec(bound).map_err(io::Error::other)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // 输入绑定和只读生命周期捕获可以并发，只接受完全相同的首次落盘身份。
                let previous: BoundProcesses =
                    serde_json::from_slice(&read_private(&path)?).map_err(io::Error::other)?;
                if previous == *bound {
                    Ok(())
                } else {
                    Err(io::Error::other("owned Grok 进程身份记录已变化"))
                }
            }
            Err(error) => Err(error),
        }
    }

    /// 只保存本次 exec 与私有 socket 的内核身份，不构造输入 target、不授权或合成事件。
    /// 在后台执行；插件缺失也要独立记录原生进程，供退出清理和恢复核对。
    pub(crate) fn capture_owned_processes(&mut self) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched
            || linux_boot_session()? != self.manifest.boot_session
        {
            return Err(io::Error::other("owned Grok 生命周期捕获状态无效"));
        }
        self.refresh_bound_processes()?;
        if self.processes.is_some() {
            return Ok(());
        }
        let receipt = self.exec_receipt()?;
        let tui_handle = LinuxProcessHandle::capture(receipt.process.pid)?;
        let snapshot = tui_handle.snapshot()?;
        let tui = snapshot.identity;
        if !exec_identity_matches(&receipt.process, tui)
            || snapshot.process_group != receipt.process_group
            || snapshot.tty_device != receipt.tty_device
        {
            return Err(io::Error::other("owned Grok exec 身份已经改变"));
        }
        validate_socket_directory(&self.manifest)?;
        let socket = fs::symlink_metadata(&self.manifest.socket_path)?;
        if !socket.file_type().is_socket()
            || socket.uid() != unsafe { libc::geteuid() }
            || socket.mode() & 0o022 != 0
        {
            return Err(io::Error::other("owned Grok 生命周期 socket 无效"));
        }
        // 只建立本机 socket 并读取内核凭据，不发送原生协议帧或模型输入。
        let peer = UnixStream::connect(&self.manifest.socket_path)?;
        let leader = linux_peer_identity(&peer)?;
        let leader_handle = LinuxProcessHandle::capture(leader.pid)?;
        let actual = tui_handle.snapshot()?;
        let leader_snapshot = leader_handle.snapshot()?;
        if actual.identity != tui
            || leader_snapshot.identity != leader
            || actual.executable != self.manifest.executable
            || leader_snapshot.executable != self.manifest.executable
            || actual.arguments.get(1..) != Some(native_arguments(&self.manifest).as_slice())
            || actual.tty_device != self.manifest.tty_device
            || actual.process_group != receipt.process_group
        {
            return Err(io::Error::other("owned Grok Linux 映像、参数或 PTY 不匹配"));
        }
        let socket_after = fs::symlink_metadata(&self.manifest.socket_path)?;
        if linux_process_identity(tui.pid)? != tui
            || linux_process_identity(leader.pid)? != leader
            || socket.dev() != socket_after.dev()
            || socket.ino() != socket_after.ino()
        {
            return Err(io::Error::other("owned Grok 生命周期捕获期间身份改变"));
        }
        let bound = BoundProcesses {
            version: 1,
            launch_id: self.launch_id(),
            manifest_sha256: self.manifest_sha256.clone(),
            tui: tui.into(),
            leader: leader.into(),
        };
        self.record_owned_processes(&bound, &receipt)?;
        self.processes = Some((tui, leader));
        Ok(())
    }

    /// 原生 TUI 已退出才请求停止此启动独有的 leader；侧车断开不能触发停止。
    /// 发出信号不是退出证明；下一次 reaper 仍须分别核对真实进程身份。
    pub(crate) fn stop_leader_after_tui_exit(&self) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched
            || linux_boot_session()? != self.manifest.boot_session
        {
            return Ok(());
        }
        let Some((tui, leader)) = self.processes else {
            return Err(io::Error::other(
                "owned Grok 尚未捕获进程，不能猜测退出目标",
            ));
        };
        if identity_exited(tui) && !identity_exited(leader) {
            linux_signal_owned_process(leader, &self.manifest.boot_session, libc::SIGTERM)?;
        }
        Ok(())
    }

    /// 后台 GUI 绑定完成后，原全局占用对象也读取同一清单下的已验证身份。
    pub(crate) fn refresh_bound_processes(&mut self) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched || self.processes.is_some() {
            return Ok(());
        }
        let path = self.directory.join("bound.json");
        if let Some(bytes) = read_optional_private(&path)? {
            let bound: BoundProcesses = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            validate_bound_processes(&bound, &self.exec_receipt()?)?;
            self.processes = Some((kernel_identity(&bound.tui), kernel_identity(&bound.leader)));
        }
        Ok(())
    }

    pub(crate) fn is_retired(&self) -> bool {
        self.phase == LaunchPhase::Released
    }

    pub(crate) fn was_dispatched(&self) -> bool {
        self.phase == LaunchPhase::Dispatched
    }

    pub(crate) fn confirm_history_exit(&mut self) -> io::Result<()> {
        if self.phase == LaunchPhase::Released && read_optional_private(&self.directory.join("dispatched"))?.is_none() { return self.confirm_not_dispatched(); }
        let previous_phase = self.phase;
        if self.phase == LaunchPhase::Released { self.phase = LaunchPhase::Dispatched; }
        let refreshed = self.refresh_bound_processes();
        self.phase = previous_phase;
        refreshed?;
        self.confirm_exit()
    }

    pub(crate) fn matches_history_pty(&self, pty: &GrokOwnedPty) -> io::Result<bool> {
        Ok(self.manifest.shell == pty.shell && self.manifest.tty_device == pty.device)
    }

    pub(crate) fn working_directory(&self) -> &Path {
        &self.manifest.cwd
    }

    fn confirm_exit(&self) -> io::Result<()> {
        if linux_boot_session()? == self.manifest.boot_session {
            if let Some((tui, leader)) = self.processes {
                if !identity_exited(tui) || !identity_exited(leader) {
                    return Err(io::Error::other("owned Grok 原生进程仍在运行或退出不明"));
                }
            } else {
                // 只有 exec 明确返回失败才能证明未派生原生 leader；socket 消失本身不是退出证据。
                let failure: ExecFailure = serde_json::from_slice(&read_private(
                    &self.directory.join("exec-failed.json"),
                )?)
                .map_err(io::Error::other)?;
                let receipt = self.exec_receipt()?;
                let socket_absent = matches!(
                    fs::symlink_metadata(&self.manifest.socket_path),
                    Err(error) if error.kind() == io::ErrorKind::NotFound
                );
                if failure.receipt != receipt
                    || !identity_exited(kernel_identity(&receipt.process))
                    || !socket_absent
                {
                    return Err(io::Error::other("owned Grok exec 失败后的退出尚未确认"));
                }
            }
        }
        Ok(())
    }

    fn confirm_not_dispatched(&self) -> io::Result<()> {
        for name in [
            "dispatched",
            "exec.json",
            "exec-failed.json",
            "bound.json",
            "leader.sock",
        ] {
            if !matches!(fs::symlink_metadata(self.directory.join(name)),
                Err(error) if error.kind() == io::ErrorKind::NotFound)
            {
                return Err(io::Error::other("owned Grok 存在派发或原生执行证据"));
            }
        }
        if !matches!(fs::symlink_metadata(&self.manifest.socket_path),
            Err(error) if error.kind() == io::ErrorKind::NotFound)
        {
            return Err(io::Error::other("owned Grok 原生 socket 存在或无法核对"));
        }
        Ok(())
    }

    fn retire(&mut self, dispatched: bool, ctx: &mut AppContext) -> io::Result<()> {
        self.record_retired(dispatched)?;
        if let Some(reservation) = self.reservation.take() {
            CliAgentUpdatesModel::handle(ctx)
                .update(ctx, |model, ctx| model.release_launch(reservation, ctx));
        }
        self.phase = LaunchPhase::Released;
        Ok(())
    }

    fn record_retired(&self, dispatched: bool) -> io::Result<()> {
        // 先保留可恢复的退出收据，再释放占用。任务历史仍引用清单，不能直接删目录。
        let receipt = RetiredLaunch {
            version: 1,
            launch_id: self.launch_id(),
            manifest_sha256: self.manifest_sha256.clone(),
            dispatched,
        };
        cleanup_socket_directory(&self.manifest, self.processes.map(|(_, leader)| leader))?;
        write_new(
            &self.directory.join("retired.json"),
            &serde_json::to_vec(&receipt).map_err(io::Error::other)?,
        )?;
        Ok(())
    }

    /// 仅后台核对远端占用；断连、socket 消失或无法读取进程均不代表已退出。
    pub(crate) fn release_remote_after_exit(&mut self) -> io::Result<()> {
        if self.reservation.is_some() {
            return Err(io::Error::other("不能用远端清理接口释放本地启动占用"));
        }
        if self.phase == LaunchPhase::Released {
            self.confirm_exit()?;
            return Ok(());
        }
        if self.phase != LaunchPhase::Dispatched {
            return Err(io::Error::other("远端 Grok 尚未派发"));
        }
        self.refresh_bound_processes()?;
        if self.processes.is_none()
            && linux_boot_session()? == self.manifest.boot_session
            && read_optional_private(&self.directory.join("exec-failed.json"))?.is_none()
        {
            self.capture_owned_processes()?;
        }
        self.confirm_exit()?;
        self.record_retired(true)?;
        self.phase = LaunchPhase::Released;
        Ok(())
    }

    /// 只释放本次已核对的 TUI 与 leader 都消失后的启动占用；不发送任何终止信号。
    pub(crate) fn release_after_exit(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase != LaunchPhase::Dispatched {
            return Err(io::Error::other("owned Grok 尚未派发或已释放"));
        }
        self.confirm_exit()?;
        self.retire(true, ctx)
    }

    /// 尚未派发才可撤销；已派发的更新占用必须等真实 TUI/leader 退出证据再释放。
    pub(crate) fn cancel_before_dispatch(&mut self, ctx: &mut AppContext) -> io::Result<()> {
        if self.phase == LaunchPhase::Released {
            return Ok(());
        }
        if self.phase == LaunchPhase::Dispatched {
            return Err(io::Error::other("owned Grok 已派发，退出尚未确认"));
        }
        self.confirm_not_dispatched()?;
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

fn exec_identity_matches(before: &ProcessIdentity, after: LinuxProcessIdentity) -> bool {
    // Linux 没有 macOS 的 exec 代数；生存期相同且映像已换成固定 CLI 才能继续。
    let before = kernel_identity(before);
    before.same_lifetime(after)
        && before.uid == after.uid
        && (before.executable_device, before.executable_inode)
            != (after.executable_device, after.executable_inode)
}

fn identity_exited(expected: LinuxProcessIdentity) -> bool {
    // 内核接口错误不能当作退出证明，也不能改用 kill(pid, 0)。
    linux_process_exited(expected).unwrap_or(false)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn kernel_identity(value: &ProcessIdentity) -> LinuxProcessIdentity {
    LinuxProcessIdentity {
        pid: value.pid,
        start_time_ticks: value.start_time_ticks,
        proc_inode: value.proc_inode,
        pid_namespace_device: value.pid_namespace_device,
        pid_namespace_inode: value.pid_namespace_inode,
        uid: value.uid,
        executable_device: value.executable_device,
        executable_inode: value.executable_inode,
    }
}

fn validate_bound_processes(bound: &BoundProcesses, receipt: &ExecReceipt) -> io::Result<()> {
    if bound.version != 1
        || bound.launch_id != receipt.launch_id
        || bound.manifest_sha256 != receipt.manifest_sha256
        || !exec_identity_matches(&receipt.process, kernel_identity(&bound.tui))
        || bound.leader.pid <= 0
        || bound.leader.pid == bound.tui.pid
        || bound.leader.start_time_ticks == 0
        || bound.leader.pid_namespace_inode == 0
    {
        return Err(io::Error::other("owned Grok 进程绑定与 exec 记录不匹配"));
    }
    Ok(())
}

fn validate_manifest(path: &Path, manifest: &LaunchManifest) -> io::Result<()> {
    if path.file_name() != Some(std::ffi::OsStr::new("launch.json"))
        || manifest.launch_id.is_nil()
        || manifest.session_id.is_nil()
        || Uuid::parse_str(&manifest.boot_session).is_err()
        || manifest.shell.pid <= 0
        || manifest.shell.start_time_ticks == 0
        || manifest.shell.pid_namespace_inode == 0
        || !manifest.executable.is_absolute()
        || !manifest.app_executable.is_absolute()
        || !manifest.cwd.is_absolute()
    {
        return Err(io::Error::other("owned Grok 启动清单无效"));
    }
    match (manifest.version, &manifest.socket_directory) {
        (3, Some(identity))
            if identity.inode != 0
                && manifest.socket_path.file_name()
                    == Some(std::ffi::OsStr::new("leader.sock"))
                && manifest.socket_path.parent().is_some_and(|root| {
                    root.parent() == Some(Path::new("/tmp"))
                        && root
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with("isp-grok-owned-socket-"))
                }) =>
        {
            Ok(())
        }
        // Linux 只接受独立版本清单；不能把 macOS 或未知版本的身份记录用于启动。
        _ => Err(io::Error::other("owned Grok socket 清单版本或路径无效")),
    }
}

fn create_launch_directories(
    state_directory: &Path,
) -> io::Result<(tempfile::TempDir, tempfile::TempDir)> {
    let state = fs::symlink_metadata(state_directory)?;
    if state_directory.canonicalize()? != state_directory
        || !state.is_dir()
        || state.uid() != unsafe { libc::geteuid() }
        || state.mode() & 0o022 != 0
    {
        return Err(io::Error::other("owned Grok 恢复目录不可信"));
    }
    let parent = state_directory.join("grok-owned-terminal");
    match fs::DirBuilder::new().mode(0o700).create(&parent) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_private_directory(&parent)?;
    let directory = tempfile::Builder::new()
        .prefix("launch-")
        .tempdir_in(&parent)?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let socket = tempfile::Builder::new()
        .prefix("isp-grok-owned-socket-")
        .tempdir_in("/tmp")?;
    fs::set_permissions(socket.path(), fs::Permissions::from_mode(0o700))?;
    // 清单留在数据库数据域；socket 用短路径，避免用户目录或 profile 名超过 sockaddr_un 限制。
    Ok((directory, socket))
}

fn validate_private_directory(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if path.canonicalize()? != path
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("owned Grok 私有目录被替换或权限无效"));
    }
    Ok(metadata)
}

fn validate_socket_directory(manifest: &LaunchManifest) -> io::Result<()> {
    if let Some(identity) = &manifest.socket_directory {
        let root = manifest
            .socket_path
            .parent()
            .ok_or_else(|| io::Error::other("socket 缺少目录"))?;
        let metadata = validate_private_directory(root)?;
        if metadata.dev() != identity.device || metadata.ino() != identity.inode {
            return Err(io::Error::other("owned Grok socket 目录身份已变化"));
        }
    }
    Ok(())
}

fn cleanup_socket_directory(
    manifest: &LaunchManifest,
    leader: Option<LinuxProcessIdentity>,
) -> io::Result<()> {
    // 旧清单的 socket 与恢复记录同目录，不删除旧恢复目录。
    if manifest.socket_directory.is_none() {
        return Ok(());
    }
    let root = manifest
        .socket_path
        .parent()
        .ok_or_else(|| io::Error::other("socket 缺少目录"))?;
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    validate_socket_directory(manifest)?;
    let lock = root.join("leader.lock");
    if let Some(bytes) = read_optional_private(&lock)? {
        // 固定 1.0.41 留下只含 leader PID 的私有锁文件。只有已绑定进程退出，且
        // 原始字节与该 leader 完全一致时才清理；文件名或可复用的 PID 单独均不够。
        let expected = leader.ok_or_else(|| io::Error::other("缺少原生 leader 锁归属"))?;
        if !identity_exited(expected)
            || bytes != expected.pid.to_string().as_bytes()
            || fs::symlink_metadata(&lock)?.nlink() != 1
        {
            return Err(io::Error::other("原生 leader 锁归属或退出状态不匹配"));
        }
        fs::remove_file(&lock)?;
    }
    match fs::symlink_metadata(&manifest.socket_path) {
        Ok(metadata)
            if metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() } =>
        {
            fs::remove_file(&manifest.socket_path)?
        }
        Ok(_) => return Err(io::Error::other("owned Grok socket 被非原生文件替换")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // 不递归删除；出现未知文件时保留现场并返回清理失败。
    fs::remove_dir(root)
}

fn verify_binary(path: &Path) -> io::Result<File> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let length = file.read(&mut bytes)?;
        if length == 0 {
            break;
        }
        hash.update(&bytes[..length]);
    }
    if format!("{:x}", hash.finalize()) != CLI_SHA256 {
        return Err(io::Error::other("Grok 可执行文件摘要不匹配"));
    }
    Ok(file)
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    File::open(
        path.parent()
            .ok_or_else(|| io::Error::other("私有记录缺少目录"))?,
    )?
    .sync_all()
}

fn read_optional_private(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_private(path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("私有记录缺少目录"))?;
    let directory = fs::symlink_metadata(parent)?;
    let metadata = fs::symlink_metadata(path)?;
    let uid = unsafe { libc::geteuid() };
    if parent.canonicalize()? != parent
        || !directory.is_dir()
        || directory.uid() != uid
        || directory.mode() & 0o077 != 0
        || !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 65536
    {
        return Err(io::Error::other("owned Grok 私有记录权限或类型无效"));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let opened = file.metadata()?;
    if opened.dev() != metadata.dev()
        || opened.ino() != metadata.ino()
        || opened.uid() != uid
        || opened.mode() & 0o077 != 0
    {
        return Err(io::Error::other("owned Grok 私有记录已替换"));
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(io::Error::other("owned Grok 私有记录过大"));
    }
    Ok(bytes)
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

pub(super) fn exec_owned(path: &Path) -> io::Result<()> {
    let bytes = read_private(path)?;
    let manifest: LaunchManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("启动清单缺少目录"))?;
    validate_manifest(path, &manifest)?;
    validate_socket_directory(&manifest)?;
    if manifest.cwd.canonicalize()? != manifest.cwd
        || std::env::current_exe()?.canonicalize()? != manifest.app_executable
        || linux_boot_session()? != manifest.boot_session
        || unsafe { libc::getppid() } != manifest.shell.pid
        || ProcessIdentity::from(linux_process_identity(manifest.shell.pid)?) != manifest.shell
        || read_private(&directory.join("dispatched"))? != digest(&bytes).as_bytes()
    {
        return Err(io::Error::other("owned Grok 启动身份已变化"));
    }
    for descriptor in [0, 1, 2] {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::isatty(descriptor) } != 1
            || unsafe { libc::fstat(descriptor, stat.as_mut_ptr()) } != 0
            || unsafe { stat.assume_init() }.st_rdev as u64 != manifest.tty_device
        {
            return Err(io::Error::other("owned Grok 必须继承同一个原生 PTY"));
        }
    }
    let process_group = unsafe { libc::getpgrp() };
    if process_group <= 0 || unsafe { libc::tcgetpgrp(0) } != process_group {
        return Err(io::Error::other("owned Grok 未拥有前台 PTY 作业"));
    }
        if let Some(history) = &manifest.history {
            history.validate_target(&manifest.cwd, manifest.session_id)?;
            history.verify_exited()?;
        }
    let executable = verify_binary(&manifest.executable)?;
    let receipt = ExecReceipt {
        version: 1,
        launch_id: manifest.launch_id,
        manifest_sha256: digest(&bytes),
        process: linux_process_identity(std::process::id() as i32)?.into(),
        process_group,
        tty_device: manifest.tty_device,
    };
    write_new(
        &directory.join("exec.json"),
        &serde_json::to_vec(&receipt).map_err(io::Error::other)?,
    )?;
    // 不创建新进程或新进程组，不改认证和用户配置；exec 后原生 TUI 接收 Ctrl-C/Ctrl-Z。
    let error = Command::new(format!("/proc/self/fd/{}", executable.as_raw_fd()))
        .arg0(&manifest.executable)
        .args(native_arguments(&manifest))
        .current_dir(&manifest.cwd)
        .env("GROK_DISABLE_AUTOUPDATER", "1")
        .exec();
    // exec 成功不会返回；只保存 OS 错误码，不把环境或原生错误正文写入恢复记录。
    let failure = ExecFailure {
        receipt,
        os_error: error.raw_os_error(),
    };
    write_new(
        &directory.join("exec-failed.json"),
        &serde_json::to_vec(&failure).map_err(io::Error::other)?,
    )?;
    Err(error)
}
