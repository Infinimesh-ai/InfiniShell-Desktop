//! `manager.rs` 的单元测试。
//!
//! 前半部分是纯函数 helper 的测试(不需要 App 上下文),
//! 后半部分借助 `App::test` 覆盖 `RemoteServerManager` 本体的行为。

use futures::channel::oneshot;
use warp_core::SessionId;
use warp_core::channel::Channel;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::App;

use super::{
    HostRequestError, PendingHostRequest, RemoteServerManager, RemoteServerManagerEvent,
    RipgrepSearchParams, SshRouteNodeState, should_enforce_remote_version_check,
    version_is_compatible,
};
use crate::HostId;
use crate::proto::{ClientMessage, RemoteAgentContextSnapshot, WriteFile, host_scoped_request};
use crate::protocol::RequestId;

// ---------------------------------------------------------------------------
// version_is_compatible
// ---------------------------------------------------------------------------

#[test]
fn version_compat_both_tagged_and_equal() {
    assert!(version_is_compatible(
        Some("v0.2026.05.10.stable"),
        "v0.2026.05.10.stable",
    ));
}

#[test]
fn version_compat_both_tagged_and_different() {
    assert!(!version_is_compatible(
        Some("v0.2026.05.10.stable"),
        "v0.2026.05.10.preview",
    ));
}

#[test]
fn version_compat_both_untagged() {
    // 客户端没有 GIT_RELEASE_TAG(cargo run),服务器也报空串
    // (`script/deploy_remote_server` dev 部署):视为兼容,保留
    // 本地开发循环不受影响。
    assert!(version_is_compatible(None, ""));
}

#[test]
fn version_compat_client_tagged_server_untagged() {
    // 客户端是 release,服务器是 dev 部署 → 视为不兼容,正常
    // 触发 reinstall 流程。
    assert!(!version_is_compatible(Some("v0.2026.05.10.stable"), ""));
}

#[test]
fn version_compat_client_untagged_server_tagged() {
    // **关键场景**:Zap 客户端无 tag(cargo build),
    // 服务器是从官方 CDN 下来的 release(带 tag)。原 helper 判定
    // 不兼容,会触发 `remove_remote_server_binary` → 死循环。
    // 这个 test 仅记录 `version_is_compatible` 自身的行为不变,
    // 真正"跳过校验"由 [`should_enforce_remote_version_check`] 负责。
    assert!(!version_is_compatible(None, "v0.2026.05.10.stable"));
}

// ---------------------------------------------------------------------------
// should_enforce_remote_version_check
// ---------------------------------------------------------------------------

#[test]
fn enforce_version_check_skipped_on_oss() {
    // Zap 临时复用官方 release 二进制时,客户端与服务端版本
    // 永远不一致,必须跳过严格校验。
    assert!(!should_enforce_remote_version_check(Channel::Oss));
}

#[test]
fn enforce_version_check_kept_on_official_channels() {
    // 官方 channel 上客户端和服务端要么都来自同一次 release CI,
    // 要么都来自 `script/deploy_remote_server` 的本地部署,严格
    // 校验仍然必要 —— 保留原有 stale binary 自愈路径。
    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Dev,
        Channel::Local,
        Channel::Integration,
    ] {
        assert!(
            should_enforce_remote_version_check(channel),
            "channel {channel:?} should still enforce version check"
        );
    }
}

// ---------------------------------------------------------------------------
// RemoteServerManager
// ---------------------------------------------------------------------------

#[test]
fn abort_host_request_removes_pending_request_and_resolves_caller() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let host_id = HostId::new("test-host".to_string());
        let request_id = RequestId::new();
        let (result_tx, result_rx) = oneshot::channel();
        let msg = ClientMessage::host_scoped(
            request_id.to_string(),
            host_scoped_request::Message::WriteFile(WriteFile {
                path: "/tmp/test".to_string(),
                content: String::new(),
            }),
        );

        manager.update(&mut app, |manager, _ctx| {
            manager.pending_host_requests.insert(
                request_id.clone(),
                PendingHostRequest {
                    host_id,
                    dispatched_session_id: SessionId::from(1),
                    msg,
                    result_tx,
                    timeout_abort: None,
                },
            );
            manager.abort_host_request(&request_id);
            assert!(!manager.pending_host_requests.contains_key(&request_id));
        });

        assert!(matches!(
            result_rx.await.expect("manager should resolve caller"),
            Err(HostRequestError::Aborted)
        ));
    });
}

#[test]
fn remote_agent_context_snapshot_is_a_host_scoped_manager_event() {
    let host_id = HostId::new("test-host".to_string());
    let event = RemoteServerManagerEvent::RemoteAgentContextSnapshot {
        host_id,
        snapshot: RemoteAgentContextSnapshot {
            revision: 1,
            home_dir: "/home/user".to_string(),
            skills: Vec::new(),
            global_rules: Vec::new(),
        },
    };
    assert!(event.session_id().is_none());
}

