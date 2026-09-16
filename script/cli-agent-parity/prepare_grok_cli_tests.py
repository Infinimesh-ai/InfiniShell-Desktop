#!/usr/bin/env python3
"""固定 Grok 文件准备器的离线文件与隔离回归；不执行 CLI 或模型请求。"""

import hashlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest import mock

import prepare_grok_cli as grok


def response(payload, url):
    stream = io.BytesIO(payload)
    stream.geturl = lambda: url
    return stream


def elf_header(machine=62):
    data = bytearray(64)
    data[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<HH", data, 16, 3, machine)
    return bytes(data)


def pe_header(machine=0x8664):
    data = bytearray(154)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\x00\x00"
    struct.pack_into("<H", data, 132, machine)
    struct.pack_into("<H", data, 152, 0x20B)
    return bytes(data)


class FixedGrokInputsTests(unittest.TestCase):
    def test_pins_match_full_official_download_evidence(self):
        fixture = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/grok-1.0.30-fixed-platform-inputs.json"
        evidence = json.loads(fixture.read_text(encoding="utf-8"))
        self.assertEqual(evidence["source_commit"], grok.SOURCE_COMMIT)
        self.assertEqual(evidence["version"], grok.VERSION)
        self.assertFalse(evidence["official_checksum_manifest_verified"])
        self.assertEqual(set(evidence["platforms"]), set(grok.RELEASES))
        for target, (artifact, name, size, checksum) in grok.RELEASES.items():
            actual = evidence["platforms"][target]
            self.assertEqual((actual["artifact"], actual["binary"], actual["bytes"], actual["sha256"]),
                             (artifact, name, size, checksum))
            self.assertEqual(actual["url"], f"{grok.BASE_URL}/{artifact}")
            self.assertFalse(actual["native_version_verified"])

    def test_binary_architecture_uses_actual_headers(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "binary"
            for target, data, expected in [("linux-x64", elf_header(), "elf64-x86-64"),
                                           ("win32-x64", pe_header(), "pe32+-x86-64")]:
                path.write_bytes(data)
                self.assertEqual(grok.binary_format(path, target), expected)
            for target, data in [("linux-x64", elf_header(183)), ("win32-x64", pe_header(0xAA64)),
                                 ("linux-x64", pe_header()), ("win32-x64", elf_header()),
                                 ("win32-x64", b"MZ"), ("linux-x64", b"\x7fELF")]:
                with self.subTest(target=target, bytes=len(data)):
                    path.write_bytes(data)
                    with self.assertRaises(ValueError):
                        grok.binary_format(path, target)

    def test_environment_keeps_only_system_tools_and_private_roots(self):
        inherited = {"PATH": "fixture-path", "SystemRoot": "fixture-system", "GROK_HOME": "user-grok",
                     "XAI_API_KEY": "test-only", "GROK_AUTH_TOKEN": "test-only", "GROK_CONFIG": "user-config",
                     "GROK_LEADER_SOCKET": "user-socket", "HOME": "user-home", "USERPROFILE": "user-profile",
                     "HTTPS_PROXY": "user-proxy", "LD_LIBRARY_PATH": "user-loader", "LD_PRELOAD": "user-injection"}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, inherited, clear=True):
            root = Path(temporary).resolve()
            env = grok.isolated_environment(root)
            self.assertEqual(env["PATH"], "fixture-path")
            self.assertEqual({key.upper(): value for key, value in env.items() if key.upper() == "SYSTEMROOT"},
                             {"SYSTEMROOT": "fixture-system"})
            for key in ("XAI_API_KEY", "GROK_AUTH_TOKEN", "GROK_CONFIG", "GROK_LEADER_SOCKET",
                        "HTTPS_PROXY", "LD_LIBRARY_PATH", "LD_PRELOAD"):
                self.assertNotIn(key, env)
            for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "GROK_HOME", "CODEX_HOME",
                        "CLAUDE_CONFIG_DIR", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "TMP", "TEMP", "TMPDIR"):
                self.assertTrue(Path(env[key]).is_relative_to(root))
                self.assertTrue(Path(env[key]).is_dir())
            self.assertFalse(list((root / "grok").iterdir()))
            self.assertEqual(env["GROK_AUTO_UPDATE"], "0")

    def test_scoped_directory_cannot_adopt_user_content(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, {"RUNNER_TEMP": temporary}):
            root = Path(temporary).resolve()
            directory = grok.owned_directory(root / "download")
            self.assertEqual(grok.owned_directory(directory), directory)
            with self.assertRaises(ValueError):
                grok.owned_directory(root)
            foreign = root / "foreign"
            foreign.mkdir(mode=0o700)
            (foreign / "keep").write_bytes(b"user")
            with self.assertRaises(ValueError):
                grok.owned_directory(foreign)
            self.assertEqual((foreign / "keep").read_bytes(), b"user")
            (directory / grok.MARKER).write_bytes(b"foreign")
            with self.assertRaises(ValueError):
                grok.owned_directory(directory)

    def test_explicit_private_directory_and_source_tree_guard(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, {}, clear=True):
            directory = Path(temporary).resolve() / "private"
            with self.assertRaises(ValueError):
                grok.owned_directory(directory)
            self.assertEqual(grok.owned_directory(directory, explicit_private=True), directory)
            with self.assertRaises(ValueError):
                grok.owned_directory(Path(grok.__file__).parent / "must-not-create", explicit_private=True)

    def test_complete_download_is_atomic_and_duplicate_does_not_modify(self):
        payload = elf_header()
        checksum = hashlib.sha256(payload).hexdigest()
        url = f"{grok.BASE_URL}/{grok.RELEASES['linux-x64'][0]}"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "grok"
            with mock.patch.object(grok.urllib.request, "urlopen", return_value=response(payload, url)) as opened:
                grok.fetch(url, path, len(payload), checksum)
                before = path.stat()
                grok.fetch(url, path, len(payload), checksum)
                self.assertEqual(path.read_bytes(), payload)
                self.assertEqual(path.stat().st_mtime_ns, before.st_mtime_ns)
                self.assertEqual(path.stat().st_nlink, 1)
                opened.assert_called_once()
            self.assertEqual([item.name for item in path.parent.iterdir()], ["grok"])

    def test_bad_download_redirect_and_modified_cache_fail_without_replacement(self):
        url = f"{grok.BASE_URL}/{grok.RELEASES['linux-x64'][0]}"
        checksum = hashlib.sha256(b"yes").hexdigest()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "grok"
            for payload, actual_url in [(b"too long", url), (b"bad", url), (b"y", url),
                                         (b"yes", "https://example.invalid/unreviewed")]:
                with self.subTest(payload=payload, actual_url=actual_url), mock.patch.object(
                        grok.urllib.request, "urlopen", return_value=response(payload, actual_url)):
                    with self.assertRaises(ValueError):
                        grok.fetch(url, path, 3, checksum)
                self.assertFalse(list(path.parent.iterdir()))
            path.write_bytes(b"user changes")
            with self.assertRaises(ValueError):
                grok.fetch(url, path, 3, checksum)
            self.assertEqual(path.read_bytes(), b"user changes")

    def test_hardlinked_cache_is_rejected_without_modifying_other_name(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / "original"
            original.write_bytes(b"preserve")
            os.link(original, root / "grok")
            with self.assertRaises(ValueError):
                grok.regular_file(root / "grok")
            self.assertEqual(original.read_bytes(), b"preserve")

    def test_version_verification_only_uses_fixed_argv_and_private_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            executable = root / "grok"

            def run(argv, **kwargs):
                self.assertEqual(argv, [str(executable), "--version"])
                self.assertTrue(kwargs["cwd"].is_relative_to(root))
                self.assertEqual(kwargs["timeout"], 10)
                self.assertNotIn("XAI_API_KEY", kwargs["env"])
                return subprocess.CompletedProcess(argv, 0, grok.VERSION_OUTPUT + "\n", "")

            with mock.patch.object(grok.subprocess, "run", side_effect=run):
                self.assertEqual(grok.verify_version(executable, root), grok.VERSION_OUTPUT)

    def test_download_only_never_runs_foreign_binary_and_records_unverified_native(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "download"
            argv = ["prepare_grok_cli.py", "--private-directory", str(root), "--target", "linux-x64", "--download-only"]
            with mock.patch.object(grok.sys, "argv", argv), mock.patch.object(grok, "fetch"), \
                    mock.patch.object(grok, "verify_binary", return_value={"platform": "linux-x64"}), \
                    mock.patch.object(grok, "verify_version") as version, mock.patch("builtins.print"):
                grok.main()
            version.assert_not_called()
            report = json.loads((root / "grok-fixed-inputs.json").read_text())
            self.assertFalse(report["native_version_verified"])
            self.assertFalse(report["model_input_submitted"])
            with mock.patch.object(grok.sys, "argv", argv[:-1]), self.assertRaises(ValueError):
                grok.main()


if __name__ == "__main__":
    unittest.main()
