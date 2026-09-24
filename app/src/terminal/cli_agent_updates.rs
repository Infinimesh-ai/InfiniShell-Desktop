//! 三款本地 CLI 的后台升级；安装来源、目标版本与活跃会话共同决定是否可以执行。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
#[cfg(test)]
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use warpui::r#async::Timer;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::cli_agent::{CLIAgent, CLIAgentInstallModel};

mod sources;

const AGENTS: [CLIAgent; 3] = [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok];
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const TICK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CliAgentUpdatePhase {
    NotChecked,
    Checking,
    UpToDate,
    Available,
    WaitingForIdle,
    Updating,
    Verifying,
    Failed,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CliAgentUpdateSource {
    Native,
    Npm,
    Homebrew,
    WinGet,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CliAgentUpdateChannel {
    #[default]
    FollowInstallation,
    Latest,
    Stable,
    Alpha,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CliAgentUpdateError {
    NotInstalled,
    UnsupportedSource,
    UnsupportedPlatform,
    SourceChanged,
    Network,
    InvalidRelease,
    ProbeFailed,
    PermissionDenied,
    CommandFailed,
    TimedOut,
    VersionMismatch,
    ChannelMismatch,
    RecoveryRequired,
    PersistenceFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliAgentUpdateStatus {
    pub phase: CliAgentUpdatePhase,
    pub installed_version: Option<String>,
    /// 当前选择渠道的目标版本；Claude stable 不等同于 latest。
    pub latest_version: Option<String>,
    pub source: CliAgentUpdateSource,
    pub error: Option<CliAgentUpdateError>,
    pub auto_update: bool,
    pub busy: bool,
    pub channel: CliAgentUpdateChannel,
    pub effective_channel: Option<CliAgentUpdateChannel>,
}

#[derive(Clone, Copy, Debug)]
pub enum CliAgentUpdateEvent {
    Changed { agent: CLIAgent },
    InstallationChanged { agent: CLIAgent },
}

struct Entry {
    status: CliAgentUpdateStatus,
    plan: Option<sources::UpdatePlan>,
    operation: u64,
    active: bool,
    manual_update: bool,
    recheck: bool,
    failed_target: Option<String>,
    next_check: Instant,
    check_failures: u8,
    recovery_required: bool,
    reported_busy: bool,
    launch_reservations: HashSet<EntityId>,
}

impl Entry {
    fn new(now: Instant) -> Self {
        Self {
            status: CliAgentUpdateStatus {
                phase: CliAgentUpdatePhase::NotChecked,
                installed_version: None,
                latest_version: None,
                source: CliAgentUpdateSource::Unknown,
                error: None,
                auto_update: true,
                // 注册模型后，装配层必须先同步设置和会话，再允许修改安装。
                busy: true,
                channel: CliAgentUpdateChannel::FollowInstallation,
                effective_channel: None,
            },
            plan: None,
            operation: 0,
            active: false,
            manual_update: false,
            recheck: false,
            failed_target: None,
            next_check: now,
            check_failures: 0,
            recovery_required: false,
            reported_busy: true,
            launch_reservations: HashSet::new(),
        }
    }

    fn wants_update(&self) -> bool {
        self.manual_update
            || (self.status.auto_update
                && self.failed_target.as_ref() != self.status.latest_version.as_ref())
    }

    fn ready_to_update(&self) -> bool {
        !self.active
            && !self.status.busy
            && self.plan.is_some()
            && self.wants_update()
            && matches!(
                self.status.phase,
                CliAgentUpdatePhase::Available | CliAgentUpdatePhase::WaitingForIdle
            )
    }

    fn check_failed(&mut self, error: CliAgentUpdateError, now: Instant) {
        self.plan = None;
        self.status.error = Some(error);
        self.status.phase = match error {
            CliAgentUpdateError::NotInstalled
            | CliAgentUpdateError::UnsupportedSource
            | CliAgentUpdateError::UnsupportedPlatform
            | CliAgentUpdateError::ChannelMismatch => CliAgentUpdatePhase::Unsupported,
            CliAgentUpdateError::SourceChanged
            | CliAgentUpdateError::Network
            | CliAgentUpdateError::InvalidRelease
            | CliAgentUpdateError::ProbeFailed
            | CliAgentUpdateError::PermissionDenied
            | CliAgentUpdateError::CommandFailed
            | CliAgentUpdateError::TimedOut
            | CliAgentUpdateError::VersionMismatch
            | CliAgentUpdateError::RecoveryRequired
            | CliAgentUpdateError::PersistenceFailed => CliAgentUpdatePhase::Failed,
        };
        // 只重试只读网络检查；安装命令失败不能进入后台无限重投。
        self.check_failures = self.check_failures.saturating_add(1);
        let delay = if error == CliAgentUpdateError::Network {
            match self.check_failures {
                1 => Duration::from_secs(60),
                2 => Duration::from_secs(300),
                3.. => CHECK_INTERVAL,
                0 => CHECK_INTERVAL,
            }
        } else {
            CHECK_INTERVAL
        };
        self.next_check = now + delay;
    }
}

pub struct CliAgentUpdatesModel {
    entries: HashMap<CLIAgent, Entry>,
    client: Arc<http_client::Client>,
    operations_enabled: bool,
    #[cfg(test)]
    recorded_updates: Option<SyncSender<CLIAgent>>,
}

impl CliAgentUpdatesModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut model = Self {
            entries: AGENTS
                .into_iter()
                .map(|agent| {
                    let mut entry = Entry::new(Instant::now());
                    if !cfg!(test) && sources::recovery_pending(agent) {
                        entry.recovery_required = true;
                        entry.status.phase = CliAgentUpdatePhase::Failed;
                        entry.status.error = Some(CliAgentUpdateError::RecoveryRequired);
                    }
                    (agent, entry)
                })
                .collect(),
            client: Arc::new(http_client::Client::new()),
            operations_enabled: !cfg!(test),
            #[cfg(test)]
            recorded_updates: None,
        };
        if model.operations_enabled {
            model.schedule_tick(ctx);
        }
        model
    }

    #[cfg(test)]
    pub(crate) fn set_phase_for_test(&mut self, agent: CLIAgent, phase: CliAgentUpdatePhase) {
        if let Some(entry) = self.entries.get_mut(&agent) {
            entry.status.phase = phase;
        }
    }

    #[cfg(test)]
    pub(crate) fn stage_recorded_update_for_test(
        &mut self,
        agent: CLIAgent,
        ctx: &mut ModelContext<Self>,
    ) -> Receiver<CLIAgent> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.recorded_updates = Some(sender);
        self.operations_enabled = true;
        let entry = self.entries.get_mut(&agent).unwrap();
        entry.status.phase = CliAgentUpdatePhase::Available;
        entry.status.latest_version = Some("synthetic-target".to_owned());
        entry.plan = Some(sources::plan_for_test());
        self.try_update(agent, ctx);
        receiver
    }

    pub fn status(&self, agent: CLIAgent) -> Option<&CliAgentUpdateStatus> {
        self.entries.get(&agent).map(|entry| &entry.status)
    }

    pub fn is_updating(&self, agent: CLIAgent) -> bool {
        self.entries.get(&agent).is_some_and(|entry| {
            let status = &entry.status;
            entry.recovery_required
                || matches!(
                    status.phase,
                    CliAgentUpdatePhase::Updating | CliAgentUpdatePhase::Verifying
                )
                || status.error == Some(CliAgentUpdateError::RecoveryRequired)
        })
    }

    /// 装配层在同一事件内一次同步全部条件，避免旧空闲状态或旧渠道抢先派发。
    pub fn configure(
        &mut self,
        agent: CLIAgent,
        auto_update: bool,
        busy: bool,
        channel: CliAgentUpdateChannel,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        let channel_changed = entry.status.channel != channel;
        let changed =
            channel_changed || entry.status.auto_update != auto_update || entry.status.busy != busy;
        entry.status.auto_update = auto_update;
        entry.reported_busy = busy;
        entry.status.busy = busy || !entry.launch_reservations.is_empty();
        entry.status.channel = channel;
        if channel_changed {
            entry.plan = None;
            entry.failed_target = None;
            if entry.active {
                entry.recheck = true;
            }
        }
        if !auto_update
            && !entry.manual_update
            && entry.status.phase == CliAgentUpdatePhase::WaitingForIdle
        {
            entry.status.phase = CliAgentUpdatePhase::Available;
        }
        let should_check = channel_changed && !entry.active;
        if changed {
            self.changed(agent, ctx);
        }
        if should_check {
            self.check_now(agent, ctx);
        } else {
            self.try_update(agent, ctx);
        }
    }

    pub fn set_auto_update(
        &mut self,
        agent: CLIAgent,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if entry.status.auto_update == enabled {
            return;
        }
        entry.status.auto_update = enabled;
        if !enabled
            && !entry.manual_update
            && entry.status.phase == CliAgentUpdatePhase::WaitingForIdle
        {
            entry.status.phase = CliAgentUpdatePhase::Available;
        }
        self.changed(agent, ctx);
        self.try_update(agent, ctx);
    }

    pub fn set_busy(&mut self, agent: CLIAgent, busy: bool, ctx: &mut ModelContext<Self>) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        entry.reported_busy = busy;
        let effective_busy = busy || !entry.launch_reservations.is_empty();
        if entry.status.busy != effective_busy {
            entry.status.busy = effective_busy;
            self.changed(agent, ctx);
        }
        self.try_update(agent, ctx);
    }

    /// PTY 派发到 shell Preexec 之间也属于活跃启动，不能在此窗口开始更新。
    pub fn reserve_launch(
        &mut self,
        agent: CLIAgent,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_updating(agent) {
            return false;
        }
        let Some(entry) = self.entries.get_mut(&agent) else {
            return true;
        };
        let changed = entry.launch_reservations.insert(terminal_view_id);
        entry.status.busy = true;
        if changed {
            self.changed(agent, ctx);
        }
        true
    }

    pub fn release_launch(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        for agent in AGENTS {
            let Some(entry) = self.entries.get_mut(&agent) else {
                continue;
            };
            if !entry.launch_reservations.remove(&terminal_view_id) {
                continue;
            }
            entry.status.busy = entry.reported_busy || !entry.launch_reservations.is_empty();
            self.changed(agent, ctx);
            self.try_update(agent, ctx);
        }
    }

    pub fn set_channel(
        &mut self,
        agent: CLIAgent,
        channel: CliAgentUpdateChannel,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if entry.status.channel == channel {
            return;
        }
        entry.status.channel = channel;
        entry.plan = None;
        entry.failed_target = None;
        // 已派发的更新使用原计划完成；新渠道只影响下一次检查，不能使回执失配。
        if entry.active {
            entry.recheck = true;
            self.changed(agent, ctx);
        } else {
            self.check_now(agent, ctx);
        }
    }

    pub fn check_now(&mut self, agent: CLIAgent, ctx: &mut ModelContext<Self>) {
        if !self.operations_enabled {
            return;
        }
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if entry.active {
            entry.recheck = true;
            return;
        }
        let installation = CLIAgentInstallModel::as_ref(ctx);
        if !installation.is_scan_complete() {
            return;
        }
        let executable = installation.executable(agent).map(ToOwned::to_owned);
        entry.active = true;
        entry.operation += 1;
        entry.recheck = false;
        entry.status.phase = CliAgentUpdatePhase::Checking;
        if entry.status.error != Some(CliAgentUpdateError::RecoveryRequired) {
            entry.status.error = None;
        }
        let operation = entry.operation;
        let channel = entry.status.channel;
        let client = self.client.clone();
        self.changed(agent, ctx);
        ctx.spawn(
            async move { sources::inspect(agent, executable, channel, &client).await },
            move |model, result, ctx| model.checked(agent, operation, channel, result, ctx),
        );
    }

    pub fn update_now(&mut self, agent: CLIAgent, ctx: &mut ModelContext<Self>) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if matches!(
            entry.status.phase,
            CliAgentUpdatePhase::Updating | CliAgentUpdatePhase::Verifying
        ) {
            return;
        }
        entry.manual_update = true;
        entry.failed_target = None;
        // 手动请求同样重新读取来源和目标，不能执行设置页中已过时的计划。
        self.check_now(agent, ctx);
    }

    fn checked(
        &mut self,
        agent: CLIAgent,
        operation: u64,
        channel: CliAgentUpdateChannel,
        result: Result<sources::CheckReport, CliAgentUpdateError>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if !entry.active || entry.operation != operation {
            return;
        }
        entry.active = false;
        if entry.status.channel != channel || entry.recheck {
            entry.plan = None;
            entry.recheck = false;
            self.check_now(agent, ctx);
            return;
        }
        entry.next_check = Instant::now() + CHECK_INTERVAL;
        match result {
            Ok(report) => {
                entry.recovery_required = false;
                entry.check_failures = 0;
                entry.failed_target = report.failed_target;
                entry.status.installed_version = Some(report.installed_version);
                entry.status.latest_version = Some(report.latest_version);
                entry.status.source = report.source;
                entry.status.effective_channel = Some(report.effective_channel);
                entry.status.error = report.error;
                entry.plan = report.plan;
                entry.status.phase = if report.error.is_some() {
                    entry.manual_update = false;
                    CliAgentUpdatePhase::Unsupported
                } else if report.up_to_date {
                    entry.manual_update = false;
                    CliAgentUpdatePhase::UpToDate
                } else if entry.status.busy && entry.wants_update() {
                    CliAgentUpdatePhase::WaitingForIdle
                } else {
                    CliAgentUpdatePhase::Available
                };
                if !report.up_to_date
                    && report.error.is_none()
                    && entry.failed_target == entry.status.latest_version
                {
                    entry.status.error = Some(CliAgentUpdateError::CommandFailed);
                }
            }
            Err(error) => {
                let error = if entry.recovery_required
                    || entry.status.error == Some(CliAgentUpdateError::RecoveryRequired)
                {
                    CliAgentUpdateError::RecoveryRequired
                } else {
                    error
                };
                if error != CliAgentUpdateError::Network {
                    entry.manual_update = false;
                }
                if error == CliAgentUpdateError::NotInstalled {
                    entry.status.installed_version = None;
                    entry.status.latest_version = None;
                    entry.status.source = CliAgentUpdateSource::Unknown;
                }
                entry.check_failed(error, Instant::now());
            }
        }
        self.changed(agent, ctx);
        self.try_update(agent, ctx);
    }

    fn try_update(&mut self, agent: CLIAgent, ctx: &mut ModelContext<Self>) {
        if !self.operations_enabled {
            return;
        }
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if !entry.ready_to_update() {
            if !entry.active
                && entry.status.busy
                && entry.plan.is_some()
                && entry.wants_update()
                && entry.status.phase == CliAgentUpdatePhase::Available
            {
                entry.status.phase = CliAgentUpdatePhase::WaitingForIdle;
                self.changed(agent, ctx);
            }
            return;
        }
        let Some(plan) = entry.plan.take() else {
            return;
        };
        entry.active = true;
        entry.operation += 1;
        entry.status.phase = CliAgentUpdatePhase::Updating;
        entry.status.error = None;
        entry.manual_update = false;
        let operation = entry.operation;
        self.changed(agent, ctx);
        #[cfg(test)]
        if let Some(sender) = &self.recorded_updates {
            // 接线测试走真实延期与派发状态机，但不触碰用户安装或事务目录。
            sender.try_send(agent).expect("更新只能派发一次");
            return;
        }
        let (verification_started_tx, verification_started_rx) = async_channel::bounded(1);
        let (verification_continue_tx, verification_continue_rx) = async_channel::bounded(1);
        ctx.spawn(
            async move { verification_started_rx.recv().await },
            move |model, result, ctx| {
                if result.is_ok() {
                    model.mark_verifying(agent, operation, ctx);
                }
                // 即使该操作已经过期，也要放行其事务清理；旧操作不能更新当前状态。
                let _ = verification_continue_tx.try_send(());
            },
        );
        ctx.spawn(
            async move {
                sources::execute(
                    plan,
                    Some(sources::VerificationProgress::new(
                        verification_started_tx,
                        verification_continue_rx,
                    )),
                )
                .await
            },
            move |model, result, ctx| model.updated(agent, operation, result, ctx),
        );
    }

    fn updated(
        &mut self,
        agent: CLIAgent,
        operation: u64,
        result: Result<String, CliAgentUpdateError>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if !entry.active || entry.operation != operation {
            return;
        }
        entry.active = false;
        match result {
            Ok(version) => {
                entry.status.installed_version = Some(version);
                entry.status.phase = CliAgentUpdatePhase::UpToDate;
                entry.failed_target = None;
                entry.status.error = None;
                ctx.emit(CliAgentUpdateEvent::InstallationChanged { agent });
            }
            Err(error) => {
                if error == CliAgentUpdateError::RecoveryRequired {
                    entry.recovery_required = true;
                }
                entry.failed_target = entry.status.latest_version.clone();
                entry.check_failed(error, Instant::now());
            }
        }
        let recheck = entry.recheck;
        self.changed(agent, ctx);
        if recheck {
            self.check_now(agent, ctx);
        }
    }

    fn mark_verifying(&mut self, agent: CLIAgent, operation: u64, ctx: &mut ModelContext<Self>) {
        let Some(entry) = self.entries.get_mut(&agent) else {
            return;
        };
        if !entry.active
            || entry.operation != operation
            || entry.status.phase != CliAgentUpdatePhase::Updating
        {
            return;
        }
        entry.status.phase = CliAgentUpdatePhase::Verifying;
        self.changed(agent, ctx);
    }

    fn schedule_tick(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async { Timer::after(TICK_INTERVAL).await },
            |model, _, ctx| {
                let now = Instant::now();
                for agent in AGENTS {
                    if model
                        .entries
                        .get(&agent)
                        .is_some_and(|entry| !entry.active && entry.next_check <= now)
                    {
                        model.check_now(agent, ctx);
                    }
                }
                model.schedule_tick(ctx);
            },
        );
    }

    fn changed(&self, agent: CLIAgent, ctx: &mut ModelContext<Self>) {
        ctx.emit(CliAgentUpdateEvent::Changed { agent });
        ctx.notify();
    }
}

impl Entity for CliAgentUpdatesModel {
    type Event = CliAgentUpdateEvent;
}

impl SingletonEntity for CliAgentUpdatesModel {}

#[cfg(test)]
#[path = "cli_agent_updates_tests.rs"]
mod tests;
