#!/usr/bin/env python3
"""Claude 真实验收运行器的离线边界测试，不调用 CLI 或网络。"""

import argparse
import copy
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import run_claude_adapter_live as runner
from run_claude_adapter_live import (
    MARKER, PROJECT_SETTINGS, authenticated_environment, authorized_account_summary,
    authorized_default_account_environment, load_api_environment, prepare_project,
    probe_authorized_default_account, sanitize, sanitize_event, validate_paths,
    validate_auth_selection, verified_acceptance,
)


SUMMARY = "test result: ok. 1 passed; 0 failed; 0 ignored;"
NATIVE_ID = "00000000-0000-4000-8000-000000000001"


def complete_events():
    events = []
    for index, (phase, output, outcome) in enumerate([
        ("first_turn", "PARITY_ONE", "Completed"), ("second_turn", "PARITY_TWO", "Completed"),
        ("approval_allow", "APPROVED", "Completed"), ("approval_deny", "DENIED", "Completed"),
        ("queued_input", "QUEUE_PARENT_DONE", "Completed"), ("queued_input", "QUEUED_APPLIED_fixture", "Completed"),
        ("cancel", "", "Cancelled"), ("resume_result", "QUEUED_APPLIED_fixture", "Completed"),
    ]):
        turn_id = f"turn-{index}"
        events.extend([
            {"event": "message_accepted", "phase": phase, "turn_id": turn_id, "message_id": turn_id},
            {"event": "turn_started", "phase": phase, "turn_id": turn_id},
            {"event": "turn_finished", "phase": phase, "turn_id": turn_id,
             "outcome": outcome, "output": output, "native_session_id": NATIVE_ID},
        ])
    for phase, allowed, decision in (("approval_allow", True, "AllowOnce"), ("approval_deny", False, "DenyOnce")):
        events.extend([
            {"event": "approval_requested", "phase": phase, "exact_write_fixture": True, "decision": decision},
            {"event": "file_effect_verified", "phase": phase, "allowed": allowed},
        ])
    events.extend([
        {"event": "queued_input_submitted", "submitted_while_running": True, "same_turn_steering_verified": False},
        {"event": "queued_input_result_verified", "marker": "QUEUED_APPLIED_fixture",
         "native_acknowledgement_verified": True, "same_turn_steering_supported": False},
        {"event": "connection_shutdown", "native_session_id": NATIVE_ID},
        {"event": "connection_shutdown", "native_session_id": NATIVE_ID},
        {"event": "acceptance_passed", "scope": "rust_adapter_process_restart", "native_session_id": NATIVE_ID,
         "queued_input_verified": True, "same_turn_steering_supported": False,
         "app_restart_and_ui_verified": False, "parent_permission_ceiling_verified": False},
    ])
    return events


