#!/usr/bin/env python3
"""验证普通 Grok PTY 准备与拒绝路径；不运行原生 CLI、模型、网络或授权。"""

import contextlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_grok_official_pty_live as runner

SESSION = "12345678-1234-4567-89ab-123456789abc"


class Channel:
    def __init__(self, reads, short_write=False):
        self.reads = list(reads)
        self.writes = []
        self.short_write = short_write

    def write(self, data):
        self.writes.append(data)
        return len(data) - 1 if self.short_write else len(data)

    def read(self, timeout):
        return self.reads.pop(0) if self.reads else None


class Clock:
    def __init__(self):
        self.value = 0

    def __call__(self):
        self.value += 0.05
        return self.value


class GrokOfficialPtyRunnerTests(unittest.TestCase):
    def setUp(self):
        self.plan = runner.build_plan(SESSION)

    def observe(self, channel, stage=None, screen=None):
        with tempfile.TemporaryDirectory() as directory:
            frames = runner.PrivateFrames(Path(directory) / "private.ndjson")
            try:
                return runner.observe_stage(channel, self.plan[0] if stage is None else stage,
                    frames, runner.Screen() if screen is None else screen, timeout=1, now=Clock())
            finally:
                frames.close()

    def test_plan_has_two_multilingual_response_inputs_and_one_cancel_probe(self):
        self.assertEqual(len(self.plan), runner.MAX_INPUTS)
        self.assertEqual([stage["phase"] for stage in self.plan],
            ["english_multiline", "chinese_multiline", "cancel_signal_probe"])
        self.assertEqual(len(self.plan[0]["payload"].splitlines()), 2)
        self.assertEqual(len(self.plan[1]["payload"].splitlines()), 3)
        self.assertIn("中文", self.plan[1]["payload"])
        for stage in self.plan:
            payload = stage["payload"].encode()
            self.assertEqual(stage["payload_bytes"], len(payload))
            self.assertEqual(stage["payload_sha256"], runner.digest(payload))
            if stage["expected"] is not None:
                self.assertNotIn(stage["expected"].encode(), payload)

    def test_invalid_identity_cannot_enter_arguments_or_prompt(self):
        for identity in (SESSION.upper(), SESSION + "\nPRIVATE_CANARY", "PRIVATE_ID", "a" * 64):
            with self.assertRaises(ValueError):
                runner.build_plan(identity)
            with self.assertRaises(ValueError):
                runner.tui_arguments(PurePosixPath("/private/tmp/isolated"), identity)

    def test_tui_argv_is_fixed_documented_set_and_uses_only_private_socket(self):
        root = PurePosixPath("/private/tmp/isolated")
        self.assertEqual(runner.tui_arguments(root, SESSION), ["--minimal", "--no-alt-screen",
            "--cwd", str(root / "project"), "--leader-socket", str(root / "tmp/pty-leader/leader.sock"), "--session-id", SESSION,
            "--no-subagents", "--disable-web-search"])
        for value in ("--always-approve", "--oauth", "agent", "stdio", "--single", "--continue"):
            self.assertNotIn(value, runner.tui_arguments(root, SESSION))

    def test_sandbox_keeps_exact_network_socket_and_blocks_host_ssh_and_compatibility(self):
        profile = runner.pty_sandbox_profile(PurePosixPath("/private/tmp/isolated"), PurePosixPath("/private/tmp/auth-cache"),
            12345, PurePosixPath("/Users/offline-fixture"))
        self.assertIn('(deny network*)', profile)
        self.assertIn('localhost:12345', profile)
        self.assertIn('/private/tmp/isolated/tmp/pty-leader/leader.sock', profile)
        for value in ("/private/tmp/auth-cache", "/Users/offline-fixture/.ssh", "/Users/offline-fixture/.claude",
                "/Users/offline-fixture/.codex", "/Users/offline-fixture/.grok/auth.json"):
            self.assertIn(value, profile)
        self.assertNotIn("localhost:*", profile)
        for port in (True, 0, 65536, "12345"):
            with self.assertRaises(ValueError):
                runner.pty_sandbox_profile(PurePosixPath("/private/tmp/isolated"), PurePosixPath("/private/tmp/auth-cache"), port, PurePosixPath("/Users/fixture"))

    def test_english_paste_render_enter_and_text_response_are_distinct(self):
        stage = self.plan[0]
        channel = Channel([stage["payload"].encode().replace(b"\n", b"\r\n"), b"\r\n" + stage["expected"].encode() + b"\r\n"])
        events = self.observe(channel)
        self.assertEqual(channel.writes, [runner.PASTE_BEGIN + stage["payload"].encode() + runner.PASTE_END, b"\r"])
        self.assertEqual([event["event"] for event in events],
            ["bracketed_paste_written", "paste_render_observed", "enter_written", "response_text_observed"])
        self.assertFalse(events[-1]["task_completed_verified"])
        self.assertFalse(events[-1]["native_ack_verified"])
        self.assertNotIn(stage["expected"], json.dumps(events))

    def test_isolated_environment_drops_api_tokens_and_uses_private_ssh_config(self):
        canary = "PRIVATE_ENV_CANARY"
        with patch.dict(os.environ, {"ANTHROPIC_API_KEY": canary, "GROK_API_KEY": canary,
                "OPENAI_API_KEY": canary, "SSH_AUTH_SOCK": canary, "LANG": canary}, clear=True):
            environment = runner.isolated_environment(PurePosixPath("/private/tmp/offline fixture"), 12345)
        self.assertNotIn(canary, json.dumps(environment))
        self.assertNotIn("INFINISHELL_GROK_BYOK_KEY", environment)
        self.assertEqual(environment["GROK_CLAUDE_HOOKS_ENABLED"], "0")
        self.assertEqual(environment["GROK_CODEX_MCPS_ENABLED"], "0")
        self.assertEqual(environment["HTTPS_PROXY"], "http://127.0.0.1:12345")
        self.assertEqual(environment["GIT_SSH_COMMAND"], "ssh -F '/private/tmp/offline fixture/home/.ssh/config'")

    def test_chinese_split_utf8_multiline_paste_remains_correct(self):
        stage = self.plan[1]
        payload = stage["payload"].encode().replace(b"\n", b"\r\n")
        channel = Channel([payload[:1], payload[1:5], payload[5:], b"\r\n" + stage["expected"].encode()])
        events = self.observe(channel, stage)
        self.assertEqual(events[-1]["response_marker_sha256"], stage["expected_response_sha256"])

    def test_prompt_echo_without_response_is_rejected(self):
        channel = Channel([self.plan[0]["payload"].encode().replace(b"\n", b"\r\n"), b"\r\n"])
        with self.assertRaises(ValueError):
            self.observe(channel)

    def test_complete_marker_in_prompt_is_rejected_before_any_write(self):
        stage = dict(self.plan[0], payload=self.plan[0]["expected"])
        channel = Channel([])
        with self.assertRaises(ValueError):
            self.observe(channel, stage)
        self.assertEqual(channel.writes, [])

    def test_marker_already_on_screen_cannot_be_counted_as_post_enter_response(self):
        screen = runner.Screen()
        screen.feed((self.plan[0]["expected"] + "\r\n").encode())
        channel = Channel([self.plan[0]["payload"].encode().replace(b"\n", b"\r\n")])
        with self.assertRaises(ValueError):
            self.observe(channel, screen=screen)
        self.assertNotIn(b"\r", channel.writes)

    def test_hidden_osc_marker_does_not_count_as_rendered_response(self):
        marker = self.plan[0]["expected"].encode()
        channel = Channel([self.plan[0]["payload"].encode().replace(b"\n", b"\r\n"), b"\x1b]0;" + marker + b"\x07"])
        with self.assertRaises(ValueError):
            self.observe(channel)

    def test_screen_supports_cursor_erase_color_and_chinese_width(self):
        screen = runner.Screen()
        screen.feed("旧正文\x1b[2K\r\x1b[31m中文\x1b[0m".encode())
        self.assertEqual(screen.lines()[0], "中文")
        self.assertEqual(screen.x, 4)
        self.assertTrue(screen.supported)
        screen.feed(b"\x1b[2J\x1b[HABC\r\x1b[1C\x1b[K")
        self.assertEqual(screen.lines()[0], "A")

    def test_unknown_control_sequence_invalidates_response_even_when_marker_exists(self):
        channel = Channel([self.plan[0]["payload"].encode().replace(b"\n", b"\r\n"),
            b"\x1b[7Q\r\n" + self.plan[0]["expected"].encode()])
        with self.assertRaises(ValueError):
            self.observe(channel)

    def test_invalid_utf8_error_never_contains_raw_terminal_body(self):
        with self.assertRaises(ValueError) as result:
            runner.Screen().feed(b"\xffPRIVATE_TERMINAL_CANARY")
        self.assertNotIn("PRIVATE_TERMINAL_CANARY", str(result.exception))

    def test_disconnect_and_short_write_are_fixed_failures(self):
        for channel in (Channel([]), Channel([], short_write=True)):
            with self.assertRaises(ValueError):
                self.observe(channel)

    def test_timeout_is_bounded_when_empty_reads_never_finish(self):
        class Empty(Channel):
            def read(self, timeout):
                self.assert_timeout = timeout
                return b""
        channel = Empty([])
        with self.assertRaises(ValueError):
            self.observe(channel)
        self.assertLessEqual(channel.assert_timeout, 0.2)

    def test_cancel_byte_requires_real_visible_partial_text_and_does_not_claim_cancelled(self):
        stage = self.plan[2]
        channel = Channel([stage["payload"].encode().replace(b"\n", b"\r\n"), b"\r\n1\r\n2\r\n3\r\n"])
        events = self.observe(channel, stage)
        self.assertEqual(channel.writes[-1], b"\x03")
        self.assertFalse(events[-1]["task_cancelled_verified"])
        self.assertFalse(events[-1]["all_processes_exited_verified"])
        channel = Channel([stage["payload"].encode().replace(b"\n", b"\r\n"), b"1 2 3"])
        with self.assertRaises(ValueError):
            self.observe(channel, stage)
        self.assertNotIn(b"\x03", channel.writes)

    def test_private_raw_frames_are_mode600_bounded_and_public_contains_no_body(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "private.ndjson"
            frames = runner.PrivateFrames(path)
            frames.record("stdout", b"PRIVATE_TUI_CANARY")
            result = frames.close()
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual(result["private_frame_file_sha256"], runner.digest(path.read_bytes()))
            self.assertNotIn("PRIVATE_TUI_CANARY", json.dumps(result))
            with self.assertRaises(FileExistsError):
                runner.PrivateFrames(path)
            with patch.object(runner, "MAX_RAW_BYTES", 1):
                frames = runner.PrivateFrames(Path(directory) / "bounded.ndjson")
                try:
                    with self.assertRaises(ValueError):
                        frames.record("stdout", b"too large")
                finally:
                    frames.close()
            with patch.object(runner, "MAX_FRAME_FILE_BYTES", 1):
                frames = runner.PrivateFrames(Path(directory) / "bounded-frames.ndjson")
                try:
                    with self.assertRaises(ValueError):
                        frames.record("stdout", b"x")
                finally:
                    frames.close()

    def test_live_refuses_before_auth_network_config_or_cli_on_every_platform(self):
        for platform in ("darwin", "linux", "win32"):
            with self.subTest(platform=platform), patch.object(runner.sys, "platform", platform), \
                    patch.object(runner.official, "copy_private_auth") as auth, \
                    patch.object(runner.official, "OfficialTunnel") as tunnel, \
                    patch.object(runner.official, "prepare_native") as native, \
                    patch.object(runner.official.shared, "network_canary") as canary, \
                    contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(runner.main(["--live", "--grok", "/PRIVATE_ARGUMENT_CANARY"]), 2)
                for mock in (auth, tunnel, native, canary):
                    mock.assert_not_called()
                refusal = json.loads(output.getvalue())
                self.assertFalse(refusal["auth_copied"])
                self.assertFalse(refusal["model_request_sent"])
                self.assertFalse(refusal["normal_cleanup_verified"])
                self.assertNotIn("PRIVATE_ARGUMENT_CANARY", output.getvalue())

    def test_prepare_only_copies_no_auth_opens_no_tunnel_and_exports_no_prompt(self):
        # 只在测试新建的临时文件域运行配置生成；二进制摘要和宿主平台均为模拟。
        with tempfile.TemporaryDirectory() as directory:
            test_root = Path(directory)
            native = test_root / "fake-native"
            native.write_bytes(b"offline binary fixture")
            native.chmod(0o700)
            args = SimpleNamespace(grok=native, timeout=120, output=test_root / "public.ndjson")
            prepared = test_root / "private-workspace"
            prepared.mkdir(mode=0o700)
            with patch.object(runner.sys, "platform", "darwin"), \
                    patch.object(runner.tempfile, "mkdtemp", return_value=str(prepared)), \
                    patch.object(runner.official.shared, "digest", return_value=runner.official.shared.BINARY_SHA256), \
                    patch.object(runner.official, "copy_private_auth") as auth, \
                    patch.object(runner.official, "OfficialTunnel") as tunnel:
                self.assertEqual(runner.prepare_only(args), 0)
                auth.assert_not_called()
                tunnel.assert_not_called()
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            workspace = Path(metadata["private_workspace"])
            try:
                public = args.output.read_text()
                self.assertNotIn("Do not call tools", public)
                self.assertNotIn("第一行", public)
                self.assertNotIn("expected\"", public)
                self.assertFalse((workspace / "home/.grok/auth.json").exists())
                self.assertFalse((workspace / "grok-sandbox").exists())
                self.assertFalse(metadata["pty_containment_verified"])
                self.assertFalse(metadata["normal_cleanup_verified"])
                self.assertFalse(metadata["real_input_verified"])
                self.assertFalse(metadata["version_actually_detected"])
                network = json.loads(args.output.with_suffix(".network.json").read_text())
                self.assertEqual(network["tls_connections_attempted"], 0)
                self.assertEqual(network["max_tunnels"], 32)
                self.assertFalse(network["network_canary_verified"])
                for relative in ("home/.ssh/config", "home/.ssh/known_hosts", "private-input-plan.json"):
                    if os.name == "posix":
                        self.assertEqual(stat.S_IMODE((workspace / relative).stat().st_mode), 0o600)
            finally:
                shutil.rmtree(workspace)


if __name__ == "__main__":
    unittest.main()
