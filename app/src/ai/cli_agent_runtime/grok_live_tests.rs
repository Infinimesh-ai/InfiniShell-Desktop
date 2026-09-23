//! 真实 Grok 验收复用生产进程、监督者、传输与运行命令；最终结果须有完整原生历史证据。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{GrokProtocol, run_process, verified_final_snapshot};
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeController, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, channels, managed_process,
};

const SCOPE: &str = "production_process_transport_with_runtime_commands";
const RECEIPT_SOURCE: &str = "verified_native_final_history";

pub(super) type VerifiedFinalHistories = Arc<Mutex<HashMap<String, NativeFinalHistory>>>;

#[derive(Clone)]
pub(super) struct NativeFinalHistory {
    pub(super) session_id: String,
    pub(super) turn_id: String,
    pub(super) completion_watermark: String,
    pub(super) replay: Value,
}

pub(super) struct NativeResponseReceipt {
    pub(super) final_response: String,
    pub(super) stream_start_ms: Option<i64>,
    pub(super) completion_watermark: String,
    pub(super) full_output: String,
}

/// 只提取已确认终态前的最后响应流；不读取思考正文，不改变产品完整输出。
pub(super) fn final_response_receipt(
    snapshot: &NativeFinalHistory,
    expected_session_id: &str,
    expected_turn_id: &str,
    outcome: &TurnOutcome,
) -> Result<NativeResponseReceipt, String> {
    if snapshot.session_id != expected_session_id || snapshot.turn_id != expected_turn_id {
        return Err("最终回放没有关联到当前会话与回合".into());
    }
    let (full_output, watermark) = verified_final_snapshot(
        &snapshot.replay,
        expected_session_id,
        expected_turn_id,
        outcome,
    )
    .map_err(|_| "原生最终回放没有通过生产终态验证".to_owned())?
    .ok_or("原生最终回放缺少完成水位")?;
    if watermark != snapshot.completion_watermark {
        return Err("最终回放完成水位与生产确认不一致".into());
    }
    let mut final_response = String::new();
    let mut stream_start_ms = None;
    let mut closed_streams = HashSet::new();
    let mut completed = false;
    for record in snapshot.replay["updates"]
        .as_array()
        .expect("生产已验证回放列表")
    {
        let params = &record["params"];
        if params["_meta"]["eventId"] == watermark {
            completed = true;
            continue;
        }
        let update = &params["update"];
        if record["method"] != "session/update"
            || params["_meta"]["promptId"] != expected_turn_id
            || !matches!(
                update["sessionUpdate"].as_str(),
                Some("agent_message_chunk" | "agent_thought_chunk")
            )
        {
            continue;
        }
        if completed {
            return Err("完成水位之后出现同回合响应事件".into());
        }
        let stream = params["_meta"]["streamStartMs"]
            .as_i64()
            .ok_or("原生最终响应缺少流身份")?;
        if stream_start_ms != Some(stream) {
            if closed_streams.contains(&stream) {
                return Err("原生最终响应重新使用已关闭的流身份".into());
            }
            if let Some(previous) = stream_start_ms {
                closed_streams.insert(previous);
            }
            // streamStartMs 是身份，不能以数值大小代替原生事件顺序。
            stream_start_ms = Some(stream);
            final_response.clear();
        }
        if update["sessionUpdate"] == "agent_message_chunk" {
            let text = update["content"]["text"]
                .as_str()
                .filter(|_| update["content"]["type"] == "text")
                .ok_or("原生最终响应包含不支持的正文类型")?;
            final_response.push_str(text);
        }
    }
    if outcome == &TurnOutcome::Completed
        && (stream_start_ms.is_none() || final_response.is_empty())
    {
        return Err("最后原生响应流缺少正文，不能以此前文本替代回执".into());
    }
    Ok(NativeResponseReceipt {
        final_response,
        stream_start_ms,
        completion_watermark: watermark,
        full_output,
    })
}

struct Evidence {
    file: File,
    root: PathBuf,
}

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        let text = serde_json::to_string(&value).map_err(|error| error.to_string())?;
        let root = serde_json::to_string(&self.root).map_err(|error| error.to_string())?;
        let encoded = text.replace(root.trim_matches('"'), "<probe-root>");
        writeln!(self.file, "{encoded}").map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())
    }
}

struct LiveSession {
    generation: Uuid,
    controller: RuntimeController,
    events: mpsc::Receiver<RuntimeEvent>,
    task: Option<JoinHandle<Result<usize, RuntimeError>>>,
    native_id: Option<String>,
    expected_native_id: Option<String>,
    state_dir: PathBuf,
    final_histories: VerifiedFinalHistories,
    test_candidate_1041_p0: bool,
}

