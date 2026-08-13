use super::*;

#[test]
fn reasoning_item_signature_roundtrips_unknown_fields() {
	let item = serde_json::json!({
		"type": "reasoning",
		"id": "rs_1",
		"encrypted_content": "opaque",
		"future_field": { "kept": true }
	});

	let signature = reasoning_item_signature(&item).unwrap();

	assert_eq!(reasoning_item_from_signature(&signature), Some(item));
}
