#!/usr/bin/env python3
"""只验证 runner 的输入和证据边界，不执行 CLI、网络或升级。"""
import base64
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("npm_live", Path(__file__).with_name("run_grok_npm_update_live.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def archive(entries):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w:gz", format=tarfile.USTAR_FORMAT) as output:
        for name, content, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.size = len(content) if kind == tarfile.REGTYPE else 0
            member.linkname = "../../outside" if kind != tarfile.REGTYPE else ""
            output.addfile(member, io.BytesIO(content) if kind == tarfile.REGTYPE else None)
    data = raw.getvalue()
    return data, "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode()


class BoundaryTests(unittest.TestCase):
    def mounted_volume(self, root):
        volumes = root / "Volumes"
        volume = volumes / "SanDisk"
        volume.mkdir(parents=True)
        self.enterContext(patch.object(runner, "VOLUMES_ROOT", volumes))
        self.enterContext(patch.object(runner.platform, "system", return_value="Darwin"))
        self.enterContext(patch.object(runner.os.path, "ismount", side_effect=lambda path: Path(path) == volume))
        return volume

    def test_external_output_and_binary_remain_rejected_by_default(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            binary = volume / "worker"
            binary.write_bytes(b"worker")
            for path, output in ((volume / "new" / "case", True), (binary, False)):
                with self.subTest(output=output), self.assertRaisesRegex(ValueError, "requires?_internal_disk"):
                    runner.verify_execution_path(path, None, output=output)
            self.assertFalse((volume / "new").exists())

    def test_explicit_volume_allows_its_output_and_binaries_and_internal_node(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            worker, node = volume / "worker", root / "node"
            worker.write_bytes(b"worker")
            node.write_bytes(b"node")
            allowed = runner.execution_volume(volume)
            self.assertEqual(allowed, {"path":str(volume), "st_dev":volume.stat().st_dev})
            for path, output in ((volume / "new" / "case", True), (worker, False), (node, False)):
                with self.subTest(path=path):
                    self.assertEqual(runner.verify_execution_path(path, allowed, output=output), path)

    def test_explicit_volume_must_be_a_mounted_volume_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            child, unmounted = volume / "directory", volume.parent / "Unmounted"
            child.mkdir()
            unmounted.mkdir()
            for path in (root, volume.parent, child, unmounted):
                with self.subTest(path=path), self.assertRaisesRegex(ValueError, "not_mounted_root"):
                    runner.execution_volume(path)

    def test_explicit_volume_is_rejected_on_non_macos(self):
        for system in ("Linux", "Windows"):
            with self.subTest(system=system), patch.object(runner.platform, "system", return_value=system):
                with self.assertRaisesRegex(ValueError, "requires_macos"):
                    runner.execution_volume(Path("/Volumes/SanDisk"))

    def test_missing_volumes_container_keeps_default_internal_paths_available(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary = root / "worker"
            binary.write_bytes(b"worker")
            with patch.object(runner, "VOLUMES_ROOT", root / "absent-volumes"):
                self.assertEqual(runner.verify_execution_path(binary, None), binary)
                self.assertEqual(runner.verify_execution_path(root / "new", None, output=True), root / "new")

    def test_volume_container_aliases_do_not_bypass_disk_policy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            worker = volume / "worker"
            worker.write_bytes(b"worker")
            allowed = runner.execution_volume(volume)
            aliases = (root / "volumes", root / "System" / "Volumes" / "Data" / "Volumes")
            for container in aliases:
                for name in ("SanDisk", "Other"):
                    path = container / name / "worker"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"worker")
            original = Path.samefile
            def samefile(path, other):
                if path in aliases and Path(other) == volume.parent:
                    return True
                return original(path, other)
            with patch.object(Path, "samefile", samefile):
                for container in aliases:
                    with self.subTest(container=container):
                        self.assertEqual(runner.execution_volume(container / "SanDisk"), allowed)
                        for path, output in ((container / "SanDisk" / "new", True),
                                             (container / "SanDisk" / "worker", False)):
                            with self.assertRaisesRegex(ValueError, "requires?_internal_disk"):
                                runner.verify_execution_path(path, None, output=output)
                            self.assertEqual(runner.verify_execution_path(path, allowed, output=output),
                                             volume / path.name)
                        with self.assertRaisesRegex(ValueError, "outside_allowed_volume"):
                            runner.verify_execution_path(container / "Other" / "worker", allowed)

    def test_other_volume_and_prefix_lookalike_do_not_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            allowed = runner.execution_volume(volume)
            for name in ("Other", "SanDisk-copy"):
                other = volume.parent / name
                other.mkdir()
                binary = other / "worker"
                binary.write_bytes(b"worker")
                for path, output in ((other / "new", True), (binary, False)):
                    with self.subTest(path=path), self.assertRaisesRegex(ValueError, "outside_allowed_volume"):
                        runner.verify_execution_path(path, allowed, output=output)
            with self.assertRaisesRegex(ValueError, "outside_allowed_volume"):
                runner.verify_execution_path(root / "internal-output", allowed, output=True)

    def test_resolved_aliases_cannot_escape_the_allowed_volume(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            other = volume.parent / "Other"
            other.mkdir()
            worker = other / "worker"
            worker.write_bytes(b"worker")
            allowed = runner.execution_volume(volume)
            aliases = {volume / "escape" / "new":other / "new",
                       volume / "escape" / "worker":worker,
                       root / "volume-alias":volume,
                       root / "output-alias":volume / "new"}
            original = Path.resolve
            def resolve(path, strict=False):
                return aliases[path] if path in aliases else original(path, strict=strict)
            # 用解析结果模拟符号链接，不要求 Windows 测试账号有创建链接权限。
            with patch.object(Path, "resolve", resolve):
                self.assertEqual(runner.execution_volume(root / "volume-alias"), allowed)
                self.assertEqual(runner.verify_execution_path(root / "output-alias", allowed, output=True),
                                 volume / "new")
                with self.assertRaisesRegex(ValueError, "requires_internal_disk"):
                    runner.verify_execution_path(root / "output-alias", None, output=True)
                for path, output in ((volume / "escape" / "new", True), (volume / "escape" / "worker", False)):
                    with self.subTest(path=path), self.assertRaisesRegex(ValueError, "outside_allowed_volume"):
                        runner.verify_execution_path(path, allowed, output=output)

    def test_nested_mount_device_is_rejected_for_output_and_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            nested = volume / "nested-mount"
            returned = nested / "original-device"
            returned.mkdir(parents=True)
            binary = returned / "worker"
            binary.write_bytes(b"worker")
            allowed = runner.execution_volume(volume)
            original = Path.stat
            def stat(path, *args, **kwargs):
                value = original(path, *args, **kwargs)
                if path == nested:
                    fields = list(value)
                    fields[2] = allowed["st_dev"] + 1
                    return runner.os.stat_result(fields)
                return value
            with patch.object(Path, "stat", stat):
                for path, output in ((returned / "new" / "case", True), (binary, False)):
                    with self.subTest(output=output), self.assertRaisesRegex(ValueError, "device_mismatch"):
                        runner.verify_execution_path(path, allowed, output=output)

    def test_nested_mount_is_rejected_even_with_the_same_device(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            nested = volume / "nested-mount"
            nested.mkdir()
            allowed = runner.execution_volume(volume)
            with patch.object(runner.os.path, "ismount", side_effect=lambda path: Path(path) in (volume, nested)):
                with self.assertRaisesRegex(ValueError, "nested_mount"):
                    runner.verify_execution_path(nested / "new" / "case", allowed, output=True)

    def test_unmounted_volume_is_rejected_after_initial_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            allowed = runner.execution_volume(volume)
            with patch.object(runner.os.path, "ismount", return_value=False):
                with self.assertRaisesRegex(ValueError, "execution_volume_changed"):
                    runner.verify_execution_path(volume / "new", allowed, output=True)

    def test_main_rejects_external_output_before_creation_binding_or_download(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            output = volume / "new" / "case"
            arguments = ["runner", "--repo", str(root), "--output", str(output)]
            for role in ("test-binary", "supervisor", "node", "npm-cli"):
                arguments += ["--" + role, str(root / role), "--" + role + "-sha256", "unused"]
            with patch.object(sys, "argv", arguments), patch.object(runner.platform, "machine", return_value="arm64"), \
                    patch.object(runner, "binding") as bind, patch.object(runner, "download_inputs") as download:
                with self.assertRaisesRegex(ValueError, "runtime_requires_internal_disk"):
                    runner.main()
            bind.assert_not_called()
            download.assert_not_called()
            self.assertFalse((volume / "new").exists())

    def test_main_rejects_other_volume_binary_before_output_creation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            other = volume.parent / "Other"
            other.mkdir()
            binary = other / "worker"
            binary.write_bytes(b"bound input")
            output = volume / "new" / "case"
            arguments = ["runner", "--repo", str(root), "--output", str(output), "--allow-external-volume", str(volume)]
            for role in ("test-binary", "supervisor", "node", "npm-cli"):
                arguments += ["--" + role, str(binary), "--" + role + "-sha256", runner.sha(binary)]
            with patch.object(sys, "argv", arguments), patch.object(runner.platform, "machine", return_value="arm64"), \
                    patch.object(runner, "download_inputs") as download:
                with self.assertRaisesRegex(ValueError, "outside_allowed_volume"):
                    runner.main()
            download.assert_not_called()
            self.assertFalse((volume / "new").exists())

    def main_fixture(self, root):
        volume = self.mounted_volume(root)
        output = volume / "new" / "case"
        paths = {"test-binary":volume / "worker", "supervisor":volume / "supervisor",
                 "node":root / "node", "npm-cli":root / "npm" / "bin" / "npm-cli.js"}
        for path in paths.values():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"bound input")
        (root / "npm" / "package.json").write_text(json.dumps({"name":"npm", "bin":{"npm":"bin/npm-cli.js"}}))
        contract = root / "script/cli-agent-parity/grok_1041_npm_manifest.json"
        contract.parent.mkdir(parents=True)
        contract.write_text("{}")
        arguments = ["runner", "--repo", str(root), "--output", str(output), "--official-inputs", str(root),
                     "--allow-external-volume", str(volume), "--case", "updated"]
        for role, path in paths.items():
            arguments += ["--" + role, str(path), "--" + role + "-sha256", "source-digest"]
        self.enterContext(patch.object(sys, "argv", arguments))
        self.enterContext(patch.object(runner.platform, "machine", return_value="arm64"))
        self.enterContext(patch.object(runner, "sha", return_value="source-digest"))
        self.enterContext(patch.object(runner, "verify_embedded"))
        self.enterContext(patch.object(runner.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0, b"", b""),
            subprocess.CompletedProcess([], 0, "a" * 40, ""),
            subprocess.CompletedProcess([], 0, b"", b"")]))
        space = self.enterContext(patch.object(runner.shutil, "disk_usage"))
        space.return_value.free = 4 * 1024**3
        self.enterContext(patch.object(runner, "official_inputs", return_value=({}, {runner.OLD:{}})))
        self.enterContext(patch.object(runner, "prepare_old", return_value={}))
        self.enterContext(patch.object(runner, "run_test"))
        self.enterContext(patch.object(runner, "verify_product_archives"))
        def fixture(*arguments):
            case = arguments[0].output / "fixture"
            case.mkdir()
            (case / "result-execute.safe.json").write_text('{"accepted":true}')
            return case, {"supervisor":{"path":str(paths["supervisor"])}}
        self.enterContext(patch.object(runner, "fixture", side_effect=fixture))
        self.enterContext(patch.object(runner, "download_inputs", side_effect=AssertionError("unexpected_download")))
        return volume, output

    def test_source_and_success_receipts_record_explicit_execution_volume(self):
        with tempfile.TemporaryDirectory() as temporary:
            volume, output = self.main_fixture(Path(temporary).resolve())
            runner.main()
            expected = {"mode":"explicit_external_volume",
                        "external_volume":{"path":str(volume), "st_dev":volume.stat().st_dev}}
            for name in ("source.safe.json", "summary.safe.json"):
                with self.subTest(receipt=name):
                    self.assertEqual(json.loads((output / name).read_bytes())["execution_policy"], expected)
            self.assertFalse(json.loads((output / "summary.safe.json").read_bytes())["g09_closed"])

    def test_main_rechecks_volume_after_output_creation(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = self.main_fixture(Path(temporary).resolve())[1]
            original = Path.mkdir
            def mkdir(path, *args, **kwargs):
                original(path, *args, **kwargs)
                if path == output:
                    self.enterContext(patch.object(runner.os.path, "ismount", return_value=False))
            with patch.object(Path, "mkdir", mkdir), self.assertRaisesRegex(ValueError, "execution_volume_changed"):
                runner.main()
            runner.official_inputs.assert_not_called()
            self.assertFalse((output / "source.safe.json").exists())
            self.assertFalse((output / "summary.safe.json").exists())

    def test_unmount_during_case_does_not_produce_success_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = self.main_fixture(Path(temporary).resolve())[1]
            def unmount(*args):
                self.enterContext(patch.object(runner.os.path, "ismount", return_value=False))
            runner.verify_product_archives.side_effect = unmount
            with self.assertRaisesRegex(ValueError, "execution_volume_changed"):
                runner.main()
            self.assertTrue((output / "source.safe.json").is_file())
            self.assertTrue((output / "fixture/result-execute.safe.json").is_file())
            self.assertFalse((output / "summary.safe.json").exists())

    def test_changed_volume_device_is_rejected_after_initial_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            volume = self.mounted_volume(Path(temporary).resolve())
            allowed = runner.execution_volume(volume)
            original = Path.stat
            def stat(path, *args, **kwargs):
                value = original(path, *args, **kwargs)
                if path == volume:
                    fields = list(value)
                    fields[2] = allowed["st_dev"] + 1
                    return runner.os.stat_result(fields)
                return value
            with patch.object(Path, "stat", stat), self.assertRaisesRegex(ValueError, "execution_volume_changed"):
                runner.verify_execution_path(volume / "new", allowed, output=True)

    def test_valid_archive_preserves_exact_bytes(self):
        raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/bin/grok", b"official", tarfile.REGTYPE)])
        self.assertEqual(runner.archive_members(raw, sri)["bin/grok"][0], b"official")

    def test_corrupt_sri_is_rejected(self):
        raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            runner.archive_members(raw + b"x", sri)

    def test_non_sha512_integrity_is_rejected(self):
        with self.assertRaises(ValueError):
            runner.archive_members(b"x", "sha256-AA==")

    def test_paths_and_duplicates_are_rejected(self):
        for path in ("package/../outside", "/package/absolute", "package/bin\\escape", "package/C:alias", "package/./alias", "package//alias"):
            with self.subTest(path=path):
                raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), (path, b"x", tarfile.REGTYPE)])
                with self.assertRaises(ValueError):
                    runner.archive_members(raw, sri)
        raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/package.json", b"changed", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            runner.archive_members(raw, sri)

    def test_links_and_special_entries_are_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.CHRTYPE, tarfile.FIFOTYPE):
            raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/member", b"", kind)])
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                runner.archive_members(raw, sri)

    def test_environment_has_only_private_configuration(self):
        env = runner.environment(Path("/private/case"), {"supervisor":{"path":"/internal/supervisor"}}, "execute")
        self.assertEqual(env["NPM_CONFIG_USERCONFIG"], str(Path("/private/case") / "npm/user.npmrc"))
        self.assertEqual(env["INFINISHELL_GROK_NPM_UPDATE_STEP"], "execute")
        self.assertNotIn("ANTHROPIC_API_KEY", env)
        self.assertNotIn("NODE_OPTIONS", env)
        self.assertNotIn("HTTP_PROXY", env)
        self.assertNotIn("DYLD_INSERT_LIBRARIES", env)

    def test_writes_do_not_overwrite_prior_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "failure.safe.json"
            runner.write(path, b"original failure")
            with self.assertRaises(FileExistsError):
                runner.write(path, b"replacement")
            self.assertEqual(path.read_bytes(), b"original failure")

    def test_binary_binding_rejects_changed_input(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "binary"
            path.write_bytes(b"original")
            expected = runner.sha(path)
            path.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                runner.binding(path, expected)

    def test_empty_test_filter_is_not_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = subprocess.CompletedProcess([], 0, b"0 passed", b"")
            with patch.object(runner.subprocess, "run", return_value=result), self.assertRaises(ValueError):
                runner.run_test("binary", "exact", root, {}, root / "attempt")
            self.assertEqual((root / "attempt.stdout").read_bytes(), b"0 passed")

    def test_failed_product_test_preserves_raw_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = subprocess.CompletedProcess([], 101, b"raw stdout", b"raw stderr")
            with patch.object(runner.subprocess, "run", return_value=result), self.assertRaises(ValueError):
                runner.run_test("binary", "exact", root, {}, root / "attempt")
            self.assertEqual((root / "attempt.stderr").read_bytes(), b"raw stderr")
            self.assertEqual(json.loads((root / "attempt.safe.json").read_text())["returncode"], 101)

    def test_timeout_is_not_cleanup_confirmation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            error = subprocess.TimeoutExpired([], 900, output=b"partial", stderr=b"last error")
            with patch.object(runner.subprocess, "run", side_effect=error), self.assertRaises(subprocess.TimeoutExpired):
                runner.run_test("binary", "exact", root, {}, root / "attempt")
            evidence = json.loads((root / "attempt.timeout.safe.json").read_text())
            self.assertTrue(evidence["timeout"])
            self.assertNotIn("cleanup_confirmed", evidence)

    def releases_and_receipt(self, root, case="updated"):
        releases = {}
        for version in (runner.OLD, runner.TARGET):
            native = ("native-" + version).encode()
            releases[version] = {"members":{"package.json":(b"{}", False), "bin/grok":(b"wrapper", True), "bin/postinstall.js":(b"not executed", False), "node_modules/@iarna/toml/toml.js":(b"toml", False)},
                "archives":[hashlib.sha256((version + str(index)).encode()).hexdigest() for index in range(3)],
                "native":{"length":len(native), "sha256":hashlib.sha256(native).hexdigest()}}
        prepared = runner.installed_contract(releases[runner.TARGET])
        journal = {"archives":[list(bytes.fromhex(value)) for value in releases[runner.TARGET]["archives"]],
            "prepared":{"link":{},"tree":{"nodes":{name:{"length":spec["length"],"sha256":list(bytes.fromhex(spec["sha256"]))} for name,spec in prepared.items()}}}}
        (root / "prepared-journal.safe.json").write_text(json.dumps(journal))
        version = runner.OLD if case in ("swap_receipt_missing", "candidate_changed_preserved") else runner.TARGET
        package = root / "prefix/lib/node_modules/@xai-official/grok"
        runner.write_members(package, {name:value for name,value in releases[version]["members"].items() if name != "bin/grok"})
        (package / "bin/grok-native").write_bytes(("native-" + version).encode())
        (package / "bin/grok").symlink_to("./grok-native")
        mirror = root / "home/.grok/bin"
        mirror.mkdir(parents=True)
        for v in {version, runner.OLD}:
            (mirror / ("grok-" + v)).write_bytes(("native-" + v).encode())
        (mirror / "grok").symlink_to("grok-" + version)
        (mirror / "unrelated-history").write_bytes(b"preserve this unrelated history\n")
        (root / "home/.grok/config.toml").write_bytes(b'[cli]\ninstaller = "npm"\nchannel = "stable"\n')
        if case == "external_change_preserved":
            (package / "external-npm-change").write_bytes(b"later external install must survive\n")
        return releases, package, journal

    @unittest.skipUnless(os.name == "posix", "真实 Unix symlink 夹具仅适用于目标平台")
    def test_independent_receipts_cover_all_four_cases(self):
        for case in runner.CASES:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                releases, _, _ = self.releases_and_receipt(root, case)
                runner.verify_product_archives(root, case, releases)
                receipt = json.loads((root / "independent-archive-check.safe.json").read_bytes())
                self.assertTrue(receipt["user_bin_checked"])

    @unittest.skipUnless(os.name == "posix", "真实 Unix symlink 夹具仅适用于目标平台")
    def test_prepared_digest_and_archive_receipts_cannot_be_forged(self):
        for mutation in ("archive", "digest", "missing-toml", "link"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                releases, _, journal = self.releases_and_receipt(root)
                if mutation == "archive":
                    journal["archives"][0] = [0] * 32
                elif mutation == "digest":
                    journal["prepared"]["tree"]["nodes"]["bin/grok-native"]["sha256"] = [0] * 32
                elif mutation == "missing-toml":
                    del journal["prepared"]["tree"]["nodes"]["node_modules/@iarna/toml/toml.js"]
                else:
                    journal["prepared"]["link"] = None
                (root / "prepared-journal.safe.json").write_text(json.dumps(journal))
                with self.assertRaises(ValueError):
                    runner.verify_product_archives(root, "updated", releases)

    @unittest.skipUnless(os.name == "posix", "真实 Unix symlink 夹具仅适用于目标平台")
    def test_mirror_and_history_are_independently_verified(self):
        for member in ("grok-1.0.41", "grok-1.0.40", "unrelated-history"):
            with self.subTest(member=member), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                releases, _, _ = self.releases_and_receipt(root)
                (root / "home/.grok/bin" / member).write_bytes(b"changed externally")
                with self.assertRaises(ValueError):
                    runner.verify_product_archives(root, "updated", releases)

    @unittest.skipUnless(os.name == "posix", "真实 Unix symlink 夹具仅适用于目标平台")
    def test_external_exception_does_not_allow_any_other_extra_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            releases, package, _ = self.releases_and_receipt(root, "external_change_preserved")
            (package / "second-change").write_bytes(b"unexpected")
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "external_change_preserved", releases)

    @unittest.skipUnless(os.name == "posix", "真实 Unix symlink 夹具仅适用于目标平台")
    def test_public_link_cannot_point_outside_bound_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            releases, package, _ = self.releases_and_receipt(root)
            (package / "bin/grok").unlink()
            (package / "bin/grok").symlink_to("../../outside")
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "updated", releases)

    @unittest.skipUnless(os.name == "posix", "真实 Unix symlink 夹具仅适用于目标平台")
    def test_rollback_does_not_leave_new_user_bin_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            releases, _, _ = self.releases_and_receipt(root, "swap_receipt_missing")
            (root / "home/.grok/bin/grok-1.0.41").write_bytes(b"unexpected")
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "swap_receipt_missing", releases)

    def test_clone_never_uses_hardlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "source").write_bytes(b"original")
            runner.clone_file(root / "source", root / "copy")
            self.assertNotEqual((root / "source").stat().st_ino, (root / "copy").stat().st_ino)
            (root / "copy").write_bytes(b"mutated")
            self.assertEqual((root / "source").read_bytes(), b"original")

    def test_old_native_brotli_mismatch_is_rejected_without_cli_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            node = root / "node"
            node.write_bytes(b"bound node")
            members = {"bin/grok":(b"wrapper", True), "node_modules/@xai-official/grok-darwin-arm64/bin/grok.br":(b"compressed", False)}
            def decompress(command, **kwargs):
                self.assertEqual(command[1], "-e")
                self.assertNotIn("NODE_OPTIONS", kwargs["env"])
                Path(command[4]).write_bytes(b"wrong native")
                return subprocess.CompletedProcess(command, 0, b"", b"")
            release = {"members":members, "native":{"length":5,"sha256":hashlib.sha256(b"right").hexdigest()}}
            with patch.object(runner.subprocess, "run", side_effect=decompress), self.assertRaises(ValueError):
                runner.prepare_old(root, release, "darwin-arm64", runner.binding(node))

    def test_intel_and_uncontracted_platforms_are_not_enabled(self):
        for system,machine in (("Darwin","x86_64"),("Linux","aarch64"),("Windows","AMD64")):
            with self.subTest(system=system), patch.object(runner.platform, "system", return_value=system), patch.object(runner.platform, "machine", return_value=machine), self.assertRaises(ValueError):
                runner.target_platform()

    def fixed_input_fixture(self, target):
        contract = {"packages":{}, "native":{version:{target:{"length":1,"sha256":"00"*32}} for version in (runner.OLD,runner.TARGET)}}
        payloads = {}
        for version in (runner.OLD, runner.TARGET):
            for package,_,_ in runner.package_coordinates(target, version):
                package_version = "3.0.0" if package == "@iarna/toml" else version
                metadata = {"name":package,"version":package_version}
                embedded = json.dumps(metadata).encode()
                raw,sri = archive([("package/package.json",embedded,tarfile.REGTYPE)])
                contract["packages"][package+"@"+package_version] = {"integrity":sri,"manifest":metadata.copy(),"files":{"package.json":{"length":len(embedded),"sha256":hashlib.sha256(embedded).hexdigest(),"executable":False}}}
                archive_url = "https://registry.npmjs.org/"+package+"/-/"+package.rsplit("/",1)[1]+"-"+package_version+".tgz"
                metadata["dist"] = {"integrity":sri,"tarball":archive_url}
                payloads["https://registry.npmjs.org/"+package+"/"+package_version] = json.dumps(metadata).encode()
                payloads[archive_url] = raw
        return contract,payloads

    def test_official_download_uses_only_fixed_urls_and_preserves_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            contract,payloads = self.fixed_input_fixture("linux-x64")
            seen = []
            def download(url,path,limit):
                self.assertIn(url,payloads)
                self.assertLessEqual(len(payloads[url]),limit)
                seen.append(url)
                runner.write(path,payloads[url])
            with patch.object(runner, "download_exact", side_effect=download):
                cache = runner.download_inputs(root / "inputs",contract,"linux-x64")
            self.assertEqual(set(seen),set(payloads))
            self.assertEqual(len(seen),10)
            before = {path.name:runner.sha(path) for path in cache.iterdir()}
            inputs,_ = runner.official_inputs(cache,contract,"linux-x64")
            self.assertEqual({path.name:runner.sha(path) for path in cache.iterdir()},before)
            self.assertEqual(len(inputs),10)
            self.assertTrue(all(Path(record["path"]).parent == cache.resolve() for record in inputs.values()))

    def test_download_never_follows_metadata_to_unpinned_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            contract,payloads = self.fixed_input_fixture("linux-x64")
            seen = []
            def download(url,path,limit):
                seen.append(url)
                metadata=json.loads(payloads[url])
                metadata["dist"]["tarball"]="https://untrusted.invalid/other.tgz"
                runner.write(path,json.dumps(metadata).encode())
            with patch.object(runner, "download_exact", side_effect=download),self.assertRaises(ValueError):
                runner.download_inputs(root / "inputs",contract,"linux-x64")
            self.assertEqual(len(seen),1)

    def test_existing_archive_digest_change_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            contract,payloads = self.fixed_input_fixture("linux-x64")
            with patch.object(runner, "download_exact", side_effect=lambda url,path,limit:runner.write(path,payloads[url])):
                cache=runner.download_inputs(root / "inputs",contract,"linux-x64")
            with (cache / "wrapper.tgz").open("ab") as output:
                output.write(b"modified")
            with self.assertRaises(ValueError):
                runner.official_inputs(cache,contract,"linux-x64")

    def test_final_binding_check_rejects_midrun_binary_or_source_change(self):
        for changed in ("binary","source"):
            with self.subTest(changed=changed),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary)
                for name in ("binary","source"):
                    (root/name).write_bytes(b"original")
                binaries={"worker":runner.binding(root/"binary")}
                sources={"source":runner.sha(root/"source")}
                runner.verify_run_bindings(root,binaries,sources)
                (root/changed).write_bytes(b"changed")
                with self.assertRaises(ValueError):
                    runner.verify_run_bindings(root,binaries,sources)


if __name__ == "__main__":
    unittest.main()
