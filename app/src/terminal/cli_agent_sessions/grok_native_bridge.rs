//! 普通本地 PTY 的固定 Grok 原生事务客户端；不启动用户 CLI、不发按键、不重投。
//!
//! 所有阻塞发现与 I/O 必须在后台执行。调用者须先持久领取消息，才可调用 submit；
//! 写入后的任何错误均属未知结果，只能携带原身份查询，不能据此清除用户草稿。

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::grok_native_bridge_prompt::{NativeBridgeImage, NativeBridgePrompt, frame_too_large};
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

#[cfg(unix)]
#[path = "grok_native_bridge_transport_unix.rs"]
mod transport;
#[cfg(windows)]
#[path = "grok_native_bridge_transport_windows.rs"]
mod transport;
pub(crate) use transport::NativeBridgeBinding;
use transport::Transport;

const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_TEXT_BYTES: usize = 128 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const LEASE_LIFETIME: Duration = Duration::from_secs(5);

/// 查询必须携带持久领取时的期望身份，不能采信回包中的另一会话或另一正文。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeMessage {
    pub message_id: Uuid,
    pub session_id: String,
    pub payload_digest: String,
}

impl NativeBridgeMessage {
    pub(crate) fn new(message_id: Uuid, session_id: String, text: &str) -> io::Result<Self> {
        if message_id.is_nil()
            || !valid_session(&session_id)
            || text.trim().is_empty()
            || text.len() > MAX_TEXT_BYTES
        {
            return Err(invalid());
        }
        Ok(Self {
            message_id,
            session_id,
            payload_digest: format!("blake3:{}", blake3::hash(text.as_bytes()).to_hex()),
        })
    }

    pub(crate) fn for_prompt(
        message_id: Uuid,
        session_id: String,
        prompt: &NativeBridgePrompt,
    ) -> io::Result<Self> {
        let expected = Self {
            message_id,
            session_id,
            payload_digest: prompt.payload_digest().to_owned(),
        };
        expected.validate()?;
        Ok(expected)
    }

    fn validate(&self) -> io::Result<()> {
        if self.message_id.is_nil()
            || !valid_session(&self.session_id)
            || !self
                .payload_digest
                .strip_prefix("blake3:")
                .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(lower_hex))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeBridgeReceiptState {
    Claimed,
    Dispatched,
    NativeAcknowledged,
    NativeRejected,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeReceipt {
    pub message_id: String,
    pub prompt_id: String,
    pub session_id: String,
    pub payload_digest: String,
    pub state: NativeBridgeReceiptState,
}

/// Unknown 和所有 I/O 错误都不能解锁重投；Rejected 也不代表先前消息从未提交。
pub(crate) enum NativeBridgeResponse {
    Receipt(NativeBridgeReceipt),
    Unknown,
    Rejected { reason: String },
}

pub(crate) struct NativeBridgeState {
    pub ready: bool,
    pub input_epoch: u64,
    pub reason: Option<String>,
    pub lease: Option<NativeBridgeLease>,
    pub typed_png_images: u32,
}

/// 不实现 Clone、Serialize 或 Deserialize，过期租约不能从磁盘重新获得授权。
pub(crate) struct NativeBridgeLease {
    instance_id: Uuid,
    issued_before: Instant,
    wire: WireLease,
    typed_png_images: u32,
}

/// 只确认首页初始化的固定目标，不授予正文写入权，也不是接收回执。
pub(crate) struct NativeBridgePreparation {
    instance_id: Uuid,
    input_epoch: u64,
    agent_id: usize,
    expected_binding_epoch: u32,
    expected_session_id: String,
}

impl NativeBridgePreparation {
    pub(crate) fn lease_from_state(
        &self,
        state: NativeBridgeState,
    ) -> io::Result<Option<NativeBridgeLease>> {
        if state.input_epoch < self.input_epoch {
            return Err(invalid());
        }
        let Some(lease) = state.lease else {
            return Ok(None);
        };
        if !state.ready
            || lease.instance_id != self.instance_id
            || lease.wire.agent_id != self.agent_id
            || lease.wire.binding_epoch != self.expected_binding_epoch
            || lease.wire.session_id != self.expected_session_id
        {
            return Err(invalid());
        }
        Ok(Some(lease))
    }
}

impl NativeBridgeLease {
    pub(crate) fn session_id(&self) -> &str {
        &self.wire.session_id
    }
    pub(crate) fn binding_epoch(&self) -> u32 {
        self.wire.binding_epoch
    }
    pub(crate) fn input_epoch(&self) -> u64 {
        self.wire.input_epoch
    }
}

pub(crate) struct NativeBridge {
    transport: Transport,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireLease {
    lease_id: String,
    session_id: String,
    agent_id: usize,
    binding_epoch: u32,
    input_epoch: u64,
}

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Request<'a> {
    State {
        instance_id: &'a str,
    },
    PrepareSessionIfIdle {
        instance_id: &'a str,
        input_epoch: u64,
    },
    SubmitIfIdle {
        instance_id: &'a str,
        lease_id: &'a str,
        session_id: &'a str,
        binding_epoch: u32,
        input_epoch: u64,
        message_id: String,
        prompt_id: String,
        text: &'a str,
        #[serde(skip_serializing_if = "<[NativeBridgeImage]>::is_empty")]
        images: &'a [NativeBridgeImage],
    },
    QueryReceipt {
        instance_id: &'a str,
        message_id: String,
    },
}

#[derive(Serialize)]
struct Envelope<'a> {
    token: &'a str,
    request: Request<'a>,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum WireResponse {
    State {
        instance_id: String,
        input_epoch: u64,
        ready: bool,
        reason: Option<String>,
        lease: Option<WireLease>,
        #[serde(default)]
        typed_png_images: u32,
    },
    Prepared {
        instance_id: String,
        input_epoch: u64,
        agent_id: usize,
        expected_binding_epoch: u32,
        expected_session_id: String,
    },
    NotReady {
        reason: String,
    },
    Receipt {
        receipt: NativeBridgeReceipt,
    },
    Unknown {
        message_id: String,
    },
    Rejected {
        reason: String,
    },
}

impl NativeBridge {
    pub(crate) fn discover(root: &Path, pty: LocalPtyIdentity) -> io::Result<Option<Self>> {
        Ok(Transport::discover(root, pty)?.map(|transport| Self { transport }))
    }

