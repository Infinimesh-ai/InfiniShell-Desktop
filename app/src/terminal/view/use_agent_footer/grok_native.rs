//! 普通 Grok PTY 通过原生原子桥提交；持久领取前不发送，未知回执只查询。

use std::rc::Rc;
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use serde_json::json;
use uuid::Uuid;
use warpui::{AppContext, EntityId, SingletonEntity, ViewContext};

use super::{CliInputSubmission, TerminalView, file_attachments};
use crate::persistence::local_cli_tasks::grok_native_bridge::{
    self as ledger, NativeBridgeClaimOutcome, NativeBridgeDeliveryState, NativeBridgeInput,
    NativeBridgeInputRecord,
};
use crate::persistence::{ModelEvent, PersistenceWriter};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::cli_agent_sessions::grok_native_bridge::{
    NativeBridge, NativeBridgeLease, NativeBridgeMessage, NativeBridgeReceiptState,
    NativeBridgeResponse,
};
use crate::terminal::cli_agent_sessions::grok_native_bridge_root;
use crate::terminal::cli_agent_sessions::grok_native_bridge_submission::SubmissionLease;
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;
use crate::terminal::view::{BlockId, SessionId};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

#[derive(Clone, PartialEq, Eq)]
struct InputTarget {
    pty: LocalPtyIdentity,
    block: BlockId,
    session: SessionId,
    model_events: EntityId,
    native_session: Option<String>,
    listener: Option<EntityId>,
    cwd: String,
}

struct PreparedInput {
    bridge: NativeBridge,
    lease: Option<NativeBridgeLease>,
    previous: Option<NativeBridgeInputRecord>,
    session_id: Uuid,
    body: String,
    replaces: Option<Uuid>,
}

enum DeliveryOutcome {
    Acknowledged,
    AlreadyAcknowledged(Uuid),
    Unconfirmed,
}