#[derive(Clone, Copy)]
enum LiveCapabilityProfile {
    Extended,
    P0,
    CurrentRootCandidate,
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        // 失败也让监督控制连接关闭，由生产清理路径回收进程树。
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl LiveSession {
    fn start(options: SessionOptions, current_root_candidate_for_live: bool) -> Self {
        let mut protocol = GrokProtocol::new(options);
        protocol.current_root_candidate_for_live = current_root_candidate_for_live;
        Self::start_protocol(protocol, false)
    }

    fn start_candidate_1041_p0(options: SessionOptions) -> Result<Self, String> {
        let native = PathBuf::from(
            env::var_os("INFINISHELL_GROK_TEST_CANDIDATE_NATIVE")
                .ok_or("缺少固定的 Grok 1.0.41 原生二进制路径")?,
        );
        let mut protocol = GrokProtocol::new(options);
        protocol.test_only_1041_profile = true;
        protocol.test_only_1041_p0_for_live = true;
        protocol.test_only_1041_native_binary = Some(native);
        Ok(Self::start_protocol(protocol, true))
    }

    fn start_protocol(mut protocol: GrokProtocol, test_candidate_1041_p0: bool) -> Self {
        let generation = protocol.options.generation;
        let state_dir = protocol.options.state_dir.clone();
        let expected_native_id = match &protocol.options.target {
            SessionTarget::New => None,
            SessionTarget::Resume { native_session_id } => Some(native_session_id.clone()),
        };
        let (controller, commands, sender, events) = channels(generation);
        let final_histories = Arc::new(Mutex::new(HashMap::new()));
        protocol.verified_final_histories_for_live = Some(final_histories.clone());
        let task = tokio::spawn(async move {
            let result = run_process(&mut protocol, commands, &sender).await;
            let reason = match &result {
                Ok(()) => "验收连接关闭".to_owned(),
                Err(error) => error.to_string(),
            };
            let _ = sender.try_send(protocol.event(RuntimeEventKind::Disconnected { reason }));
            result.map(|()| protocol.queued_submissions)
        });
        Self {
            generation,
            controller,
            events,
            task: Some(task),
            native_id: None,
            expected_native_id,
            state_dir,
            final_histories,
            test_candidate_1041_p0,
        }
    }

    async fn send(&self, message_id: Uuid, action: RuntimeAction) -> Result<(), String> {
        self.controller
            .send(RuntimeCommand {
                generation: self.generation,
                message_id,
                action,
            })
            .await
            .map_err(|error| error.to_string())
    }

    async fn submit(&self, id: Uuid, prompt: String) -> Result<(), String> {
        self.send(
            id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text(prompt)],
            },
        )
        .await
    }

    async fn next(&mut self) -> Result<RuntimeEvent, String> {
        let event = tokio::time::timeout(Duration::from_secs(90), self.events.recv())
            .await
            .map_err(|_| "等待原生 Grok 事件超时".to_owned())?
            .ok_or_else(|| "Grok 事件流关闭".to_owned())?;
        if event.generation != self.generation {
            return Err("收到过时连接代次".into());
        }
        if let Some(id) = &event.native_session_id {
            Uuid::parse_str(id).map_err(|_| "原生会话 ID 非法".to_owned())?;
            if self
                .native_id
                .as_ref()
                .is_some_and(|previous| previous != id)
                || self
                    .expected_native_id
                    .as_ref()
                    .is_some_and(|expected| expected != id)
            {
                return Err("原生会话身份变化，恢复不能计为通过".into());
            }
            self.native_id = Some(id.clone());
        } else if self.native_id.is_some() {
            return Err("已关联会话的事件丢失原生身份".into());
        }
        Ok(event)
    }

    async fn ready(
        &mut self,
        evidence: &mut Evidence,
        profile: LiveCapabilityProfile,
    ) -> Result<(), String> {
        let event = self.next().await?;
        let RuntimeEventKind::SessionReady {
            verified_cli_version,
            effective_permissions,
            ..
        } = event.kind
        else {
            evidence.record(json!({"event":"initialization_failed","kind":event.kind}))?;
            return Err("Grok 未完成真实初始化".into());
        };
        if self.test_candidate_1041_p0 && verified_cli_version.as_deref() != Some("1.0.41") {
            return Err("Grok 1.0.41 P0 会话版本不匹配".into());
        }
        let (enabled, disabled): (&[&str], &[&str]) = match profile {
            LiveCapabilityProfile::Extended => (
                &[
                    "newSession",
                    "emptyHistoryRecovery",
                    "closeSession",
                    "submit",
                    "queuedSubmit",
                    "approval",
                    "cancel",
                    "resume",
                ],
                &["steer", "localTools", "childTasks"],
            ),
            LiveCapabilityProfile::P0 => (
                &["newSession", "submit", "approval", "cancel", "resume"],
                &[
                    "emptyHistoryRecovery",
                    "closeSession",
                    "queuedSubmit",
                    "steer",
                    "localTools",
                    "childTasks",
                ],
            ),
            LiveCapabilityProfile::CurrentRootCandidate => (
                &[
                    "newSession",
                    "submit",
                    "queuedSubmit",
                    "approval",
                    "cancel",
                    "resume",
                ],
                &[
                    "emptyHistoryRecovery",
                    "closeSession",
                    "steer",
                    "localTools",
                    "childTasks",
                ],
            ),
        };
        for &key in enabled {
            if effective_permissions["verifiedCapabilities"][key] != true {
                return Err("固定版本基础托管能力缺少验证声明".into());
            }
        }
        for &key in disabled {
            if effective_permissions["verifiedCapabilities"][key] != false {
                return Err("真实测试不得打开未验证的 SDK 或同轮追加能力".into());
            }
        }
        let current_model_id = match profile {
            LiveCapabilityProfile::CurrentRootCandidate if !self.test_candidate_1041_p0 => {
                let expected = env::var("INFINISHELL_GROK_LIVE_MODEL")
                    .map_err(|_| "当前版根验收缺少固定模型".to_owned())?;
                let models = &effective_permissions["reportedMetadata"]["models"];
                if models["currentModelId"] != expected
                    || !models["availableModels"].as_array().is_some_and(|models| {
                        models.iter().any(|model| model["modelId"] == expected)
                    })
                {
                    return Err("当前版根验收模型与官方目录不匹配".into());
                }
                Some(expected)
            }
            LiveCapabilityProfile::P0 if self.test_candidate_1041_p0 => {
                let expected = env::var("INFINISHELL_GROK_LIVE_MODEL")
                    .map_err(|_| "1.0.41 P0 验收缺少固定模型".to_owned())?;
                let models = &effective_permissions["reportedMetadata"]["models"];
                if models["currentModelId"] != expected
                    || !models["availableModels"].as_array().is_some_and(|models| {
                        models.iter().any(|model| model["modelId"] == expected)
                    })
                {
                    return Err("1.0.41 P0 模型与官方目录不匹配".into());
                }
                Some(expected)
            }
            LiveCapabilityProfile::Extended
            | LiveCapabilityProfile::P0
            | LiveCapabilityProfile::CurrentRootCandidate => None,
        };
        if self.native_id.is_none() {
            return Err("初始化缺少真实会话 ID".into());
        }
        evidence.record(
            json!({"event":"session_ready", "native_session_id":self.native_id,
            "public_product_gate_open":false,"permission_policy":"Inherit",
            "permission_enforcement_verified":false,"scope":SCOPE,
            "current_model_id":current_model_id}),
        )
    }

    async fn shutdown(&mut self, evidence: &mut Evidence) -> Result<usize, String> {
        self.send(Uuid::new_v4(), RuntimeAction::Shutdown).await?;
        let mut task = self.task.take().ok_or_else(|| "连接已关闭".to_owned())?;
        let result = tokio::time::timeout(Duration::from_secs(30), &mut task).await;
        if result.is_err() {
            task.abort();
            return Err("真实监督进程关闭超时".into());
        }
        let queued = result
            .expect("超时已处理")
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let receipt = managed_process::confirmed_exit(&self.state_dir, self.generation)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "缺少生产监督退出回执".to_owned())?;
        if !receipt.cleanup_confirmed {
            return Err("生产进程树清理未确认".into());
        }
        evidence.record(
            json!({"event":"connection_shutdown", "native_session_id":self.native_id,
            "generation":self.generation,"cleanup_confirmed":true,"containment":receipt.containment,
            "exit_reason":receipt.exit_reason,"exit_code":receipt.exit_code,
            "queued_submissions_observed_inside_adapter":queued}),
        )?;
        Ok(queued)
    }
}

