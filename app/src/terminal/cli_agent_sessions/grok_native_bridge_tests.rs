use super::*;
use serde_json::{Value, json};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _, symlink};
use std::os::unix::net::UnixListener;

fn instance() -> Uuid {
    Uuid::parse_str("72912484-76c6-46df-931c-38db2f552c4e").unwrap()
}
fn message() -> NativeBridgeMessage {
    NativeBridgeMessage::new(instance(), "native-session".into(), "中文\n第二行").unwrap()
}
fn wire(value: Value) -> WireResponse {
    serde_json::from_value(value).unwrap()
}

fn state() -> Value {
    json!({ "status": "state", "instance_id": instance(), "input_epoch": 7, "ready": true,
        "reason": null, "lease": {"lease_id": instance(), "session_id": "native-session", "agent_id": 3, "binding_epoch": 9, "input_epoch": 7} })
}

fn prepared() -> Value {
    json!({"status":"prepared", "instance_id":instance(), "input_epoch":7,
        "agent_id":3,"expected_binding_epoch":9,"expected_session_id":"native-session"})
}

fn receipt() -> Value {
    let expected = message();
    json!({"status": "receipt", "receipt": {
        "message_id": expected.message_id, "prompt_id": expected.message_id,
        "session_id": expected.session_id, "payload_digest": expected.payload_digest,
        "state": "native_acknowledged"
    }})
}

#[test]
fn state_requires_coherent_native_identity_epoch_and_live_lease() {
    let value = parse_state(wire(state()), instance(), Instant::now()).unwrap();
    let lease = value.lease.unwrap();
    assert_eq!(lease.session_id(), "native-session");
    assert_eq!(lease.binding_epoch(), 9);
    assert_eq!(lease.input_epoch(), 7);
    for (pointer, replacement) in [
        ("/instance_id", json!(Uuid::new_v4())),
        ("/ready", json!(false)),
        ("/reason", json!("session_busy")),
        ("/lease", Value::Null),
        ("/lease/lease_id", json!(Uuid::nil())),
        ("/lease/input_epoch", json!(8)),
        ("/lease/session_id", json!("")),
    ] {
        let mut value = state();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert!(parse_state(wire(value), instance(), Instant::now()).is_err());
    }
    assert!(parse_state(wire(state()), instance(), Instant::now() - LEASE_LIFETIME).is_err());
    let blocked = json!({"status":"state", "instance_id":instance(), "input_epoch":8,
        "ready":false,"reason":"session_busy","lease":null});
    let blocked = parse_state(wire(blocked), instance(), Instant::now()).unwrap();
    assert!(!blocked.ready && blocked.lease.is_none());
}

#[test]
fn preparation_request_has_only_instance_and_epoch_without_prompt_or_message() {
    let instance_id = instance().to_string();
    let request = Request::PrepareSessionIfIdle {
        instance_id: &instance_id,
        input_epoch: 6,
    };
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "operation":"prepare_session_if_idle", "instance_id":instance(), "input_epoch":6,
        })
    );
}

#[test]
fn prepared_response_consumes_exact_epoch_and_binds_actual_native_target() {
    let preparation = parse_preparation(wire(prepared()), instance(), 6)
        .unwrap()
        .unwrap();
    let state = parse_state(wire(state()), instance(), Instant::now()).unwrap();
    let lease = preparation.lease_from_state(state).unwrap().unwrap();
    assert_eq!(lease.session_id(), "native-session");
    assert_eq!(lease.binding_epoch(), 9);
    assert_eq!(lease.input_epoch(), 7);
    assert!(parse_preparation(wire(prepared()), Uuid::new_v4(), 6).is_err());
    assert!(parse_preparation(wire(prepared()), instance(), 5).is_err());
    assert!(parse_preparation(wire(prepared()), instance(), 7).is_err());
    assert!(parse_preparation(wire(prepared()), instance(), u64::MAX).is_err());
    let mut invalid_session = prepared();
    invalid_session["expected_session_id"] = json!("");
    assert!(parse_preparation(wire(invalid_session), instance(), 6).is_err());
}

#[test]
fn preparation_loading_only_yields_lease_for_original_target_after_binding() {
    let preparation = parse_preparation(wire(prepared()), instance(), 6)
        .unwrap()
        .unwrap();
    let loading = json!({"status":"state", "instance_id":instance(), "input_epoch":8,
        "ready":false,"reason":"session_loading","lease":null});
    let loading = parse_state(wire(loading), instance(), Instant::now()).unwrap();
    assert!(preparation.lease_from_state(loading).unwrap().is_none());
    let mut ready = state();
    ready["input_epoch"] = json!(9);
    ready["lease"]["input_epoch"] = json!(9);
    assert!(
        preparation
            .lease_from_state(parse_state(wire(ready), instance(), Instant::now()).unwrap())
            .unwrap()
            .is_some()
    );
}

