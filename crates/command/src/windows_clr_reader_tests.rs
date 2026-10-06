use super::*;

fn binding() -> Value {
    json!({"operation":1,"nonce":"0123456789abcdef0123456789abcdef","event_sequence":4,"pid":10,
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

fn native_return_binding() -> Value {
    let mut original = binding();
    original["operation"] = json!(3);
    original["event_hresult"] = json!(0);
    original
}

fn native_return_reply() -> Value {
    let mut value = reply();
    value["operation"] = json!(3);
    value["event_hresult"] = json!(0);
    value["exception_source"] = json!("none");
    value["exception_api_hresult"] = json!(0x8000000a_u32);
    value["exception_state_flags"] = json!(0);
    value["tracker_complete"] = json!(false);
    value["object_chain_complete"] = json!(false);
    value["chain"] = json!([]);
    value["frames"] = json!([]);
    value["stack_api_hresult"] = json!(1);
    value["budget_exhausted"] = json!(false);
    value
}

#[test]
fn native_return_stop_requires_its_own_first_chance_single_step() {
    assert!(valid_stop_kind(3, 0x80000004, 1, 0));
    assert!(!valid_stop_kind(3, 0xe0434352, 1, 0));
    assert!(!valid_stop_kind(3, 0x80000003, 1, 0));
    assert!(!valid_stop_kind(3, 0x80000004, 0, 0));
    assert!(!valid_stop_kind(3, 0x80000004, 1, 0x80070005));
    assert!(!valid_stop_kind(1, 0x80000004, 1, 0));
    assert!(valid_stop_kind(1, 0xe0434352, 1, 0x80070005));
    assert!(!valid_stop_kind(2, 0x80000004, 1, 0));
    assert!(valid_stop_kind(4, 0x80000004, 1, 0));
    assert!(!valid_stop_kind(4, 0xe0434352, 1, 0));
}

#[test]
fn native_return_reply_requires_the_requested_operation() {
    assert!(validate_reply(&native_return_reply(), &native_return_binding()).is_ok());
    assert!(validate_reply(&native_return_reply(), &binding()).is_err());
    assert!(validate_reply(&reply(), &native_return_binding()).is_err());
    let mut original = native_return_binding();
    original.as_object_mut().unwrap().remove("operation");
    assert!(validate_reply(&native_return_reply(), &original).is_err());
    original["operation"] = json!(2);
    assert!(validate_reply(&native_return_reply(), &original).is_err());
}

#[test]
fn native_return_reply_rejects_exception_observation_fields() {
    for (field, invalid) in [
        ("event_hresult", json!(0x80070005_u32)),
        ("exception_source", json!("last_thrown_object_candidate")),
        ("exception_api_hresult", json!(0)),
        ("exception_state_flags", json!(2)),
        ("tracker_complete", json!(true)),
        ("object_chain_complete", json!(true)),
        ("chain", json!([{"type":"System.Exception"}])),
        ("chain", Value::Null),
        ("frames", Value::Null),
    ] {
        let mut value = native_return_reply();
        value[field] = invalid;
        assert!(
            validate_reply(&value, &native_return_binding()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn native_return_observed_requires_a_complete_nonempty_stack() {
    let mut value = native_return_reply();
    value["status"] = json!("observed");
    assert!(validate_reply(&value, &native_return_binding()).is_err());
    value["stack_api_hresult"] = json!(0);
    assert!(validate_reply(&value, &native_return_binding()).is_err());
    value["frames"] = json!([{"method_token":100663297}]);
    assert!(validate_reply(&value, &native_return_binding()).is_ok());
    value["budget_exhausted"] = json!(true);
    assert!(validate_reply(&value, &native_return_binding()).is_err());
    value["status"] = json!("partial");
    assert!(validate_reply(&value, &native_return_binding()).is_ok());
}

fn managed_spec() -> ClrManagedMethodSpec {
    ClrManagedMethodSpec {
        module_mvid: "00112233-4455-6677-8899-aabbccddeeff".into(),
        method_token: 0x06001270,
        approved_il_offsets: vec![0x230, 0x232, 0x293, 0x294],
        bool_local_index: 0,
        start_info_local_index: Some(5),
    }
}

fn private_managed_target() -> Value {
    json!({"process_id":10,"thread_id":11,"process_birth":100,"thread_birth":101,"pre_sequence":4,
        "pre_nonce":"0123456789abcdef0123456789abcdef","module_mvid":"00112233-4455-6677-8899-aabbccddeeff",
        "method_token":0x06001270_u32,"il_offset":0x230,"enc_version":1,"frame_rsp":0x80000,
        "extent_start":0x10000,"extent_end":0x11000,"address":0x10200,"map_count":8,
        "map_sha256":"0101010101010101010101010101010101010101010101010101010101010101",
        "code_sha256":"0202020202020202020202020202020202020202020202020202020202020202"})
}

fn managed_target_fixture() -> ManagedContinuationTarget {
    managed_target(
        &private_managed_target(),
        &native_return_binding(),
        &managed_spec(),
    )
    .unwrap()
}

fn managed_observation() -> Value {
    json!({"bound":true,"api_hresult":0,"mapping":managed_target_fixture().safe_evidence(),
        "boolean_local":{"index":0,"api_hresult":0,"get_local_hresult":0,"locations_hresult":0,"locations":1,
            "type_hresult":0,"type_name_hresult":0,"type_name":"System.Boolean","flags_hresult":0,"flags":1,
            "size_hresult":0,"size":1,"bytes_hresult":0,"bytes_read":1,"value":true},
        "start_info_field":{"requested":true,"index":5,"api_hresult":0,"local_api_hresult":0,
            "local_get_local_hresult":0,"local_locations_hresult":0,"local_locations":1,"local_type_hresult":0x8000000a_u32,
            "local_type_name_hresult":0x8000000a_u32,"local_flags_hresult":0,"local_flags":16,"local_size_hresult":0,
            "local_size":8,"local_bytes_hresult":0,"local_bytes_read":8,"module_mvid":"00112233-4455-6677-8899-aabbccddeeff",
            "type_token":0x02000001_u32,"field_token":0x04000001_u32,"field_name":"useShellExecute",
            "field_type":2,"field_sig_type":2,"field_read_hresult":0,"value":false}})
}

#[test]
fn managed_spec_rejects_ambiguous_or_unbounded_method_selection() {
    assert!(managed_spec().validate().is_ok());
    let mut spec = managed_spec();
    spec.approved_il_offsets = vec![0x230, 0x230];
    assert!(spec.validate().is_err());
    spec.approved_il_offsets = vec![0, 1, 2, 3, 4];
    assert!(spec.validate().is_err());
    spec.approved_il_offsets = vec![0xffff_fffd];
    assert!(spec.validate().is_err());
    spec = managed_spec();
    spec.module_mvid = "00000000-0000-0000-0000-000000000000".into();
    assert!(spec.validate().is_err());
    spec = managed_spec();
    spec.method_token = 0x06000000;
    assert!(spec.validate().is_err());
    spec = managed_spec();
    spec.start_info_local_index = Some(0);
    assert!(spec.validate().is_err());
}

#[test]
fn managed_wire_preserves_v1_and_has_a_fixed_v2_private_extension() {
    let bytes = managed_extension(&managed_spec(), None).unwrap();
    assert_eq!(bytes.len(), 200);
    assert_eq!(&bytes[..36], b"00112233-4455-6677-8899-aabbccddeeff");
    assert_eq!(&bytes[40..44], &4_u32.to_le_bytes());
    assert_eq!(&bytes[68..], &[0; 132]);
    let target = managed_target_fixture();
    let bound = managed_extension(&managed_spec(), Some(&target)).unwrap();
    assert_eq!(&bound[68..76], &4_u64.to_le_bytes());
    assert_eq!(&bound[124..132], &0x10200_u64.to_le_bytes());
    let original = request(3, [1; 16], Instant::now() + Duration::from_secs(1)).unwrap();
    assert_eq!(original.len(), 168);
    assert_eq!(&original[8..12], &1_u32.to_le_bytes());
}

#[test]
fn managed_private_target_rejects_replaced_identity_or_unapproved_il() {
    for field in [
        "process_id",
        "thread_id",
        "process_birth",
        "thread_birth",
        "pre_sequence",
        "method_token",
        "il_offset",
    ] {
        let mut value = private_managed_target();
        value[field] = json!(0);
        assert!(
            managed_target(&value, &native_return_binding(), &managed_spec()).is_err(),
            "{field}"
        );
    }
    let mut value = private_managed_target();
    value["pre_nonce"] = json!("1123456789abcdef0123456789abcdef");
    assert!(managed_target(&value, &native_return_binding(), &managed_spec()).is_err());
}

#[test]
fn managed_private_target_rejects_outside_or_truncated_code_evidence() {
    for (field, invalid) in [
        ("address", json!(0x11000)),
        ("extent_end", json!(0x30000)),
        ("frame_rsp", json!(0x80001)),
        ("map_count", json!(257)),
        ("map_sha256", json!("00")),
        ("code_sha256", Value::Null),
    ] {
        let mut value = private_managed_target();
        value[field] = invalid;
        assert!(
            managed_target(&value, &native_return_binding(), &managed_spec()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn managed_target_safe_evidence_and_debug_do_not_contain_private_addresses() {
    let target = managed_target_fixture();
    let value = target.safe_evidence();
    assert_eq!(value["extent_length"], 4096);
    assert_eq!(value["pre_sequence"], 4);
    for key in [
        "address",
        "frame_rsp",
        "extent_start",
        "extent_end",
        "pre_nonce",
    ] {
        assert!(value.get(key).is_none());
    }
    assert_eq!(
        format!("{target:?}"),
        "ManagedContinuationTarget { 私有地址已隐藏 }"
    );
}

#[test]
fn managed_observation_requires_readable_boolean_not_an_empty_success_value() {
    let target = managed_target_fixture();
    assert!(validate_managed_observation(&managed_observation(), &target).is_ok());
    for (field, invalid) in [
        ("locations", json!(0)),
        ("size", json!(8)),
        ("bytes_read", json!(0)),
        ("type_name", json!("System.UInt64")),
        ("flags", json!(16)),
        ("get_local_hresult", json!(0x80004001_u32)),
        ("value", Value::Null),
    ] {
        let mut value = managed_observation();
        value["boolean_local"][field] = invalid;
        assert!(
            validate_managed_observation(&value, &target).is_err(),
            "{field}"
        );
    }
}

#[test]
fn managed_unknown_values_remain_null_without_becoming_false() {
    let target = managed_target_fixture();
    let mut value = managed_observation();
    value["boolean_local"]["value"] = Value::Null;
    value["boolean_local"]["api_hresult"] = json!(0x80004002_u32);
    value["boolean_local"]["locations"] = json!(0);
    value["start_info_field"]["value"] = Value::Null;
    value["start_info_field"]["api_hresult"] = json!(0x80004002_u32);
    assert!(validate_managed_observation(&value, &target).is_ok());
    assert!(value["boolean_local"]["value"].is_null());
    assert!(value["start_info_field"]["value"].is_null());
}

#[test]
fn managed_field_value_requires_real_reference_and_metadata() {
    let target = managed_target_fixture();
    for (field, invalid) in [
        ("local_size", json!(4)),
        ("local_locations", json!(0)),
        ("local_type_hresult", json!(0)),
        ("local_type_name_hresult", json!(0)),
        ("field_type", json!(8)),
        ("field_sig_type", json!(8)),
        ("field_token", json!(0x04000000_u32)),
        ("module_mvid", json!("unknown")),
        ("field_read_hresult", json!(0x80004002_u32)),
    ] {
        let mut value = managed_observation();
        value["start_info_field"][field] = invalid;
        assert!(
            validate_managed_observation(&value, &target).is_err(),
            "{field}"
        );
    }
}

#[test]
fn managed_stop_without_exact_mapping_cannot_publish_any_local_value() {
    let target = managed_target_fixture();
    let mut value = managed_observation();
    value["mapping"]["enc_version"] = json!(2);
    assert!(validate_managed_observation(&value, &target).is_err());
    value = managed_observation();
    value["bound"] = json!(false);
    assert!(validate_managed_observation(&value, &target).is_err());
    let unknown = json!({"bound":false,"api_hresult":0x80004002_u32,"mapping":null,"boolean_local":null,"start_info_field":null});
    assert!(validate_managed_observation(&unknown, &target).is_ok());
}
