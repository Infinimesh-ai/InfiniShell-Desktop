#!/usr/bin/env python3
"""审核 Grok 协调器证据的离线拒绝路径；不调用 CLI、网络、模型或图形界面。"""

import copy
from contextlib import closing
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid

import run_grok_coordinator_live as runner


def uid(number):
    return str(uuid.UUID(int=number))


def successful_evidence():
    # 本函数仅生成审核器的离线测试数据，不作为任何真实执行证据。
    events = [{"event": "acceptance_started", "scope": runner.SCOPE, "max_native_inputs": 8,
        "production_runtime_commands": True, "test_only_internal_command_switch": False,
        "permission_policy": "Inherit", "local_tools": None, "real_gui_verified": False,
        "app_restart_verified": False, "sdk_verified": False, "parent_child_verified": False,
        "parent_permission_ceiling_verified": False, "credential_files_read_by_probe": False}]
    session, task = uid(800), uid(900)
    tokens = (uid(1001), uid(1002))
    marker = "GROK_COORD_QUEUED_" + "a" * 32
    outputs = ("GROK_COORD_ONE", "GROK_COORD_TWO", "Preparing\nAPPROVED", "Denied",
        "1\n2\nQUEUE_PARENT_DONE", marker, "Partial", marker)
    submitted = []
    history = []
    messages = []
    proof_lines = []
    for index in range(8):
        token = tokens[index == 7]
        message_id = uid(index + 1)
        turn = uid(index + 2001)
        expected = outputs[index] if index in (0, 1, 5, 7) else None
        item = {"event": "input_submitted", "phase": runner.PHASES[index], "message_id": message_id,
            "submission_generation": runner.GENERATIONS[index], "runtime_generation": token,
            "submitted_while_running": index == 5, "active_turn_id": uid(2005) if index == 5 else None,
            "expected_marker": expected, "expected_outcome": runner.OUTCOMES[index], "submitted_input_count": index + 1}
        submitted.append(item)
        item.update(body_bytes=9, body_sha256=runner.sha("test-body"))
        message = {"message_id": message_id, "sender_task_id": task, "recipient_task_id": task,
            "sender_generation": runner.GENERATIONS[index], "recipient_generation": runner.GENERATIONS[index],
            "subject": "user_input", "state": "acknowledged", "receipt_kind": "native_protocol",
            "body_bytes": 9, "body_sha256": runner.sha("test-body")}
        messages.append(message)
        record = {"task_id": task, "harness": "grok", "parent_task_id": None, "parent_generation": None,
            "generation": index + 1, "revision": index + 10, "state": runner.OUTCOMES[index].lower(),
            "native_session_id": session, "result_bytes": len(outputs[index].encode()), "result_sha256": runner.sha(outputs[index]),
            "terminal_evidence_present": True, "terminal_native_session_id": session,
            "terminal_turn_id": turn, "terminal_outcome": runner.OUTCOMES[index], "runtime_generation": token,
            "permission_policy": "Inherit", "permission_ceiling": None, "claude_profile": None,
            "local_tools": None, "model": None, "selected_skills": [], "grok_pending_inputs": [],
            "grok_current_input": {"message_id": message_id, "submission_generation": runner.GENERATIONS[index],
                "runtime_generation": token, "native_turn_id": turn}}
        history.append(record)
        proof_lines.append(runner.HISTORY_TRACE + json.dumps({"runtime_generation": token, "session_id": session,
            "turn_id": turn, "completion_watermark": session + "-" + str(index + 1),
            "outcome": runner.OUTCOMES[index], "full_output_bytes": len(outputs[index].encode()),
            "full_output_sha256": runner.sha(outputs[index])}))

    def runtime(index, kind, **extra):
        return {"event": "runtime", "kind": kind, "task_id": task, "task_generation": index + 1,
            "runtime_generation": tokens[index == 7], "native_session_id": session, **extra}

    def begin(index, include_submit=True):
        if include_submit:
            events.append(submitted[index])
        events.append(runtime(index, "MessageAccepted", message_id=uid(index + 1), turn_id=uid(index + 2001), native_receipt=True))
        events.append(runtime(index, "TurnStarted", turn_id=uid(index + 2001)))

    def finish(index):
        events.append(runtime(index, "TurnFinished", turn_id=uid(index + 2001), message_id=uid(index + 1),
            phase=runner.PHASES[index], outcome=runner.OUTCOMES[index], output_bytes=len(outputs[index].encode()),
            output_sha256=runner.sha(outputs[index]), expected_marker_matched=True if submitted[index]["expected_marker"] else None,
            submission_generation=runner.GENERATIONS[index]))

    for index in range(4):
        begin(index)
        if index in (2, 3):
            approval = uid(index + 4001)
            decision = "AllowOnce" if index == 2 else "DenyOnce"
            events.append(runtime(index, "ApprovalRequested", turn_id=uid(index + 2001), approval_id=approval, exact_write_verified=True))
            events.append({"event": "approval_decision_submitted", "approval_id": approval,
                "message_id": uid(index + 5001), "decision": decision, "native_receipt": False})
            events.append(runtime(index, "CommandDispatched", turn_id=uid(index + 2001), message_id=uid(index + 5001), native_receipt=False))
            events.append(runtime(index, "ApprovalResolved", approval_id=approval, decision=decision, native_receipt=False))
        finish(index)
        if index in (2, 3):
            events.append({"event": "file_effect_verified", "phase": runner.PHASES[index],
                "allowed": index == 2, "exact_file_state_verified": True})
    begin(4)
    events.append(runtime(4, "FirstText", turn_id=uid(2005), text_bytes=1))
    events.append(submitted[5])
    finish(4)
    begin(5, False)
    finish(5)
    begin(6)
    events.append(runtime(6, "FirstText", turn_id=uid(2007), text_bytes=1))
    events.append({"event": "cancel_submitted", "message_id": uid(7001), "execution_turn_id": uid(2007),
        "native_input_acknowledged": True, "real_text_started": True})
    events.append(runtime(6, "CommandDispatched", message_id=uid(7001), turn_id=uid(2007), native_receipt=False))
    finish(6)
    for index in range(2):
        cleanup = {"event": "cleanup_confirmed", "runtime_generation": tokens[index], "receipt": {
            "version": 1, "generation": tokens[index], "cleanup_confirmed": True, "exit_code": 0,
            "exit_reason": "stdio_closed", "containment": "macos_resource_coalition", "manifest_sha256": "b" * 64}}
        if index == 0:
            events.append(cleanup)
            before = {"event": "sqlite_snapshot", "label": "before_history_reload", "task": copy.deepcopy(history[6]),
                "history": copy.deepcopy(history[:7]), "messages": copy.deepcopy(messages[:7])}
            ready_task = copy.deepcopy(history[7])
            ready_task.update(state="queued", result_bytes=None, result_sha256=None, terminal_evidence_present=False,
                terminal_native_session_id=None, terminal_turn_id=None, terminal_outcome=None,
                grok_current_input=None, grok_pending_inputs=None)
            ready = {"event": "sqlite_snapshot", "label": "resume_ready_no_replay", "task": ready_task,
                "history": copy.deepcopy(history[:7]) + [ready_task], "messages": copy.deepcopy(messages[:7])}
            events.extend((before, ready))
            begin(7)
            finish(7)
        else:
            events.append(cleanup)
    database = {"task": copy.deepcopy(history[7]), "history": copy.deepcopy(history), "messages": copy.deepcopy(messages)}
    events.append({"event": "sqlite_snapshot", "label": "final", **copy.deepcopy(database)})
    events.append({"event": "acceptance_passed", "scope": runner.SCOPE, "native_inputs": 8, "native_executions": 8,
        "native_acknowledged_inputs": 8, "completed_inputs": 6, "cancelled_inputs": 2, "approvals_verified": 2,
        "sqlite_verified": True, "same_native_session_verified": True, "cleanup_confirmed": True,
        "real_gui_verified": False, "app_restart_verified": False, "sdk_verified": False,
        "parent_child_verified": False, "parent_permission_ceiling_verified": False})
    output = "\n".join(proof_lines) + "\ntest result: ok. 1 passed; 0 failed; 0 ignored; 123 filtered out;\n"
    return output, events, database


