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
        "raw_return_u64":0x4550,"raw_return_low32":0x4550,
        "flags":0x2000,"registers_restored":true,"execution_context_unchanged":true,
        "post_start_clr_stack_required":true})
}

fn pre_node_receipt() -> serde_json::Value {
    serde_json::json!({"generation":Uuid::from_bytes([1;16]).as_bytes(),
        "identity":{"process_id":7,"thread_id":8,"process_birth":100,"thread_birth":101},
        "entry_sequence":3,"return_sequence":4,"expected_node_matched":true,
        "raw_return_u64":0x4550,"raw_return_low32":0x4550,
        "flags":0x2000,"registers_restored":true,"execution_context_unchanged":true,
        "pre_node_clr_stack_required":true})
}

fn post_node_exception_event() -> DEBUG_EVENT {
    let mut event = return_event(7, 1, CLR_EXCEPTION);
    let mut information = unsafe { event.u.Exception };
    information.ExceptionRecord.NumberParameters = 1;
    information.ExceptionRecord.ExceptionInformation[0] = 0x80070005;
    event.u.Exception = information;
    event
}

fn post_node_exception_receipt() -> serde_json::Value {
    serde_json::json!({"generation":Uuid::from_bytes([1;16]).as_bytes(),
        "identity":{"process_id":7,"thread_id":8,"process_birth":100,"thread_birth":101},
        "pre_return_sequence":4,"node_create_sequence":5,"node_process_id":9,"node_process_birth":102,
        "event_sequence":6,"exception_code":CLR_EXCEPTION,"first_chance":1,"parameter_count":1,
        "event_hresult":0x80070005u32,"reader_eligible":true,"registers_restored":true,
        "execution_context_unchanged":true})
}

fn node_create_receipt() -> serde_json::Value {
    serde_json::json!({"sequence":5,"process_id":9,"process_birth":102,"original_create_bound":true})
}

#[test]
fn post_node_exception_preserves_the_original_hresult_without_satisfying_return_gates() {
    let event = post_node_exception_event();
    let receipt = post_node_exception_receipt();
    assert_eq!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .unwrap(),
        Some(0x80070005)
    );
    assert!(validate_return_binding(&event, 7, 6, &receipt).is_err());
    assert!(
        validate_pre_node_return_binding(&event, 7, 6, Uuid::from_bytes([1; 16]), &receipt)
            .is_err()
    );
}

#[test]
fn post_node_exception_rejects_a_different_generation_or_pre_worker_identity() {
    let event = post_node_exception_event();
    let receipt = post_node_exception_receipt();
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([2; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
    let mut wrong_worker = pre_node_receipt();
    wrong_worker["identity"]["thread_birth"] = serde_json::json!(103);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &wrong_worker,
            &node_create_receipt(),
        )
        .is_err()
    );
    wrong_worker["identity"]["thread_birth"] = serde_json::json!(101);
    wrong_worker["identity"]["thread_id"] = serde_json::json!(10);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &wrong_worker,
            &node_create_receipt(),
        )
        .is_err()
    );
}

#[test]
fn post_node_exception_requires_pre_return_then_real_node_then_original_event() {
    let event = post_node_exception_event();
    let receipt = post_node_exception_receipt();
    let mut node = node_create_receipt();
    node["sequence"] = serde_json::json!(4);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node,
        )
        .is_err()
    );
    node["sequence"] = serde_json::json!(6);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node,
        )
        .is_err()
    );
    node["sequence"] = serde_json::json!(5);
    node["original_create_bound"] = serde_json::json!(false);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node,
        )
        .is_err()
    );
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            7,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
}

#[test]
fn post_node_exception_rejects_child_events_and_unrestored_registers() {
    let mut event = post_node_exception_event();
    event.dwProcessId = 9;
    let mut receipt = post_node_exception_receipt();
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
    event.dwProcessId = 7;
    receipt["registers_restored"] = serde_json::json!(false);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
    receipt["registers_restored"] = serde_json::json!(true);
    receipt["execution_context_unchanged"] = serde_json::json!(false);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
}

