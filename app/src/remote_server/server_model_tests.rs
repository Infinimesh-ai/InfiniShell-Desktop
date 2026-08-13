use std::collections::{HashMap, HashSet};
#[cfg(feature = "local_fs")]
use std::fs;

use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::super::diff_state_tracker::RemoteDiffStateManager;
use super::super::proto::{
    Authenticate, BundledSkillMetadata, HomeSkillMetadata, Initialize, RemoteAgentContextSnapshot,
    RemoteContextFileProto, RemoteSkillProto, ServerMessage, WriteFileResponse, WriteFileSuccess,
    remote_skill_proto, server_message, write_file_response,
};
#[cfg(feature = "local_fs")]
use super::super::proto::{
    CreateDirectory, ListDirectory, ReadFileChunk, ResolvePath, WriteFileChunk,
    list_directory_response, read_file_chunk_response, resolve_path_response,
    write_file_chunk_response,
};
use super::super::protocol::RequestId;
#[cfg(feature = "local_fs")]
use super::super::server_buffer_tracker::ServerBufferTracker;
use super::{ConnectionId, PendingFileOps, ServerModel};
use crate::code_review::diff_state::DiffMode;
use crate::remote_server::diff_state_tracker::DiffModelKey;

fn test_model(app: &mut App) -> ServerModel {
    ServerModel {
        connection_senders: HashMap::new(),
        snapshot_sent_roots_by_connection: HashMap::new(),
        grace_timer_cancel: None,
        in_progress: HashMap::new(),
        host_id: "test-host-id".to_string(),
        bundled_skills: Vec::new(),
        remote_agent_context_snapshot: RemoteAgentContextSnapshot {
            revision: 1,
            home_dir: "/home/user".to_string(),
            skills: Vec::new(),
            global_rules: Vec::new(),
        },
        remote_agent_context_snapshot_sent: HashSet::new(),
        executors: HashMap::new(),
        pending_file_ops: PendingFileOps::new(),
        auth_token: None,
        #[cfg(feature = "local_fs")]
        buffers: ServerBufferTracker::new(),
        diff_states: app.add_model(|_| RemoteDiffStateManager::new()),
        host_scoped_requests: HashMap::new(),
        git_status_models: HashMap::new(),
        github_repo_models: HashMap::new(),
        git_status_subscribers: HashMap::new(),
        git_status_repo_by_conn: HashMap::new(),
    }
}

/// Zap:`Initialize` 只用 `auth_token` 一个字段(没有账号体系,身份/崩溃
/// 上报字段客户端永不填写),测试统一走这个构造器。
fn test_initialize(auth_token: &str) -> Initialize {
    Initialize {
        auth_token: auth_token.to_string(),
        user_id: String::new(),
        user_email: String::new(),
        crash_reporting_enabled: false,
        codebase_index_limits: None,
    }
}

/// Uses `try_new` instead of `try_from_local` so that Unix-style paths
/// like `/repo` are recognised as absolute on all platforms (including Windows).
fn test_key(repo: &str, mode: DiffMode) -> DiffModelKey {
    DiffModelKey {
        repo_path: StandardizedPath::try_new(repo).unwrap(),
        mode,
    }
}

fn test_bundled_skill_proto(id: &str) -> RemoteSkillProto {
    RemoteSkillProto {
        path: format!(
            "/home/user/.warp/remote-server/bundled_resources/bundled/skills/{id}/SKILL.md"
        ),
        content: format!("# {id}"),
        source: Some(remote_skill_proto::Source::Bundled(BundledSkillMetadata {
            id: id.to_string(),
            requires_mcp: None,
        })),
    }
}

