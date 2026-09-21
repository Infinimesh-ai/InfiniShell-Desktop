#!/usr/bin/env python3
"""当前 Codex 生命周期探针的离线边界测试。"""

import os
from pathlib import Path
import tempfile
import unittest

import probe_codex_current_lifecycle as probe


class ProbeCodexCurrentLifecycleTests(unittest.TestCase):
    def test_server_version_only_comes_from_a_semantic_version_user_agent(self):
        self.assertEqual(
            probe.server_cli_version({"userAgent": "codex_app_server/0.155.1"}),
            "0.155.1",
        )
        self.assertIsNone(probe.server_cli_version({"userAgent": "codex_app_server/dev"}))
        self.assertIsNone(probe.server_cli_version({}))

    def test_exact_command_rejects_changed_arguments_or_working_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            expected = ["/usr/bin/python3", "fixture.py", "allow"]
            details = {
                "command": "/usr/bin/python3 fixture.py allow",
                "cwd": str(project),
            }
            self.assertTrue(probe.exact_command(details, expected, project))
            self.assertTrue(
                probe.exact_command(
                    {**details, "command": "/bin/sh -c '/usr/bin/python3 fixture.py allow'"},
                    expected,
                    project,
                )
            )
            self.assertFalse(
                probe.exact_command(
                    {**details, "command": "/usr/bin/python3 fixture.py deny"},
                    expected,
                    project,
                )
            )
            self.assertFalse(
                probe.exact_command(details, expected, project / "other")
            )

    def test_private_auth_copy_preserves_bytes_without_widening_permissions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "auth.json"
            destination = root / "copy" / "auth.json"
            destination.parent.mkdir(mode=0o700)
            source.write_bytes(b'{"temporary":"fixture"}')
            source.chmod(0o600)

            copied = probe.private_auth_copy(source, destination)

            self.assertEqual(copied, source.stat().st_size)
            self.assertEqual(destination.read_bytes(), source.read_bytes())
            self.assertEqual(os.stat(destination).st_mode & 0o777, 0o600)

    def test_private_auth_copy_rejects_symlinks_and_group_readable_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "auth.json"
            source.write_bytes(b"fixture")
            source.chmod(0o640)
            with self.assertRaises(RuntimeError):
                probe.private_auth_copy(source, root / "wide-copy")

            source.chmod(0o600)
            link = root / "auth-link.json"
            link.symlink_to(source)
            with self.assertRaises(RuntimeError):
                probe.private_auth_copy(link, root / "link-copy")


if __name__ == "__main__":
    unittest.main()
