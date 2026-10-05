use super::*;

fn clr_event(pid: u32, first_chance: u32, parameter_count: u32, hresult: usize) -> DEBUG_EVENT {
    use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_DEBUG_INFO, EXCEPTION_RECORD};
    let mut event = DEBUG_EVENT::default();
    event.dwDebugEventCode = EXCEPTION_DEBUG_EVENT;
    event.dwProcessId = pid;
    event.dwThreadId = 8;
    event.u.Exception = EXCEPTION_DEBUG_INFO {
        ExceptionRecord: EXCEPTION_RECORD {
            ExceptionCode: windows::Win32::Foundation::NTSTATUS(CLR_EXCEPTION as i32),
            NumberParameters: parameter_count,
            ExceptionInformation: [hresult, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        },
        dwFirstChance: first_chance,
    };
    event
}

#[test]
fn clr_hresult_accepts_zero_and_sign_extended_event_parameters() {
    assert_eq!(
        exception_hresult(&clr_event(7, 1, 1, 0x80070005), 7).unwrap(),
        Some(0x80070005)
    );
    assert_eq!(
        exception_hresult(&clr_event(7, 1, 1, 0xffffffff80070005), 7).unwrap(),
        Some(0x80070005)
    );
}

#[test]
fn clr_reader_ignores_child_second_chance_and_non_clr_events() {
    assert_eq!(
        exception_hresult(&clr_event(9, 1, 1, 0x80070005), 7).unwrap(),
        None
    );
    assert_eq!(
        exception_hresult(&clr_event(7, 0, 1, 0x80070005), 7).unwrap(),
        None
    );
    let mut event = clr_event(7, 1, 1, 0x80070005);
    event.dwDebugEventCode = CREATE_THREAD_DEBUG_EVENT;
    assert_eq!(exception_hresult(&event, 7).unwrap(), None);
    event.dwDebugEventCode = EXCEPTION_DEBUG_EVENT;
    let mut information = unsafe { event.u.Exception };
    information.ExceptionRecord.ExceptionCode = EXCEPTION_BREAKPOINT;
    event.u.Exception = information;
    assert_eq!(exception_hresult(&event, 7).unwrap(), None);
}

#[test]
fn clr_reader_rejects_missing_or_out_of_bounds_exception_parameters() {
    assert!(exception_hresult(&clr_event(7, 1, 0, 0x80070005), 7).is_err());
    assert!(exception_hresult(&clr_event(7, 1, 16, 0x80070005), 7).is_err());
}

#[test]
fn clr_event_receipts_preserve_later_events_without_overwriting_the_first() {
    let directory = tempfile::tempdir().unwrap();
    let first = serde_json::json!({"event_sequence":1,"partial":true});
    let later = serde_json::json!({"event_sequence":17,"partial":false});
    write_receipt(
        &directory.path().join("native-clr-exception-1.json"),
        &first,
    )
    .unwrap();
    write_receipt(
        &directory.path().join("native-clr-exception-17.json"),
        &later,
    )
    .unwrap();
    assert_eq!(
        write_receipt(
            &directory.path().join("native-clr-exception-1.json"),
            &later
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::AlreadyExists
    );
    let recorded: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("native-clr-exception-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(recorded, first);
    let recorded: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("native-clr-exception-17.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(recorded, later);
}
