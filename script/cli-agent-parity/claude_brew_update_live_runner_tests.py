"""私有 cask 验收的输入、执行卷和证据边界；不运行 CLI、Ruby 或包管理器。"""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("claude_brew_runner", Path(__file__).with_name("run_claude_brew_update_live.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ClaudeBrewRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="claude-brew-unit-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.contract = json.loads(runner.CONTRACT_PATH.read_bytes())

    def test_derived_metadata_is_explicit_and_fixed(self):
        raw = runner.derived_metadata(self.contract)
        value = json.loads(raw)
        self.assertEqual(value["token"], "claude-code@latest")
        self.assertEqual(value["version"], "2.1.280")
        self.assertEqual(self.contract["metadata"]["kind"], "derived_fixture")
        self.assertNotEqual(value["version"], "2.1.283")

    def test_changed_cask_fields_rejected(self):
        self.contract["metadata"]["value"]["url"] = "https://invalid.example/claude"
        with self.assertRaisesRegex(ValueError, "derived_metadata_digest"):
            runner.derived_metadata(self.contract)

    def test_stable_cask_cannot_claim_latest_release(self):
        self.contract["metadata"]["value"]["token"] = "claude-code"
        with self.assertRaisesRegex(ValueError, "derived_metadata_identity"):
            runner.derived_metadata(self.contract)

    def test_non_private_output_is_not_reused(self):
        path = self.root / "receipt.json"
        runner.write(path, b"original")
        with self.assertRaises(FileExistsError):
            runner.write(path, b"replacement")
        self.assertEqual(path.read_bytes(), b"original")

    def test_official_input_symlink_is_rejected(self):
        target = self.root / "native"
        target.write_bytes(b"official fixture")
        link = self.root / "alias"
        link.symlink_to(target)
        with self.assertRaisesRegex(ValueError, "regular_canonical_file_required"):
            runner.regular(link)

    def test_official_input_hardlink_is_rejected(self):
        target = self.root / "native"
        target.write_bytes(b"official fixture")
        os.link(target, self.root / "alias")
        with self.assertRaisesRegex(ValueError, "regular_canonical_file_required"):
            runner.regular(target)

    def test_official_input_changed_bytes_rejected(self):
        first = next(iter(self.contract["official_inputs"].values()))
        (self.root / first["file"]).write_bytes(b"untrusted replacement")
        with self.assertRaisesRegex(ValueError, "original_input_digest"):
            runner.official_inputs(self.root, self.contract, self.root)

    def test_clean_environment_excludes_inherited_credentials(self):
        with mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY":"synthetic-secret", "DYLD_INSERT_LIBRARIES":"untrusted"}):
            env = runner.environment(self.root, {"supervisor":{"path":"/fixed/supervisor"}}, "execute")
        self.assertNotIn("ANTHROPIC_API_KEY", env)
        self.assertNotIn("DYLD_INSERT_LIBRARIES", env)
        self.assertEqual(env["HOME"], str(self.root / "home"))
        self.assertEqual(env["INFINISHELL_CLAUDE_BREW_STEP"], "execute")

    def test_external_volume_requires_explicit_authorization(self):
        volumes = self.root / "Volumes"
        volume = volumes / "private"
        volume.mkdir(parents=True)
        with mock.patch.object(runner, "VOLUMES_ROOT", volumes):
            with self.assertRaisesRegex(ValueError, "external_execution_requires_explicit_volume"):
                runner.verify_execution_path(volume / "new-output", None, output=True)

    def test_external_volume_requires_actual_mount(self):
        volumes = self.root / "Volumes"
        volume = volumes / "private"
        volume.mkdir(parents=True)
        with mock.patch.object(runner, "VOLUMES_ROOT", volumes), mock.patch.object(runner.platform, "system", return_value="Darwin"), mock.patch.object(runner.os.path, "ismount", return_value=False):
            with self.assertRaisesRegex(ValueError, "external_volume_not_mounted_root"):
                runner.execution_volume(volume)

    def test_external_execution_rejects_other_volume(self):
        volumes = self.root / "Volumes"
        volume = volumes / "private"
        other = volumes / "other"
        volume.mkdir(parents=True)
        other.mkdir()
        with mock.patch.object(runner, "VOLUMES_ROOT", volumes):
            with self.assertRaisesRegex(ValueError, "execution_path_outside_allowed_volume"):
                runner.verify_execution_path(other / "new-output", {"path":str(volume), "st_dev":volume.stat().st_dev}, output=True)

    def test_external_execution_rejects_replaced_device(self):
        volumes = self.root / "Volumes"
        volume = volumes / "private"
        volume.mkdir(parents=True)
        with mock.patch.object(runner, "VOLUMES_ROOT", volumes), mock.patch.object(runner.os.path, "ismount", return_value=True):
            with self.assertRaisesRegex(ValueError, "execution_volume_changed"):
                runner.verify_execution_path(volume / "new-output", {"path":str(volume), "st_dev":-1}, output=True)

    def test_external_execution_rejects_nested_mount(self):
        volumes = self.root / "Volumes"
        volume = volumes / "private"
        nested = volume / "nested"
        nested.mkdir(parents=True)
        with mock.patch.object(runner, "VOLUMES_ROOT", volumes), mock.patch.object(runner.os.path, "ismount", side_effect=lambda p: Path(p) in (volume, nested)):
            with self.assertRaisesRegex(ValueError, "execution_volume_device_mismatch"):
                runner.verify_execution_path(nested / "new-output", {"path":str(volume), "st_dev":volume.stat().st_dev}, output=True)

    def test_runtime_failure_cannot_become_accepted_result(self):
        (self.root / "project").mkdir()
        manifest = {"supervisor":{"path":"/fixed/supervisor"}}
        result = mock.Mock(returncode=1, stdout=b"1 passed", stderr=b"failure")
        with mock.patch.object(runner.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(ValueError, "product_test_failed"):
                runner.run_test("/fixed/worker", self.root, manifest, "execute")
        self.assertEqual(json.loads((self.root / "execute.safe.json").read_bytes())["returncode"], 1)

    def test_official_target_digest_matches_cask_and_manifest_contract(self):
        expected = self.contract["official_inputs"][self.contract["metadata"]["value"]["url"]]
        self.assertEqual(expected["sha256"], "387a5c5dcdbb815085edf0baf79591f9d8894efe922bceaf3d75b1b08055229d")
        self.assertEqual(expected["length"], 217254576)


if __name__ == "__main__":
    unittest.main()
