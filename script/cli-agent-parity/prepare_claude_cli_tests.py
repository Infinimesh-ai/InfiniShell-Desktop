#!/usr/bin/env python3
"""固定 Claude 文件准备器的离线完整性与隔离回归；不运行或安装 CLI。"""

import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import prepare_claude_cli as prepare
from prepare_claude_cli import (BASE_URL, MANIFEST_SHA256, MANIFEST_SIZE, MARKER, RELEASES,
                               RELEASE_COMMIT, VERSION, fetch, isolated_environment, owned_directory)


class FixedClaudeInputsTests(unittest.TestCase):
    def test_pins_match_archived_official_manifest(self):
        fixture = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/claude-2.1.273-release-manifest.json"
        data = fixture.read_bytes()
        manifest = json.loads(data)
        self.assertEqual(len(data), MANIFEST_SIZE)
        self.assertEqual(hashlib.sha256(data).hexdigest(), MANIFEST_SHA256)
        self.assertEqual((manifest["version"], manifest["commit"]), (VERSION, RELEASE_COMMIT))
        for platform, (name, size, checksum) in RELEASES.items():
            self.assertEqual(manifest["platforms"][platform], {"binary": name, "size": size, "checksum": checksum})

    def test_explicit_versions_match_their_own_archived_official_manifest(self):
        self.assertEqual(tuple(prepare.RELEASE_CATALOG), ("2.1.273", "2.1.278"))
        self.assertEqual(VERSION, "2.1.273")
        for version, contract in prepare.RELEASE_CATALOG.items():
            with self.subTest(version=version):
                fixture = Path(__file__).resolve().parents[2] / f"specs/cli-agent-parity/fixtures/claude-{version}-release-manifest.json"
                data = fixture.read_bytes()
                manifest = json.loads(data)
                self.assertEqual(len(data), contract["manifest_size"])
                self.assertEqual(hashlib.sha256(data).hexdigest(), contract["manifest_sha256"])
                self.assertEqual((manifest["version"], manifest["commit"]), (version, contract["commit"]))
                for target, (name, size, checksum) in contract["platforms"].items():
                    self.assertEqual(manifest["platforms"][target], {"binary": name, "size": size, "checksum": checksum})

    def test_main_defaults_to_the_latest_verified_release(self):
        argv = ["prepare_claude_cli.py", "--private-directory", "/not-used"]
        with mock.patch.object(prepare.sys, "argv", argv), \
                mock.patch.object(prepare, "release_contract", side_effect=RuntimeError("stop")) as contract, \
                self.assertRaisesRegex(RuntimeError, "stop"):
            prepare.main()
        contract.assert_called_once_with("2.1.278")

    def test_binary_bytes_and_digest_must_belong_to_selected_version(self):
        # 缩小的合成文件只验证绑定关系；真实发行摘要由上面的原始 manifest 回归核对。
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "claude"
            contracts = {version: {"platforms": {"darwin-arm64": (
                "claude", len(version), hashlib.sha256(version.encode()).hexdigest())}}
                for version in ("2.1.273", "2.1.278")}
            with mock.patch.dict(prepare.RELEASE_CATALOG, contracts):
                for version in contracts:
                    path.write_bytes(version.encode())
                    self.assertIn(f"/{version}/", prepare.verify_binary(path, "darwin-arm64", version)["url"])
                    other = "2.1.278" if version == "2.1.273" else "2.1.273"
                    with self.assertRaises(ValueError):
                        prepare.verify_binary(path, "darwin-arm64", other)
                    path.write_bytes(version.encode() + b"extra")
                    with self.assertRaises(ValueError):
                        prepare.verify_binary(path, "darwin-arm64", version)
            for version in ("latest", "2.1.279", "2.1.278-beta", None, []):
                with self.subTest(version=version), self.assertRaises(ValueError):
                    prepare.verify_binary(path, "darwin-arm64", version)

    def test_version_probe_is_isolated_and_rejects_known_version_mismatch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY": "synthetic-secret"}), \
                    mock.patch.object(prepare.subprocess, "run") as run:
                for expected in ("2.1.273", "2.1.278"):
                    run.return_value = SimpleNamespace(stdout=f"{expected} (Claude Code)\n")
                    self.assertEqual(prepare.verify_version(root / "claude", root, expected),
                                     f"{expected} (Claude Code)")
                    self.assertNotIn("ANTHROPIC_API_KEY", run.call_args.kwargs["env"])
                    other = "2.1.278" if expected == "2.1.273" else "2.1.273"
                    with self.assertRaises(ValueError):
                        prepare.verify_version(root / "claude", root, other)
                run.reset_mock()
                with self.assertRaises(ValueError):
                    prepare.verify_version(root / "claude", root, "latest")
                run.assert_not_called()

    def test_version_specific_directory_and_download_reject_cross_version_reuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            directory = owned_directory(root / "old", explicit_private=True)
            original = (directory / MARKER).read_bytes()
            with self.assertRaises(ValueError):
                owned_directory(directory, explicit_private=True, version="2.1.278")
            self.assertEqual((directory / MARKER).read_bytes(), original)
            fresh = owned_directory(root / "new", explicit_private=True, version="2.1.278")
            self.assertIn(b"2.1.278", (fresh / MARKER).read_bytes())
            payload = b"isolated test payload"
            url = "https://downloads.claude.ai/claude-code-releases/2.1.278/fixture"
            with mock.patch.object(prepare.urllib.request, "urlopen", return_value=io.BytesIO(payload)) as opened:
                with self.assertRaises(ValueError):
                    fetch(url, fresh / "claude", len(payload), hashlib.sha256(payload).hexdigest())
                opened.assert_not_called()
                fetch(url, fresh / "claude", len(payload), hashlib.sha256(payload).hexdigest(), version="2.1.278")
                self.assertEqual((fresh / "claude").read_bytes(), payload)

    def test_environment_is_private_on_all_platforms_and_has_no_credentials(self):
        inherited = {"PATH": "fixture-path", "SystemRoot": "fixture-system", "ANTHROPIC_API_KEY": "test-only",
                     "CLAUDE_CODE_OAUTH_TOKEN": "test-only", "HOME": "user-home", "USERPROFILE": "user-profile",
                     "APPDATA": "user-appdata", "CLAUDE_CONFIG_DIR": "user-config", "HTTPS_PROXY": "user-proxy"}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, inherited, clear=True):
            root = Path(temporary).resolve()
            env = isolated_environment(root)
            self.assertEqual(env["PATH"], "fixture-path")
            # Windows 的真实 os.environ 会把写入键正规化为大写；隔离环境保留该平台枚举出的键。
            system_root_key = "SYSTEMROOT" if os.name == "nt" else "SystemRoot"
            self.assertEqual({key: value for key, value in env.items() if key.upper() == "SYSTEMROOT"},
                             {system_root_key: "fixture-system"})
            for name in ("ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "HTTPS_PROXY"):
                self.assertNotIn(name, env)
            for name in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME",
                         "XDG_DATA_HOME", "XDG_CACHE_HOME", "CLAUDE_CONFIG_DIR", "TMP", "TEMP", "TMPDIR"):
                self.assertTrue(Path(env[name]).resolve().is_relative_to(root))
                self.assertTrue(Path(env[name]).is_dir())
            self.assertEqual(env["DISABLE_AUTOUPDATER"], "1")
            self.assertEqual(env["DISABLE_UPDATES"], "1")
            self.assertEqual((root / "claude/settings.json").read_bytes(), b"{}\n")
            self.assertFalse((root / "claude/.credentials.json").exists())

    def test_runner_directory_is_scoped_and_existing_content_is_not_adopted(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, {"RUNNER_TEMP": temporary}):
            root = Path(temporary).resolve()
            directory = owned_directory(root / "download")
            self.assertTrue((directory / MARKER).is_file())
            self.assertEqual(owned_directory(directory), directory)
            with self.assertRaises(ValueError):
                owned_directory(root)
            foreign = root / "foreign"
            foreign.mkdir(mode=0o700)
            (foreign / "user-file").write_bytes(b"preserve")
            with self.assertRaises(ValueError):
                owned_directory(foreign)
            self.assertEqual((foreign / "user-file").read_bytes(), b"preserve")

    def test_explicit_private_directory_does_not_need_runner_environment(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, {}, clear=True):
            directory = Path(temporary).resolve() / "private"
            with self.assertRaises(ValueError):
                owned_directory(directory)
            self.assertEqual(owned_directory(directory, explicit_private=True), directory)

    def test_verified_download_is_atomic_and_reusable(self):
        payload = b"fixed test bytes"
        checksum = hashlib.sha256(payload).hexdigest()
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "claude"
            with mock.patch("prepare_claude_cli.urllib.request.urlopen", return_value=io.BytesIO(payload)) as opened:
                fetch(BASE_URL + "/fixture", target, len(payload), checksum)
                self.assertEqual(target.read_bytes(), payload)
                self.assertEqual(target.stat().st_nlink, 1)
                fetch(BASE_URL + "/fixture", target, len(payload), checksum)
                opened.assert_called_once()
            self.assertEqual(sorted(path.name for path in target.parent.iterdir()), ["claude"])

    def test_failed_download_and_custom_cache_never_replace_a_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "claude"
            for payload in (b"too long", b"bad"):
                with self.subTest(payload=payload), mock.patch("prepare_claude_cli.urllib.request.urlopen", return_value=io.BytesIO(payload)):
                    with self.assertRaises(ValueError):
                        fetch(BASE_URL + "/fixture", target, 3, hashlib.sha256(b"yes").hexdigest())
                self.assertFalse(target.exists())
                self.assertFalse(list(target.parent.iterdir()))
            target.write_bytes(b"user modification")
            with self.assertRaises(ValueError):
                fetch(BASE_URL + "/fixture", target, 3, hashlib.sha256(b"yes").hexdigest())
            self.assertEqual(target.read_bytes(), b"user modification")


if __name__ == "__main__":
    unittest.main()
