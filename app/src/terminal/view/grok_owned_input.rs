//! 普通 Grok 的显式 GUI 启动：先落盘，后核对同一草稿与 PTY，最后只派发一次。

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::json;
use uuid::Uuid;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::{BlockId, SessionId, ShellLaunchState, ShellType, TerminalView};
use crate::ai::cli_agent_runtime;
use crate::editor::EditorBufferRevision;
use crate::persistence::PersistenceWriter;
use crate::persistence::local_cli_tasks::checkpoint_task;
use crate::persistence::local_cli_tasks::grok_terminal::GrokTerminalOwner;
use crate::persistence::model::{LocalCliTask, LocalCliTaskState};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::grok_leader_input::calibrated_platform;
use crate::terminal::cli_agent_sessions::grok_owned_launch::{GrokOwnedLaunch, GrokOwnedPty};
use crate::terminal::cli_agent_sessions::grok_owned_worker::{
    GrokOwnedInputLease, GrokOwnedWorker,
};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, GrokPermissionEvidence};
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;
#[cfg(not(windows))]
use crate::terminal::model::session::command_executor::shell_quote_arg;
#[cfg(windows)]
use crate::terminal::cli_agent_sessions::grok_owned_launch::windows_launch_command;

#[derive(Clone, PartialEq, Eq)]
struct LaunchSnapshot {
    pty: LocalPtyIdentity,
    session: SessionId,
    block: BlockId,
    cwd: PathBuf,
    shell: ShellType,
}

pub(super) struct OwnedInput {
    pub(super) task: LocalCliTask,
    pub(super) owner: GrokTerminalOwner,
    snapshot: LaunchSnapshot,
    pub(super) worker: Option<GrokOwnedWorker>,
    pub(super) lease: Option<GrokOwnedInputLease>,
    pub(super) last_attempt: Option<(EditorBufferRevision, u64)>,
    pub(super) sending: bool,
    pub(super) invalidated: bool,
    pub(super) unconfirmed_drafts: HashMap<Uuid, (String, Vec<PathBuf>, Vec<String>)>,
    binding: bool,
    launch_command: Option<String>,
}

impl Drop for OwnedInput {
    fn drop(&mut self) {
        if let Some(lease) = &self.lease {
            lease.revoke();
        }
    }
}

impl TerminalView {
    fn owned_launch_snapshot(&self, ctx: &AppContext) -> Option<LaunchSnapshot> {
        if calibrated_platform().is_err() || self.inactive_pty_reads_rx(ctx).is_none() {
            return None;
        }
        let model = self.model.lock();
        let block = model.block_list().active_block();
        if !model.block_list().is_bootstrapped()
            || block.is_active_and_long_running()
            || model.shared_session_status().is_sharer_or_viewer()
            || model.is_conversation_transcript_viewer()
            || !matches!(
                model.shell_launch_state(),
                ShellLaunchState::ShellSpawned { .. }
            )
            || !model
                .shell_launch_state()
                .available_shell()
                .is_some_and(|shell| !shell.is_docker_sandbox())
        {
            return None;
        }
        let session_id = block.session_id()?;
        let session = self.sessions.as_ref(ctx).get(session_id)?;
        let shell = session.shell().shell_type();
        if !session.is_local()
            || session.is_subshell_or_ssh()
            || session.is_wsl()
            || session.is_msys2()
            || !(cfg!(unix) && matches!(shell, ShellType::Bash | ShellType::Zsh)
                || cfg!(windows) && shell == ShellType::PowerShell)
        {
            return None;
        }
        let cwd = session
            .launch_data()?
            .maybe_convert_absolute_path(block.metadata().current_working_directory()?)?;
        Some(LaunchSnapshot {
            pty: model.local_pty_identity()?,
            session: session_id,
            block: block.id().clone(),
            cwd,
            shell,
        })
    }

