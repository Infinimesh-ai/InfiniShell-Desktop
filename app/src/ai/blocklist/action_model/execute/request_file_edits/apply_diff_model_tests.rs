use std::collections::VecDeque;
use std::fs;
use std::sync::Arc;

use ai::diff_validation::ParsedDiff;
use ai::project_context::model::ProjectContextModel;
use prost::Message;
use remote_server::proto::{
    ClientMessage, FileOperationError, ReadFileChunk, ReadFileChunkSuccess, RenameFile,
    RestoreFileForDiff, ServerMessage, UndoMoveFileForDiff, host_scoped_request, server_message,
};
use remote_server::protocol::RequestId;
use warp_files::FileModel;
use warpui::{App, ModelHandle, ModelSpawner};

use super::*;
use crate::auth::AuthState;
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::remote_server::server_model::ServerModel;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[derive(Clone)]
struct Daemon {
    spawner: ModelSpawner<ServerModel>,
    connection_id: uuid::Uuid,
    responses: async_channel::Receiver<ServerMessage>,
}

impl Daemon {
    fn new(app: &mut App) -> (Self, ModelHandle<ServerModel>) {
        initialize_app_for_terminal_view(app);
        app.add_singleton_model(FileModel::new);
        app.add_singleton_model(|_| ProjectContextModel::default());
        app.add_singleton_model(GlobalBufferModel::new);
        let server = app.add_model(ServerModel::new);
        let connection_id = uuid::Uuid::new_v4();
        let (sender, responses) = async_channel::unbounded();
        let spawner = server.update(app, |server, ctx| {
            server.register_connection(connection_id, sender, ctx);
            ctx.spawner()
        });
        (
            Self {
                spawner,
                connection_id,
                responses,
            },
            server,
        )
    }

    async fn request(&self, request: host_scoped_request::Message) -> ServerMessage {
        let request_id = RequestId::new().to_string();
        let message = ClientMessage::host_scoped(request_id.clone(), request);
        let message = ClientMessage::decode(message.encode_to_vec().as_slice()).unwrap();
        let connection_id = self.connection_id;
        self.spawner
            .spawn(move |server, ctx| server.handle_message(connection_id, message, ctx))
            .await
            .unwrap();
        let response = self.responses.recv().await.unwrap();
        assert_eq!(response.request_id, request_id);
        ServerMessage::decode(response.encode_to_vec().as_slice()).unwrap()
    }

    async fn read_file(&self, path: &str) -> FileReadResult {
        read_remote_file_chunks(|offset, max_bytes| async move {
            let response = self
                .request(host_scoped_request::Message::ReadFileChunk(ReadFileChunk {
                    path: path.to_owned(),
                    offset,
                    max_bytes,
                }))
                .await;
            match response.message {
                Some(server_message::Message::ReadFileChunkResponse(response)) => Ok(response),
                _ => Err(HostRequestError::UnexpectedResponse),
            }
        })
        .await
    }
}

#[test]
fn daemon_chunks_preserve_crlf_through_move_preparation_and_undo() {
    App::test((), |mut app| async move {
        let (daemon, _server) = Daemon::new(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        let source_content = "source 中\r\n";
        let target_content = "private 目标\r\nsecond line\r\n";
        fs::write(&source, source_content).unwrap();
        fs::write(&target, target_content).unwrap();
        let reader = daemon.clone();
        let diffs = apply_edits(
            vec![FileEdit::Edit(ParsedDiff::V4AEdit {
                file: Some(source.to_str().unwrap().to_owned()),
                move_to: Some(target.to_str().unwrap().to_owned()),
                hunks: vec![],
            })],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| {
                let reader = reader.clone();
                async move { reader.read_file(&path).await }
            },
        )
        .await
        .unwrap();
        assert_eq!(diffs.len(), 1);
        let diff = diffs.into_iter().next().unwrap();
        assert_eq!(diff.original_content, source_content);
        assert_eq!(
            diff.rename_target_original_content.as_deref(),
            Some(target_content)
        );
        let response = daemon
            .request(host_scoped_request::Message::RenameFile(RenameFile {
                path: source.to_str().unwrap().to_owned(),
                new_path: target.to_str().unwrap().to_owned(),
                content: diff.original_content.clone(),
                require_restorable: true,
                expected_target_content: diff.rename_target_original_content.clone(),
            }))
            .await;
        let permissions = remote_server::host_response::rename_file_result(&response)
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), source_content.as_bytes());
        let response = daemon
            .request(host_scoped_request::Message::UndoMoveFileForDiff(
                UndoMoveFileForDiff {
                    path: target.to_str().unwrap().to_owned(),
                    new_path: source.to_str().unwrap().to_owned(),
                    content: diff.original_content,
                },
            ))
            .await;
        remote_server::host_response::rename_file_result(&response).unwrap();
        let response = daemon
            .request(host_scoped_request::Message::RestoreFileForDiff(
                RestoreFileForDiff {
                    path: target.to_str().unwrap().to_owned(),
                    content: diff.rename_target_original_content.unwrap(),
                    permissions: Some(permissions.into()),
                },
            ))
            .await;
        remote_server::host_response::restore_file_for_diff_result(&response).unwrap();
        assert_eq!(fs::read(&source).unwrap(), source_content.as_bytes());
        assert_eq!(fs::read(&target).unwrap(), target_content.as_bytes());
    });
}

