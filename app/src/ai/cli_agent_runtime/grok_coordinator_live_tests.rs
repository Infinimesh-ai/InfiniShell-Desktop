//! 官方 Grok 根任务的真实协调器与 SQLite 验收；App::test 不代表 GUI。

use std::env;
use std::fs::{self, File};
use std::io::Write as _;

use serde_json::Value;
use sha2::{Digest, Sha256};
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};

use super::super::runtime_host::confirmed_exit;
use super::super::{ApprovalDecision, current_state_dir};
use super::*;
use crate::persistence::local_cli_tasks::load_task_messages;

const SCOPE: &str = "real_grok_root_production_coordinator";
const MAX_NATIVE_INPUTS: usize = 8;
const CONTENT: &str = "GROK_COORDINATOR_APPROVAL";

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

struct Evidence {
    file: File,
}

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        let value = serde_json::to_string(&value).map_err(|_| "安全证据编码失败")?;
        writeln!(self.file, "{value}").map_err(|_| "安全证据写入失败")?;
        self.file.flush().map_err(|_| "安全证据刷新失败".into())
    }
}

fn input_link_projection(value: &Value) -> Result<Value, String> {
    let object = value.as_object().ok_or("输入关联不是对象")?;
    let keys = [
        "message_id",
        "submission_generation",
        "runtime_generation",
        "native_turn_id",
    ];
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err("输入关联字段不符合固定契约".into());
    }
    for key in ["message_id", "runtime_generation", "native_turn_id"] {
        if key == "native_turn_id" && value[key].is_null() {
            continue;
        }
        let text = value[key].as_str().ok_or("输入关联身份类型无效")?;
        let id = Uuid::parse_str(text).map_err(|_| "输入关联身份无效")?;
        if id.to_string() != text {
            return Err("输入关联身份格式无效".into());
        }
    }
    if !value["submission_generation"]
        .as_i64()
        .is_some_and(|generation| generation > 0)
    {
        return Err("输入关联提交代无效".into());
    }
    Ok(json!({"message_id":value["message_id"],
        "submission_generation":value["submission_generation"],
        "runtime_generation":value["runtime_generation"],
        "native_turn_id":value["native_turn_id"]}))
}

fn pending_inputs_projection(value: &Value) -> Result<Value, String> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let entries = value.as_array().ok_or("待发输入不是数组")?;
    if entries.len() > MAX_NATIVE_INPUTS {
        return Err("待发输入超过夹具固定预算".into());
    }
    entries
        .iter()
        .map(input_link_projection)
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn task_projection(task: &LocalCliTask) -> Result<Value, String> {
    let config: Value = serde_json::from_str(&task.config_json).map_err(|_| "任务配置解码失败")?;
    if task.harness != "grok"
        || task.parent_task_id.is_some()
        || task.parent_generation.is_some()
        || config["permission_policy"] != "Inherit"
        || !config["permission_ceiling"].is_null()
        || !config["claude_profile"].is_null()
        || !config["local_tools"].is_null()
        || !config["model"].is_null()
        || config["selected_skills"] != json!([])
    {
        return Err("根任务权限、工具或父子边界变化".into());
    }
    let proof: Value = task
        .terminal_evidence
        .as_ref()
        .map(|text| serde_json::from_str(text).map_err(|_| "终态证据解码失败"))
        .transpose()?
        .unwrap_or(Value::Null);
    let current_input = if config["grok_current_input"].is_null() {
        Value::Null
    } else {
        input_link_projection(&config["grok_current_input"])?
    };
    let pending_inputs = pending_inputs_projection(&config["grok_pending_inputs"])?;
    let observed = &proof["event"]["TurnFinished"]["outcome"];
    let terminal_outcome = if observed == "Completed" {
        Some("Completed")
    } else if observed == "Cancelled" {
        Some("Cancelled")
    } else if observed == "Failed"
        || observed
            .as_object()
            .is_some_and(|object| object.len() == 1 && object.contains_key("Failed"))
    {
        Some("Failed")
    } else {
        None
    };
    Ok(json!({"task_id":task.task_id,"harness":task.harness,
        "parent_task_id":task.parent_task_id,"parent_generation":task.parent_generation,
        "generation":task.generation,"revision":task.revision,"state":task.state,
        "native_session_id":task.native_session_id,
        "result_bytes":task.result.as_ref().map(|text|text.len()),
        "result_sha256":task.result.as_ref().map(|text|sha(text)),
        "terminal_evidence_present":task.terminal_evidence.is_some(),
        "terminal_native_session_id":proof["native_session_id"],
        "terminal_turn_id":proof["event"]["TurnFinished"]["turn_id"],
        "terminal_outcome":terminal_outcome,
        "runtime_generation":config["runtime_generation"],
        "permission_policy":config["permission_policy"],
        "permission_ceiling":null,"claude_profile":null,"local_tools":null,
        "model":null,"selected_skills":[],
        "grok_pending_inputs":pending_inputs,
        "grok_current_input":current_input}))
}

