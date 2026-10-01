use super::*;
use websocket::tungstenite::accept;

const THREAD: &str = "10000000-0000-4000-8000-000000000001";
const OTHER: &str = "10000000-0000-4000-8000-000000000002";

fn thread() -> Value {
    json!({"thread": {"id": THREAD, "cwd": "/fixture/project", "ephemeral": false,
        "parentThreadId": null, "status": {"type": "idle"}, "turns": []}})
}

/// 只走真实 UnixStream/WebSocket；夹具逐条拒绝任何非只读业务 RPC。
fn discover(replies: Vec<(&'static str, Value)>) -> io::Result<Uuid> {
    read_session(replies, None, &|_| Ok(()))
}

fn read_session(
    replies: Vec<(&'static str, Value)>,
    expected: Option<Uuid>,
    validate: &impl Fn(&UnixStream) -> io::Result<()>,
) -> io::Result<Uuid> {
    let (client, server) = UnixStream::pair().unwrap();
    let worker = std::thread::spawn(move || {
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = accept(server).unwrap();
        let init: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(init["method"], "initialize");
        socket
            .send(Message::Text(
                json!({"id": init["id"], "result": {
            "userAgent": "codex/0.156.1 fixture"}})
                .to_string(),
            ))
            .unwrap();
        let initialized: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(initialized["method"], "initialized");
        for (method, result) in replies {
            assert!(matches!(method, "thread/loaded/list" | "thread/read"));
            let request: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(request["method"], method);
            if method == "thread/read" {
                assert_eq!(
                    request["params"],
                    json!({"threadId": THREAD, "includeTurns": false})
                );
            }
            socket
                .send(Message::Text(
                    json!({"id": request["id"], "result": result}).to_string(),
                ))
                .unwrap();
        }
    });
    let result = match expected {
        Some(id) => CodexImageQueue::connect(client, id, Path::new("/fixture/project"), validate)
            .map(|_| id),
        None => CodexImageQueue::discover(client, Path::new("/fixture/project"), validate),
    };
    worker.join().unwrap();
    result
}

#[test]
fn zero_turn_loaded_thread_is_discovered_without_start_or_input() {
    let result = discover(vec![
        (
            "thread/loaded/list",
            json!({"data": [THREAD], "nextCursor": null}),
        ),
        ("thread/read", thread()),
    ])
    .unwrap();
    assert_eq!(result.to_string(), THREAD);
}

#[test]
fn discovery_requires_complete_pagination() {
    let result = discover(vec![
        (
            "thread/loaded/list",
            json!({"data": [], "nextCursor": "page-2"}),
        ),
        (
            "thread/loaded/list",
            json!({"data": [THREAD], "nextCursor": null}),
        ),
        ("thread/read", thread()),
    ])
    .unwrap();
    assert_eq!(result.to_string(), THREAD);
}

#[test]
fn another_loaded_thread_on_a_later_page_is_rejected() {
    let result = discover(vec![
        (
            "thread/loaded/list",
            json!({"data": [THREAD], "nextCursor": "page-2"}),
        ),
        (
            "thread/loaded/list",
            json!({"data": [OTHER], "nextCursor": null}),
        ),
    ]);
    assert!(result.is_err());
}

#[test]
fn cyclic_loaded_cursor_is_rejected() {
    let result = discover(vec![
        (
            "thread/loaded/list",
            json!({"data": [], "nextCursor": "same"}),
        ),
        (
            "thread/loaded/list",
            json!({"data": [], "nextCursor": "same"}),
        ),
    ]);
    assert!(result.is_err());
}

#[test]
fn unloaded_persisted_history_cannot_supply_a_session() {
    assert!(
        discover(vec![(
            "thread/loaded/list",
            json!({"data": [], "nextCursor": null})
        )])
        .is_err()
    );
}

#[test]
fn changed_cwd_is_rejected() {
    let mut response = thread();
    response["thread"]["cwd"] = json!("/another/project");
    assert!(
        discover(vec![
            (
                "thread/loaded/list",
                json!({"data": [THREAD], "nextCursor": null})
            ),
            ("thread/read", response),
        ])
        .is_err()
    );
}

#[test]
fn thread_replaced_between_list_and_read_is_rejected() {
    let mut response = thread();
    response["thread"]["id"] = json!(OTHER);
    assert!(
        discover(vec![
            (
                "thread/loaded/list",
                json!({"data": [THREAD], "nextCursor": null})
            ),
            ("thread/read", response),
        ])
        .is_err()
    );
}

#[test]
fn repeated_thread_on_two_pages_is_rejected() {
    assert!(
        discover(vec![
            (
                "thread/loaded/list",
                json!({"data": [THREAD], "nextCursor": "page-2"})
            ),
            (
                "thread/loaded/list",
                json!({"data": [THREAD], "nextCursor": null})
            ),
        ])
        .is_err()
    );
}

#[test]
fn two_threads_on_one_page_are_rejected() {
    assert!(
        discover(vec![(
            "thread/loaded/list",
            json!({"data": [THREAD, OTHER], "nextCursor": null})
        )])
        .is_err()
    );
}

#[test]
fn malformed_native_session_is_rejected() {
    assert!(
        discover(vec![(
            "thread/loaded/list",
            json!({"data": ["not-a-uuid"], "nextCursor": null})
        )])
        .is_err()
    );
}

#[test]
fn empty_native_session_is_rejected() {
    assert!(
        discover(vec![(
            "thread/loaded/list",
            json!({"data": [""], "nextCursor": null})
        )])
        .is_err()
    );
}

#[test]
fn nil_native_session_is_rejected() {
    assert!(
        discover(vec![(
            "thread/loaded/list",
            json!({"data": ["00000000-0000-0000-0000-000000000000"], "nextCursor": null})
        )])
        .is_err()
    );
}

#[test]
fn later_connection_cannot_move_a_pinned_sid_to_another_loaded_thread() {
    assert!(
        read_session(
            vec![(
                "thread/loaded/list",
                json!({"data": [OTHER], "nextCursor": null})
            )],
            Some(Uuid::parse_str(THREAD).unwrap()),
            &|_| Ok(())
        )
        .is_err()
    );
}

#[test]
fn peer_identity_change_after_readonly_rpc_rejects_the_proof() {
    let calls = std::cell::Cell::new(0);
    let result = read_session(
        vec![
            (
                "thread/loaded/list",
                json!({"data": [THREAD], "nextCursor": null}),
            ),
            ("thread/read", thread()),
        ],
        None,
        &|_| {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                Err(io::Error::other("fixture_peer_changed"))
            } else {
                Ok(())
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(calls.get(), 3);
}