enum TurnMode {
    Plain,
    Approval {
        decision: ApprovalDecision,
        path: PathBuf,
        content: String,
    },
    ReadApproval {
        decision: ApprovalDecision,
        cwd: PathBuf,
        path: PathBuf,
    },
    Queue {
        marker: String,
    },
    Cancel,
}

struct ObservedTurn {
    outcome: TurnOutcome,
    output: String,
    receipt: NativeResponseReceipt,
}

fn exact_write(details: &Value, path: &Path, content: &str) -> bool {
    let call = &details["toolCall"];
    call["kind"] == "edit"
        && call["_meta"]["x.ai/tool"]["name"] == "write"
        && call["rawInput"] == json!({"variant":"Write","file_path":path,"content":content})
}

fn exact_read(details: &Value, cwd: &Path, path: &Path) -> bool {
    let call = &details["toolCall"];
    let raw = &call["rawInput"];
    let target = raw["target_file"]
        .as_str()
        .map(|target| cwd.join(target))
        .and_then(|target| target.canonicalize().ok());
    call["kind"] == "read"
        && call["_meta"]["x.ai/tool"]["version"].as_u64() == Some(1)
        && call["_meta"]["x.ai/tool"]["name"] == "read_file"
        && call["_meta"]["x.ai/tool"]["namespace"] == "grok_build"
        && call["_meta"]["x.ai/tool"]["read_only"] == true
        && raw.as_object().is_some_and(|raw| {
            raw.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "variant" | "target_file" | "offset" | "limit" | "pages" | "format"
                )
            })
        })
        && raw["variant"] == "ReadFile"
        && target.as_deref() == Some(path)
        && !path.is_symlink()
        && ["offset", "limit"].into_iter().all(|key| {
            raw.get(key)
                .is_none_or(|value| value.is_null() || value.as_i64() == Some(1))
        })
        && ["pages", "format"]
            .into_iter()
            .all(|key| raw.get(key).is_none_or(Value::is_null))
}

