//! 项目命令按稳定调用身份逐条派生独立监督代；不把活跃命令交给 Abortable 队列。

use std::collections::{HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{BoxFuture, FutureExt as _};
use futures::io::AsyncReadExt as _;
use futures::{pin_mut, select};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::local_tools::NativeLocalToolRequest;
use super::managed_process::{self, ProcessCompletion};
use super::reviewed_project_commands::{ReviewedCommand, ReviewedCommandCeilingV1, reject};
use super::reviewed_project_commands_lifecycle::ReviewedCommandLifecycle;
use super::{
    ApprovalDecision, RuntimeAction, RuntimeCommand, RuntimeError, RuntimeEventKind, SessionOptions,
};

const OUTPUT_BUDGET: usize = 256 * 1024;
const RECORD_NAME: &str = "reviewed-command.json";
const MAX_WAITING_COMMANDS: usize = 32;

struct Pending {
    request: NativeLocalToolRequest,
    options: SessionOptions,
    ceiling: ReviewedCommandCeilingV1,
    command: ReviewedCommand,
    approval_id: String,
}

#[derive(Default)]
pub(super) struct ReviewedCommandHost {
    pending: Option<Pending>,
    running: Option<BoxFuture<'static, Result<RuntimeCommand, RuntimeError>>>,
    cancel: Option<oneshot::Sender<()>>,
    call_id: Option<String>,
    running_turn: Option<String>,
    waiting: VecDeque<Pending>,
    seen: HashSet<(Uuid, String, String)>,
    cancelled_events: Vec<RuntimeEventKind>,
}

impl ReviewedCommandHost {
    pub(super) fn request(
        &mut self,
        request: NativeLocalToolRequest,
        options: &SessionOptions,
        ceiling: &ReviewedCommandCeilingV1,
    ) -> Result<Vec<RuntimeEventKind>, RuntimeError> {
        if self.waiting.len() >= MAX_WAITING_COMMANDS
            || request.turn_id.is_empty()
            || request.call_id.is_empty()
        {
            return Err(reject("reviewed_commands_queue_or_identity_invalid"));
        }
        let command = ceiling
            .approve_host(&request.arguments)
            .ok_or_else(|| reject("reviewed_commands_request_outside_parent"))?;
        let identity = (
            options.generation,
            request.turn_id.clone(),
            request.call_id.clone(),
        );
        if !self.seen.insert(identity) {
            return Err(reject("reviewed_commands_duplicate_native_call"));
        }
        let command_id = command_generation(options.generation, &request.turn_id, &request.call_id);
        self.waiting.push_back(Pending {
            request,
            options: options.clone(),
            ceiling: ceiling.clone(),
            command,
            approval_id: format!("project-command:{command_id}"),
        });
        self.promote();
        Ok(self.take_cancelled_events())
    }

    fn promote(&mut self) {
        if self.pending.is_some() || self.running.is_some() {
            return;
        }
        if let Some(pending) = self.waiting.pop_front() {
            let mut details = pending.command.approval_context();
            details["commandId"] = json!(command_generation(
                pending.options.generation,
                &pending.request.turn_id,
                &pending.request.call_id
            ));
            details["parentGeneration"] = json!(pending.options.generation);
            self.cancelled_events
                .push(RuntimeEventKind::ApprovalRequested {
                    approval_id: pending.approval_id.clone(),
                    turn_id: pending.request.turn_id.clone(),
                    method: "infinishell/reviewed_project_command".into(),
                    details,
                });
            self.call_id = Some(pending.request.call_id.clone());
            self.pending = Some(pending);
        }
    }

    pub(super) fn owns_call(&self, call_id: &str) -> bool {
        (self.call_id.as_deref() == Some(call_id)
            && (self.pending.is_some() || self.running.is_some()))
            || self
                .waiting
                .iter()
                .any(|pending| pending.request.call_id == call_id)
    }

    pub(super) fn respond(
        &mut self,
        command: &RuntimeCommand,
        lifecycle: &mut ReviewedCommandLifecycle,
    ) -> Option<Result<Vec<RuntimeEventKind>, RuntimeError>> {
        let RuntimeAction::RespondApproval {
            approval_id,
            decision,
        } = &command.action
        else {
            return None;
        };
        if !approval_id.starts_with("project-command:") {
            return None;
        }
        Some(self.respond_inner(command, approval_id, *decision, lifecycle))
    }

    fn respond_inner(
        &mut self,
        envelope: &RuntimeCommand,
        approval_id: &str,
        decision: ApprovalDecision,
        lifecycle: &mut ReviewedCommandLifecycle,
    ) -> Result<Vec<RuntimeEventKind>, RuntimeError> {
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| {
                pending.approval_id == approval_id
                    && pending.options.generation == envelope.generation
            })
            .ok_or_else(|| reject("reviewed_commands_stale_approval"))?;
        if decision == ApprovalDecision::AllowOnce {
            pending.ceiling.verify_sources()?;
            if !lifecycle.can_approve() {
                return Err(reject("reviewed_commands_generation_already_used"));
            }
        }
        let pending = self.pending.take().expect("审批已匹配当前原生调用");
        let turn = pending.request.turn_id.clone();
        if decision == ApprovalDecision::AllowOnce {
            // 生命周期标记先于执行 future；取消不得把尚未派生的同一授权重新排队。
            lifecycle.start_host(
                turn.clone(),
                pending.request.call_id.clone(),
                &pending.command,
            )?;
            let (cancel, cancelled) = oneshot::channel();
            self.cancel = Some(cancel);
            self.running_turn = Some(turn.clone());
            self.running = Some(async move { execute(pending, cancelled).await }.boxed());
        } else {
            self.running =
                Some(async move { Ok(reply(&pending, Err("user_denied".into()))) }.boxed());
        }
        Ok(vec![
            RuntimeEventKind::ApprovalResolved {
                approval_id: approval_id.into(),
                decision,
            },
            RuntimeEventKind::CommandDispatched {
                message_id: envelope.message_id,
                turn_id: Some(turn),
            },
        ])
    }

    pub(super) async fn next_reply(
        &mut self,
        lifecycle: &mut ReviewedCommandLifecycle,
    ) -> Result<RuntimeCommand, RuntimeError> {
        let result = match self.running.as_mut() {
            Some(running) => running.await,
            None => futures::future::pending().await,
        };
        self.running.take();
        self.cancel.take();
        self.call_id.take();
        self.running_turn.take();
        let reply = result?;
        if let RuntimeAction::RespondLocalTool {
            call_id, turn_id, ..
        } = &reply.action
        {
            if lifecycle.owns_tool(call_id) {
                self.cancelled_events
                    .extend(lifecycle.host_cleanup_confirmed(turn_id, call_id)?);
            }
        }
        // execute 只有确认子命令完整清理才返回；拒绝审批的分支从未派生进程。
        if !lifecycle.retiring() {
            self.promote();
        }
        Ok(reply)
    }

    pub(super) fn cancel_turn(&mut self, turn: &str) -> Vec<RuntimeCommand> {
        let mut cancelled = Vec::new();
        let mut retained = VecDeque::new();
        while let Some(pending) = self.waiting.pop_front() {
            if pending.request.turn_id == turn {
                cancelled.push(reply(&pending, Err("cancelled_before_approval".into())));
            } else {
                retained.push_back(pending);
            }
        }
        self.waiting = retained;
        if self.running_turn.as_deref() == Some(turn) {
            if let Some(cancel) = self.cancel.take() {
                let _ = cancel.send(());
            }
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request.turn_id == turn)
        {
            let pending = self.pending.take().expect("已核对被取消回合");
            cancelled.push(reply(&pending, Err("cancelled_before_spawn".into())));
            self.cancelled_events
                .push(RuntimeEventKind::ApprovalCancelled {
                    approval_id: pending.approval_id,
                });
            self.call_id.take();
        }
        self.promote();
        cancelled
    }

    pub(super) fn cancel_call(&mut self, call_id: &str) {
        self.waiting
            .retain(|pending| pending.request.call_id != call_id);
        if self.running.is_some() && self.call_id.as_deref() == Some(call_id) {
            if let Some(cancel) = self.cancel.take() {
                let _ = cancel.send(());
            }
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request.call_id == call_id)
        {
            let pending = self.pending.take().expect("已核对原生取消调用");
            self.cancelled_events
                .push(RuntimeEventKind::ApprovalCancelled {
                    approval_id: pending.approval_id,
                });
            self.call_id.take();
        }
        self.promote();
    }

    pub(super) fn cancel(&mut self) {
        self.waiting.clear();
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(pending) = self.pending.take() {
            self.cancelled_events
                .push(RuntimeEventKind::ApprovalCancelled {
                    approval_id: pending.approval_id,
                });
        }
    }

    pub(super) fn take_cancelled_events(&mut self) -> Vec<RuntimeEventKind> {
        std::mem::take(&mut self.cancelled_events)
    }

    pub(super) async fn finish(&mut self) -> Result<(), RuntimeError> {
        self.cancel();
        if let Some(running) = self.running.take() {
            // 不使用 abort/drop 替代回执；即使 SDK 连接先断开，也继续等待真实清理。
            running.await?;
        }
        self.call_id.take();
        Ok(())
    }
}

