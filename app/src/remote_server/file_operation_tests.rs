use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use ai::project_context::model::ProjectContextModel;
use prost::Message;
use remote_server::proto::{
    ClientMessage, DeleteFile, DeleteFileForDiff, ErrorCode, HostScopedRequest, ListDirectory,
    RenameFile, RestoreFileForDiff, ServerMessage, UndoMoveFileForDiff, client_message,
    host_scoped_request, server_message,
};
use remote_server::protocol::RequestId;
use warp_files::FileModel;
use warpui::{App, ModelHandle};

use super::ServerModel;
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn add_server(
    app: &mut App,
) -> (
    ModelHandle<ServerModel>,
    async_channel::Receiver<ServerMessage>,
) {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(|_| ProjectContextModel::default());
    app.add_singleton_model(GlobalBufferModel::new);
    let server = app.add_model(ServerModel::new);
    let conn_id = uuid::Uuid::new_v4();
    let (tx, rx) = async_channel::unbounded();
    server.update(app, |server, ctx| {
        server.register_connection(conn_id, tx, ctx)
    });
    (server, rx)
}

#[test]
#[cfg(unix)]
fn remote_delete_removes_link_without_touching_referent() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("referent.txt");
        let link = dir.path().join("link.txt");
        fs::write(&referent, "original\n").unwrap();
        std::os::unix::fs::symlink(&referent, &link).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::DeleteFile(DeleteFile {
                path: link.to_str().unwrap().to_owned(),
            }),
        );
        let response = responses.recv().await.unwrap();
        remote_server::host_response::delete_file_result(&response).unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(fs::read_to_string(&referent).unwrap(), "original\n");
    });
}

#[test]
#[cfg(unix)]
fn remote_move_moves_link_entry_not_referent() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("referent.txt");
        let link = dir.path().join("link.txt");
        let destination = dir.path().join("moved-link.txt");
        fs::write(&referent, "original\n").unwrap();
        std::os::unix::fs::symlink(&referent, &link).unwrap();

        move_file(&mut app, &server, &link, destination.clone());
        remote_server::host_response::rename_file_result(&responses.recv().await.unwrap()).unwrap();

        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(fs::read_link(&destination).unwrap(), referent);
        assert_eq!(fs::read_to_string(&referent).unwrap(), "updated\n");
    });
}

#[test]
fn remote_hard_link_collision_fails_before_writing() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("alias.txt");
        fs::write(&source, "original\n").unwrap();
        fs::hard_link(&source, &destination).unwrap();

        move_file(&mut app, &server, &source, destination.clone());
        assert!(
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .is_err()
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original\n");
    });
}

fn dispatch(
    app: &mut App,
    server: &ModelHandle<ServerModel>,
    request: host_scoped_request::Message,
) {
    let message = ClientMessage::host_scoped(RequestId::new().to_string(), request);
    let message = ClientMessage::decode(message.encode_to_vec().as_slice()).unwrap();
    server.update(app, |server, ctx| {
        let conn_id = *server.connection_senders.keys().next().unwrap();
        server.handle_message(conn_id, message, ctx);
    });
}

fn move_file(app: &mut App, server: &ModelHandle<ServerModel>, from: &Path, to: PathBuf) {
    dispatch(
        app,
        server,
        host_scoped_request::Message::RenameFile(RenameFile {
            path: from.to_str().unwrap().to_owned(),
            new_path: to.to_str().unwrap().to_owned(),
            content: "updated\n".to_owned(),
            require_restorable: false,
            expected_target_content: None,
        }),
    );
}

#[test]
#[cfg(unix)]
fn remote_move_replaces_destination_and_preserves_source_permissions() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.sh");
        let target = dir.path().join("target.sh");
        fs::write(&source, "original\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(&target, "replace me\n").unwrap();
        move_file(&mut app, &server, &source, target.clone());
        remote_server::host_response::rename_file_result(&responses.recv().await.unwrap()).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
    });
}

#[test]
fn remote_move_destination_setup_failure_leaves_source_unchanged() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let blocker = dir.path().join("not-a-directory");
        fs::write(&source, "original\n").unwrap();
        fs::write(&blocker, "blocker").unwrap();
        move_file(&mut app, &server, &source, blocker.join("target.txt"));
        let result =
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap());

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&blocker).unwrap(), "blocker");
    });
}

