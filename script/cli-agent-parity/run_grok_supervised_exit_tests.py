#!/usr/bin/env python3
"""监督清理运行器的离线安全回归；合成收据不计为真实 CLI 验收。"""

import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
import uuid

import run_grok_supervised_exit as runner


INPUT_DIGEST = "a" * 64


def case(name, target="win32-x64"):
    generation = str(uuid.uuid4())
    return {
        "event": "case_passed", "scenario": name, "cli_version": "1.0.41", "generation": generation,
        "inputs_manifest_sha256": INPUT_DIGEST, "manifest_sha256": "b" * 64,
        "exit_receipt_sha256": "c" * 64,
        "exit_receipt": {"version": 1, "generation": generation, "cleanup_confirmed": True,
                         "containment": runner.CONTAINMENT[target], "exit_reason": "stdio_closed",
                         "exit_code": 0, "manifest_sha256": "b" * 64},
        "observed_native_processes": 2 if name.startswith("private_leader_") else 1,
        "observed_native_processes_exited": True, "native_identity_mechanism": runner.IDENTITY[target],
        "stdout_eof_confirmed": True, "drained_tail_bytes": 0,
        "old_generation_rejected": True, "isolated_home": True, "credentials_provided": False,
        "model_commands_sent": 0, "authentication_commands_sent": 0, "acp_messages_sent": 5,
        "setup_phases": list(runner.SETUP_PHASES[:5] if name.startswith("private_leader_") else runner.SETUP_PHASES),
        "missing_cancel_ack_claimed": False, "full_adapter_lifecycle_claimed": False,
        "fixed_policy_permissions_claimed": False,
    }


def transcript(target="win32-x64"):
    values = []
    for name in runner.SCENARIOS:
        values.append({"event": "case_started", "scenario": name})
        values.extend({"event": "case_stage", "scenario": name, "stage": stage} for stage in runner.CASE_STAGES)
        values.append(case(name, target))
    return values


