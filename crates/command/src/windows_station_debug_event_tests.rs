use super::*;

#[test]
fn create_event_keeps_remote_values_until_explicit_handle_transfer() {
    let mut native = DEBUG_EVENT {
        dwDebugEventCode: CREATE_PROCESS_DEBUG_EVENT,
        dwProcessId: 24,
        dwThreadId: 25,
        ..Default::default()
    };
    native.u.CreateProcessInfo.hFile = HANDLE(12_usize as *mut c_void);
    native.u.CreateProcessInfo.hProcess = HANDLE(16_usize as *mut c_void);
    native.u.CreateProcessInfo.hThread = HANDLE(20_usize as *mut c_void);
    native.u.CreateProcessInfo.lpBaseOfImage = 0x7ff00000_usize as *mut c_void;
    native.u.CreateProcessInfo.fUnicode = 1;

    let encoded = serde_json::to_vec(&Event::capture(7, &native).unwrap()).unwrap();
    let decoded: Event = serde_json::from_slice(&encoded).unwrap();
    let restored = decoded.restore().unwrap();

    assert_eq!(decoded.sequence, 7);
    assert_eq!(restored.dwProcessId, 24);
    assert_eq!(restored.dwThreadId, 25);
    unsafe {
        assert_eq!(
            restored.u.CreateProcessInfo.hFile,
            HANDLE(12_usize as *mut c_void)
        );
        assert_eq!(
            restored.u.CreateProcessInfo.hProcess,
            HANDLE(16_usize as *mut c_void)
        );
        assert_eq!(
            restored.u.CreateProcessInfo.hThread,
            HANDLE(20_usize as *mut c_void)
        );
        assert_eq!(
            restored.u.CreateProcessInfo.lpBaseOfImage,
            0x7ff00000_usize as *mut c_void
        );
        assert_eq!(restored.u.CreateProcessInfo.fUnicode, 1);
    }
}

#[test]
fn exception_preserves_signed_status_and_remote_exception_parameters() {
    let mut native = DEBUG_EVENT {
        dwDebugEventCode: EXCEPTION_DEBUG_EVENT,
        dwProcessId: 10,
        dwThreadId: 11,
        ..Default::default()
    };
    native.u.Exception.ExceptionRecord.ExceptionCode = NTSTATUS(0xc0000005_u32 as i32);
    native.u.Exception.ExceptionRecord.NumberParameters = 2;
    native.u.Exception.ExceptionRecord.ExceptionInformation =
        [1, 0x7fff0000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    native.u.Exception.dwFirstChance = 1;

    let event = Event::capture(1, &native).unwrap().restore().unwrap();

    unsafe {
        assert_eq!(
            event.u.Exception.ExceptionRecord.ExceptionCode.0,
            0xc0000005_u32 as i32
        );
        assert_eq!(event.u.Exception.ExceptionRecord.NumberParameters, 2);
        assert_eq!(
            event.u.Exception.ExceptionRecord.ExceptionInformation[1],
            0x7fff0000
        );
        assert_eq!(event.u.Exception.dwFirstChance, 1);
    }
}

#[test]
fn truncated_event_is_rejected_before_union_access() {
    let event = Event {
        sequence: 1,
        pid: 10,
        tid: 11,
        code: CREATE_PROCESS_DEBUG_EVENT.0,
        values: vec![12, 16],
    };
    assert!(event.restore().is_err());
}

#[test]
fn unknown_event_cannot_be_reinterpreted_as_a_handle_event() {
    let event = Event {
        sequence: 1,
        pid: 10,
        tid: 11,
        code: 1000,
        values: vec![12, 16, 20],
    };
    assert!(event.restore().is_err());
}

#[test]
fn exception_parameter_count_cannot_exceed_native_array() {
    let event = Event {
        sequence: 1,
        pid: 10,
        tid: 11,
        code: EXCEPTION_DEBUG_EVENT.0,
        values: vec![
            0, 0, 0, 0, 16, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    };
    assert!(event.restore().is_err());
}
