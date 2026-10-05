use super::*;

fn binding() -> Value {
    json!({"nonce":"0123456789abcdef0123456789abcdef","event_sequence":4,"pid":10,
        "tid":11,"process_birth":100,"thread_birth":101,"event_hresult":0x80070005_u32,"read_budget_bytes":FIXTURE_READ_BYTES})
}

fn reply() -> Value {
    let mut value = binding();
    for (key, item) in [
        ("schema", json!(1)),
        ("operation", json!(1)),
        ("status", json!("partial")),
        ("target_identity_verified", json!(true)),
        ("dac_sha256_verified", json!(true)),
        ("dac_loaded", json!(true)),
        ("read_bytes", json!(4096)),
        ("read_calls", json!(16)),
    ] {
        value[key] = item;
    }
    value
}

#[test]
fn reply_preserves_partial_observation_with_exact_original_stop() {
    let value = reply();
    assert!(validate_reply(&value, &binding()).is_ok());
    assert_eq!(value["status"], "partial");
}

#[test]
fn reply_rejects_matching_ids_with_replaced_birth_or_stop() {
    for field in [
        "nonce",
        "event_sequence",
        "pid",
        "tid",
        "process_birth",
        "thread_birth",
        "event_hresult",
    ] {
        let mut value = reply();
        value[field] = json!(0);
        assert!(validate_reply(&value, &binding()).is_err(), "{field}");
    }
}

#[test]
fn reply_rejects_missing_original_binding() {
    let mut original = binding();
    original.as_object_mut().unwrap().remove("thread_birth");
    let mut value = reply();
    value.as_object_mut().unwrap().remove("thread_birth");
    assert!(validate_reply(&value, &original).is_err());
}

#[test]
fn reply_rejects_unverified_target_or_dac() {
    for field in [
        "target_identity_verified",
        "dac_sha256_verified",
        "dac_loaded",
    ] {
        let mut value = reply();
        value[field] = json!(false);
        assert!(validate_reply(&value, &binding()).is_err(), "{field}");
    }
}

#[test]
fn reply_rejects_another_wire_operation_or_version() {
    for field in ["schema", "operation"] {
        let mut value = reply();
        value[field] = json!(2);
        assert!(validate_reply(&value, &binding()).is_err());
    }
}

#[test]
fn poll_slice_expiry_is_not_the_original_deadline() {
    assert_eq!(poll_wait(Duration::ZERO), None);
    assert_eq!(poll_wait(Duration::from_nanos(1)), Some(1));
    assert_eq!(poll_wait(Duration::from_millis(25)), Some(25));
    assert_eq!(poll_wait(Duration::from_secs(30)), Some(100));
}

#[test]
fn only_fixture_and_powershell_read_budgets_are_accepted() {
    assert!(valid_read_budget(16 * 1024 * 1024));
    assert!(valid_read_budget(24 * 1024 * 1024));
    for bytes in [0, 4 * 1024 * 1024, 16 * 1024 * 1024 + 1, 32 * 1024 * 1024] {
        assert!(!valid_read_budget(bytes));
    }
}

#[test]
fn reply_accepts_known_incomplete_states_and_rejects_unknown_status() {
    for status in ["observed", "partial", "unavailable"] {
        let mut value = reply();
        value["status"] = json!(status);
        assert!(validate_reply(&value, &binding()).is_ok());
        assert_eq!(value["status"], status);
    }
    for status in [Value::Null, json!(true), json!("success")] {
        let mut value = reply();
        value["status"] = status;
        assert!(validate_reply(&value, &binding()).is_err());
    }
}

#[test]
fn reply_rejects_unknown_or_exceeded_original_read_limits() {
    for (field, invalid) in [
        ("read_bytes", json!(FIXTURE_READ_BYTES + 1)),
        ("read_calls", json!(8193)),
        ("read_bytes", json!(-1)),
        ("read_calls", json!(1.5)),
        ("read_bytes", Value::Null),
        ("read_calls", json!("16")),
    ] {
        let mut value = reply();
        value[field] = invalid;
        assert!(validate_reply(&value, &binding()).is_err());
    }
    let mut value = reply();
    value["read_bytes"] = json!(POWERSHELL_READ_BYTES);
    value["read_calls"] = json!(8192);
    assert!(validate_reply(&value, &binding()).is_err());
    let mut original = binding();
    original["read_budget_bytes"] = json!(POWERSHELL_READ_BYTES);
    assert!(validate_reply(&value, &original).is_ok());
    original["read_budget_bytes"] = json!(32 * 1024 * 1024);
    assert!(validate_reply(&value, &original).is_err());
}
