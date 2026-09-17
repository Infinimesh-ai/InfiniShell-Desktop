#!/usr/bin/env python3
"""生产 Claude PNG、相同ID去重、中文多行及同原生会话恢复的隔离验收。"""

import argparse
import hashlib
import json
from pathlib import Path
import re

import run_claude_adapter_live as adapter
import run_claude_image_probe as image_probe


TEST_NAME = ("ai::cli_agent_runtime::claude::live_tests::managed_image_live_tests::"
             "real_claude_managed_png_lifecycle")
SCOPE = "claude_managed_png_process_resume"
MARKER = "CLAUDE_NATIVE_PROTOCOL_IDS "
PROJECT_SETTINGS = {"permissions": {"deny": ["*"]}}
TRACE_KEYS = (image_probe.BASE_NATIVE_KEYS - {"direction"}) | {
    "runtime_generation", "image_content_projection", "result_text_sha256",
}
PHASES = ("image", "multiline", "recall")
EVENT_KEYS = {
    "acceptance_started": {"event", "scope", "max_native_inputs", "production_adapter", "credential_files_read_by_probe", "real_gui_verified", "sqlite_verified", "parent_permission_ceiling_verified", "http_request_count_verified"},
    "attachment_prepared": {"event", "image_sha256", "image_bytes", "prompt_sha256", "prompt_bytes", "block_types", "media_type", "replay_array_sha256", "attachment_hash_name", "reference_persisted", "expected_reply_sha256"},
    "session_ready": {"event", "native_session_id", "permission_mode", "native_session_association_confirmed", "filesystem_sandbox_verified", "parent_permission_ceiling_verified"},
    "managed_connection_ready": {"event", "phase", "generation", "native_session_id", "requested_native_session_id", "native_session_association_confirmed", "inputs_replayed"},
    "input_submitted": {"event", "phase", "generation", "message_id", "typed_input", "controller_send_count", "identical_message_retransmitted", "expected_reply_sha256"},
    "message_accepted": {"event", "phase", "generation", "message_id", "turn_id", "native_session_id", "receipt_source"},
    "cached_native_ack_replayed": {"event", "phase", "observed_phase", "generation", "message_id", "turn_id", "native_session_id", "receipt_source", "native_input_added"},
    "turn_started": {"event", "phase", "generation", "turn_id", "native_session_id"},
    "turn_finished": {"event", "phase", "generation", "turn_id", "native_session_id", "outcome", "full_output_sha256", "trimmed_output_sha256", "output_bytes"},
    "connection_shutdown": {"event", "native_session_id"},
    "cleanup_checked": {"event", "generation", "native_session_id", "transport_closed", "cleanup_receipt_read", "cleanup_confirmed", "normal_exit", "receipt"},
    "attachment_restore_verified": {"event", "image_sha256", "image_bytes", "typed_reference_matches", "image_replayed_to_native"},
    "acceptance_passed": {"event", "scope", "native_session_id", "native_inputs", "runtime_generations", "same_id_deduplication_verified", "durable_attachment_restore_verified", "production_adapter_verified", "native_framing_only", "app_restart_and_ui_verified", "sqlite_verified", "parent_permission_ceiling_verified", "http_request_count_verified"},
    "native_protocol_ids": {"event", "sequence", "generation", "native"},
}
_require, _uuid, _sha = image_probe._require, image_probe._uuid, image_probe._sha