fn message_projection(message: &LocalCliMessage) -> Value {
    json!({"message_id":message.message_id,"sender_task_id":message.sender_task_id,
        "recipient_task_id":message.recipient_task_id,
        "sender_generation":message.sender_generation,
        "recipient_generation":message.recipient_generation,"subject":message.subject,
        "state":message.state,"receipt_kind":message.receipt_kind,
        "body_bytes":message.body.len(),"body_sha256":sha(&message.body)})
}

type Wake = Option<(LocalCliTask, RuntimeEvent)>;

struct Input {
    id: Uuid,
    phase: &'static str,
    submission_generation: i64,
    expected: Option<String>,
    outcome: TurnOutcome,
}

struct Probe {
    coordinator: ModelHandle<LocalCLITaskCoordinator>,
    sender: SyncSender<ModelEvent>,
    receiver: mpsc::UnboundedReceiver<Wake>,
    evidence: Evidence,
    task_id: String,
    options: SessionOptions,
    cleaned: HashSet<Uuid>,
    inputs: Vec<Input>,
    accepted: HashMap<Uuid, String>,
    started: HashSet<String>,
    finished: HashSet<String>,
    text: HashSet<String>,
    native_id: Option<String>,
    approvals: HashSet<String>,
    root: PathBuf,
}

impl Probe {
    fn snapshot(&self, app: &App) -> Result<ManagedTaskSnapshot, String> {
        self.coordinator
            .read(app, |model, _| model.snapshot(&self.task_id).cloned())
            .ok_or_else(|| "根任务快照缺失".into())
    }

    async fn saved(&self) -> Result<LocalCliTask, String> {
        load_tasks(&self.sender, false)?
            .await
            .map_err(|_| "任务读取确认已关闭")??
            .into_iter()
            .find(|task| task.task_id == self.task_id)
            .ok_or_else(|| "真实 SQLite 根任务丢失".into())
    }

    async fn sqlite_snapshot(&mut self, label: &str) -> Result<Value, String> {
        let task = self.saved().await?;
        let history = load_task_generations(&self.sender, self.task_id.clone())?
            .await
            .map_err(|_| "历史读取确认已关闭")??;
        let messages = load_task_messages(&self.sender, self.task_id.clone())?
            .await
            .map_err(|_| "消息读取确认已关闭")??;
        let value = json!({"event":"sqlite_snapshot","label":label,
            "task":task_projection(&task)?,
            "history":history.iter().map(task_projection).collect::<Result<Vec<_>,_>>()?,
            "messages":messages.iter().map(message_projection).collect::<Vec<_>>()});
        self.evidence.record(value.clone())?;
        Ok(value)
    }

    async fn request(
        &self,
        app: &mut App,
        message: Uuid,
        action: RuntimeAction,
    ) -> Result<(), String> {
        let generation = self.snapshot(app)?.task.generation;
        self.coordinator
            .update(app, |model, _| {
                model.request_for_generation(&self.task_id, generation, message, action)
            })?
            .with_timeout(Duration::from_secs(15))
            .await
            .map_err(|_| "协调器提交超时")?
            .map_err(|_| "协调器提交确认已关闭")?
    }