async fn run_turn(
    session: &mut LiveSession,
    phase: &str,
    prompt: String,
    mode: TurnMode,
    evidence: &mut Evidence,
) -> Result<ObservedTurn, String> {
    let primary = Uuid::new_v4();
    let mut expected = HashSet::from([primary]);
    let mut native_turns = HashMap::new();
    let mut started = HashSet::new();
    let mut finished = HashMap::new();
    let mut controls = HashMap::new();
    let mut dispatched = HashSet::new();
    let mut approvals = HashSet::new();
    let mut resolved = HashSet::new();
    let mut queued_id = None;
    let mut cancel_sent = false;
    let mut output_started = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    evidence.record(json!({"event":"phase_started","phase":phase,"message_id":primary}))?;
    session.submit(primary, prompt).await?;
    loop {
        let event = tokio::time::timeout_at(deadline, session.next())
            .await
            .map_err(|_| "真实回合总时限耗尽".to_owned())??;
        let native_id = event.native_session_id;
        match event.kind {
            RuntimeEventKind::InputJoined { .. } => {
                return Err("Grok 不支持 Claude 合并输入事件".into());
            }
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                let turn_id = turn_id.ok_or_else(|| "原生接收没有回合 ID".to_owned())?;
                if !expected.contains(&message_id)
                    || native_turns.contains_key(&message_id)
                    || native_turns.values().any(|id| id == &turn_id)
                {
                    return Err("原生接收身份重复或不属于待发送输入".into());
                }
                Uuid::parse_str(&turn_id).map_err(|_| "原生回合 ID 非法".to_owned())?;
                native_turns.insert(message_id, turn_id.clone());
                evidence.record(
                    json!({"event":"message_accepted","phase":phase,"message_id":message_id,
                    "turn_id":turn_id,"native_session_id":native_id,"native_receipt":true}),
                )?;
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                if !native_turns.values().any(|id| id == &turn_id)
                    || !started.insert(turn_id.clone())
                {
                    return Err("未知或重复的原生运行事件".into());
                }
                evidence.record(json!({"event":"turn_started","phase":phase,"turn_id":turn_id,"native_session_id":native_id}))?;
            }
            RuntimeEventKind::TextDelta { turn_id, text, .. } => {
                if !started.contains(&turn_id) || finished.contains_key(&turn_id) {
                    return Err("文本没有关联到活跃原生回合".into());
                }
                if native_turns.get(&primary) == Some(&turn_id)
                    && !text.is_empty()
                    && !output_started
                {
                    output_started = true;
                    evidence.record(
                        json!({"event":"real_text_started","phase":phase,"turn_id":turn_id}),
                    )?;
                    match &mode {
                        TurnMode::Queue { marker } => {
                            let id = Uuid::new_v4();
                            expected.insert(id);
                            queued_id = Some(id);
                            session.submit(id, format!("INFINISHELL_GROK_ADAPTER：这是下一轮排队输入。请记住完整标记 {marker}，不使用工具，只回复 {marker}。")).await?;
                            evidence.record(json!({"event":"queued_input_submitted","phase":phase,
                                "message_id":id,"active_turn_id":turn_id,"submitted_after_real_text":true,
                                "same_turn_steering_verified":false}))?;
                        }
                        TurnMode::Cancel => {
                            let id = Uuid::new_v4();
                            controls.insert(id, turn_id.clone());
                            session
                                .send(
                                    id,
                                    RuntimeAction::Interrupt {
                                        turn_id: turn_id.clone(),
                                    },
                                )
                                .await?;
                            cancel_sent = true;
                            evidence.record(json!({"event":"cancel_submitted","phase":phase,
                                "message_id":id,"turn_id":turn_id,"submitted_after_real_text":true}))?;
                        }
                        TurnMode::Plain
                        | TurnMode::Approval { .. }
                        | TurnMode::ReadApproval { .. } => {}
                    }
                }
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                let exact_write_fixture = match &mode {
                    TurnMode::Approval { path, content, .. } => {
                        method == "session/request_permission"
                            && native_turns.get(&primary) == Some(&turn_id)
                            && approvals.is_empty()
                            && exact_write(&details, path, content)
                            && !path.exists()
                    }
                    TurnMode::Plain
                    | TurnMode::ReadApproval { .. }
                    | TurnMode::Queue { .. }
                    | TurnMode::Cancel => false,
                };
                let exact_read_fixture = match &mode {
                    TurnMode::ReadApproval { cwd, path, .. } => {
                        method == "session/request_permission"
                            && native_turns.get(&primary) == Some(&turn_id)
                            && approvals.is_empty()
                            && exact_read(&details, cwd, path)
                    }
                    TurnMode::Plain
                    | TurnMode::Approval { .. }
                    | TurnMode::Queue { .. }
                    | TurnMode::Cancel => false,
                };
                let safe = exact_write_fixture || exact_read_fixture;
                let decision = match &mode {
                    TurnMode::Approval { decision, .. }
                    | TurnMode::ReadApproval { decision, .. }
                        if safe =>
                    {
                        *decision
                    }
                    TurnMode::Plain
                    | TurnMode::Approval { .. }
                    | TurnMode::ReadApproval { .. }
                    | TurnMode::Queue { .. }
                    | TurnMode::Cancel => ApprovalDecision::DenyOnce,
                };
                let id = Uuid::new_v4();
                controls.insert(id, turn_id.clone());
                approvals.insert(approval_id.clone());
                session
                    .send(
                        id,
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision,
                        },
                    )
                    .await?;
                evidence.record(
                    json!({"event":"approval_requested","phase":phase,"approval_id":approval_id,
                    "decision":decision,"exact_write_fixture":exact_write_fixture,
                    "exact_read_fixture":exact_read_fixture}),
                )?;
                if !safe {
                    return Err("超出精确读写验收范围的工具请求已拒绝".into());
                }
            }
            RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id,
            } => {
                if controls.get(&message_id) != turn_id.as_ref() || !dispatched.insert(message_id) {
                    return Err("未知或重复的控制派发；不能充当原生接收".into());
                }
                evidence.record(
                    json!({"event":"command_dispatched","phase":phase,"message_id":message_id,
                    "turn_id":turn_id,"native_receipt":false}),
                )?;
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                let expected_decision = match &mode {
                    TurnMode::Approval { decision, .. }
                    | TurnMode::ReadApproval { decision, .. } => Some(*decision),
                    TurnMode::Plain | TurnMode::Queue { .. } | TurnMode::Cancel => None,
                };
                if !approvals.contains(&approval_id)
                    || !resolved.insert(approval_id.clone())
                    || expected_decision != Some(decision)
                {
                    return Err("审批结束身份或决定不匹配".into());
                }
                evidence.record(
                    json!({"event":"approval_resolved","phase":phase,"approval_id":approval_id,
                    "decision":decision,"native_receipt":false}),
                )?;
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                if !started.contains(&turn_id) || finished.contains_key(&turn_id) {
                    return Err("回合没有关联运行事件或重复结束".into());
                }
                let snapshot = session
                    .final_histories
                    .lock()
                    .map_err(|_| "最终回放锁已损坏")?
                    .remove(&turn_id)
                    .ok_or("回合终态缺少生产验证的最终回放")?;
                let receipt = final_response_receipt(
                    &snapshot,
                    native_id.as_deref().ok_or("终态缺少会话身份")?,
                    &turn_id,
                    &outcome,
                )?;
                if receipt.full_output != output {
                    return Err("产品完整输出与已验证最终回放不一致".into());
                }
                evidence.record(json!({"event":"turn_finished","phase":phase,"turn_id":turn_id,
                    "native_session_id":native_id,"outcome":outcome,"output":output.chars().take(4096).collect::<String>(),
                    "output_truncated":output.chars().count()>4096,"full_output_bytes":output.len(),
                    "full_output_sha256":format!("{:x}",Sha256::digest(output.as_bytes())),
                    "final_response":receipt.final_response,"final_response_bytes":receipt.final_response.len(),
                    "final_response_sha256":format!("{:x}",Sha256::digest(receipt.final_response.as_bytes())),
                    "final_response_stream_start_ms":receipt.stream_start_ms,"receipt_source":RECEIPT_SOURCE,
                    "history_verified":true,"history_native_session_id":snapshot.session_id,
                    "history_native_turn_id":snapshot.turn_id,"history_completion_watermark":receipt.completion_watermark,
                    "product_full_output_preserved":true}))?;
                finished.insert(
                    turn_id,
                    ObservedTurn {
                        outcome,
                        output,
                        receipt,
                    },
                );
            }
            RuntimeEventKind::Progress { .. } => {}
            RuntimeEventKind::RequestFailed { message, .. } => {
                return Err(format!("Grok 请求失败：{message}"));
            }
            RuntimeEventKind::Disconnected { reason } => {
                return Err(format!("Grok 连接断开：{reason}"));
            }
            RuntimeEventKind::ApprovalCancelled { .. } => return Err("审批未决时被关闭".into()),
            RuntimeEventKind::SessionReady { .. } => return Err("运行中意外重建会话".into()),
            RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::LocalToolCancelled { .. } => {
                return Err("未授权客户端工具回调".into());
            }
        }
        if native_turns.len() == expected.len()
            && finished.len() == expected.len()
            && dispatched.len() == controls.len()
            && approvals == resolved
        {
            if matches!(
                mode,
                TurnMode::Approval { .. } | TurnMode::ReadApproval { .. }
            ) && approvals.len() != 1
            {
                return Err("没有观察到恰好一次原生审批".into());
            }
            if let TurnMode::Queue { marker } = &mode {
                let queued = queued_id
                    .and_then(|id| native_turns.get(&id))
                    .and_then(|id| finished.get(id))
                    .ok_or_else(|| "追加输入缺少独立原生确认及结果".to_owned())?;
                completed(queued, marker)?;
                evidence.record(
                    json!({"event":"queued_input_result_verified","marker":marker,
                    "native_acknowledgement_verified":true,"same_turn_steering_supported":false}),
                )?;
            }
            if matches!(mode, TurnMode::Cancel) && !cancel_sent {
                return Err("尚未真实发送取消".into());
            }
            return native_turns
                .get(&primary)
                .and_then(|id| finished.remove(id))
                .ok_or_else(|| "主回合结果缺失".to_owned());
        }
    }
}

