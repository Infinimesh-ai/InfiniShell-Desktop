//! 独占 Grok 进程的原生工具租约；原生字段缺失与派生的进程能力来源分别保留。

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_RECORDS: usize = 256;
const MAX_BYTES: usize = 1024 * 1024;
const LEASE_DURATION: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrokToolLeaseState {
    Streaming,
    Ready,
    Bound,
    ReplyWritten,
    Closed,
}

#[derive(Clone)]
struct ToolLease {
    runtime_generation: Uuid,
    turn_id: String,
    qualified_name: String,
    final_fingerprint: Option<[u8; 32]>,
    approved: bool,
    native_completed: bool,
    reply_success: Option<bool>,
    expires: Option<Instant>,
    state: GrokToolLeaseState,
}

/// 只能由真实工具账本与当前独占连接创建，不从 MCP 或模型参数反序列化。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedGrokToolLease {
    process_epoch: Uuid,
    runtime_generation: Uuid,
    server_id: String,
    native_call_id: String,
    turn_id: String,
    inner_key: String,
}

impl VerifiedGrokToolLease {
    pub(crate) fn process_epoch(&self) -> Uuid {
        self.process_epoch
    }
    pub(crate) fn runtime_generation(&self) -> Uuid {
        self.runtime_generation
    }
    pub(crate) fn turn_id(&self) -> &str {
        &self.turn_id
    }
    pub(crate) fn native_call_id(&self) -> &str {
        &self.native_call_id
    }
}

#[derive(Clone)]
struct InnerBinding {
    fingerprint: [u8; 32],
    lease: VerifiedGrokToolLease,
}

#[derive(Clone)]
pub(crate) struct GrokToolLeaseLedger {
    process_epoch: Uuid,
    runtime_generation: Uuid,
    server_id: String,
    server_name: String,
    native_session_id: String,
    confirmed_turn_id: Option<String>,
    retired: bool,
    leases: HashMap<String, ToolLease>,
    events: HashMap<String, [u8; 32]>,
    approvals: HashMap<String, ([u8; 32], bool)>,
    inner: HashMap<String, InnerBinding>,
    outer: HashMap<String, ([u8; 32], String)>,
}

impl GrokToolLeaseLedger {
    /// 调用者先确认 owned process、generation、注册 nonce 和原生会话；本模块不启动进程。
    pub(crate) fn new(
        process_epoch: Uuid,
        runtime_generation: Uuid,
        server_id: String,
        server_name: String,
        native_session_id: String,
    ) -> Result<Self, String> {
        valid_id(&server_id)?;
        valid_id(&server_name)?;
        valid_id(&native_session_id)?;
        Ok(Self {
            process_epoch,
            runtime_generation,
            server_id,
            server_name,
            native_session_id,
            confirmed_turn_id: None,
            retired: false,
            leases: HashMap::new(),
            events: HashMap::new(),
            approvals: HashMap::new(),
            inner: HashMap::new(),
            outer: HashMap::new(),
        })
    }

    /// 目录只可复用本进程、代次、注册 nonce 和原生会话的活跃账本，不授予任何工具租约。
    pub(crate) fn matches_catalog_connection(
        &self,
        process_epoch: Uuid,
        runtime_generation: Uuid,
        server_id: &str,
        native_session_id: &str,
    ) -> bool {
        !self.retired
            && self.process_epoch == process_epoch
            && self.runtime_generation == runtime_generation
            && self.server_id == server_id
            && self.server_name == super::local_tools::MCP_SERVER_NAME
            && self.native_session_id == native_session_id
    }

    /// 回合身份来自已验证的 Started；普通完成可沿用进程，取消必须另建 epoch。
    pub(crate) fn begin_turn(
        &mut self,
        runtime_generation: Uuid,
        turn_id: &str,
    ) -> Result<(), String> {
        self.active()?;
        valid_id(turn_id)?;
        if let Some(previous) = self.confirmed_turn_id.as_deref() {
            if previous == turn_id {
                return if runtime_generation == self.runtime_generation {
                    Ok(())
                } else {
                    Err("同一原生回合改变了执行代次".into())
                };
            }
            // 同一连接的 generation 可以保持不变；新原生回合仍需旧租约全部闭合。
            if self
                .leases
                .values()
                .any(|lease| lease.state != GrokToolLeaseState::Closed)
            {
                return Err("上一原生回合仍有未闭合工具租约".into());
            }
        }
        self.runtime_generation = runtime_generation;
        self.confirmed_turn_id = Some(turn_id.into());
        Ok(())
    }