class AcceptanceTests(unittest.TestCase):
    def joined_events(self):
        events = complete_events()
        parent = next(event for event in events if event.get("event") == "turn_finished" and event.get("turn_id") == "turn-4")
        joined = next(event for event in events if event.get("event") == "turn_finished" and event.get("turn_id") == "turn-5")
        parent["output"] = joined["output"]
        events.remove(parent)
        events.insert(events.index(joined) + 1, parent)
        start = next(event for event in events if event.get("event") == "turn_started" and event.get("turn_id") == "turn-5")
        start.update({"event":"input_joined", "message_id":"turn-5", "turn_id":"turn-4", "native_session_id":NATIVE_ID})
        return events

    def test_joined_input_requires_the_real_execution_and_ordered_batch_result(self):
        events = self.joined_events()
        self.assertTrue(verified_acceptance(0, SUMMARY, events))
        for key, value in (("turn_id", "turn-3"), ("native_session_id", "other"), ("message_id", "unknown")):
            changed = copy.deepcopy(events)
            next(event for event in changed if event.get("event") == "input_joined")[key] = value
            with self.subTest(key=key):
                self.assertFalse(verified_acceptance(0, SUMMARY, changed))
        changed = self.joined_events()
        parent = next(event for event in changed if event.get("event") == "turn_finished" and event.get("turn_id") == "turn-4")
        parent["output"] = "different-result"
        self.assertFalse(verified_acceptance(0, SUMMARY, changed))
        changed = self.joined_events()
        joined = next(event for event in changed if event.get("event") == "input_joined")
        changed.remove(joined)
        changed.append(joined)
        self.assertFalse(verified_acceptance(0, SUMMARY, changed))

    def test_duplicate_or_unknown_join_cannot_replace_a_native_start(self):
        events = self.joined_events()
        joined = next(event for event in events if event.get("event") == "input_joined")
        self.assertFalse(verified_acceptance(0, SUMMARY, events + [joined.copy()]))
        self.assertFalse(verified_acceptance(0, SUMMARY, complete_events() + [joined.copy()]))

    def test_requires_one_matching_test_and_full_native_evidence(self):
        events = complete_events()
        self.assertTrue(verified_acceptance(0, SUMMARY, events))
        self.assertFalse(verified_acceptance(1, SUMMARY, events))
        self.assertFalse(verified_acceptance(0, "test result: ok. 0 passed; 0 failed; 0 ignored;", events))
        self.assertFalse(verified_acceptance(0, SUMMARY, []))
        self.assertFalse(verified_acceptance(0, SUMMARY, [events[-1]]))
        self.assertFalse(verified_acceptance(0, SUMMARY, events + [{"event": "acceptance_failed"}]))

    def test_file_effects_and_real_cancel_cannot_be_replaced_by_local_ack(self):
        events = complete_events()
        for name in ("file_effect_verified", "message_accepted", "turn_started", "queued_input_submitted", "connection_shutdown"):
            with self.subTest(name=name):
                self.assertFalse(verified_acceptance(0, SUMMARY, [event for event in events if event["event"] != name]))
        changed = copy.deepcopy(events)
        next(event for event in changed if event.get("phase") == "cancel" and event["event"] == "turn_finished")["outcome"] = "Completed"
        self.assertFalse(verified_acceptance(0, SUMMARY, changed))

    def test_queue_and_resume_cannot_claim_same_turn_steering_or_new_identity(self):
        for field in ("same_turn_steering_supported", "app_restart_and_ui_verified", "parent_permission_ceiling_verified"):
            events = complete_events()
            events[-1][field] = True
            with self.subTest(field=field):
                self.assertFalse(verified_acceptance(0, SUMMARY, events))
        events = complete_events()
        next(event for event in events if event.get("phase") == "resume_result" and event["event"] == "turn_finished")["native_session_id"] = "different-session"
        self.assertFalse(verified_acceptance(0, SUMMARY, events))
        events = complete_events()
        next(event for event in events if event.get("phase") == "resume_result" and event["event"] == "turn_finished")["output"] = "PARITY_ONE"
        self.assertFalse(verified_acceptance(0, SUMMARY, events))

    def test_duplicate_completion_and_nonfixture_approval_fail(self):
        events = complete_events()
        self.assertFalse(verified_acceptance(0, SUMMARY, events + [events[2]]))
        events = complete_events()
        next(event for event in events if event["event"] == "approval_requested")["exact_write_fixture"] = False
        self.assertFalse(verified_acceptance(0, SUMMARY, events))