#[test]
fn remote_agent_context_snapshot_broadcasts_replacements_and_initializes_once() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();
        let (tx, rx) = async_channel::unbounded();
        model.connection_senders.insert(conn, tx);

        model.send_remote_agent_context_snapshot_to_connection(conn);
        assert!(matches!(
            rx.try_recv().map(|msg| msg.message),
            Ok(Some(server_message::Message::RemoteAgentContextSnapshot(_)))
        ));
        model.send_remote_agent_context_snapshot_to_connection(conn);
        assert!(rx.try_recv().is_err());

        model.remote_agent_context_snapshot = RemoteAgentContextSnapshot {
            revision: 2,
            home_dir: "/home/user".to_string(),
            skills: vec![
                test_bundled_skill_proto("test-skill"),
                RemoteSkillProto {
                    path: "/home/user/.agents/skills/test/SKILL.md".to_string(),
                    content: "skill content".to_string(),
                    source: Some(remote_skill_proto::Source::Home(HomeSkillMetadata {})),
                },
            ],
            global_rules: vec![RemoteContextFileProto {
                path: "/home/user/.agents/AGENTS.md".to_string(),
                content: "rule content".to_string(),
            }],
        };
        model.broadcast_remote_agent_context_snapshot();

        match rx
            .try_recv()
            .expect("remote Agent Mode context replacement")
            .message
        {
            Some(server_message::Message::RemoteAgentContextSnapshot(snapshot)) => {
                assert_eq!(snapshot.revision, 2);
                assert_eq!(snapshot.skills.len(), 2);
                assert_eq!(snapshot.skills[1].content, "skill content");
                assert_eq!(snapshot.global_rules[0].content, "rule content");
            }
            other => panic!("expected RemoteAgentContextSnapshot, got {other:?}"),
        }

        let late_conn = uuid::Uuid::new_v4();
        let (late_tx, late_rx) = async_channel::unbounded();
        model.connection_senders.insert(late_conn, late_tx);
        model.send_remote_agent_context_snapshot_to_connection(late_conn);
        assert!(matches!(
            late_rx.try_recv().map(|msg| msg.message),
            Ok(Some(server_message::Message::RemoteAgentContextSnapshot(_)))
        ));
        model.send_remote_agent_context_snapshot_to_connection(late_conn);
        assert!(late_rx.try_recv().is_err());
    });
}

#[test]
fn fresh_model_starts_without_auth_token() {
    App::test((), |mut app| async move {
        let model = test_model(&mut app);

        assert_eq!(model.auth_token(), None);
    });
}

#[test]
fn initialize_with_auth_token_stores_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);

        model.apply_initialize_auth(&test_initialize("initial-token"));

        assert_eq!(model.auth_token(), Some("initial-token"));
    });
}

/// Zap:凭据只在非空时写入,空 `Initialize` 不清除已有 daemon 凭据。
#[test]
fn empty_initialize_preserves_existing_auth_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.apply_initialize_auth(&test_initialize("initial-token"));

        model.apply_initialize_auth(&test_initialize(""));

        assert_eq!(model.auth_token(), Some("initial-token"));
    });
}

#[test]
fn authenticate_with_auth_token_replaces_auth_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.apply_initialize_auth(&test_initialize("initial-token"));

        model.handle_authenticate(Authenticate {
            auth_token: "rotated-token".to_string(),
        });

        assert_eq!(model.auth_token(), Some("rotated-token"));
    });
}

/// Zap:空 `Authenticate` 是「无凭据可用」而非「清除凭据」,已有凭据保留。
#[test]
fn empty_authenticate_preserves_existing_auth_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.apply_initialize_auth(&test_initialize("initial-token"));

        model.handle_authenticate(Authenticate {
            auth_token: String::new(),
        });

        assert_eq!(model.auth_token(), Some("initial-token"));
    });
}

// ── Diff state: connection cleanup ──────────────────────────────────

#[test]
fn deregister_connection_cleans_up_diff_state_subscriptions() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();

        // Register the connection.
        let (tx, _rx) = async_channel::unbounded();
        model.connection_senders.insert(conn, tx);

        // Subscribe the connection to diff state via the manager.
        let key = test_key("/repo", DiffMode::Head);
        let key2 = key.clone();
        let key3 = key.clone();
        model.diff_states.update(&mut app, |mgr, _ctx| {
            mgr.subscribe_connection(key, conn);
        });
        let has_sub = model.diff_states.read(&app, |mgr, _ctx| {
            !mgr.subscribed_connections(&key2).is_empty()
        });
        assert!(has_sub);

        // Simulate deregister_connection's diff state cleanup.
        model.diff_states.update(&mut app, |mgr, _ctx| {
            mgr.remove_connection(conn);
        });
        let has_sub = model.diff_states.read(&app, |mgr, _ctx| {
            !mgr.subscribed_connections(&key3).is_empty()
        });
        assert!(!has_sub);
    });
}