def project_protocol_output(output):
    # 只接收生产读取处已投影的固定字段；原生正文、环境、思考及base64一概拒绝。
    projected = []
    for line in output.splitlines():
        if MARKER not in line:
            continue
        _require(line.count(MARKER) == 1)
        value = json.loads(line.split(MARKER, 1)[1])
        image_probe._keys(value, TRACE_KEYS)
        _uuid(value["runtime_generation"])
        _require(value["tools"] == [] and value["tool_use_id"] is None)
        for key in ("uuid", "session_id", "message_id", "user_message_uuid", "command_uuid",
                    "request_id", "response_request_id"):
            item = value[key]
            _require(item is None or isinstance(item, str) and len(item) <= 160
                     and re.fullmatch(r"[A-Za-z0-9_-]+|sha256:[0-9a-f]{64}", item))
        for key in ("type", "subtype", "state", "request_subtype", "terminal_reason", "response_subtype"):
            _require(value[key] is None or isinstance(value[key], str)
                     and re.fullmatch(r"[a-z_]{1,64}", value[key]))
        _require(value["is_error"] is None or type(value["is_error"]) is bool)
        inputs = value["user_message_uuids"]
        _require(inputs is None or isinstance(inputs, list) and len(inputs) == 1)
        if inputs is not None:
            _uuid(inputs[0])
        if value["result_text_sha256"] is not None:
            _sha(value["result_text_sha256"])
            _require(value["type"] == "result")
        image = value["image_content_projection"]
        if image is not None:
            image_probe._keys(image, {"array_sha256", "text_sha256", "text_bytes", "image_sha256",
                                      "image_bytes", "block_types", "media_type"})
            _require(value["type"] == "user" and image["block_types"] == ["text", "image"]
                     and image["media_type"] == "image/png")
            for key in ("array_sha256", "text_sha256", "image_sha256"):
                _sha(image[key])
            image_probe._integer(image["text_bytes"], 1, 1024 * 1024)
            image_probe._integer(image["image_bytes"], 1, 1024 * 1024)
        projected.append({"event": "native_protocol_ids", "sequence": len(projected) + 1,
                          "generation": value.pop("runtime_generation"), "native": value})
        _require(len(projected) <= 512)
    return projected