#[test]
fn remote_move_native_rename_failure_reports_saved_source() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("directory");
        fs::write(&source, "original\n").unwrap();
        fs::create_dir(&target).unwrap();
        move_file(&mut app, &server, &source, target.clone());
        let error =
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .unwrap_err();

        assert!(matches!(
            error,
            remote_server::manager::HostRequestError::RenameFailedAfterSave(_)
        ));
        assert!(error.to_string().contains("Saved content at"));
        assert_eq!(fs::read_to_string(&source).unwrap(), "updated\n");
        assert!(target.is_dir());
    });
}

#[test]
fn remote_same_path_move_writes_without_deleting_source() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        move_file(&mut app, &server, &source, source.clone());
        remote_server::host_response::rename_file_result(&responses.recv().await.unwrap()).unwrap();

        assert_eq!(fs::read_to_string(&source).unwrap(), "updated\n");
    });
}

#[test]
fn remote_delete_removes_initially_empty_file() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("empty.txt");
        fs::write(&source, "").unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::DeleteFile(DeleteFile {
                path: source.to_str().unwrap().to_owned(),
            }),
        );
        remote_server::host_response::delete_file_result(&responses.recv().await.unwrap()).unwrap();

        assert!(!source.exists());
    });
}

#[test]
fn remote_normalized_same_path_move_writes_without_deleting_source() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        fs::write(&source, "original\n").unwrap();
        move_file(&mut app, &server, &source, dir.path().join("./source.txt"));
        remote_server::host_response::rename_file_result(&responses.recv().await.unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "updated\n");
    });
}

#[test]
fn old_daemon_decoding_rename_as_unknown_returns_error_without_writing() {
    // 旧协议的 22 号是目录读取；新移动不能被解码为该操作。
    #[derive(Clone, PartialEq, Message)]
    struct LegacyHostRequest {
        #[prost(message, optional, tag = "22")]
        list_directory: Option<ListDirectory>,
    }
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        let request = HostScopedRequest {
            message: Some(host_scoped_request::Message::RenameFile(RenameFile {
                path: source.to_str().unwrap().to_owned(),
                new_path: target.to_str().unwrap().to_owned(),
                content: "updated\n".to_owned(),
                require_restorable: false,
                expected_target_content: None,
            })),
        };
        let legacy = LegacyHostRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert!(legacy.list_directory.is_none());
        // 模拟旧 daemon 丢弃未知字段后进入现有的空消息拒绝分支。
        let decoded = HostScopedRequest::decode(legacy.encode_to_vec().as_slice()).unwrap();
        let message = ClientMessage {
            request_id: RequestId::new().to_string(),
            message: Some(client_message::Message::HostScoped(decoded)),
        };
        server.update(&mut app, |server, ctx| {
            let conn_id = *server.connection_senders.keys().next().unwrap();
            server.handle_message(conn_id, message, ctx);
        });
        let response = responses.recv().await.unwrap();
        let Some(server_message::Message::Error(error)) = &response.message else {
            panic!("expected unsupported request error");
        };
        assert_eq!(error.code, i32::from(ErrorCode::InvalidRequest));
        assert!(remote_server::host_response::rename_file_result(&response).is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(!target.exists());
    });
}

#[test]
#[cfg(unix)]
fn remote_protected_delete_restores_private_permissions_without_overwriting_new_entries() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("private.txt");
        let referent = dir.path().join("referent.txt");
        fs::write(&source, "private\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::DeleteFileForDiff(DeleteFileForDiff {
                path: source.to_str().unwrap().to_owned(),
            }),
        );
        let permissions = remote_server::host_response::delete_file_for_diff_result(
            &responses.recv().await.unwrap(),
        )
        .unwrap();
        assert_eq!(permissions.unix_mode, Some(0o600));
        assert!(!source.exists());
        fs::write(&referent, "untouched\n").unwrap();
        std::os::unix::fs::symlink(&referent, &source).unwrap();
        let restore = RestoreFileForDiff {
            path: source.to_str().unwrap().to_owned(),
            content: "private\n".to_owned(),
            permissions: Some(permissions.into()),
        };
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RestoreFileForDiff(restore.clone()),
        );
        assert!(
            remote_server::host_response::restore_file_for_diff_result(
                &responses.recv().await.unwrap()
            )
            .is_err()
        );
        assert_eq!(fs::read_link(&source).unwrap(), referent);
        assert_eq!(fs::read_to_string(&referent).unwrap(), "untouched\n");
        fs::remove_file(&source).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RestoreFileForDiff(restore),
        );
        remote_server::host_response::restore_file_for_diff_result(
            &responses.recv().await.unwrap(),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "private\n");
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o600
        );
    });
}

