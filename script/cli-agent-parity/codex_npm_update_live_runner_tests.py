#!/usr/bin/env python3
"""只验证 runner 的输入和证据边界，不执行 CLI、网络或升级。"""
import base64
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("codex_npm_live", Path(__file__).with_name("run_codex_npm_update_live.py"))
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
                    patch.object(runner, "binding") as bind, patch.object(runner, "get") as download:
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
                    patch.object(runner, "get") as download:
                with self.assertRaisesRegex(ValueError, "outside_allowed_volume"):
                    runner.main()
            download.assert_not_called()
            self.assertFalse((volume / "new").exists())

    def test_source_receipt_records_explicit_execution_volume(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            volume = self.mounted_volume(root)
            output = volume / "new" / "case"
            paths = {"test-binary":volume / "worker", "supervisor":volume / "supervisor",
                     "node":root / "node", "npm-cli":root / "npm" / "bin" / "npm-cli.js"}
            for path in paths.values():
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"bound input")
            (root / "npm" / "package.json").write_text(json.dumps({"name":"npm", "bin":{"npm":"bin/npm-cli.js"}}))
            arguments = ["runner", "--repo", str(root), "--output", str(output), "--allow-external-volume", str(volume)]
            for role, path in paths.items():
                arguments += ["--" + role, str(path), "--" + role + "-sha256", runner.sha(path)]
            with patch.object(sys, "argv", arguments), patch.object(runner.platform, "machine", return_value="arm64"), \
                    patch.object(runner, "sha", return_value="source-digest"), patch.object(runner, "verify_embedded"), \
                    patch.object(runner.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"", b"")), \
                    patch.object(runner.subprocess, "check_output", side_effect=["a" * 40, ""]), \
                    patch.object(runner.shutil, "disk_usage") as space, \
                    patch.object(runner, "prepare_old", side_effect=RuntimeError("stop_before_fixture")), \
                    patch.object(runner, "get") as download:
                space.return_value.free = 4 * 1024**3
                with self.assertRaisesRegex(RuntimeError, "stop_before_fixture"):
                    runner.main()
            download.assert_not_called()
            receipt = json.loads((output / "source.safe.json").read_bytes())
            self.assertEqual(receipt["execution_policy"], {"mode":"explicit_external_volume",
                "external_volume":{"path":str(volume), "st_dev":volume.stat().st_dev}})

    def test_valid_archive_preserves_exact_bytes(self):
        raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/bin/codex.js", b"official", tarfile.REGTYPE)])
        self.assertEqual(runner.archive_members(raw, sri)["bin/codex.js"][0], b"official")

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
        root = Path("/private/case")
        env = runner.environment(root, {"supervisor":{"path":"/internal/supervisor"}}, "execute")
        self.assertEqual(env["NPM_CONFIG_USERCONFIG"], str(root / "npm/user.npmrc"))
        self.assertEqual(env["INFINISHELL_CLI_CODEX_NPM_UPDATE_STEP"], "execute")
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

    def product_receipt(self, root):
        # 这里只验证归档字节与证据映射；离线夹具不要求宿主能够运行对应平台二进制。
        target, triple = "darwin-arm64", "aarch64-apple-darwin"
        self.enterContext(patch.object(runner, "target_platform", return_value=(target, triple)))
        wrapper_json = {"name":runner.PACKAGE, "version":runner.TARGET,
                        "bin":{"codex":"bin/codex.js"}}
        native_json = {"name":runner.PACKAGE, "version":runner.TARGET + "-" + target}
        wrapper_files = {"package.json":json.dumps(wrapper_json).encode(),
                         "bin/codex.js":b"node-public-launcher", "README.md":b"wrapper-readme"}
        native_files = {"package.json":json.dumps(native_json).encode(),
                        "vendor/" + triple + "/bin/codex":b"official-native",
                        "vendor/" + triple + "/codex-package.json":b"native-layout"}
        wrapper, wrapper_sri = archive([("package/" + name, data, tarfile.REGTYPE) for name,data in wrapper_files.items()])
        native, native_sri = archive([("package/" + name, data, tarfile.REGTYPE) for name,data in native_files.items()])
        self.enterContext(patch.dict(runner.INTEGRITIES, {
            runner.TARGET:wrapper_sri, runner.TARGET + "-" + target:native_sri}))
        (root / "verified-wrapper.tgz").write_bytes(wrapper)
        (root / "verified-platform.tgz").write_bytes(native)
        for name, contract, members, sri in (("wrapper",wrapper_json,wrapper_files,wrapper_sri),
                                              ("platform",native_json,native_files,native_sri)):
            metadata = dict(contract, dist={"integrity":sri,"fileCount":len(members),
                                          "unpackedSize":sum(map(len,members.values()))})
            (root / ("verified-" + name + ".metadata.json")).write_text(json.dumps(metadata))
        contents = dict(wrapper_files)
        contents.update({"node_modules/" + runner.PACKAGE + "-" + target + "/" + name:data
                         for name,data in native_files.items()})
        package = root / "prefix/lib/node_modules/@openai/codex"
        for relative, content in contents.items():
            path = package / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        journal = {"wrapper_archive_sha256":list(hashlib.sha256(wrapper).digest()),
                   "platform_archive_sha256":list(hashlib.sha256(native).digest()),
                   "prepared":{"nodes":{name:{"sha256":list(hashlib.sha256(content).digest()),
                                                 "length":len(content)} for name,content in contents.items()}}}
        (root / "prepared-journal.safe.json").write_text(json.dumps(journal))
        return package, journal

    def test_independent_archive_check_accepts_exact_prepared_and_published_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.product_receipt(root)
            runner.verify_product_archives(root, "updated", root / "unused-old")
            self.assertTrue(json.loads((root / "independent-archive-check.safe.json").read_text())["sri_verified"])

    def test_independent_archive_check_rejects_forged_prepared_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            _, journal = self.product_receipt(root)
            journal["prepared"]["nodes"]["bin/codex.js"]["sha256"] = [0] * 32
            (root / "prepared-journal.safe.json").write_text(json.dumps(journal))
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "updated", root / "unused-old")

    def test_independent_archive_check_rejects_changed_published_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package, _ = self.product_receipt(root)
            (package / "bin/codex.js").write_bytes(b"later changed")
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "updated", root / "unused-old")

    def test_external_change_exception_is_exactly_one_owned_marker(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package, _ = self.product_receipt(root)
            (package / "external-npm-change").write_bytes(b"later external install must survive\n")
            (package / "unexpected-extra").write_bytes(b"not part of the specified external change")
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "external_change_preserved", root / "unused-old")

    def test_fixed_scope_excludes_unreviewed_platforms_and_versions(self):
        self.assertEqual((runner.OLD, runner.TARGET), ("0.155.1", "0.156.1"))
        self.assertEqual(set(runner.CASES), {"updated", "swap_receipt_missing",
                         "external_change_preserved", "candidate_changed_preserved"})
        for system, machine in (("Darwin","x86_64"),("Linux","aarch64"),("Windows","AMD64")):
            with self.subTest(system=system, machine=machine):
                with patch.object(runner.platform,"system",return_value=system), patch.object(
                    runner.platform,"machine",return_value=machine), self.assertRaises(ValueError):
                    runner.target_platform()
        with tempfile.TemporaryDirectory() as temporary, patch.object(runner,"get") as download:
            with self.assertRaises(ValueError):
                runner.official_package("0.157.0",Path(temporary))
            download.assert_not_called()

    def test_raw_metadata_is_archived_before_rejection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            raw = b'{"name":"unexpected", "version":"0.155.1"}'
            with patch.object(runner,"get",return_value=raw), self.assertRaises(ValueError):
                runner.official_package(runner.OLD, root)
            self.assertEqual((root / "openai-codex-0.155.1.metadata.json").read_bytes(), raw)

    def test_reused_official_material_revalidates_fixed_sri_without_network(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache, reused = root / "new", root / "original"
            cache.mkdir()
            reused.mkdir()
            manifest = {"name":runner.PACKAGE,"version":runner.OLD}
            encoded = json.dumps(manifest).encode()
            raw, sri = archive([("package/package.json",encoded,tarfile.REGTYPE)])
            metadata = dict(manifest,dist={"integrity":sri,"fileCount":1,"unpackedSize":len(encoded),
                "tarball":"https://registry.npmjs.org/@openai/codex/-/codex-0.155.1.tgz"})
            original = json.dumps(metadata).encode()
            (reused / "openai-codex-0.155.1.metadata.json").write_bytes(original)
            (reused / "openai-codex-0.155.1.tgz").write_bytes(raw)
            with patch.dict(runner.INTEGRITIES,{runner.OLD:sri}), patch.object(runner,"get") as download:
                members, _ = runner.official_package(runner.OLD,cache,reused)
            download.assert_not_called()
            self.assertEqual(members["package.json"][0],encoded)
            self.assertEqual((reused / "openai-codex-0.155.1.metadata.json").read_bytes(),original)
            self.assertEqual((cache / "openai-codex-0.155.1.tgz").read_bytes(),raw)

    def test_private_old_tree_uses_alias_directory_and_keeps_node_launcher(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target, triple = "linux-x64", "x86_64-unknown-linux-musl"
            self.enterContext(patch.object(runner, "target_platform", return_value=(target, triple)))
            dependency = runner.PACKAGE + "-" + target
            wrapper = {"package.json":(b"wrapper-manifest",False),"README.md":(b"readme",False),
                       "bin/codex.js":(b"public-node-launcher",True)}
            native = {"package.json":(b"native-manifest",False),
                      "vendor/" + triple + "/bin/codex":(b"native",True),
                      "vendor/" + triple + "/codex-package.json":(b"resources-layout",False)}
            top = {"bin":{"codex":"bin/codex.js"},"optionalDependencies":{
                dependency:"npm:" + runner.PACKAGE + "@" + runner.OLD + "-" + target}}
            child = {"os":[target.split("-")[0]],"cpu":[target.split("-")[1]]}
            with patch.object(runner,"official_package",side_effect=[(wrapper,top),(native,child)]):
                old = runner.prepare_old(root)
            self.assertEqual((old / "bin/codex.js").read_bytes(),b"public-node-launcher")
            self.assertEqual((old / "node_modules" / dependency / "vendor" / triple / "bin/codex").read_bytes(),b"native")
            self.assertFalse((old / "bin/codex").exists())

    def test_platform_alias_metadata_name_must_remain_codex(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.product_receipt(root)
            path = root / "verified-platform.metadata.json"
            value = json.loads(path.read_bytes())
            value["name"] = runner.PACKAGE + "-" + runner.target_platform()[0]
            path.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root,"updated",root / "unused-old")

if __name__ == "__main__":
    unittest.main()