#[test]
fn first_non_clr_exception_remains_unread_even_with_a_valid_hresult_parameter() {
    let mut event = post_node_exception_event();
    let mut information = unsafe { event.u.Exception };
    information.ExceptionRecord.ExceptionCode =
        windows::Win32::Foundation::NTSTATUS(0x80000003u32 as i32);
    event.u.Exception = information;
    let mut receipt = post_node_exception_receipt();
    receipt["exception_code"] = serde_json::json!(0x80000003u32);
    receipt["reader_eligible"] = serde_json::json!(false);
    receipt["registers_restored"] = serde_json::json!(false);
    receipt["execution_context_unchanged"] = serde_json::json!(false);
    assert_eq!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .unwrap(),
        None
    );
    receipt["reader_eligible"] = serde_json::json!(true);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
}

#[test]
fn malformed_clr_parameters_cannot_be_replaced_with_a_synthetic_zero_hresult() {
    let mut event = post_node_exception_event();
    let mut information = unsafe { event.u.Exception };
    information.ExceptionRecord.NumberParameters = 16;
    event.u.Exception = information;
    let mut receipt = post_node_exception_receipt();
    receipt["parameter_count"] = serde_json::json!(16);
    receipt["reader_eligible"] = serde_json::json!(false);
    receipt["event_hresult"] = serde_json::Value::Null;
    assert_eq!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .unwrap(),
        None
    );
    receipt["event_hresult"] = serde_json::json!(0);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
    information.ExceptionRecord.NumberParameters = 0;
    event.u.Exception = information;
    receipt["parameter_count"] = serde_json::json!(0);
    receipt["event_hresult"] = serde_json::Value::Null;
    assert_eq!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .unwrap(),
        None
    );
}

#[test]
fn second_chance_clr_exception_is_preserved_without_becoming_reader_eligible() {
    let mut event = post_node_exception_event();
    let mut information = unsafe { event.u.Exception };
    information.dwFirstChance = 0;
    event.u.Exception = information;
    let mut receipt = post_node_exception_receipt();
    receipt["first_chance"] = serde_json::json!(0);
    receipt["reader_eligible"] = serde_json::json!(false);
    assert_eq!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .unwrap(),
        None
    );
    receipt["event_hresult"] = serde_json::json!(0x80070006u32);
    assert!(
        validate_post_node_exception_binding(
            &event,
            7,
            6,
            Uuid::from_bytes([1; 16]),
            &receipt,
            &pre_node_receipt(),
            &node_create_receipt(),
        )
        .is_err()
    );
}

#[test]
fn exception_budget_consumes_one_selection_without_consuming_post_classification() {
    let mut budget = ReturnBudget::default();
    assert!(budget.reserve(ReturnStage::PreNode, 4, false).is_ok());
    assert!(budget.reserve_exception(6, false).is_ok());
    assert!(budget.reserve_exception(7, false).is_err());
    assert!(budget.reserve(ReturnStage::PostNode, 8, false).is_ok());
    assert!(budget.reserve(ReturnStage::PostNode, 9, false).is_err());
    assert_eq!(budget.pre_node, Some(4));
    assert_eq!(budget.post_node_exception, Some(6));
    assert_eq!(budget.post_node, Some(8));
    assert_eq!(budget.last_sequence(), 8);
}

#[test]
fn exception_budget_requires_pre_return_and_precedes_post_classification() {
    let mut budget = ReturnBudget::default();
    assert!(budget.reserve_exception(6, false).is_err());
    assert!(budget.reserve(ReturnStage::PreNode, 4, false).is_ok());
    assert!(budget.reserve_exception(4, false).is_err());
    assert!(budget.reserve_exception(6, true).is_err());
    assert_eq!(budget.post_node_exception, None);
    assert!(budget.reserve(ReturnStage::PostNode, 8, false).is_ok());
    assert!(budget.reserve_exception(9, false).is_err());
    assert_eq!(budget.post_node_exception, None);
}

