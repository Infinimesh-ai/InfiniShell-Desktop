use super::*;

fn return_event(pid: u32, first_chance: u32, code: u32) -> DEBUG_EVENT {
    use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_DEBUG_INFO, EXCEPTION_RECORD};
    let mut event = DEBUG_EVENT::default();
    event.dwDebugEventCode = EXCEPTION_DEBUG_EVENT;
    event.dwProcessId = pid;
    event.dwThreadId = 8;
    event.u.Exception = EXCEPTION_DEBUG_INFO {
        ExceptionRecord: EXCEPTION_RECORD {
            ExceptionCode: windows::Win32::Foundation::NTSTATUS(code as i32),
            ..Default::default()
        },
        dwFirstChance: first_chance,
    };
    event
}

fn classification_receipt() -> serde_json::Value {
    serde_json::json!({"identity":{"process_id":7,"thread_id":8},
        "node_create_sequence":2,"entry_sequence":3,"return_sequence":4,
        "node_process_id":9,"node_process_birth":100,"expected_node_matched":true,
        "flags":0x2000,"registers_restored":true,"execution_context_unchanged":true,
        "post_start_clr_stack_required":true})
}

#[test]
fn clr_reader_requires_the_original_owned_return_event() {
    let receipt = classification_receipt();
    assert!(validate_return_binding(&return_event(7, 1, SINGLE_STEP), 7, 4, &receipt).is_ok());
    assert!(validate_return_binding(&return_event(9, 1, SINGLE_STEP), 7, 4, &receipt).is_err());
    assert!(validate_return_binding(&return_event(7, 0, SINGLE_STEP), 7, 4, &receipt).is_err());
    assert!(validate_return_binding(&return_event(7, 1, SINGLE_STEP), 7, 5, &receipt).is_err());
}

#[test]
fn clr_reader_never_reuses_original_clr_exceptions_as_classification_stops() {
    let receipt = classification_receipt();
    assert!(validate_return_binding(&return_event(7, 1, 0xe0434352), 7, 4, &receipt).is_err());
    assert!(validate_return_binding(&return_event(7, 1, 0x80000003), 7, 4, &receipt).is_err());
    let mut event = return_event(7, 1, SINGLE_STEP);
    event.dwDebugEventCode = CREATE_THREAD_DEBUG_EVENT;
    assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
}

#[test]
fn clr_reader_requires_node_create_before_the_classification_pair() {
    let event = return_event(7, 1, SINGLE_STEP);
    let mut receipt = classification_receipt();
    receipt["node_create_sequence"] = serde_json::json!(3);
    assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
    receipt["node_create_sequence"] = serde_json::json!(2);
    receipt["node_process_birth"] = serde_json::json!(0);
    assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
}

#[test]
fn clr_reader_cannot_begin_before_owned_registers_are_restored() {
    let event = return_event(7, 1, SINGLE_STEP);
    let mut receipt = classification_receipt();
    receipt["registers_restored"] = serde_json::json!(false);
    assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
    receipt["registers_restored"] = serde_json::json!(true);
    receipt["execution_context_unchanged"] = serde_json::json!(false);
    assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
}

#[test]
fn clr_event_receipts_preserve_later_events_without_overwriting_the_first() {
    let directory = tempfile::tempdir().unwrap();
    let first = serde_json::json!({"event_sequence":1,"partial":true});
    let later = serde_json::json!({"event_sequence":17,"partial":false});
    write_receipt(
        &directory.path().join("native-clr-classification-1.json"),
        &first,
    )
    .unwrap();
    write_receipt(
        &directory.path().join("native-clr-classification-17.json"),
        &later,
    )
    .unwrap();
    assert_eq!(
        write_receipt(
            &directory.path().join("native-clr-classification-1.json"),
            &later
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::AlreadyExists
    );
    let recorded: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("native-clr-classification-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(recorded, first);
    let recorded: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("native-clr-classification-17.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(recorded, later);
}