def audit_events(events):
    _require(isinstance(events, list) and 25 <= len(events) <= 560)
    _require(all(isinstance(e, dict) and e.get("event") in EVENT_KEYS for e in events))
    for event in events:
        image_probe._keys(event, EVENT_KEYS[event["event"]])
    def selected(kind, **fields):
        return [e for e in events if e["event"] == kind and all(e.get(k) == v for k, v in fields.items())]
    def one(kind, **fields):
        found = selected(kind, **fields)
        _require(len(found) == 1)
        return found[0]
    beginning, ending = one("acceptance_started"), one("acceptance_passed")
    _require(events[0] is beginning and beginning["scope"] == ending["scope"] == SCOPE)
    _require(beginning["max_native_inputs"] == ending["native_inputs"] == 3
             and ending["runtime_generations"] == 2)
    image_probe._flags(beginning, true=("production_adapter",), false=("credential_files_read_by_probe",
                       "real_gui_verified", "sqlite_verified", "parent_permission_ceiling_verified",
                       "http_request_count_verified"))
    image_probe._flags(ending, true=("same_id_deduplication_verified", "durable_attachment_restore_verified",
                       "production_adapter_verified"), false=("native_framing_only", "app_restart_and_ui_verified",
                       "sqlite_verified", "parent_permission_ceiling_verified", "http_request_count_verified"))
    native_id = _uuid(ending["native_session_id"])
    original, resumed = one("managed_connection_ready", phase="new"), one("managed_connection_ready", phase="resume")
    generations = [_uuid(original["generation"]), _uuid(resumed["generation"])]
    _require(generations[0] != generations[1] and type(original["inputs_replayed"]) is int
             and type(resumed["inputs_replayed"]) is int and original["inputs_replayed"] == resumed["inputs_replayed"] == 0
             and original["native_session_id"] in {None, native_id} and resumed["native_session_id"] in {None, native_id}
             and original["requested_native_session_id"] is None and resumed["requested_native_session_id"] == native_id)
    # 启动就绪不补原生ID；显式下一输入的真实ACK、开始和结果仍必须确认同一历史会话。
    for ready in (original, resumed):
        _require(type(ready["native_session_association_confirmed"]) is bool
                 and ready["native_session_association_confirmed"] is (ready["native_session_id"] is not None))
    prepared, restored = one("attachment_prepared"), one("attachment_restore_verified")
    image_sha = _sha(prepared["image_sha256"])
    _require(prepared["image_bytes"] == restored["image_bytes"] == 12420
             and restored["image_sha256"] == image_sha and prepared["block_types"] == ["text", "image"]
             and prepared["media_type"] == "image/png" and prepared["attachment_hash_name"] == image_sha + ".png")
    image_probe._flags(prepared, true=("reference_persisted",))
    image_probe._flags(restored, true=("typed_reference_matches",), false=("image_replayed_to_native",))
    _sha(prepared["prompt_sha256"])
    _require(prepared["prompt_sha256"] == "356737d1060e0761b76a4b7cb20ffb253b4b3d0209229def05c63e0d150f0c06"
             and type(prepared["prompt_bytes"]) is int and prepared["prompt_bytes"] == 310)
    input_ids, output_hashes = [], []
    for index, phase in enumerate(PHASES):
        submitted, accepted, started, finished = [one(kind, phase=phase) for kind in
                                                 ("input_submitted", "message_accepted", "turn_started", "turn_finished")]
        expected_generation = generations[0 if index < 2 else 1]
        message_id = _uuid(submitted["message_id"])
        input_ids.append(message_id)
        _require(all(e["generation"] == expected_generation for e in (submitted, accepted, started, finished)))
        _require(accepted["message_id"] == accepted["turn_id"] == started["turn_id"] == finished["turn_id"] == message_id
                 and accepted["receipt_source"] == "NativeProtocol" and finished["outcome"] == "Completed")
        _require(all(e["native_session_id"] == native_id for e in (accepted, started, finished)))
        _require(submitted["typed_input"] is True and type(submitted["controller_send_count"]) is int
                 and submitted["controller_send_count"] == (2 if index == 0 else 1)
                 and submitted["identical_message_retransmitted"] is (index == 0))
        _require(events.index(submitted) < events.index(accepted) < events.index(started) < events.index(finished))
        _require(_sha(finished["trimmed_output_sha256"]) == _sha(submitted["expected_reply_sha256"]))
        output_hashes.append(_sha(finished["full_output_sha256"]))
        image_probe._integer(finished["output_bytes"], 1, 1024 * 1024)
    _require(len(set(input_ids)) == 3)
    _require(one("input_submitted", phase="image")["expected_reply_sha256"] == prepared["expected_reply_sha256"]
             == one("input_submitted", phase="recall")["expected_reply_sha256"])
    text_hash = hashlib.sha256(b"CLAUDE_PNG_TEXT").hexdigest()
    _require(one("input_submitted", phase="multiline")["expected_reply_sha256"] == text_hash)
    for kind in ("input_submitted", "message_accepted", "turn_started", "turn_finished"):
        _require(len(selected(kind)) == 3)
    cached = selected("cached_native_ack_replayed")
    _require(len(cached) <= 1)
    for replay in cached:
        _require(replay["phase"] == "image" and replay["observed_phase"] in {"image", "multiline"}
                 and replay["generation"] == generations[0] and replay["native_session_id"] == native_id
                 and replay["message_id"] == replay["turn_id"] == input_ids[0]
                 and replay["receipt_source"] == "CachedNativeProtocol" and replay["native_input_added"] is False
                 and events.index(one("message_accepted", phase="image")) < events.index(replay)
                 < events.index(one("turn_finished", phase=replay["observed_phase"])))
        if replay["observed_phase"] == "multiline":
            _require(events.index(one("input_submitted", phase="multiline")) < events.index(replay))
    _require(len(selected("managed_connection_ready")) == len(selected("connection_shutdown"))
             == len(selected("cleanup_checked")) == 2)
    for generation in generations:
        cleanup = one("cleanup_checked", generation=generation)
        image_probe._flags(cleanup, true=("transport_closed", "cleanup_receipt_read", "cleanup_confirmed", "normal_exit"))
        receipt = cleanup["receipt"]
        image_probe._keys(receipt, {"version", "generation", "cleanup_confirmed", "containment", "exit_reason", "exit_code"})
        _require(type(receipt["version"]) is int and receipt["version"] == 1 and receipt["generation"] == generation
                 and receipt["cleanup_confirmed"] is True and receipt["exit_reason"] == "stdio_closed"
                 and type(receipt["exit_code"]) is int and receipt["exit_code"] == 0
                 and receipt["containment"] in {"macos_resource_coalition", "linux_subtree", "windows_job", "unix_process_group"}
                 and cleanup["native_session_id"] == native_id)
    _require(all(e["native_session_id"] == native_id for e in selected("connection_shutdown")))
    _require(events.index(one("turn_finished", phase="multiline")) < events.index(one("cleanup_checked", generation=generations[0]))
             < events.index(restored) < events.index(resumed) < events.index(one("input_submitted", phase="recall"))
             < events.index(one("cleanup_checked", generation=generations[1])) < events.index(ending))
    for ready in selected("session_ready"):
        _require(ready["permission_mode"] == "default" and ready["native_session_id"] in {None, native_id})
        _require(ready["native_session_association_confirmed"] is (ready["native_session_id"] is not None))
        image_probe._flags(ready, false=("filesystem_sandbox_verified", "parent_permission_ceiling_verified"))
    protocol = selected("native_protocol_ids")
    _require(9 <= len(protocol) <= 512)
    ledger = []
    for number, event in enumerate(protocol, 1):
        _require(type(event["sequence"]) is int and event["sequence"] == number and event["generation"] in generations)
        row = event["native"]
        image_probe._keys(row, TRACE_KEYS - {"runtime_generation"})
        _require(row["tools"] == [] and row["tool_use_id"] is None and row["type"] in {
            "user", "assistant", "result", "system", "command_lifecycle", "control_response",
            "stream_event", "keep_alive", "rate_limit_event"})
        if row["type"] in {"user", "assistant", "result", "system", "command_lifecycle"}:
            _require(row["session_id"] == native_id)
        if row["type"] == "control_response":
            _require(row["response_subtype"] == "success" and isinstance(row["response_request_id"], str)
                     and re.fullmatch(f"infinishell-{event['generation']}-[0-9]+", row["response_request_id"]))
        if row["type"] != "user":
            _require(row["image_content_projection"] is None)
        if row["type"] != "result":
            _require(row["result_text_sha256"] is None)
        ledger.append((event["generation"], row))
    _require(len([row for _, row in ledger if row["type"] == "user"]) == 3
             and len([row for _, row in ledger if row["type"] == "result"]) == 3)
    for index, message_id in enumerate(input_ids):
        generation = generations[0 if index < 2 else 1]
        def native_selected(kind, **fields):
            return [(i, row) for i, (g, row) in enumerate(ledger) if g == generation and row["type"] == kind
                    and all(row[k] == v for k, v in fields.items())]
        users = native_selected("user", uuid=message_id)
        starts = native_selected("command_lifecycle", command_uuid=message_id, state="started")
        queued = native_selected("command_lifecycle", command_uuid=message_id, state="queued")
        results = native_selected("result", user_message_uuid=message_id)
        assistants = native_selected("assistant", user_message_uuid=message_id)
        _require(len(users) == len(starts) == len(results) == 1 and len(queued) <= 1)
        _require(assistants and all(starts[0][0] < position < results[0][0] for position, _ in assistants))
        _require(starts[0][0] < users[0][0] < results[0][0] and (not queued or queued[0][0] < starts[0][0]))
        result = results[0][1]
        _require(result["user_message_uuids"] == [message_id] and result["is_error"] is False
                 and result["subtype"] == "success" and result["terminal_reason"] == "completed"
                 and result["result_text_sha256"] == output_hashes[index])
        _uuid(result["uuid"])
        image = users[0][1]["image_content_projection"]
        if index == 0:
            _require(isinstance(image, dict) and image["image_sha256"] == image_sha
                     and image["image_bytes"] == prepared["image_bytes"] and image["text_sha256"] == prepared["prompt_sha256"]
                     and image["text_bytes"] == prepared["prompt_bytes"] and image["block_types"] == ["text", "image"]
                     and image["media_type"] == "image/png")
            _require(_sha(image["array_sha256"]) == _sha(prepared["replay_array_sha256"]))
        else:
            _require(image is None)
    for generation, row in ledger:
        if row["type"] == "command_lifecycle":
            _require(row["command_uuid"] in input_ids and row["state"] in {"queued", "started", "completed"})
            index = input_ids.index(row["command_uuid"])
            _require(generation == generations[0 if index < 2 else 1])
            completed = [r for g, r in ledger if g == generation and r["type"] == "command_lifecycle"
                         and r["command_uuid"] == row["command_uuid"] and r["state"] == "completed"]
            _require(len(completed) <= 1)
        if row["type"] == "assistant":
            _uuid(row["uuid"])
            _require(row["user_message_uuid"] in input_ids)
            index = input_ids.index(row["user_message_uuid"])
            _require(generation == generations[0 if index < 2 else 1]
                     and row["user_message_uuids"] in (None, [row["user_message_uuid"]]))
    _require(len({row["uuid"] for _, row in ledger if row["type"] == "result"}) == 3)
    assistant_ids = [row["uuid"] for _, row in ledger if row["type"] == "assistant"]
    _require(len(set(assistant_ids)) == len(assistant_ids))
    _require(all(any(g == generation and row["type"] == "control_response"
                     and row["response_request_id"] == f"infinishell-{generation}-1" for g, row in ledger)
                 for generation in generations))
    return {"native_session_id": native_id, "generations": generations, "input_ids": input_ids,
            "native_inputs": 3, "image_sha256": image_sha, "image_replay_array_sha256":
            next(row["image_content_projection"]["array_sha256"] for _, row in ledger
                 if row["type"] == "user" and row["uuid"] == input_ids[0]),
            "cached_native_ack_replay_count": len(cached), "same_id_deduplication_verified": True, "durable_attachment_restore_verified": True,
            "production_adapter_verified": True, "app_restart_and_ui_verified": False, "sqlite_verified": False}


