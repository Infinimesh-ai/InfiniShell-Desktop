use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::{AuthManager, AuthManagerEvent};
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
pub const WARP_WORKER_HOST: &str = "warp";

/// Zap:上游此类型来自云端网关 `server::server_api::ai`,该模块已在本地优先 fork 中删除。
/// 这里保留同名本地结构体,以便编排 UI / 测试的既有引用继续成立。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectedSelfHostedWorker {
    pub worker_host: String,
    pub connection_count: i64,
    pub connected_at: String,
    pub last_seen_at: String,
}

pub enum ConnectedSelfHostedWorkersEvent {
    Changed,
}

pub struct ConnectedSelfHostedWorkersModel {
    workers: Vec<ConnectedSelfHostedWorker>,
}

impl ConnectedSelfHostedWorkersModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&NetworkStatus::handle(ctx), |me, _, event, ctx| {
            if let NetworkStatusEvent::NetworkStatusChanged {
                new_status: NetworkStatusKind::Online,
            } = event
            {
                me.refresh(ctx);
            }
        });

        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| match event {
            AuthManagerEvent::AuthComplete => {
                me.refresh(ctx);
            }
            AuthManagerEvent::AuthFailed(_)
            | AuthManagerEvent::SkippedLogin
            | AuthManagerEvent::NeedsReauth => {
                me.clear_workers(ctx);
            }
            // Zap:`crate::auth` 是单文件 facade,`LoginOverrideDetected` /
            // `MintCustomTokenFailed` / `ReceivedDeviceAuthorizationCode` 三个云端
            // 登录流程事件已随之删除,这里只保留仍存在的变体。
            AuthManagerEvent::CreateAnonymousUserFailed
            | AuthManagerEvent::AttemptedLoginGatedFeature { .. } => {}
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                me.refresh(ctx);
            }
        });

        let mut me = Self {
            workers: Vec::new(),
        };
        me.refresh(ctx);
        me
    }

    pub fn worker_hosts_excluding(&self, excluded: Option<&str>) -> Vec<String> {
        let mut hosts: Vec<String> = self
            .workers
            .iter()
            .map(|worker| worker.worker_host.clone())
            .filter(|host| !host.trim().is_empty())
            .filter(|host| !host.eq_ignore_ascii_case(WARP_WORKER_HOST))
            .filter(|host| match excluded {
                Some(excluded) => !host.eq_ignore_ascii_case(excluded),
                None => true,
            })
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }

    /// Zap:自托管 worker 列表原本走云端网关 `server_api.list_connected_self_hosted_workers()`,
    /// 该网关已随本地优先化删除。这里保留方法签名与订阅链路(网络/登录/团队变更仍会触发),
    /// 但不再发起任何远端请求,只清空本地缓存,使 `worker_hosts_excluding` 恒返回空列表。
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        self.clear_workers(ctx);
    }

    fn clear_workers(&mut self, ctx: &mut ModelContext<Self>) {
        if self.clear_worker_cache() {
            ctx.emit(ConnectedSelfHostedWorkersEvent::Changed);
        }
    }

    fn clear_worker_cache(&mut self) -> bool {
        if self.workers.is_empty() {
            return false;
        }
        self.workers.clear();
        true
    }
}

#[cfg(test)]
impl ConnectedSelfHostedWorkersModel {
    /// Test hook: set the connected workers and emit `Changed`.
    pub fn set_workers_for_test(&mut self, worker_hosts: &[&str], ctx: &mut ModelContext<Self>) {
        self.workers = worker_hosts
            .iter()
            .map(|host| ConnectedSelfHostedWorker {
                worker_host: (*host).to_string(),
                connection_count: 1,
                connected_at: String::new(),
                last_seen_at: String::new(),
            })
            .collect();
        ctx.emit(ConnectedSelfHostedWorkersEvent::Changed);
    }
}

impl Entity for ConnectedSelfHostedWorkersModel {
    type Event = ConnectedSelfHostedWorkersEvent;
}

impl SingletonEntity for ConnectedSelfHostedWorkersModel {}

#[cfg(test)]
#[path = "connected_self_hosted_workers_tests.rs"]
mod tests;