    /// 必须传入当前 owned transport 的真实 NativeTool 帧；缺失 session/prompt 字段不会回填。
    pub(crate) fn observe_native_tool(
        &mut self,
        message: &Value,
        now: Instant,
    ) -> Result<(), String> {
        self.active()?;
        self.expire_unbound(now)?;
        let params = &message["params"];
        let update = &params["update"];
        let turn = params["_meta"]["promptId"]
            .as_str()
            .ok_or("工具帧缺少原生回合")?;
        let event = params["_meta"]["eventId"]
            .as_str()
            .ok_or("工具帧缺少原生事件 ID")?;
        let call = update["toolCallId"]
            .as_str()
            .ok_or("工具帧缺少原生调用 ID")?;
        valid_id(event)?;
        valid_id(call)?;
        if message["jsonrpc"] != "2.0"
            || message["method"] != "session/update"
            || message.get("id").is_some()
            || message.get("result").is_some()
            || message.get("error").is_some()
            || params["sessionId"] != self.native_session_id
            || Some(turn) != self.confirmed_turn_id.as_deref()
            || params["_meta"]["isReplay"] == true
            || update["_meta"]["isReplay"] == true
        {
            return Err("工具帧不是当前进程的已确认原生回合".into());
        }
        let frame_hash = fingerprint(message)?;
        if let Some(previous) = self.events.get(event) {
            return if previous == &frame_hash {
                Ok(())
            } else {
                Err("原生事件 ID 被不同内容复用".into())
            };
        }
        if self.events.len() >= MAX_RECORDS {
            return Err("原生工具事件预算已耗尽".into());
        }
        match update["sessionUpdate"].as_str() {
            Some("tool_call") => {
                if update["_meta"]["x.ai/tool"]["name"] != "use_tool" {
                    return Err("原生工具不是 UseTool".into());
                }
                let (name, _) = self.native_input(&update["rawInput"], false)?;
                if let Some(previous) = self.leases.get(call) {
                    if previous.runtime_generation != self.runtime_generation
                        || previous.turn_id != turn
                        || previous.qualified_name != name
                    {
                        return Err("原生工具调用 ID 改变了身份".into());
                    }
                } else {
                    if self.leases.len() >= MAX_RECORDS {
                        return Err("原生工具租约预算已耗尽".into());
                    }
                    self.leases.insert(
                        call.into(),
                        ToolLease {
                            runtime_generation: self.runtime_generation,
                            turn_id: turn.into(),
                            qualified_name: name,
                            final_fingerprint: None,
                            approved: false,
                            native_completed: false,
                            reply_success: None,
                            expires: None,
                            state: GrokToolLeaseState::Streaming,
                        },
                    );
                }
            }
            Some("tool_call_update") => {
                let previous = self.leases.get(call).ok_or("工具更新没有初始账本")?;
                if previous.runtime_generation != self.runtime_generation
                    || previous.turn_id != turn
                {
                    return Err("工具更新跨越了原生回合身份".into());
                }
                if update.get("rawInput").is_some() {
                    let (name, input_hash) = self.native_input(&update["rawInput"], true)?;
                    let lease = self
                        .leases
                        .get_mut(call)
                        .ok_or("完整工具输入没有初始账本")?;
                    if lease.runtime_generation != self.runtime_generation
                        || lease.qualified_name != name
                        || lease.turn_id != turn
                        || lease.native_completed
                        || lease
                            .final_fingerprint
                            .is_some_and(|previous| previous != input_hash)
                        || update.get("kind").is_some_and(|kind| kind != "other")
                        || update["_meta"]["x.ai/tool"]
                            .get("name")
                            .is_some_and(|name| name != "use_tool")
                    {
                        return Err("完整工具输入改变了原生租约".into());
                    }
                    lease.final_fingerprint = Some(input_hash);
                    if lease.state == GrokToolLeaseState::Streaming {
                        lease.state = GrokToolLeaseState::Ready;
                    }
                }
                if update["status"] == "completed" {
                    let lease = self.leases.get_mut(call).ok_or("工具完成没有初始账本")?;
                    lease.native_completed = true;
                    match lease.state {
                        GrokToolLeaseState::ReplyWritten
                        | GrokToolLeaseState::Streaming
                        | GrokToolLeaseState::Ready => lease.state = GrokToolLeaseState::Closed,
                        GrokToolLeaseState::Bound | GrokToolLeaseState::Closed => {}
                    }
                } else if update["status"] == "failed" {
                    let lease = self.leases.get_mut(call).ok_or("工具失败没有初始账本")?;
                    // 已实际回写的业务错误只结束对应调用；未知原生失败仍退休整个能力。
                    if lease.state == GrokToolLeaseState::ReplyWritten
                        && lease.reply_success == Some(false)
                    {
                        lease.native_completed = true;
                        lease.state = GrokToolLeaseState::Closed;
                    } else {
                        self.retire();
                        return Err("原生工具失败，旧 SDK 进程能力已退休".into());
                    }
                }
            }
            Some(_) | None => return Err("不是原生工具生命周期帧".into()),
        }
        self.events.insert(event.into(), frame_hash);
        Ok(())
    }

