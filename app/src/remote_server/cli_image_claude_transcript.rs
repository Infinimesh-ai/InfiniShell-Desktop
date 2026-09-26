//! 只读固定 Claude 会话历史，确认当前请求派生的 Read 已产生真实内联图片。
//! 恢复时重放历史解析，不重放输入；路径被替换或历史压缩时保留 Unknown。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    ClaudeImageAttempt, ClaudeImageReadReceipt, ClaudeImageReceipt, ClaudeInboxTarget,
    DigestWriter, MAX_TOTAL_IMAGE_BYTES, identity, is_absolute_normal_path, open_private_file,
    rejected, verify_file_path,
};

const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;
const MAX_POLL_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCANNED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RECORDS: usize = 16 * 1024;

struct NativeRead {
    tool_use_id: String,
    image_index: usize,
}

struct NativeResult {
    tool_use_id: String,
    image_indexes: Vec<usize>,
}

struct NativeRecord {
    uuid: Uuid,
    parent: Option<Uuid>,
    source_assistant: Option<Uuid>,
    request_root: bool,
    raw_sha256: [u8; 32],
    reads: Vec<NativeRead>,
    results: Vec<NativeResult>,
}

pub(crate) struct ClaudeTranscript {
    file: File,
    attempt: ClaudeImageAttempt,
    offset: u64,
    pending: Vec<u8>,
    records: HashMap<Uuid, NativeRecord>,
}

impl ClaudeTranscript {
    /// 从已持久化的同一次派发恢复查询，不读取密钥、不发送消息、不新建 CLI。
    /// target 的 transcript/session 必须先由服务端恢复记录与原生来源精确绑定。
    pub(crate) fn recover(
        target: &ClaudeInboxTarget,
        attempt: &ClaudeImageAttempt,
    ) -> io::Result<Self> {
        if target.session_id != attempt.session_id
            || target.transcript_path != attempt.transcript_path
            || attempt.client_message_id.is_nil()
            || attempt.session_id.is_nil()
            || !is_absolute_normal_path(&attempt.transcript_path)
            || attempt.images.is_empty()
            || attempt.images.len() > 20
            || attempt
                .images
                .iter()
                .try_fold(0_u64, |total, image| total.checked_add(image.byte_len))
                .is_none_or(|total| total > MAX_TOTAL_IMAGE_BYTES)
            || attempt.images.iter().any(|image| {
                !is_absolute_normal_path(&image.path)
                    || !(8..=20 * 1024 * 1024).contains(&image.byte_len)
            })
        {
            return Err(rejected("transcript_attempt_mismatch"));
        }
        let mut file = open_private_file(&attempt.transcript_path)?;
        let metadata = file.metadata()?;
        if identity(&metadata) != attempt.transcript_identity
            || metadata.len() < attempt.transcript_offset
        {
            return Err(rejected("transcript_replaced_or_truncated"));
        }
        file.seek(SeekFrom::Start(attempt.transcript_offset))?;
        Ok(Self {
            file,
            attempt: attempt.clone(),
            offset: attempt.transcript_offset,
            pending: Vec::new(),
            records: HashMap::new(),
        })
    }