#[test]
fn remote_agent_context_snapshot_revisions_are_deduplicated_per_host() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let host_id = HostId::new("test-host".to_string());
        let other_host_id = HostId::new("other-host".to_string());

        manager.update(&mut app, |manager, ctx| {
            assert!(manager.accept_remote_agent_context_snapshot_revision(&host_id, 2));
            assert!(!manager.accept_remote_agent_context_snapshot_revision(&host_id, 2));
            assert!(!manager.accept_remote_agent_context_snapshot_revision(&host_id, 1));
            assert!(manager.accept_remote_agent_context_snapshot_revision(&host_id, 3));
            assert!(manager.accept_remote_agent_context_snapshot_revision(&other_host_id, 1));

            manager.handle_host_disconnected(&host_id, ctx);
            assert!(manager.accept_remote_agent_context_snapshot_revision(&host_id, 3));
        });
    });
}

#[test]
fn ssh_route_graph_validates_depth_and_cycles_and_returns_root_to_leaf_path() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        manager.update(&mut app, |manager, _ctx| {
            let root = SessionId::from(1);
            let child = SessionId::from(2);
            let leaf = SessionId::from(3);
            manager
                .register_ssh_route(root, None, 1, "bastion".to_string(), None)
                .unwrap();
            manager
                .register_ssh_route(child, Some(root), 2, "staging".to_string(), Some(2222))
                .unwrap();
            manager
                .register_ssh_route(leaf, Some(child), 3, "database".to_string(), None)
                .unwrap();

            let path = manager.ssh_route_path(leaf);
            assert_eq!(
                path.iter().map(|node| node.session_id).collect::<Vec<_>>(),
                vec![root, child, leaf]
            );
            assert!(
                manager
                    .register_ssh_route(
                        SessionId::from(4),
                        Some(root),
                        3,
                        "invalid-depth".to_string(),
                        None,
                    )
                    .is_err()
            );
            assert!(
                manager
                    .register_ssh_route(root, Some(leaf), 4, "cycle".to_string(), None)
                    .is_err()
            );

            assert_eq!(
                manager.ssh_route_targets(leaf),
                Some(vec![
                    ("bastion".to_string(), None),
                    ("staging".to_string(), Some(2222)),
                    ("database".to_string(), None),
                ])
            );
        });
    });
}

#[test]
fn ssh_route_display_path_uses_connection_labels_without_exposing_route_internals() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        manager.update(&mut app, |manager, _ctx| {
            let root = SessionId::from(1);
            let child = SessionId::from(2);
            manager
                .register_ssh_route(root, None, 1, "bastion".to_string(), None)
                .unwrap();
            manager
                .register_ssh_route(child, Some(root), 2, "staging".to_string(), None)
                .unwrap();
            manager.session_labels.insert(root, "bastion".to_string());
            manager.session_labels.insert(child, "staging".to_string());

            assert_eq!(
                manager.ssh_route_display_path(child).as_deref(),
                Some("Local > bastion > staging")
            );
            assert_eq!(manager.ssh_route_display_path(SessionId::from(3)), None);
        });
    });
}

#[test]
fn parent_disconnect_blocks_all_route_descendants() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        manager.update(&mut app, |manager, _ctx| {
            let root = SessionId::from(1);
            let child = SessionId::from(2);
            let leaf = SessionId::from(3);
            manager
                .register_ssh_route(root, None, 1, "bastion".to_string(), None)
                .unwrap();
            manager
                .register_ssh_route(child, Some(root), 2, "staging".to_string(), None)
                .unwrap();
            manager
                .register_ssh_route(leaf, Some(child), 3, "database".to_string(), None)
                .unwrap();
            manager.mark_route_descendants_blocked(root);

            assert_eq!(
                manager.ssh_route(child).unwrap().state,
                SshRouteNodeState::BlockedByParent
            );
            assert_eq!(
                manager.ssh_route(leaf).unwrap().state,
                SshRouteNodeState::BlockedByParent
            );
        });
    });
}

#[test]
fn start_ripgrep_search_without_connected_host_resolves_immediately() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let host_id = HostId::new("missing-host".to_string());
        let pending = manager.update(&mut app, |manager, _ctx| {
            manager.start_ripgrep_search(
                &host_id,
                RipgrepSearchParams {
                    pattern: "needle".to_string(),
                    roots: vec![StandardizedPath::try_new("/repo").unwrap()],
                    ignore_case: false,
                    multiline: false,
                    max_matches: 100,
                },
            )
        });

        assert!(matches!(
            pending.result().await,
            Err(HostRequestError::AllSessionsDisconnected)
        ));
    });
}