class EnvironmentTests(unittest.TestCase):
    def test_reuses_auth_paths_without_touching_settings_or_reading_native_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            config = root / "existing-config"
            home = root / "existing-home"
            config.mkdir()
            home.mkdir()
            settings = config / "settings.json"
            settings.write_text('{"existing":"preserve"}\n', encoding="utf-8")
            inherited = {"PATH": "test-path", "SystemRoot": "test-system", "ANTHROPIC_API_KEY": "inherited-secret",
                         "ANTHROPIC_AUTH_TOKEN": "inherited-token", "ANTHROPIC_BASE_URL": "https://inherited.invalid",
                         "HTTPS_PROXY": "inherited-proxy", "DYLD_INSERT_LIBRARIES": "inherited-library"}
            with mock.patch.dict(os.environ, inherited, clear=True), \
                    mock.patch.object(Path, "read_bytes", side_effect=AssertionError("不能读取认证文件")), \
                    mock.patch.object(Path, "read_text", side_effect=AssertionError("不能读取认证文件")):
                environment = authenticated_environment(root / "probe", config, home)
            self.assertEqual(environment["CLAUDE_CONFIG_DIR"], str(config))
            self.assertEqual(environment["HOME"], str(home))
            self.assertEqual(environment["USERPROFILE"], str(home))
            self.assertEqual(environment["PATH"], "test-path")
            for key in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "HTTPS_PROXY", "DYLD_INSERT_LIBRARIES"):
                self.assertNotIn(key, environment)
            self.assertEqual(settings.read_text(encoding="utf-8"), '{"existing":"preserve"}\n')

    def test_default_account_environment_preserves_identity_without_private_auth_overrides(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            inherited = {
                "PATH": "test-path", "HOME": "/Users/unit", "USER": "unit", "LOGNAME": "unit",
                "SHELL": "/bin/zsh", "CLAUDE_CONFIG_DIR": "/private/claude",
                "ANTHROPIC_API_KEY": "inherited-secret", "ANTHROPIC_AUTH_TOKEN": "inherited-token",
                "ANTHROPIC_BASE_URL": "https://inherited.invalid", "HTTPS_PROXY": "inherited-proxy",
            }
            with mock.patch.dict(os.environ, inherited, clear=True):
                environment = authorized_default_account_environment(root)
            self.assertEqual(environment["HOME"], "/Users/unit")
            self.assertEqual(environment["USER"], "unit")
            self.assertEqual(environment["LOGNAME"], "unit")
            self.assertEqual(environment["SHELL"], "/bin/zsh")
            self.assertEqual(environment["PATH"], "test-path")
            for key in ("CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN",
                        "ANTHROPIC_BASE_URL", "HTTPS_PROXY"):
                self.assertNotIn(key, environment)

    def test_default_account_status_keeps_only_nonidentity_subscription_summary(self):
        raw = {
            "loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
            "subscriptionType": "pro", "email": "person@example.invalid",
            "organizationId": "00000000-0000-4000-8000-000000000001", "accessToken": "secret",
        }
        self.assertEqual(authorized_account_summary(raw), {
            "loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
            "subscriptionType": "pro",
        })
        for changed in (
            raw | {"loggedIn": False}, raw | {"authMethod": "apiKey"},
            raw | {"apiProvider": "bedrock"}, raw | {"subscriptionType": None},
            raw | {"subscriptionType": "free"}, [],
        ):
            with self.subTest(value=changed), self.assertRaises(ValueError):
                authorized_account_summary(changed)

    def test_default_account_probe_uses_fixed_readonly_command_and_discards_identity_fields(self):
        raw = {
            "loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
            "subscriptionType": "pro", "email": "person@example.invalid", "token": "secret",
        }
        completed = SimpleNamespace(returncode=0, stdout=json.dumps(raw), stderr="private stderr")
        with mock.patch.object(runner.subprocess, "run", return_value=completed) as run:
            result = probe_authorized_default_account(
                Path("/fixed/claude"), {"HOME": "/Users/unit"}, Path("/probe")
            )
        self.assertEqual(result, {key: raw[key] for key in
                                  ("loggedIn", "authMethod", "apiProvider", "subscriptionType")})
        run.assert_called_once_with(
            ["/fixed/claude", "auth", "status", "--json"], cwd=Path("/probe"),
            env={"HOME": "/Users/unit"}, capture_output=True, text=True, encoding="utf-8",
            errors="strict", timeout=30,
        )
        with mock.patch.object(runner.subprocess, "run", return_value=SimpleNamespace(
                returncode=1, stdout=json.dumps(raw), stderr="person@example.invalid org_secret_value")):
            with self.assertRaisesRegex(ValueError, "状态检查失败") as raised:
                probe_authorized_default_account(Path("/fixed/claude"), {}, Path("/probe"))
        self.assertNotIn("person@", str(raised.exception))
        self.assertNotIn("org_secret", str(raised.exception))

    def test_project_only_requests_write_approval_and_refuses_reinitialization(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            settings = prepare_project(root)
            self.assertEqual(json.loads(settings.read_text(encoding="utf-8")), PROJECT_SETTINGS)
            self.assertEqual(PROJECT_SETTINGS, {"permissions": {"defaultMode": "default", "ask": ["Write"]}})
            self.assertEqual((root / ".infinishell-claude-live-probe").read_text(encoding="utf-8"), MARKER)
            with self.assertRaises(FileExistsError):
                prepare_project(root)

    def test_api_file_only_loads_explicit_whitelist_and_one_authentication_method(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "api.json"
            safe = {"ANTHROPIC_API_KEY": "fake-unit-test-key", "ANTHROPIC_BASE_URL": "https://example.invalid",
                    "ANTHROPIC_MODEL": "fixture-model"}
            path.write_text(json.dumps(safe), encoding="utf-8")
            self.assertEqual(load_api_environment(path), safe)
            self.assertEqual(load_api_environment(None), {})
            for value in (safe | {"DYLD_INSERT_LIBRARIES": "extra"}, safe | {"ANTHROPIC_AUTH_TOKEN": "other"},
                          {"ANTHROPIC_MODEL": "only-model"}, safe | {"ANTHROPIC_API_KEY": "line\nbreak"},
                          safe | {"ANTHROPIC_API_KEY": False}):
                with self.subTest(value=list(value)):
                    path.write_text(json.dumps(value), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        load_api_environment(path)

    def test_api_and_path_redaction_preserves_json_and_never_hashes_secrets(self):
        secret = 'fake-secret-"quote"-\\slash'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            redact = lambda text: sanitize(text, root, root / "claude", root / "home",
                {"ANTHROPIC_API_KEY": secret, "ANTHROPIC_BASE_URL": "https://private.invalid"})
            event = {"message": f"credential={secret}", "items": ["https://private.invalid", str(root / "claude")], "passed": True}
            cleaned = sanitize_event(event, redact)
            encoded = json.dumps(cleaned)
            self.assertEqual(json.loads(encoded), cleaned)
            self.assertEqual(cleaned["message"], "credential=<redacted>")
            self.assertEqual(cleaned["items"], ["<redacted>", "<private-claude-config>"])
            self.assertTrue(cleaned["passed"])
            self.assertNotIn(secret, redact(json.dumps(event)))
            self.assertNotIn("private.invalid", encoded)
            identity = sanitize_event({"email":"person@example.invalid", "organizationId":"org_secret_value",
                                       "token":"secret-token", "message":"person@example.invalid"}, redact)
            self.assertEqual(identity["email"], "<redacted>")
            self.assertEqual(identity["organizationId"], "<redacted>")
            self.assertEqual(identity["token"], "<redacted>")
            self.assertEqual(identity["message"], "<redacted-email>")
            invalid = redact('email=person@example.invalid organization_id=org_secret_value token=secret-token')
            self.assertNotIn("person@example.invalid", invalid)
            self.assertNotIn("org_secret_value", invalid)
            self.assertNotIn("secret-token", invalid)

    def test_output_rejects_credentials_executables_and_existing_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for name in ("claude", "libtest", "supervisor", "api.json"):
                (root / name).write_text("fixture", encoding="utf-8")
            (root / "config").mkdir()
            (root / "home").mkdir()
            args = argparse.Namespace(test_binary=root / "libtest", claude=root / "claude", supervisor=root / "supervisor",
                config_dir=root / "config", auth_home=root / "home", api_environment_file=root / "api.json", output=root / "new.ndjson")
            validate_paths(args)
            for output in (root / "api.json", root / "libtest", root / "home/new.ndjson", root / "config/new.ndjson", root / "new.metadata.json"):
                with self.subTest(output=output.name), self.assertRaises(ValueError):
                    validate_paths(argparse.Namespace(**(vars(args) | {"output": output})))
            args.output.write_text("old evidence", encoding="utf-8")
            with self.assertRaises(ValueError):
                validate_paths(args)

    def test_default_account_requires_explicit_opt_in_and_excludes_private_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for name in ("claude", "libtest", "supervisor", "api.json"):
                (root / name).write_text("fixture", encoding="utf-8")
            (root / "config").mkdir()
            (root / "home").mkdir()
            common = dict(test_binary=root / "libtest", claude=root / "claude",
                          supervisor=root / "supervisor", output=root / "events.ndjson",
                          claude_version="2.1.278")
            online = argparse.Namespace(**common, config_dir=None, auth_home=None,
                                        api_environment_file=None, use_authorized_default_account=True)
            validate_paths(online)
            for changes in (
                {"config_dir": root / "config"}, {"auth_home": root / "home"},
                {"api_environment_file": root / "api.json"}, {"claude_version": "2.1.273"},
                {"use_authorized_default_account": False},
            ):
                args = argparse.Namespace(**(vars(online) | changes))
                with self.subTest(changes=changes), self.assertRaises(ValueError):
                    validate_paths(args)
            self.assertTrue(validate_auth_selection(online))


class FixedVersionRunnerTests(unittest.TestCase):
    def test_unverified_input_is_rejected_before_api_read_or_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "claude"
            executable.write_bytes(b"not an official binary")
            with mock.patch.object(runner, "load_api_environment") as load_api, \
                    mock.patch.object(runner, "verify_version") as probe, \
                    mock.patch.object(runner.subprocess, "Popen") as spawn:
                for version in ("2.1.273", "2.1.278", "latest", "2.1.279"):
                    with self.subTest(version=version), self.assertRaises(ValueError):
                        runner.run(argparse.Namespace(claude=executable, claude_version=version))
                load_api.assert_not_called()
                probe.assert_not_called()
                spawn.assert_not_called()

    def test_probe_failure_stops_before_api_read(self):
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}), \
                mock.patch.object(runner.tempfile, "mkdtemp", return_value=temporary), \
                mock.patch.object(runner, "verify_version", side_effect=ValueError("版本不匹配")), \
                mock.patch.object(runner, "load_api_environment") as load_api, \
                mock.patch.object(runner.subprocess, "Popen") as spawn:
            with self.assertRaises(ValueError):
                runner.run(argparse.Namespace(claude=Path(temporary) / "claude", claude_version="2.1.278"))
            load_api.assert_not_called()
            spawn.assert_not_called()

    def test_direct_default_account_run_rejects_private_inputs_before_reading_api(self):
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}), \
                mock.patch.object(runner.tempfile, "mkdtemp", return_value=temporary), \
                mock.patch.object(runner, "verify_version", return_value="2.1.278 (Claude Code)"), \
                mock.patch.object(runner, "load_api_environment") as load_api, \
                mock.patch.object(runner, "probe_authorized_default_account") as probe:
            root = Path(temporary)
            args = argparse.Namespace(
                claude=root / "claude", claude_version="2.1.278",
                use_authorized_default_account=True, config_dir=root / "config",
                auth_home=root / "home", api_environment_file=root / "api.json",
            )
            with self.assertRaisesRegex(ValueError, "不能同时提供"):
                runner.run(args)
            load_api.assert_not_called()
            probe.assert_not_called()

    def test_default_and_explicit_version_reach_runtime_and_metadata(self):
        for selected in (None, "2.1.278"):
            expected = selected or "2.1.273"
            with self.subTest(version=expected), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                for name in ("claude", "libtest", "supervisor"):
                    (root / name).write_bytes(b"synthetic executable")
                (root / "config").mkdir()
                (root / "home").mkdir()
                args = argparse.Namespace(claude=root / "claude", test_binary=root / "libtest",
                    supervisor=root / "supervisor", config_dir=root / "config", auth_home=root / "home",
                    output=root / "events.ndjson", api_environment_file=None, model=None)
                if selected is not None:
                    args.claude_version = selected

                def spawn(command, **kwargs):
                    self.assertEqual(kwargs["env"]["INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION"], expected)
                    self.assertEqual(command[0], str(args.test_binary))
                    args.output.write_text("".join(json.dumps(row) + "\n" for row in complete_events()), encoding="utf-8")
                    return SimpleNamespace(returncode=0, communicate=lambda timeout: (SUMMARY, None))

                with mock.patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}) as verify, \
                        mock.patch.object(runner, "verify_version", return_value=f"{expected} (Claude Code)") as probe, \
                        mock.patch.object(runner.tempfile, "mkdtemp", return_value=str(root)), \
                        mock.patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout="")), \
                        mock.patch.object(runner.subprocess, "Popen", side_effect=spawn), \
                        mock.patch("builtins.print"):
                    self.assertEqual(runner.run(args), 0)
                probe.assert_called_once_with(args.claude, root, expected)
                self.assertEqual(verify.call_count, 3)
                self.assertTrue(all(call.args[-1] == expected for call in verify.call_args_list))
                metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
                self.assertEqual(metadata["requested_cli_version"], expected)
                self.assertEqual(metadata["cli_version"], f"{expected} (Claude Code)")
                self.assertTrue(metadata["acceptance_passed"])
                self.assertTrue(metadata["cli_binary_unchanged"])

    def test_authorized_default_account_is_probed_before_lifecycle_without_config_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name in ("claude", "libtest", "supervisor"):
                (root / name).write_bytes(b"synthetic executable")
            args = argparse.Namespace(
                claude=root / "claude", claude_version="2.1.278", test_binary=root / "libtest",
                supervisor=root / "supervisor", config_dir=None, auth_home=None,
                api_environment_file=None, use_authorized_default_account=True,
                output=root / "events.ndjson", model=None,
            )
            status = {"loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
                      "subscriptionType": "pro"}

            def spawn(command, **kwargs):
                self.assertNotIn("CLAUDE_CONFIG_DIR", kwargs["env"])
                self.assertNotIn("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR", kwargs["env"])
                self.assertEqual(kwargs["env"]["INFINISHELL_CLAUDE_LIVE_AUTH_MODE"],
                                 "authorized_default_account")
                args.output.write_text("".join(json.dumps(row) + "\n" for row in complete_events()),
                                       encoding="utf-8")
                return SimpleNamespace(returncode=0, communicate=lambda timeout: (SUMMARY, None))

            environment = {"HOME": str(Path.home()), "PATH": "test-path"}
            with mock.patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}), \
                    mock.patch.object(runner, "verify_version", return_value="2.1.278 (Claude Code)"), \
                    mock.patch.object(runner.tempfile, "mkdtemp", return_value=str(root)), \
                    mock.patch.object(runner, "authorized_default_account_environment",
                                      return_value=environment.copy()), \
                    mock.patch.object(runner, "probe_authorized_default_account", return_value=status) as probe, \
                    mock.patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout="")), \
                    mock.patch.object(runner.subprocess, "Popen", side_effect=spawn), \
                    mock.patch("builtins.print"):
                self.assertEqual(runner.run(args), 0)
            probe.assert_called_once_with(args.claude, environment, root / "project")
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertEqual(metadata["authentication_source"], "authorized_default_account")
            self.assertEqual(metadata["authorized_default_account"], status)
            self.assertFalse(metadata["private_config_supplied"])
            self.assertTrue(metadata["default_user_home_preserved"])


if __name__ == "__main__":
    unittest.main()
