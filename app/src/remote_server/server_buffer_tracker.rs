use std::collections::{HashMap, HashSet};

use bimap::BiMap;
use warp_editor::content::buffer::Buffer;
use warp_util::file::FileId;
use warpui::{ModelContext, ModelHandle, SingletonEntity as _};

use super::server_model::{ConnectionId, ServerModel};
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::remote_server::protocol::RequestId;

/// Distinguishes the type of pending buffer request so the event
/// subscription can send the correct response message.
#[derive(Clone, Copy, Debug)]
pub enum PendingBufferRequestKind {
    OpenBuffer,
    SaveBuffer,
    ResolveConflict,
}

/// An in-flight buffer request awaiting a `GlobalBufferModelEvent` to
/// correlate it back to the originating connection.
#[derive(Clone, Debug)]
pub struct PendingBufferRequest {
    pub request_id: RequestId,
    pub connection_id: ConnectionId,
    pub kind: PendingBufferRequestKind,
}

/// Bridges the ServerModel's per-connection state with the GlobalBufferModel's
/// tracked buffers. Manages:
/// - Wire path → FileId mappings for open server-local buffers
/// - Per-buffer connection sets (which connections have each buffer open)
/// - Pending async requests (OpenBuffer, SaveBuffer, ResolveConflict) awaiting events
pub struct ServerBufferTracker {
    /// 双向映射 wire path ↔ FileId,两侧查找都 O(1)。
    /// `path_for_file_id` 在每次 `ServerLocalBufferUpdated` 都会被调用,
    /// 用 BiMap 避免线性扫描。
    open_buffers: BiMap<String, FileId>,
    /// 持有每个已打开 server-local buffer 的**强引用** `ModelHandle<Buffer>`。
    ///
    /// `GlobalBufferModel` 内部只存 `WeakModelHandle`,客户端靠编辑器 view 持有
    /// 强引用让 buffer 存活;但 daemon 没有 view —— 若不在这里持有强引用,
    /// `handle_open_buffer` 返回后 buffer 引用计数归零,会被 WarpUI 的
    /// `flush_effects` 回收,导致随后 `FileModel` 异步加载完成时 weak handle 已
    /// 失效(日志「Cannot populate buffer with content due to deallocated model
    /// handle」)。buffer 关闭(无连接)时一并 drop。
    buffer_handles: HashMap<FileId, ModelHandle<Buffer>>,
    /// Tracks which connections have each buffer open.
    /// File-watcher pushes go to all connections in the set.
    buffer_connections: HashMap<FileId, HashSet<ConnectionId>>,
    /// Tracks in-flight OpenBuffer / SaveBuffer / ResolveConflict requests so
    /// `GlobalBufferModelEvent`s can be correlated back to the originating
    /// request and connection. Uses a `Vec` to support concurrent requests
    /// for the same buffer from different connections.
    pending_requests: HashMap<FileId, Vec<PendingBufferRequest>>,
}

impl ServerBufferTracker {
    pub fn new() -> Self {
        Self {
            open_buffers: BiMap::new(),
            buffer_handles: HashMap::new(),
            buffer_connections: HashMap::new(),
            pending_requests: HashMap::new(),
        }
    }

    // ── Path ↔ FileId mapping ─────────────────────────────────────

    /// Register a wire path → FileId mapping,并持有 buffer 的强引用让它在
    /// daemon 端存活(见 `buffer_handles` 字段说明)。
    pub fn track_open_buffer(
        &mut self,
        path: String,
        file_id: FileId,
        buffer: ModelHandle<Buffer>,
    ) {
        self.open_buffers.insert(path, file_id);
        self.buffer_handles.insert(file_id, buffer);
    }

    /// Look up a FileId by its wire path. O(1)。
    pub fn file_id_for_path(&self, path: &str) -> Option<FileId> {
        self.open_buffers.get_by_left(path).copied()
    }

    /// Look up the wire path for a given FileId. O(1) via BiMap。
    /// 返回 owned `String` 而非 `&str`,让调用方在持有结果的同时还能借用
    /// 其它 `&mut self`(典型场景:在事件 handler 里拿到 path 然后回头
    /// `send_server_message(...)`)。push 频率不高,clone 开销可忽略。
    pub fn path_for_file_id(&self, file_id: FileId) -> Option<String> {
        self.open_buffers.get_by_right(&file_id).cloned()
    }