#[test]
fn pre_node_receipt_never_satisfies_the_post_node_gate() {
    let event = return_event(7, 1, SINGLE_STEP);
    let receipt = pre_node_receipt();
    let generation = Uuid::from_bytes([1; 16]);
    assert!(validate_pre_node_return_binding(&event, 7, 4, generation, &receipt).is_ok());
    assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
    assert!(
        validate_pre_node_return_binding(&event, 7, 4, generation, &classification_receipt())
            .is_err()
    );
}

#[test]
fn pre_node_receipt_rejects_even_null_or_zero_node_bindings() {
    let event = return_event(7, 1, SINGLE_STEP);
    for field in [
        "node_create_sequence",
        "node_process_id",
        "node_process_birth",
    ] {
        let mut receipt = pre_node_receipt();
        receipt[field] = serde_json::Value::Null;
        assert!(
            validate_pre_node_return_binding(&event, 7, 4, Uuid::from_bytes([1; 16]), &receipt)
                .is_err()
        );
        receipt[field] = serde_json::json!(0);
        assert!(
            validate_pre_node_return_binding(&event, 7, 4, Uuid::from_bytes([1; 16]), &receipt)
                .is_err()
        );
        receipt[field] = serde_json::json!(2);
        assert!(
            validate_pre_node_return_binding(&event, 7, 4, Uuid::from_bytes([1; 16]), &receipt)
                .is_err()
        );
    }
}

#[test]
fn pre_node_receipt_requires_the_original_generation_and_thread() {
    let event = return_event(7, 1, SINGLE_STEP);
    let generation = Uuid::from_bytes([1; 16]);
    let mut receipt = pre_node_receipt();
    assert!(
        validate_pre_node_return_binding(&event, 7, 4, Uuid::from_bytes([2; 16]), &receipt)
            .is_err()
    );
    receipt["identity"]["thread_id"] = serde_json::json!(9);
    assert!(validate_pre_node_return_binding(&event, 7, 4, generation, &receipt).is_err());
    receipt["identity"]["thread_id"] = serde_json::json!(8);
    receipt["identity"]["process_birth"] = serde_json::json!(0);
    assert!(validate_pre_node_return_binding(&event, 7, 4, generation, &receipt).is_err());
    receipt["identity"]["process_birth"] = serde_json::json!(100);
    receipt["identity"]["thread_birth"] = serde_json::Value::Null;
    assert!(validate_pre_node_return_binding(&event, 7, 4, generation, &receipt).is_err());
}

#[test]
fn pre_node_receipt_requires_entry_before_the_original_return_sequence() {
    let event = return_event(7, 1, SINGLE_STEP);
    for entry in [0, 4, 5] {
        let mut receipt = pre_node_receipt();
        receipt["entry_sequence"] = serde_json::json!(entry);
        assert!(
            validate_pre_node_return_binding(&event, 7, 4, Uuid::from_bytes([1; 16]), &receipt)
                .is_err()
        );
    }
    assert!(
        validate_pre_node_return_binding(
            &event,
            7,
            5,
            Uuid::from_bytes([1; 16]),
            &pre_node_receipt()
        )
        .is_err()
    );
}

#[test]
fn pre_node_reader_rejects_another_event_or_unrestored_registers() {
    let generation = Uuid::from_bytes([1; 16]);
    let mut receipt = pre_node_receipt();
    assert!(
        validate_pre_node_return_binding(
            &return_event(9, 1, SINGLE_STEP),
            7,
            4,
            generation,
            &receipt
        )
        .is_err()
    );
    assert!(
        validate_pre_node_return_binding(
            &return_event(7, 0, SINGLE_STEP),
            7,
            4,
            generation,
            &receipt
        )
        .is_err()
    );
    assert!(
        validate_pre_node_return_binding(
            &return_event(7, 1, 0xe0434352),
            7,
            4,
            generation,
            &receipt
        )
        .is_err()
    );
    receipt["registers_restored"] = serde_json::json!(false);
    assert!(
        validate_pre_node_return_binding(
            &return_event(7, 1, SINGLE_STEP),
            7,
            4,
            generation,
            &receipt
        )
        .is_err()
    );
    receipt["registers_restored"] = serde_json::json!(true);
    receipt["post_start_clr_stack_required"] = serde_json::json!(false);
    assert!(
        validate_pre_node_return_binding(
            &return_event(7, 1, SINGLE_STEP),
            7,
            4,
            generation,
            &receipt
        )
        .is_err()
    );
}

