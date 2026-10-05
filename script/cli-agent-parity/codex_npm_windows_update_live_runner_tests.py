#!/usr/bin/env python3
"""Windows npm 验收入口的离线合同测试；不安装 npm 包，不运行 Node/CLI。"""
import hashlib
import json
import os
import re
from pathlib import Path, PureWindowsPath
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_codex_npm_windows_update_live as runner


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.binaries = {name:{"path":str(self.root / (name + ".exe")), "sha256":"a" * 64}
                         for name in ("node", "npm_cli", "worker", "supervisor", "station_bootstrap")}
        for binary in self.binaries.values():
            Path(binary["path"]).write_bytes(b"offline fixture; never execute")

    def tearDown(self):
        self.temporary.cleanup()

    def source_fixture(self):
        repo = self.root / "repo"
        payloads = {}
        for name in runner.SOURCE_FILES:
            # 每份源码用独特小字节串，避免一个成员被另一成员的内容意外覆盖。
            payloads[name] = b"source-binding<" + hashlib.sha256(name.encode()).hexdigest().encode() + b">"
            path = repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(payloads[name])
        return repo, payloads

    def test_source_roles_match_real_compiled_tables_and_paths(self):
        repo = Path(__file__).resolve().parents[2]
        source = repo / "app/src/terminal/cli_agent_updates/sources.rs"
        text = source.read_text(encoding="utf-8")
        table = re.search(r"static SUPERVISOR_UPDATER_SOURCE_BINDING: \[&\[u8\]; (\d+)\] = \[(.*?)\n\];", text, re.S)
        self.assertIsNotNone(table)
        embedded = re.findall(r'include_bytes!\(\s*"([^"]+)"\s*\)', table.group(2))
        self.assertEqual(len(embedded), int(table.group(1)))
        embedded_paths = [(source.parent / name).resolve() for name in embedded]
        self.assertTrue(all(path.is_file() for path in embedded_paths))
        embedded_names = {path.relative_to(repo).as_posix() for path in embedded_paths}
        self.assertEqual(len(embedded_names), len(embedded))

        worker = source.with_name("sources_codex_npm_windows_live_tests.rs")
        table = re.search(r"const SOURCES:.*?= &\[(.*?)\n\];", worker.read_text(encoding="utf-8"), re.S)
        self.assertIsNotNone(table)
        pairs = re.findall(r'\(\s*"([^"]+)",\s*include_bytes!\(\s*"([^"]+)"\s*\)', table.group(1))
        self.assertEqual(len(pairs), len({name for name, relative in pairs}))
        for name, relative in pairs:
            with self.subTest(name=name):
                path = (worker.parent / relative).resolve()
                self.assertTrue(path.is_file())
                self.assertEqual(path, repo / name)

        self.assertEqual({name for name, relative in pairs}, set(runner.SOURCE_FILES))
        self.assertEqual(len(runner.SOURCE_FILES), len(set(runner.SOURCE_FILES)))
        self.assertFalse(set(runner.SUPERVISOR_SOURCE_FILES) & set(runner.ACCEPTANCE_SOURCE_FILES))
        self.assertLessEqual(set(runner.SUPERVISOR_SOURCE_FILES), embedded_names)
        self.assertFalse(set(runner.ACCEPTANCE_SOURCE_FILES) & embedded_names)
        builder = (repo / "script/ci/g09-clr-fixture/build-reader.ps1").read_text(encoding="utf-8-sig")
        table = re.search(r"foreach \(\$name in @\((.*?)\)\)", builder, re.S)
        self.assertIsNotNone(table)
        inputs = re.findall(r"'([^']+)'", table.group(1))
        self.assertEqual(len(inputs), 9)
        self.assertLessEqual({"script/ci/g09-clr-reader/" + name for name in inputs},
                             set(runner.ACCEPTANCE_SOURCE_FILES))
        self.assertEqual(runner.SOURCE_FILES[-1], "script/cli-agent-parity/run_codex_npm_windows_update_live.py")

    def test_supervisor_requires_production_bytes_without_acceptance_files(self):
        repo, payloads = self.source_fixture()
        binary = Path(self.binaries["supervisor"]["path"])
        data = b"".join(payloads[name] for name in runner.SUPERVISOR_SOURCE_FILES)
        binary.write_bytes(data)
        self.assertTrue(all(payloads[name] not in data for name in runner.ACCEPTANCE_SOURCE_FILES))
        runner.verify_embedded(binary, repo)

    def test_every_missing_production_source_is_rejected_and_named(self):
        repo, payloads = self.source_fixture()
        binary = Path(self.binaries["supervisor"]["path"])
        for missing in runner.SUPERVISOR_SOURCE_FILES:
            with self.subTest(missing=missing):
                binary.write_bytes(b"".join(payloads[name] for name in runner.SUPERVISOR_SOURCE_FILES if name != missing))
                with self.assertRaises(ValueError) as caught:
                    runner.verify_embedded(binary, repo)
                self.assertEqual(str(caught.exception), "supervisor_source_binding: missing=" + missing)

    def test_binding_reports_all_missing_production_sources(self):
        repo, payloads = self.source_fixture()
        binary = Path(self.binaries["supervisor"]["path"])
        missing = runner.SUPERVISOR_SOURCE_FILES[::2]
        binary.write_bytes(b"".join(payloads[name] for name in runner.SUPERVISOR_SOURCE_FILES if name not in missing))
        with self.assertRaises(ValueError) as caught:
            runner.verify_embedded(binary, repo)
        self.assertEqual(str(caught.exception), "supervisor_source_binding: missing=" + ", ".join(missing))

    def test_embedded_source_spanning_read_blocks_is_preserved(self):
        repo, payloads = self.source_fixture()
        binary = Path(self.binaries["supervisor"]["path"])
        first = payloads[runner.SUPERVISOR_SOURCE_FILES[0]]
        binary.write_bytes(b"\0" * (1024 * 1024 - len(first) // 2)
                           + b"".join(payloads[name] for name in runner.SUPERVISOR_SOURCE_FILES))
        runner.verify_embedded(binary, repo)

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
                                    "NPM_CONFIG_PREFIX":"user-prefix","PSExecutionPolicyPreference":"Bypass",
                                    "INFINISHELL_WINDOWS_NATIVE_WITNESS_ALLOW":"codex-npm-first-cmd-v1",
                                    "INFINISHELL_WINDOWS_NATIVE_WITNESS_GENERATION":"stale",
                                    "INFINISHELL_WINDOWS_NATIVE_WITNESS_MODE":"powershell",
                                    "INFINISHELL_CLR_READER":"must-not-propagate-reader.exe"}):
            result = runner.environment(self.root,self.binaries,self.root / "Windows","execute")
        self.assertNotIn("OPENAI_API_KEY",result)
        self.assertNotIn("NODE_OPTIONS",result)
        self.assertNotIn("NPM_CONFIG_PREFIX",result)
        self.assertNotIn("PSExecutionPolicyPreference",result)
        self.assertNotIn("INFINISHELL_WINDOWS_NATIVE_WITNESS_ALLOW",result)
        self.assertNotIn("INFINISHELL_WINDOWS_NATIVE_WITNESS_GENERATION",result)
        self.assertNotIn("INFINISHELL_WINDOWS_NATIVE_WITNESS_MODE",result)
        self.assertNotIn("INFINISHELL_CLR_READER",result)
        self.assertEqual(Path(result["NPM_CONFIG_USERCONFIG"]),self.root / "npm/user.npmrc")
        self.assertEqual(Path(result["CODEX_HOME"]),self.root / "home/.codex")
        self.assertEqual(result["INFINISHELL_CLI_CODEX_WINDOWS_NPM_ALLOW"],runner.SCOPE)

    def test_witness_rejects_repeated_or_full_matrix_before_native_setup(self):
        for cases in (None, ["old_moved"], ["updated", "updated"], ["updated", "old_moved"]):
            with self.subTest(cases=cases), patch.object(runner, "parser") as parser:
                parser.return_value.parse_args.return_value = SimpleNamespace(native_witness=True, native_witness_mode="cmd", case=cases)
                with patch.object(runner.platform, "system") as platform_read:
                    with self.assertRaisesRegex(ValueError, "witness_requires_one_updated_case"):
                        runner.main()
                    platform_read.assert_not_called()

    def test_witness_mode_defaults_to_cmd_and_selects_only_exact_powershell(self):
        self.assertEqual(runner.parser().get_default("native_witness_mode"), "cmd")
        self.assertEqual(runner.native_witness_permit(True, "cmd", ["updated"]), "codex-npm-first-cmd-v1")
        self.assertEqual(runner.native_witness_permit(True, "powershell", ["updated"]), "codex-npm-first-powershell-v1")
        self.assertIsNone(runner.native_witness_permit(False, "cmd", None))
        with self.assertRaisesRegex(ValueError, "witness_mode_requires_witness"):
            runner.native_witness_permit(False, "powershell", ["updated"])
        with self.assertRaisesRegex(ValueError, "witness_mode_invalid"):
            runner.native_witness_permit(True, "pwsh", ["updated"])
        with self.assertRaisesRegex(ValueError, "witness_requires_one_updated_case"):
            runner.native_witness_permit(True, "powershell", ["updated", "updated"])

    def test_clr_reader_requires_exact_powershell_updated_scope_and_both_arguments(self):
        reader = Path(self.binaries["worker"]["path"])
        digest = runner.sha(reader)
        result = runner.clr_reader_configuration(True, "powershell", ["updated"], reader, digest)
        self.assertEqual(result, {"path":str(runner.canonical(reader)), "sha256":digest})
        for requested, mode, cases in ((False, "cmd", None), (True, "cmd", ["updated"]),
                                       (True, "powershell", ["old_moved"]),
                                       (True, "powershell", ["updated", "updated"])):
            with self.subTest(requested=requested, mode=mode, cases=cases):
                with self.assertRaisesRegex(ValueError, "clr_reader_scope"):
                    runner.clr_reader_configuration(requested, mode, cases, reader, digest)
        for path, expected in ((None, None), (reader, None), (None, digest)):
            with self.subTest(path=path, expected=expected):
                with self.assertRaisesRegex(ValueError, "clr_reader_scope"):
                    runner.clr_reader_configuration(True, "powershell", ["updated"], path, expected)
        self.assertIsNone(runner.clr_reader_configuration(False, "cmd", None, None, None))
        self.assertIsNone(runner.clr_reader_configuration(True, "cmd", ["updated"], None, None))

    def test_clr_reader_rejects_changed_bytes_and_file_or_parent_links(self):
        reader = Path(self.binaries["worker"]["path"])
        digest = runner.sha(reader)
        reader.write_bytes(b"different same role")
        with self.assertRaisesRegex(ValueError, "clr_reader_binding"):
            runner.clr_reader_configuration(True, "powershell", ["updated"], reader, digest)
        digest = runner.sha(reader)
        link = self.root / "linked.exe"
        link.symlink_to(reader)
        with self.assertRaisesRegex(ValueError, "reparse_point"):
            runner.clr_reader_configuration(True, "powershell", ["updated"], link, digest)
        parent = self.root / "linked-parent"
        parent.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "reparse_point"):
            runner.clr_reader_configuration(True, "powershell", ["updated"], parent / reader.name, digest)

    def test_clr_reader_rejects_hardlinks_empty_files_and_malformed_hashes(self):
        reader = Path(self.binaries["worker"]["path"])
        digest = runner.sha(reader)
        linked = self.root / "hardlink.exe"
        os.link(reader, linked)
        with self.assertRaisesRegex(ValueError, "clr_reader_file"):
            runner.clr_reader_configuration(True, "powershell", ["updated"], reader, digest)
        linked.unlink()
        reader.write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "clr_reader_file"):
            runner.clr_reader_configuration(True, "powershell", ["updated"], reader, runner.sha(reader))
        with self.assertRaisesRegex(ValueError, "clr_reader_sha256"):
            runner.clr_reader_configuration(True, "powershell", ["updated"], reader, "../not-a-hash")

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
        for name in ("test-binary","supervisor","station-bootstrap","node","npm-cli"):
            arguments.extend(["--"+name,str(self.root / name),"--"+name+"-sha256","a"*64])
        args = runner.parser().parse_args(arguments + ["--case","old_moved","--case","published_receipt_missing"])
        self.assertEqual(args.case,["old_moved","published_receipt_missing"])
        self.assertEqual(set(runner.COLD_CASES),{"old_moved","published_receipt_missing","external_change_preserved"})


if __name__ == "__main__":
    unittest.main()
