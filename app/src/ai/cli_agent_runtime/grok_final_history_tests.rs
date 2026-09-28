use serde_json::{Value, json};

use super::{GrokFinalOutcome, verified_final_snapshot};

fn snapshot() -> Value {
    let session = "session-uuid";
    let turn = "turn-uuid";
    json!({
        "updates": [
            {"method":"session/update","params":{"sessionId":session,
                "_meta":{"eventId":"session-uuid-1","promptId":turn},
                "update":{"sessionUpdate":"agent_message_chunk",
                    "content":{"type":"text","text":"中文第一行\n"}}}},
            {"method":"session/update","params":{"sessionId":session,
                "_meta":{"eventId":"session-uuid-2","promptId":turn},
                "update":{"sessionUpdate":"agent_message_chunk",
                    "content":{"type":"text","text":"second line"}}}},
            {"method":"_x.ai/session/update","params":{"sessionId":session,
                "_meta":{"eventId":"session-uuid-3"},
                "update":{"sessionUpdate":"turn_completed","prompt_id":turn,
                    "stop_reason":"end_turn"}}}
        ],
        "hasMore":false,"totalCount":3,"lastEventId":"session-uuid-3"
    })
}

fn verify(snapshot: &Value) -> Result<Option<(String, String)>, &'static str> {
    verified_final_snapshot(
        snapshot,
        "session-uuid",
        "turn-uuid",
        GrokFinalOutcome::Completed,
        3,
        64,
    )
}

#[test]
fn complete_history_preserves_multilingual_output_and_watermark() {
    assert_eq!(
        verify(&snapshot()).unwrap(),
        Some(("中文第一行\nsecond line".into(), "session-uuid-3".into()))
    );
}

#[test]
fn incomplete_or_wrong_turn_history_never_returns_an_answer() {
    let mut missing = snapshot();
    missing["updates"].as_array_mut().unwrap().pop();
    missing["totalCount"] = json!(2);
    missing["lastEventId"] = json!("session-uuid-2");
    assert_eq!(verify(&missing).unwrap(), None);

    let mut wrong_turn = snapshot();
    wrong_turn["updates"][2]["params"]["update"]["prompt_id"] = json!("another-turn");
    assert_eq!(verify(&wrong_turn).unwrap(), None);

    let mut truncated = snapshot();
    truncated["hasMore"] = json!(true);
    assert!(verify(&truncated).is_err());
}

#[test]
fn mismatched_completion_or_text_after_watermark_is_rejected() {
    let mut cancelled = snapshot();
    cancelled["updates"][2]["params"]["update"]["stop_reason"] = json!("cancelled");
    assert!(verify(&cancelled).is_err());
    assert!(
        verified_final_snapshot(
            &cancelled,
            "session-uuid",
            "turn-uuid",
            GrokFinalOutcome::Cancelled,
            3,
            64,
        )
        .unwrap()
        .is_some()
    );

    let mut late_text = snapshot();
    late_text["updates"].as_array_mut().unwrap().push(json!({
        "method":"session/update","params":{"sessionId":"session-uuid",
            "_meta":{"eventId":"session-uuid-4","promptId":"turn-uuid"},
            "update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"late"}}}
    }));
    late_text["totalCount"] = json!(4);
    late_text["lastEventId"] = json!("session-uuid-4");
    assert!(
        verified_final_snapshot(
            &late_text,
            "session-uuid",
            "turn-uuid",
            GrokFinalOutcome::Completed,
            4,
            64,
        )
        .is_err()
    );
}

#[test]
fn native_hook_updates_may_precede_an_earlier_numbered_answer() {
    let session = "session-uuid";
    let turn = "turn-uuid";
    // 原生 1.0.41 的磁盘历史按写入顺序排列，hook 事件 94 会排在正文 90 前。
    let auxiliary = |sequence, kind: &str| {
        json!({"method":"_x.ai/session/update","params":{"sessionId":session,
            "_meta":{"eventId":format!("{session}-{sequence}")},
            "update":{"sessionUpdate":kind}}})
    };
    let result = json!({"updates":[
        auxiliary(2,"hook_execution"),
        auxiliary(5,"background_tasks"),
        auxiliary(7,"hook_execution"),
        auxiliary(8,"user_message_chunk"),
        auxiliary(63,"agent_thought_chunk"),
        auxiliary(94,"hook_execution"),
        {"method":"session/update","params":{"sessionId":session,
            "_meta":{"eventId":"session-uuid-90","promptId":turn,"chunkId":81},
            "update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"verified answer"}}}},
        {"method":"_x.ai/session/update","params":{"sessionId":session,
            "_meta":{"eventId":"session-uuid-95"},
            "update":{"sessionUpdate":"turn_completed","prompt_id":turn,
                "stop_reason":"end_turn"}}}
    ],"totalCount":8,"hasMore":false,"lastEventId":"session-uuid-95"});
    assert_eq!(
        verified_final_snapshot(&result, session, turn, GrokFinalOutcome::Completed, 8, 64)
            .unwrap(),
        Some(("verified answer".into(), "session-uuid-95".into()))
    );

    let mut duplicate = result.clone();
    duplicate["updates"][5]["params"]["_meta"]["eventId"] = json!("session-uuid-90");
    assert!(
        verified_final_snapshot(&duplicate, session, turn, GrokFinalOutcome::Completed, 8, 64)
            .is_err()
    );
    let mut regressed = snapshot();
    regressed["updates"][0]["params"]["_meta"]["eventId"] = json!("session-uuid-91");
    regressed["updates"][1]["params"]["_meta"]["eventId"] = json!("session-uuid-90");
    regressed["updates"][2]["params"]["_meta"]["eventId"] = json!("session-uuid-95");
    regressed["lastEventId"] = json!("session-uuid-95");
    assert!(verify(&regressed).is_err());

    let mut conflicting = snapshot();
    conflicting["updates"][1]["params"]["_meta"]["eventId"] = json!("session-uuid-1");
    conflicting["updates"][1]["params"]["update"]["content"]["text"] =
        json!("different answer");
    assert!(verify(&conflicting).is_err());
}