#[test]
#[cfg(unix)]
fn remote_protected_delete_and_move_reject_linked_entries_before_writing() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let alias = dir.path().join("alias.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        fs::hard_link(&source, &alias).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::DeleteFileForDiff(DeleteFileForDiff {
                path: source.to_str().unwrap().to_owned(),
            }),
        );
        assert!(
            remote_server::host_response::delete_file_for_diff_result(
                &responses.recv().await.unwrap()
            )
            .is_err()
        );
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RenameFile(RenameFile {
                path: source.to_str().unwrap().to_owned(),
                new_path: target.to_str().unwrap().to_owned(),
                content: "updated\n".to_owned(),
                require_restorable: true,
                expected_target_content: None,
            }),
        );
        assert!(
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .is_err()
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&alias).unwrap(), "original\n");
        assert!(!target.exists());
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::DeleteFileForDiff(DeleteFileForDiff {
                path: alias.to_str().unwrap().to_owned(),
            }),
        );
        assert!(
            remote_server::host_response::delete_file_for_diff_result(
                &responses.recv().await.unwrap()
            )
            .is_err()
        );
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RenameFile(RenameFile {
                path: source.to_str().unwrap().to_owned(),
                new_path: alias.to_str().unwrap().to_owned(),
                content: "updated\n".to_owned(),
                require_restorable: true,
                expected_target_content: None,
            }),
        );
        assert!(
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .is_err()
        );
        assert_eq!(fs::read_link(&alias).unwrap(), source);
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
    });
}

#[test]
fn old_daemon_rejects_protected_delete_and_restore_as_unknown_requests() {
    #[derive(Clone, PartialEq, Message)]
    struct LegacyHostRequest {
        #[prost(message, optional, tag = "2")]
        delete_file: Option<DeleteFile>,
    }
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        fs::write(&source, "original\n").unwrap();
        for request in [
            host_scoped_request::Message::UndoMoveFileForDiff(UndoMoveFileForDiff {
                path: source.to_str().unwrap().to_owned(),
                new_path: dir.path().join("moved.txt").to_str().unwrap().to_owned(),
                content: "changed".to_owned(),
            }),
            host_scoped_request::Message::DeleteFileForDiff(DeleteFileForDiff {
                path: source.to_str().unwrap().to_owned(),
            }),
            host_scoped_request::Message::RestoreFileForDiff(RestoreFileForDiff {
                path: source.to_str().unwrap().to_owned(),
                content: "changed".to_owned(),
                permissions: Some(
                    warp_util::file::FilePermissions {
                        unix_mode: Some(0o600),
                        readonly: false,
                    }
                    .into(),
                ),
            }),
        ] {
            let wrapper = HostScopedRequest {
                message: Some(request),
            };
            let legacy = LegacyHostRequest::decode(wrapper.encode_to_vec().as_slice()).unwrap();
            assert!(legacy.delete_file.is_none());
            let decoded = HostScopedRequest::decode(legacy.encode_to_vec().as_slice()).unwrap();
            server.update(&mut app, |server, ctx| {
                let conn_id = *server.connection_senders.keys().next().unwrap();
                server.handle_message(
                    conn_id,
                    ClientMessage {
                        request_id: RequestId::new().to_string(),
                        message: Some(client_message::Message::HostScoped(decoded)),
                    },
                    ctx,
                );
            });
            let response = responses.recv().await.unwrap();
            let Some(server_message::Message::Error(error)) = response.message else {
                panic!("expected unknown request rejection")
            };
            assert_eq!(error.code, i32::from(ErrorCode::InvalidRequest));
            assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        }
    });
}

