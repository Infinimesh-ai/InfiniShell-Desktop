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

spec = importlib.util.spec_from_file_location("npm_live", Path(__file__).with_name("run_claude_npm_update_live.py"))
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
        raw, sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/bin/claude.exe", b"official", tarfile.REGTYPE)])
        self.assertEqual(runner.archive_members(raw, sri)["bin/claude.exe"][0], b"official")

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
        self.assertEqual(env["NPM_CONFIG_USERCONFIG"], "/private/case/npm/user.npmrc")
        self.assertEqual(env["INFINISHELL_CLI_NPM_UPDATE_STEP"], "execute")
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
        wrapper, wrapper_sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/bin/claude.exe", b"placeholder", tarfile.REGTYPE)])
        native, native_sri = archive([("package/package.json", b"{}", tarfile.REGTYPE), ("package/claude", b"official-native", tarfile.REGTYPE)])
        name = "@anthropic-ai/claude-code-darwin-arm64"
        (root / "verified-wrapper.tgz").write_bytes(wrapper)
        (root / "verified-platform.tgz").write_bytes(native)
        (root / "verified-wrapper.metadata.json").write_text(json.dumps({"name":"@anthropic-ai/claude-code","version":runner.TARGET,"dist":{"integrity":wrapper_sri}}))
        (root / "verified-platform.metadata.json").write_text(json.dumps({"name":name,"version":runner.TARGET,"dist":{"integrity":native_sri}}))
        contents = {"package.json":b"{}", "bin/claude.exe":b"official-native", "node_modules/"+name+"/package.json":b"{}", "node_modules/"+name+"/claude":b"official-native"}
        package = root / "prefix/lib/node_modules/@anthropic-ai/claude-code"
        for relative, content in contents.items():
            path = package / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        journal = {"wrapper_archive_sha256":list(hashlib.sha256(wrapper).digest()),"platform_archive_sha256":list(hashlib.sha256(native).digest()),
                   "prepared":{"nodes":{name:{"sha256":list(hashlib.sha256(content).digest()),"length":len(content)} for name,content in contents.items()}}}
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
            journal["prepared"]["nodes"]["bin/claude.exe"]["sha256"] = [0] * 32
            (root / "prepared-journal.safe.json").write_text(json.dumps(journal))
            with self.assertRaises(ValueError):
                runner.verify_product_archives(root, "updated", root / "unused-old")

    def test_independent_archive_check_rejects_changed_published_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package, _ = self.product_receipt(root)
            (package / "bin/claude.exe").write_bytes(b"later changed")
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

    def test_case_scope_does_not_claim_downgrade_support(self):
        self.assertIn("unreviewed_downgrade_rejected", runner.CASES)
        self.assertNotIn("downgraded", runner.CASES)
        self.assertEqual((runner.OLD, runner.TARGET), ("2.1.278", "2.1.280"))

if __name__ == "__main__":
    unittest.main()