fn completed(turn: &ObservedTurn, expected: &str) -> Result<(), String> {
    if turn.outcome != TurnOutcome::Completed
        || turn.receipt.final_response.trim() != expected
        || turn.receipt.full_output != turn.output
    {
        return Err("真实结果与固定回执不匹配".into());
    }
    Ok(())
}

async fn exercise(
    root: &Path,
    evidence: &mut Evidence,
    current_root_candidate: bool,
) -> Result<(), String> {
    let official = match env::var("INFINISHELL_GROK_LIVE_AUTH_MODE") {
        Ok(mode) if mode == "official-cached-token" => true,
        Ok(mode) => return Err(format!("不支持的验收认证模式：{mode}")),
        Err(env::VarError::NotPresent) => false,
        Err(env::VarError::NotUnicode(_)) => return Err("验收认证模式不是有效文本".into()),
    };
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let state_dir = env::var_os("INFINISHELL_GROK_LIVE_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("state"));
    let mut options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE")
                .ok_or_else(|| "缺少固定 Grok 沙箱入口".to_owned())?,
        ),
        cwd: cwd.clone(),
        state_dir,
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let mut session = LiveSession::start(options.clone(), current_root_candidate);
    session
        .ready(
            evidence,
            if current_root_candidate {
                LiveCapabilityProfile::CurrentRootCandidate
            } else {
                LiveCapabilityProfile::Extended
            },
        )
        .await?;
    let native_id = session
        .native_id
        .clone()
        .ok_or_else(|| "没有真实会话 ID".to_owned())?;
    completed(
        &run_turn(
            &mut session,
            "first_turn",
            "INFINISHELL_GROK_ADAPTER 第一轮：你好。\n不调用工具，只回复 PARITY_ONE。".into(),
            TurnMode::Plain,
            evidence,
        )
        .await?,
        "PARITY_ONE",
    )?;
    completed(
        &run_turn(
            &mut session,
            "second_turn",
            "INFINISHELL_GROK_ADAPTER 第二轮：不调用工具，只回复 PARITY_TWO。".into(),
            TurnMode::Plain,
            evidence,
        )
        .await?,
        "PARITY_TWO",
    )?;
    for (label, decision) in [
        ("allow", ApprovalDecision::AllowOnce),
        ("deny", ApprovalDecision::DenyOnce),
    ] {
        let path = cwd.join(format!("approval-{label}.txt"));
        let content = "PARITY_APPROVAL".to_owned();
        if path.exists() || path.is_symlink() {
            return Err("审批路径不为空或为符号链接".into());
        }
        let prompt = format!(
            "INFINISHELL_GROK_ADAPTER: Use only the write tool once to create {} with exactly PARITY_APPROVAL (no newline). Request approval. Do not inspect files, run commands, use another tool, or retry if denied. After successful write reply exactly APPROVED.",
            json!(path)
        );
        let observed = run_turn(
            &mut session,
            &format!("approval_{label}"),
            prompt,
            TurnMode::Approval {
                decision,
                path: path.clone(),
                content: content.clone(),
            },
            evidence,
        )
        .await?;
        match decision {
            ApprovalDecision::AllowOnce => {
                completed(&observed, "APPROVED")?;
                if fs::read_to_string(&path).map_err(|error| error.to_string())? != content {
                    return Err("允许后文件内容错误".into());
                }
            }
            ApprovalDecision::DenyOnce => {
                if observed.outcome != TurnOutcome::Cancelled || path.exists() {
                    return Err("拒绝后未取消或仍产生目标文件".into());
                }
            }
        }
        evidence.record(json!({"event":"file_effect_verified","phase":format!("approval_{label}"),"allowed":decision==ApprovalDecision::AllowOnce}))?;
    }
    let marker = format!("QUEUED_APPLIED_{}", Uuid::new_v4().simple());
    let queued = run_turn(&mut session,"queued_input","INFINISHELL_GROK_ADAPTER：不要调用工具，逐个列出从1到300的整数，最后输出 QUEUE_PARENT_DONE。".into(), TurnMode::Queue { marker: marker.clone() },evidence).await?;
    if queued.outcome != TurnOutcome::Completed {
        return Err("排队时主回合未完成".into());
    }
    let cancelled = run_turn(
        &mut session,
        "cancel",
        "INFINISHELL_GROK_ADAPTER：不要调用工具，先输出 READY 然后逐个列出从1到100000的整数。"
            .into(),
        TurnMode::Cancel,
        evidence,
    )
    .await?;
    if cancelled.outcome != TurnOutcome::Cancelled {
        return Err("取消没有原生终态".into());
    }
    if session.shutdown(evidence).await? != 1 {
        return Err("适配器内部没有恰好一次运行中排队，不能冒称通过".into());
    }
    options.generation = Uuid::new_v4();
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    let mut resumed = LiveSession::start(options, current_root_candidate);
    resumed
        .ready(
            evidence,
            if current_root_candidate {
                LiveCapabilityProfile::CurrentRootCandidate
            } else {
                LiveCapabilityProfile::Extended
            },
        )
        .await?;
    let recovered = run_turn(&mut resumed,"resume_result","INFINISHELL_GROK_ADAPTER：不要调用工具，只回复上一会话运行中追加指令要求记住的完整 QUEUED_APPLIED 标记。".into(),TurnMode::Plain,evidence).await?;
    completed(&recovered, &marker)?;
    if resumed.native_id.as_ref() != Some(&native_id) {
        return Err("恢复没有关联原会话".into());
    }
    if resumed.shutdown(evidence).await? != 0 {
        return Err("恢复阶段出现额外排队".into());
    }
    evidence.record(json!({"event":"acceptance_passed","scope":SCOPE,"native_session_id":native_id,
        "queued_input_verified":true,"same_turn_steering_supported":false,"public_product_gate_open":false,
        "app_restart_and_ui_verified":false,"parent_permission_ceiling_verified":false,
        "official_grok_model_tested":official,
        "model_path":if official { "Grok Build + 官方缓存登录" } else { "Grok Build + 自定义 Claude 后端" }}))
}

