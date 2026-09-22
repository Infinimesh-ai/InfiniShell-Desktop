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


def macho_header(machine=0x0100000C, file_type=2):
    data = bytearray(64)
    struct.pack_into("<IIII", data, 0, 0xFEEDFACF, machine, 0, file_type)
    return bytes(data)


class FixedGrokInputsTests(unittest.TestCase):
    def test_legacy_pins_remain_available_for_explicit_regression(self):
        fixture = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/grok-1.0.30-fixed-platform-inputs.json"
        evidence = json.loads(fixture.read_text(encoding="utf-8"))
        releases = grok.releases_for(grok.LEGACY_VERSION)
        self.assertEqual(evidence["source_commit"], grok.LEGACY_SOURCE_COMMIT)
        self.assertEqual(evidence["version"], grok.LEGACY_VERSION)
        self.assertFalse(evidence["official_checksum_manifest_verified"])
        self.assertEqual(set(evidence["platforms"]), set(releases))
        for target, (artifact, name, size, checksum) in releases.items():
            actual = evidence["platforms"][target]
            self.assertEqual((actual["artifact"], actual["binary"], actual["bytes"], actual["sha256"]),
                             (artifact, name, size, checksum))
            self.assertEqual(actual["url"], f"{grok.BASE_URL}/{artifact}")
            self.assertFalse(actual["native_version_verified"])

    def test_default_latest_pins_match_complete_downloads(self):
        fixture = Path(__file__).resolve().parents[2] / "specs/cli-agent-parity/fixtures/grok-1.0.40-fixed-platform-inputs.json"
        evidence = json.loads(fixture.read_text(encoding="utf-8"))
        self.assertEqual(grok.VERSION, "1.0.40")
        self.assertIs(grok.releases_for(grok.VERSION), grok.RELEASES)
        self.assertEqual(evidence["version"], grok.VERSION)
        self.assertEqual(evidence["expected_version_output"], grok.VERSION_OUTPUT)
        self.assertFalse(evidence["official_checksum_manifest_verified"])
        self.assertEqual(set(evidence["platforms"]), set(grok.RELEASES))
        for target, (artifact, name, size, checksum) in grok.RELEASES.items():
            actual = evidence["platforms"][target]
            self.assertEqual((actual["artifact"], actual["binary"], actual["bytes"], actual["sha256"]),
                             (artifact, name, size, checksum))
            self.assertEqual(actual["url"], f"{grok.BASE_URL}/{artifact}")
            self.assertEqual(actual["native_version_verified"], target == "darwin-arm64")
            self.assertEqual(actual["target_executed"], target == "darwin-arm64")

    def test_binary_architecture_uses_actual_headers(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "binary"
            for target, data, expected in [("linux-x64", elf_header(), "elf64-x86-64"),
                                           ("win32-x64", pe_header(), "pe32+-x86-64"),
                                           ("darwin-arm64", macho_header(), "mach-o64-arm64")]:
                path.write_bytes(data)
                self.assertEqual(grok.binary_format(path, target), expected)
            for target, data in [("linux-x64", elf_header(183)), ("win32-x64", pe_header(0xAA64)),
                                 ("linux-x64", pe_header()), ("win32-x64", elf_header()),
                                 ("win32-x64", b"MZ"), ("linux-x64", b"\x7fELF"),
                                 ("darwin-arm64", macho_header(0x01000007)),
                                 ("darwin-arm64", macho_header(file_type=6)),
                                 ("darwin-arm64", b"\xcf\xfa\xed\xfe"),
                                 ("darwin-arm64", elf_header())]:
                with self.subTest(target=target, bytes=len(data)):
                    path.write_bytes(data)
                    with self.assertRaises(ValueError):
                        grok.binary_format(path, target)

    def test_platform_detection_requires_selected_version_platform_pair(self):
        for system, machine, version, expected in [
                ("linux", "x86_64", "1.0.30", "linux-x64"),
                ("win32", "AMD64", "1.0.34", "win32-x64"),
                ("darwin", "arm64", "1.0.40", "darwin-arm64"),
                ("darwin", "aarch64", "1.0.40", "darwin-arm64")]:
            with self.subTest(system=system, machine=machine, version=version), \
                    mock.patch.object(grok.sys, "platform", system), \
                    mock.patch.object(grok.platform, "machine", return_value=machine):
                self.assertEqual(grok.current_platform(version), expected)
        for system, machine, version in [("darwin", "arm64", "1.0.30"),
                                         ("darwin", "x86_64", "1.0.34"),
                                         ("linux", "aarch64", "1.0.34"),
                                         ("win32", "ARM64", "1.0.34"),
                                         ("linux", "x86_64", "1.0.35")]:
            with self.subTest(system=system, machine=machine, version=version), \
                    mock.patch.object(grok.sys, "platform", system), \
                    mock.patch.object(grok.platform, "machine", return_value=machine), \
                    self.assertRaises(ValueError):
                grok.current_platform(version)

    def test_binary_digest_remains_bound_to_selected_version_and_platform(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "grok"
            payloads = {"1.0.30": elf_header() + b"30", "1.0.34": elf_header() + b"34",
                        "1.0.40": elf_header() + b"40"}
            recipes = {version: {"linux-x64": (f"grok-{version}-linux-x86_64", "grok", len(payload),
                                               hashlib.sha256(payload).hexdigest())}
                       for version, payload in payloads.items()}
            with mock.patch.object(grok, "VERSION_RELEASES", recipes):
                for actual_version, payload in payloads.items():
                    path.write_bytes(payload)
                    self.assertEqual(grok.verify_binary(path, "linux-x64", actual_version)["sha256"],
                                     hashlib.sha256(payload).hexdigest())
                    for other_version in set(payloads) - {actual_version}:
                        with self.assertRaises(ValueError):
                            grok.verify_binary(path, "linux-x64", other_version)
                    with self.assertRaises(ValueError):
                        grok.verify_binary(path, "win32-x64", actual_version)
                    path.write_bytes(payload + b"extra")
                    with self.assertRaises(ValueError):
                        grok.verify_binary(path, "linux-x64", actual_version)

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

    def test_version_markers_cannot_adopt_the_other_version_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for version in grok.VERSION_RELEASES:
                directory = grok.owned_directory(root / version, explicit_private=True, version=version)
                marker = directory / grok.MARKER
                before = marker.read_bytes()
                self.assertEqual(before, f"isolated Grok {version} verification inputs\n".encode())
                self.assertEqual(grok.owned_directory(directory, True, version), directory)
                for other_version in set(grok.VERSION_RELEASES) - {version}:
                    with self.assertRaises(ValueError):
                        grok.owned_directory(directory, True, other_version)
                self.assertEqual(marker.read_bytes(), before)
            unknown = root / "unknown"
            with self.assertRaises(ValueError):
                grok.owned_directory(unknown, True, "latest")
            self.assertFalse(unknown.exists())

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

    def test_transient_network_failure_retries_without_publishing_a_partial_file(self):
        payload = elf_header()
        checksum = hashlib.sha256(payload).hexdigest()
        url = f"{grok.BASE_URL}/{grok.RELEASES['linux-x64'][0]}"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "grok"
            with mock.patch.object(grok.urllib.request, "urlopen",
                                   side_effect=[TimeoutError("fixture-timeout"), response(payload, url)]) as opened, \
                    mock.patch.object(grok.time, "sleep") as sleep:
                grok.fetch(url, path, len(payload), checksum)
            self.assertEqual(opened.call_count, 2)
            sleep.assert_called_once_with(2)
            self.assertEqual(path.read_bytes(), payload)
            self.assertEqual([item.name for item in path.parent.iterdir()], ["grok"])

    def test_bad_download_redirect_and_modified_cache_fail_without_replacement(self):
        url = f"{grok.BASE_URL}/{grok.RELEASES['linux-x64'][0]}"
        checksum = hashlib.sha256(b"yes").hexdigest()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "grok"
            for payload, actual_url in [(b"too long", url), (b"bad", url), (b"y", url),
                                         (b"yes", "https://example.invalid/unreviewed")]:
                with self.subTest(payload=payload, actual_url=actual_url), mock.patch.object(
                        grok.urllib.request, "urlopen", return_value=response(payload, actual_url)) as opened, \
                        mock.patch.object(grok.time, "sleep") as sleep:
                    with self.assertRaises(ValueError):
                        grok.fetch(url, path, 3, checksum)
                opened.assert_called_once()
                sleep.assert_not_called()
                self.assertFalse(list(path.parent.iterdir()))
            path.write_bytes(b"user changes")
            with self.assertRaises(ValueError):
                grok.fetch(url, path, 3, checksum)
            self.assertEqual(path.read_bytes(), b"user changes")

    def test_download_url_must_belong_to_selected_version_before_network_or_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "grok"
            for version, url in [("1.0.30", f"{grok.BASE_URL}/grok-1.0.40-linux-x86_64"),
                                 ("1.0.34", f"{grok.BASE_URL}/grok-1.0.30-linux-x86_64"),
                                 ("1.0.40", f"{grok.BASE_URL}/grok-1.0.34-linux-x86_64"),
                                 ("1.0.40", f"{grok.BASE_URL}/grok-1.0.40-linux-aarch64"),
                                 ("latest", f"{grok.BASE_URL}/grok-1.0.40-linux-x86_64")]:
                with self.subTest(version=version, url=url), \
                        mock.patch.object(grok.urllib.request, "urlopen") as opened:
                    with self.assertRaises(ValueError):
                        grok.fetch(url, path, 0, hashlib.sha256(b"").hexdigest(), version)
                    opened.assert_not_called()
                self.assertFalse(list(path.parent.iterdir()))

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

            for version, expected_output in grok.VERSION_OUTPUTS.items():
                def run(argv, **kwargs):
                    self.assertEqual(argv, [str(executable), "--version"])
                    self.assertTrue(kwargs["cwd"].is_relative_to(root))
                    self.assertEqual(kwargs["timeout"], 10)
                    self.assertNotIn("XAI_API_KEY", kwargs["env"])
                    return subprocess.CompletedProcess(argv, 0, expected_output + "\n", "")

                with self.subTest(version=version), mock.patch.object(grok.subprocess, "run", side_effect=run):
                    self.assertEqual(grok.verify_version(executable, root, version), expected_output)

    def test_version_output_rejects_other_supported_version_and_unknown_before_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            executable = root / "grok"
            for version in grok.VERSION_OUTPUTS:
                other_output = next(output for other, output in grok.VERSION_OUTPUTS.items()
                                    if other != version)
                for output in [other_output, f"grok {version} (unexpected-build)", f"grok {version}"]:
                    with self.subTest(version=version, output=output), mock.patch.object(
                            grok.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output, "")):
                        with self.assertRaises(ValueError):
                            grok.verify_version(executable, root, version)
            with mock.patch.object(grok.subprocess, "run") as run, self.assertRaises(ValueError):
                grok.verify_version(executable, root, "1.0.35")
            run.assert_not_called()

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
            self.assertEqual(report["version"], grok.VERSION)
            self.assertFalse(report["native_version_verified"])
            self.assertFalse(report["model_input_submitted"])
            with mock.patch.object(grok.sys, "argv", argv[:-1]), self.assertRaises(ValueError):
                grok.main()

    def test_explicit_34_download_only_binds_recipe_and_never_runs_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            for target, (artifact, name, size, checksum) in grok.releases_for("1.0.34").items():
                root = Path(temporary).resolve() / target
                argv = ["prepare_grok_cli.py", "--private-directory", str(root), "--version", "1.0.34",
                        "--target", target, "--download-only"]
                with self.subTest(target=target), mock.patch.object(grok.sys, "argv", argv), \
                        mock.patch.object(grok, "fetch") as fetch, \
                        mock.patch.object(grok, "verify_binary", return_value={"platform": target}) as verify, \
                        mock.patch.object(grok, "verify_version") as version, mock.patch("builtins.print"):
                    grok.main()
                version.assert_not_called()
                fetch.assert_called_once_with(f"{grok.BASE_URL}/{artifact}", root / name, size, checksum, "1.0.34")
                verify.assert_called_once_with(root / name, target, "1.0.34")
                report = json.loads((root / "grok-fixed-inputs.json").read_text(encoding="utf-8"))
                self.assertEqual(report["version"], "1.0.34")
                self.assertIsNone(report["version_output"])
                for key in ("native_version_verified", "installed", "credentials_provided", "model_input_submitted"):
                    self.assertFalse(report[key])

    def test_explicit_34_native_mode_propagates_version_to_both_byte_checks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "native"
            argv = ["prepare_grok_cli.py", "--private-directory", str(root), "--version", "1.0.34"]
            with mock.patch.object(grok.sys, "argv", argv), mock.patch.object(grok, "fetch"), \
                    mock.patch.object(grok, "current_platform", return_value="darwin-arm64") as platform, \
                    mock.patch.object(grok, "verify_binary", return_value={"platform": "darwin-arm64"}) as verify, \
                    mock.patch.object(grok, "verify_version", return_value=grok.VERSION_OUTPUTS["1.0.34"]) as version, \
                    mock.patch("builtins.print"):
                grok.main()
            platform.assert_called_once_with("1.0.34")
            version.assert_called_once_with(root / "grok", root, "1.0.34")
            self.assertEqual(verify.call_args_list, [mock.call(root / "grok", "darwin-arm64", "1.0.34")] * 2)
            report = json.loads((root / "grok-fixed-inputs.json").read_text(encoding="utf-8"))
            self.assertTrue(report["native_version_verified"])
            self.assertEqual(report["version_output"], grok.VERSION_OUTPUTS["1.0.34"])

    def test_unknown_version_and_unsupported_target_fail_before_directory_or_download(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "must-not-create"
            base = ["prepare_grok_cli.py", "--private-directory", str(root)]
            for args, error in [(["--version", "latest", "--download-only"], SystemExit),
                                (["--version", "1.0.35", "--download-only"], SystemExit),
                                (["--version", "1.0.30", "--target", "darwin-arm64", "--download-only"], ValueError),
                                (["--version", "1.0.34", "--target", "linux-x64"], ValueError)]:
                with self.subTest(args=args), mock.patch.object(grok.sys, "argv", base + args), \
                        mock.patch.object(grok.sys, "stderr", io.StringIO()), \
                        mock.patch.object(grok, "fetch") as fetch, \
                        mock.patch.object(grok.subprocess, "run") as run, self.assertRaises(error):
                    grok.main()
                fetch.assert_not_called()
                run.assert_not_called()
                self.assertFalse(root.exists())


if __name__ == "__main__":
    unittest.main()