    fn record_input(
        &mut self,
        action: &RuntimeAction,
        phase: &'static str,
        generation: i64,
        expected: Option<String>,
        outcome: TurnOutcome,
        active_turn: Option<String>,
    ) -> Result<Uuid, String> {
        if self.inputs.len() >= MAX_NATIVE_INPUTS {
            return Err("原生输入超过固定八条预算".into());
        }
        let id = Uuid::new_v4();
        let body = serde_json::to_string(action).map_err(|_| "提交消息编码失败")?;
        self.evidence
            .record(json!({"event":"input_submitted","phase":phase,
            "message_id":id,"submission_generation":generation,
            "runtime_generation":self.options.generation,
            "submitted_while_running":active_turn.is_some(),"active_turn_id":active_turn,
            "expected_marker":expected,"expected_outcome":outcome,
            "body_bytes":body.len(),"body_sha256":sha(&body),
            "submitted_input_count":self.inputs.len()+1}))?;
        self.inputs.push(Input {
            id,
            phase,
            submission_generation: generation,
            expected,
            outcome,
        });
        Ok(id)
    }

    async fn submit(
        &mut self,
        app: &mut App,
        phase: &'static str,
        prompt: String,
        expected: Option<String>,
        outcome: TurnOutcome,
    ) -> Result<Uuid, String> {
        let before = self.snapshot(app)?;
        let generation = if before.task.state.is_terminal() {
            before.task.generation + 1
        } else {
            before.task.generation
        };
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text(prompt)],
        };
        let id = self.record_input(
            &action,
            phase,
            generation,
            expected,
            outcome,
            before.active_turn_id,
        )?;
        self.request(app, id, action).await?;
        Ok(id)
    }

    async fn next(&mut self, app: &mut App, allow_disconnect: bool) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let wake = self
                .receiver
                .recv()
                .with_timeout(deadline.saturating_duration_since(Instant::now()))
                .await
                .map_err(|_| "协调器真实事件等待超时")?
                .ok_or("协调器真实事件流关闭")?;
            let Some((task, event)) = wake else {
                if self.snapshot(app)?.error.is_some() {
                    return Err("生产协调器报告运行错误".into());
                }
                continue;
            };
            if task.task_id != self.task_id || event.generation != self.options.generation {
                return Err("收到其他任务或旧运行代事件".into());
            }
            if let Some(native) = &event.native_session_id {
                if self
                    .native_id
                    .as_ref()
                    .is_some_and(|expected| expected != native)
                {
                    return Err("原生会话身份发生变化".into());
                }
                self.native_id = Some(native.clone());
            }
            let mut value = json!({"event":"runtime","task_id":task.task_id,
                "task_generation":task.generation,"runtime_generation":event.generation,
                "native_session_id":event.native_session_id});
            match event.kind {
                RuntimeEventKind::SessionReady { .. } => {
                    value["kind"] = json!("SessionReady");
                }
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } => {
                    let turn = turn_id.ok_or("原生 ACK 没有回合身份")?;
                    if !self.inputs.iter().any(|input| input.id == message_id)
                        || self.accepted.contains_key(&message_id)
                        || self.accepted.values().any(|existing| existing == &turn)
                        || self.native_id.is_none()
                    {
                        return Err("原生 ACK 重复或未关联实际输入".into());
                    }
                    self.accepted.insert(message_id, turn.clone());
                    value["kind"] = json!("MessageAccepted");
                    value["message_id"] = json!(message_id);
                    value["turn_id"] = json!(turn);
                    value["native_receipt"] = json!(true);
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if !self.accepted.values().any(|turn| turn == &turn_id)
                        || !self.started.insert(turn_id.clone())
                    {
                        return Err("原生 started 未经 ACK 或重复".into());
                    }
                    value["kind"] = json!("TurnStarted");
                    value["turn_id"] = json!(turn_id);
                }
                RuntimeEventKind::TextDelta { turn_id, text, .. } => {
                    if !self.started.contains(&turn_id) {
                        return Err("正文未关联真实运行回合".into());
                    }
                    if !self.text.insert(turn_id.clone()) {
                        continue;
                    }
                    value["kind"] = json!("FirstText");
                    value["turn_id"] = json!(turn_id);
                    value["text_bytes"] = json!(text.len());
                }
                RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } => {
                    if !self.started.contains(&turn_id) || !self.finished.insert(turn_id.clone()) {
                        return Err("终态未关联 started 或重复".into());
                    }
                    let input = self
                        .inputs
                        .iter()
                        .find(|input| self.accepted.get(&input.id) == Some(&turn_id))
                        .ok_or("终态输入关联丢失")?;
                    if outcome != input.outcome
                        || input
                            .expected
                            .as_ref()
                            .is_some_and(|expected| expected != &output)
                    {
                        let observed = match outcome {
                            TurnOutcome::Completed => "Completed",
                            TurnOutcome::Cancelled => "Cancelled",
                            TurnOutcome::Failed { .. } => "Failed",
                        };
                        self.evidence.record(json!({"event":"unexpected_turn_finished",
                            "turn_id":turn_id,"task_generation":task.generation,
                            "runtime_generation":event.generation,"native_session_id":event.native_session_id,
                            "outcome":observed,"output_bytes":output.len(),"output_sha256":sha(&output)}))?;
                        return Err("真实终态或完整正文不符合固定夹具".into());
                    }
                    // 下一条排队输入可立即换代；读取实际历史，而非要求当前快照停在旧代。
                    let history = load_task_generations(&self.sender, self.task_id.clone())?
                        .await
                        .map_err(|_| "终态历史读取确认已关闭")??;
                    let saved = history
                        .iter()
                        .find(|saved| saved.generation == task.generation)
                        .ok_or("真实终态历史代丢失")?;
                    if saved.state != task.state
                        || saved.result.as_ref() != Some(&output)
                        || saved.terminal_evidence.is_none()
                    {
                        return Err("真实终态未完整提交 SQLite".into());
                    }
                    value["kind"] = json!("TurnFinished");
                    value["turn_id"] = json!(turn_id);
                    value["message_id"] = json!(input.id);
                    value["phase"] = json!(input.phase);
                    value["outcome"] = json!(outcome);
                    value["output_bytes"] = json!(output.len());
                    value["output_sha256"] = json!(sha(&output));
                    value["expected_marker_matched"] = json!(input.expected.as_ref().map(|_| true));
                    value["submission_generation"] = json!(input.submission_generation);
                }
                RuntimeEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    method,
                    details,
                } => {
                    let input = self
                        .inputs
                        .iter()
                        .find(|input| self.accepted.get(&input.id) == Some(&turn_id))
                        .ok_or("审批未关联已确认输入")?;
                    let decision = match input.phase {
                        "approval_allow" => ApprovalDecision::AllowOnce,
                        "approval_deny" => ApprovalDecision::DenyOnce,
                        _ => return Err("非审批夹具请求额外工具权限".into()),
                    };
                    let name = if decision == ApprovalDecision::AllowOnce {
                        "approval-allow.txt"
                    } else {
                        "approval-deny.txt"
                    };
                    let call = &details["toolCall"];
                    if !self.approvals.insert(approval_id.clone())
                        || call["kind"] != "edit"
                        || call["_meta"]["x.ai/tool"]["name"] != "write"
                        || call["rawInput"]
                            != json!({"variant":"Write","file_path":self.root.join("project").join(name),"content":CONTENT})
                    {
                        return Err("审批不是固定路径与内容的唯一 Write".into());
                    }
                    value["kind"] = json!("ApprovalRequested");
                    value["approval_id"] = json!(approval_id);
                    value["turn_id"] = json!(turn_id);
                    value["method"] = json!(method);
                    value["exact_write_verified"] = json!(true);
                    self.evidence.record(value.clone())?;
                    let control = Uuid::new_v4();
                    self.evidence.record(
                        json!({"event":"approval_decision_submitted","approval_id":approval_id,
                        "message_id":control,"decision":decision,"native_receipt":false}),
                    )?;
                    self.request(
                        app,
                        control,
                        RuntimeAction::RespondApproval {
                            approval_id,
                            decision,
                        },
                    )
                    .await?;
                    return Ok(value);
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } => {
                    value["kind"] = json!("ApprovalResolved");
                    value["approval_id"] = json!(approval_id);
                    value["decision"] = json!(decision);
                    value["native_receipt"] = json!(false);
                }
                RuntimeEventKind::CommandDispatched {
                    message_id,
                    turn_id,
                } => {
                    value["kind"] = json!("CommandDispatched");
                    value["message_id"] = json!(message_id);
                    value["turn_id"] = json!(turn_id);
                    value["native_receipt"] = json!(false);
                }
                RuntimeEventKind::Disconnected { .. } => {
                    if !allow_disconnect {
                        return Err("运行中连接提前中断".into());
                    }
                    value["kind"] = json!("Disconnected");
                }
                RuntimeEventKind::Progress { .. } => continue,
                RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::RequestFailed { .. } => {
                    return Err("收到未授权工具、合并输入或失败事件".into());
                }
            }
            self.evidence.record(value.clone())?;
            return Ok(value);
        }
    }

    async fn until_finished(
        &mut self,
        app: &mut App,
        primary: Uuid,
        queue_marker: Option<&str>,
        cancel: bool,
    ) -> Result<(), String> {
        let mut queued = None;
        let mut interrupted = false;
        loop {
            let event = self.next(app, false).await?;
            if event["kind"] == "FirstText"
                && self
                    .accepted
                    .get(&primary)
                    .is_some_and(|turn| event["turn_id"] == *turn)
            {
                if let Some(marker) = queue_marker
                    && queued.is_none()
                {
                    queued = Some(
                        self.submit(
                            app,
                            "queue_instruction",
                            format!("不要调用工具，只回复 {marker}，并在本会话记住该标记。"),
                            Some(marker.into()),
                            TurnOutcome::Completed,
                        )
                        .await?,
                    );
                    self.sqlite_snapshot("queued_original_submission").await?;
                }
                if cancel && !interrupted {
                    let turn = self
                        .accepted
                        .get(&primary)
                        .ok_or("取消回合身份丢失")?
                        .clone();
                    let control = Uuid::new_v4();
                    self.evidence.record(json!({"event":"cancel_submitted","message_id":control,"execution_turn_id":turn,
                        "native_input_acknowledged":true,"real_text_started":true}))?;
                    self.request(app, control, RuntimeAction::Interrupt { turn_id: turn })
                        .await?;
                    interrupted = true;
                }
            }
            let primary_done = self
                .accepted
                .get(&primary)
                .is_some_and(|turn| self.finished.contains(turn));
            let queue_done = match queued {
                Some(id) => self
                    .accepted
                    .get(&id)
                    .is_some_and(|turn| self.finished.contains(turn)),
                None => queue_marker.is_none(),
            };
            if primary_done && queue_done {
                if cancel && !interrupted {
                    return Err("取消没有实际派发".into());
                }
                self.sqlite_snapshot("phase_completed").await?;
                return Ok(());
            }
        }
    }

    async fn close(&mut self, app: &mut App) -> Result<(), String> {
        if self.snapshot(app)?.connected {
            self.request(app, Uuid::new_v4(), RuntimeAction::Shutdown)
                .await?;
            while self.snapshot(app)?.connected {
                self.next(app, true).await?;
            }
        }
        // 断开先于退出回执时，等待生产监督者真实清理，不从断开推断退出。
        let token = self.options.generation;
        if !self.cleaned.contains(&token) {
            let deadline = Instant::now() + Duration::from_secs(35);
            let receipt = loop {
                if let Some(receipt) =
                    confirmed_exit(&current_state_dir(), token).map_err(|_| "退出回执校验失败")?
                {
                    break receipt;
                }
                if Instant::now() >= deadline {
                    return Err("真实退出清理回执缺失".into());
                }
                Timer::after(Duration::from_millis(50)).await;
            };
            self.evidence.record(
                json!({"event":"cleanup_confirmed","runtime_generation":token,
                    "runtime_host_receipt":receipt,"cleanup_confirmed":true}),
            )?;
            self.cleaned.insert(token);
        }
        while let Ok(wake) = self.receiver.try_recv() {
            if let Some((task, event)) = wake {
                if !matches!(
                    event.kind,
                    RuntimeEventKind::Disconnected { .. }
                        | RuntimeEventKind::CommandDispatched { .. }
                ) {
                    return Err("清理后仍有未审核执行事件".into());
                }
                self.evidence.record(json!({"event":"shutdown_tail","task_id":task.task_id,"runtime_generation":event.generation}))?;
            }
        }
        Ok(())
    }

    async fn exercise(&mut self, app: &mut App) -> Result<(), String> {
        let task = LocalCliTask {
            version: 1,
            task_id: self.task_id.clone(),
            parent_task_id: None,
            parent_generation: None,
            harness: "grok".into(),
            working_directory: self.options.cwd.to_string_lossy().into(),
            config_json: "{}".into(),
            native_session_id: None,
            generation: 1,
            revision: 0,
            state: LocalCliTaskState::Queued,
            result: None,
            terminal_evidence: None,
        };
        let first_input = vec![InputContent::Text(
            "你好。不要调用工具，只回复 GROK_COORD_ONE。".into(),
        )];
        let first = self.record_input(
            &RuntimeAction::Submit {
                input: first_input.clone(),
            },
            "first_turn",
            1,
            Some("GROK_COORD_ONE".into()),
            TurnOutcome::Completed,
            None,
        )?;
        self.coordinator.update(app, |model, ctx| {
            model.start_with_input(task, self.options.clone(), None, first, first_input, ctx)
        })?;
        self.until_finished(app, first, None, false).await?;
        let second = self
            .submit(
                app,
                "second_turn",
                "第二轮：不要调用工具，只回复 GROK_COORD_TWO。".into(),
                Some("GROK_COORD_TWO".into()),
                TurnOutcome::Completed,
            )
            .await?;
        self.until_finished(app, second, None, false).await?;
        for (phase, name, outcome) in [
            (
                "approval_allow",
                "approval-allow.txt",
                TurnOutcome::Completed,
            ),
            ("approval_deny", "approval-deny.txt", TurnOutcome::Cancelled),
        ] {
            let path = self.options.cwd.join(name);
            if path.exists() || path.is_symlink() {
                return Err("审批目标文件不为空".into());
            }
            let id = self.submit(app,phase,format!("Use only the write tool once to create {} with exactly {CONTENT} (no newline). Request approval. Do not inspect files, run commands, use another tool, or retry if denied. After successful write reply exactly APPROVED.",json!(path)),None,outcome).await?;
            self.until_finished(app, id, None, false).await?;
            let allowed = phase == "approval_allow";
            if allowed {
                if fs::read_to_string(&path).map_err(|_| "允许后目标文件缺失")? != CONTENT
                {
                    return Err("允许后文件内容错误".into());
                }
            } else if path.exists() {
                return Err("拒绝后仍产生目标文件".into());
            }
            self.evidence.record(json!({"event":"file_effect_verified","phase":phase,"allowed":allowed,"exact_file_state_verified":true}))?;
        }
        let marker = format!("GROK_COORD_QUEUED_{}", Uuid::new_v4().simple());
        let queue = self
            .submit(
                app,
                "queue_primary",
                "不要调用工具，逐个列出从1到300的整数，最后输出 QUEUE_PARENT_DONE。".into(),
                None,
                TurnOutcome::Completed,
            )
            .await?;
        self.until_finished(app, queue, Some(&marker), false)
            .await?;
        let cancel = self
            .submit(
                app,
                "cancel",
                "不要调用工具，逐个列出从1到10000的整数，每行一个。".into(),
                None,
                TurnOutcome::Cancelled,
            )
            .await?;
        self.until_finished(app, cancel, None, true).await?;
        self.close(app).await?;
        let before = self.sqlite_snapshot("before_history_reload").await?;
        let previous = self.saved().await?;
        let generation = previous.generation;
        let resumed = next_task_generation(&previous)?;
        self.options.generation = Uuid::new_v4();
        self.options.target = SessionTarget::Resume {
            native_session_id: self.native_id.clone().ok_or("恢复原生身份缺失")?,
        };
        self.coordinator.update(app, |model, ctx| {
            model.start(resumed, self.options.clone(), Some(generation), ctx)
        })?;
        loop {
            let event = self.next(app, false).await?;
            if event["kind"] == "SessionReady" && !event["native_session_id"].is_null() {
                break;
            }
            if event["kind"] != "SessionReady" {
                return Err("恢复加载阶段自动重投了输入".into());
            }
        }
        let ready = self.sqlite_snapshot("resume_ready_no_replay").await?;
        if ready["messages"] != before["messages"]
            || self.inputs.len() != 7
            || self.started.len() != 7
        {
            return Err("恢复加载改变旧消息或自动执行".into());
        }
        let resume = self
            .submit(
                app,
                "resume_result",
                "不要调用工具，只回复上一会话追加指令要求记住的完整 GROK_COORD_QUEUED 标记。"
                    .into(),
                Some(marker),
                TurnOutcome::Completed,
            )
            .await?;
        self.until_finished(app, resume, None, false).await?;
        self.close(app).await?;
        self.sqlite_snapshot("final").await?;
        if self.inputs.len() != 8
            || self.accepted.len() != 8
            || self.started.len() != 8
            || self.finished.len() != 8
            || self.approvals.len() != 2
            || self.cleaned.len() != 2
        {
            return Err("实际输入、执行、审批或清理计数不完整".into());
        }
        Ok(())
    }
}