    /// 有界读取，None 包括仍在排队、等待审批、工具未读图及原生压缩后原字节不匹配。
    /// 只有 Some 表示全部原图已有精确消费证据；它不证明模型理解正确。
    pub(crate) fn poll(&mut self) -> io::Result<Option<ClaudeImageReceipt>> {
        verify_file_path(&self.file, &self.attempt.transcript_path)?;
        let metadata = self.file.metadata()?;
        if identity(&metadata) != self.attempt.transcript_identity || metadata.len() < self.offset {
            return Err(rejected("transcript_replaced_or_truncated"));
        }
        let remaining = metadata.len() - self.offset;
        let to_read = remaining.min(MAX_POLL_BYTES as u64) as usize;
        if self.offset - self.attempt.transcript_offset + to_read as u64 > MAX_SCANNED_BYTES {
            return Err(rejected("transcript_scan_budget_exhausted"));
        }
        let mut added = vec![0; to_read];
        self.file.seek(SeekFrom::Start(self.offset))?;
        self.file.read_exact(&mut added)?;
        self.pending.extend_from_slice(&added);
        self.offset += to_read as u64;
        let complete_end = self
            .pending
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let complete = self.pending.drain(..complete_end).collect::<Vec<_>>();
        if self.pending.len() > MAX_LINE_BYTES {
            return Err(rejected("transcript_line_too_large"));
        }
        for line in complete.split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_LINE_BYTES {
                return Err(rejected("transcript_line_too_large"));
            }
            let Some(record) = parse_record(line, &self.attempt)? else {
                continue;
            };
            if let Some(previous) = self.records.get(&record.uuid) {
                // 相同字节重投无副作用；同 UUID 的不同记录不能覆盖已见证明。
                if previous.raw_sha256 != record.raw_sha256 {
                    return Err(rejected("transcript_duplicate_uuid_conflict"));
                }
                continue;
            }
            if self.records.len() >= MAX_RECORDS {
                return Err(rejected("transcript_record_budget_exhausted"));
            }
            self.records.insert(record.uuid, record);
        }
        verify_file_path(&self.file, &self.attempt.transcript_path)?;
        self.receipt()
    }

    /// 单次状态 RPC 可重建解析器；读取到调用时已存在的 EOF，不追逐之后的新追加。
    /// 总扫描仍限 256 MiB，避免大行因每次恢复只读首个 8 MiB 而永远无法完成。
    pub(crate) fn poll_available(&mut self) -> io::Result<Option<ClaudeImageReceipt>> {
        let boundary = self.file.metadata()?.len();
        loop {
            if let Some(receipt) = self.poll()? {
                return Ok(Some(receipt));
            }
            if self.offset >= boundary {
                return Ok(None);
            }
        }
    }

    fn receipt(&self) -> io::Result<Option<ClaudeImageReceipt>> {
        let request_id = self.attempt.client_message_id;
        if !self
            .records
            .get(&request_id)
            .is_some_and(|record| record.request_root)
        {
            return Ok(None);
        }
        // 先保存乱序记录，再按原生 parentUuid 连通关系计算；无父节点不猜归属。
        let mut children = HashMap::<Uuid, Vec<Uuid>>::new();
        for record in self.records.values() {
            if let Some(parent) = record.parent {
                children.entry(parent).or_default().push(record.uuid);
            }
        }
        let mut descendants = HashSet::from([request_id]);
        let mut pending = vec![request_id];
        while let Some(parent) = pending.pop() {
            if let Some(nodes) = children.get(&parent) {
                for child in nodes {
                    if descendants.insert(*child) {
                        pending.push(*child);
                    }
                }
            }
        }
        let mut reads = HashMap::new();
        for record in self
            .records
            .values()
            .filter(|record| descendants.contains(&record.uuid))
        {
            for read in &record.reads {
                if reads
                    .insert(read.tool_use_id.as_str(), (record.uuid, read.image_index))
                    .is_some()
                {
                    return Err(rejected("transcript_duplicate_read_id"));
                }
            }
        }
        let mut matched = vec![None; self.attempt.images.len()];
        for record in self
            .records
            .values()
            .filter(|record| descendants.contains(&record.uuid))
        {
            for result in &record.results {
                let Some((assistant_uuid, image_index)) = reads.get(result.tool_use_id.as_str())
                else {
                    continue;
                };
                if record.source_assistant != Some(*assistant_uuid)
                    || !result.image_indexes.contains(image_index)
                {
                    continue;
                }
                // Read 的最终 tool_result 已包含原图；独立 PostToolUse 回调不足以代替它。
                matched[*image_index] = Some(ClaudeImageReadReceipt {
                    tool_use_id: result.tool_use_id.clone(),
                    assistant_message_uuid: *assistant_uuid,
                    result_message_uuid: record.uuid,
                    image_sha256: self.attempt.images[*image_index].sha256,
                    raw_result_sha256: record.raw_sha256,
                });
            }
        }
        Ok(matched
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .map(|reads| ClaudeImageReceipt {
                attempt: self.attempt.clone(),
                reads,
            }))
    }
}

