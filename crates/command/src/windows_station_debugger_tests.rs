use std::fs;

use super::*;

#[test]
fn application_path_normalization_preserves_canonical_request_and_argv0() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("中文 程序.exe");
    fs::write(&path, b"original").unwrap();
    let canonical = path.canonicalize().unwrap();
    let command = wide(format!("\"{}\" --version", canonical.display()).as_ref()).unwrap();
    let (request, image) = Spawn::new(
        &path,
        command.clone(),
        vec![0, 0],
        temporary.path(),
        [HANDLE::default(); 3],
        false,
    )
    .unwrap();

    let application = image.application_path().unwrap();

    assert_eq!(request.program, canonical);
    assert_eq!(request.command, command);
    assert_eq!(request.program_identity, image.entries[0].2);
    assert_ne!(application, wide(request.program.as_os_str()).unwrap());
    image.verify().unwrap();
}

#[test]
fn replayed_resume_is_rejected_before_native_dispatch() {
    let message = Message {
        nonce: "first".to_owned(),
        sequence: 3,
        operation: Operation::Resume,
    };
    assert!(validate_message("first", 3, &message).is_err());
    assert!(validate_message("first", 2, &message).is_ok());
}

#[test]
fn continue_from_another_control_generation_is_rejected() {
    let message = Message {
        nonce: "old".to_owned(),
        sequence: 4,
        operation: Operation::Continue {
            event: 2,
            pid: 10,
            tid: 11,
            code: CREATE_PROCESS_DEBUG_EVENT.0,
            status: DBG_CONTINUE.0,
        },
    };
    assert!(validate_message("new", 3, &message).is_err());
}

#[test]
fn skipped_control_message_cannot_authorize_close() {
    let message = Message {
        nonce: "first".to_owned(),
        sequence: 5,
        operation: Operation::Close,
    };
    assert!(validate_message("first", 3, &message).is_err());
}

#[test]
fn continue_must_match_original_event_generation_and_process_thread() {
    let pending = Pending {
        sequence: 5,
        pid: 10,
        tid: 11,
        code: LOAD_DLL_DEBUG_EVENT.0,
    };
    assert!(!pending.matches(4, 10, 11, LOAD_DLL_DEBUG_EVENT.0));
    assert!(!pending.matches(5, 20, 11, LOAD_DLL_DEBUG_EVENT.0));
    assert!(!pending.matches(5, 10, 12, LOAD_DLL_DEBUG_EVENT.0));
    assert!(!pending.matches(5, 10, 11, CREATE_PROCESS_DEBUG_EVENT.0));
    assert!(pending.matches(5, 10, 11, LOAD_DLL_DEBUG_EVENT.0));
}

#[test]
fn concurrent_clone_control_is_rejected_without_waiting_for_owner() {
    let debugger = StationDebugger(Arc::new(Mutex::new(None)));
    let alias = debugger.clone();
    let _owner = debugger.0.lock().unwrap();

    let failure = alias
        .wait_event(100, Instant::now() + CLEANUP_TIMEOUT)
        .err()
        .unwrap();

    assert_eq!(failure.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(
        alias.invalidate().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}