fn reply(pending: &Pending, result: Result<Value, String>) -> RuntimeCommand {
    RuntimeCommand {
        generation: pending.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::RespondLocalTool {
            call_id: pending.request.call_id.clone(),
            turn_id: pending.request.turn_id.clone(),
            result,
        },
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CommandRecord {
    version: u32,
    parent_generation: Uuid,
    command_generation: Uuid,
    turn_id: String,
    call_id: String,
    approval_id: String,
    command_sha256: String,
    platform: String,
}

fn command_generation(parent: Uuid, turn: &str, call: &str) -> Uuid {
    let hash = Sha256::digest(
        serde_json::to_vec(&("infinishell-project-command-v1", parent, turn, call))
            .expect("命令身份可序列化"),
    );
    Uuid::from_bytes(hash[..16].try_into().expect("SHA-256 至少 16 字节"))
}

fn state_for_parent(state: &Path, generation: Uuid) -> PathBuf {
    dunce::simplified(state)
        .join("reviewed-project-commands")
        .join(generation.to_string())
}

async fn execute(
    pending: Pending,
    mut cancelled: oneshot::Receiver<()>,
) -> Result<RuntimeCommand, RuntimeError> {
    if cancelled
        .try_recv()
        .map_err(|_| RuntimeError::ControllerClosed)?
        .is_some()
    {
        return Ok(reply(&pending, Err("cancelled_before_spawn".into())));
    }
    pending.ceiling.verify_sources()?;
    let child_generation = command_generation(
        pending.options.generation,
        &pending.request.turn_id,
        &pending.request.call_id,
    );
    let state = state_for_parent(&pending.options.state_dir, pending.options.generation)
        .join(child_generation.to_string());
    fs::create_dir_all(&state)?;
    if dunce::canonicalize(&state)? != state {
        return Err(reject("reviewed_commands_state_redirected"));
    }
    let bytes = serde_json::to_vec(&CommandRecord {
        version: 1,
        platform: std::env::consts::OS.into(),
        parent_generation: pending.options.generation,
        command_generation: child_generation,
        turn_id: pending.request.turn_id.clone(),
        call_id: pending.request.call_id.clone(),
        approval_id: pending.approval_id.clone(),
        command_sha256: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&pending.command).expect("命令可序列化"))
        ),
    })
    .expect("命令身份可序列化");
    // 每个原生调用仅能领取一次；恢复只核收据，绝不重新执行记录中的命令。
    let mut record = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(state.join(RECORD_NAME))?;
    record.write_all(&bytes)?;
    record.sync_all()?;
    let mut raw = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(state.join("stdout.bin"))?;
    let mut child = match spawn_command(&state, child_generation, &pending.command).await {
        Ok(child) => child,
        Err(error) => {
            // 握手失败也可能已经启动监督者；只有真实终态才能解除父任务的占用。
            let cleanup = async {
                loop {
                    // 派生 future 已经返回错误，此后目录缺失才可证明未启动。
                    match managed_process::process_completion(&state, child_generation)? {
                        ProcessCompletion::NotStarted => {
                            let mut marker = OpenOptions::new()
                                .create_new(true)
                                .write(true)
                                .open(state.join("not-started.json"))?;
                            marker.write_all(&bytes)?;
                            marker.sync_all()?;
                            return Ok::<_, io::Error>(());
                        }
                        ProcessCompletion::Exited(receipt)
                            if receipt.cleanup_confirmed
                                && valid_containment(&receipt.containment) =>
                        {
                            return Ok(());
                        }
                        ProcessCompletion::Exited(_) | ProcessCompletion::Unconfirmed => {}
                    }
                    async_io::Timer::after(Duration::from_millis(100)).await;
                }
            };
            cleanup
                .with_timeout(Duration::from_secs(30))
                .await
                .map_err(|_| RuntimeError::RequestTimedOut)??;
            return Err(error.into());
        }
    };
    let stdin = child.stdin.take();
    let Some(mut stdout) = child.stdout.take() else {
        drop(stdin);
        child.finish().await?;
        return Err(RuntimeError::Protocol(
            "reviewed_command_stdout_missing".into(),
        ));
    };
    let mut output = Vec::new();
    let mut raw_bytes = 0_u64;
    let mut raw_sha = Sha256::new();
    let outcome = {
        let read = collect_output(
            &mut stdout,
            &mut raw,
            &mut output,
            &mut raw_bytes,
            &mut raw_sha,
            Some(OUTPUT_BUDGET as u64),
        )
        .fuse();
        let timeout =
            async_io::Timer::after(Duration::from_millis(pending.command.timeout_ms)).fuse();
        let cancelled = cancelled.fuse();
        pin_mut!(read, timeout, cancelled);
        select! {
            result = read => result.map_err(|error| error.to_string()),
            _ = timeout => Err("reviewed_command_timeout".into()),
            _ = cancelled => Err("reviewed_command_cancelled".into()),
        }
    };
    drop(stdin);
    // 到输出预算立即停止整条命令，但仍将清理期间管道中实际收到的原始字节落盘。
    let drain = collect_output(
        &mut stdout,
        &mut raw,
        &mut output,
        &mut raw_bytes,
        &mut raw_sha,
        None,
    );
    let (receipt, drained) =
        futures::join!(child.finish(), drain.with_timeout(Duration::from_secs(30)));
    raw.sync_all()?;
    let receipt = receipt?;
    drained.map_err(|_| RuntimeError::RequestTimedOut)??;
    if !receipt.cleanup_confirmed || !valid_containment(&receipt.containment) {
        return Err(reject("reviewed_commands_cleanup_unconfirmed"));
    }
    let failed = outcome.is_err() || receipt.exit_code != Some(0);
    let value = json!({"commandGeneration":child_generation,"exitCode":receipt.exit_code,
        "output":String::from_utf8_lossy(&output),"outputPreviewTruncated":raw_bytes > output.len() as u64,
        "outputRawPath":state.join("stdout.bin"),"outputBytes":raw_bytes,
        "outputSha256":format!("{:x}",raw_sha.finalize()),
        "error":outcome.err(),"cleanup":receipt,"osSandbox":false});
    let mut saved = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(state.join("result.json"))?;
    saved.write_all(&serde_json::to_vec(&value).expect("结果可序列化"))?;
    saved.sync_all()?;
    Ok(reply(
        &pending,
        if failed {
            Err(value.to_string())
        } else {
            Ok(value)
        },
    ))
}