    /// 仅在应用已处理真实审批后调用；许可的输入仍须匹配完整 NativeTool 账本。
    pub(crate) fn record_permission(
        &mut self,
        message: &Value,
        allowed: bool,
        now: Instant,
    ) -> Result<(), String> {
        self.active()?;
        self.expire_unbound(now)?;
        let params = &message["params"];
        let tool = &params["toolCall"];
        let call = tool["toolCallId"].as_str().ok_or("审批缺少原生调用 ID")?;
        let approval_key = rpc_key(&message["id"])?;
        let frame_hash = fingerprint(message)?;
        if let Some((previous, decision)) = self.approvals.get(&approval_key) {
            return if previous == &frame_hash && *decision == allowed {
                Ok(())
            } else {
                Err("审批请求身份或决定发生冲突".into())
            };
        }
        let (name, input_hash) = self.native_input(&tool["rawInput"], true)?;
        let lease = self
            .leases
            .get_mut(call)
            .ok_or("审批没有完整原生工具租约")?;
        if message["jsonrpc"] != "2.0"
            || message["method"] != "session/request_permission"
            || message.get("result").is_some()
            || message.get("error").is_some()
            || params["sessionId"] != self.native_session_id
            || params["_meta"]["isReplay"] == true
            || tool["_meta"]["isReplay"] == true
            || tool["kind"] != "other"
            || params["_meta"]
                .get("promptId")
                .is_some_and(|id| id.as_str() != Some(lease.turn_id.as_str()))
            || tool["_meta"]["x.ai/tool"]
                .get("name")
                .is_some_and(|name| name != "use_tool")
            || lease.runtime_generation != self.runtime_generation
            || lease.qualified_name != name
            || lease.final_fingerprint != Some(input_hash)
            || lease.state != GrokToolLeaseState::Ready
            || lease.native_completed
        {
            return Err("审批未匹配完整原生工具输入与身份".into());
        }
        if self.approvals.len() >= MAX_RECORDS {
            return Err("审批身份预算已耗尽".into());
        }
        lease.approved = allowed;
        // 等待用户不占用回调期限；真实许可后才等待 SDK 业务请求。
        lease.expires = allowed.then_some(now + LEASE_DURATION);
        self.approvals.insert(approval_key, (frame_hash, allowed));
        if !allowed {
            self.retire();
        }
        Ok(())
    }