#[test]
fn grok_coordinator_input_projection_rejects_unknown_fields_and_invalid_types() {
    let link = json!({"message_id":Uuid::new_v4(),"submission_generation":5,
        "runtime_generation":Uuid::new_v4(),"native_turn_id":Uuid::new_v4()});
    assert_eq!(input_link_projection(&link).unwrap(), link);
    for (key, value) in [
        ("output", json!("a".repeat(64))),
        ("link", json!(Uuid::new_v4())),
        ("message_id", json!("a".repeat(64))),
        ("runtime_generation", json!("PRIVATE_LINK_CANARY")),
        ("submission_generation", json!(true)),
        ("submission_generation", json!("5")),
        ("submission_generation", json!(0)),
        ("submission_generation", json!(u64::MAX)),
        ("native_turn_id", json!("PRIVATE_LINK_CANARY")),
    ] {
        let mut changed = link.clone();
        changed[key] = value;
        assert!(input_link_projection(&changed).is_err());
        assert!(pending_inputs_projection(&json!([changed])).is_err());
    }
    let mut missing = link.clone();
    missing.as_object_mut().unwrap().remove("native_turn_id");
    assert!(input_link_projection(&missing).is_err());
}

#[test]
fn grok_coordinator_pending_projection_preserves_only_fixed_input_contract() {
    let link = json!({"message_id":Uuid::new_v4(),"submission_generation":5,
        "runtime_generation":Uuid::new_v4(),"native_turn_id":null});
    assert_eq!(
        pending_inputs_projection(&json!([link])).unwrap(),
        json!([link])
    );
    assert_eq!(
        pending_inputs_projection(&Value::Null).unwrap(),
        Value::Null
    );
    for invalid in [
        json!([null]),
        json!({}),
        json!(vec![link; MAX_NATIVE_INPUTS + 1]),
    ] {
        assert!(pending_inputs_projection(&invalid).is_err());
    }
}