    pub(crate) fn binding(&self) -> &NativeBridgeBinding {
        self.transport.binding()
    }

    pub(crate) fn state(&self) -> io::Result<NativeBridgeState> {
        let issued_before = Instant::now();
        let response = self.exchange(
            Request::State {
                instance_id: &self.binding().instance_id.to_string(),
            },
            None,
            || true,
        )?;
        parse_state(response, self.binding().instance_id, issued_before)
    }

    /// 仅由用户本次提交在首页触发一次；请求不含正文，响应丢失后不得重试初始化。
    pub(crate) fn prepare_session_if_idle(
        &self,
        state: NativeBridgeState,
    ) -> io::Result<Option<NativeBridgePreparation>> {
        if state.ready
            || state.lease.is_some()
            || state.reason.as_deref() != Some("no_active_agent")
        {
            return Err(invalid());
        }
        let response = self.exchange(
            Request::PrepareSessionIfIdle {
                instance_id: &self.binding().instance_id.to_string(),
                input_epoch: state.input_epoch,
            },
            None,
            || true,
        )?;
        parse_preparation(response, self.binding().instance_id, state.input_epoch)
    }

    /// 租约按值消费；任何部分写入或响应丢失都留给持久消息身份做只读查询。
    pub(crate) fn submit(
        &self,
        lease: NativeBridgeLease,
        message_id: Uuid,
        text: &str,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<NativeBridgeResponse> {
        self.submit_prompt(lease, message_id, &NativeBridgePrompt::text(text)?, admit)
    }

    /// 必须在持久领取前执行；完整帧包括 token、真实会话和 JSON 转义开销。
    pub(crate) fn validate_prompt(
        &self,
        lease: &NativeBridgeLease,
        message_id: Uuid,
        prompt: &NativeBridgePrompt,
    ) -> io::Result<()> {
        if lease.instance_id != self.binding().instance_id
            || lease.issued_before.elapsed() >= LEASE_LIFETIME
        {
            return Err(invalid());
        }
        validate_prompt_frame(lease, message_id, prompt)
    }

    pub(crate) fn submit_prompt(
        &self,
        lease: NativeBridgeLease,
        message_id: Uuid,
        prompt: &NativeBridgePrompt,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<NativeBridgeResponse> {
        self.validate_prompt(&lease, message_id, prompt)?;
        let expected =
            NativeBridgeMessage::for_prompt(message_id, lease.wire.session_id.clone(), prompt)?;
        let response = self.exchange(
            Request::SubmitIfIdle {
                instance_id: &self.binding().instance_id.to_string(),
                lease_id: &lease.wire.lease_id,
                session_id: &lease.wire.session_id,
                binding_epoch: lease.wire.binding_epoch,
                input_epoch: lease.wire.input_epoch,
                message_id: message_id.to_string(),
                prompt_id: message_id.to_string(),
                text: prompt.text_value(),
                images: prompt.images(),
            },
            Some(lease.issued_before + LEASE_LIFETIME),
            admit,
        )?;
        parse_receipt(response, &expected)
    }

    pub(crate) fn query(&self, expected: &NativeBridgeMessage) -> io::Result<NativeBridgeResponse> {
        expected.validate()?;
        let response = self.exchange(
            Request::QueryReceipt {
                instance_id: &self.binding().instance_id.to_string(),
                message_id: expected.message_id.to_string(),
            },
            None,
            || true,
        )?;
        parse_receipt(response, expected)
    }

    fn exchange(
        &self,
        request: Request<'_>,
        lease_deadline: Option<Instant>,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<WireResponse> {
        let bytes = self.transport.exchange(request, lease_deadline, admit)?;
        serde_json::from_slice(&bytes).map_err(|_| invalid())
    }
}

fn validate_prompt_frame(
    lease: &NativeBridgeLease,
    message_id: Uuid,
    prompt: &NativeBridgePrompt,
) -> io::Result<()> {
    NativeBridgeMessage::for_prompt(message_id, lease.wire.session_id.clone(), prompt)?;
    if prompt.has_images() && lease.typed_png_images != 1 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "grok_native_bridge.typed_png_unavailable",
        ));
    }
    // 发现器已严格验证 token 为 64 个小写十六进制字符，故占位符编码长度精确相同。
    let bytes = serde_json::to_vec(&Envelope {
        token: &"0".repeat(64),
        request: Request::SubmitIfIdle {
            instance_id: &lease.instance_id.to_string(),
            lease_id: &lease.wire.lease_id,
            session_id: &lease.wire.session_id,
            binding_epoch: lease.wire.binding_epoch,
            input_epoch: lease.wire.input_epoch,
            message_id: message_id.to_string(),
            prompt_id: message_id.to_string(),
            text: prompt.text_value(),
            images: prompt.images(),
        },
    })
    .map_err(|_| invalid())?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(frame_too_large());
    }
    Ok(())
}

