#!/usr/bin/env python3
"""固定 Linux CLI 提取只写预定普通文件，不跟随归档或缓存中的链接。"""

import io
from pathlib import Path
import tarfile
import tempfile
import unittest

from prepare_codex_cli import LINUX_MEMBER, extract_linux_cli


class LinuxExtractionTests(unittest.TestCase):
    def archive(self, directory, entries):
        path = directory / "fixture.tar.gz"
        with tarfile.open(path, "w:gz") as output:
            for name, kind, body in entries:
                entry = tarfile.TarInfo(name)
                entry.type = kind
                entry.linkname = "outside" if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE) else ""
                entry.size = len(body) if kind == tarfile.REGTYPE else 0
                output.addfile(entry, io.BytesIO(body) if entry.size else None)
        return path

    def test_only_fixed_member_is_written_and_verified_cache_is_reusable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = self.archive(root, [(LINUX_MEMBER, tarfile.REGTYPE, b"fixed-cli"),
                                          ("../outside", tarfile.REGTYPE, b"forbidden")])
            destination = root / "codex"
            self.assertEqual(extract_linux_cli(archive, destination).read_bytes(), b"fixed-cli")
            self.assertEqual(extract_linux_cli(archive, destination).read_bytes(), b"fixed-cli")
            self.assertEqual({path.name for path in root.iterdir()}, {"fixture.tar.gz", "codex"})
            destination.write_bytes(b"user-modified")
            with self.assertRaises(ValueError):
                extract_linux_cli(archive, destination)
            self.assertEqual(destination.read_bytes(), b"user-modified")

    def test_missing_duplicate_or_non_regular_target_is_rejected(self):
        cases = [[], [(LINUX_MEMBER, tarfile.REGTYPE, b"one"), (LINUX_MEMBER, tarfile.REGTYPE, b"two")],
                 [(LINUX_MEMBER, tarfile.SYMTYPE, b"")], [(LINUX_MEMBER, tarfile.LNKTYPE, b"")],
                 [(LINUX_MEMBER, tarfile.CHRTYPE, b"")], [(LINUX_MEMBER, tarfile.REGTYPE, b"")]]
        for entries in cases:
            with self.subTest(entries=entries), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = self.archive(root, entries)
                with self.assertRaises(ValueError):
                    extract_linux_cli(archive, root / "codex")
                self.assertEqual({path.name for path in root.iterdir()}, {"fixture.tar.gz"})


if __name__ == "__main__":
    unittest.main()