#[test]
fn diff_states_starts_empty() {
    App::test((), |mut app| async move {
        let model = test_model(&mut app);
        let key = test_key("/repo", DiffMode::Head);
        let empty = model.diff_states.read(&app, |mgr, _ctx| {
            mgr.subscribed_connections(&key).is_empty()
        });
        assert!(empty);
    });
}

// ── Git status / GitHub: navigation-driven model cleanup ────────────

#[test]
fn subscribe_git_status_records_subscriber_and_current_repo() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();
        let repo = StandardizedPath::try_new("/repo").unwrap();

        model.subscribe_git_status(conn, &repo);

        assert_eq!(model.git_status_repo_by_conn.get(&conn), Some(&repo));
        assert!(model.git_status_subscribers[&repo].contains(&conn));
    });
}

#[test]
fn navigating_between_repos_moves_the_subscription() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();
        let repo_a = StandardizedPath::try_new("/repo-a").unwrap();
        let repo_b = StandardizedPath::try_new("/repo-b").unwrap();

        model.subscribe_git_status(conn, &repo_a);
        model.subscribe_git_status(conn, &repo_b);

        // Moved off A (now empty) and onto B.
        assert!(!model.git_status_subscribers.contains_key(&repo_a));
        assert!(model.git_status_subscribers[&repo_b].contains(&conn));
        assert_eq!(model.git_status_repo_by_conn.get(&conn), Some(&repo_b));
    });
}

#[test]
fn snapshot_request_does_not_move_another_repos_subscription() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();
        let repo_a = StandardizedPath::try_new("/repo-a").unwrap();
        let repo_b = StandardizedPath::try_new("/repo-b").unwrap();

        // Navigation put the connection in repo A.
        model.subscribe_git_status(conn, &repo_a);

        // A snapshot request for repo B riding this connection must not move
        // the navigation-driven subscription off repo A (mirrors the guard in
        // `handle_update_git_status`).
        if !model.git_status_repo_by_conn.contains_key(&conn) {
            model.subscribe_git_status(conn, &repo_b);
        }
        assert_eq!(model.git_status_repo_by_conn.get(&conn), Some(&repo_a));
        assert!(model.git_status_subscribers[&repo_a].contains(&conn));
        assert!(!model.git_status_subscribers.contains_key(&repo_b));

        // An untracked connection is registered normally.
        let conn2 = uuid::Uuid::new_v4();
        if !model.git_status_repo_by_conn.contains_key(&conn2) {
            model.subscribe_git_status(conn2, &repo_b);
        }
        assert!(model.git_status_subscribers[&repo_b].contains(&conn2));
        assert_eq!(model.git_status_repo_by_conn.get(&conn2), Some(&repo_b));
    });
}

#[test]
fn last_subscriber_leaving_evicts_the_repo() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();
        let repo = StandardizedPath::try_new("/repo").unwrap();

        model.subscribe_git_status(conn, &repo);
        assert!(model.git_status_subscribers.contains_key(&repo));

        model.unsubscribe_git_status(conn);

        // Subscriber set, current-repo mapping, and the per-repo model maps are
        // all cleared once no connection remains in the repo.
        assert!(!model.git_status_subscribers.contains_key(&repo));
        assert!(!model.git_status_repo_by_conn.contains_key(&conn));
        assert!(!model.git_status_models.contains_key(&repo));
        assert!(!model.github_repo_models.contains_key(&repo));
    });
}

