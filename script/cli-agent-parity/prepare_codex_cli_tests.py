#!/usr/bin/env python3
"""完整固定运行包的提取、缓存和失败恢复边界；不执行异平台文件。"""

import hashlib
import io
import json
import os
from contextlib import redirect_stderr, redirect_stdout
from copy import deepcopy
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import prepare_codex_cli as prepare


class RuntimeExtractionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="codex-package-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.destination = self.root / "runtime"
        self.target = "fixture"
        self.package = {
            "target": "fixture-target", "entrypoint": "bin/codex",
            "directories": {"bin": 0o755, "codex-path": 0o755, "codex-resources": 0o755},
            "files": {},
        }
        self.bodies = {
            "bin/codex": b"fixed-main",
            "bin/codex-code-mode-host": b"fixed-tool-host",
            "codex-path/rg": b"fixed-search",
            "codex-resources/helper": b"fixed-resource",
            "codex-package.json": json.dumps(prepare.expected_metadata(self.package)).encode(),
        }
        for name, body in self.bodies.items():
            self.package["files"][name] = (len(body), hashlib.sha256(body).hexdigest(),
                                            0o644 if name.endswith(".json") else 0o755)
        self.entries = [(name, tarfile.DIRTYPE, b"", mode)
                        for name, mode in self.package["directories"].items()]
        self.entries += [(name, tarfile.REGTYPE, body, self.package["files"][name][2])
                         for name, body in self.bodies.items()]
        self.archive = self.root / "fixture.tar.gz"
        self.write_archive(self.entries)
        self.patcher = patch.dict(prepare.PACKAGES, {self.target: self.package})
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def write_archive(self, entries):
        with tarfile.open(self.archive, "w:gz") as output:
            for name, kind, body, mode in entries:
                member = tarfile.TarInfo(name)
                member.type, member.mode = kind, mode
                member.linkname = "outside" if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE) else ""
                member.size = len(body) if kind == tarfile.REGTYPE else 0
                output.addfile(member, io.BytesIO(body) if member.size else None)
        self.package["bytes"] = self.archive.stat().st_size
        self.package["sha256"] = hashlib.sha256(self.archive.read_bytes()).hexdigest()

    def extract(self, version=prepare.CODEX_VERSION):
        return prepare.extract_runtime_package(self.archive, self.destination, self.target, version)

    def latest_fixture(self, target, version="0.155.1"):
        self.target = target
        self.package = deepcopy(prepare.packages_for_version(version)[target])
        self.bodies = {name: (json.dumps(self.package["metadata"]).encode()
                             if name == "codex-package.json" else ("fixture:" + name).encode())
                       for name in self.package["files"]}
        self.package["files"] = {name: (len(body), hashlib.sha256(body).hexdigest(),
                                      self.package["files"][name][2])
                                 for name, body in self.bodies.items()}
        self.entries = [(name, tarfile.DIRTYPE, b"", mode)
                        for name, mode in self.package["directories"].items()]
        self.entries += [(name, tarfile.REGTYPE, body, self.package["files"][name][2])
                         for name, body in self.bodies.items()]
        self.write_archive(self.entries)

    def assert_rejected_without_partial_publication(self):
        with self.assertRaises((ValueError, OSError)):
            self.extract()
        self.assertFalse(self.destination.exists())
        self.assertEqual({p.name for p in self.root.iterdir()}, {"fixture.tar.gz"})

    def test_complete_runtime_preserves_helpers_metadata_and_reuses_exact_cache(self):
        self.assertEqual(self.extract(), self.destination / "bin/codex")
        for name, body in self.bodies.items():
            path = self.destination / name
            self.assertEqual(path.read_bytes(), body)
            if os.name != "nt":
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), self.package["files"][name][2] & 0o700)
        before = {p: p.stat().st_mtime_ns for p in self.destination.rglob("*")}
        self.assertEqual(self.extract(), self.destination / "bin/codex")
        self.assertEqual(before, {p: p.stat().st_mtime_ns for p in before})

    def test_all_three_official_layouts_keep_every_helper_and_resource(self):
        for target in ("linux-x64", "windows-x64", "windows-arm64"):
            with self.subTest(target=target):
                self.target = target
                self.destination = self.root / target
                self.package = deepcopy(prepare.PACKAGES[target])
                bodies = {name: (json.dumps(prepare.expected_metadata(self.package)).encode()
                                 if name == "codex-package.json" else ("fixture:" + name).encode())
                          for name in self.package["files"]}
                self.package["files"] = {name: (len(body), hashlib.sha256(body).hexdigest(),
                                              self.package["files"][name][2])
                                         for name, body in bodies.items()}
                entries = [(name, tarfile.DIRTYPE, b"", mode)
                           for name, mode in self.package["directories"].items()]
                entries += [(name, tarfile.REGTYPE, body, self.package["files"][name][2])
                            for name, body in bodies.items()]
                self.write_archive(entries)
                with patch.dict(prepare.PACKAGES, {target: self.package}):
                    self.assertEqual(self.extract(), self.destination / self.package["entrypoint"])
                self.assertEqual({p.relative_to(self.destination).as_posix(): p.read_bytes()
                                  for p in self.destination.rglob("*") if p.is_file()}, bodies)

    def test_missing_tool_host_is_rejected_even_when_main_program_is_present(self):
        self.write_archive([entry for entry in self.entries if entry[0] != "bin/codex-code-mode-host"])
        self.assert_rejected_without_partial_publication()

    def test_duplicate_member_is_rejected(self):
        self.write_archive(self.entries + [self.entries[-1]])
        self.assert_rejected_without_partial_publication()

    def test_paths_outside_canonical_fixed_layout_are_rejected(self):
        for name in ("../outside", "/outside", "bin/../outside", "bin//extra", "./bin/extra",
                     "bin\\extra", "C:/outside", "bin/extra", "bin/extra:stream"):
            with self.subTest(name=name):
                self.write_archive(self.entries + [(name, tarfile.REGTYPE, b"extra", 0o755)])
                self.assert_rejected_without_partial_publication()

    def test_link_or_device_at_expected_entry_is_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.CHRTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind):
                entries = [entry if entry[0] != "bin/codex" else (entry[0], kind, b"", entry[3])
                           for entry in self.entries]
                self.write_archive(entries)
                self.assert_rejected_without_partial_publication()

    def test_member_size_hash_and_mode_must_all_match_fixed_input(self):
        for body, mode in ((b"short", 0o755), (b"wrong-main", 0o755), (b"fixed-main", 0o4755)):
            with self.subTest(body=body, mode=mode):
                entries = [entry if entry[0] != "bin/codex" else (entry[0], entry[1], body, mode)
                           for entry in self.entries]
                self.write_archive(entries)
                self.assert_rejected_without_partial_publication()

    def test_archive_digest_and_size_checked_before_any_publication(self):
        for key, value in (("sha256", "0" * 64), ("bytes", self.package["bytes"] + 1)):
            with self.subTest(key=key), patch.dict(self.package, {key: value}):
                self.assert_rejected_without_partial_publication()

    def test_bounds_reject_oversized_members_totals_and_counts(self):
        for name, limit in (("MAX_FILE_BYTES", 1), ("MAX_TOTAL_BYTES", 1), ("MAX_MEMBERS", 1)):
            with self.subTest(name=name), patch.object(prepare, name, limit):
                self.assert_rejected_without_partial_publication()

    def test_cached_modified_helper_is_preserved_and_rejected(self):
        self.extract()
        host = self.destination / "bin/codex-code-mode-host"
        host.write_bytes(b"locally-modified")
        with self.assertRaises(ValueError):
            self.extract()
        self.assertEqual(host.read_bytes(), b"locally-modified")

    @unittest.skipIf(os.name == "nt", "Windows ACL 不能由 POSIX mode 回归证明")
    def test_world_writable_runtime_root_or_bin_cache_is_rejected_unchanged(self):
        self.extract()
        files_before = {name: (self.destination / name).read_bytes() for name in self.bodies}
        for directory in (self.destination, self.destination / "bin"):
            with self.subTest(directory=directory.name):
                directory.chmod(0o777)
                try:
                    with self.assertRaisesRegex(ValueError, "权限为0700"):
                        self.extract()
                    self.assertEqual(stat.S_IMODE(directory.stat().st_mode), 0o777)
                    self.assertEqual({name: (self.destination / name).read_bytes()
                                      for name in self.bodies}, files_before)
                finally:
                    directory.chmod(0o700)

    @unittest.skipIf(os.name == "nt", "Windows ACL 不能由 POSIX mode 回归证明")
    def test_world_writable_download_parent_rejects_existing_runtime_unchanged(self):
        self.extract()
        self.root.chmod(0o777)
        try:
            with self.assertRaisesRegex(ValueError, "权限为0700"):
                self.extract()
            self.assertEqual(stat.S_IMODE(self.root.stat().st_mode), 0o777)
            self.assertEqual((self.destination / "bin/codex").read_bytes(), self.bodies["bin/codex"])
        finally:
            self.root.chmod(0o700)

    @unittest.skipIf(os.name == "nt", "Windows ACL 不能由 POSIX mode 回归证明")
    def test_writable_ancestor_requires_sticky_protection_for_owned_child(self):
        ancestor = self.root / "shared"
        ancestor.mkdir()
        download = ancestor / "private-download"
        download.mkdir(mode=0o700)
        self.destination = download / "runtime"
        ancestor.chmod(0o777)
        with self.assertRaisesRegex(ValueError, "父目录"):
            self.extract()
        self.assertFalse(self.destination.exists())
        self.assertEqual(stat.S_IMODE(ancestor.stat().st_mode), 0o777)
        ancestor.chmod(0o1777)
        self.assertEqual(self.extract(), self.destination / "bin/codex")

    @unittest.skipIf(os.name == "nt", "Windows 所有者需要真实 ACL 证据")
    def test_foreign_directory_owner_is_rejected_even_with_private_or_sticky_mode(self):
        original_lstat = Path.lstat
        foreign_uid = os.geteuid() + 1
        observed_mode = 0o700

        def observed_lstat(path, *args, **kwargs):
            result = original_lstat(path, *args, **kwargs)
            if path == self.root:
                return SimpleNamespace(st_mode=stat.S_IFDIR | observed_mode, st_uid=foreign_uid)
            return result

        with patch.object(Path, "lstat", observed_lstat):
            for observed_mode in (0o700, 0o1777):
                with self.subTest(mode=observed_mode):
                    with self.assertRaisesRegex(ValueError, "当前用户拥有"):
                        prepare.directory_without_links(self.root, private=True)
                    with self.assertRaisesRegex(ValueError, "父目录"):
                        prepare.directory_without_links(self.root)
        self.assertFalse(self.destination.exists())

    @unittest.skipUnless(sys.platform == "darwin", "只验证 macOS 已知系统 /tmp 别名")
    def test_macos_system_tmp_alias_keeps_private_cache_usable(self):
        with tempfile.TemporaryDirectory(prefix="codex-system-tmp-", dir="/tmp") as temporary:
            self.destination = Path(temporary) / "runtime"
            entry = self.extract()
            self.assertEqual(entry, self.destination / "bin/codex")
            self.assertEqual(self.extract(), entry)
            self.assertEqual(entry.read_bytes(), self.bodies["bin/codex"])

    def test_missing_or_extra_cached_entries_are_rejected_without_repair(self):
        self.extract()
        host = self.destination / "bin/codex-code-mode-host"
        host.unlink()
        with self.assertRaises(ValueError):
            self.extract()
        self.assertFalse(host.exists())
        host.write_bytes(self.bodies["bin/codex-code-mode-host"])
        host.chmod(0o700)
        extra = self.destination / "local-user-file"
        extra.write_bytes(b"preserve")
        with self.assertRaises(ValueError):
            self.extract()
        self.assertEqual(extra.read_bytes(), b"preserve")

    def test_existing_preparation_lock_does_not_get_deleted_or_overwritten(self):
        lock = self.destination.with_name("runtime.prepare-lock")
        lock.mkdir()
        marker = lock / "owner"
        marker.write_bytes(b"other-preparer")
        with self.assertRaises(FileExistsError):
            self.extract()
        self.assertEqual(marker.read_bytes(), b"other-preparer")
        self.assertFalse(self.destination.exists())

    def test_old_standalone_binary_is_never_adopted_or_changed(self):
        old = self.root / "codex.exe"
        old.write_bytes(b"old-incomplete-runtime")
        self.assertEqual(self.extract(), self.destination / "bin/codex")
        self.assertEqual(old.read_bytes(), b"old-incomplete-runtime")

    def test_write_failure_does_not_publish_partial_runtime(self):
        with patch.object(prepare.os, "fsync", side_effect=OSError("fixture-write-failed")):
            self.assert_rejected_without_partial_publication()

    def test_cleanup_failure_keeps_original_error_and_releases_owned_lock(self):
        self.write_archive([entry for entry in self.entries if entry[0] != "bin/codex-code-mode-host"])
        with patch.object(prepare.shutil, "rmtree", side_effect=OSError("fixture-cleanup-failed")) as remove, \
                patch.object(prepare.time, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "完整归档有缺失成员") as failure:
                self.extract()
        self.assertEqual(remove.call_count, prepare.STAGING_CLEANUP_ATTEMPTS)
        self.assertEqual(sleep.call_count, prepare.STAGING_CLEANUP_ATTEMPTS - 1)
        sleep.assert_any_call(0.05)
        sleep.assert_any_call(0.05 * (prepare.STAGING_CLEANUP_ATTEMPTS - 1))
        self.assertIn("fixture-cleanup-failed", " ".join(failure.exception.__notes__))
        self.assertFalse(self.destination.exists())
        self.assertFalse(self.destination.with_name("runtime.prepare-lock").exists())
        self.assertEqual(len(list(self.root.glob(".codex-package-*"))), 1)

    def test_transient_staging_cleanup_failure_is_retried(self):
        self.write_archive([entry for entry in self.entries if entry[0] != "bin/codex-code-mode-host"])
        original = shutil.rmtree
        calls = []

        def transient(path):
            calls.append(path)
            if len(calls) == 1:
                raise PermissionError("fixture-transient-sharing")
            original(path)

        with patch.object(prepare.shutil, "rmtree", side_effect=transient), \
                patch.object(prepare.time, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "完整归档有缺失成员"):
                self.extract()
        self.assertEqual(len(calls), 2)
        sleep.assert_called_once_with(0.05)
        self.assertFalse(self.destination.exists())
        self.assertFalse(self.destination.with_name("runtime.prepare-lock").exists())
        self.assertEqual({p.name for p in self.root.iterdir()}, {"fixture.tar.gz"})

    def test_metadata_must_describe_actual_fixed_layout(self):
        body = b'{"layoutVersion":2}'
        self.package["files"]["codex-package.json"] = (len(body), hashlib.sha256(body).hexdigest(), 0o644)
        self.write_archive([entry if entry[0] != "codex-package.json" else (entry[0], entry[1], body, entry[3])
                            for entry in self.entries])
        self.assert_rejected_without_partial_publication()

    @unittest.skipIf(os.name == "nt", "真实 Windows 重解析点由原生平台单独验证，不假定有建链接权限")
    def test_symlink_destination_or_parent_does_not_write_outside(self):
        outside = self.root / "outside"
        outside.mkdir()
        self.destination.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.extract()
        self.destination.unlink()
        linked_parent = self.root / "linked"
        linked_parent.symlink_to(outside, target_is_directory=True)
        self.destination = linked_parent / "runtime"
        with self.assertRaises(ValueError):
            self.extract()
        self.assertEqual(list(outside.iterdir()), [])

    @unittest.skipIf(os.name == "nt", "真实 Windows 重解析点由原生平台单独验证，不假定有建链接权限")
    def test_cached_symlink_and_hardlink_are_rejected_without_modifying_target(self):
        self.extract()
        host = self.destination / "bin/codex-code-mode-host"
        outside = self.root / "retained-helper"
        outside.write_bytes(host.read_bytes())
        host.unlink()
        host.symlink_to(outside)
        with self.assertRaises(ValueError):
            self.extract()
        host.unlink()
        os.link(outside, host)
        with self.assertRaises(ValueError):
            self.extract()
        self.assertEqual(outside.read_bytes(), self.bodies["bin/codex-code-mode-host"])

    def test_explicit_legacy_returns_complete_runtime_entrypoint_on_supported_architectures(self):
        for platform_name, machine, target in (("linux", "x86_64", "linux-x64"),
                                                ("win32", "AMD64", "windows-x64"),
                                                ("win32", "ARM64", "windows-arm64")):
            with self.subTest(target=target):
                download = self.root / target
                package = dict(self.package, archive="fixture.tar.gz")

                def fetch(url, destination, digest, size):
                    self.assertEqual(url, "https://github.com/openai/codex/releases/download/rust-v0.147.0/fixture.tar.gz")
                    self.assertEqual((digest, size), (package["sha256"], package["bytes"]))
                    shutil.copyfile(self.archive, destination)

                stdout = io.StringIO()
                with patch.dict(prepare.PACKAGES, {target: package}), \
                        patch.object(prepare.sys, "platform", platform_name), \
                        patch.object(prepare.platform, "machine", return_value=machine), \
                        patch.dict(os.environ, {"RUNNER_TEMP": str(self.root)}, clear=True), \
                        patch.object(sys, "argv", ["prepare_codex_cli.py", "--version", "0.147.0", "--download-dir", str(download)]), \
                        patch.object(prepare, "fetch_file", side_effect=fetch), \
                        patch.object(prepare, "verified_version") as version, redirect_stdout(stdout):
                    prepare.main()
                executable = download / f"runtime-0.147.0-{target}/bin/codex"
                self.assertEqual(stdout.getvalue(), f"{executable}\n")
                version.assert_called_once_with(executable, self.root, "0.147.0")
                self.assertTrue(executable.with_name("codex-code-mode-host").is_file())


    def test_latest_four_complete_layouts_reuse_exact_cache_with_expanded_resources(self):
        for target in ("linux-x64", "windows-x64", "windows-arm64", "macos-arm64"):
            with self.subTest(target=target):
                self.latest_fixture(target)
                self.destination = self.root / target
                with patch.dict(prepare.packages_for_version("0.155.1"), {target: self.package}):
                    self.assertEqual(self.extract("0.155.1"), self.destination / self.package["entrypoint"])
                    before = {p: p.stat().st_mtime_ns for p in self.destination.rglob("*")}
                    self.extract("0.155.1")
                    self.assertEqual(before, {p: p.stat().st_mtime_ns for p in before})
                self.assertEqual({p.relative_to(self.destination).as_posix(): p.read_bytes()
                                  for p in self.destination.rglob("*") if p.is_file()}, self.bodies)

    def test_latest_archive_cannot_supply_legacy_metadata_even_with_matching_file_hash(self):
        self.latest_fixture("linux-x64")
        metadata = dict(self.package["metadata"], version="0.147.0")
        body = json.dumps(metadata).encode()
        self.package["files"]["codex-package.json"] = (len(body), hashlib.sha256(body).hexdigest(), 0o644)
        self.write_archive([entry if entry[0] != "codex-package.json"
                            else (entry[0], entry[1], body, entry[3]) for entry in self.entries])
        with patch.dict(prepare.packages_for_version("0.155.1"), {self.target: self.package}):
            with self.assertRaisesRegex(ValueError, "布局元数据"):
                self.extract("0.155.1")
        self.assertFalse(self.destination.exists())
        self.assertEqual({p.name for p in self.root.iterdir()}, {"fixture.tar.gz"})

    def test_latest_nested_resource_tampering_is_not_silently_repaired(self):
        self.latest_fixture("linux-x64")
        with patch.dict(prepare.packages_for_version("0.155.1"), {self.target: self.package}):
            self.extract("0.155.1")
            resource = self.destination / "codex-resources/voice/runtime.json"
            resource.write_bytes(b"preserve user change")
            with self.assertRaises(ValueError):
                self.extract("0.155.1")
            self.assertEqual(resource.read_bytes(), b"preserve user change")

    def test_optional_01561_windows_voice_runtime_is_complete_and_tampering_is_rejected(self):
        for target in ("windows-x64", "windows-arm64"):
            with self.subTest(target=target):
                self.latest_fixture(target, "0.156.1")
                self.destination = self.root / target
                with patch.dict(prepare.packages_for_version("0.156.1"), {target: self.package}):
                    self.assertEqual(self.extract("0.156.1"),
                                     self.destination / self.package["entrypoint"])
                    voice = self.destination / "codex-resources/voice/bin/codex-voice-host.exe"
                    self.assertTrue(voice.is_file())
                    voice.write_bytes(b"modified voice runtime")
                    with self.assertRaisesRegex(ValueError, "大小或摘要不匹配"):
                        self.extract("0.156.1")
                    self.assertEqual(voice.read_bytes(), b"modified voice runtime")

    def test_optional_01561_four_platforms_use_explicit_version_cache(self):
        for platform_name, machine, target in (("linux", "x86_64", "linux-x64"),
                                               ("win32", "AMD64", "windows-x64"),
                                               ("win32", "ARM64", "windows-arm64"),
                                               ("darwin", "arm64", "macos-arm64")):
            with self.subTest(target=target):
                self.latest_fixture(target, "0.156.1")
                download = self.root / target

                def fetch(url, destination, digest, size):
                    self.assertEqual(url, f"https://github.com/openai/codex/releases/download/rust-v0.156.1/{self.package['archive']}")
                    self.assertEqual(destination.name, "0.156.1-" + self.package["archive"])
                    self.assertEqual((digest, size), (self.package["sha256"], self.package["bytes"]))
                    shutil.copyfile(self.archive, destination)

                stdout = io.StringIO()
                with patch.dict(prepare.packages_for_version("0.156.1"), {target: self.package}), \
                        patch.object(prepare.sys, "platform", platform_name), \
                        patch.object(prepare.platform, "machine", return_value=machine), \
                        patch.dict(os.environ, {"RUNNER_TEMP": str(self.root)}, clear=True), \
                        patch.object(sys, "argv", ["prepare_codex_cli.py", "--version", "0.156.1",
                                                "--download-dir", str(download)]), \
                        patch.object(prepare, "fetch_file", side_effect=fetch), \
                        patch.object(prepare, "verified_version") as version, redirect_stdout(stdout):
                    prepare.main()
                executable = download / f"runtime-0.156.1-{target}" / self.package["entrypoint"]
                self.assertEqual(stdout.getvalue(), f"{executable}\n")
                version.assert_called_once_with(executable, self.root, "0.156.1")

    def test_main_keeps_01551_default_and_legacy_archive(self):
        for platform_name, machine, target in (("linux", "x86_64", "linux-x64"),
                                               ("win32", "AMD64", "windows-x64"),
                                               ("win32", "ARM64", "windows-arm64"),
                                               ("darwin", "arm64", "macos-arm64")):
            with self.subTest(target=target):
                self.latest_fixture(target)
                download = self.root / target
                download.mkdir(mode=0o700)
                legacy_archive = download / self.package["archive"]
                legacy_archive.write_bytes(b"existing legacy cache")

                def fetch(url, destination, digest, size):
                    self.assertEqual(url, f"https://github.com/openai/codex/releases/download/rust-v0.155.1/{self.package['archive']}")
                    self.assertEqual(destination.name, "0.155.1-" + self.package["archive"])
                    self.assertEqual((digest, size), (self.package["sha256"], self.package["bytes"]))
                    shutil.copyfile(self.archive, destination)

                stdout = io.StringIO()
                with patch.dict(prepare.packages_for_version("0.155.1"), {target: self.package}), \
                        patch.object(prepare.sys, "platform", platform_name), \
                        patch.object(prepare.platform, "machine", return_value=machine), \
                        patch.dict(os.environ, {"RUNNER_TEMP": str(self.root)}, clear=True), \
                        patch.object(sys, "argv", ["prepare_codex_cli.py", "--download-dir", str(download)]), \
                        patch.object(prepare, "fetch_file", side_effect=fetch), \
                        patch.object(prepare, "verified_version") as version, redirect_stdout(stdout):
                    prepare.main()
                executable = download / f"runtime-0.155.1-{target}" / self.package["entrypoint"]
                self.assertEqual(stdout.getvalue(), f"{executable}\n")
                version.assert_called_once_with(executable, self.root, "0.155.1")
                self.assertEqual(legacy_archive.read_bytes(), b"existing legacy cache")