#[test]
fn daemon_chunk_reader_distinguishes_missing_files_from_other_read_failures() {
    App::test((), |mut app| async move {
        let (daemon, _server) = Daemon::new(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.txt");
        assert!(matches!(
            daemon.read_file(missing.to_str().unwrap()).await,
            FileReadResult::NotFound
        ));
        assert!(matches!(
            daemon.read_file(dir.path().to_str().unwrap()).await,
            FileReadResult::ReadError(_)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let unreadable = dir.path().join("not found in name.txt");
            fs::write(&unreadable, "private").unwrap();
            fs::set_permissions(&unreadable, fs::Permissions::from_mode(0)).unwrap();
            let result = daemon.read_file(unreadable.to_str().unwrap()).await;
            fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
            // root 可越过权限位，普通测试账户必须收到读失败而不是缺失文件。
            if unsafe { libc::geteuid() } != 0 {
                assert!(matches!(result, FileReadResult::ReadError(_)));
            }
        }
    });
}

fn chunk(bytes: &[u8], next_offset: u64, total_size: u64, eof: bool) -> ReadFileChunkResponse {
    ReadFileChunkResponse {
        result: Some(read_file_chunk_response::Result::Success(
            ReadFileChunkSuccess {
                bytes: bytes.to_vec(),
                next_offset,
                total_size: Some(total_size),
                eof,
            },
        )),
    }
}

async fn read_fixture(chunks: Vec<ReadFileChunkResponse>) -> FileReadResult {
    let mut chunks = VecDeque::from(chunks);
    read_remote_file_chunks(|_, _| futures::future::ready(Ok(chunks.pop_front().unwrap()))).await
}

#[test]
fn remote_chunk_reader_decodes_utf8_only_after_joining_chunks() {
    App::test((), |_| async move {
        let result = read_fixture(vec![
            chunk(&[0xe4, 0xb8], 2, 5, false),
            chunk(&[0xad, b'\r', b'\n'], 5, 5, true),
        ])
        .await;
        let FileReadResult::Found(content) = result else {
            panic!("expected complete UTF-8 content");
        };
        assert_eq!(content, "中\r\n");
    });
}

#[test]
fn remote_chunk_reader_rejects_inconsistent_progress_truncation_and_oversize() {
    App::test((), |_| async move {
        for responses in [
            vec![chunk(b"a", 2, 2, true)],
            vec![chunk(b"", 0, 2, false)],
            vec![chunk(b"a", 1, 2, true)],
            vec![chunk(b"a", 1, MAX_DIFF_READ_BYTES + 1, false)],
            vec![chunk(b"a", 1, 2, false), chunk(b"b", 2, 3, false)],
        ] {
            assert!(matches!(
                read_fixture(responses).await,
                FileReadResult::ReadError(_)
            ));
        }
    });
}

#[test]
fn remote_chunk_reader_does_not_guess_not_found_from_legacy_or_permission_errors() {
    App::test((), |_| async move {
        for message in [
            "Failed to read file chunk /tmp/source: No such file or directory (os error 2)",
            "Failed to read file chunk /tmp/not found: Permission denied (os error 13)",
        ] {
            let response = ReadFileChunkResponse {
                result: Some(read_file_chunk_response::Result::Error(
                    FileOperationError {
                        message: message.to_owned(),
                    },
                )),
            };
            assert!(matches!(
                read_fixture(vec![response]).await,
                FileReadResult::ReadError(_)
            ));
        }
        assert!(matches!(
            read_remote_file_chunks(|_, _| futures::future::ready(Err(
                HostRequestError::UnexpectedResponse
            )))
            .await,
            FileReadResult::ReadError(_)
        ));
    });
}