def verified_acceptance(exit_code, output, events):
    if type(exit_code) is not int or exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    try:
        audit_events(events + project_protocol_output(output))
    except (ValueError, TypeError, KeyError, AttributeError, IndexError):
        return False
    return True


def compact_stdout(path):
    if not path.exists():
        return None
    raw = path.read_bytes()
    patterns = {
        "api_key": rb"(?<![A-Za-z0-9_-])(?:sk-[A-Za-z0-9_-]{16,}|xai-[A-Za-z0-9_-]{16,})",
        "jwt": rb"eyJ[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}",
        "bearer": rb"(?i)Bearer\s+[A-Za-z0-9._~+/=-]{16,}",
        "private_key": rb"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----",
        "credential_assignment": rb"(?i)(?:api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password)\s*[=:]\s*[\"']?[A-Za-z0-9._~+/=-]{16,}",
        "email": rb"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
    }
    proof = {"raw_stdout_archived": False, "sanitized_stdout_sha256": hashlib.sha256(raw).hexdigest(),
             "sanitized_stdout_bytes": len(raw), "hash_boundary": "base运行器脱敏后的stdout，不是原始原生帧",
             "credential_shape_counts": {key: len(re.findall(pattern, raw)) for key, pattern in patterns.items()},
             "test_success_summary_count": len(re.findall(rb"test result: ok\. 1 passed; 0 failed; 0 ignored;", raw))}
    path.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n")
    return proof


