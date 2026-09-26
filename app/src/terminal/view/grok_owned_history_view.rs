//! 普通 Grok 的显式历史继续和重启后的只读重关联，共用原生进程身份，不重投草稿。

use super::*;
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
use crate::terminal::cli_agent::discover_cli_agent_executable;
use crate::terminal::cli_agent_sessions::GrokPermissionObservation;
use crate::terminal::cli_agent_sessions::grok_owned_history::HistorySource;

#[derive(Clone, PartialEq, Eq)]
struct HistorySnapshot {
    launch: LaunchSnapshot,
    idle: bool,
    command: Option<String>,
}

enum HistoryPrepared {
    Associated {
        launch: GrokOwnedLaunch,
        task: LocalCliTask,
        owner: GrokTerminalOwner,
    },
    Continued {
        launch: GrokOwnedLaunch,
        task: LocalCliTask,
        owner: GrokTerminalOwner,
        saved: bool,
    },
}

impl TerminalView {
    fn grok_history_snapshot(&self, ctx: &AppContext) -> Option<HistorySnapshot> {
        if let Some(launch) = self.owned_launch_snapshot(ctx) {
            return Some(HistorySnapshot {
                launch,
                idle: true,
                command: None,
            });
        }
        if calibrated_platform().is_err() || self.inactive_pty_reads_rx(ctx).is_none() {
            return None;
        }
        let model = self.model.lock();
        let block = model.block_list().active_block();
        if !model.block_list().is_bootstrapped()
            || !block.is_active_and_long_running()
            || model.shared_session_status().is_sharer_or_viewer()
            || model.is_conversation_transcript_viewer()
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
        Some(HistorySnapshot {
            idle: false,
            command: Some(block.command_with_secrets_obfuscated(false)),
            launch: LaunchSnapshot {
                pty: model.local_pty_identity()?,
                session: session_id,
                block: block.id().clone(),
                shell,
                cwd: session
                    .launch_data()?
                    .maybe_convert_absolute_path(block.metadata().current_working_directory()?)?,
            },
        })
    }

    pub(crate) fn queue_owned_grok_history(
        &mut self,
        task: LocalCliTask,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        self.prepare_grok_history(task, true, ctx)
    }

    /// SessionStart/SessionUpdated 只允许关联仍活跃的原进程；这里永远不创建新进程。
    pub(in crate::terminal::view) fn observe_owned_grok_history(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.grok_owned_input.is_some() || self.grok_owned_history_request.is_some() {
            return;
        }
        let Some((_, observed)) = self.grok_history_observation(ctx) else {
            return;
        };
        let records = LocalCLITaskCoordinator::as_ref(ctx).restored_tasks();
        let mut matching = records.iter().filter(|task| {
            task.native_session_id.as_deref() == Some(observed.session_id.as_str())
                && HistorySource::from_task(task).is_ok()
        });
        let Some(task) = matching.next().cloned() else {
            return;
        };
        if matching.next().is_none() {
            let _ = self.prepare_grok_history(task, false, ctx);
        }
    }

    fn grok_history_observation(
        &self,
        ctx: &AppContext,
    ) -> Option<(warpui::EntityId, GrokPermissionObservation)> {
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        if session.is_remote() {
            return None;
        }
        match &session.session_context.grok_permission_evidence {
            GrokPermissionEvidence::Observed(observed) if observed.mode == "default" => {
                Some((session.listener.as_ref()?.id(), observed.clone()))
            }
            GrokPermissionEvidence::Observed(_)
            | GrokPermissionEvidence::Unobserved
            | GrokPermissionEvidence::Invalidated => None,
        }
    }

