#!/usr/bin/env python3
"""只验证 runner 的输入和证据边界，不执行 CLI、网络或升级。"""
import base64
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
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