#[test]
fn sibling_connection_keeps_the_repo_alive() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn_a = uuid::Uuid::new_v4();
        let conn_b = uuid::Uuid::new_v4();
        let repo = StandardizedPath::try_new("/repo").unwrap();

        model.subscribe_git_status(conn_a, &repo);
        model.subscribe_git_status(conn_b, &repo);

        // First connection leaves: the repo stays for the sibling.
        model.unsubscribe_git_status(conn_a);
        assert!(model.git_status_subscribers[&repo].contains(&conn_b));

        // Second connection leaves: now evicted.
        model.unsubscribe_git_status(conn_b);
        assert!(!model.git_status_subscribers.contains_key(&repo));
    });
}

#[test]
fn unsubscribe_unknown_connection_is_a_noop() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.unsubscribe_git_status(uuid::Uuid::new_v4());
        assert!(model.git_status_subscribers.is_empty());
        assert!(model.git_status_repo_by_conn.is_empty());
    });
}

// ── Daemon host-scoped response failover ────────────────────────────

/// A throwaway host-scoped response payload used to assert routing.
fn write_file_success_message() -> server_message::Message {
    server_message::Message::WriteFileResponse(WriteFileResponse {
        result: Some(write_file_response::Result::Success(WriteFileSuccess {})),
    })
}

#[test]
fn host_scoped_response_fails_over_when_target_send_fails() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let request_id = RequestId::new();
        let target: ConnectionId = uuid::Uuid::new_v4();
        let alternate: ConnectionId = uuid::Uuid::new_v4();

        // The target connection's receiver is dropped, so its sender still
        // exists in the map but `try_send` fails (channel closed).
        let (target_tx, target_rx) = async_channel::bounded(1);
        drop(target_rx);
        model.connection_senders.insert(target, target_tx);

        // The alternate connection has a live receiver.
        let (alt_tx, alt_rx) = async_channel::unbounded();
        model.connection_senders.insert(alternate, alt_tx);

        // Mark the request as host-scoped so failover is eligible.
        model
            .host_scoped_requests
            .insert(request_id.clone(), target);

        model.send_server_message(
            Some(target),
            Some(&request_id),
            write_file_success_message(),
        );

        // The response was re-routed to the alternate connection.
        let received = alt_rx
            .try_recv()
            .expect("alternate should receive failover response");
        assert_eq!(received.request_id, request_id.to_string());
        // The host-scoped entry is consumed regardless of delivery path.
        assert!(!model.host_scoped_requests.contains_key(&request_id));
    });
}

#[test]
fn host_scoped_response_fails_over_when_target_missing() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let request_id = RequestId::new();
        let target: ConnectionId = uuid::Uuid::new_v4();
        let alternate: ConnectionId = uuid::Uuid::new_v4();

        // Target connection is gone entirely (not in the senders map), but the
        // request is still tracked as host-scoped.
        let (alt_tx, alt_rx) = async_channel::unbounded();
        model.connection_senders.insert(alternate, alt_tx);
        model
            .host_scoped_requests
            .insert(request_id.clone(), target);

        model.send_server_message(
            Some(target),
            Some(&request_id),
            write_file_success_message(),
        );

        let received = alt_rx
            .try_recv()
            .expect("alternate should receive failover response");
        assert_eq!(received.request_id, request_id.to_string());
        assert!(!model.host_scoped_requests.contains_key(&request_id));
    });
}

#[test]
fn non_host_scoped_response_is_not_failed_over() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let request_id = RequestId::new();
        let target: ConnectionId = uuid::Uuid::new_v4();
        let alternate: ConnectionId = uuid::Uuid::new_v4();

        // Target sender exists but is closed; the request is NOT tracked as
        // host-scoped, so the message must be dropped rather than re-routed.
        let (target_tx, target_rx) = async_channel::bounded(1);
        drop(target_rx);
        model.connection_senders.insert(target, target_tx);
        let (alt_tx, alt_rx) = async_channel::unbounded::<ServerMessage>();
        model.connection_senders.insert(alternate, alt_tx);

        model.send_server_message(
            Some(target),
            Some(&request_id),
            write_file_success_message(),
        );

        assert!(
            alt_rx.try_recv().is_err(),
            "non-host-scoped response must not fail over to another connection"
        );
    });
}

