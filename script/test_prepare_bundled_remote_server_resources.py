#!/usr/bin/env python3

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import prepare_bundled_remote_server_resources as resources


class PrepareBundledRemoteServerResourcesTest(unittest.TestCase):
    def create_artifacts(self, root: Path) -> None:
        for os_name, arch_name, file_name in resources.ARTIFACTS:
            (root / file_name).write_bytes(f"{os_name}-{arch_name}".encode())

    def test_artifact_matrix_contains_windows_zip_archives(self) -> None:
        self.assertIn(
            ("windows", "x86_64", "infinishell-windows-x86_64.zip"),
            resources.ARTIFACTS,
        )
        self.assertIn(
            ("windows", "aarch64", "infinishell-windows-aarch64.zip"),
            resources.ARTIFACTS,
        )

    def test_create_verify_and_copy_all_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source = root / "source"
            destination = root / "destination"
            copied = root / "copied"
            source.mkdir()
            self.create_artifacts(source)

            resources.create(source, destination, "v-test")
            resources.verify(destination, "v-test")
            resources.copy(destination, copied, "v-test")

            manifest = json.loads((copied / "manifest.json").read_text())
            self.assertEqual("v-test", manifest["version"])
            self.assertEqual(6, len(manifest["artifacts"]))
            self.assertEqual(
                {artifact["file"] for artifact in manifest["artifacts"]},
                {file_name for _, _, file_name in resources.ARTIFACTS},
            )

    def test_create_rejects_missing_windows_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            source = Path(temporary_directory)
            self.create_artifacts(source)
            (source / "infinishell-windows-aarch64.zip").unlink()

            with self.assertRaisesRegex(ValueError, "infinishell-windows-aarch64.zip"):
                resources.create(source, source / "destination", "v-test")

    def test_partial_bundle_accepts_one_explicit_platform(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source = root / "source"
            destination = root / "destination"
            copied = root / "copied"
            source.mkdir()
            archive = source / "infinishell-windows-x86_64.zip"
            archive.write_bytes(b"windows-x86_64")

            resources.create(source, destination, "unversioned", allow_partial=True)
            resources.verify(destination, "unversioned", allow_partial=True)
            resources.copy(
                destination,
                copied,
                "unversioned",
                allow_partial=True,
            )

            manifest = json.loads((copied / "manifest.json").read_text())
            self.assertEqual(
                ["infinishell-windows-x86_64.zip"],
                [artifact["file"] for artifact in manifest["artifacts"]],
            )
            with self.assertRaisesRegex(ValueError, "缺少 remote-server 产物"):
                resources.verify(destination, "unversioned")


if __name__ == "__main__":
    unittest.main()
