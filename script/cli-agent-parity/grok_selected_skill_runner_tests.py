"""默认入口技能组合运行器的离线合同；不调用 CLI、模型或真实认证。"""

from contextlib import contextmanager
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import run_grok_selected_skill as runner

SKILL = "INFINISHELL_SKILL_" + "a" * 64
CONTEXT = "INFINISHELL_CONTEXT_" + "b" * 64
EXPECTED = hashlib.sha256(f"{SKILL}\n{CONTEXT}\nREVIEW_CHECK".encode()).hexdigest()


def evidence(mode="leader"):
    row = {"selected_name": "infinishell-native-skill", "command_count": 25, "selected_name_count": 1,
        "before_first_submit": True, "metadata_present": True, "path_present": True, "path_absolute": True,
        "path_matches_selected": True, "metadata_scope": "local", "bare_name_matches": True,
        "qualified_name_matches": False}
    catalog = {"event": "skill_catalog_observed", "scope": runner.SCOPE, "case": mode,
        "snapshots": [row], "overflow": False, "unassociated_snapshot_count": 0}
    end = {key: True for key in runner.BOOLS}
    end.update({key: 1 for key in runner.COUNTERS})
    end.update(event="selected_skill_finished", scope=runner.SCOPE, case=mode,
        test_agent_profile_override=False, full_cli_parity_acceptance_passed=False,
        expected_sha256=EXPECTED, final_response_sha256=EXPECTED)
    return [runner.started(mode), catalog, end]