// ── 本地文件系统 handler(local_fs)────────────────────────────────

#[cfg(feature = "local_fs")]
#[test]
fn resolve_path_reports_file_metadata() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.txt");
        fs::write(&file_path, "hello").unwrap();
        let model = test_model(&mut app);

        let response = model.handle_resolve_path(ResolvePath {
            path: file_path.to_string_lossy().to_string(),
        });

        let server_message::Message::ResolvePathResponse(response) = response.into_message() else {
            panic!("expected ResolvePathResponse");
        };
        let Some(resolve_path_response::Result::Success(success)) = response.result else {
            panic!("expected resolve path success");
        };
        assert_eq!(
            success.canonical_path,
            fs::canonicalize(&file_path).unwrap().to_string_lossy()
        );
        assert_eq!(
            success.kind,
            super::super::proto::FileSystemEntryKind::File as i32
        );
        assert_eq!(success.size_bytes, Some(5));
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn list_directory_returns_sorted_metadata() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        fs::create_dir(dir.path().join("a-dir")).unwrap();
        let model = test_model(&mut app);

        let response = model.handle_list_directory(ListDirectory {
            path: dir.path().to_string_lossy().to_string(),
        });

        let server_message::Message::ListDirectoryResponse(response) = response.into_message()
        else {
            panic!("expected ListDirectoryResponse");
        };
        let Some(list_directory_response::Result::Success(success)) = response.result else {
            panic!("expected list directory success");
        };
        assert_eq!(
            success.canonical_path,
            fs::canonicalize(dir.path()).unwrap().to_string_lossy()
        );
        assert_eq!(success.entries.len(), 2);
        assert_eq!(success.entries[0].name, "a-dir");
        assert_eq!(
            success.entries[0].kind,
            super::super::proto::FileSystemEntryKind::Directory as i32
        );
        assert_eq!(success.entries[1].name, "b.txt");
        assert_eq!(
            success.entries[1].kind,
            super::super::proto::FileSystemEntryKind::File as i32
        );
        assert_eq!(success.entries[1].size_bytes, Some(1));
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn read_and_write_file_chunks_round_trip_binary_data() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("blob.bin");
        let model = test_model(&mut app);

        let write_response = model.handle_write_file_chunk(WriteFileChunk {
            path: file_path.to_string_lossy().to_string(),
            offset: 0,
            bytes: vec![0, 1, 2, 3],
            truncate: true,
            executable: None,
        });
        let server_message::Message::WriteFileChunkResponse(write_response) =
            write_response.into_message()
        else {
            panic!("expected WriteFileChunkResponse");
        };
        let Some(write_file_chunk_response::Result::Success(write_success)) = write_response.result
        else {
            panic!("expected write chunk success");
        };
        assert_eq!(write_success.next_offset, 4);

        let read_response = model.handle_read_file_chunk(ReadFileChunk {
            path: file_path.to_string_lossy().to_string(),
            offset: 1,
            max_bytes: 2,
        });
        let server_message::Message::ReadFileChunkResponse(read_response) =
            read_response.into_message()
        else {
            panic!("expected ReadFileChunkResponse");
        };
        let Some(read_file_chunk_response::Result::Success(read_success)) = read_response.result
        else {
            panic!("expected read chunk success");
        };
        assert_eq!(read_success.bytes, vec![1, 2]);
        assert_eq!(read_success.next_offset, 3);
        assert_eq!(read_success.total_size, Some(4));
        assert!(!read_success.eof);
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn create_directory_creates_nested_directories() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let model = test_model(&mut app);

        let response = model.handle_create_directory(CreateDirectory {
            path: nested.to_string_lossy().to_string(),
        });

        let server_message::Message::CreateDirectoryResponse(response) = response.into_message()
        else {
            panic!("expected CreateDirectoryResponse");
        };
        assert!(matches!(
            response.result,
            Some(super::super::proto::create_directory_response::Result::Success(_))
        ));
        assert!(nested.is_dir());
    });
}
