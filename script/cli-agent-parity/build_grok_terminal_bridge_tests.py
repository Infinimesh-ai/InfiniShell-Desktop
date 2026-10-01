"""验证新旧原生能力的源码与工件合同不会混用；不执行 Cargo 或原生程序。"""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_grok_terminal_bridge as build


class SourceProfileTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.repo = Path(self.root.name)
        self.source = self.repo / "native/grok-build"
        self.source.mkdir(parents=True)
        self.override = patch.object(build, "REPOSITORY", self.repo)
        self.override.start()
        self.addCleanup(self.override.stop)

    def fixture(self, capability):
        name, patch_name, version = build.SOURCE_PROFILES[capability]
        payload = ("公开源码补丁夹具:" + capability).encode()
        (self.source / patch_name).write_bytes(payload)
        metadata = {
            "upstream": build.UPSTREAM,
            "base_commit": build.BASE,
            "cross_platform_build": {
                "patch_sha256": hashlib.sha256(payload).hexdigest(),
                "native_tree": "a" * 40,
                "custom_version": version,
            },
        }
        path = self.source / name
        path.write_text(json.dumps(metadata), encoding="utf-8")
        return path, metadata

    def test_profiles_require_distinct_patch_metadata_and_version(self):
        for capability in build.SOURCE_PROFILES:
            self.fixture(capability)
        old = build.load_source("terminal-bridge")
        new = build.load_source("session-notifications")
        self.assertNotEqual(old[2], new[2])
        self.assertNotEqual(old[3], new[3])
        self.assertNotEqual(old[1]["custom_version"], new[1]["custom_version"])

    def test_old_version_cannot_authorize_notification_patch(self):
        path, metadata = self.fixture("session-notifications")
        metadata["cross_platform_build"]["custom_version"] = build.SOURCE_PROFILES["terminal-bridge"][2]
        path.write_text(json.dumps(metadata), encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "版本不匹配"):
            build.load_source("session-notifications")

    def test_patch_tampering_and_wrong_upstream_are_rejected(self):
        path, metadata = self.fixture("session-notifications")
        (self.source / "session-notifications.patch").write_bytes(b"changed")
        with self.assertRaisesRegex(RuntimeError, "补丁摘要"):
            build.load_source("session-notifications")
        metadata["upstream"] = "https://example.invalid/source"
        path.write_text(json.dumps(metadata), encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "公开源码基线"):
            build.load_source("session-notifications")

    def test_unknown_capability_cannot_select_an_arbitrary_path(self):
        with self.assertRaises(KeyError):
            build.load_source("../../source.json")


if __name__ == "__main__":
    unittest.main()