class SelectedSkillRunnerTests(unittest.TestCase):
    def test_complete_file_evidence_proves_only_one_default_entry_combination(self):
        raw = "\n".join(json.dumps(row) for row in evidence()).encode()
        with mock.patch.object(runner.isolation, "private_bytes", return_value=raw) as read:
            rows = runner.native.read_events(Path("offline.ndjson"))
        read.assert_called_once_with(Path("offline.ndjson"), 128 * 1024)
        result = runner.observation(0, rows, "leader", EXPECTED)
        self.assertTrue(result["case_passed"])
        self.assertTrue(result["product_selected_skill_combination_verified"])
        self.assertFalse(result["gui_composer_verified"])
        self.assertFalse(result["full_cli_parity_acceptance_passed"])
        self.assertTrue(runner.observation(0, evidence("sdk"), "sdk", EXPECTED)["case_passed"])
        self.assertFalse(runner.observation(0, evidence("sdk"), "leader", EXPECTED)["case_passed"])

    def test_catalog_after_input_or_wrong_path_duplicate_scope_or_overflow_cannot_pass(self):
        for key, value in (("before_first_submit", False), ("path_matches_selected", False),
                           ("bare_name_matches", False), ("selected_name_count", 2), ("metadata_scope", "private text")):
            rows = evidence()
            rows[1]["snapshots"][0][key] = value
            self.assertFalse(runner.observation(0, rows, "leader", EXPECTED)["case_passed"])
        rows = evidence()
        rows[1]["overflow"] = True
        self.assertFalse(runner.observation(0, rows, "leader", EXPECTED)["case_passed"])

    def test_missing_native_read_wrong_result_or_second_input_cannot_pass(self):
        for key, value in (("native_context_read_count", 0), ("native_tool_event_count", 0),
                           ("submitted_input_count", 2), ("accepted_input_count", True),
                           ("final_response_sha256", "f" * 64), ("approval_count", 2),
                           ("test_agent_profile_override", True), ("final_history_verified", False)):
            rows = evidence()
            rows[-1][key] = value
            self.assertFalse(runner.observation(0, rows, "leader", EXPECTED)["case_passed"])
        self.assertFalse(runner.observation(101, evidence(), "leader", EXPECTED)["case_passed"])
        self.assertFalse(runner.observation(0, evidence(), "leader", "f" * 64)["case_passed"])

    def test_unknown_public_fields_replays_and_truncation_are_rejected(self):
        rows = evidence()
        rows[1]["snapshots"][0]["path"] = "private path"
        self.assertFalse(runner.validate_event(rows[1]))
        rows = evidence()
        rows[-1]["native_text"] = "private body"
        self.assertFalse(runner.validate_event(rows[-1]))
        self.assertFalse(runner.observation(0, evidence() * 2, "leader", EXPECTED)["case_passed"])
        self.assertFalse(runner.observation(0, evidence()[:2], "leader", EXPECTED)["case_passed"])

    def test_non_string_case_is_rejected_without_raising_or_publishing_success(self):
        for value in ([], {}, ["leader"], {"mode": "leader"}, None, True, 1, 1.0):
            for index in range(3):
                with self.subTest(value=value, event_index=index):
                    rows = evidence()
                    rows[index]["case"] = value
                    self.assertFalse(runner.validate_event(rows[index]))
                    self.assertFalse(runner.observation(0, rows, "leader", EXPECTED)["case_passed"])

    def test_skill_document_matches_the_frozen_rust_fixture_without_secret_in_description(self):
        source = Path(__file__).resolve().parents[2] / "app/src/ai/cli_agent_runtime/grok_selected_skill_live_tests.rs"
        text = source.read_text(encoding="utf-8")
        match = re.search(r'fn skill_document\(sentinel: &str\) -> String \{\s*format!\(\s*("(?:[^"\\]|\\.)*")', text)
        self.assertIsNotNone(match)
        rust_template = json.loads(match.group(1))
        self.assertEqual(runner.skill_document(SKILL), rust_template.replace("{sentinel}", SKILL))
        self.assertNotIn(SKILL, runner.skill_document(SKILL).split("---\n")[1])
        self.assertNotIn(CONTEXT, runner.skill_document(SKILL))

    def test_project_snapshot_rejects_mutated_added_or_replaced_context(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / runner.SKILL_PATH.parent).mkdir(parents=True)
            snapshot = {runner.SKILL_PATH: runner.skill_document(SKILL).encode(), runner.CONTEXT_PATH: (CONTEXT + "\n").encode()}
            for relative, body in snapshot.items():
                (root / relative).write_bytes(body)
            self.assertTrue(runner.project_unchanged(root, snapshot))
            (root / runner.CONTEXT_PATH).write_text("changed", encoding="utf-8")
            self.assertFalse(runner.project_unchanged(root, snapshot))
            (root / runner.CONTEXT_PATH).write_bytes(snapshot[runner.CONTEXT_PATH])
            (root / "project/extra").touch()
            self.assertFalse(runner.project_unchanged(root, snapshot))

    def test_boundary_requires_native_argv_receipt_cleanup_network_and_no_extra_launch(self):
        metadata = {key: True for key in ("tunnels_stopped", "private_auth_copy_removed", "original_auth_stat_unchanged", "project_snapshot_unchanged")}
        metadata.update(test_exit_code=0, private_settings_audit={"settings_scope_verified": True})
        tunnel = SimpleNamespace(forwarded=1, bytes=20, events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        launches = [{"kind": kind, "arguments_unchanged": kind == "version", "synthetic_project_trust_requested": kind != "version"}
                    for kind in ("version", "version", "private_leader")]
        self.assertTrue(runner.boundary_passed(metadata, launches, tunnel, "leader"))
        self.assertFalse(runner.boundary_passed(metadata, launches, tunnel, "sdk"))
        self.assertFalse(runner.boundary_passed(metadata, launches + launches[-1:], tunnel, "leader"))
        changed = dict(metadata, tunnels_stopped=False)
        self.assertFalse(runner.boundary_passed(changed, launches, tunnel, "leader"))
        tunnel.bytes = runner.lease.MAX_TLS_BYTES + 1
        self.assertFalse(runner.boundary_passed(metadata, launches, tunnel, "leader"))

    def test_current_native_marketplace_purge_is_the_only_extra_settings_change(self):
        before = b"[cli]\nuse_leader = true\n[permission]\nmode = 'ask'\n"
        after = before + b"[marketplace]\ndefault_skills_installs_purged = true\n"
        audit = runner.audit_current_settings(before, after)
        self.assertTrue(audit["native_marketplace_purge_only"])
        self.assertTrue(audit["settings_scope_verified"])
        for changed in (
            before + b"[marketplace]\ndefault_skills_installs_purged = false\n",
            before + b"[marketplace]\ndefault_skills_installs_purged = true\nextra = true\n",
            b"[cli]\nuse_leader = false\n[permission]\nmode = 'ask'\n[marketplace]\ndefault_skills_installs_purged = true\n",
        ):
            audit = runner.audit_current_settings(before, changed)
            self.assertFalse(audit["native_marketplace_purge_only"])
            self.assertFalse(audit["settings_scope_verified"])

    @unittest.skipUnless(os.name == "posix", "私有 HOME 与 leader socket 的原生目录权限为 POSIX 合同")
    def test_wrapper_accepts_only_default_mode_and_trusts_only_its_synthetic_project(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for part in ("home/.grok", "project", "tmp/leader"):
                (root / part).mkdir(parents=True, mode=0o700)
            native = root / "offline-native"
            native.write_bytes(b"offline native")
            environment = {"HOME": str(root / "home"), "GROK_HOME": str(root / "home/.grok")}
            for mode, argv in (("leader", ["agent", "stdio", "--leader-socket", str(root / "tmp/leader/leader.sock")]),
                               ("sdk", ["agent", "--no-leader", "stdio"])):
                with mock.patch.dict(runner.CURRENT, sha256=runner.shared.digest(native)), \
                        mock.patch.object(runner.shared, "BINARY_SHA256", runner.shared.digest(native)):
                    wrapper, _ = runner.prepare_native(root, native, root / "source", 1, mode)
                code = compile(wrapper.read_text(encoding="utf-8"), str(wrapper), "exec")
                with mock.patch.object(sys, "argv", [str(wrapper), *argv]), mock.patch.dict(os.environ, environment, clear=True), \
                        mock.patch.object(Path, "cwd", return_value=root / "project"), mock.patch.object(os, "execv") as launch:
                    exec(code, {})
                    self.assertEqual(launch.call_args.args[1][3:], [str(native), "--trust", *argv])
                    row = json.loads((root / "wrapper-audit.ndjson").read_text(encoding="utf-8").splitlines()[-1])
                    self.assertFalse(row["arguments_unchanged"])
                    self.assertTrue(row["synthetic_project_trust_requested"])
                with mock.patch.object(sys, "argv", [str(wrapper), *argv]), mock.patch.dict(os.environ, environment, clear=True), \
                        mock.patch.object(Path, "cwd", return_value=root), mock.patch.object(os, "execv") as launch:
                    with self.assertRaises(SystemExit):
                        exec(code, {})
                    launch.assert_not_called()
                with mock.patch.object(sys, "argv", [str(wrapper), "agent", "--agent-profile", "fake", "stdio"]), mock.patch.object(os, "execv") as launch:
                    with self.assertRaises(SystemExit):
                        exec(code, {})
                    launch.assert_not_called()

    def test_tunnel_close_exception_does_not_skip_auth_copy_cleanup_or_publish_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            root = directory / "private"
            root.mkdir()
            original_home = directory / "original"
            original_home.mkdir()
            executable = directory / "offline-executable"
            executable.write_bytes(b"offline binary")
            args = SimpleNamespace(output=directory / "result.ndjson", mode="leader", timeout=450,
                official_grok_home=original_home, grok=executable, test_binary=executable, supervisor=executable)
            tunnel = SimpleNamespace(deadline=10**30, forwarded=0, bytes=20, events=[], start=lambda: 1,
                close=mock.Mock(side_effect=OSError("OFFLINE_PRIVATE_ERROR")))
            @contextmanager
            def bounded(timeout):
                yield tunnel
            def prepare(*unused):
                settings = root / "home/.grok/config.toml"
                settings.write_text("[cli]\nuse_leader=true\n", encoding="utf-8")
                return root / "wrapper", settings
            def popen(*unused, **kwargs):
                tunnel.forwarded = 1
                tunnel.events.append({"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"})
                return SimpleNamespace(returncode=0, wait=lambda **ignored: None)
            launches = [{"kind": kind, "arguments_unchanged": kind == "version", "synthetic_project_trust_requested": kind != "version"}
                        for kind in ("version", "version", "private_leader")]
            with mock.patch.object(runner.tempfile, "mkdtemp", return_value=str(root)), \
                    mock.patch.object(runner.secrets, "token_hex", side_effect=["a" * 64, "b" * 64]), \
                    mock.patch.object(runner.lease, "auth_identity", return_value=(1,)), \
                    mock.patch.object(runner.isolation, "bounded_tunnel", side_effect=bounded), \
                    mock.patch.object(runner.official, "copy_private_auth", side_effect=lambda source, target: (target / "auth.json").write_text("opaque offline", encoding="utf-8")), \
                    mock.patch.object(runner.shared, "network_canary", return_value={"allowed_proxy": True}), \
                    mock.patch.object(runner.isolation, "project_write_canary", return_value={"project_write": 1}), \
                    mock.patch.object(runner, "prepare_native", side_effect=prepare), \
                    mock.patch.object(runner.official, "official_environment", return_value={}), \
                    mock.patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout=runner.shared.VERSION)), \
                    mock.patch.object(runner.subprocess, "Popen", side_effect=popen), \
                    mock.patch.object(runner.native, "read_events", return_value=evidence()), \
                    mock.patch.object(runner.isolation, "private_events", return_value=launches):
                self.assertEqual(runner.run(args), 1)
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text(encoding="utf-8"))
            self.assertTrue(metadata["private_auth_copy_removed"])
            self.assertFalse((root / "home/.grok/auth.json").exists())
            self.assertFalse(metadata["case_passed"])
            self.assertFalse(metadata["default_entrypoint_verified"])
            self.assertFalse(metadata["product_selected_skill_combination_verified"])
            self.assertEqual(metadata["tunnel_close_error_type"], "OSError")
            self.assertNotIn("OFFLINE_PRIVATE_ERROR", args.output.with_suffix(".metadata.json").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