#[test]
#[cfg(unix)]
fn remote_protected_move_returns_target_permissions_for_complete_undo() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.sh");
        let target = dir.path().join("private.txt");
        fs::write(&source, "source\n").unwrap();
        fs::write(&target, "private\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RenameFile(RenameFile {
                path: source.to_str().unwrap().to_owned(),
                new_path: target.to_str().unwrap().to_owned(),
                content: "updated\n".to_owned(),
                require_restorable: true,
                expected_target_content: Some("private\n".to_owned()),
            }),
        );
        let permissions =
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(permissions.unix_mode, Some(0o600));
        assert!(!source.exists());
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RenameFile(RenameFile {
                path: target.to_str().unwrap().to_owned(),
                new_path: source.to_str().unwrap().to_owned(),
                content: "source\n".to_owned(),
                require_restorable: true,
                expected_target_content: None,
            }),
        );
        assert!(
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .unwrap()
                .is_none()
        );
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::RestoreFileForDiff(RestoreFileForDiff {
                path: target.to_str().unwrap().to_owned(),
                content: "private\n".to_owned(),
                permissions: Some(permissions.into()),
            }),
        );
        remote_server::host_response::restore_file_for_diff_result(
            &responses.recv().await.unwrap(),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "source\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "private\n");
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    });
}

#[test]
fn remote_undo_move_rejects_new_original_file_and_can_retry() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        fs::write(&source, "original\n").unwrap();
        move_file(&mut app, &server, &source, target.clone());
        remote_server::host_response::rename_file_result(&responses.recv().await.unwrap()).unwrap();
        fs::write(&source, "new user file\n").unwrap();
        let undo = UndoMoveFileForDiff {
            path: target.to_str().unwrap().to_owned(),
            new_path: source.to_str().unwrap().to_owned(),
            content: "original\n".to_owned(),
        };
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::UndoMoveFileForDiff(undo.clone()),
        );
        assert!(
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .is_err()
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "new user file\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
        fs::rename(&source, dir.path().join("user-file.txt")).unwrap();
        dispatch(
            &mut app,
            &server,
            host_scoped_request::Message::UndoMoveFileForDiff(undo),
        );
        remote_server::host_response::rename_file_result(&responses.recv().await.unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(!target.exists());
    });
}

#[test]
fn remote_move_snapshot_guard_rejects_deleted_created_or_modified_targets() {
    App::test((), |mut app| async move {
        let (server, responses) = add_server(&mut app);
        let dir = tempfile::tempdir().unwrap();
        for (index, (expected, actual)) in [
            (Some("old target\n"), None),
            (None, Some("new target\n")),
            (Some("old target\n"), Some("changed target\n")),
        ]
        .into_iter()
        .enumerate()
        {
            let source = dir.path().join(format!("source-{index}.txt"));
            let target = dir.path().join(format!("target-{index}.txt"));
            fs::write(&source, "original\n").unwrap();
            if let Some(actual) = actual {
                fs::write(&target, actual).unwrap();
            }
            let request = RenameFile {
                path: source.to_str().unwrap().to_owned(),
                new_path: target.to_str().unwrap().to_owned(),
                content: "updated\n".to_owned(),
                require_restorable: true,
                expected_target_content: expected.map(str::to_owned),
            };
            dispatch(
                &mut app,
                &server,
                host_scoped_request::Message::RenameFile(request.clone()),
            );
            assert!(
                remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                    .is_err()
            );
            assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
            assert_eq!(fs::read_to_string(&target).ok().as_deref(), actual);
            if let Some(expected) = expected {
                fs::write(&target, expected).unwrap();
            } else {
                fs::remove_file(&target).unwrap();
            }
            dispatch(
                &mut app,
                &server,
                host_scoped_request::Message::RenameFile(request),
            );
            remote_server::host_response::rename_file_result(&responses.recv().await.unwrap())
                .unwrap();
            assert_eq!(fs::read_to_string(&target).unwrap(), "updated\n");
            assert!(!source.exists());
        }
    });
}