impl TerminalView {
    fn native_grok_input_target(&self, ctx: &AppContext) -> Option<InputTarget> {
        let cli = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id)?;
        if cli.agent != CLIAgent::Grok || cli.remote_host.is_some() {
            return None;
        }
        let model = self.model.lock();
        let block = model.block_list().active_block();
        if !block.is_active_and_long_running()
            || model.shared_session_status().is_sharer_or_viewer()
            || model.is_conversation_transcript_viewer()
            || model
                .shell_launch_state()
                .available_shell()
                .is_none_or(|shell| shell.is_docker_sandbox())
        {
            return None;
        }
        let session_id = block.session_id()?;
        let session = self.sessions.as_ref(ctx).get(session_id)?;
        if !session.is_local()
            || session.is_subshell_or_ssh()
            || session.is_wsl()
            || session.is_msys2()
        {
            return None;
        }
        let cwd = session
            .launch_data()?
            .maybe_convert_absolute_path(block.metadata().current_working_directory()?)?;
        Some(InputTarget {
            pty: model.local_pty_identity()?,
            block: block.id().clone(),
            session: session_id,
            model_events: self.model_events_handle.id(),
            native_session: cli.session_context.session_id.clone(),
            listener: cli.listener.as_ref().map(|listener| listener.id()),
            cwd: cwd.to_str()?.to_owned(),
        })
    }

    pub(super) fn submit_native_grok_input(
        &mut self,
        text: String,
        generation: Uuid,
        ctx: &mut ViewContext<Self>,
    ) {
        self.submit_native_grok_attempt(text, generation, None, ctx);
    }

    fn submit_native_grok_attempt(
        &mut self,
        text: String,
        generation: Uuid,
        repeat: Option<Uuid>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(target) = self.native_grok_input_target(ctx) else {
            self.reject_unsafe_cli_agent_input(generation, Some(text), ctx);
            return;
        };
        if !self
            .ai_context_model
            .as_ref(ctx)
            .pending_images()
            .is_empty()
        {
            self.show_error_toast(crate::t!("cli-agent-grok-native-images-unavailable"), ctx);
            return;
        }
        let Some(database) = PersistenceWriter::as_ref(ctx).sender() else {
            self.show_error_toast(crate::t!("cli-agent-task-save-failed"), ctx);
            return;
        };
        if !self.cli_agent_input_target_matches(generation, ctx) {
            self.show_error_toast(crate::t!("cli-agent-input-target-changed"), ctx);
            return;
        }
        if !CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
            sessions.begin_input_submission(self.view_id, generation)
        }) {
            self.show_error_toast(crate::t!("cli-agent-input-still-sending"), ctx);
            return;
        }
        let snapshot = Rc::new(CliInputSubmission {
            agent: CLIAgent::Grok,
            query: text.clone(),
            editor_revision: self
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx),
            attachments_revision: self
                .ai_context_model
                .as_ref(ctx)
                .pending_attachments_revision(),
        });
        let files = self
            .ai_context_model
            .as_ref(ctx)
            .pending_files()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let observed_session = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .and_then(|session| session.session_context.session_id.clone());
        let preparation_target = target.clone();
        let preparation_database = database.clone();
        ctx.spawn(
            async move {
                let body = if files.is_empty() {
                    text
                } else {
                    file_attachments::prepare_file_attachments(text, files).await?
                };
                blocking::unblock(move || {
                    prepare_input(
                        body,
                        &preparation_target,
                        observed_session,
                        repeat,
                        &preparation_database,
                    )
                })
                .await
            },
            move |view, result, ctx| {
                let current = view.native_grok_input_target(ctx).as_ref() == Some(&target)
                    && view.cli_agent_input_generation_matches(generation, ctx)
                    && view
                        .input
                        .as_ref(ctx)
                        .editor()
                        .as_ref(ctx)
                        .buffer_revision(ctx)
                        == snapshot.editor_revision
                    && view
                        .ai_context_model
                        .as_ref(ctx)
                        .pending_attachments_revision()
                        == snapshot.attachments_revision;
                if !current {
                    view.fail_cli_agent_text_submit(
                        generation,
                        crate::t!("cli-agent-input-target-changed"),
                        ctx,
                    );
                    return;
                }
                let prepared = match result {
                    Ok(Some(prepared)) => prepared,
                    Ok(None) => {
                        view.reject_unsafe_cli_agent_input(
                            generation,
                            Some(snapshot.query.clone()),
                            ctx,
                        );
                        return;
                    }
                    Err(message) => {
                        view.fail_cli_agent_text_submit(generation, message, ctx);
                        return;
                    }
                };
                let Some(authorization) =
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                        sessions.register_native_grok_submission(
                            view.view_id,
                            generation,
                            prepared.session_id.to_string(),
                        )
                    })
                else {
                    view.fail_cli_agent_text_submit(
                        generation,
                        crate::t!("cli-agent-input-target-changed"),
                        ctx,
                    );
                    return;
                };
                let worker_target = target.clone();
                ctx.spawn(
                    async move {
                        blocking::unblock(move || {
                            deliver_input(prepared, worker_target, authorization, database)
                        })
                        .await
                    },
                    move |view, result, ctx| {
                        if view.native_grok_input_target(ctx).as_ref() != Some(&target) {
                            view.release_cli_agent_input_submission(generation, ctx);
                            return;
                        }
                        match result {
                            Ok(DeliveryOutcome::Acknowledged) => {
                                view.complete_cli_agent_text_submit(generation, Some(snapshot), ctx)
                            }
                            Ok(DeliveryOutcome::AlreadyAcknowledged(message_id)) => {
                                view.release_cli_agent_input_submission(generation, ctx);
                                if view.cli_agent_input_generation_matches(generation, ctx) {
                                    view.offer_native_grok_repeat(
                                        generation, snapshot, target, message_id, ctx,
                                    );
                                }
                            }
                            Ok(DeliveryOutcome::Unconfirmed) => {
                                view.fail_cli_agent_text_submit(
                                    generation,
                                    crate::t!("cli-agent-grok-owned-input-claimed"),
                                    ctx,
                                );
                            }
                            Err(message) => {
                                view.fail_cli_agent_text_submit(generation, message, ctx)
                            }
                        }
                    },
                );
            },
        );
    }

    fn offer_native_grok_repeat(
        &mut self,
        generation: Uuid,
        snapshot: Rc<CliInputSubmission>,
        target: InputTarget,
        message_id: Uuid,
        ctx: &mut ViewContext<Self>,
    ) {
        let view = ctx.handle();
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |stack, ctx| {
            stack.add_ephemeral_toast(
                DismissibleToast::error(crate::t!("cli-agent-grok-native-already-received"))
                    .with_object_id(format!("grok-repeat-{message_id}"))
                    .with_on_body_click(move |ctx| {
                        let Some(view) = view.upgrade(ctx) else {
                            return;
                        };
                        view.update(ctx, |view, ctx| {
                            if view.cli_agent_input_generation_matches(generation, ctx)
                                && view.native_grok_input_target(ctx).as_ref() == Some(&target)
                                && view
                                    .input
                                    .as_ref(ctx)
                                    .editor()
                                    .as_ref(ctx)
                                    .buffer_revision(ctx)
                                    == snapshot.editor_revision
                                && view
                                    .ai_context_model
                                    .as_ref(ctx)
                                    .pending_attachments_revision()
                                    == snapshot.attachments_revision
                            {
                                view.submit_native_grok_attempt(
                                    snapshot.query.clone(),
                                    generation,
                                    Some(message_id),
                                    ctx,
                                );
                            } else {
                                view.show_error_toast(
                                    crate::t!("cli-agent-input-target-changed"),
                                    ctx,
                                );
                            }
                        });
                    }),
                window_id,
                ctx,
            );
        });
    }
}