    /// 新事务原子绑定唯一已批准租约；已有事务保留原绑定，绝不再查当前回合。
    pub(crate) fn bind_sdk(
        &mut self,
        message: &Value,
        now: Instant,
    ) -> Result<VerifiedGrokToolLease, String> {
        self.active()?;
        self.expire_unbound(now)?;
        let params = &message["params"];
        let inner = &params["message"];
        if !closed(message, &["jsonrpc", "id", "method", "params"])
            || message["jsonrpc"] != "2.0"
            || !matches!(
                message["method"].as_str(),
                Some("x.ai/mcp/sdk_call" | "_x.ai/mcp/sdk_call")
            )
            || !closed(params, &["serverId", "message"])
            || params["serverId"] != self.server_id
            || !closed(inner, &["jsonrpc", "id", "method", "params"])
            || inner["jsonrpc"] != "2.0"
            || inner["method"] != "tools/call"
            || !inner["params"].as_object().is_some_and(|fields| {
                (fields.len() == 2 || fields.len() == 3)
                    && fields.contains_key("name")
                    && fields.contains_key("arguments")
                    && fields
                        .keys()
                        .all(|key| matches!(key.as_str(), "name" | "arguments" | "_meta"))
            })
        {
            return Err("SDK 调用不是当前进程的封闭业务事务".into());
        }
        let outer_key = rpc_key(&message["id"])?;
        let inner_key = rpc_key(&inner["id"])?;
        let outer_hash = fingerprint(message)?;
        let inner_hash = fingerprint(inner)?;
        if let Some((previous, registered_inner)) = self.outer.get(&outer_key) {
            if previous != &outer_hash || registered_inner != &inner_key {
                return Err("SDK 外层事务 ID 内容冲突".into());
            }
        }
        if let Some(previous) = self.inner.get(&inner_key) {
            if previous.fingerprint != inner_hash {
                return Err("SDK 内层事务 ID 内容冲突".into());
            }
            let proof = previous.lease.clone();
            if !self.outer.contains_key(&outer_key) {
                if self.outer.len() >= MAX_RECORDS {
                    return Err("SDK 外层事务预算已耗尽".into());
                }
                self.outer.insert(outer_key, (outer_hash, inner_key));
            }
            return Ok(proof);
        }
        if self.inner.len() >= MAX_RECORDS || self.outer.len() >= MAX_RECORDS {
            return Err("SDK 事务预算已耗尽".into());
        }
        let tool = inner["params"]["name"].as_str().ok_or("SDK 缺少工具名称")?;
        valid_id(tool)?;
        let qualified_name = format!("{}__{tool}", self.server_name);
        let input_hash = input_fingerprint(&qualified_name, &inner["params"]["arguments"])?;
        let candidates = self
            .leases
            .iter()
            .filter(|(_, lease)| {
                lease.runtime_generation == self.runtime_generation
                    && lease.state == GrokToolLeaseState::Ready
                    && lease.approved
                    && !lease.native_completed
                    && lease.qualified_name == qualified_name
                    && lease.final_fingerprint == Some(input_hash)
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            if candidates.len() > 1 {
                self.retire();
            }
            return Err("SDK 调用没有唯一完整原生工具租约".into());
        }
        let call = candidates.into_iter().next().expect("唯一候选已核对");
        let lease = self.leases.get_mut(&call).expect("候选租约存在");
        lease.state = GrokToolLeaseState::Bound;
        let proof = VerifiedGrokToolLease {
            process_epoch: self.process_epoch,
            runtime_generation: self.runtime_generation,
            server_id: self.server_id.clone(),
            native_call_id: call,
            turn_id: lease.turn_id.clone(),
            inner_key: inner_key.clone(),
        };
        self.inner.insert(
            inner_key.clone(),
            InnerBinding {
                fingerprint: inner_hash,
                lease: proof.clone(),
            },
        );
        self.outer.insert(outer_key, (outer_hash, inner_key));
        Ok(proof)
    }

    pub(crate) fn verify_bound(
        &self,
        proof: &VerifiedGrokToolLease,
        server_id: &str,
        mcp_id: &Value,
    ) -> Result<(), String> {
        self.active()?;
        if proof.process_epoch != self.process_epoch
            || proof.server_id != self.server_id
            || server_id != self.server_id
            || rpc_key(mcp_id)? != proof.inner_key
            || !self
                .inner
                .get(&proof.inner_key)
                .is_some_and(|binding| binding.lease == *proof)
        {
            return Err("工具回复不属于当前进程的已绑定租约".into());
        }
        Ok(())
    }

    /// 由 transport 在真实 SDK 回复写入成功后调用；缓存或排队不等于写入。
    pub(crate) fn record_reply_written(
        &mut self,
        proof: &VerifiedGrokToolLease,
        success: bool,
    ) -> Result<(), String> {
        self.active()?;
        self.verify_bound(
            proof,
            &self.server_id,
            &serde_json::from_str::<Value>(&proof.inner_key).map_err(|_| "租约事务 ID 无效")?,
        )?;
        let lease = self
            .leases
            .get_mut(&proof.native_call_id)
            .ok_or("回复缺少原生租约")?;
        if lease
            .reply_success
            .is_some_and(|previous| previous != success)
        {
            return Err("重复回写改变了工具业务结果".into());
        }
        match lease.state {
            GrokToolLeaseState::Bound => {
                lease.state = if lease.native_completed {
                    GrokToolLeaseState::Closed
                } else {
                    GrokToolLeaseState::ReplyWritten
                }
            }
            GrokToolLeaseState::ReplyWritten | GrokToolLeaseState::Closed => {}
            GrokToolLeaseState::Streaming | GrokToolLeaseState::Ready => {
                return Err("工具回复尚未绑定业务事务".into());
            }
        }
        lease.reply_success = Some(success);
        Ok(())
    }

    pub(crate) fn state(
        &self,
        proof: &VerifiedGrokToolLease,
    ) -> Result<GrokToolLeaseState, String> {
        self.active()?;
        if proof.process_epoch != self.process_epoch || proof.server_id != self.server_id {
            return Err("租约属于旧进程代次".into());
        }
        self.leases
            .get(&proof.native_call_id)
            .filter(|lease| lease.runtime_generation == proof.runtime_generation)
            .map(|lease| lease.state)
            .ok_or("租约不存在".into())
    }

    /// 调用者随后须停止 owned process；仅设置状态并不宣称已取消或回收进程。
    pub(crate) fn retire(&mut self) {
        self.retired = true;
    }

    pub(crate) fn is_retired(&self) -> bool {
        self.retired
    }

    fn active(&self) -> Result<(), String> {
        if self.retired {
            Err("旧 SDK 进程能力已退休".into())
        } else {
            Ok(())
        }
    }

    fn expire_unbound(&mut self, now: Instant) -> Result<(), String> {
        if self.leases.values().any(|lease| {
            matches!(
                lease.state,
                GrokToolLeaseState::Streaming | GrokToolLeaseState::Ready
            ) && lease.expires.is_some_and(|expires| now >= expires)
        }) {
            self.retire();
            return Err("未绑定原生工具租约过期，旧进程能力已退休".into());
        }
        Ok(())
    }

    fn native_input(&self, input: &Value, final_input: bool) -> Result<(String, [u8; 32]), String> {
        let fields = input.as_object().ok_or("原生工具输入不是对象")?;
        if fields.len() != if final_input { 3 } else { 2 }
            || !fields.contains_key("tool_name")
            || !fields.contains_key("tool_input")
            || (final_input && input["variant"] != "UseTool")
        {
            return Err("原生工具输入不是完整 UseTool 契约".into());
        }
        let name = input["tool_name"].as_str().ok_or("原生工具缺少名称")?;
        valid_id(name)?;
        if !name.starts_with(&format!("{}__", self.server_name)) {
            return Err("原生工具不是已注册服务".into());
        }
        Ok((name.into(), input_fingerprint(name, &input["tool_input"])?))
    }
}

fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 256 {
        Err("原生身份字段无效".into())
    } else {
        Ok(())
    }
}

fn rpc_key(id: &Value) -> Result<String, String> {
    if id.as_u64().is_none()
        && id
            .as_str()
            .is_none_or(|value| value.is_empty() || value.len() > 256)
    {
        return Err("SDK 事务 ID 无效".into());
    }
    Ok(id.to_string())
}

fn closed(value: &Value, keys: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.len() == keys.len() && fields.keys().all(|key| keys.contains(&key.as_str()))
    })
}

fn fingerprint(value: &Value) -> Result<[u8; 32], String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "工具指纹编码失败")?;
    if bytes.len() > MAX_BYTES {
        return Err("工具输入超过预算".into());
    }
    Ok(Sha256::digest(bytes).into())
}

fn input_fingerprint(name: &str, arguments: &Value) -> Result<[u8; 32], String> {
    if !arguments.is_object() {
        return Err("工具参数必须是对象".into());
    }
    fingerprint(&json!({"qualified_name": name, "arguments": arguments}))
}

#[cfg(test)]
#[path = "grok_tool_lease_tests.rs"]
mod tests;
