#!/usr/bin/env python3
"""一次原生 Claude 图片校准；凭据只传给隔离原生进程，不开放生产图片门禁。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import uuid

import run_claude_adapter_live as adapter


TEST_NAME = "ai::cli_agent_runtime::claude::live_tests::image_probe_live_tests::real_claude_image_input_probe"
SCOPE = "claude_native_image_input_probe"
PROJECT_SETTINGS = {"permissions": {"deny": ["*"]}}
EMPTY_SHA = hashlib.sha256(b"").hexdigest()
BASE_NATIVE_KEYS = {
    "direction", "type", "subtype", "uuid", "session_id", "message_id", "user_message_uuid",
    "user_message_uuids", "command_uuid", "state", "request_id", "request_subtype", "tool_use_id",
    "tools", "terminal_reason", "is_error", "response_request_id", "response_subtype",
}
NATIVE_KEYS = BASE_NATIVE_KEYS | {
    "is_replay", "parent_tool_use_present", "content", "assistant_text_sha256",
    "assistant_trimmed_sha256", "result_text_sha256", "result_trimmed_sha256",
    "system_tool_count", "system_mcp_count", "native_version", "initialize_session_state",
    "permission_mode", "assistant_error_present", "message_role",
}
EVENT_KEYS = {
    "acceptance_started": {"event", "scope", "generation", "max_native_inputs", "native_framing_only",
                           "production_image_gate_open", "credential_files_read_by_probe", "real_gui_verified",
                           "persistence_verified", "http_request_count_verified"},
    "png_generated": {"event", "image_sha256", "image_bytes", "width", "height", "quadrants",
                      "expected_reply_sha256", "prompt_sha256"},
    "native_protocol_ids": {"event", "sequence", "native"},
    "input_submitted": {"event", "message_id", "submitted_input_count", "array_content", "transport_write_confirmed"},
    "cleanup_checked": {"event", "generation", "cleanup_confirmed", "transport_closed", "cleanup_receipt_read", "receipt"},
    "shutdown_output_drained": {"event", "bytes", "sha256", "frames", "native", "verified"},
    "native_image_proof": {"event", "native_session_id", "message_id", "native_result_uuid", "user_message_uuids",
                           "native_inputs", "replay_array_verified", "native_ack_verified", "native_started_verified",
                           "native_no_tools_verified", "assistant_origin_verified", "assistant_frames",
                           "assistant_output_sha256", "assistant_trimmed_sha256", "result_trimmed_sha256", "image_pixels_verified"},
    "acceptance_passed": {"event", "scope", "native_session_id", "message_id", "native_inputs", "native_framing_only",
                          "production_image_gate_open", "cleanup_confirmed", "transport_closed", "credential_files_read_by_probe",
                          "persistence_verified", "app_restart_and_ui_verified", "parent_permission_ceiling_verified",
                          "http_request_count_verified"},
}


def _require(condition):
    if not condition:
        raise ValueError("原生图片校准的安全证据不完整或相互矛盾")


def _uuid(value):
    _require(isinstance(value, str))
    parsed = uuid.UUID(value)
    _require(parsed.int != 0 and str(parsed) == value)
    return value


def _sha(value):
    _require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None)
    return value


def _integer(value, minimum, maximum):
    _require(type(value) is int and minimum <= value <= maximum)
    return value


def _keys(value, keys):
    _require(isinstance(value, dict) and set(value) == keys)


def _flags(value, true=(), false=()):
    for key in true:
        _require(value.get(key) is True)
    for key in false:
        _require(value.get(key) is False)


def _content(value):
    _require(isinstance(value, dict))
    shape = value.get("shape")
    if shape == "array":
        _keys(value, {"shape", "array_sha256", "blocks"})
        if value["array_sha256"] is not None:
            _sha(value["array_sha256"])
        _require(isinstance(value["blocks"], list) and len(value["blocks"]) <= 8)
        for block in value["blocks"]:
            kind = block.get("type") if isinstance(block, dict) else None
            if kind == "text":
                _keys(block, {"type", "text_sha256", "text_bytes"})
                _sha(block["text_sha256"])
                _integer(block["text_bytes"], 0, 1024 * 1024)
            elif kind == "image":
                _keys(block, {"type", "source_type", "media_type", "image_sha256", "image_bytes"})
                _require(block["source_type"] == "base64" and block["media_type"] == "image/png")
                _sha(block["image_sha256"])
                _integer(block["image_bytes"], 1, 1024 * 1024)
            elif kind == "thinking":
                _keys(block, {"type"})
            else:
                _require(False)
    elif shape == "missing":
        _keys(value, {"shape"})
    else:
        # 此验收不接受把真实数组降为字符串摘要，也不接收任意未知块。
        _require(False)


def audit_events(events):
    _require(isinstance(events, list) and 12 <= len(events) <= 270)
    for event in events:
        _require(isinstance(event, dict) and event.get("event") in EVENT_KEYS)
        _keys(event, EVENT_KEYS[event["event"]])
    def one(kind):
        matching = [event for event in events if event["event"] == kind]
        _require(len(matching) == 1)
        return matching[0]
    start, generated, submitted = [one(kind) for kind in ("acceptance_started", "png_generated", "input_submitted")]
    cleanup, proof, ending = [one(kind) for kind in ("cleanup_checked", "native_image_proof", "acceptance_passed")]
    _require(events[0] is start and events[-1] is ending and start["scope"] == ending["scope"] == SCOPE)
    generation, input_id, native_id = _uuid(start["generation"]), _uuid(submitted["message_id"]), _uuid(proof["native_session_id"])
    _require(input_id != native_id and proof["message_id"] == ending["message_id"] == input_id
             and ending["native_session_id"] == native_id)
    for value in (start["max_native_inputs"], submitted["submitted_input_count"], proof["native_inputs"], ending["native_inputs"]):
        _integer(value, 1, 1)
    _flags(start, true=("native_framing_only",), false=("production_image_gate_open", "credential_files_read_by_probe",
           "real_gui_verified", "persistence_verified", "http_request_count_verified"))
    _flags(submitted, true=("array_content", "transport_write_confirmed"))
    _flags(ending, true=("native_framing_only", "cleanup_confirmed", "transport_closed"),
           false=("production_image_gate_open", "credential_files_read_by_probe", "persistence_verified",
                  "app_restart_and_ui_verified", "parent_permission_ceiling_verified", "http_request_count_verified"))
    _flags(proof, true=("replay_array_verified", "native_ack_verified", "native_started_verified",
           "native_no_tools_verified", "assistant_origin_verified", "image_pixels_verified"))
    _integer(generated["width"], 64, 64)
    _integer(generated["height"], 64, 64)
    _integer(generated["quadrants"], 4, 4)
    image_bytes = _integer(generated["image_bytes"], 1, 1024 * 1024)
    image_sha, prompt_sha, expected_sha = [_sha(generated[key]) for key in ("image_sha256", "prompt_sha256", "expected_reply_sha256")]
    _require(len({image_sha, prompt_sha, expected_sha, EMPTY_SHA}) == 4)
    _flags(cleanup, true=("cleanup_confirmed", "transport_closed", "cleanup_receipt_read"))
    _require(cleanup["generation"] == generation)
    receipt = cleanup["receipt"]
    _keys(receipt, {"generation", "containment", "cleanup_confirmed", "exit_code", "exit_reason"})
    _require(receipt["generation"] == generation and receipt["cleanup_confirmed"] is True)
    _require(receipt["containment"] in {"macos_resource_coalition", "linux_subtree", "windows_job", "unix_process_group"})
    _require(receipt["exit_reason"] == "stdio_closed")
    _require(type(receipt["exit_code"]) is int and receipt["exit_code"] == 0)
    shutdown = [event for event in events if event["event"] == "shutdown_output_drained"]
    _require(len(shutdown) == 1)
    shutdown = shutdown[0]
    _flags(shutdown, true=("verified",))
    shutdown_bytes = _integer(shutdown["bytes"], 0, 8 * 1024 * 1024)
    shutdown_frames = _integer(shutdown["frames"], 0, 1)
    _sha(shutdown["sha256"])
    _require(isinstance(shutdown["native"], list) and len(shutdown["native"]) == shutdown_frames)
    if shutdown_bytes == 0:
        _require(shutdown_frames == 0 and shutdown["sha256"] == EMPTY_SHA)
    else:
        _require(shutdown_frames == 1 and shutdown["sha256"] != EMPTY_SHA)
    for row in shutdown["native"]:
        _keys(row, NATIVE_KEYS)
        _require(row["type"] == "command_lifecycle" and row["direction"] == "stdout"
                 and row["state"] == "completed" and row["session_id"] == native_id
                 and row["command_uuid"] == input_id and row["tools"] == []
                 and row["parent_tool_use_present"] is False and row["assistant_error_present"] is False
                 and row["content"] == {"shape": "missing"})
    projections = [event for event in events if event["event"] == "native_protocol_ids"]
    _require(6 <= len(projections) <= 256)
    rows = []
    for number, event in enumerate(projections, 1):
        _require(type(event["sequence"]) is int and event["sequence"] == number)
        row = event["native"]
        _keys(row, NATIVE_KEYS)
        _require(row["direction"] in {"stdin", "stdout"})
        _require(row["type"] in {"control_request", "control_response", "user", "system", "command_lifecycle", "assistant", "result", "keep_alive", "rate_limit_event"})
        _require(row["tools"] == [] and row["parent_tool_use_present"] is False)
        _require(row["assistant_error_present"] is False)
        _require(row["is_replay"] is None or type(row["is_replay"]) is bool)
        for key in ("assistant_text_sha256", "assistant_trimmed_sha256", "result_text_sha256", "result_trimmed_sha256"):
            if row[key] is not None:
                _sha(row[key])
        for key in ("uuid", "session_id", "message_id", "user_message_uuid", "command_uuid", "request_id", "tool_use_id", "response_request_id"):
            value = row[key]
            if value is not None:
                _require(isinstance(value, str) and len(value) <= 160
                         and (re.fullmatch(r"[a-zA-Z0-9_-]+", value) is not None
                              or re.fullmatch(r"sha256:[0-9a-f]{64}", value) is not None))
        _content(row["content"])
        if row["direction"] == "stdout" and row["type"] in {"user", "system", "command_lifecycle", "assistant", "result"}:
            _require(row["session_id"] == native_id)
        rows.append(row)
    def selected(**filters):
        return [row for row in rows if all(row.get(key) == value for key, value in filters.items())]
    initialize_id = f"infinishell-{generation}-1"
    initializes = selected(direction="stdin", type="control_request", request_subtype="initialize")
    responses = selected(direction="stdout", type="control_response")
    _require(len(initializes) == len(responses) == 1 and initializes[0]["request_id"] == initialize_id
             and responses[0]["response_request_id"] == initialize_id
             and responses[0]["response_subtype"] == "success" and responses[0]["initialize_session_state"] == "idle"
             and responses[0]["permission_mode"] == "default")
    _require(len(selected(type="control_request")) == 1)
    _require(len(selected(direction="stdin")) == 2)
    inputs, replays = selected(direction="stdin", type="user"), selected(direction="stdout", type="user")
    _require(len(inputs) == len(replays) == 1)
    written, replay = inputs[0], replays[0]
    _require(written["uuid"] == replay["uuid"] == input_id and replay["is_replay"] is True
             and written["message_role"] == replay["message_role"] == "user"
             and written["session_id"] == "sha256:" + EMPTY_SHA and written["content"] == replay["content"])
    blocks = written["content"].get("blocks")
    _sha(written["content"].get("array_sha256"))
    _require(written["content"]["shape"] == "array" and isinstance(blocks, list) and len(blocks) == 2
             and blocks[0]["type"] == "text" and blocks[0]["text_sha256"] == prompt_sha
             and blocks[1] == {"type": "image", "source_type": "base64", "media_type": "image/png",
                               "image_sha256": image_sha, "image_bytes": image_bytes})
    lifecycle = selected(direction="stdout", type="command_lifecycle")
    _require(lifecycle and all(row["command_uuid"] == input_id and row["state"] in {"queued", "started", "completed"} for row in lifecycle))
    starts = [row for row in lifecycle if row["state"] == "started"]
    _require(len(starts) == 1 and sum(row["state"] == "queued" for row in lifecycle) <= 1
             and sum(row["state"] == "completed" for row in lifecycle) <= 1)
    systems = selected(direction="stdout", type="system")
    _require(len(systems) == 1 and systems[0]["subtype"] == "init" and systems[0]["native_version"] == "2.1.273"
             and systems[0]["permission_mode"] == "default"
             and type(systems[0]["system_tool_count"]) is int and systems[0]["system_tool_count"] == 0
             and type(systems[0]["system_mcp_count"]) is int and systems[0]["system_mcp_count"] == 0)
    assistants = selected(direction="stdout", type="assistant")
    _require(assistants and _integer(proof["assistant_frames"], 1, 256) == len(assistants))
    assistant_ids = [_uuid(row["uuid"]) for row in assistants]
    _require(len(set(assistant_ids)) == len(assistant_ids))
    for row in assistants:
        _require(row["user_message_uuid"] == input_id and row["content"]["shape"] == "array"
                 and row["message_role"] == "assistant"
                 and row["content"]["array_sha256"] is None
                 and rows.index(starts[0]) < rows.index(row))
        text_blocks = [block for block in row["content"]["blocks"] if block["type"] == "text"]
        _require(len(text_blocks) <= 1)
        _require(row["assistant_text_sha256"] == (text_blocks[0]["text_sha256"] if text_blocks else EMPTY_SHA))
    text_frames = [row for row in assistants if row["assistant_text_sha256"] != EMPTY_SHA]
    _require(len(text_frames) == 1 and text_frames[0]["assistant_trimmed_sha256"] == expected_sha
             and proof["assistant_output_sha256"] == text_frames[0]["assistant_text_sha256"]
             and proof["assistant_trimmed_sha256"] == proof["result_trimmed_sha256"] == expected_sha)
    results = selected(direction="stdout", type="result")
    _require(len(results) == 1)
    result = results[0]
    _require(_uuid(result["uuid"]) == proof["native_result_uuid"] and result["user_message_uuid"] == input_id
             and result["user_message_uuids"] == proof["user_message_uuids"] == [input_id]
             and result["subtype"] == "success" and result["is_error"] is False
             and result["terminal_reason"] in {None, "completed", "end_turn"}
             and result["result_trimmed_sha256"] == expected_sha and result["result_text_sha256"] is not None)
    _require(rows.index(initializes[0]) < rows.index(responses[0]) < rows.index(written) < rows.index(starts[0])
             and all(rows.index(row) < rows.index(result) for row in assistants + [replay, systems[0]])
             and all(rows.index(row) < rows.index(result) for row in lifecycle))
    position = lambda event: events.index(event)
    written_event = projections[rows.index(written)]
    _require(position(generated) < position(written_event) < position(submitted) < position(projections[rows.index(starts[0])])
             and position(projections[rows.index(result)]) < position(shutdown) < position(cleanup) < position(proof) < position(ending))
    return {"native_session_id": native_id, "runtime_generation": generation, "input_id": input_id,
            "native_result_uuid": result["uuid"], "image_sha256": image_sha, "image_bytes": image_bytes,
            "prompt_sha256": prompt_sha, "expected_reply_sha256": expected_sha, "native_protocol_records": len(rows),
            "native_framing_only": True, "production_image_gate_open": False}


def verified_acceptance(exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    try:
        audit_events(events)
    except (ValueError, TypeError, KeyError, AttributeError, IndexError):
        return False
    return True


def run(args):
    if args.api_environment_file is None:
        raise ValueError("图片校准必须显式提供私有 API 环境文件")
    if not isinstance(args.model, str) or not args.model or any(character in args.model for character in "\x00\r\n"):
        raise ValueError("图片校准必须显式固定原生模型")
    private = Path(tempfile.mkdtemp(prefix="infinishell-claude-image-auth-")).resolve()
    args.config_dir, args.auth_home = private / "claude", private / "home"
    args.config_dir.mkdir(mode=0o700)
    args.auth_home.mkdir(mode=0o700)
    adapter.validate_paths(args)
    if os.name != "nt" and args.api_environment_file.stat().st_mode & 0o077:
        raise ValueError("API 环境文件必须仅当前用户可读写")
    replacements = {"TEST_NAME": TEST_NAME, "PROJECT_SETTINGS": PROJECT_SETTINGS,
                    "verified_acceptance": verified_acceptance}
    original = {key: getattr(adapter, key) for key in replacements}
    try:
        for key, value in replacements.items():
            setattr(adapter, key, value)
        result = adapter.run(args)
    finally:
        for key, value in original.items():
            setattr(adapter, key, value)
    path = args.output.with_suffix(".metadata.json")
    metadata = json.loads(path.read_text(encoding="utf-8"))
    metadata.update({"scope": SCOPE, "authentication_source": "explicit_api_environment",
                     "fresh_private_auth_home": True, "private_auth_workspace": str(private), "max_native_inputs": 1,
                     "native_framing_only": True, "native_image_input_verified": False, "production_image_gate_open": False,
                     "http_request_count_verified": False, "persistence_verified": False, "app_restart_and_ui_verified": False,
                     "parent_permission_ceiling_verified": False, "filesystem_sandbox_verified": False})
    if metadata.get("acceptance_passed") is True:
        try:
            events = [json.loads(line) for line in args.output.read_text(encoding="utf-8").splitlines()]
            metadata["native_image_proof"] = audit_events(events)
            metadata["native_image_input_verified"] = True
        except (ValueError, TypeError, KeyError, AttributeError, IndexError):
            metadata["acceptance_passed"] = False
            metadata["runner_error"] = "公开证据的独立图片校准审核未通过"
            result = 1
    path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True, help="固定 2.1.273 原生可执行文件")
    parser.add_argument("--supervisor", type=Path, required=True, help="同提交生产监督入口")
    parser.add_argument("--api-environment-file", type=Path, required=True, help="私有显式 Anthropic API 环境 JSON")
    parser.add_argument("--model", required=True, help="固定真实原生模型")
    parser.add_argument("--output", type=Path, required=True, help="全新 .ndjson 证据路径")
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, adapter.subprocess.SubprocessError) as error:
        parser.exit(2, f"图片校准运行器启动失败：{type(error).__name__}；请检查显式输入文件。\n")


if __name__ == "__main__":
    raise SystemExit(main())