fn assert_preparation_rejects_changed_target(pointer: &str, replacement: Value) {
    let preparation = parse_preparation(wire(prepared()), instance(), 6)
        .unwrap()
        .unwrap();
    let mut changed = state();
    changed["input_epoch"] = json!(8);
    changed["lease"]["input_epoch"] = json!(8);
    *changed.pointer_mut(pointer).unwrap() = replacement;
    let state = parse_state(wire(changed), instance(), Instant::now()).unwrap();
    assert!(preparation.lease_from_state(state).is_err());
}

#[test]
fn preparation_rejects_native_tab_switch_rebind_or_different_session() {
    assert_preparation_rejects_changed_target("/lease/agent_id", json!(4));
    assert_preparation_rejects_changed_target("/lease/binding_epoch", json!(10));
    assert_preparation_rejects_changed_target("/lease/session_id", json!("another-session"));
    let preparation = parse_preparation(wire(prepared()), instance(), 6)
        .unwrap()
        .unwrap();
    let old = json!({"status":"state", "instance_id":instance(), "input_epoch":6,
        "ready":false,"reason":"session_loading","lease":null});
    assert!(
        preparation
            .lease_from_state(parse_state(wire(old), instance(), Instant::now()).unwrap())
            .is_err()
    );
    let mut changed_instance = state();
    let new_instance = Uuid::new_v4();
    changed_instance["instance_id"] = json!(new_instance);
    let state = parse_state(wire(changed_instance), new_instance, Instant::now()).unwrap();
    assert!(preparation.lease_from_state(state).is_err());
}

#[test]
fn preparation_refusal_or_wrong_response_never_provides_a_submission_lease() {
    assert!(
        parse_preparation(
            wire(json!({"status":"not_ready","reason":"application_owns_input"})),
            instance(),
            6
        )
        .unwrap()
        .is_none()
    );
    assert!(
        parse_preparation(
            wire(json!({"status":"rejected","reason":"stale_epoch"})),
            instance(),
            6
        )
        .unwrap()
        .is_none()
    );
    assert!(
        parse_preparation(
            wire(json!({"status":"not_ready","reason":""})),
            instance(),
            6
        )
        .is_err()
    );
    assert!(parse_preparation(wire(receipt()), instance(), 6).is_err());
    assert!(parse_preparation(wire(state()), instance(), 6).is_err());
}

#[test]
fn prepared_response_is_neither_input_acknowledgement_nor_ready_state() {
    assert!(parse_receipt(wire(prepared()), &message()).is_err());
    assert!(parse_state(wire(prepared()), instance(), Instant::now()).is_err());
    assert!(
        parse_receipt(
            wire(json!({"status":"not_ready","reason":"no_active_agent"})),
            &message()
        )
        .is_err()
    );
    let mut with_prompt = prepared();
    with_prompt["text"] = json!("不得从准备回包取得正文");
    assert!(serde_json::from_value::<WireResponse>(with_prompt).is_err());
}

#[test]
fn receipt_must_match_message_prompt_session_and_exact_original_digest() {
    assert!(matches!(
        parse_receipt(wire(receipt()), &message()).unwrap(),
        NativeBridgeResponse::Receipt(NativeBridgeReceipt {
            state: NativeBridgeReceiptState::NativeAcknowledged,
            ..
        })
    ));
    for (key, replacement) in [
        ("message_id", json!(Uuid::new_v4())),
        ("prompt_id", json!(Uuid::new_v4())),
        ("session_id", json!("other-session")),
        (
            "payload_digest",
            json!(format!("blake3:{}", "0".repeat(64))),
        ),
    ] {
        let mut value = receipt();
        value["receipt"][key] = replacement;
        assert!(parse_receipt(wire(value), &message()).is_err());
    }
    for state in ["claimed", "dispatched", "unknown", "native_rejected"] {
        let mut value = receipt();
        value["receipt"]["state"] = state.into();
        assert!(
            matches!(parse_receipt(wire(value), &message()).unwrap(), NativeBridgeResponse::Receipt(value)
            if value.state != NativeBridgeReceiptState::NativeAcknowledged)
        );
    }
    assert!(matches!(
        parse_receipt(
            wire(json!({"status":"unknown", "message_id":instance()})),
            &message()
        )
        .unwrap(),
        NativeBridgeResponse::Unknown
    ));
    assert!(
        parse_receipt(
            wire(json!({"status":"unknown", "message_id":Uuid::new_v4()})),
            &message()
        )
        .is_err()
    );
}