async fn collect_output(
    stdout: &mut (impl futures::io::AsyncRead + Unpin),
    raw: &mut fs::File,
    preview: &mut Vec<u8>,
    count: &mut u64,
    sha: &mut Sha256,
    limit: Option<u64>,
) -> io::Result<()> {
    let mut chunk = [0_u8; 8192];
    loop {
        let length = stdout.read(&mut chunk).await?;
        if length == 0 {
            return Ok(());
        }
        raw.write_all(&chunk[..length])?;
        sha.update(&chunk[..length]);
        *count += length as u64;
        let keep = length.min(OUTPUT_BUDGET.saturating_sub(preview.len()));
        preview.extend_from_slice(&chunk[..keep]);
        if limit.is_some_and(|limit| *count > limit) {
            return Err(io::Error::other("reviewed_command_output_budget"));
        }
    }
}

async fn spawn_command(
    state: &Path,
    generation: Uuid,
    command: &ReviewedCommand,
) -> io::Result<managed_process::ManagedChild> {
    #[cfg(windows)]
    {
        managed_process::spawn_reviewed_windows_command(state, generation, command).await
    }
    #[cfg(unix)]
    {
        managed_process::spawn_reviewed_unix_command(state, generation, command).await
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (state, generation, command);
        Err(io::Error::other("项目命令平台未校准"))
    }
}