fn prepare_input(
    body: String,
    target: &InputTarget,
    observed_session: Option<String>,
    repeat: Option<Uuid>,
    database: &SyncSender<ModelEvent>,
) -> Result<Option<PreparedInput>, String> {
    let unavailable = || crate::t!("cli-agent-grok-owned-input-unavailable");
    let root = grok_native_bridge_root::root().map_err(|_| unavailable())?;
    let Some(bridge) = NativeBridge::discover(&root, target.pty.clone()).map_err(|_| unavailable())? else {
        return Ok(None);
    };
    let observed_session = observed_session
        .map(|session| Uuid::parse_str(&session))
        .transpose()
        .map_err(|_| unavailable())?;
    let binding = serde_json::to_value(bridge.binding()).map_err(|_| unavailable())?;
    let lookup = |session| {
        block_on(
            ledger::lookup_input(database, binding.clone(), session, body.clone())
                .map_err(|_| crate::t!("cli-agent-task-save-failed"))?,
        )
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))?
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))
    };
    let ready_lease = || {
        let state = bridge.state().map_err(|_| unavailable())?;
        if state.ready {
            return state.lease.ok_or_else(unavailable);
        }
        let preparation = bridge
            .prepare_session_if_idle(state)
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if Instant::now() >= deadline {
                return Err(unavailable());
            }
            // 初始化只发生一次；之后仅查询固定目标，不重试创建、不发送正文或回答原生弹窗。
            let state = bridge.state().map_err(|_| unavailable())?;
            if let Some(lease) = preparation
                .lease_from_state(state)
                .map_err(|_| unavailable())?
            {
                return Ok(lease);
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    };
    let mut previous = lookup(observed_session)?;
    let mut lease = None;
    if previous.is_none() {
        let current = ready_lease()?;
        let session_id = Uuid::parse_str(current.session_id()).map_err(|_| unavailable())?;
        if observed_session.is_some_and(|observed| observed != session_id) {
            return Err(unavailable());
        }
        // 无插件通知的重启会话也必须按原生 ID 查旧账，不能借新 instance 重投旧草稿。
        previous = lookup(Some(session_id))?;
        lease = Some(current);
    }
    let replaces = previous
        .as_ref()
        .and_then(|record| match record.delivery.state {
            NativeBridgeDeliveryState::NotSent | NativeBridgeDeliveryState::NativeRejected => {
                Some(record.delivery.message_id)
            }
            NativeBridgeDeliveryState::NativeAcknowledged
                if repeat == Some(record.delivery.message_id) =>
            {
                Some(record.delivery.message_id)
            }
            NativeBridgeDeliveryState::NativeAcknowledged | NativeBridgeDeliveryState::Unknown => {
                None
            }
        });
    if repeat.is_some() && repeat != replaces {
        return Err(unavailable());
    }
    let previous_session = previous.as_ref().map(|record| record.delivery.session_id);
    if replaces.is_some() {
        previous = None;
    }
    let session_id = if let Some(record) = &previous {
        lease = None;
        record.delivery.session_id
    } else {
        if lease.is_none() {
            lease = Some(ready_lease()?);
        }
        let session_id =
            Uuid::parse_str(lease.as_ref().unwrap().session_id()).map_err(|_| unavailable())?;
        if observed_session.is_some_and(|observed| observed != session_id)
            || previous_session.is_some_and(|previous| previous != session_id)
        {
            return Err(unavailable());
        }
        session_id
    };
    Ok(Some(PreparedInput {
        bridge,
        lease,
        previous,
        session_id,
        body,
        replaces,
    }))
}

