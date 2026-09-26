//! 应用重启仅恢复已落盘的普通 Grok 启动占用，不启动原生进程、不恢复输入授权。

use serde_json::Value;
use uuid::Uuid;
use warpui::ModelContext;
use warpui::r#async::{SpawnedFutureHandle, Timer};

use super::CLIAgentSessionsModel;
use super::grok_owned_launch::GrokOwnedLaunch;
use crate::persistence::local_cli_tasks::grok_terminal::GrokTerminalOwner;
use crate::persistence::model::LocalCliTask;

#[derive(Default)]
pub(super) struct GrokOwnedRecovery {
    pub(super) pending: bool,
    failed: bool,
    request: Option<Uuid>,
    launches: Vec<GrokOwnedLaunch>,
    timer: Option<(Uuid, SpawnedFutureHandle)>,
}

impl GrokOwnedRecovery {
    pub(super) fn busy(&self) -> bool {
        self.pending || self.failed || !self.launches.is_empty()
    }
}

impl CLIAgentSessionsModel {
    pub(super) fn restore_grok_owned_launches(
        &mut self,
        tasks: Vec<LocalCliTask>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.grok_owned_recovery.pending = true;
        let request = Uuid::new_v4();
        self.grok_owned_recovery.request = Some(request);
        ctx.spawn(
            blocking::unblock(move || restore_known_launches(&tasks)),
            move |model, restored, ctx| {
                if model.grok_owned_recovery.request != Some(request) {
                    return;
                }
                model.grok_owned_recovery.pending = false;
                for result in restored {
                    let Ok(mut launch) = result else {
                        // 丢失或损坏的清单不能用来证明 CLI 已退出，保留恢复阻塞。
                        model.grok_owned_recovery.failed = true;
                        continue;
                    };
                    if launch.is_retired() {
                        continue;
                    }
                    if let Some(existing) = model
                        .grok_owned_recovery
                        .launches
                        .iter()
                        .find(|existing| existing.launch_id() == launch.launch_id())
                    {
                        if existing.manifest_sha256() != launch.manifest_sha256()
                            || existing.manifest_path() != launch.manifest_path()
                        {
                            model.grok_owned_recovery.failed = true;
                        }
                        continue;
                    }
                    if !launch.was_dispatched() {
                        if launch.cancel_before_dispatch(ctx).is_err() {
                            model.grok_owned_recovery.failed = true;
                        }
                        continue;
                    }
                    if launch.reserve_for_recovery(ctx).is_err() {
                        model.grok_owned_recovery.failed = true;
                    }
                    model.grok_owned_recovery.launches.push(launch);
                }
                model.reap_grok_owned_launches(ctx);
                ctx.notify();
            },
        );
    }

    fn reap_grok_owned_launches(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some((_, timer)) = self.grok_owned_recovery.timer.take() {
            timer.abort();
        }
        self.grok_owned_recovery.launches.retain_mut(|launch| {
            // 侧车断开、会话 Ended 或回合 Stop 均不能代替两个真实进程的退出证据。
            launch.release_after_exit(ctx).is_err()
        });
        if !self.grok_owned_recovery.launches.is_empty() {
            let token = Uuid::new_v4();
            self.grok_owned_recovery.timer = Some((
                token,
                ctx.spawn(
                    async {
                        Timer::after(std::time::Duration::from_secs(2)).await;
                    },
                    move |model, (), ctx| {
                        if model
                            .grok_owned_recovery
                            .timer
                            .as_ref()
                            .map(|(current, _)| *current)
                            != Some(token)
                        {
                            return;
                        }
                        model.reap_grok_owned_launches(ctx);
                        ctx.notify();
                    },
                ),
            ));
        }
    }
}

fn restore_known_launches(tasks: &[LocalCliTask]) -> Vec<std::io::Result<GrokOwnedLaunch>> {
    let mut restored = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for task in tasks {
        let Ok(config) = serde_json::from_str::<Value>(&task.config_json) else {
            // 损坏的 Grok 配置无法排除本应用拥有的原生进程，不能放行升级。
            if task.harness == "grok" {
                restored.push(Err(std::io::Error::other("Grok 恢复配置无法解析")));
            }
            continue;
        };
        if config["execution_kind"] != "grok_owned_terminal" {
            continue;
        }
        restored.push((|| {
            let invalid = || std::io::Error::other("普通 Grok 恢复身份不完整或重复");
            let owned: GrokTerminalOwner =
                serde_json::from_value(config["grok_terminal"].clone()).map_err(|_| invalid())?;
            let path = config["launch_manifest"].as_str().ok_or_else(invalid)?;
            let sha256 = config["launch_sha256"].as_str().ok_or_else(invalid)?;
            if task.version != 1
                || task.harness != "grok"
                || task.generation != 1
                || task.parent_task_id.is_some()
                || task.parent_generation.is_some()
                || !Uuid::parse_str(&task.task_id).is_ok_and(|id| !id.is_nil())
                || owned.version != 1
                || owned.launch_id.is_nil()
                || owned.session_id.is_nil()
                || owned.binding_id.is_nil()
                || owned.permission_revision.is_nil()
                || owned.input_revision.is_nil()
                || owned.cli_version != "1.0.41"
                || owned.model_id != "grok-4.7"
                || owned.permission_mode != "default"
                || task.native_session_id.as_deref() != Some(owned.session_id.to_string().as_str())
                || sha256.len() != 64
                || !sha256.bytes().all(|c| c.is_ascii_hexdigit())
                || !seen.insert(owned.launch_id)
            {
                return Err(invalid());
            }
            let launch = GrokOwnedLaunch::restore(std::path::Path::new(path), sha256)?;
            if launch.launch_id() != owned.launch_id
                || launch.session_id() != owned.session_id
                || launch.working_directory() != std::path::Path::new(&task.working_directory)
            {
                return Err(invalid());
            }
            Ok(launch)
        })());
    }
    restored
}

#[cfg(test)]
#[path = "grok_owned_recovery_tests.rs"]
mod tests;
