//! Entity submodel that encapsulates all filesystem access for diff application.
//!
//! The executor holds a [`ModelHandle<ApplyDiffModel>`] and calls
//! [`ApplyDiffModel::apply_diffs`] without knowing whether the session is local
//! or remote. Internally the method resolves the session context and remote
//! client from the model context, then dispatches:
//!
//! - **Local**: calls [`apply_edits`] with a `std::fs`-backed closure.
//! - **Remote**: calls [`apply_edits`] with a [`RemoteServerClient`]-backed closure.

use std::future::Future;

use ai::diff_validation::AIRequestedCodeDiff;
use futures::FutureExt;
use remote_server::host_response::READ_FILE_NOT_FOUND_PREFIX;
use remote_server::manager::{HostRequestError, HostRequestHandle};
use remote_server::proto::{ReadFileChunkResponse, read_file_chunk_response};
use vec1::Vec1;
use warpui::r#async::BoxFuture;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity as _};

use super::diff_application::{DiffApplicationError, FileReadResult, apply_edits};
use crate::ai::agent::{AIIdentifiers, FileEdit};
use crate::ai::blocklist::SessionContext;
use crate::auth::AuthStateProvider;
use crate::terminal::model::session::active_session::ActiveSession;

/// Entity submodel that encapsulates filesystem access for diff application.
///
/// Held as a [`ModelHandle`] by the [`super::RequestFileEditsExecutor`].
pub(crate) struct ApplyDiffModel {
    active_session: ModelHandle<ActiveSession>,
}

impl Entity for ApplyDiffModel {
    type Event = ();
}

impl ApplyDiffModel {
    pub fn new(active_session: ModelHandle<ActiveSession>) -> Self {
        Self { active_session }
    }

    /// Resolves session context and remote client from the model context, then
    /// returns a future that applies the edits locally or remotely.
    pub fn apply_diffs(
        &self,
        edits: Vec<FileEdit>,
        ai_identifiers: &AIIdentifiers,
        passive_diff: bool,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, Result<Vec<AIRequestedCodeDiff>, Vec1<DiffApplicationError>>> {
        let session_context = SessionContext::from_session(self.active_session.as_ref(ctx), ctx);
        let background_executor = ctx.background_executor();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let ai_identifiers = ai_identifiers.clone();

        let host_request_handle = session_context.host_id().map(|host_id| {
            remote_server::manager::RemoteServerManager::as_ref(ctx).host_request_handle(host_id)
        });

        let is_remote = session_context.is_remote();
        let fut = async move {
            if is_remote {
                match host_request_handle {
                    Some(handle) => {
                        apply_edits(
                            edits,
                            &session_context,
                            &ai_identifiers,
                            background_executor,
                            auth_state,
                            passive_diff,
                            |path| {
                                let handle = &handle;
                                async move { read_remote_file(handle, &path).await }
                            },
                        )
                        .await
                    }
                    None => Err(vec1::vec1![
                        DiffApplicationError::RemoteFileOperationsUnsupported
                    ]),
                }
            } else {
                apply_edits(
                    edits,
                    &session_context,
                    &ai_identifiers,
                    background_executor,
                    auth_state,
                    passive_diff,
                    |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
                )
                .await
            }
        };
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                fut.boxed_local()
            } else {
                fut.boxed()
            }
        }
    }
}

/// 远端差异的单文件预算与 daemon 单块读取上限。
const MAX_DIFF_READ_BYTES: u64 = 10_000_000;
const DIFF_READ_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

async fn read_remote_file(handle: &HostRequestHandle, path: &str) -> FileReadResult {
    read_remote_file_chunks(|offset, max_bytes| {
        handle.read_file_chunk(path.to_owned(), offset, max_bytes)
    })
    .await
}

/// 完整拼接后再解码 UTF-8；分块不能改变换行、拆坏字符或静默接受截断。
async fn read_remote_file_chunks<F, Fut>(mut read_chunk: F) -> FileReadResult
where
    F: FnMut(u64, u64) -> Fut,
    Fut: Future<Output = Result<ReadFileChunkResponse, HostRequestError>>,
{
    let mut bytes = Vec::new();
    let mut offset = 0;
    let mut total_size = None;
    // 最多两批内容；恰好达到预算时额外读取一字节，验证真正的 EOF。
    for batch in 0..3 {
        let remaining = MAX_DIFF_READ_BYTES - offset;
        if batch == 2 && remaining != 0 {
            return FileReadResult::ReadError(
                "Remote file did not complete within the chunk budget".to_owned(),
            );
        }
        let max_bytes = if remaining == 0 {
            1
        } else {
            remaining.min(DIFF_READ_CHUNK_BYTES)
        };
        let response = match read_chunk(offset, max_bytes).await {
            Ok(response) => response,
            Err(error) => return FileReadResult::ReadError(error.to_string()),
        };
        let chunk = match response.result {
            Some(read_file_chunk_response::Result::Success(chunk)) => chunk,
            Some(read_file_chunk_response::Result::Error(error)) => {
                return if offset == 0 && error.message.starts_with(READ_FILE_NOT_FOUND_PREFIX) {
                    FileReadResult::NotFound
                } else {
                    FileReadResult::ReadError(error.message)
                };
            }
            None => {
                return FileReadResult::ReadError("Empty remote file chunk response".to_owned());
            }
        };
        let Some(size) = chunk.total_size else {
            return FileReadResult::ReadError(
                "Missing remote file size for diff application".to_owned(),
            );
        };
        if size > MAX_DIFF_READ_BYTES {
            return FileReadResult::ReadError(format!(
                "File exceeds the {MAX_DIFF_READ_BYTES}-byte limit for remote diff application"
            ));
        }
        if total_size.is_some_and(|previous| previous != size) {
            return FileReadResult::ReadError(
                "Remote file size changed during diff preparation".to_owned(),
            );
        }
        total_size = Some(size);
        let read = chunk.bytes.len() as u64;
        if read > max_bytes
            || chunk.next_offset != offset + read
            || chunk.next_offset > size
            || chunk.next_offset > MAX_DIFF_READ_BYTES
        {
            return FileReadResult::ReadError(
                "Invalid remote file chunk offset or length".to_owned(),
            );
        }
        if chunk.eof && chunk.next_offset != size {
            return FileReadResult::ReadError(
                "Remote file was truncated during diff preparation".to_owned(),
            );
        }
        if !chunk.eof && read == 0 {
            return FileReadResult::ReadError("Remote file chunk made no progress".to_owned());
        }
        bytes.extend(chunk.bytes);
        offset = chunk.next_offset;
        if chunk.eof && (offset < MAX_DIFF_READ_BYTES || remaining == 0) {
            return match String::from_utf8(bytes) {
                Ok(content) => FileReadResult::Found(content),
                Err(error) => {
                    FileReadResult::ReadError(format!("Remote file is not valid UTF-8: {error}"))
                }
            };
        }
    }
    FileReadResult::ReadError("Remote file did not reach EOF within the read budget".to_owned())
}

#[cfg(all(test, feature = "local_fs", not(target_family = "wasm")))]
#[path = "apply_diff_model_tests.rs"]
mod tests;