fn deliver_input(
    prepared: PreparedInput,
    target: InputTarget,
    authorization: SubmissionLease,
    database: SyncSender<ModelEvent>,
) -> Result<DeliveryOutcome, String> {
    let PreparedInput {
        bridge,
        lease,
        previous,
        session_id,
        body,
        replaces,
    } = prepared;
    let persistence_error = || crate::t!("cli-agent-task-save-failed");
    let binding = serde_json::to_value(bridge.binding()).map_err(|_| persistence_error())?;
    let (record, can_write) = match previous {
        Some(record) => (record, false),
        None => {
            let input = NativeBridgeInput {
                binding: binding.clone(),
                session_id,
                working_directory: target.cwd,
                input_revision: Uuid::new_v4(),
                message_id: Uuid::new_v4(),
                body: body.clone(),
                replaces,
            };
            match block_on(ledger::claim_input(&database, input).map_err(|_| persistence_error())?)
                .map_err(|_| persistence_error())?
                .map_err(|_| persistence_error())?
            {
                NativeBridgeClaimOutcome::Claimed(record) => (record, true),
                NativeBridgeClaimOutcome::Existing(record) => (record, false),
            }
        }
    };
    if record.delivery.state == NativeBridgeDeliveryState::NativeAcknowledged {
        return Ok(DeliveryOutcome::AlreadyAcknowledged(
            record.delivery.message_id,
        ));
    }
    let expected =
        NativeBridgeMessage::new(record.delivery.message_id, session_id.to_string(), &body)
            .map_err(|_| persistence_error())?;
    // 数据库提交成功之后才有可能写原生 socket；任何错误都保留领取记录，绝不换 ID 重投。
    let first = if can_write {
        match lease {
            Some(lease) => bridge.submit(lease, expected.message_id, &body, || {
                authorization.claim_write()
            }),
            None => return Ok(DeliveryOutcome::Unconfirmed),
        }
    } else {
        bridge.query(&expected)
    };
    // submit 已返回且授权从未领取，证明首字节尚未写出；下一次用户点击才可新建尝试。
    let rejected_before_admission = matches!(
        &first,
        Ok(NativeBridgeResponse::Rejected { reason }) if matches!(reason.as_str(),
            "stale_lease" | "receipt_capacity" | "session_mismatch"
                | "native_bridge_unavailable" | "journal_unavailable" | "access_blocked"
                | "connection_transition" | "no_active_agent" | "application_owns_input"
                | "session_unbound" | "session_loading" | "session_busy" | "queue_pending"
                | "editor_not_ready" | "receipt_pending"
        )
    );
    // 固定原生版本中这些拒绝都发生于去重检查之后、账本 claim 之前；只接受本次 submit 回包。
    // query 的拒绝以及 message_id_conflict 不具备未准入证明，不能释放旧 Unknown。
    if can_write
        && ((first.is_err() && !authorization.was_write_claimed()) || rejected_before_admission)
    {
        block_on(
            ledger::mark_not_sent(&database, record.delivery.clone())
                .map_err(|_| persistence_error())?,
        )
        .map_err(|_| persistence_error())?
        .map_err(|_| persistence_error())?;
        return Err(crate::t!("cli-agent-grok-owned-input-not-dispatched"));
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut response = first;
    loop {
        match response {
            Ok(NativeBridgeResponse::Receipt(ref receipt))
                if receipt.state == NativeBridgeReceiptState::NativeAcknowledged =>
            {
                let saved = block_on(
                    ledger::acknowledge_input(
                        &database,
                        record.delivery.clone(),
                        binding.clone(),
                        json!({"status": "receipt", "receipt": receipt}),
                    )
                    .map_err(|_| persistence_error())?,
                )
                .map_err(|_| persistence_error())?
                .map_err(|_| persistence_error())?;
                return Ok(if saved.is_some() {
                    DeliveryOutcome::Acknowledged
                } else {
                    DeliveryOutcome::Unconfirmed
                });
            }
            Ok(NativeBridgeResponse::Rejected { .. }) => return Ok(DeliveryOutcome::Unconfirmed),
            Ok(NativeBridgeResponse::Receipt(ref receipt))
                if receipt.state == NativeBridgeReceiptState::NativeRejected =>
            {
                block_on(
                    ledger::acknowledge_input(
                        &database,
                        record.delivery.clone(),
                        binding.clone(),
                        json!({"status": "receipt", "receipt": receipt}),
                    )
                    .map_err(|_| persistence_error())?,
                )
                .map_err(|_| persistence_error())?
                .map_err(|_| persistence_error())?;
                return Err(crate::t!("cli-agent-grok-owned-input-not-dispatched"));
            }
            Ok(NativeBridgeResponse::Receipt(_) | NativeBridgeResponse::Unknown) | Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Ok(DeliveryOutcome::Unconfirmed);
        }
        std::thread::sleep(Duration::from_millis(150));
        response = bridge.query(&expected);
    }
}
