//! 应用明确发起的固定 Grok 普通 TUI 启动；通过 exec 继承原 PTY，不接管原生审批。
//! 仅 macOS arm64 已有原生合同。其他平台不启动，不能据此宣称跨平台验收完成。

use std::io;

pub const OWNED_GROK_COMMAND: &str = "--infinishell-owned-grok-tui";

/// 必须在 GUI、线程池和信号处理器初始化前调用；入口只接受单个私有 manifest 路径。
pub fn run_owned_grok_from_args() -> Option<io::Result<()>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(OWNED_GROK_COMMAND)) {
        return None;
    }
    let Some(path) = arguments.next() else {
        return Some(Err(io::Error::other("缺少 owned Grok 启动清单")));
    };
    if arguments.next().is_some() {
        return Some(Err(io::Error::other("owned Grok 启动不接受额外参数")));
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some(native::exec_owned(std::path::Path::new(&path)));
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = path;
        Some(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "当前平台尚未验证 owned Grok 普通终端",
        )))
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) use native::{GrokOwnedBinding, GrokOwnedLaunch, GrokOwnedPty};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[path = "."]
mod native {
    use std::ffi::OsString;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::{
        FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
    };
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    use command::blocking::Command;
    use command::managed::{
        MacosProcessIdentity, macos_boot_session, macos_peer_identity, macos_process_identity,
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

    const CLI_SHA256: &str = "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d";
    const CLI_VERSION_OUTPUT: &str = "grok 1.0.41 (4220f3b224a6)";
    const MODEL: &str = "grok-4.7";

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ProcessIdentity {
        pid: i32,
        pid_version: u32,
        unique_id: u64,
        resource_cid: u64,
    }

    impl From<MacosProcessIdentity> for ProcessIdentity {
        fn from(value: MacosProcessIdentity) -> Self {
            Self {
                pid: value.pid,
                pid_version: value.pid_version,
                unique_id: value.unique_id,
                resource_cid: value.resource_cid,
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

        #[cfg(test)]
        pub(crate) fn capture(shell_pid: i32, slave: &Path) -> io::Result<Self> {
            let metadata = fs::metadata(slave)?;
            if !metadata.file_type().is_char_device() || shell_pid <= 0 {
                return Err(io::Error::other("owned Grok 缺少真实本地 PTY"));
            }
            Ok(Self {
                shell: macos_process_identity(shell_pid)?.into(),
                device: metadata.rdev(),
            })
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LaunchManifest {
        version: u32,
        launch_id: Uuid,
        session_id: Uuid,
        executable: PathBuf,
        app_executable: PathBuf,
        cwd: PathBuf,
        socket_path: PathBuf,
        boot_session: String,
        shell: ProcessIdentity,
        tty_device: u64,
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
        processes: Option<(MacosProcessIdentity, MacosProcessIdentity)>,
    }

    impl GrokOwnedLaunch {
        /// 只重建已知清单，不派生或派发；未派发的旧清单也不能重新领取启动。
        pub(crate) fn restore(path: &Path, expected_sha256: &str) -> io::Result<Self> {
            let bytes = read_private(path)?;
            if digest(&bytes) != expected_sha256 {
                return Err(io::Error::other("owned Grok 启动清单摘要已变化"));
            }
            let manifest: LaunchManifest =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
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
                let bound: BoundProcesses =
                    serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                let receipt = launch.exec_receipt()?;
                validate_bound_processes(&bound, &receipt)?;
                launch.processes =
                    Some((kernel_identity(&bound.tui), kernel_identity(&bound.leader)));
            }
            Ok(launch)
        }

        /// 在后台线程准备，不持有 TerminalModel 锁；不发送用户输入，不复制认证材料。
        pub(crate) fn prepare(
            executable: &Path,
            app_executable: &Path,
            cwd: &Path,
            pty: GrokOwnedPty,
        ) -> io::Result<Self> {
            let executable = executable.canonicalize()?;
            verify_binary(&executable)?;
            let output = Command::new(&executable).arg("--version").output()?;
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
            let directory = tempfile::Builder::new()
                .prefix("isp-grok-owned-")
                .tempdir_in("/private/tmp")?;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
            let root = directory.path().canonicalize()?;
            let manifest = LaunchManifest {
                version: 1,
                launch_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                executable,
                app_executable: app_executable.canonicalize()?,
                cwd,
                socket_path: root.join("leader.sock"),
                boot_session: macos_boot_session()?,
                shell: pty.shell,
                tty_device: pty.device,
            };
            let bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
            write_new(&root.join("launch.json"), &bytes)?;
            let _ = directory.keep();
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
                || receipt.process.unique_id == 0
                || receipt.process.resource_cid == 0
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
            let actual = macos_process_identity(receipt.process.pid)?;
            if receipt.version != 1
                || receipt.launch_id != self.manifest.launch_id
                || receipt.manifest_sha256 != self.manifest_sha256
                || receipt.tty_device != self.manifest.tty_device
                || receipt.process.pid == self.manifest.shell.pid
                || !exec_identity_matches(&receipt.process, actual)
                || unsafe { libc::getpgid(actual.pid) } != receipt.process_group
                || macos_boot_session()? != self.manifest.boot_session
            {
                return Err(GrokLeaderInputError::IdentityChanged);
            }
            // PID 来自本次入口的真实 exec 进程；leader 来自本次私有 socket 的内核对端。
            let peer = UnixStream::connect(&self.manifest.socket_path)?;
            let leader = macos_peer_identity(&peer)?;
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
            let bound = BoundProcesses {
                version: 1,
                launch_id: self.manifest.launch_id,
                manifest_sha256: self.manifest_sha256.clone(),
                tui: actual.into(),
                leader: leader.into(),
            };
            validate_bound_processes(&bound, &receipt)?;
            let path = self.directory.join("bound.json");
            if let Some(bytes) = read_optional_private(&path)? {
                let previous: BoundProcesses =
                    serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                if previous != bound {
                    return Err(GrokLeaderInputError::IdentityChanged);
                }
            } else {
                write_new(
                    &path,
                    &serde_json::to_vec(&bound).map_err(io::Error::other)?,
                )?;
            }
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

        /// 只释放本次已核对的 TUI 与 leader 都消失后的启动占用；不发送任何终止信号。
        pub(crate) fn release_after_exit(&mut self, ctx: &mut AppContext) -> io::Result<()> {
            if self.phase != LaunchPhase::Dispatched {
                return Err(io::Error::other("owned Grok 尚未派发或已释放"));
            }
            if macos_boot_session()? == self.manifest.boot_session {
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
            if let Some(reservation) = self.reservation.take() {
                CliAgentUpdatesModel::handle(ctx)
                    .update(ctx, |model, ctx| model.release_launch(reservation, ctx));
            }
            fs::remove_dir_all(&self.directory)?;
            self.phase = LaunchPhase::Released;
            Ok(())
        }

        /// 尚未派发才可撤销并清理；已派发的更新占用必须等真实 TUI/leader 退出证据再释放。
        pub(crate) fn cancel_before_dispatch(&mut self, ctx: &mut AppContext) -> io::Result<()> {
            if self.phase == LaunchPhase::Dispatched {
                return Err(io::Error::other("owned Grok 已派发，退出尚未确认"));
            }
            if let Some(reservation) = self.reservation.take() {
                CliAgentUpdatesModel::handle(ctx)
                    .update(ctx, |model, ctx| model.release_launch(reservation, ctx));
            }
            fs::remove_dir_all(&self.directory)?;
            self.phase = LaunchPhase::Released;
            Ok(())
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

    fn exec_identity_matches(before: &ProcessIdentity, after: MacosProcessIdentity) -> bool {
        // macOS exec 保留进程生存期 unique_id，却递增 pid_version；随后另核对新映像与精确 argv。
        before.pid > 0
            && before.pid == after.pid
            && before.unique_id == after.unique_id
            && before.resource_cid == after.resource_cid
            && before.pid_version != after.pid_version
    }

    fn same_process_lifetime(
        expected: MacosProcessIdentity,
        current: MacosProcessIdentity,
    ) -> bool {
        // 后续 exec 不代表进程已经退出；只凭 pid_version 改变不能提前释放 updater 占用。
        expected.pid == current.pid && expected.unique_id == current.unique_id
    }

    fn identity_exited(expected: MacosProcessIdentity) -> bool {
        match macos_process_identity(expected.pid) {
            Ok(current) => !same_process_lifetime(expected, current),
            Err(_) => {
                (unsafe { libc::kill(expected.pid, 0) }) == -1
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            }
        }
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn kernel_identity(value: &ProcessIdentity) -> MacosProcessIdentity {
        MacosProcessIdentity {
            pid: value.pid,
            pid_version: value.pid_version,
            unique_id: value.unique_id,
            resource_cid: value.resource_cid,
        }
    }

    fn validate_bound_processes(bound: &BoundProcesses, receipt: &ExecReceipt) -> io::Result<()> {
        if bound.version != 1
            || bound.launch_id != receipt.launch_id
            || bound.manifest_sha256 != receipt.manifest_sha256
            || !exec_identity_matches(&receipt.process, kernel_identity(&bound.tui))
            || bound.leader.pid <= 0
            || bound.leader.pid == bound.tui.pid
            || bound.leader.unique_id == 0
            || bound.leader.resource_cid == 0
        {
            return Err(io::Error::other("owned Grok 进程绑定与 exec 记录不匹配"));
        }
        Ok(())
    }

    fn validate_manifest(path: &Path, manifest: &LaunchManifest) -> io::Result<()> {
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("启动清单缺少目录"))?;
        if path.file_name() != Some(std::ffi::OsStr::new("launch.json"))
            || manifest.version != 1
            || manifest.launch_id.is_nil()
            || manifest.session_id.is_nil()
            || Uuid::parse_str(&manifest.boot_session).is_err()
            || manifest.shell.pid <= 0
            || manifest.shell.unique_id == 0
            || manifest.shell.resource_cid == 0
            || !manifest.executable.is_absolute()
            || !manifest.app_executable.is_absolute()
            || !manifest.cwd.is_absolute()
            || manifest.socket_path != directory.join("leader.sock")
        {
            return Err(io::Error::other("owned Grok 启动清单无效"));
        }
        Ok(())
    }

    fn verify_binary(path: &Path) -> io::Result<()> {
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
        Ok(())
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
            "--session-id".into(),
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
        if manifest.cwd.canonicalize()? != manifest.cwd
            || std::env::current_exe()?.canonicalize()? != manifest.app_executable
            || macos_boot_session()? != manifest.boot_session
            || unsafe { libc::getppid() } != manifest.shell.pid
            || ProcessIdentity::from(macos_process_identity(manifest.shell.pid)?) != manifest.shell
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
        verify_binary(&manifest.executable)?;
        let receipt = ExecReceipt {
            version: 1,
            launch_id: manifest.launch_id,
            manifest_sha256: digest(&bytes),
            process: macos_process_identity(std::process::id() as i32)?.into(),
            process_group,
            tty_device: manifest.tty_device,
        };
        write_new(
            &directory.join("exec.json"),
            &serde_json::to_vec(&receipt).map_err(io::Error::other)?,
        )?;
        // 不创建新进程或新进程组，不改认证和用户配置；exec 后原生 TUI 接收 Ctrl-C/Ctrl-Z。
        let error = Command::new(&manifest.executable)
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

    #[cfg(test)]
    #[path = "grok_owned_launch_tests.rs"]
    mod tests;

    #[cfg(all(test, feature = "local_fs"))]
    #[path = "grok_owned_native_live_tests.rs"]
    mod live_tests;
}