fn valid_containment(containment: &str) -> bool {
    matches!(
        (std::env::consts::OS, containment),
        ("windows", "windows_job")
            | ("linux", "linux_subtree")
            | ("macos", "macos_resource_coalition")
    )
}

/// 父 CLI 回收必须同时确认所有独立命令；缺收据、未知状态与改写身份均不能放行。
pub(super) fn cleanup_for_parent(state: &Path, parent: Uuid) -> io::Result<bool> {
    let root = state_for_parent(state, parent);
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
        Ok(metadata) if metadata.is_dir() && dunce::canonicalize(&root)? == root => {}
        Ok(_) => return Err(io::Error::other("项目命令记录目录身份无效")),
    }
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let directory = entry.path();
        if !entry.file_type()?.is_dir() || dunce::canonicalize(&directory)? != directory {
            return Err(io::Error::other("项目命令记录目录被重定向"));
        }
        let path = directory.join(RECORD_NAME);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file()
            || metadata.len() > 16 * 1024
            || dunce::canonicalize(&path)? != path
        {
            return Err(io::Error::other("项目命令领取记录无效"));
        }
        let record: CommandRecord =
            serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if record.version != 1
            || record.parent_generation != parent
            || record.platform != std::env::consts::OS
            || record.command_generation
                != command_generation(parent, &record.turn_id, &record.call_id)
            || directory.file_name().and_then(|name| name.to_str())
                != Some(record.command_generation.to_string().as_str())
        {
            return Err(io::Error::other("项目命令代次绑定改变"));
        }
        let complete =
            match managed_process::process_completion(&directory, record.command_generation)? {
                ProcessCompletion::NotStarted => {
                    let marker = directory.join("not-started.json");
                    match fs::symlink_metadata(&marker) {
                        Ok(metadata)
                            if metadata.file_type().is_file()
                                && metadata.len() <= 16 * 1024
                                && dunce::canonicalize(&marker)? == marker =>
                        {
                            fs::read(marker)?
                                == serde_json::to_vec(&record).map_err(io::Error::other)?
                        }
                        Ok(_) => return Err(io::Error::other("项目命令未启动凭据无效")),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                        Err(error) => return Err(error),
                    }
                }
                ProcessCompletion::Exited(receipt) => {
                    let binding = launch_binding(record.command_sha256)?;
                    receipt.cleanup_confirmed
                        && valid_containment(&receipt.containment)
                        && managed_process::confirmed_exit_with_binding(
                            &directory,
                            record.command_generation,
                            &binding,
                        )?
                        .is_some()
                }
                ProcessCompletion::Unconfirmed => false,
            };
        if !complete {
            return Ok(false);
        }
    }
    Ok(true)
}

fn launch_binding(digest: String) -> io::Result<managed_process::PreparedLaunchBinding> {
    #[cfg(windows)]
    {
        managed_process::PreparedLaunchBinding::windows_reviewed_project_command(digest)
    }
    #[cfg(unix)]
    {
        managed_process::PreparedLaunchBinding::unix_reviewed_project_command(digest)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = digest;
        Err(io::Error::other("项目命令平台未校准"))
    }
}
