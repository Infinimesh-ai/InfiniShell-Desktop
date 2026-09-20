"""技能黑盒证据的离线回归；不运行 CLI、模型或读取认证。"""

import copy
import hashlib
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest import mock

import run_grok_native_skill as runner


SENTINEL = "INFINISHELL_SKILL_" + "a" * 64
DIGEST = hashlib.sha256(SENTINEL.encode()).hexdigest()


def evidence(case="visible"):
    positive = case == "visible"
    end = {"event": "skill_finished", "scope": runner.SCOPE, "case": case,
        "passed": True, "production_connect_path": True, "test_argv_override": True,
        "product_selected_skills_verified": False, "final_history_verified": True,
        "exact_skill_read_only_verified": True, "native_skill_read_count": 0,
        "native_skill_expansion_verified": positive, "hidden_control_verified": not positive,
        "sentinel_matched": positive, "sentinel_sha256": DIGEST,
        "final_response_sha256": DIGEST if positive else hashlib.sha256(b"SKILL_UNAVAILABLE").hexdigest(),
        "submitted_input_count": 1, "accepted_input_count": 1, "approval_count": 0,
        "native_tool_event_count": 0, "transport_closed": True, "cleanup_confirmed": True,
        "skill_snapshot_unchanged": True, "full_cli_parity_acceptance_passed": False}
    return [runner.started(case), end]