    // ── Connection tracking ───────────────────────────────────────

    /// Add a connection to a buffer's subscriber set.
    pub fn add_connection(&mut self, file_id: FileId, conn_id: ConnectionId) {
        self.buffer_connections
            .entry(file_id)
            .or_default()
            .insert(conn_id);
    }

    /// Returns the set of connections subscribed to a buffer.
    pub fn connections_for_buffer(&self, file_id: &FileId) -> Option<&HashSet<ConnectionId>> {
        self.buffer_connections.get(file_id)
    }

    /// Remove a connection from all buffer subscription sets.
    /// Returns the list of FileIds that have no remaining connections
    /// (orphaned buffers that should be deallocated).
    pub fn remove_connection(
        &mut self,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<ServerModel>,
    ) -> Vec<FileId> {
        // 丢弃该连接产生的所有 pending 请求,避免断连后留下陈旧条目。
        for entries in self.pending_requests.values_mut() {
            entries.retain(|req| req.connection_id != conn_id);
        }
        self.pending_requests
            .retain(|_, entries| !entries.is_empty());

        let orphaned: Vec<FileId> = self
            .buffer_connections
            .iter_mut()
            .filter_map(|(file_id, conns)| {
                conns.remove(&conn_id);
                if conns.is_empty() {
                    Some(*file_id)
                } else {
                    None
                }
            })
            .collect();

        for &file_id in &orphaned {
            self.buffer_connections.remove(&file_id);
            self.open_buffers.remove_by_right(&file_id);
            self.pending_requests.remove(&file_id);
            // 释放强引用,允许 buffer 被回收。
            self.buffer_handles.remove(&file_id);
            GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| gbm.remove(file_id, ctx));
        }

        orphaned
    }

    /// Remove a single connection from a buffer's subscriber set.
    /// If no connections remain, deallocates the buffer entirely.
    pub fn close_buffer(
        &mut self,
        path: &str,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<ServerModel>,
    ) {
        let Some(&file_id) = self.open_buffers.get_by_left(path) else {
            return;
        };

        if let Some(conns) = self.buffer_connections.get_mut(&file_id) {
            conns.remove(&conn_id);
            if !conns.is_empty() {
                return; // Other connections still using this buffer.
            }
        }

        // No connections remain — deallocate.
        self.buffer_connections.remove(&file_id);
        self.open_buffers.remove_by_left(path);
        self.pending_requests.remove(&file_id);
        // 释放强引用,允许 buffer 被回收。
        self.buffer_handles.remove(&file_id);
        GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| gbm.remove(file_id, ctx));
    }

    // ── Pending request tracking ──────────────────────────────────

    /// Stash a pending async request for later correlation with an event.
    pub fn insert_pending(
        &mut self,
        file_id: FileId,
        request_id: RequestId,
        conn_id: ConnectionId,
        kind: PendingBufferRequestKind,
    ) {
        self.pending_requests
            .entry(file_id)
            .or_default()
            .push(PendingBufferRequest {
                request_id,
                connection_id: conn_id,
                kind,
            });
    }

    /// Returns the connection IDs that have pending `OpenBuffer` requests
    /// for the given FileId, without consuming them. Used by the
    /// `ServerLocalBufferUpdated` handler to exclude connections that will
    /// receive content via `OpenBufferResponse` instead of the broadcast push.
    pub fn pending_connections_for_open_buffer(&self, file_id: &FileId) -> HashSet<ConnectionId> {
        self.pending_requests
            .get(file_id)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|req| matches!(req.kind, PendingBufferRequestKind::OpenBuffer))
                    .map(|req| req.connection_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Retrieve and remove pending requests that match `kind` for the given
    /// FileId. Other pending requests for the same FileId are left in place.
    pub fn take_pending_by_kind(
        &mut self,
        file_id: &FileId,
        kind: PendingBufferRequestKind,
    ) -> Vec<PendingBufferRequest> {
        let Some(entries) = self.pending_requests.get_mut(file_id) else {
            return Vec::new();
        };
        let mut matched = Vec::new();
        entries.retain(|req| {
            if std::mem::discriminant(&req.kind) == std::mem::discriminant(&kind) {
                matched.push(req.clone());
                false // remove from the vec
            } else {
                true // keep
            }
        });
        if entries.is_empty() {
            self.pending_requests.remove(file_id);
        }
        matched
    }
}