fn parse_record(line: &[u8], attempt: &ClaudeImageAttempt) -> io::Result<Option<NativeRecord>> {
    let value: Value =
        serde_json::from_slice(line).map_err(|_| rejected("invalid_transcript_json"))?;
    if value.get("sessionId").and_then(value_uuid) != Some(attempt.session_id)
        || value.get("isSidechain").and_then(Value::as_bool) != Some(false)
    {
        return Ok(None);
    }
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Ok(None);
    };
    if kind != "assistant" && kind != "user" {
        return Ok(None);
    }
    let Some(uuid) = value.get("uuid").and_then(value_uuid) else {
        return Err(rejected("invalid_transcript_message_uuid"));
    };
    let request_root = uuid == attempt.client_message_id;
    if request_root
        && (kind != "user"
            || value.pointer("/origin/kind").and_then(Value::as_str) != Some("peer")
            || value.pointer("/origin/msg_id").and_then(value_uuid)
                != Some(attempt.client_message_id))
    {
        return Err(rejected("transcript_request_origin_mismatch"));
    }
    let mut record = NativeRecord {
        uuid,
        parent: value.get("parentUuid").and_then(value_uuid),
        source_assistant: value.get("sourceToolAssistantUUID").and_then(value_uuid),
        request_root,
        raw_sha256: Sha256::digest(line).into(),
        reads: Vec::new(),
        results: Vec::new(),
    };
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return Ok(Some(record));
    };
    for block in content {
        if kind == "assistant"
            && block.get("type").and_then(Value::as_str) == Some("tool_use")
            && block.get("name").and_then(Value::as_str) == Some("Read")
        {
            let Some(id) = block.get("id").and_then(native_tool_id) else {
                continue;
            };
            let path = block.pointer("/input/file_path").and_then(Value::as_str);
            if let Some(image_index) = attempt
                .images
                .iter()
                .position(|image| image.path.to_str() == path)
            {
                record.reads.push(NativeRead {
                    tool_use_id: id.to_owned(),
                    image_index,
                });
            }
        }
        if kind == "user"
            && block.get("type").and_then(Value::as_str) == Some("tool_result")
            && block.get("is_error").and_then(Value::as_bool) != Some(true)
        {
            let Some(id) = block.get("tool_use_id").and_then(native_tool_id) else {
                continue;
            };
            let Some(result_content) = block.get("content").and_then(Value::as_array) else {
                continue;
            };
            let mut image_indexes = Vec::new();
            for image in result_content {
                if image.get("type").and_then(Value::as_str) != Some("image")
                    || image.pointer("/source/type").and_then(Value::as_str) != Some("base64")
                    || image.pointer("/source/media_type").and_then(Value::as_str)
                        != Some("image/png")
                {
                    continue;
                }
                let Some(encoded) = image.pointer("/source/data").and_then(Value::as_str) else {
                    continue;
                };
                let mut decoder = base64::read::DecoderReader::new(encoded.as_bytes(), &STANDARD);
                let mut digest = Sha256::new();
                let byte_len = io::copy(
                    &mut (&mut decoder).take(20 * 1024 * 1024 + 1),
                    &mut DigestWriter(&mut digest),
                )
                .map_err(|_| rejected("invalid_transcript_image"))?;
                let sha256: [u8; 32] = digest.finalize().into();
                for (index, expected) in attempt.images.iter().enumerate() {
                    if byte_len == expected.byte_len && sha256 == expected.sha256 {
                        image_indexes.push(index);
                    }
                }
            }
            if !image_indexes.is_empty() {
                record.results.push(NativeResult {
                    tool_use_id: id.to_owned(),
                    image_indexes,
                });
            }
        }
    }
    Ok(Some(record))
}

fn value_uuid(value: &Value) -> Option<Uuid> {
    value
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|uuid| !uuid.is_nil())
}

fn native_tool_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
}
