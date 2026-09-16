#!/usr/bin/env python3
"""固定 Claude 文件准备器的离线完整性与隔离回归；不运行或安装 CLI。"""

import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

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