def run(args):
    if args.max_native_inputs != 3 or args.timeout_seconds != 900:
        raise ValueError("生产图片验收固定最多3次原生输入和900秒外层时限")
    original_environment = adapter.authenticated_environment
    def environment(*values):
        result = original_environment(*values)
        result.update({"INFINISHELL_CLAUDE_MANAGED_IMAGE_TRACE": "1",
                       "INFINISHELL_CLAUDE_MANAGED_IMAGE_MAX_NATIVE_INPUTS": "3"})
        return result
    def verify(exit_code, output, events):
        passed = verified_acceptance(exit_code, output, events)
        try:
            projected = project_protocol_output(output)
        except (ValueError, TypeError, KeyError, AttributeError, IndexError):
            return False
        # 失败也保留严格白名单投影，stdout压缩后仍能审计实际协议；不补成功结论。
        events.extend(projected)
        args.output.write_text("".join(json.dumps(e, ensure_ascii=False) + "\n" for e in events), encoding="utf-8")
        return passed
    replacements = {"TEST_NAME": TEST_NAME, "SCOPE": SCOPE, "PROJECT_SETTINGS": PROJECT_SETTINGS,
                    "verified_acceptance": verify, "audit_events": audit_events}
    originals = {key: getattr(image_probe, key) for key in replacements}
    transcript = args.output.with_suffix(".test-output.txt")
    transcript_preexisted = transcript.exists()
    stdout_proof = None
    adapter.authenticated_environment = environment
    try:
        for key, value in replacements.items():
            setattr(image_probe, key, value)
        result = image_probe.run(args)
    finally:
        adapter.authenticated_environment = original_environment
        for key, value in originals.items():
            setattr(image_probe, key, value)
        # 异常也清理本轮新建的stdout产物；预先存在的输出仍由既有校验拒绝且不改写。
        if not transcript_preexisted:
            stdout_proof = compact_stdout(transcript)
    metadata_path = args.output.with_suffix(".metadata.json")
    metadata = json.loads(metadata_path.read_text())
    metadata.update({"scope": SCOPE, "max_native_inputs": 3, "timeout_seconds": 900,
                     "native_framing_only": False, "managed_image_adapter_verified": False,
                     "durable_attachment_restore_verified": False, "sqlite_verified": False,
                     "app_restart_and_ui_verified": False, "parent_permission_ceiling_verified": False,
                     "http_request_count_verified": False})
    # 裸原生校准的门禁字段不能替代生产路径与本次验收结论。
    for key in ("native_image_proof", "native_image_input_verified", "production_image_gate_open", "persistence_verified"):
        metadata.pop(key, None)
    if metadata.get("acceptance_passed") is True:
        try:
            events = [json.loads(line) for line in args.output.read_text().splitlines()]
            metadata["managed_image_proof"] = audit_events(events)
            metadata["managed_image_adapter_verified"] = True
            metadata["durable_attachment_restore_verified"] = True
        except (ValueError, TypeError, KeyError, AttributeError, IndexError):
            metadata["acceptance_passed"] = False
            metadata["runner_error"] = "生产PNG公开证据复核失败"
            result = 1
    if stdout_proof is not None:
        metadata["stdout_proof"] = stdout_proof
        if any(stdout_proof["credential_shape_counts"].values()):
            metadata["acceptance_passed"] = False
            metadata["managed_image_adapter_verified"] = False
            metadata["durable_attachment_restore_verified"] = False
            metadata["runner_error"] = "脱敏stdout凭据形态扫描未通过"
            result = 1
    metadata_path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--api-environment-file", type=Path, required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-native-inputs", type=int, choices=[3], default=3)
    parser.add_argument("--timeout-seconds", type=int, choices=[900], default=900)
    args = parser.parse_args()
    try:
        parser.exit(run(args))
    except (OSError, ValueError, TypeError, KeyError, AttributeError):
        parser.exit(2, "生产Claude图片运行器启动失败；请检查显式私有参数。\n")


if __name__ == "__main__":
    main()