class NativeSkillRunnerTests(unittest.TestCase):
    def test_tunnel_close_error_still_removes_auth_and_publishes_only_error_type(self):
        from contextlib import ExitStack, nullcontext, redirect_stdout
        import io
        from types import SimpleNamespace
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary); root = base / "workspace"; root.mkdir()
            binary = base / "binary"; binary.write_bytes(b"offline-binary")
            args = SimpleNamespace(output=base / "safe.ndjson", grok=binary, test_binary=binary,
                supervisor=binary, official_grok_home=base / "source", timeout=30, case="visible")
            tunnel = SimpleNamespace(start=lambda: 1234, forwarded=0, bytes=0, events=[])
            def close():
                raise OSError("OFFLINE_PRIVATE_CLOSE_CANARY")
            tunnel.close = close
            def copy_auth(source, target):
                (target / "auth.json").write_text("OFFLINE_PRIVATE_AUTH_CANARY", encoding="utf-8")
            with ExitStack() as stack:
                stack.enter_context(patch.object(runner.tempfile, "mkdtemp", return_value=str(root)))
                stack.enter_context(patch.object(runner.isolation, "reserve_artifacts"))
                stack.enter_context(patch.object(runner.lease, "auth_identity", return_value=(1, 2, 3)))
                stack.enter_context(patch.object(runner.isolation, "bounded_tunnel", return_value=nullcontext(tunnel)))
                stack.enter_context(patch.object(runner.official, "copy_private_auth", side_effect=copy_auth))
                stack.enter_context(patch.object(runner.subprocess, "Popen", side_effect=AssertionError("不应启动进程")))
                stack.enter_context(patch.object(runner.shared, "network_canary", side_effect=OSError("OFFLINE_PRIVATE_STAGE_CANARY")))
                stack.enter_context(patch.object(runner.isolation, "private_events", return_value=[]))
                stack.enter_context(redirect_stdout(io.StringIO()))
                self.assertEqual(runner.run(args), 1)
            self.assertFalse((root / "home/.grok/auth.json").exists())
            text = args.output.with_suffix(".metadata.json").read_text(encoding="utf-8")
            metadata = json.loads(text)
            self.assertEqual(metadata["tunnel_cleanup_error_type"], "OSError")
            self.assertFalse(metadata["tunnels_stopped"])
            self.assertTrue(metadata["private_auth_copy_removed"])
            self.assertFalse(metadata["boundary_passed"])
            self.assertFalse(metadata["case_passed"])
            self.assertNotIn("OFFLINE_PRIVATE", text)

    def test_evidence_reader_accepts_legacy_and_three_event_catalog_contracts(self):
        catalog = {"event": "skill_catalog_observed", "scope": runner.SCOPE, "case": "visible",
            "overflow": False, "unassociated_snapshot_count": 0, "snapshots": [{
                "selected_name": "infinishell-native-skill", "command_count": 8, "selected_name_count": 1,
                "before_first_submit": True, "metadata_present": True, "path_present": True,
                "path_absolute": True, "path_matches_selected": True, "metadata_scope": "local",
                "bare_name_matches": True, "qualified_name_matches": False}]}
        path = Path("offline-events.ndjson")
        for include_catalog in (False, True):
            events = evidence()
            if include_catalog:
                events.insert(1, catalog)
            raw = ("\n".join(json.dumps(event) for event in events) + "\n").encode()
            with mock.patch.object(runner.isolation, "private_bytes", return_value=raw) as read:
                loaded = runner.read_events(path)
                read.assert_called_once_with(path, 128 * 1024)
                self.assertEqual(loaded, events)
                result = runner.observation(0, loaded, "visible", DIGEST)
                self.assertTrue(result["case_passed"])
                self.assertEqual(result["native_catalog_selected_path_verified"], include_catalog)
                self.assertEqual(result["native_catalog_selected_path_seen_before_input"], include_catalog)
                self.assertFalse(result["product_selected_skills_verified"])
                self.assertFalse(result["full_cli_parity_acceptance_passed"])
                if include_catalog:
                    with self.assertRaises(ValueError):
                        runner.lease.read_events(path)

    def test_evidence_reader_rejects_extra_events_duplicate_keys_and_nonfinite_numbers(self):
        for body in ('{}\n' * 4, '[]\n', '{"event":"first","event":"second"}\n',
                     '{"nested":{"counter":1,"counter":2}}\n',
                     '{"counter":NaN}\n', '{"counter":Infinity}\n', '{"counter":-Infinity}\n'):
            with self.subTest(body=body), mock.patch.object(runner.isolation, "private_bytes", return_value=body.encode()):
                with self.assertRaises(ValueError):
                    runner.read_events(Path("offline-events.ndjson"))
        with mock.patch.object(runner.isolation, "private_bytes", return_value=b"\xff"):
            with self.assertRaises(ValueError):
                runner.read_events(Path("offline-events.ndjson"))
        with mock.patch.object(runner.isolation, "private_bytes", side_effect=ValueError("大小预算")) as read:
            with self.assertRaisesRegex(ValueError, "大小预算"):
                runner.read_events(Path("offline-events.ndjson"))
            read.assert_called_once_with(Path("offline-events.ndjson"), 128 * 1024)

    def test_evidence_reader_does_not_bypass_safe_projection_or_sequence_validation(self):
        for case in ("private_field", "duplicate_finish", "missing_start"):
            events = evidence()
            if case == "private_field":
                events[-1]["raw_model_text"] = "OFFLINE_PRIVATE_TEXT_CANARY"
            elif case == "duplicate_finish":
                events.append(copy.deepcopy(events[-1]))
            else:
                events = events[1:]
            raw = ("\n".join(json.dumps(event) for event in events) + "\n").encode()
            with mock.patch.object(runner.isolation, "private_bytes", return_value=raw):
                loaded = runner.read_events(Path("offline-events.ndjson"))
            self.assertFalse(runner.observation(0, loaded, "visible", DIGEST)["case_passed"])
            if case == "private_field":
                self.assertFalse(all(runner.validate_event(event) for event in loaded))

    @unittest.skipUnless(os.name == "posix", "真实证据文件权限与 O_NOFOLLOW 是 POSIX 验证")
    def test_evidence_real_file_permissions_symlink_and_size_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "events.ndjson"
            path.touch(mode=0o600)
            events = evidence()
            events.insert(1, {"event": "skill_catalog_observed", "scope": runner.SCOPE,
                "case": "visible", "overflow": False, "unassociated_snapshot_count": 0, "snapshots": []})
            path.write_text("\n".join(json.dumps(event) for event in events) + "\n")
            self.assertEqual(runner.read_events(path), events)
            link = Path(temporary) / "linked.ndjson"
            link.symlink_to(path)
            with self.assertRaises(OSError):
                runner.read_events(link)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                runner.read_events(path)
            path.chmod(0o600)
            path.write_bytes(b" " * (128 * 1024 + 1))
            with self.assertRaises(ValueError):
                runner.read_events(path)

    def test_catalog_projection_records_only_matched_native_path_facts(self):
        event = {"event": "skill_catalog_observed", "scope": runner.SCOPE, "case": "visible",
            "overflow": False, "unassociated_snapshot_count": 0, "snapshots": [{
                "selected_name": "infinishell-native-skill", "command_count": 1, "selected_name_count": 1,
                "before_first_submit": True, "metadata_present": True, "path_present": True,
                "path_absolute": True, "path_matches_selected": True, "metadata_scope": "local",
                "bare_name_matches": True, "qualified_name_matches": True}]}
        events = evidence()
        events.insert(1, event)
        result = runner.observation(0, events, "visible", DIGEST)
        self.assertTrue(result["case_passed"])
        self.assertTrue(result["native_catalog_selected_path_verified"])
        self.assertTrue(result["native_catalog_selected_path_seen_before_input"])
        self.assertFalse(result["product_selected_skills_verified"])
        for key in ("path", "description", "input", "session_id"):
            changed = copy.deepcopy(events)
            changed[1]["snapshots"][0][key] = "private value"
            self.assertFalse(runner.validate_event(changed[1]))
        event["snapshots"][0]["before_first_submit"] = False
        self.assertFalse(runner.observation(0, events, "visible", DIGEST)["native_catalog_selected_path_seen_before_input"])
        event["overflow"] = True
        self.assertFalse(runner.observation(0, events, "visible", DIGEST)["native_catalog_selected_path_verified"])
        event["overflow"] = False
        event["snapshots"][0]["selected_name_count"] = 2
        self.assertFalse(runner.validate_event(event))
        self.assertFalse(runner.observation(0, evidence(), "visible", DIGEST)["native_catalog_observed"])

    def test_catalog_unknown_values_replay_order_and_bool_counts_are_rejected(self):
        event = {"event": "skill_catalog_observed", "scope": runner.SCOPE, "case": "visible",
            "overflow": False, "unassociated_snapshot_count": 0, "snapshots": []}
        events = evidence()
        events.insert(1, event)
        self.assertTrue(runner.observation(0, events, "visible", DIGEST)["case_passed"])
        self.assertFalse(runner.observation(0, events + [event], "visible", DIGEST)["case_passed"])
        self.assertFalse(runner.observation(0, [events[0], events[2], event], "visible", DIGEST)["case_passed"])
        event["unassociated_snapshot_count"] = True
        self.assertFalse(runner.validate_event(event))

    def test_visible_proves_only_isolated_native_skill(self):
        result = runner.observation(0, evidence(), "visible", DIGEST)
        self.assertTrue(result["case_passed"])
        self.assertTrue(result["native_skill_expansion_verified"])
        self.assertFalse(result["product_selected_skills_verified"])
        self.assertFalse(result["full_cli_parity_acceptance_passed"])

    def test_hidden_negative_does_not_claim_positive_expansion(self):
        result = runner.observation(0, evidence("hidden"), "hidden", DIGEST)
        self.assertTrue(result["hidden_control_verified"])
        self.assertFalse(result["native_skill_expansion_verified"])
        self.assertFalse(runner.observation(0, evidence("hidden"), "visible", DIGEST)["case_passed"])

    def test_tool_or_approval_without_verified_skill_read_invalidates_model_answer(self):
        for key in ("native_tool_event_count", "approval_count"):
            with self.subTest(key=key):
                events = evidence()
                events[-1][key] = 1
                self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])

    def test_visible_accepts_one_exact_skill_read_with_optional_once_approval(self):
        for approvals in (0, 1):
            events = evidence()
            events[-1].update(native_skill_read_count=1, native_tool_event_count=3, approval_count=approvals)
            self.assertTrue(runner.observation(0, events, "visible", DIGEST)["case_passed"])
        events[-1]["exact_skill_read_only_verified"] = False
        self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])

    def test_hidden_or_second_file_read_cannot_pass(self):
        for case, reads in (("hidden", 1), ("visible", 2)):
            events = evidence(case)
            events[-1].update(native_skill_read_count=reads, native_tool_event_count=3)
            self.assertFalse(runner.observation(0, events, case, DIGEST)["case_passed"])

    def test_unrelated_or_missing_random_sentinel_cannot_pass(self):
        self.assertFalse(runner.observation(0, evidence(), "visible", "b" * 64)["case_passed"])
        events = evidence()
        events[-1]["final_response_sha256"] = "b" * 64
        self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])

    def test_failed_runtime_partial_or_duplicate_history_cannot_pass(self):
        for events in ([], evidence()[:1], evidence() + evidence()[-1:]):
            self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])
        self.assertFalse(runner.observation(101, evidence(), "visible", DIGEST)["case_passed"])
        events = evidence()
        events[-1]["final_history_verified"] = False
        self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])

    def test_secret_in_prompt_or_unknown_public_field_is_rejected(self):
        events = evidence()
        events[0]["secret_in_submitted_prompt"] = True
        self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])
        events = evidence()
        events[-1]["raw_model_text"] = SENTINEL
        self.assertFalse(runner.validate_event(events[-1]))

    def test_bool_or_multiple_inputs_cannot_pass(self):
        for value in (True, 2, -1, 1.0):
            events = evidence()
            events[-1]["submitted_input_count"] = value
            self.assertFalse(runner.observation(0, events, "visible", DIGEST)["case_passed"])

    def test_skill_secret_exists_only_in_body_and_case_controls_visibility(self):
        for case in ("visible", "hidden"):
            content = runner.skill_document(case, SENTINEL)
            frontmatter = content.split("---\n")[1]
            self.assertNotIn(SENTINEL, frontmatter)
            self.assertNotIn(SENTINEL, runner.PROMPT)
            self.assertEqual(content.count(SENTINEL), 1)
            self.assertIn("user-invocable: " + ("true" if case == "visible" else "false"), frontmatter)
            self.assertIn("disable-model-invocation: true", frontmatter)

    def test_project_snapshot_rejects_added_file_or_changed_skill(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / runner.SKILL_PATH
            path.parent.mkdir(parents=True)
            content = runner.skill_document("visible", SENTINEL).encode()
            path.write_bytes(content)
            self.assertTrue(runner.project_unchanged(root, content))
            path.write_bytes(content + b"changed")
            self.assertFalse(runner.project_unchanged(root, content))
            path.write_bytes(content)
            (root / "project/extra").touch()
            self.assertFalse(runner.project_unchanged(root, content))

    def test_project_snapshot_uses_platform_path_separators(self):
        for path_type in (PurePosixPath, PureWindowsPath):
            with self.subTest(path_type=path_type):
                root = mock.MagicMock()
                project = root.__truediv__.return_value
                project.read_bytes.return_value = b"offline skill"
                entries = []
                for relative in (".grok", ".grok/skills", ".grok/skills/infinishell-native-skill",
                                 ".grok/skills/infinishell-native-skill/SKILL.md"):
                    entry = mock.Mock()
                    entry.relative_to.return_value = path_type(relative)
                    entry.is_symlink.return_value = False
                    entries.append(entry)
                project.rglob.return_value = entries
                with mock.patch.object(runner, "Path", path_type), \
                        mock.patch.object(runner, "SKILL_PATH", path_type("project/.grok/skills/infinishell-native-skill/SKILL.md")):
                    self.assertTrue(runner.project_unchanged(root, b"offline skill"))

    def test_wrapper_preserves_profile_and_explicitly_adds_only_private_project_trust(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "home/.grok").mkdir(parents=True)
            profile = root / "profile.md"
            profile.write_text("offline fixture")
            wrapper, _ = runner.prepare_native(root, Path("/offline/grok"), root / "source", 1, profile)
            code = wrapper.read_text()
            self.assertIn("--agent-profile", code)
            self.assertIn(runner.shared.digest(profile), code)
            self.assertIn("kind='native_skill'", code)
            self.assertNotIn("kind='direct_agent'", code)
            self.assertNotIn("kind='private_leader'", code)
            self.assertIn("str(native),*(['--trust',*args] if kind=='native_skill' else args)", code)
            self.assertNotIn(SENTINEL, code)

    def test_wrapper_trust_requires_private_cwd_and_both_private_home_variables(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "home/.grok").mkdir(parents=True)
            (root / "project").mkdir()
            native = root / "offline-native"
            native.write_bytes(b"offline executable fixture")
            profile = root / "profile.md"
            profile.write_text("offline profile")
            with mock.patch.object(runner.shared, "BINARY_SHA256", runner.shared.digest(native)):
                wrapper, _ = runner.prepare_native(root, native, root / "source", 1, profile)
            original = ["agent", "--no-leader", "--agent-profile", str(profile), "stdio"]
            code = compile(wrapper.read_text(), str(wrapper), "exec")
            expected_home = {"HOME": str(root / "home"), "GROK_HOME": str(root / "home/.grok")}
            for change in (None, "cwd", "HOME", "GROK_HOME", "missing_home", "home_symlink"):
                if change == "home_symlink" and os.name != "posix":
                    with self.subTest(change=change):
                        self.skipTest("此分支验证 POSIX 私有 HOME 符号链接；不要求 Windows 创建链接权限")
                    continue
                environment = dict(expected_home)
                cwd = root / "project"
                if change in ("HOME", "GROK_HOME"):
                    environment[change] = str(root / "external")
                elif change == "missing_home":
                    del environment["HOME"]
                elif change == "cwd":
                    cwd = root
                elif change == "home_symlink":
                    (root / "home").rename(root / "moved-home")
                    (root / "home").symlink_to(root / "moved-home", target_is_directory=True)
                try:
                    with mock.patch.object(sys, "argv", [str(wrapper), *original]), \
                            mock.patch.dict(os.environ, environment, clear=True), \
                            mock.patch.object(Path, "cwd", return_value=cwd), \
                            mock.patch.object(os, "execv") as launch:
                        if change is None:
                            exec(code, {})
                            self.assertEqual(launch.call_args.args[1][3:], [str(native), "--trust", *original])
                            row = json.loads((root / "wrapper-audit.ndjson").read_text().splitlines()[-1])
                            self.assertFalse(row["arguments_unchanged"])
                            self.assertTrue(row["synthetic_project_trust_requested"])
                        else:
                            with self.assertRaises(SystemExit) as rejected:
                                exec(code, {})
                            self.assertEqual(rejected.exception.code, 96)
                            launch.assert_not_called()
                finally:
                    if change == "home_symlink":
                        (root / "home").unlink()
                        (root / "moved-home").rename(root / "home")
            with mock.patch.object(sys, "argv", [str(wrapper), "--version"]), \
                    mock.patch.object(os, "execv") as launch:
                exec(code, {})
                self.assertEqual(launch.call_args.args[1][3:], [str(native), "--version"])
                row = json.loads((root / "wrapper-audit.ndjson").read_text().splitlines()[-1])
                self.assertTrue(row["arguments_unchanged"])
                self.assertFalse(row["synthetic_project_trust_requested"])

    def test_boundary_requires_profile_project_auth_and_network_receipts(self):
        metadata = {key: True for key in ("tunnels_stopped", "private_auth_copy_removed",
            "original_auth_stat_unchanged", "profile_snapshot_unchanged", "project_snapshot_unchanged")}
        metadata.update(test_exit_code=0, private_settings_audit={"settings_scope_verified": True},
            synthetic_project_trust_requested=True, project_trust_scope="synthetic_project_only",
            project_trust_flag="--trust", trust_store_scope="private_home_only")
        launches = [{"kind": kind, "arguments_unchanged": kind != "native_skill",
            "synthetic_project_trust_requested": kind == "native_skill"}
            for kind in ("version", "version", "native_skill")]
        tunnel = SimpleNamespace(forwarded=2, bytes=1024,
            events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        self.assertTrue(runner.boundary_passed(metadata, launches, tunnel))
        for key in ("profile_snapshot_unchanged", "project_snapshot_unchanged", "original_auth_stat_unchanged"):
            changed = copy.deepcopy(metadata)
            changed[key] = False
            self.assertFalse(runner.boundary_passed(changed, launches, tunnel))
        for key in ("arguments_unchanged", "synthetic_project_trust_requested"):
            changed = copy.deepcopy(launches)
            changed[-1][key] = not changed[-1][key]
            self.assertFalse(runner.boundary_passed(metadata, changed, tunnel))
        changed = copy.deepcopy(metadata)
        changed["project_trust_scope"] = "user_project"
        self.assertFalse(runner.boundary_passed(changed, launches, tunnel))
        tunnel.bytes = runner.lease.MAX_TLS_BYTES + 1
        self.assertFalse(runner.boundary_passed(metadata, launches, tunnel))


if __name__ == "__main__":
    unittest.main()