fn parse_state(
    response: WireResponse,
    expected: Uuid,
    issued_before: Instant,
) -> io::Result<NativeBridgeState> {
    let WireResponse::State {
        instance_id,
        input_epoch,
        ready,
        reason,
        lease,
        typed_png_images,
    } = response
    else {
        return Err(invalid());
    };
    if canonical_uuid(&instance_id)? != expected
        || ready != lease.is_some()
        || ready == reason.is_some()
        || reason.as_ref().is_some_and(|reason| !valid_reason(reason))
        || issued_before.elapsed() >= LEASE_LIFETIME
    {
        return Err(invalid());
    }
    let lease = lease
        .map(|wire| {
            canonical_uuid(&wire.lease_id)?;
            if !valid_session(&wire.session_id) || wire.input_epoch != input_epoch {
                return Err(invalid());
            }
            Ok(NativeBridgeLease {
                instance_id: expected,
                issued_before,
                wire,
                typed_png_images,
            })
        })
        .transpose()?;
    Ok(NativeBridgeState {
        ready,
        input_epoch,
        reason,
        lease,
        typed_png_images,
    })
}

fn parse_receipt(
    response: WireResponse,
    expected: &NativeBridgeMessage,
) -> io::Result<NativeBridgeResponse> {
    expected.validate()?;
    match response {
        WireResponse::Receipt { receipt } => {
            if receipt.message_id != expected.message_id.to_string()
                || receipt.prompt_id != receipt.message_id
                || receipt.session_id != expected.session_id
                || receipt.payload_digest != expected.payload_digest
            {
                return Err(invalid());
            }
            Ok(NativeBridgeResponse::Receipt(receipt))
        }
        WireResponse::Unknown { message_id } if message_id == expected.message_id.to_string() => {
            Ok(NativeBridgeResponse::Unknown)
        }
        WireResponse::Rejected { reason } if valid_reason(&reason) => {
            Ok(NativeBridgeResponse::Rejected { reason })
        }
        WireResponse::State { .. }
        | WireResponse::Prepared { .. }
        | WireResponse::NotReady { .. }
        | WireResponse::Unknown { .. }
        | WireResponse::Rejected { .. } => Err(invalid()),
    }
}

fn parse_preparation(
    response: WireResponse,
    expected_instance: Uuid,
    requested_epoch: u64,
) -> io::Result<Option<NativeBridgePreparation>> {
    match response {
        WireResponse::Prepared {
            instance_id,
            input_epoch,
            agent_id,
            expected_binding_epoch,
            expected_session_id,
        } => {
            if canonical_uuid(&instance_id)? != expected_instance
                || requested_epoch.checked_add(1) != Some(input_epoch)
                || !valid_session(&expected_session_id)
            {
                return Err(invalid());
            }
            Ok(Some(NativeBridgePreparation {
                instance_id: expected_instance,
                input_epoch,
                agent_id,
                expected_binding_epoch,
                expected_session_id,
            }))
        }
        WireResponse::NotReady { reason } | WireResponse::Rejected { reason }
            if valid_reason(&reason) =>
        {
            Ok(None)
        }
        WireResponse::State { .. }
        | WireResponse::Receipt { .. }
        | WireResponse::Unknown { .. }
        | WireResponse::NotReady { .. }
        | WireResponse::Rejected { .. } => Err(invalid()),
    }
}

fn canonical_uuid(value: &str) -> io::Result<Uuid> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil() && id.to_string() == value)
        .ok_or_else(invalid)
}

fn valid_instance_name(value: &str) -> bool {
    value
        .strip_prefix("t-")
        .is_some_and(|value| value.len() == 32 && value.bytes().all(lower_hex))
}

fn lower_hex(value: u8) -> bool {
    value.is_ascii_digit() || (b'a'..=b'f').contains(&value)
}
fn valid_session(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096
}
fn valid_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value == b'_')
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "grok_native_bridge.protocol_or_identity",
    )
}

#[cfg(test)]
#[path = "grok_native_bridge_tests.rs"]
mod tests;
