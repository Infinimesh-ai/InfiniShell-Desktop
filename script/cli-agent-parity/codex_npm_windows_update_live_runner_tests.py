#!/usr/bin/env python3
"""Windows npm 验收入口的离线合同测试；不安装 npm 包，不运行 Node/CLI。"""
import hashlib
import json
import os
from pathlib import Path, PureWindowsPath
import tempfile
import unittest
from unittest.mock import patch

import run_codex_npm_windows_update_live as runner


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.binaries = {name:{"path":str(self.root / (name + ".exe")), "sha256":"a" * 64}
                         for name in ("node", "npm_cli", "worker", "supervisor")}
        for binary in self.binaries.values():
            Path(binary["path"]).write_bytes(b"offline fixture; never execute")

    def tearDown(self):
        self.temporary.cleanup()

    def package(self, version):
        metadata = {"name":runner.PACKAGE,"version":version,"scripts":None,"dependencies":None}
        if version in (runner.OLD, runner.TARGET):
            metadata.update(bin={"codex":"bin/codex.js"},
                optionalDependencies={"@openai/codex-win32-x64":"npm:@openai/codex@" + version + "-win32-x64"})
            members = {"README.md":(b"fixed package", False),"bin/codex.js":(b"launcher", True)}
        else:
            metadata.update(os=["win32"],cpu=["x64"])
            members = {"vendor/x86_64-pc-windows-msvc/bin/codex.exe":(b"fixture native", True)}
        members["package.json"] = (json.dumps(metadata).encode(), False)
        metadata["dist"] = {"integrity":runner.INTEGRITIES[version],"fileCount":len(members),
                            "unpackedSize":sum(len(value[0]) for value in members.values())}
        return metadata, members

    def test_fixed_wrapper_and_platform_contract(self):
        for version in runner.INTEGRITIES:
            metadata,members = self.package(version)
            self.assertEqual(runner.package_contract(metadata,members,version)["version"],version)

    def test_alias_must_reference_exact_official_platform_version(self):
        metadata,members = self.package(runner.OLD)
        metadata["optionalDependencies"]["@openai/codex-win32-x64"] = "latest"
        with self.assertRaisesRegex(ValueError,"metadata_manifest_contract"):
            runner.package_contract(metadata,members,runner.OLD)

    def test_archive_inventory_rejects_missing_resources(self):
        metadata,members = self.package(runner.TARGET + "-win32-x64")
        members.pop("vendor/x86_64-pc-windows-msvc/bin/codex.exe")
        with self.assertRaisesRegex(ValueError,"complete_inventory"):
            runner.package_contract(metadata,members,runner.TARGET + "-win32-x64")

    def test_unknown_versions_and_sri_not_admitted(self):
        with self.assertRaisesRegex(ValueError,"version_not_fixed"):
            runner.official_package("0.156.2",self.root)
        metadata,members = self.package(runner.TARGET)
        metadata["dist"]["integrity"] = runner.INTEGRITIES[runner.OLD]
        with self.assertRaisesRegex(ValueError,"fixed_registry_integrity"):
            runner.package_contract(metadata,members,runner.TARGET)

    def test_environment_discards_auth_and_user_npm_settings(self):
        with patch.dict(os.environ,{"OPENAI_API_KEY":"must-not-propagate","NODE_OPTIONS":"--require injected.js",
                                    "NPM_CONFIG_PREFIX":"user-prefix","PSExecutionPolicyPreference":"Bypass"}):
            result = runner.environment(self.root,self.binaries,self.root / "Windows","execute")
        self.assertNotIn("OPENAI_API_KEY",result)
        self.assertNotIn("NODE_OPTIONS",result)
        self.assertNotIn("NPM_CONFIG_PREFIX",result)
        self.assertNotIn("PSExecutionPolicyPreference",result)
        self.assertEqual(Path(result["NPM_CONFIG_USERCONFIG"]),self.root / "npm/user.npmrc")
        self.assertEqual(Path(result["CODEX_HOME"]),self.root / "home/.codex")
        self.assertEqual(result["INFINISHELL_CLI_CODEX_WINDOWS_NPM_ALLOW"],runner.SCOPE)

    def test_actual_npm_cli_arguments_only_target_private_prefix(self):
        archive = self.root / "official-inputs/fixed.tgz"
        archive.parent.mkdir()
        archive.write_bytes(b"offline archive argument")
        (self.root / "prefix").mkdir()
        arguments = runner.npm_install_arguments(self.binaries,self.root / "prefix",archive)
        self.assertEqual(arguments[:3],[self.binaries["node"]["path"],self.binaries["npm_cli"]["path"],"install"])
        self.assertEqual(arguments[arguments.index("--prefix")+1],str(self.root / "prefix"))
        self.assertIn("--ignore-scripts",arguments)
        self.assertIn("--install-strategy=nested",arguments)
        self.assertIn("--registry=https://registry.npmjs.org/",arguments)
        self.assertEqual(arguments[-1],str(archive))
        self.assertFalse(any("ExecutionPolicy" in item or item == "--force" for item in arguments))

    def test_npm_uses_same_objects_with_dos_arguments_and_preserves_bindings(self):
        original = str(PureWindowsPath(r"\\?\C:\private fixture\prefix"))
        binaries = {name:{"path":str(PureWindowsPath(r"\\?\C:\tools") / (name + ".exe")),"sha256":"a"*64}
                    for name in ("node","npm_cli")}
        before = json.dumps(binaries,sort_keys=True)
        with patch.object(runner.os,"name","nt"), patch.object(runner.os.path,"samefile",return_value=True) as same:
            args = runner.npm_install_arguments(binaries,original,original + r"\official.tgz")
        self.assertEqual(args[0],r"C:\tools\node.exe")
        self.assertEqual(args[1],r"C:\tools\npm_cli.exe")
        self.assertEqual(args[args.index("--prefix")+1],r"C:\private fixture\prefix")
        self.assertEqual(args[-1],r"C:\private fixture\prefix\official.tgz")
        self.assertEqual(same.call_count,4)
        self.assertEqual(json.dumps(binaries,sort_keys=True),before)

    def test_npm_path_rejects_identity_change_or_nonlocal_device(self):
        with patch.object(runner.os,"name","nt"), patch.object(runner.os.path,"samefile",return_value=False):
            with self.assertRaisesRegex(ValueError,"npm_path_identity_changed"):
                runner.npm_path(r"\\?\C:\private\file")
            for path in (r"\\?\UNC\server\share", r"\\server\share", r"C:relative", r"\\.\device"):
                with self.subTest(path=path), self.assertRaisesRegex(ValueError,"npm_path_not_local_drive"):
                    runner.npm_path(path)

    def test_private_npm_logs_preserve_original_bytes(self):
        (self.root / ".infinishell-windows-npm-live").write_bytes(runner.MARKER)
        logs = self.root / "npm/cache/_logs"
        logs.mkdir(parents=True)
        raw = b"0 verbose stack RangeError\r\n1 npm private fixture\n"
        (logs / "debug-0.log").write_bytes(raw)
        runner.preserve_npm_debug_logs(self.root)
        self.assertEqual((self.root / "npm-debug-00.stderr").read_bytes(),raw)
        self.assertEqual((logs / "debug-0.log").read_bytes(),raw)
        receipt = json.loads((self.root / "npm-debug-logs.safe.json").read_bytes())
        self.assertTrue(receipt["private_cache_only"])
        self.assertEqual(receipt["files"][0]["sha256"],hashlib.sha256(raw).hexdigest())

    def test_private_npm_log_directory_cannot_be_copied_as_file(self):
        (self.root / ".infinishell-windows-npm-live").write_bytes(runner.MARKER)
        (self.root / "npm/cache/_logs/directory.log").mkdir(parents=True)
        with self.assertRaisesRegex(ValueError,"npm_log_not_bounded_regular_file"):
            runner.preserve_npm_debug_logs(self.root)
        self.assertFalse((self.root / "npm-debug-00.stderr").exists())

    def test_no_handwritten_shim_can_replace_registration_receipt(self):
        with self.assertRaisesRegex(ValueError,"npm_registration_receipt"):
            runner.verify_registration(self.root,{}, {"operation":"synthetic_shims","exit_code":0,"scripts_disabled":True})
        with self.assertRaisesRegex(ValueError,"npm_registration_receipt"):
            runner.verify_registration(self.root,{}, {"operation":"real_npm_private_install","exit_code":1,"scripts_disabled":True})

    def test_registered_tree_rejects_extra_or_changed_file(self):
        package = self.root / "prefix/node_modules/@openai/codex"
        package.mkdir(parents=True)
        (package / "package.json").write_bytes(b"fixed")
        expected = runner.inventory(package)
        (package / "extra.js").write_bytes(b"later change")
        receipt = {"operation":"real_npm_private_install","exit_code":0,"scripts_disabled":True}
        with self.assertRaisesRegex(ValueError,"npm_installed_complete_official_tree"):
            runner.verify_registration(self.root,expected,receipt)
        (package / "extra.js").unlink()
        (package / "package.json").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError,"npm_installed_complete_official_tree"):
            runner.verify_registration(self.root,expected,receipt)

    def test_inventory_detects_reparse_points_without_following_them(self):
        target = self.root / "directory"
        target.mkdir()
        original = target.lstat()
        self.assertEqual(runner.plain(target).st_mode, original.st_mode)
        class Reparse:
            # Python 3.11/3.12 的 is_symlink 也会读取 lstat，保留真实目录类型。
            st_mode = original.st_mode
            st_file_attributes = 0x400
        with patch.object(Path,"lstat",return_value=Reparse()):
            with self.assertRaisesRegex(ValueError,"reparse_point"):
                runner.plain(target)

    def test_full_tree_keeps_platform_auxiliary_files(self):
        expected = runner.expected_files({"bin/codex.js":(b"js",True)},
            {"vendor/native.exe":(b"pe",True),"vendor/resources/config.json":(b"{}",False)})
        self.assertEqual(set(expected),{"bin/codex.js",runner.DEPENDENCY+"vendor/native.exe",
                                     runner.DEPENDENCY+"vendor/resources/config.json"})
        self.assertEqual(expected[runner.DEPENDENCY+"vendor/resources/config.json"]["sha256"],hashlib.sha256(b"{}").hexdigest())

    def test_parser_exposes_both_publication_recovery_points(self):
        arguments = ["--repo",str(self.root),"--output",str(self.root / "new")]
        for name in ("test-binary","supervisor","node","npm-cli"):
            arguments.extend(["--"+name,str(self.root / name),"--"+name+"-sha256","a"*64])
        args = runner.parser().parse_args(arguments + ["--case","old_moved","--case","published_receipt_missing"])
        self.assertEqual(args.case,["old_moved","published_receipt_missing"])
        self.assertEqual(set(runner.COLD_CASES),{"old_moved","published_receipt_missing","external_change_preserved"})


if __name__ == "__main__":
    unittest.main()