class FixedRuntimeVersionTests(unittest.TestCase):
    def test_windows_main_digests_match_previous_fixed_native_inputs(self):
        from codex_windows_hook_inputs import RELEASE_ASSETS
        for target, architecture in (("windows-x64", "x86_64"), ("windows-arm64", "aarch64")):
            with self.subTest(target=target):
                size, digest = RELEASE_ASSETS[architecture][1:]
                self.assertEqual(prepare.PACKAGES[target]["files"]["bin/codex.exe"][:2], (size, digest))

    def test_version_call_uses_only_private_configuration_without_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            executable = root / "runtime/bin/codex"
            observed = []

            def run(argv, **options):
                self.assertEqual(argv, [str(executable), "--version"])
                env = options["env"]
                observed.append(Path(env["HOME"]).parent)
                for name in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "CODEX_HOME",
                             "TMPDIR", "TEMP", "TMP"):
                    self.assertTrue(Path(env[name]).is_relative_to(root))
                    self.assertTrue(Path(env[name]).is_dir())
                for name in ("OPENAI_API_KEY", "CODEX_API_KEY", "HTTPS_PROXY", "LD_LIBRARY_PATH"):
                    self.assertNotIn(name, env)
                self.assertEqual({key.upper(): value for key, value in env.items() if key.upper() == "SYSTEMROOT"},
                                 {"SYSTEMROOT": "fixture-system-root"})
                self.assertEqual((Path(env["CODEX_HOME"]) / "config.toml").read_text(encoding="utf-8"),
                                 'cli_auth_credentials_store = "file"\n')
                self.assertFalse((Path(env["CODEX_HOME"]) / "auth.json").exists())
                self.assertEqual(options["cwd"], observed[0])
                self.assertEqual(options["timeout"], 10)
                return subprocess.CompletedProcess(argv, 0, "codex-cli 0.147.0\n", "")

            inherited = {"PATH": "fixture-path", "SystemRoot": "fixture-system-root",
                         "HOME": "user-home", "CODEX_HOME": "user-codex",
                         "OPENAI_API_KEY": "never-used", "CODEX_API_KEY": "never-used",
                         "HTTPS_PROXY": "never-used", "LD_LIBRARY_PATH": "never-used"}
            with patch.dict(os.environ, inherited, clear=True), patch.object(prepare.subprocess, "run", side_effect=run):
                prepare.verified_version(executable, root)
            self.assertFalse(observed[0].exists())

    def test_unexpected_reported_version_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(prepare.subprocess, "run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, "codex-cli 0.154.0\n", "")
            with self.assertRaises(ValueError):
                prepare.verified_version(Path(temporary) / "codex", Path(temporary))


class LatestRuntimeVersionTests(unittest.TestCase):
    def test_release_contracts_bind_version_tag_commit_and_cli_output(self):
        self.assertEqual(prepare.DEFAULT_VERSION, "0.155.1")
        self.assertEqual(prepare.release_contract("0.147.0"), {
            "version": "0.147.0", "tag": "rust-v0.147.0",
            "commit": "be6e8eac029b183056b7e4402879f15d2c85f61b", "cli": "codex-cli 0.147.0",
        })
        self.assertEqual(prepare.release_contract("0.155.1"), {
            "version": "0.155.1", "tag": "rust-v0.155.1",
            "commit": "be2951ea34f0d295ed0becf97079f92fa5f6950e", "cli": "codex-cli 0.155.1",
        })
        self.assertEqual(prepare.require_cli_version("codex-cli 0.155.1", "0.155.1")["commit"],
                         "be2951ea34f0d295ed0becf97079f92fa5f6950e")
        self.assertEqual(prepare.release_contract("0.156.1"), {
            "version": "0.156.1", "tag": "rust-v0.156.1",
            "commit": "b412ff32c417f855c2b2d1581b77058eed87c84b", "cli": "codex-cli 0.156.1",
        })
        with self.assertRaisesRegex(ValueError, "CLI 版本"):
            prepare.require_cli_version("codex-cli 0.147.0", "0.155.1")

    def test_windows_asset_identity_is_selected_by_explicit_version(self):
        legacy = prepare.packages_for_version("0.147.0")["windows-x64"]
        current = prepare.packages_for_version("0.155.1")["windows-x64"]
        self.assertEqual(
            (legacy["bytes"], legacy["sha256"]),
            (126777040, "c156c8feb8cb20197bf74d2c6daffed1fec0a8c21a03bc2ca90d7ff81927b0c5"),
        )
        self.assertEqual(
            (current["bytes"], current["sha256"]),
            (139547261, "f45c273b7835c192aaa9cef5b93aa9528966ac7301444632de80a565a9bf14e8"),
        )
        self.assertNotEqual(
            (legacy["bytes"], legacy["sha256"]),
            (current["bytes"], current["sha256"]),
        )

    def test_fixed_manifest_counts_and_metadata_are_per_platform(self):
        packages = prepare.packages_for_version("0.155.1")
        self.assertEqual({key: len(value["files"]) + len(value["directories"])
                          for key, value in packages.items()},
                         {"linux-x64": 54, "windows-x64": 9, "windows-arm64": 9, "macos-arm64": 52})
        for package in packages.values():
            with self.subTest(target=package["target"]):
                self.assertEqual(prepare.expected_metadata(package, "0.155.1"), package["metadata"])
                self.assertEqual(package["metadata"]["version"], "0.155.1")
                self.assertEqual(package["metadata"]["target"], package["target"])

        candidate = prepare.packages_for_version("0.156.1")
        self.assertEqual({key: len(value["files"]) + len(value["directories"])
                          for key, value in candidate.items()},
                         {"linux-x64": 54, "windows-x64": 51, "windows-arm64": 51, "macos-arm64": 52})
        for package in candidate.values():
            with self.subTest(target=package["target"]):
                self.assertEqual(prepare.expected_metadata(package, "0.156.1"), package["metadata"])
                self.assertEqual(package["metadata"]["version"], "0.156.1")

    def test_unknown_version_is_rejected_before_file_or_process_access(self):
        with patch.object(prepare, "regular_file") as regular, patch.object(prepare.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                prepare.packages_for_version("latest")
            with self.assertRaises(ValueError):
                prepare.verified_version(Path("unused"), Path("unused"), "0.156.0")
            regular.assert_not_called()
            run.assert_not_called()

    def test_manifest_tampering_is_rejected_before_using_member_contracts(self):
        prepare.packages_for_version.cache_clear()
        try:
            with patch.object(prepare, "sha256", return_value="0" * 64):
                for version in ("0.155.1", "0.156.1"):
                    with self.subTest(version=version), self.assertRaisesRegex(ValueError, "清单大小或摘要"):
                        prepare.packages_for_version(version)
        finally:
            prepare.packages_for_version.cache_clear()

    def test_reported_version_must_match_the_explicit_selection(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(prepare.subprocess, "run") as run:
            root = Path(temporary)
            run.return_value = subprocess.CompletedProcess([], 0, "codex-cli 0.155.1\n", "")
            prepare.verified_version(root / "codex", root, "0.155.1")
            with self.assertRaisesRegex(ValueError, "CLI 版本"):
                prepare.verified_version(root / "codex", root)
            run.return_value = subprocess.CompletedProcess([], 0, "codex-cli 0.147.0\n", "")
            with self.assertRaisesRegex(ValueError, "CLI 版本"):
                prepare.verified_version(root / "codex", root, "0.155.1")

    def test_macos_legacy_and_unverified_latest_architecture_remain_unavailable(self):
        with patch.object(prepare.sys, "platform", "darwin"), \
                patch.object(prepare.platform, "machine", return_value="x86_64"), \
                patch.object(prepare, "fetch_file") as fetch, redirect_stderr(io.StringIO()):
            with patch.object(sys, "argv", ["prepare_codex_cli.py", "--version", "0.147.0", "--download-dir", "unused"]):
                with self.assertRaises(SystemExit):
                    prepare.main()
            with patch.object(sys, "argv", ["prepare_codex_cli.py", "--download-dir", "unused"]):
                with self.assertRaisesRegex(ValueError, "macOS ARM64"):
                    prepare.main()
            fetch.assert_not_called()


class LatestPackageDownloadTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="codex-latest-download-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.destination = self.root / "0.155.1-package.tar.gz"
        self.body = b"fixed archive fixture"
        self.package = deepcopy(prepare.packages_for_version("0.155.1")["linux-x64"])
        self.package["sha256"] = hashlib.sha256(self.body).hexdigest()
        self.package["bytes"] = len(self.body)
        self.url = f"https://github.com/openai/codex/releases/download/rust-v0.155.1/{self.package['archive']}"
        patcher = patch.dict(prepare.packages_for_version("0.155.1"), {"linux-x64": self.package})
        patcher.start()
        self.addCleanup(patcher.stop)

    def response(self, body):
        response = io.BytesIO(body)
        response.url = "https://release-assets.githubusercontent.com/synthetic-package"
        return response

    def fetch(self):
        prepare.fetch_file(self.url, self.destination, self.package["sha256"], self.package["bytes"])

    def test_exact_new_package_download_and_cache_reuse(self):
        with patch.object(prepare.urllib.request, "urlopen", return_value=self.response(self.body)) as request:
            self.fetch()
            self.fetch()
            request.assert_called_once()
            self.assertEqual(request.call_args.kwargs["timeout"], 60)
        self.assertEqual(self.destination.read_bytes(), self.body)
        self.assertEqual(self.destination.stat().st_nlink, 1)
        self.assertEqual(list(self.root.iterdir()), [self.destination])

    def test_optional_01561_download_requires_its_own_fixed_asset_identity(self):
        package = deepcopy(prepare.packages_for_version("0.156.1")["linux-x64"])
        package["sha256"] = hashlib.sha256(self.body).hexdigest()
        package["bytes"] = len(self.body)
        url = f"https://github.com/openai/codex/releases/download/rust-v0.156.1/{package['archive']}"
        destination = self.root / "0.156.1-package.tar.gz"
        with patch.dict(prepare.packages_for_version("0.156.1"), {"linux-x64": package}), \
                patch.object(prepare.urllib.request, "urlopen", return_value=self.response(self.body)) as request:
            prepare.fetch_file(url, destination, package["sha256"], package["bytes"])
            request.assert_called_once()
            for digest, size in (("0" * 64, package["bytes"]),
                                 (package["sha256"], package["bytes"] + 1)):
                with self.assertRaises(ValueError):
                    prepare.fetch_file(url, self.root / "rejected.tar.gz", digest, size)
        self.assertEqual(destination.read_bytes(), self.body)
        self.assertFalse((self.root / "rejected.tar.gz").exists())

    def test_transient_network_failure_retries_with_a_fresh_temporary_file(self):
        with patch.object(prepare.urllib.request, "urlopen",
                          side_effect=[TimeoutError("fixture-timeout"), self.response(self.body)]) as request, \
                patch.object(prepare.time, "sleep") as sleep:
            self.fetch()
        self.assertEqual(request.call_count, 2)
        sleep.assert_called_once_with(2)
        self.assertEqual(self.destination.read_bytes(), self.body)
        self.assertEqual(list(self.root.iterdir()), [self.destination])

    def test_unknown_url_size_or_digest_cannot_start_a_download(self):
        with patch.object(prepare.urllib.request, "urlopen") as request:
            for url, digest, size in ((self.url.replace("0.155.1", "0.156.0"), self.package["sha256"], self.package["bytes"]),
                                      (self.url, "0" * 64, self.package["bytes"]),
                                      (self.url, self.package["sha256"], self.package["bytes"] + 1)):
                with self.subTest(url=url, size=size):
                    with self.assertRaises(ValueError):
                        prepare.fetch_file(url, self.destination, digest, size)
            request.assert_not_called()
        self.assertEqual(list(self.root.iterdir()), [])

    def test_modified_cached_archive_is_preserved_without_network(self):
        self.destination.write_bytes(b"preserve changed archive")
        with patch.object(prepare.urllib.request, "urlopen") as request:
            with self.assertRaises(ValueError):
                self.fetch()
            request.assert_not_called()
        self.assertEqual(self.destination.read_bytes(), b"preserve changed archive")

    def test_bad_payload_is_never_published(self):
        for body in (b"short", self.body + b"extra", b"x" * len(self.body)):
            with self.subTest(body=body), \
                    patch.object(prepare.urllib.request, "urlopen", return_value=self.response(body)) as request, \
                    patch.object(prepare.time, "sleep") as sleep:
                with self.assertRaises(ValueError):
                    self.fetch()
                request.assert_called_once()
                sleep.assert_not_called()
                self.assertEqual(list(self.root.iterdir()), [])

    def test_plain_http_redirect_is_rejected(self):
        response = self.response(self.body)
        response.url = "http://example.invalid/package"
        with patch.object(prepare.urllib.request, "urlopen", return_value=response):
            with self.assertRaisesRegex(ValueError, "HTTPS"):
                self.fetch()
        self.assertEqual(list(self.root.iterdir()), [])

    def test_concurrent_cache_publication_preserves_the_other_writer(self):
        def publish(source, destination):
            destination.write_bytes(b"other preparer")
            raise FileExistsError("fixture-publish-race")

        with patch.object(prepare.urllib.request, "urlopen", return_value=self.response(self.body)), \
                patch.object(prepare.os, "link", side_effect=publish):
            with self.assertRaises(FileExistsError):
                self.fetch()
        self.assertEqual(self.destination.read_bytes(), b"other preparer")
        self.assertEqual(list(self.root.iterdir()), [self.destination])

    def test_cleanup_failure_does_not_replace_primary_download_failure(self):
        with patch.object(prepare.urllib.request, "urlopen", side_effect=OSError("fixture-network-failed")), \
                patch.object(Path, "unlink", side_effect=OSError("fixture-cleanup-failed")):
            with self.assertRaisesRegex(OSError, "fixture-network-failed") as failure:
                self.fetch()
        self.assertIn("fixture-cleanup-failed", str(failure.exception.__notes__))
        self.assertFalse(self.destination.exists())

    def test_old_downloads_still_use_the_original_fixed_hook_helper(self):
        with patch.object(prepare, "fetch_legacy_file") as legacy, \
                patch.object(prepare.urllib.request, "urlopen") as request:
            prepare.fetch_file("https://github.com/openai/codex/releases/download/rust-v0.147.0/fixed.tar.gz",
                               self.destination, "old-digest", 3)
            legacy.assert_called_once_with("https://github.com/openai/codex/releases/download/rust-v0.147.0/fixed.tar.gz",
                                           self.destination, "old-digest", 3)
            request.assert_not_called()



if __name__ == "__main__":
    unittest.main()