class ReceiptTests(unittest.TestCase):
    def test_pidfd_failure_exports_only_bounded_numeric_diagnostic(self):
        output = ("thread 'test' panicked at app/src/ai/cli_agent_runtime/codex_idle_crash_identity_tests.rs:31:13:\n"
                  "pidfd_open 失败：PRIVATE_CANARY (os error 3)\n"
                  "pidfd_open 失败：duplicate (os error 3)\n"
                  "pidfd_open 失败：invalid (os error 99999)\n")
        self.assertEqual(runner.pidfd_open_os_errors(output), [3])
        self.assertNotIn("PRIVATE_CANARY", json.dumps(runner.pidfd_open_os_errors(output)))

    def test_unrelated_output_does_not_claim_pidfd_failure(self):
        for output in ("pidfd_open 失败：unknown (os error 3)\n",
                       "thread 'test' panicked at /private/unknown.rs:31:13:\npidfd_open 失败：unknown (os error 3)\n",
                       "thread 'test' panicked at app/src/ai/cli_agent_runtime/codex_idle_crash_identity_tests.rs:31:13:\nother (os error 3)\n"):
            with self.subTest(output=output):
                self.assertEqual(runner.pidfd_open_os_errors(output), [])

    def test_four_shapes_use_platform_native_cleanup(self):
        for target in runner.CONTAINMENT:
            with self.subTest(target=target):
                cases, started = runner.project_events(transcript(target), target, INPUT_DIGEST)
                self.assertEqual(started, list(runner.SCENARIOS))
                self.assertEqual(len(cases), 4)

    def test_exit_zero_without_tree_cleanup_is_rejected(self):
        value = case(runner.SCENARIOS[0])
        value["exit_receipt"]["cleanup_confirmed"] = False
        with self.assertRaises(ValueError):
            runner.project_case(value, "win32-x64", INPUT_DIGEST)

    def test_setup_sequence_must_match_topology_without_missing_or_reordered_phases(self):
        self.assertEqual(runner.safe_failure_code(ValueError("setup_sequence_changed")), "setup_sequence_changed")
        for name in runner.SCENARIOS:
            original = case(name)
            for phases in ([], original["setup_phases"][:-1], list(reversed(original["setup_phases"])),
                           original["setup_phases"] + ["spawn_session_actor"],
                           list(runner.SETUP_PHASES if name.startswith("private_leader_") else runner.SETUP_PHASES[:5])):
                with self.subTest(scenario=name, phases=phases):
                    value = case(name)
                    value["setup_phases"] = phases
                    with self.assertRaisesRegex(ValueError, "setup_sequence_changed"):
                        runner.project_case(value, "win32-x64", INPUT_DIGEST)

    def test_stale_generation_manifest_and_wrong_containment_are_rejected(self):
        for key, replacement in (("generation", str(uuid.uuid4())), ("manifest_sha256", "d" * 64),
                                 ("containment", "unix_process_group"), ("version", True)):
            with self.subTest(key=key):
                value = case(runner.SCENARIOS[0])
                value["exit_receipt"][key] = replacement
                with self.assertRaises(ValueError):
                    runner.project_case(value, "win32-x64", INPUT_DIGEST)

    def test_private_leader_requires_native_child_and_observed_exit(self):
        for key, replacement in (("observed_native_processes", 1), ("observed_native_processes", True),
                                 ("observed_native_processes_exited", False), ("stdout_eof_confirmed", False),
                                 ("old_generation_rejected", False)):
            with self.subTest(key=key):
                value = case(runner.SCENARIOS[0])
                value[key] = replacement
                with self.assertRaises(ValueError):
                    runner.project_case(value, "win32-x64", INPUT_DIGEST)

    def test_credentials_models_and_expanded_claims_are_rejected(self):
        for key, replacement in (("credentials_provided", True), ("model_commands_sent", 1),
                                 ("authentication_commands_sent", 1), ("acp_messages_sent", 6),
                                 ("missing_cancel_ack_claimed", True), ("full_adapter_lifecycle_claimed", True),
                                 ("fixed_policy_permissions_claimed", True)):
            with self.subTest(key=key):
                value = case(runner.SCENARIOS[2])
                value[key] = replacement
                with self.assertRaises(ValueError):
                    runner.project_case(value, "win32-x64", INPUT_DIGEST)

    def test_arbitrary_native_text_cannot_enter_public_receipt(self):
        for mutate in (lambda v: v.update(native_text="PRIVATE_CANARY"),
                       lambda v: v["exit_receipt"].update(message="PRIVATE_CANARY"),
                       lambda v: v.update(native_identity_mechanism="PRIVATE_CANARY"),
                       lambda v: v.update(manifest_sha256="PRIVATE_CANARY")):
            value = case(runner.SCENARIOS[0])
            mutate(value)
            with self.assertRaises(ValueError):
                runner.project_case(value, "win32-x64", INPUT_DIGEST)

    def test_duplicate_and_reordered_scenarios_are_rejected(self):
        values = transcript()
        for changed in (values[:7] + values[:7], values[7:14] + values[:7]):
            with self.assertRaises(ValueError):
                runner.project_events(changed, "win32-x64", INPUT_DIGEST)
        values[13]["generation"] = values[6]["generation"]
        values[13]["exit_receipt"]["generation"] = values[6]["generation"]
        with self.assertRaises(ValueError):
            runner.project_events(values, "win32-x64", INPUT_DIGEST)

    def test_failed_last_case_preserves_previous_receipts(self):
        values = transcript()
        values[-1]["exit_receipt"]["cleanup_confirmed"] = False
        cases, started = runner.project_partial_events(values, "win32-x64", INPUT_DIGEST)
        self.assertEqual(len(cases), 3)
        self.assertEqual(started, list(runner.SCENARIOS))
        self.assertEqual(cases[0], values[6])

    def test_truncated_final_write_preserves_previous_complete_events(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "events.ndjson"
            complete = transcript()[:7]
            path.write_bytes(b"\n".join(json.dumps(row).encode() for row in complete) + b'\n{"event":')
            with self.assertRaises(ValueError):
                runner.read_events(path)
            events = runner.read_events(path, allow_partial=True)
            cases, started = runner.project_partial_events(events, "win32-x64", INPUT_DIGEST)
            self.assertEqual(len(cases), 1)
            self.assertEqual(started, [runner.SCENARIOS[0]])

    def test_input_manifest_digest_must_match(self):
        with self.assertRaises(ValueError):
            runner.project_case(case(runner.SCENARIOS[0]), "win32-x64", "d" * 64)

    def test_persisted_exit_must_match_safe_projection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            value = case(runner.SCENARIOS[0])
            directory = root / value["scenario"] / "state/cli-agent-processes" / value["generation"]
            directory.mkdir(parents=True)
            executable = root / "grok.exe"
            executable.write_bytes(b"offline fixture")
            manifest = {"generation": value["generation"], "launch_allowed": True, "executable": str(executable),
                        "isolated_home": str(root / value["scenario"] / "state/grok-managed" / value["generation"])}
            Path(manifest["isolated_home"]).mkdir(parents=True)
            runner.write_private(directory / "manifest.json", manifest)
            value["manifest_sha256"] = runner.digest(directory / "manifest.json")
            value["exit_receipt"]["manifest_sha256"] = value["manifest_sha256"]
            runner.write_private(directory / "exit.json", value["exit_receipt"])
            value["exit_receipt_sha256"] = runner.digest(directory / "exit.json")
            inputs = {"grok": {"path": str(executable)}}
            runner.verify_case_files(root, value, inputs)
            # 摘要有效也不能接受旧的状态域外隔离布局。
            manifest["isolated_home"] = str(root / value["scenario"] / "isolated")
            Path(manifest["isolated_home"]).mkdir()
            (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
            value["manifest_sha256"] = runner.digest(directory / "manifest.json")
            with self.assertRaisesRegex(ValueError, "persisted_manifest_binding_changed"):
                runner.verify_case_files(root, value, inputs)
            (directory / "manifest.json").write_bytes(b"{}")
            with self.assertRaises(ValueError):
                runner.verify_case_files(root, value, inputs)

    def test_manifest_accepts_same_file_alias_but_rejects_identical_copy_and_missing_object(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            value = case(runner.SCENARIOS[0])
            directory = root / value["scenario"] / "state/cli-agent-processes" / value["generation"]
            directory.mkdir(parents=True)
            home = root / value["scenario"] / "state/grok-managed" / value["generation"]
            home.mkdir(parents=True)
            executable, alias = root / "grok.exe", root / "grok-alias.exe"
            executable.write_bytes(b"offline same identity")
            alias.hardlink_to(executable)
            manifest = {"generation": value["generation"], "launch_allowed": True,
                        "executable": str(alias), "isolated_home": str(home)}
            runner.write_private(directory / "manifest.json", manifest)
            value["manifest_sha256"] = runner.digest(directory / "manifest.json")
            value["exit_receipt"]["manifest_sha256"] = value["manifest_sha256"]
            runner.write_private(directory / "exit.json", value["exit_receipt"])
            value["exit_receipt_sha256"] = runner.digest(directory / "exit.json")
            inputs = {"grok": {"path": str(executable)}}
            runner.verify_case_files(root, value, inputs)
            alias.unlink()
            alias.write_bytes(executable.read_bytes())
            with self.assertRaisesRegex(ValueError, "persisted_manifest_binding_changed"):
                runner.verify_case_files(root, value, inputs)
            alias.unlink()
            with self.assertRaisesRegex(ValueError, "persisted_manifest_binding_changed"):
                runner.verify_case_files(root, value, inputs)

    def test_minimal_environment_drops_credentials_and_proxy(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(runner.os.environ,
                {"XAI_API_KEY": "PRIVATE_CANARY", "HTTPS_PROXY": "PRIVATE_PROXY", "GROK_HOME": "PRIVATE_HOME"}, clear=True):
            environment = runner.fixed.isolated_environment(Path(temporary))
            self.assertNotIn("XAI_API_KEY", environment)
            self.assertNotIn("HTTPS_PROXY", environment)
            self.assertNotEqual(environment["GROK_HOME"], "PRIVATE_HOME")
            self.assertNotIn("PRIVATE_CANARY", json.dumps(environment))

    def test_failed_second_case_keeps_first_persisted_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            paths = {name: root / name for name in ("test_binary", "grok", "supervisor")}
            for name, path in paths.items():
                path.write_bytes(name.encode())
            args = SimpleNamespace(**paths, output=root / "receipt.json", timeout=30)
            source = {"commit": "a" * 40, "tree": "b" * 40, "source_dirty": True}

            def fake_process(command, **kwargs):
                inputs_path = Path(kwargs["env"]["INFINISHELL_GROK_SUPERVISED_EXIT_MANIFEST"])
                inputs = json.loads(inputs_path.read_bytes())
                private = Path(inputs["root"])
                value = case(runner.SCENARIOS[0])
                value["inputs_manifest_sha256"] = runner.digest(inputs_path)
                directory = private / value["scenario"] / "state/cli-agent-processes" / value["generation"]
                directory.mkdir(parents=True)
                manifest = {"generation": value["generation"], "launch_allowed": True,
                            "executable": inputs["grok"]["path"],
                            "isolated_home": str(private / value["scenario"] / "state/grok-managed" / value["generation"])}
                Path(manifest["isolated_home"]).mkdir(parents=True)
                runner.write_private(directory / "manifest.json", manifest)
                value["manifest_sha256"] = runner.digest(directory / "manifest.json")
                value["exit_receipt"]["manifest_sha256"] = value["manifest_sha256"]
                runner.write_private(directory / "exit.json", value["exit_receipt"])
                value["exit_receipt_sha256"] = runner.digest(directory / "exit.json")
                events = [{"event": "case_started", "scenario": runner.SCENARIOS[0]}]
                events.extend({"event": "case_stage", "scenario": runner.SCENARIOS[0], "stage": stage} for stage in runner.CASE_STAGES)
                events.extend([value, {"event": "case_started", "scenario": runner.SCENARIOS[1]},
                               {"event": "case_stage", "scenario": runner.SCENARIOS[1], "stage": "startup"}])
                (private / "events.private.ndjson").write_text(
                    "\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8")
                kwargs["stdout"].write(b"test result: FAILED. 0 passed; 1 failed;\n")
                return SimpleNamespace(wait=lambda timeout: 101)

            def version(binary, directory, version):
                self.assertTrue(directory.is_dir())
                self.assertEqual(version, "1.0.41")

            with mock.patch.object(runner.fixed, "current_platform", return_value="win32-x64"), \
                    mock.patch.object(runner.fixed, "verify_binary", return_value={"sha256": "f" * 64}), \
                    mock.patch.object(runner.fixed, "verify_version", side_effect=version), \
                    mock.patch.object(runner, "source_identity", return_value=source), \
                    mock.patch.object(runner.subprocess, "run", return_value=SimpleNamespace(
                        returncode=0, stdout=(runner.TEST_NAME + ": test\n").encode())), \
                    mock.patch.object(runner.subprocess, "Popen", side_effect=fake_process):
                report = runner.run(args)
            self.assertFalse(report["passed"])
            self.assertEqual(report["test_exit_code"], 101)
            self.assertEqual(report["failure_code"], "test_did_not_complete_four_cases")
            self.assertEqual(report["case_progress"][-1], {"scenario": runner.SCENARIOS[1], "stage": "startup"})
            self.assertEqual(len(report["cases"]), 1)
            self.assertEqual(report["started_scenarios"], list(runner.SCENARIOS[:2]))
            self.assertEqual(json.loads(args.output.read_bytes())["cases"], report["cases"])
            self.assertNotIn("PRIVATE_CANARY", args.output.read_text())

    def test_failure_diagnostics_publish_only_codes_and_source_locations(self):
        self.assertEqual(runner.safe_failure_code(ValueError("test_not_listed")), "test_not_listed")
        self.assertEqual(runner.safe_failure_code(ValueError("PRIVATE_CANARY")), "unclassified_safe_error")
        path = "app/src/ai/cli_agent_runtime/grok_supervised_exit_live_tests.rs"
        locations = runner.panic_locations("thread test panicked at " + path + ":42:3:\nPRIVATE_CANARY\n")
        self.assertEqual(locations, [{"path": path, "line": 42, "column": 3}])
        self.assertNotIn("PRIVATE_CANARY", json.dumps(locations))
        self.assertEqual(runner.panic_locations("panicked at app/PRIVATE_CANARY.rs:42:3:"), [])

    def test_existing_output_is_rejected_before_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output = root / "receipt.json"
            output.write_text("preserved", encoding="utf-8")
            args = mock.Mock(output=output, timeout=300)
            with mock.patch.object(runner.subprocess, "Popen") as start:
                with self.assertRaises(ValueError):
                    runner.run(args)
                start.assert_not_called()
            self.assertEqual(output.read_text(), "preserved")


if __name__ == "__main__":
    unittest.main()