#[test]
#[ignore = "仅由官方隔离运行器执行；调用真实模型并产生费用"]
fn real_grok_root_coordinator() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-coordinator-probe")).unwrap(),
        SCOPE
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    let home = PathBuf::from(env::var_os("HOME").unwrap())
        .canonicalize()
        .unwrap();
    assert!(current_state_dir().starts_with(&home));
    assert!(env::var_os("XAI_API_KEY").is_none() && env::var_os("ANTHROPIC_API_KEY").is_none());
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(artifact.starts_with(&root));
    let mut evidence = Evidence {
        file: File::create(artifact).unwrap(),
    };
    evidence
        .record(
            json!({"event":"acceptance_started","scope":SCOPE,"max_native_inputs":8,
        "production_runtime_commands":true,"test_only_internal_command_switch":false,
        "real_gui_verified":false,"app_restart_verified":false,"sdk_verified":false,
        "parent_permission_ceiling_verified":false,"parent_child_verified":false,
        "credential_files_read_by_probe":false,"permission_policy":"Inherit","local_tools":null}),
        )
        .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let (wake, receiver) = mpsc::unbounded_channel();
        app.update(|ctx| {
            ctx.subscribe_to_model(&coordinator, move |_, event, _| {
                let value = match event {
                    LocalCLITaskCoordinatorEvent::Runtime { task, event } => {
                        Some((task.clone(), event.clone()))
                    }
                    LocalCLITaskCoordinatorEvent::Changed
                    | LocalCLITaskCoordinatorEvent::MessagesChanged { .. }
                    | LocalCLITaskCoordinatorEvent::ResultReady { .. }
                    | LocalCLITaskCoordinatorEvent::ParentMessageReady { .. } => None,
                };
                let _ = wake.send(value);
            })
        });
        let token = Uuid::new_v4();
        let options = SessionOptions {
            executable: PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap()),
            cwd: root.join("project").canonicalize().unwrap(),
            state_dir: current_state_dir(),
            target: SessionTarget::New,
            generation: token,
            permission_policy: PermissionPolicy::Inherit,
            permission_ceiling: None,
            claude_profile: None,
            grok_profile: None,
            model: None,
            local_tools: None,
            selected_skills: Vec::new(),
        };
        let mut probe = Probe {
            coordinator,
            sender: writer.sender.clone(),
            receiver,
            evidence,
            task_id: Uuid::new_v4().to_string(),
            options,
            cleaned: HashSet::new(),
            inputs: Vec::new(),
            accepted: HashMap::new(),
            started: HashSet::new(),
            finished: HashSet::new(),
            text: HashSet::new(),
            native_id: None,
            approvals: HashSet::new(),
            root,
        };
        let result = probe.exercise(&mut app).await;
        let cleanup = probe.close(&mut app).await;
        // 底层错误可能包含 CLI 原文，诊断仅保存长度和摘要，不公开错误树。
        if let Err(error) = &result {
            probe.evidence.record(json!({"event":"exercise_failed","reason_bytes":error.len(),"reason_sha256":sha(error)})).unwrap();
        }
        if let Err(error) = &cleanup {
            probe.evidence.record(json!({"event":"cleanup_failed","reason_bytes":error.len(),"reason_sha256":sha(error)})).unwrap();
        }
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleanup) {
            Ok(()) => probe
                .evidence
                .record(json!({"event":"acceptance_passed","scope":SCOPE,
                "native_inputs":8,"native_executions":8,"native_acknowledged_inputs":8,
                "completed_inputs":6,"cancelled_inputs":2,"approvals_verified":2,
                "same_native_session_verified":true,"sqlite_verified":true,"cleanup_confirmed":true,
                "real_gui_verified":false,"app_restart_verified":false,"sdk_verified":false,
                "parent_child_verified":false,"parent_permission_ceiling_verified":false}))
                .unwrap(),
            Err(_) => {
                probe
                    .evidence
                    .record(json!({"event":"acceptance_failed","scope":SCOPE,
                    "reason":"真实 Grok 根任务协调器验收未通过；不归算完成"}))
                    .unwrap();
                panic!("真实 Grok 根任务协调器验收未通过；请检查安全事件与私有诊断");
            }
        }
    });
}