    pub(super) fn prepare_owned_grok_pending(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(intent) = self.pending_specific_cli_agent_launch.as_ref() else {
            return false;
        };
        if intent.preparation.is_some() {
            return false;
        }
        let Some(snapshot) = self.owned_launch_snapshot(ctx) else {
            return false;
        };
        let Some(executable) = intent.executable.clone() else {
            self.show_error_toast(crate::t!("cli-agent-grok-owned-launch-unavailable"), ctx);
            return false;
        };
        let Some(database) = PersistenceWriter::as_ref(ctx).sender() else {
            self.show_error_toast(crate::t!("cli-agent-task-save-failed"), ctx);
            return false;
        };
        let revision = intent.revision.clone();
        let token = Uuid::new_v4();
        self.pending_specific_cli_agent_launch
            .as_mut()
            .unwrap()
            .preparation = Some(token);
        let prepared_snapshot = snapshot.clone();
        let state_dir = cli_agent_runtime::current_state_dir();
        ctx.spawn(async move {
            let launch = blocking::unblock(move || {
                GrokOwnedLaunch::prepare(&executable, &std::env::current_exe()?, &prepared_snapshot.cwd,
                    &state_dir, GrokOwnedPty::from_local_pty(prepared_snapshot.pty)?)
            }).await.map_err(|_| ())?;
            let owner = GrokTerminalOwner { version: 1, launch_id: launch.launch_id(),
                session_id: launch.session_id(), binding_id: Uuid::new_v4(),
                input_revision: Uuid::new_v4(), permission_revision: Uuid::new_v4(),
                cli_version: "1.0.41".into(), model_id: "grok-4.7".into(), permission_mode: "default".into() };
            let task = LocalCliTask { version: 1, task_id: Uuid::new_v4().to_string(),
                parent_task_id: None, parent_generation: None, harness: "grok".into(),
                working_directory: launch.working_directory().to_string_lossy().into_owned(),
                config_json: json!({"execution_kind":"grok_owned_terminal", "grok_terminal":owner,
                    "launch_manifest":launch.manifest_path(), "launch_sha256":launch.manifest_sha256()}).to_string(),
                native_session_id: Some(launch.session_id().to_string()), generation: 1, revision: 0,
                state: LocalCliTaskState::Queued, result: None, terminal_evidence: None };
            let saved = match checkpoint_task(&database, task.clone(), None) {
                Ok(receipt) => matches!(receipt.await, Ok(Ok(()))), Err(_) => false,
            };
            Ok::<_, ()>((launch, owner, task, saved))
        }, move |view, result, ctx| {
            let current = view.pending_specific_cli_agent_launch.as_ref().is_some_and(|intent|
                intent.owned_grok && intent.preparation == Some(token) && intent.revision == revision)
                && view.input.as_ref(ctx).has_pending_command()
                && view.input.as_ref(ctx).buffer_text(ctx) == CLIAgent::Grok.command_prefix()
                && view.input.as_ref(ctx).editor().as_ref(ctx).buffer_revision(ctx) == revision
                && view.owned_launch_snapshot(ctx).as_ref() == Some(&snapshot);
            let Ok((mut launch, owner, task, saved)) = result else {
                if current { view.pending_specific_cli_agent_launch = None;
                    view.input.update(ctx, |input, _| input.cancel_owned_cli_pending_command());
                    view.show_error_toast(crate::t!("cli-agent-grok-owned-launch-unavailable"), ctx); }
                return;
            };
            if !current || !saved || launch.reserve_launch(ctx).is_err() {
                let _ = launch.cancel_before_dispatch(ctx);
                if current { view.pending_specific_cli_agent_launch = None;
                    view.input.update(ctx, |input, _| input.cancel_owned_cli_pending_command());
                    view.show_error_toast(crate::t!("cli-agent-grok-owned-launch-unavailable"), ctx); }
                return;
            }
            let Ok(argv) = launch.take_launch_argv() else {
                let _ = launch.cancel_before_dispatch(ctx);
                view.pending_specific_cli_agent_launch = None;
                    view.input.update(ctx, |input, _| input.cancel_owned_cli_pending_command());
                view.show_error_toast(crate::t!("cli-agent-grok-owned-launch-unavailable"), ctx); return;
            };
            // 先把占用移交应用模型，pane 销毁也不能让更新器误判空闲。
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, ctx| model.adopt_grok_owned_launch(launch, ctx));
            view.pending_specific_cli_agent_launch = None;
            #[cfg(windows)]
            let command = windows_launch_command(&argv).ok();
            #[cfg(not(windows))]
            let command = argv.iter().map(|arg| arg.to_str().map(|arg| shell_quote_arg(arg, snapshot.shell)))
                .collect::<Option<Vec<_>>>().map(|args| args.join(" "));
            view.grok_owned_input = Some(OwnedInput { task, owner, snapshot: snapshot.clone(),
                worker: None, lease: None, last_attempt: None, sending: false, invalidated: false, unconfirmed_drafts: HashMap::new(), binding: false,
                launch_command: command.clone() });
            let sent = command.is_some_and(|command| view.input.update(ctx, |input, ctx|
                input.execute_owned_cli_command_once(&command, ctx)));
            view.awaiting_pending_command_completion = sent;
            if !sent { view.show_error_toast(crate::t!("cli-agent-grok-owned-launch-unavailable"), ctx); }
        });
        false
    }

    /// 只识别本视图派发到同一 PTY 的完整命令；它只展示工具栏，不构成原生权限证据。
    pub(super) fn is_owned_grok_command(&self, model: &super::TerminalModel) -> bool {
        self.grok_owned_input.as_ref().is_some_and(|owned| {
            !owned.invalidated
                && model.local_pty_identity() == Some(owned.snapshot.pty.clone())
                && model.block_list().active_block().session_id() == Some(owned.snapshot.session)
                && !model.shared_session_status().is_sharer_or_viewer()
                && !model.is_conversation_transcript_viewer()
                && owned.launch_command.as_deref()
                    == Some(
                        model
                            .block_list()
                            .active_block()
                            .command_with_secrets_obfuscated(false)
                            .as_str(),
                    )
        })
    }

    pub(super) fn retire_owned_grok_input_for_block(
        &mut self,
        block: &BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .grok_owned_input
            .as_ref()
            .is_some_and(|owned| &owned.snapshot.block == block)
        {
            // pane 输入状态与全局进程占用独立，后者必须继续等真实退出证据。
            CLIAgentSessionsModel::handle(ctx)
                .update(ctx, |model, _| model.revoke_owned_grok_input(self.view_id));
            self.grok_owned_input = None;
        }
    }

    pub(super) fn owned_grok_identity_matches(&self) -> bool {
        let Some(owned) = &self.grok_owned_input else {
            return false;
        };
        let model = self.model.lock();
        model.local_pty_identity() == Some(owned.snapshot.pty.clone())
            && model.block_list().active_block().session_id() == Some(owned.snapshot.session)
            && !model.shared_session_status().is_sharer_or_viewer()
            && !model.is_conversation_transcript_viewer()
    }

    pub(super) fn invalidate_owned_grok_if_changed(&mut self, ctx: &mut ViewContext<Self>) {
        if self.grok_owned_input.is_some() && !self.owned_grok_identity_matches() {
            let owned = self.grok_owned_input.as_mut().unwrap();
            owned.invalidated = true;
            if let Some(lease) = &owned.lease {
                lease.revoke();
            }
            CLIAgentSessionsModel::handle(ctx)
                .update(ctx, |model, _| model.revoke_owned_grok_input(self.view_id));
        }
    }

    /// 即使尚未提交输入，也保存真实 TUI/leader 身份以核验退出。此绑定不授予输入权限。
    pub(super) fn observe_owned_grok_start(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.owned_grok_identity_matches() {
            return;
        }
        let Some(owned) = self.grok_owned_input.as_mut() else {
            return;
        };
        if owned.invalidated || owned.binding {
            return;
        }
        let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) else {
            return;
        };
        let GrokPermissionEvidence::Observed(observation) =
            &session.session_context.grok_permission_evidence
        else {
            return;
        };
        let observation = observation.clone();
        if observation.mode != "default"
            || observation.session_id != owned.owner.session_id.to_string()
        {
            return;
        }
        let config: serde_json::Value = serde_json::from_str(&owned.task.config_json).unwrap();
        let path = PathBuf::from(config["launch_manifest"].as_str().unwrap());
        let sha = config["launch_sha256"].as_str().unwrap().to_owned();
        let owner = owned.owner.clone();
        owned.binding = true;
        let launch_id = owner.launch_id;
        ctx.spawn(
            blocking::unblock(move || {
                GrokOwnedLaunch::restore(&path, &sha).and_then(|mut launch| {
                    launch
                        .bind(owner.binding_id, owner.permission_revision, &observation)
                        .map_err(|_| std::io::Error::other("原生 Grok 绑定未通过"))
                })
            }),
            move |view, _result, _ctx| {
                if let Some(owned) = view
                    .grok_owned_input
                    .as_mut()
                    .filter(|owned| owned.owner.launch_id == launch_id)
                {
                    owned.binding = false;
                }
            },
        );
    }
}

// 此夹具依赖 Unix openpty；Windows ConPTY 场景需要独立的真实派生夹具。
#[cfg(all(test, unix))]
#[path = "grok_owned_input_tests.rs"]
mod tests;

#[path = "grok_owned_history_view.rs"]
mod history;
