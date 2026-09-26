//! 远程图片提交租约：传输、文件引用和原生派发分别记录，取消不能冒充消费。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use sha2::{Digest, Sha256};
use uuid::Uuid;
use warp_core::SessionId;
use warp_core::cli_agent_protocol::{ClaudeProcessEvidence, CodexProcessEvidence};
use warpui::{AppContext, EntityId};

use super::cli_image_submission_journal as journal;

use super::client::RemoteServerClient;
use super::proto::{
    CliImageStagingActivate, CliImageStagingBegin, CliImageStagingChunk, CliImageStagingPublish,
    CliImageStagingRequest, CliImageStagingResponse, CliImageStagingScope, CliImageStagingSpec,
    CliImageStagingVerify, cli_image_staging_request::Action,
    cli_image_staging_response::Result as Response,
};

const CHUNK_BYTES: usize = 1024 * 1024;
const MAX_BATCH_BYTES: usize = 500_000_000;
const MAX_BATCH_IMAGES: usize = 20;

#[derive(Default)]
struct Registry {
    active: HashMap<EntityId, Arc<RemoteImageSubmission>>,
    unknown: HashSet<(EntityId, EntityId, EntityId, String, [u8; 32])>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NativeImageBinding {
    pub(crate) session_id: SessionId,
    pub(crate) native_session_id: String,
    pub(crate) generation: Uuid,
    pub(crate) listener_id: EntityId,
    pub(crate) model_events_id: EntityId,
    pub(crate) block_id: String,
    pub(crate) cwd: String,
    pub(crate) consumer: NativeImageConsumer,
    pub(crate) codex_owned: Option<(String, String)>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum NativeImageConsumer {
    Codex(CodexProcessEvidence),
    Claude {
        process: ClaudeProcessEvidence,
        transcript_path: String,
    },
}

impl NativeImageConsumer {
    pub(crate) fn available(&self, client: &RemoteServerClient) -> bool {
        match self {
            Self::Codex(_) => client.cli_image_codex_queue_available(),
            Self::Claude { .. } => client.cli_image_claude_read_available(),
        }
    }

    pub(crate) fn is_claude(&self) -> bool {
        matches!(self, Self::Claude { .. })
    }
}

static SUBMISSIONS: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

struct State {
    live: bool,
    // Publish 的响应丢失也可能已发布，因此发请求前登记编号。
    references: Vec<journal::Intent>,
    handed_to_native: bool,
    pending_lease: Option<journal::PendingLease>,
}

pub(crate) struct RemoteImageSubmission {
    pub(crate) client: Arc<RemoteServerClient>,
    pub(crate) scope: CliImageStagingScope,
    revision: u64,
    binding: NativeImageBinding,
    view_id: EntityId,
    state: Mutex<State>,
}

impl RemoteImageSubmission {
    pub(crate) fn begin(
        view_id: EntityId,
        client: Arc<RemoteServerClient>,
        binding: NativeImageBinding,
    ) -> Result<Arc<Self>, ()> {
        if !binding.consumer.available(&client) {
            return Err(());
        }
        let (scope, revision) = client
            .allocate_cli_image_scope(
                binding.session_id,
                &binding.native_session_id,
                binding.generation,
                Uuid::new_v4(),
            )
            .map_err(|_| ())?;
        let submission = Arc::new(Self {
            client,
            scope,
            revision,
            binding,
            view_id,
            state: Mutex::new(State {
                live: true,
                references: Vec::new(),
                handed_to_native: false,
                pending_lease: None,
            }),
        });
        let previous = SUBMISSIONS
            .lock()
            .expect("image registry poisoned")
            .active
            .insert(view_id, submission.clone());
        if let Some(previous) = previous {
            previous.revoke();
        }
        Ok(submission)
    }

    pub(crate) fn native_write_was_claimed(&self) -> bool {
        self.state
            .lock()
            .expect("image submission poisoned")
            .handed_to_native
    }

    pub(crate) fn is_live(&self) -> bool {
        self.client.cli_image_scope_is_current(&self.scope)
            && self.state.lock().expect("image submission poisoned").live
    }

    pub(crate) fn revoke(&self) {
        let mut state = self.state.lock().expect("image submission poisoned");
        state.live = false;
        self.client
            .revoke_cli_image_scope(self.scope.clone(), self.revision);
        if !state.handed_to_native {
            for reference in state.references.drain(..) {
                self.client.release_cli_image_reference(
                    self.scope.clone(),
                    self.revision,
                    reference.transfer_id,
                    reference.recovery_key,
                );
            }
            state.pending_lease = None;
        }
    }

    /// 在任何原生写入前调用；一旦尝试过图片派发，失败草稿不能自动重投。
    pub(crate) fn claim_native_write(self: &Arc<Self>, subject: [u8; 32]) -> bool {
        let mut registry = SUBMISSIONS.lock().expect("image registry poisoned");
        let mut state = self.state.lock().expect("image submission poisoned");
        if !state.live
            || self.client.is_disconnected()
            || !registry
                .active
                .get(&self.view_id)
                .is_some_and(|active| Arc::ptr_eq(active, self))
            || !registry.unknown.insert((
                self.view_id,
                self.binding.listener_id,
                self.binding.model_events_id,
                self.scope.cli_session_id.clone(),
                subject,
            ))
        {
            return false;
        }
        state.handed_to_native = true;
        true
    }

    pub(crate) fn prepare_native_queue(
        &self,
        subject: [u8; 32],
    ) -> Result<journal::QueueIntent, ()> {
        if !self.is_live() {
            return Err(());
        }
        journal::Journal::open()
            .and_then(|journal| {
                journal.prepare_queue(&self.scope, subject, self.binding.consumer.is_claude())
            })
            .map_err(|_| ())
    }

    /// 用户输入只进入当前原生 daemon 的类型化队列；不发送 PTY 字节或审批答复。
    pub(crate) async fn submit_native_queue(
        &self,
        text: String,
        subject: [u8; 32],
        intent: journal::QueueIntent,
    ) -> Result<bool, ()> {
        let references = {
            let state = self.state.lock().expect("image submission poisoned");
            if !state.live || !state.handed_to_native {
                return Err(());
            }
            state
                .references
                .iter()
                .map(|reference| super::proto::CliImageQueueReference {
                    transfer_id: reference.transfer_id.to_string(),
                    recovery_key: reference.recovery_key.to_string(),
                })
                .collect()
        };
        let action = match &self.binding.consumer {
            NativeImageConsumer::Codex(process) => {
                Action::CodexQueue(super::proto::CliImageCodexQueue {
                    binding: Some(super::proto::CliImageCodexBinding {
                        owned_ticket_id: self
                            .binding
                            .codex_owned
                            .as_ref()
                            .map(|proof| proof.0.clone())
                            .unwrap_or_default(),
                        owned_manifest_sha256: self
                            .binding
                            .codex_owned
                            .as_ref()
                            .map(|proof| proof.1.clone())
                            .unwrap_or_default(),
                        daemon_pid_candidate: process.daemon_pid_candidate,
                        codex_home: process.codex_home.clone(),
                        tty_path: process.tty_path.clone(),
                        working_directory: self.binding.cwd.clone(),
                    }),
                    text,
                    references,
                    subject_sha256: subject.to_vec(),
                    recovery_key: intent.recovery_key.to_string(),
                })
            }
            NativeImageConsumer::Claude {
                process,
                transcript_path,
            } => Action::ClaudeQueue(super::proto::CliImageClaudeQueue {
                binding: Some(super::proto::CliImageClaudeBinding {
                    process_id_candidate: process.process_id_candidate,
                    claude_config_directory: process.claude_config_directory.clone(),
                    tty_path: process.tty_path.clone(),
                    working_directory: self.binding.cwd.clone(),
                    transcript_path: transcript_path.clone(),
                }),
                text,
                references,
                subject_sha256: subject.to_vec(),
                recovery_key: intent.recovery_key.to_string(),
            }),
        };
        let response = self
            .client
            .stage_unpublished_cli_image(CliImageStagingRequest {
                scope: Some(self.scope.clone()),
                revision: self.revision,
                action: Some(action),
            })
            .await
            .map_err(|_| ())?;
        let mut result = match (self.binding.consumer.is_claude(), response.result) {
            (false, Some(Response::CodexQueue(result)))
            | (true, Some(Response::ClaudeQueue(result))) => result,
            _ => return Err(()),
        };
        // Claude 原生 Read 可能等待用户审批；这里只查询原提交，任何状态下均不重发。
        if self.binding.consumer.is_claude() {
            for _ in 0..120 {
                if result.status != "unknown" || !self.is_live() {
                    break;
                }
                async_io::Timer::after(std::time::Duration::from_secs(1)).await;
                let response = self
                    .client
                    .stage_unpublished_cli_image(CliImageStagingRequest {
                        scope: Some(self.scope.clone()),
                        revision: self.revision,
                        action: Some(Action::ClaudeQueueStatus(
                            super::proto::CliImageCodexQueueStatus {
                                submission_id: self.scope.submission_id.clone(),
                                recovery_key: intent.recovery_key.to_string(),
                            },
                        )),
                    })
                    .await
                    .map_err(|_| ())?;
                let Some(Response::ClaudeQueue(observed)) = response.result else {
                    return Err(());
                };
                result = observed;
            }
        }
        // 即使 UI 已切换，也必须保存属于原请求的精确 ACK；随后 UI 自行核对代际。
        let journal = journal::Journal::open().map_err(|_| ())?;
        let accepted = journal
            .record_queue_result(&intent, &result)
            .map_err(|_| ())?;
        if matches!(result.status.as_str(), "confirmed" | "rejected") {
            let _ = journal
                .recover(&self.client, &self.scope, self.revision)
                .await;
            let mut state = self.state.lock().expect("image submission poisoned");
            state.references.clear();
            state.pending_lease = None;
            drop(state);
            SUBMISSIONS
                .lock()
                .expect("image registry poisoned")
                .unknown
                .remove(&(
                    self.view_id,
                    self.binding.listener_id,
                    self.binding.model_events_id,
                    self.scope.cli_session_id.clone(),
                    subject,
                ));
        }
        Ok(accepted)
    }

    async fn request(&self, action: Action) -> Result<CliImageStagingResponse, ()> {
        if !self.is_live() {
            return Err(());
        }
        let revision = if matches!(action, Action::Activate(_)) {
            self.revision - 1
        } else {
            self.revision
        };
        let response = self
            .client
            .stage_unpublished_cli_image(CliImageStagingRequest {
                scope: Some(self.scope.clone()),
                revision,
                action: Some(action),
            })
            .await
            .map_err(|_| ())?;
        if !self.is_live() || matches!(response.result, Some(Response::Error(_))) {
            return Err(());
        }
        Ok(response)
    }

    /// 逐张传输并核对原字节；整个批次完成前不写入原生终端。
    pub(crate) async fn prepare(&self, png_images: Vec<Vec<u8>>) -> Result<Vec<String>, ()> {
        let journal = journal::Journal::open().map_err(|_| ())?;
        let lease = journal
            .lease(Uuid::parse_str(&self.scope.submission_id).map_err(|_| ())?)
            .map_err(|_| ())?;
        {
            let mut state = self.state.lock().expect("image submission poisoned");
            if !state.live {
                return Err(());
            }
            state.pending_lease = Some(lease);
        }
        journal
            .recover(&self.client, &self.scope, self.revision)
            .await
            .map_err(|_| ())?;
        let total = png_images
            .iter()
            .try_fold(0usize, |total, bytes| total.checked_add(bytes.len()))
            .ok_or(())?;
        if png_images.is_empty() || png_images.len() > MAX_BATCH_IMAGES || total > MAX_BATCH_BYTES {
            return Err(());
        }
        self.request(Action::Activate(CliImageStagingActivate {}))
            .await?;
        let mut paths = Vec::with_capacity(png_images.len());
        for bytes in png_images {
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(());
            }
            let transfer_id = Uuid::new_v4();
            let spec = CliImageStagingSpec {
                byte_len: bytes.len() as u64,
                sha256: Sha256::digest(&bytes).to_vec(),
            };
            let intent = journal::Intent::new(&self.scope, transfer_id, &spec).map_err(|_| ())?;
            // Publish 请求发出前即保存恢复凭据，丢失响应仍可精确找回/撤销引用。
            journal.persist_intent(&intent).map_err(|_| ())?;
            self.request(Action::Begin(CliImageStagingBegin {
                transfer_id: transfer_id.to_string(),
                spec: Some(spec.clone()),
            }))
            .await?;
            for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
                let offset = index as u64 * CHUNK_BYTES as u64;
                let response = self
                    .request(Action::Chunk(CliImageStagingChunk {
                        transfer_id: transfer_id.to_string(),
                        offset,
                        bytes: chunk.to_vec(),
                    }))
                    .await?;
                if !matches!(response.result, Some(Response::Progress(progress)) if progress.next_offset == offset + chunk.len() as u64)
                {
                    return Err(());
                }
            }
            self.request(Action::Verify(CliImageStagingVerify {
                transfer_id: transfer_id.to_string(),
                expected_spec: Some(spec.clone()),
            }))
            .await?;
            {
                let mut state = self.state.lock().expect("image submission poisoned");
                if !state.live {
                    return Err(());
                }
                state.references.push(intent.clone());
            }
            let response = self
                .request(Action::Publish(CliImageStagingPublish {
                    transfer_id: transfer_id.to_string(),
                    recovery_key: intent.recovery_key.to_string(),
                }))
                .await?;
            let Some(Response::Published(published)) = response.result else {
                return Err(());
            };
            // 服务端当前只发布 Unix 绝对 PNG 路径；不允许终端控制字节或转义语法。
            if published.spec != Some(spec)
                || !published.remote_path.starts_with('/')
                || !published.remote_path.ends_with(".png")
                || published.remote_path.chars().count() > 512
                || published
                    .remote_path
                    .chars()
                    .any(|ch| ch.is_control() || matches!(ch, '\'' | '"' | '\\'))
            {
                return Err(());
            }
            paths.push(published.remote_path);
        }
        Ok(paths)
    }
}

impl Drop for RemoteImageSubmission {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// 会话、listener、输入代际、PTY 或 Ctrl-C 变化时同步调用；不等异步工作结束。
pub(crate) fn revoke(view_id: EntityId) {
    let active = SUBMISSIONS
        .lock()
        .expect("image registry poisoned")
        .active
        .remove(&view_id);
    if let Some(active) = active {
        active.revoke();
    }
}

/// 连接恢复只查询持久状态和执行明确回收，不重发任何输入。
pub(crate) fn schedule_reference_recovery(
    client: Arc<RemoteServerClient>,
    session_id: SessionId,
    ctx: &AppContext,
) {
    let Ok((scope, revision)) = client.allocate_cli_image_scope(
        session_id,
        "reference-recovery",
        Uuid::new_v4(),
        Uuid::new_v4(),
    ) else {
        return;
    };
    ctx.background_executor()
        .spawn(async move {
            if let Ok(journal) = journal::Journal::open() {
                let _ = journal.recover(&client, &scope, revision).await;
            }
        })
        .detach();
}
