#!/usr/bin/env python3

import hashlib
import json
from pathlib import Path
import stat
import tempfile
import unittest
import uuid

import probe_supervisor_startup as probe


class LaunchRecordTests(unittest.TestCase):
    def test_launch_records_bind_attempt_to_exact_private_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            generation = uuid.uuid4()
            manifest = {
                "version": 1,
                "launch_allowed": True,
                "generation": str(generation),
                "cwd": "/tmp/验收",
            }

            path = probe.write_launch_records(state, manifest, generation)

            manifest_bytes = path.read_bytes()
            attempt = json.loads((state / "spawn-attempt.json").read_bytes())
            self.assertEqual(json.loads(manifest_bytes), manifest)
            self.assertNotIn(b" ", manifest_bytes)
            self.assertEqual(attempt["version"], 1)
            self.assertEqual(attempt["generation"], str(generation))
            self.assertEqual(
                attempt["manifest_sha256"], hashlib.sha256(manifest_bytes).hexdigest()
            )
            uuid.UUID(attempt["attempt"])
            for record in (path, state / "spawn-attempt.json"):
                self.assertEqual(stat.S_IMODE(record.stat().st_mode), 0o600)


if __name__ == "__main__":
    unittest.main()