#[test]
fn pre_node_return_preserves_u64_without_accepting_inconsistent_low_bits() {
    let event = return_event(7, 1, SINGLE_STEP);
    let generation = Uuid::from_bytes([1; 16]);
    let mut receipt = pre_node_receipt();
    receipt["raw_return_u64"] = serde_json::json!(u64::MAX);
    receipt["raw_return_low32"] = serde_json::json!(u32::MAX);
    assert!(validate_pre_node_return_binding(&event, 7, 4, generation, &receipt).is_ok());
    receipt["raw_return_low32"] = serde_json::json!(0);
    assert!(validate_pre_node_return_binding(&event, 7, 4, generation, &receipt).is_err());
}

#[test]
fn reader_budget_allows_only_one_attempt_per_stage() {
    let mut budget = ReturnBudget::default();
    assert!(budget.reserve(ReturnStage::PreNode, 4, false).is_ok());
    assert!(budget.reserve(ReturnStage::PreNode, 5, false).is_err());
    assert!(budget.reserve(ReturnStage::PostNode, 8, false).is_ok());
    assert!(budget.reserve(ReturnStage::PostNode, 9, false).is_err());
    assert_eq!(budget.pre_node, Some(4));
    assert_eq!(budget.post_node, Some(8));
}

#[test]
fn reader_budget_rejects_pre_node_after_post_node_even_when_it_was_unused() {
    let mut budget = ReturnBudget::default();
    assert!(budget.reserve(ReturnStage::PostNode, 4, false).is_ok());
    assert!(budget.reserve(ReturnStage::PreNode, 5, false).is_err());
    assert_eq!(budget.pre_node, None);
}

#[test]
fn reader_budget_rejects_expired_or_nonincreasing_sequences() {
    let mut budget = ReturnBudget::default();
    assert!(budget.reserve(ReturnStage::PreNode, 0, false).is_err());
    assert!(budget.reserve(ReturnStage::PreNode, 4, true).is_err());
    assert_eq!(budget.pre_node, None);
    assert!(budget.reserve(ReturnStage::PreNode, 4, false).is_ok());
    assert!(budget.reserve(ReturnStage::PostNode, 4, false).is_err());
    assert!(budget.reserve(ReturnStage::PostNode, 3, false).is_err());
    assert!(budget.reserve(ReturnStage::PostNode, 5, true).is_err());
    assert_eq!(budget.post_node, None);
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
fn clr_reader_preserves_the_complete_native_return_without_interpreting_it() {
    let event = return_event(7, 1, SINGLE_STEP);
    for value in [0u64, 0x4550, 0x5a4d, 0x1_0000_4550, u64::MAX] {
        let mut receipt = classification_receipt();
        receipt["raw_return_u64"] = serde_json::json!(value);
        receipt["raw_return_low32"] = serde_json::json!(value as u32);
        assert!(validate_return_binding(&event, 7, 4, &receipt).is_ok());
    }
}

#[test]
fn clr_reader_rejects_missing_or_inconsistent_native_return_bits() {
    let event = return_event(7, 1, SINGLE_STEP);
    for (complete, low) in [
        (serde_json::Value::Null, serde_json::json!(0x4550)),
        (serde_json::json!(-1), serde_json::json!(u32::MAX)),
        (serde_json::json!(0x4550), serde_json::Value::Null),
        (serde_json::json!(0x4550), serde_json::json!(0x5a4d)),
        (
            serde_json::json!(0x1_0000_4550u64),
            serde_json::json!(0x1_0000_4550u64),
        ),
    ] {
        let mut receipt = classification_receipt();
        receipt["raw_return_u64"] = complete;
        receipt["raw_return_low32"] = low;
        assert!(validate_return_binding(&event, 7, 4, &receipt).is_err());
    }
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