class GrokCoordinatorRunnerTests(unittest.TestCase):
    def setUp(self):
        self.output, self.events, self.database = successful_evidence()

    def accepted(self, events=None, output=None, database=None, exit_code=0):
        return runner.verified_acceptance(exit_code, self.output if output is None else output,
            self.events if events is None else events, self.database if database is None else database)

    def mutate_database(self, function):
        database = copy.deepcopy(self.database)
        function(database)
        events = copy.deepcopy(self.events)
        final = runner.one(events, "sqlite_snapshot", label="final")
        final.update(copy.deepcopy(database))
        return events, database

    def test_complete_correlated_offline_example_is_accepted(self):
        self.assertTrue(self.accepted())

    def test_pass_marker_and_rpc_end_turn_without_history_are_rejected(self):
        output = 'GROK_NATIVE_PROTOCOL_IDS {"id":4,"stopReason":"end_turn"}\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n'
        self.assertFalse(self.accepted(output=output))

    def test_each_missing_history_signal_is_rejected(self):
        lines = self.output.splitlines()
        for index in range(8):
            with self.subTest(index=index):
                self.assertFalse(self.accepted(output="\n".join(lines[:index] + lines[index + 1:])))

    def test_wrong_history_identity_outcome_watermark_or_digest_is_rejected(self):
        lines = self.output.splitlines()
        for field, value in {"runtime_generation": uid(99), "session_id": uid(99), "turn_id": uid(99),
                "completion_watermark": uid(99) + "-1", "outcome": "Cancelled", "full_output_bytes": 99,
                "full_output_sha256": "c" * 64}.items():
            with self.subTest(field=field):
                changed = lines.copy()
                trace = json.loads(changed[0].split(runner.HISTORY_TRACE)[1])
                trace[field] = value
                changed[0] = runner.HISTORY_TRACE + json.dumps(trace)
                self.assertFalse(self.accepted(output="\n".join(changed)))

    def test_duplicate_history_and_secret_fields_are_rejected(self):
        self.assertFalse(self.accepted(output=self.output + self.output.splitlines()[0] + "\n"))
        trace = json.loads(self.output.splitlines()[0].split(runner.HISTORY_TRACE)[1])
        trace["body"] = "private text"
        self.assertFalse(self.accepted(output=runner.HISTORY_TRACE + json.dumps(trace) + "\n" + "\n".join(self.output.splitlines()[1:])))

    def test_each_missing_ack_started_or_terminal_is_rejected(self):
        for index, event in enumerate(self.events):
            if event.get("kind") not in ("MessageAccepted", "TurnStarted", "TurnFinished"):
                continue
            with self.subTest(index=index):
                self.assertFalse(self.accepted(events=self.events[:index] + self.events[index + 1:]))

    def test_application_receipt_or_sent_does_not_replace_native_ack(self):
        for field, value in (("state", "sent"), ("receipt_kind", "application_history"), ("receipt_kind", None)):
            events, database = self.mutate_database(lambda data: data["messages"][5].update({field: value}))
            self.assertFalse(self.accepted(events=events, database=database))

    def test_queue_original_submission_and_native_link_cannot_be_rewritten(self):
        for target, field, value in (("message", "recipient_generation", 6), ("message", "sender_generation", 6),
                ("link", "submission_generation", 6), ("link", "runtime_generation", uid(99)),
                ("link", "native_turn_id", uid(99)), ("link", "message_id", uid(99))):
            with self.subTest(target=target, field=field):
                def mutation(data):
                    entry = data["messages"][5] if target == "message" else data["history"][5]["grok_current_input"]
                    entry[field] = value
                events, database = self.mutate_database(mutation)
                self.assertFalse(self.accepted(events=events, database=database))

    def test_wrong_native_session_task_or_runtime_generation_is_rejected(self):
        for field in ("native_session_id", "task_id", "runtime_generation"):
            events = copy.deepcopy(self.events)
            runner.one(events, "runtime", kind="MessageAccepted", message_id=uid(6))[field] = uid(99)
            self.assertFalse(self.accepted(events=events))

    def test_sqlite_terminal_evidence_and_full_result_are_required(self):
        for field, value in (("terminal_evidence_present", False), ("terminal_turn_id", uid(99)),
                ("terminal_native_session_id", uid(99)), ("terminal_outcome", "Failed"),
                ("result_sha256", "c" * 64), ("result_bytes", 99), ("state", "failed")):
            events, database = self.mutate_database(lambda data: data["history"][6].update({field: value}))
            self.assertFalse(self.accepted(events=events, database=database))

    def test_independent_database_must_equal_rust_final_snapshot(self):
        database = copy.deepcopy(self.database)
        database["messages"][0]["body_sha256"] = "c" * 64
        self.assertFalse(self.accepted(database=database))

    def test_resume_load_cannot_replay_or_mutate_original_messages(self):
        for mutation in (lambda ready: ready["messages"].append(copy.deepcopy(self.database["messages"][7])),
                lambda ready: ready["messages"][0].update(body_sha256="c" * 64),
                lambda ready: ready["task"].update(state="running"),
                lambda ready: ready["task"].update(grok_current_input={"message_id": uid(7)}),
                lambda ready: ready["task"].update(native_session_id=uid(99))):
            events = copy.deepcopy(self.events)
            mutation(runner.one(events, "sqlite_snapshot", label="resume_ready_no_replay"))
            self.assertFalse(self.accepted(events=events))

    def test_cancel_and_approval_dispatch_or_receipt_cannot_be_invented(self):
        for index, event in enumerate(self.events):
            if event.get("kind") not in ("CommandDispatched", "ApprovalRequested", "ApprovalResolved"):
                continue
            with self.subTest(index=index):
                self.assertFalse(self.accepted(events=self.events[:index] + self.events[index + 1:]))
        events = copy.deepcopy(self.events)
        runner.one(events, "runtime", kind="ApprovalResolved", approval_id=uid(4003))["native_receipt"] = True
        self.assertFalse(self.accepted(events=events))

    def test_all_cleanup_receipts_and_real_exit_are_required(self):
        for field, value in (("cleanup_confirmed", False), ("exit_code", 1), ("exit_reason", "forced"),
                ("generation", uid(99)), ("containment", "unknown")):
            events = copy.deepcopy(self.events)
            next(event for event in events if event.get("event") == "cleanup_confirmed")["receipt"][field] = value
            self.assertFalse(self.accepted(events=events))

    def test_extra_input_and_failed_libtest_are_rejected(self):
        self.assertFalse(self.accepted(events=self.events + [copy.deepcopy(runner.one(self.events, "input_submitted", phase="first_turn"))]))
        self.assertFalse(self.accepted(exit_code=101))
        self.assertFalse(self.accepted(output=self.output.replace("1 passed; 0 failed", "0 passed; 1 failed")))

    def test_public_projection_omits_unknown_text_and_credentials(self):
        private = [{"event": "acceptance_failed", "reason": "sk-" + "q" * 40,
            "nested": {"output": "Bearer " + "q" * 40, "safe": uid(1), "digest": "a" * 64}}]
        public = runner.public_events(private)
        encoded = json.dumps(public)
        self.assertNotIn("sk-", encoded)
        self.assertNotIn("Bearer", encoded)
        self.assertNotIn("nested", public[0])

    def test_public_projection_drops_unknown_hex_uuid_nested_and_event_fields(self):
        event = copy.deepcopy(runner.one(self.events, "input_submitted", phase="first_turn"))
        event.update(nested={"output": "a" * 64}, output="b" * 64, unknown_identity=uid(77), unknown_count=91)
        unknown = {"event": "private_unknown_event", "output": "c" * 64, "message_id": uid(88)}
        projected = runner.public_events([event, unknown])
        self.assertEqual(projected[1], {"event": "unknown_event"})
        for key in ("nested", "output", "unknown_identity", "unknown_count"):
            self.assertNotIn(key, projected[0])
        encoded = json.dumps(projected)
        for value in ("a" * 64, "b" * 64, "c" * 64, uid(77), uid(88), "private_unknown_event"):
            self.assertNotIn(value, encoded)
        self.assertEqual(projected[0]["body_sha256"], event["body_sha256"])
        self.assertEqual(projected[0]["message_id"], event["message_id"])

    def test_public_projection_keeps_auditable_known_contract_without_widening_types(self):
        self.assertTrue(self.accepted(events=runner.public_events(self.events)))
        event = copy.deepcopy(runner.one(self.events, "input_submitted", phase="first_turn"))
        event.update(phase="a" * 64, expected_marker=uid(33), body_sha256=uid(44), submitted_input_count="a" * 64)
        encoded = json.dumps(runner.public_events([event]))
        for value in ("a" * 64, uid(33), uid(44)):
            self.assertNotIn(value, encoded)
        events = [{"event": "approval_decision_submitted", "approval_id": value}
            for value in ("grok:3", "grok:" + json.dumps(uid(2)), "PRIVATE_APPROVAL_CANARY")]
        projected = runner.public_events(events)
        self.assertEqual(projected[0]["approval_id"], "grok:3")
        self.assertEqual(projected[1]["approval_id"], "grok:" + json.dumps(uid(2)))
        self.assertEqual(projected[2]["approval_id"], {"identity_sha256": runner.sha("PRIVATE_APPROVAL_CANARY")})

    def test_input_links_reject_extra_fields_and_strict_type_violations(self):
        link = copy.deepcopy(self.database["task"]["grok_current_input"])
        self.assertEqual(runner.input_link_projection(link), link)
        self.assertEqual(runner.pending_inputs_projection([link]), [link])
        mutations = ({**link, "output": "a" * 64}, {**link, "link": uid(77)},
            {**link, "message_id": "a" * 64}, {**link, "runtime_generation": link["runtime_generation"].upper()},
            {**link, "submission_generation": True}, {**link, "submission_generation": "8"},
            {**link, "submission_generation": 0}, {**link, "submission_generation": 2**63},
            {**link, "native_turn_id": "PRIVATE_LINK_CANARY"})
        task = {"task_id": uid(900), "harness": "grok", "generation": 8, "revision": 10, "state": "completed"}
        for changed in mutations:
            for key, value in (("grok_current_input", changed), ("grok_pending_inputs", [changed])):
                with self.subTest(key=key, value=value):
                    config = {"permission_policy": "Inherit", "selected_skills": [], key: value}
                    with self.assertRaises(ValueError):
                        runner.task_projection({**task, "config_json": json.dumps(config)})
        for pending in ([None], {}, [link] * 9):
            with self.assertRaises(ValueError):
                runner.pending_inputs_projection(pending)

    def test_public_sqlite_links_never_export_unrecognized_nested_fields(self):
        event = copy.deepcopy(runner.one(self.events, "sqlite_snapshot", label="final"))
        event["task"]["grok_current_input"]["link"] = "a" * 64
        event["task"]["grok_pending_inputs"] = [{**self.database["task"]["grok_current_input"], "output": "b" * 64}]
        encoded = json.dumps(runner.public_events([event]))
        self.assertNotIn("a" * 64, encoded)
        self.assertNotIn("b" * 64, encoded)
        self.assertNotIn('"link"', encoded)

    def test_audit_rejects_extra_link_fields_even_with_matching_sqlite_snapshot(self):
        for key in ("output", "link"):
            events, database = self.mutate_database(lambda data: data["history"][5]["grok_current_input"].update({key: "a" * 64}))
            self.assertFalse(self.accepted(events=events, database=database))

    def test_actual_message_body_digest_must_match_original_submission(self):
        for field, value in (("body_sha256", "f" * 64), ("body_bytes", 123), ("body_bytes", True)):
            events = copy.deepcopy(self.events)
            runner.one(events, "input_submitted", phase="queue_instruction")[field] = value
            self.assertFalse(self.accepted(events=events))
            events, database = self.mutate_database(lambda data: data["messages"][5].update({field: value}))
            self.assertFalse(self.accepted(events=events, database=database))

    def test_project_file_projection_preserves_only_fixed_names_and_hashes_canary(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            canary = "PRIVATE_FILENAME_CANARY_" + "a" * 64
            for name in ("approval-allow.txt", "approval-deny.txt", canary):
                (project / name).write_text("offline fixture")
            nested = project / "nested"
            nested.mkdir()
            (nested / "approval-allow.txt").write_text("offline fixture")
            projection = runner.project_file_projection(project)
            self.assertEqual(projection["project_files"], ["approval-allow.txt", "approval-deny.txt"])
            self.assertEqual(projection["project_file_count"], 4)
            self.assertEqual(projection["unexpected_project_file_count"], 2)
            self.assertEqual(projection["unexpected_project_file_name_sha256"], sorted([
                runner.sha(canary), runner.sha(str(Path("nested") / "approval-allow.txt"))]))
            self.assertNotIn(canary, json.dumps(projection))

    def test_actual_sqlite_projection_uses_safe_json_and_body_hashes(self):
        # 真 SQLite 文件验证查询和投影，仍不冒充模型或生产协调器执行。
        config = {"permission_policy": "Inherit", "selected_skills": [], "runtime_generation": uid(1001)}
        task = {"task_id": uid(900), "harness": "grok", "parent_task_id": None, "parent_generation": None,
            "generation": 1, "revision": 1, "state": "completed", "native_session_id": uid(800),
            "result": "safe marker", "terminal_evidence": json.dumps({"native_session_id": uid(800),
                "event": {"TurnFinished": {"turn_id": uid(2001), "outcome": "Completed", "output": "safe marker"}}}),
            "config_json": json.dumps(config)}
        message = {"message_id": uid(1), "sender_task_id": uid(900), "recipient_task_id": uid(900),
            "sender_generation": 1, "recipient_generation": 1, "subject": "user_input",
            "body": json.dumps({"Submit": {"input": [{"Text": "private-body"}]}}),
            "state": "acknowledged", "receipt_kind": "native_protocol"}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "coordinator.sqlite"
            with closing(sqlite3.connect(path)) as connection, connection:
                connection.executescript("CREATE TABLE local_cli_tasks(data TEXT); CREATE TABLE local_cli_task_generations(generation INTEGER,data TEXT); CREATE TABLE local_cli_messages(sequence INTEGER,data TEXT);")
                connection.execute("INSERT INTO local_cli_tasks VALUES (?)", (json.dumps(task),))
                connection.execute("INSERT INTO local_cli_task_generations VALUES (?,?)", (1, json.dumps(task)))
                connection.execute("INSERT INTO local_cli_messages VALUES (?,?)", (1, json.dumps(message)))
            with self.assertRaises(sqlite3.ProgrammingError):
                connection.execute("SELECT 1")
            # 保留真实连接引用，避免垃圾回收掩盖 Windows 清理时的文件占用。
            read_connection = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True)
            with patch.object(runner.sqlite3, "connect", return_value=read_connection):
                projection = runner.sqlite_projection(path)
            with self.assertRaises(sqlite3.ProgrammingError):
                read_connection.execute("SELECT 1")
            self.assertEqual(projection["messages"][0]["body_sha256"], runner.sha(message["body"]))
            self.assertEqual(projection["task"]["terminal_native_session_id"], uid(800))
            self.assertNotIn("private-body", json.dumps(projection))
            self.assertNotIn("config_json", json.dumps(projection))
            with closing(sqlite3.connect(path)) as writer, writer:
                writer.execute("UPDATE local_cli_tasks SET data = ?", ("not-json",))
            error_connection = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True)
            with patch.object(runner.sqlite3, "connect", return_value=error_connection):
                with self.assertRaises(json.JSONDecodeError):
                    runner.sqlite_projection(path)
            with self.assertRaises(sqlite3.ProgrammingError):
                error_connection.execute("SELECT 1")
            path.unlink()
        task["parent_task_id"] = uid(99)
        with self.assertRaises(ValueError):
            runner.task_projection(task)

    def test_sqlite_projection_rejects_skill_image_steer_or_wrong_terminal_body(self):
        for action in ({"Steer": {"input": [{"Text": "text"}]}},
                {"Submit": {"input": [{"Skill": {"name": "skill", "path": "private"}}]}},
                {"Submit": {"input": [{"LocalImage": "private"}]}},
                {"Submit": {"input": [{"Text": "one"}, {"Text": "two"}]}}):
            with self.assertRaises(ValueError):
                runner.message_projection({"body": json.dumps(action)})
        projection = self.database["history"][0]
        task = {"task_id": projection["task_id"], "harness": "grok", "parent_task_id": None,
            "parent_generation": None, "generation": 1, "revision": 1, "state": "completed",
            "config_json": json.dumps({"permission_policy": "Inherit", "selected_skills": []}),
            "result": "one", "terminal_evidence": json.dumps({"event": {"TurnFinished": {"output": "two"}}})}
        with self.assertRaises(ValueError):
            runner.task_projection(task)

    def test_saved_current_task_and_permission_boundaries_cannot_diverge(self):
        for field, value in (("harness", "claude"), ("parent_task_id", uid(99)),
                ("parent_generation", 1), ("permission_policy", "ReadOnly"),
                ("local_tools", {"allow_spawn": True}), ("permission_ceiling", {}),
                ("selected_skills", ["skill"]), ("model", "override")):
            events, database = self.mutate_database(lambda data: data["history"][0].update({field: value}))
            self.assertFalse(self.accepted(events=events, database=database))
        events, database = self.mutate_database(lambda data: data["task"].update(generation=7))
        self.assertFalse(self.accepted(events=events, database=database))

    def test_failed_offline_process_still_removes_auth_and_keeps_safe_sources(self):
        # 全部进程、网络、认证来源均为本测试的模拟对象，绝不使用用户登录资料。
        class Tunnel:
            deadline = time.monotonic() + 1000
            events = []
            forwarded = 0
            bytes = 0

            def start(self):
                return 12345

            def close(self):
                return True

        def prepare(root, *unused):
            wrapper = root / "wrapper"
            wrapper.write_text("offline wrapper")
            settings = root / "home/.grok/config.toml"
            settings.write_text('[cli]\nauto_update=false\n')
            (root / "project/PRIVATE_FILENAME_CANARY_OFFLINE_FAILURE").write_text("offline fixture")
            (root / "private-evidence.ndjson").write_text("".join(json.dumps(event) + "\n" for event in self.events))
            (root / "wrapper-audit.ndjson").write_text("".join(json.dumps({"event": "native_launch",
                "kind": "private_leader", "arguments_unchanged": True, "private_socket": f"tmp/{index}/leader.sock"})
                + "\n" for index in range(2)))
            return wrapper, settings

        def copy_auth(source, target):
            (target / "auth.json").write_text("opaque offline auth fixture")

        create_directory = tempfile.mkdtemp
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def private_directory(**kwargs):
                self.assertEqual(kwargs["dir"], "/private/tmp")
                kwargs["dir"] = root
                return create_directory(**kwargs)

            for name in ("libtest", "grok", "supervisor"):
                (root / name).write_text("offline binary fixture")
            args = SimpleNamespace(output=root / "evidence.ndjson", timeout=900,
                test_binary=root / "libtest", grok=root / "grok", supervisor=root / "supervisor",
                official_grok_home=root / "offline-auth-source")
            process = SimpleNamespace(returncode=101, communicate=lambda **kwargs: ("offline process failed\n", None))
            with patch.object(runner.tempfile, "mkdtemp", side_effect=private_directory), \
                    patch.object(runner.official, "OfficialTunnel", return_value=Tunnel()), \
                    patch.object(runner.official, "copy_private_auth", side_effect=copy_auth), \
                    patch.object(runner.official, "prepare_native", side_effect=prepare), \
                    patch.object(runner.official.shared, "network_canary", return_value={"offline_mock": True}), \
                    patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout=runner.official.shared.VERSION)), \
                    patch.object(runner.subprocess, "Popen", return_value=process), patch("builtins.print"):
                result = runner.run(args)
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            private = Path(metadata["private_workspace"])
            try:
                self.assertEqual(result, 1)
                self.assertFalse(metadata["acceptance_passed"])
                self.assertFalse(metadata["official_grok_model_tested"])
                self.assertTrue(metadata["private_auth_copy_removed"])
                self.assertTrue(metadata["tunnels_stopped"])
                self.assertFalse((private / "home/.grok/auth.json").exists())
                self.assertEqual(len(metadata["native_launches"]), 2)
                self.assertEqual(metadata["validated_native_history_trace_records"], 0)
                self.assertFalse(metadata["independent_sqlite_projection_available"])
                self.assertEqual(metadata["private_test_output_sha256"], runner.sha("offline process failed\n"))
                self.assertNotIn("opaque offline auth fixture", args.output.read_text())
                self.assertNotIn("PRIVATE_FILENAME_CANARY_OFFLINE_FAILURE", args.output.with_suffix(".metadata.json").read_text())
                self.assertEqual(metadata["project_files"], [])
                self.assertEqual(metadata["unexpected_project_file_count"], 1)
                self.assertEqual(metadata["unexpected_project_file_name_sha256"], [runner.sha("PRIVATE_FILENAME_CANARY_OFFLINE_FAILURE")])
            finally:
                shutil.rmtree(private)


if __name__ == "__main__":
    unittest.main()