fn start_p0_session(
    options: SessionOptions,
    test_candidate_1041: bool,
) -> Result<LiveSession, String> {
    if test_candidate_1041 {
        LiveSession::start_candidate_1041_p0(options)
    } else {
        Ok(LiveSession::start(options, false))
    }
}

async fn exercise_p0(
    root: &Path,
    evidence: &mut Evidence,
    test_candidate_1041: bool,
) -> Result<(), String> {
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let marker = format!("{:x}", Sha256::digest(Uuid::new_v4().as_bytes()));
    let allow_path = cwd.join("allow.txt");
    let deny_path = cwd.join("deny.txt");
    for path in [&allow_path, &deny_path] {
        if path.exists() || path.is_symlink() {
            return Err("P0 读审批夹具路径不为空或为符号链接".into());
        }
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        writeln!(file, "{marker}").map_err(|error| error.to_string())?;
    }
    let mut options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE")
                .ok_or_else(|| "缺少固定 Grok 沙箱入口".to_owned())?,
        ),
        cwd: cwd.clone(),
        state_dir: root.join("state"),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    if test_candidate_1041 {
        let mut handshake = start_p0_session(options.clone(), true)?;
        handshake.ready(evidence, LiveCapabilityProfile::P0).await?;
        let handshake_id = handshake
            .native_id
            .clone()
            .ok_or("零输入握手缺少会话身份")?;
        if handshake.shutdown(evidence).await? != 0 {
            return Err("零输入握手出现未验收的排队输入".into());
        }
        evidence.record(json!({"event":"candidate_zero_input_handshake_verified",
            "native_session_id":handshake_id,"model_inputs_sent":0,
            "verified_cli_version":"1.0.41","cleanup_confirmed":true}))?;
        options.generation = Uuid::new_v4();
    }
    let mut session = start_p0_session(options.clone(), test_candidate_1041)?;
    session.ready(evidence, LiveCapabilityProfile::P0).await?;
    let native_id = session
        .native_id
        .clone()
        .ok_or_else(|| "没有真实会话 ID".to_owned())?;
    completed(
        &run_turn(
            &mut session,
            "p0_read_allow",
            "只用 read_file 读取 allow.txt 一次，参数只含 target_file=allow.txt。不要使用其他工具；成功后只输出文件唯一一行的原文，不解释。".into(),
            TurnMode::ReadApproval {
                decision: ApprovalDecision::AllowOnce,
                cwd: cwd.clone(),
                path: allow_path.canonicalize().map_err(|error| error.to_string())?,
            },
            evidence,
        )
        .await?,
        &marker,
    )?;
    let denied = run_turn(
        &mut session,
        "p0_read_deny",
        "只用 read_file 读取 deny.txt 一次，参数只含 target_file=deny.txt。等待审批；拒绝后不要重试或使用其他工具。".into(),
        TurnMode::ReadApproval {
            decision: ApprovalDecision::DenyOnce,
            cwd: cwd.clone(),
            path: deny_path.canonicalize().map_err(|error| error.to_string())?,
        },
        evidence,
    )
    .await?;
    if denied.outcome != TurnOutcome::Cancelled {
        return Err("P0 读拒绝没有原生取消终态".into());
    }
    let cancelled = run_turn(
        &mut session,
        "p0_cancel",
        "不要调用工具，先输出 READY，然后逐个列出从 1 到 100000 的整数。".into(),
        TurnMode::Cancel,
        evidence,
    )
    .await?;
    if cancelled.outcome != TurnOutcome::Cancelled {
        return Err("P0 取消没有原生终态".into());
    }
    if session.shutdown(evidence).await? != 0 {
        return Err("P0 新会话不得出现未验收的排队输入".into());
    }

    options.generation = Uuid::new_v4();
    options.target = SessionTarget::Resume {
        native_session_id: native_id.clone(),
    };
    let mut resumed = start_p0_session(options, test_candidate_1041)?;
    resumed.ready(evidence, LiveCapabilityProfile::P0).await?;
    completed(
        &run_turn(
            &mut resumed,
            "p0_resume",
            "不要调用工具，只回复上一会话第一轮从 allow.txt 读取到的完整 64 位十六进制文本。"
                .into(),
            TurnMode::Plain,
            evidence,
        )
        .await?,
        &marker,
    )?;
    if resumed.native_id.as_ref() != Some(&native_id) {
        return Err("P0 恢复没有关联原会话".into());
    }
    if resumed.shutdown(evidence).await? != 0 {
        return Err("P0 恢复阶段不得出现未验收的排队输入".into());
    }
    evidence.record(json!({"event":"acceptance_passed","scope":SCOPE,
        "native_session_id":native_id,
        "verified_scope":if test_candidate_1041 { "p0-test-only-1.0.41" } else { "p0" },
        "test_only_candidate_1041":test_candidate_1041,
        "zero_input_handshake_verified":test_candidate_1041,
        "public_product_gate_open":!test_candidate_1041,
        "full_cli_parity_acceptance_passed":false,"queued_input_verified":false,
        "same_turn_steering_supported":false,"read_approval_verified":true,
        "write_approval_verified":false,"close_session_verified":false,
        "app_restart_and_ui_verified":false,"parent_permission_ceiling_verified":false,
        "official_grok_model_tested":true,"model_path":"Grok Build + 官方缓存登录"}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由 Grok 隔离运行器显式启动；会消耗用户授权模型额度，产品门禁保持关闭"]
async fn real_grok_managed_lifecycle() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .expect("私有验收目录存在");
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").expect("缺少私有 Grok HOME"))
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert!(env::var_os("XAI_API_KEY").is_none());
    assert!(env::var_os("ANTHROPIC_API_KEY").is_none());
    assert!(env::var_os("ANTHROPIC_AUTH_TOKEN").is_none());
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").expect("缺少证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence.record(json!({"event":"acceptance_started","scope":SCOPE,"credential_files_read_by_probe":false,
        "production_run_process":true,"production_run_transport":true,
        "production_runtime_commands":true,"test_only_internal_command_switch":false,
        "public_product_gate_open":false,"same_turn_steering_supported":false})).unwrap();
    let result = exercise(&root, &mut evidence, false).await;
    if let Err(error) = &result {
        evidence
            .record(json!({"event":"acceptance_failed","reason":error}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "真实 Grok Rust 适配器验收未通过；请检查脱敏证据"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由 Grok 1.0.40 官方隔离运行器显式启动；会消耗用户授权模型额度"]
async fn real_grok_current_root_lifecycle() {
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_PROFILE").as_deref(),
        Ok("root-1.0.40")
    );
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .expect("私有验收目录存在");
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").expect("缺少私有 Grok HOME"))
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert!(env::var_os("XAI_API_KEY").is_none());
    assert!(env::var_os("ANTHROPIC_API_KEY").is_none());
    assert!(env::var_os("ANTHROPIC_AUTH_TOKEN").is_none());
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").expect("缺少证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence
        .record(json!({"event":"acceptance_started","scope":SCOPE,
            "verified_scope":"current_root_candidate","credential_files_read_by_probe":false,
            "production_run_process":true,"production_run_transport":true,
            "production_runtime_commands":true,"test_only_current_candidate_gate":true,
            "public_product_gate_open":false,"same_turn_steering_supported":false,
            "skills_verified":false,"local_tools_verified":false,"child_tasks_verified":false}))
        .unwrap();
    let result = exercise(&root, &mut evidence, true).await;
    if let Err(error) = &result {
        evidence
            .record(json!({"event":"acceptance_failed","reason":error}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "真实 Grok 1.0.40 根生命周期验收未通过；请检查脱敏证据"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由 Grok 1.0.34 官方隔离运行器显式启动；会消耗用户授权模型额度"]
async fn real_grok_p0_lifecycle() {
    run_p0_lifecycle(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由 Grok 1.0.41 官方隔离运行器显式启动；最多四次模型输入"]
async fn real_grok_candidate_1041_p0_lifecycle() {
    run_p0_lifecycle(true).await;
}

async fn run_p0_lifecycle(test_candidate_1041: bool) {
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_PROFILE").as_deref(),
        Ok(if test_candidate_1041 {
            "p0-test-only-1.0.41"
        } else {
            "p0-1.0.34"
        })
    );
    if test_candidate_1041 {
        assert_eq!(
            env::var("INFINISHELL_GROK_TEST_CANDIDATE_1041")
                .ok()
                .as_deref(),
            Some("1")
        );
    }
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .expect("私有验收目录存在");
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").expect("缺少私有 Grok HOME"))
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert!(env::var_os("XAI_API_KEY").is_none());
    assert!(env::var_os("ANTHROPIC_API_KEY").is_none());
    assert!(env::var_os("ANTHROPIC_AUTH_TOKEN").is_none());
    let mut evidence = Evidence {
        file: File::create(PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").expect("缺少证据路径"),
        ))
        .unwrap(),
        root: root.clone(),
    };
    evidence
        .record(json!({"event":"acceptance_started","scope":SCOPE,
            "verified_scope":if test_candidate_1041 { "p0-test-only-1.0.41" } else { "p0" },
            "test_only_candidate_1041":test_candidate_1041,
            "max_native_inputs":4,"credential_files_read_by_probe":false,
            "production_run_process":true,"production_run_transport":true,
            "production_runtime_commands":true,"test_only_internal_command_switch":false,
            "public_product_gate_open":!test_candidate_1041,"full_cli_parity_acceptance_passed":false,
            "same_turn_steering_supported":false}))
        .unwrap();
    let result = exercise_p0(&root, &mut evidence, test_candidate_1041).await;
    if let Err(error) = &result {
        evidence
            .record(json!({"event":"acceptance_failed","reason":error}))
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "真实 Grok P0 适配器验收未通过；请检查脱敏证据"
    );
}

#[test]
fn live_approval_guard_accepts_only_the_exact_native_write() {
    let path = env::temp_dir().join("批准中文.txt");
    let mut details = json!({"toolCall":{"kind":"edit","_meta":{"x.ai/tool":{"name":"write"}},"rawInput":{"variant":"Write","file_path":path,"content":"PARITY_APPROVAL"}}});
    assert!(exact_write(&details, &path, "PARITY_APPROVAL"));
    details["toolCall"]["rawInput"]["content"] = json!("PARITY_APPROVAL\n");
    assert!(!exact_write(&details, &path, "PARITY_APPROVAL"));
    details["toolCall"]["rawInput"]["content"] = json!("PARITY_APPROVAL");
    details["toolCall"]["rawInput"]["extra"] = json!(true);
    assert!(!exact_write(&details, &path, "PARITY_APPROVAL"));
}

#[test]
fn live_p0_approval_guard_accepts_only_the_exact_native_read() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("批准读取.txt");
    fs::write(&path, "fixture\n").unwrap();
    let path = path.canonicalize().unwrap();
    let mut details = json!({"toolCall":{"kind":"read","_meta":{"x.ai/tool":{
        "version":1,"name":"read_file","namespace":"grok_build","read_only":true}},
        "rawInput":{"variant":"ReadFile","target_file":"批准读取.txt"}}});
    assert!(exact_read(&details, directory.path(), &path));
    details["toolCall"]["rawInput"]["offset"] = json!(2);
    assert!(!exact_read(&details, directory.path(), &path));
    details["toolCall"]["rawInput"]["offset"] = json!(1);
    details["toolCall"]["_meta"]["x.ai/tool"]["read_only"] = json!(false);
    assert!(!exact_read(&details, directory.path(), &path));
    details["toolCall"]["_meta"]["x.ai/tool"]["read_only"] = json!(true);
    details["toolCall"]["rawInput"]["content"] = json!("unverified");
    assert!(!exact_read(&details, directory.path(), &path));
}

fn response_record(
    session: &str,
    turn: &str,
    sequence: usize,
    stream: i64,
    kind: &str,
    text: &str,
) -> Value {
    json!({"method":"session/update","params":{"sessionId":session,
        "_meta":{"eventId":format!("{session}-{sequence}"),"promptId":turn,"streamStartMs":stream},
        "update":{"sessionUpdate":kind,"content":{"type":"text","text":text}}}})
}

fn response_receipt_fixture(prelude_stream: i64, final_stream: i64) -> NativeFinalHistory {
    let session = Uuid::new_v4().to_string();
    let turn = Uuid::new_v4().to_string();
    let watermark = format!("{session}-5");
    let updates = vec![
        response_record(
            &session,
            &turn,
            1,
            prelude_stream,
            "agent_message_chunk",
            "请批准这次写入。\n",
        ),
        json!({"method":"session/update","params":{"sessionId":session,"_meta":{"eventId":format!("{session}-2"),"promptId":turn},
            "update":{"sessionUpdate":"tool_call","toolCallId":"fixture-write"}}}),
        response_record(
            &session,
            &turn,
            3,
            final_stream,
            "agent_thought_chunk",
            "合成思考哨兵，不属于正文",
        ),
        response_record(
            &session,
            &turn,
            4,
            final_stream,
            "agent_message_chunk",
            "APPROVED",
        ),
        json!({"method":"_x.ai/session/update","params":{"sessionId":session,"_meta":{"eventId":watermark},
            "update":{"sessionUpdate":"turn_completed","prompt_id":turn,"stop_reason":"end_turn"}}}),
    ];
    NativeFinalHistory {
        session_id: session,
        turn_id: turn,
        completion_watermark: watermark.clone(),
        replay: json!({"updates":updates,"totalCount":5,"hasMore":false,"lastEventId":watermark}),
    }
}

#[test]
fn final_receipt_uses_native_response_order_and_preserves_product_prelude() {
    let snapshot = response_receipt_fixture(9000, 1000);
    let receipt = final_response_receipt(
        &snapshot,
        &snapshot.session_id,
        &snapshot.turn_id,
        &TurnOutcome::Completed,
    )
    .unwrap();
    assert_eq!(receipt.final_response, "APPROVED");
    assert_eq!(receipt.stream_start_ms, Some(1000));
    assert_eq!(receipt.full_output, "请批准这次写入。\nAPPROVED");
    assert_eq!(receipt.completion_watermark, snapshot.completion_watermark);
    assert!(!receipt.final_response.contains("合成思考哨兵"));
    assert!(
        completed(
            &ObservedTurn {
                outcome: TurnOutcome::Completed,
                output: receipt.full_output.clone(),
                receipt
            },
            "APPROVED"
        )
        .is_ok()
    );
}

#[test]
fn same_response_prelude_cannot_pass_with_an_approved_suffix() {
    let snapshot = response_receipt_fixture(1000, 1000);
    let receipt = final_response_receipt(
        &snapshot,
        &snapshot.session_id,
        &snapshot.turn_id,
        &TurnOutcome::Completed,
    )
    .unwrap();
    assert_eq!(receipt.final_response, "请批准这次写入。\nAPPROVED");
    assert!(
        completed(
            &ObservedTurn {
                outcome: TurnOutcome::Completed,
                output: receipt.full_output.clone(),
                receipt
            },
            "APPROVED"
        )
        .is_err()
    );
}

#[test]
fn receipt_rejects_wrong_session_turn_and_completion_watermark() {
    let mut snapshot = response_receipt_fixture(1000, 2000);
    assert!(
        final_response_receipt(
            &snapshot,
            "old-session",
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            "old-turn",
            &TurnOutcome::Completed
        )
        .is_err()
    );
    snapshot.completion_watermark = format!("{}-4", snapshot.session_id);
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
}

#[test]
fn receipt_never_guesses_a_missing_stream_or_reuses_a_closed_stream() {
    let mut snapshot = response_receipt_fixture(1000, 2000);
    snapshot.replay["updates"][3]["params"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("streamStartMs");
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
    let mut snapshot = response_receipt_fixture(1000, 2000);
    snapshot.replay["updates"][3]["params"]["_meta"]["streamStartMs"] = json!(1000);
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
}

#[test]
fn thought_only_last_response_cannot_reuse_earlier_approved_text() {
    let mut snapshot = response_receipt_fixture(1000, 2000);
    snapshot.replay["updates"][0]["params"]["update"]["content"]["text"] = json!("APPROVED");
    snapshot.replay["updates"][3]["params"]["update"]["sessionUpdate"] =
        json!("agent_thought_chunk");
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
}

#[test]
fn other_turns_and_post_completion_thoughts_cannot_supply_the_receipt() {
    let mut snapshot = response_receipt_fixture(1000, 2000);
    snapshot.replay["updates"][3]["params"]["_meta"]["promptId"] =
        json!(Uuid::new_v4().to_string());
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
    let mut snapshot = response_receipt_fixture(1000, 2000);
    let record = response_record(
        &snapshot.session_id,
        &snapshot.turn_id,
        6,
        3000,
        "agent_thought_chunk",
        "合成旧回调哨兵",
    );
    snapshot.replay["updates"]
        .as_array_mut()
        .unwrap()
        .push(record);
    snapshot.replay["totalCount"] = json!(6);
    snapshot.replay["lastEventId"] = json!(format!("{}-6", snapshot.session_id));
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
}

#[test]
fn unverified_or_conflicting_history_cannot_supply_a_final_receipt() {
    let mut snapshot = response_receipt_fixture(1000, 2000);
    snapshot.replay["hasMore"] = json!(true);
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
    let mut snapshot = response_receipt_fixture(1000, 2000);
    snapshot.replay["updates"][4]["params"]["update"]["stop_reason"] = json!("cancelled");
    assert!(
        final_response_receipt(
            &snapshot,
            &snapshot.session_id,
            &snapshot.turn_id,
            &TurnOutcome::Completed
        )
        .is_err()
    );
}