#[test]
fn strict_protocol_rejects_extra_fields_invalid_states_and_noncanonical_ids() {
    let mut value = receipt();
    value["receipt"]["state"] = "accepted".into();
    assert!(serde_json::from_value::<WireResponse>(value).is_err());
    let mut value = receipt();
    value["receipt"]["extra"] = true.into();
    assert!(serde_json::from_value::<WireResponse>(value).is_err());
    assert!(canonical_uuid(&instance().simple().to_string()).is_err());
    assert!(canonical_uuid(&instance().to_string().to_uppercase()).is_err());
    assert!(NativeBridgeMessage::new(Uuid::nil(), "session".into(), "text").is_err());
    assert!(
        NativeBridgeMessage::new(
            instance(),
            "session".into(),
            &"a".repeat(MAX_TEXT_BYTES + 1)
        )
        .is_err()
    );
}

#[test]
fn frames_are_bounded_and_truncated_response_is_not_acknowledged() {
    for length in [0u32, MAX_FRAME_BYTES as u32 + 1] {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        server.write_all(&length.to_be_bytes()).unwrap();
        assert!(read_frame(&mut client, Instant::now() + IO_TIMEOUT).is_err());
    }
    let (mut client, mut server) = UnixStream::pair().unwrap();
    server.write_all(&10u32.to_be_bytes()).unwrap();
    server.write_all(b"{}").unwrap();
    server.shutdown(Shutdown::Write).unwrap();
    assert_eq!(
        read_frame(&mut client, Instant::now() + IO_TIMEOUT)
            .unwrap_err()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
    let (mut client, mut server) = UnixStream::pair().unwrap();
    write_frame(&mut server, b"{}", Instant::now() + IO_TIMEOUT).unwrap();
    assert_eq!(
        read_frame(&mut client, Instant::now() + IO_TIMEOUT).unwrap(),
        b"{}"
    );
    assert_eq!(
        time_left(Instant::now() - Duration::from_secs(1))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn revoked_or_expired_admission_writes_no_request_bytes() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    server.set_nonblocking(true).unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(
        write_admitted_frame(&mut client, b"{}", None, || {
            calls.set(calls.get() + 1);
            false
        })
        .is_err()
    );
    assert_eq!(calls.get(), 1);
    assert!(
        write_admitted_frame(
            &mut client,
            b"{}",
            Some(Instant::now() - Duration::from_secs(1)),
            || {
                calls.set(calls.get() + 1);
                true
            }
        )
        .is_err()
    );
    assert_eq!(calls.get(), 1);
    let mut bytes = [0u8; 1];
    assert_eq!(
        server.read(&mut bytes).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    write_admitted_frame(&mut client, b"{}", None, || {
        calls.set(calls.get() + 1);
        true
    })
    .unwrap();
    assert_eq!(calls.get(), 2);
    server.set_nonblocking(false).unwrap();
    assert_eq!(
        read_frame(&mut server, Instant::now() + IO_TIMEOUT).unwrap(),
        b"{}"
    );
}

fn fixture() -> (tempfile::TempDir, PathBuf, UnixListener) {
    let root = tempfile::Builder::new().prefix("nb-").tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let directory = canonical.join(format!("t-{}", instance().simple()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let socket = directory.join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let manifest = json!({"protocol_version":1,"native_pid":std::process::id(),
        "instance_id":instance(),"socket":socket,"token":"3".repeat(64)});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("manifest.json"))
        .unwrap();
    file.write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    (root, directory, listener)
}

fn locator(root: &Path, directory: PathBuf) -> io::Result<Locator> {
    let root = root.canonicalize()?;
    Locator::read(
        &root,
        NodeStamp::of(&fs::symlink_metadata(&root)?),
        directory,
    )?
    .ok_or_else(invalid)
}

#[test]
fn manifest_and_socket_identity_changes_invalidate_the_locator() {
    let (root, directory, _listener) = fixture();
    let found = locator(root.path(), directory.clone()).unwrap();
    found.validate().unwrap();
    let path = directory.join("manifest.json");
    let bytes = fs::read(&path).unwrap();
    fs::rename(&path, directory.join("old.json")).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    assert!(found.validate().is_err());
    let found = locator(root.path(), directory.clone()).unwrap();
    let socket = directory.join("control.sock");
    fs::rename(&socket, directory.join("old.sock")).unwrap();
    let _replacement = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(found.validate().is_err());
}

#[test]
fn manifest_cannot_escape_private_instance_or_follow_symlinks() {
    let (root, directory, _listener) = fixture();
    let path = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["socket"] = directory
        .join("../control.sock")
        .to_string_lossy()
        .into_owned()
        .into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(locator(root.path(), directory.clone()).is_err());
    fs::rename(&path, directory.join("real.json")).unwrap();
    symlink(directory.join("real.json"), &path).unwrap();
    assert!(locator(root.path(), directory.clone()).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(locator(root.path(), directory).is_err());
}