    fn prepare_grok_history(
        &mut self,
        previous: LocalCliTask,
        allow_start: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), String> {
        if self.grok_owned_history_request.is_some() {
            return Err(crate::t!("cli-agent-task-already-running"));
        }
        let (source, previous_owner) = HistorySource::from_task(&previous)
            .map_err(|_| crate::t!("cli-agent-grok-history-unavailable"))?;
        let snapshot = self
            .grok_history_snapshot(ctx)
            .ok_or_else(|| crate::t!("cli-agent-grok-history-unavailable"))?;
        if snapshot.launch.cwd != source.cwd
            || snapshot.idle
                && (!allow_start
                    || !self.input.as_ref(ctx).buffer_text(ctx).is_empty()
                    || self.input.as_ref(ctx).has_pending_command())
        {
            return Err(crate::t!("cli-agent-grok-history-unavailable"));
        }
        let observation = self.grok_history_observation(ctx);
        let executable = discover_cli_agent_executable(CLIAgent::Grok);
        let database = PersistenceWriter::as_ref(ctx)
            .sender()
            .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
        let editor_revision = self
            .input
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .buffer_revision(ctx);
        let text = self.input.as_ref(ctx).buffer_text(ctx);
        let request = Uuid::new_v4();
        self.grok_owned_history_request = Some(request);
        let prepared_snapshot = snapshot.clone();
        let captured_observation = observation.clone();
        let state = cli_agent_runtime::current_state_dir();
        ctx.spawn(
            async move {
                let (mut old, exited) = blocking::unblock({
                    let source = source.clone();
                    move || {
                        let mut old = source.restore()?;
                        let exited = old.confirm_history_exit().is_ok();
                        Ok::<_, std::io::Error>((old, exited))
                    }
                })
                .await
                .map_err(|_| ())?;
                if !exited {
                    let observed = captured_observation
                        .map(|(_, observation)| observation)
                        .filter(|value| value.session_id == source.session_id.to_string())
                        .ok_or(())?;
                    if prepared_snapshot.idle {
                        return Err(());
                    }
                    old = blocking::unblock({
                        let previous_owner = previous_owner.clone();
                        move || {
                            let pty = GrokOwnedPty::from_local_pty(prepared_snapshot.launch.pty)?;
                            if !old.matches_history_pty(&pty)? {
                                return Err(std::io::Error::other("历史任务仍在另一终端运行"));
                            }
                            old.bind(
                                previous_owner.binding_id,
                                previous_owner.permission_revision,
                                &observed,
                            )
                            .map_err(|_| std::io::Error::other("历史任务原生关联未确认"))?;
                            Ok::<_, std::io::Error>(old)
                        }
                    })
                    .await
                    .map_err(|_| ())?;
                    return Ok(HistoryPrepared::Associated {
                        launch: old,
                        task: previous,
                        owner: previous_owner,
                    });
                }
                if !allow_start || !prepared_snapshot.idle {
                    return Err(());
                }
                let launch = blocking::unblock({
                    let source = source.clone();
                    move || {
                        let executable = executable
                            .ok_or_else(|| std::io::Error::other("未发现 Grok 历史启动入口"))?;
                        GrokOwnedLaunch::prepare_history(
                            &executable,
                            &std::env::current_exe()?,
                            &prepared_snapshot.launch.cwd,
                            &state,
                            GrokOwnedPty::from_local_pty(prepared_snapshot.launch.pty)?,
                            source,
                        )
                    }
                })
                .await
                .map_err(|_| ())?;
                let mut previous = previous;
                // 只在真实进程退出后收敛旧活动状态；既有消息回执和 Unknown 保留，不转回 Queued。
                if previous.state.is_active() || previous.state == LocalCliTaskState::Unknown {
                    previous.revision = previous.revision.checked_add(1).ok_or(())?;
                    previous.state = LocalCliTaskState::Disconnected;
                    let receipt =
                        checkpoint_task(&database, previous.clone(), Some(previous.generation))
                            .map_err(|_| ())?;
                    if !matches!(receipt.await, Ok(Ok(()))) {
                        return Err(());
                    }
                }
                let (task, owner) = source.continued_task(&previous, &launch).map_err(|_| ())?;
                let saved =
                    match checkpoint_task(&database, task.clone(), Some(previous.generation)) {
                        Ok(receipt) => matches!(receipt.await, Ok(Ok(()))),
                        Err(_) => false,
                    };
                Ok::<_, ()>(HistoryPrepared::Continued {
                    launch,
                    task,
                    owner,
                    saved,
                })
            },
            move |view, result, ctx| {
                let current = view.grok_owned_history_request == Some(request)
                    && view.grok_history_snapshot(ctx).as_ref() == Some(&snapshot)
                    && view
                        .input
                        .as_ref(ctx)
                        .editor()
                        .as_ref(ctx)
                        .buffer_revision(ctx)
                        == editor_revision
                    && view.input.as_ref(ctx).buffer_text(ctx) == text
                    && view.grok_history_observation(ctx) == observation;
                if view.grok_owned_history_request == Some(request) {
                    view.grok_owned_history_request = None;
                }
                let Ok(prepared) = result else {
                    if current && allow_start {
                        view.show_error_toast(crate::t!("cli-agent-grok-history-unavailable"), ctx);
                    }
                    return;
                };
                let (mut launch, task, owner, start, saved) = match prepared {
                    HistoryPrepared::Associated {
                        launch,
                        task,
                        owner,
                    } => (launch, task, owner, false, true),
                    HistoryPrepared::Continued {
                        launch,
                        task,
                        owner,
                        saved,
                    } => (launch, task, owner, true, saved),
                };
                if !current || !saved {
                    if start {
                        let _ = launch.cancel_before_dispatch(ctx);
                    }
                    return;
                }
                let command = if start {
                    if launch.reserve_launch(ctx).is_err() {
                        let _ = launch.cancel_before_dispatch(ctx);
                        view.show_error_toast(crate::t!("cli-agent-grok-history-unavailable"), ctx);
                        return;
                    }
                    let Ok(argv) = launch.take_launch_argv() else {
                        let _ = launch.cancel_before_dispatch(ctx);
                        return;
                    };
                    #[cfg(windows)]
                    let value = windows_launch_command(&argv).ok();
                    #[cfg(not(windows))]
                    let value = argv
                        .iter()
                        .map(|arg| {
                            arg.to_str()
                                .map(|arg| shell_quote_arg(arg, snapshot.launch.shell))
                        })
                        .collect::<Option<Vec<_>>>()
                        .map(|args| args.join(" "));
                    value
                } else {
                    // 仅供识别正在运行的原始命令，不再次写入 PTY。
                    snapshot.command.clone()
                };
                if start {
                    CLIAgentSessionsModel::handle(ctx)
                        .update(ctx, |model, ctx| model.adopt_grok_owned_launch(launch, ctx));
                }
                view.grok_owned_input = Some(OwnedInput {
                    task,
                    owner,
                    snapshot: snapshot.launch.clone(),
                    worker: None,
                    lease: None,
                    last_attempt: None,
                    sending: false,
                    invalidated: false,
                    unconfirmed_drafts: HashMap::new(),
                    binding: false,
                    launch_command: command.clone(),
                });
                if let Some(command) = command.filter(|_| start) {
                    view.set_pending_command(CLIAgent::Grok.command_prefix(), ctx);
                    let sent = view.input.update(ctx, |input, ctx| {
                        input.execute_owned_cli_command_once(&command, ctx)
                    });
                    view.awaiting_pending_command_completion = sent;
                    if !sent {
                        view.show_error_toast(crate::t!("cli-agent-grok-history-unavailable"), ctx);
                    }
                }
                view.observe_owned_grok_start(ctx);
                ctx.notify();
            },
        );
        Ok(())
    }
}
