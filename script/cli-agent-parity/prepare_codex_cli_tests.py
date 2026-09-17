#!/usr/bin/env python3
"""完整固定运行包的提取、缓存和失败恢复边界；不执行异平台文件。"""

import hashlib
import io
import json
import os
from pathlib import Path
import stat
import tarfile
import tempfile
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

    def extract(self):
        return prepare.extract_runtime_package(self.archive, self.destination, self.target)

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



if __name__ == "__main__":
    unittest.main()
