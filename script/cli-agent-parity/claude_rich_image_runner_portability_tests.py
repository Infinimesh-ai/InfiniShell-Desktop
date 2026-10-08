"""图片验收运行器的路径和编码回归；只启动 Python，不运行 CLI、模型或读取认证文件。"""

from contextlib import ExitStack, redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import probe_claude_rich_images as rich
import run_claude_image_skill_live as skill


REAL_RUN = subprocess.run
REAL_MKDTEMP = tempfile.mkdtemp
REAL_OPEN = Path.open
REAL_IS_DIR = Path.is_dir
FAILURE_TEXT = "图片恢复失败：附件 一.png\n"


class RichImageRunnerPortabilityTests(unittest.TestCase):
    def test_production_runner_import_does_not_require_pillow(self):
        source = """
import builtins
import sys
sys.path[:0] = sys.argv[1:]
original = builtins.__import__
def without_pillow(name, *args, **kwargs):
    if name == 'PIL' or name.startswith('PIL.'):
        raise ImportError('生产验收不应加载 Pillow')
    return original(name, *args, **kwargs)
builtins.__import__ = without_pillow
import probe_claude_rich_images
"""
        result = REAL_RUN([sys.executable, "-B", "-c", source, str(Path(rich.__file__).parent),
                           str(Path(skill.prepare_project.__code__.co_filename).parent)],
                          capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(result.returncode, 0, result.stderr)

    def failed_probe(self, module, missing_private_tmp=False, timed_out=False,
                     macos_registered=False, multi=False):
        with tempfile.TemporaryDirectory(prefix="claude-image-runner-offline-") as temporary:
            root = Path(temporary)
            binary = root / "未执行的二进制"
            binary.write_bytes(b"offline fixture only")
            output = root / "收据"
            args = SimpleNamespace(output=output, executable=binary, test_binary=binary,
                                   supervisor=binary, case="jpeg", model="claude-opus-5-5",
                                   private_root=root / "r-offline")
            binding = {"critical_source_sha256": {"offline-fixture": "a" * 64}}
            requested_dirs = []
            authenticated_environment = []
            if macos_registered:
                args.private_root.mkdir(mode=0o700)

            def isolated_root(*args, **kwargs):
                requested_dirs.append(kwargs.get("dir"))
                if missing_private_tmp and kwargs.get("dir") == "/private/tmp":
                    raise FileNotFoundError("目标平台不存在 /private/tmp")
                kwargs["dir"] = root
                return REAL_MKDTEMP(*args, **kwargs)

            def windows_legacy_text_open(path, mode="r", buffering=-1, encoding=None,
                                         errors=None, newline=None):
                if "b" not in mode and encoding in (None, "locale"):
                    encoding = "cp1252"
                return REAL_OPEN(path, mode, buffering, encoding, errors, newline)

            def fixture_process(command, **kwargs):
                if module is rich and command[1:] == ["auth", "status", "--json"]:
                    self.assertEqual(kwargs.get("cwd").name, "project")
                    authenticated_environment.append(kwargs["env"])
                    self.assertNotIn("CLAUDE_CONFIG_DIR", kwargs["env"])
                    self.assertTrue(rich.adapter.API_ENVIRONMENT_KEYS.isdisjoint(kwargs["env"]))
                    return subprocess.CompletedProcess(command, 0, json.dumps({
                        "loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
                        "subscriptionType": "pro", "email": "private-account@example.invalid",
                        "token": "PRIVATE_AUTH_CANARY"}), "")
                self.assertEqual(kwargs.get("cwd"), module.REPOSITORY)
                if module is rich:
                    self.assertIs(kwargs["env"], authenticated_environment[0])
                else:
                    self.assertEqual(command[1], skill.MULTI_TEST if multi else skill.TEST)
                if macos_registered:
                    self.assertEqual(kwargs["env"]["TMPDIR"], str(args.private_root))
                    self.assertEqual(kwargs["env"]["TMP"], str(args.private_root))
                    self.assertEqual(kwargs["env"]["TEMP"], str(args.private_root))
                    self.assertEqual(kwargs["env"]["INFINISHELL_CLAUDE_LIVE_ROOT"], str(args.private_root))
                for key in ("HOME", "CODEX_HOME"):
                    if key in kwargs["env"]:
                        self.assertEqual(kwargs["env"][key], os.environ.get(key))
                events = Path(kwargs["env"]["INFINISHELL_CLAUDE_LIVE_ARTIFACT"])
                events.write_bytes((json.dumps({"event": "acceptance_failed", "reason": FAILURE_TEXT},
                                               ensure_ascii=False) + "\n").encode("utf-8"))
                if timed_out:
                    raise subprocess.TimeoutExpired(command, 1, output=FAILURE_TEXT.encode("utf-8"),
                                                    stderr=b"")
                # 真实管道发出 UTF-8 字节，模拟 Rust；不调用命令行中的验收二进制。
                # setup-python 的 Linux 解释器需要自身运行库路径；只补给夹具，不改变 CLI 环境。
                kwargs["env"] = kwargs["env"].copy()
                if "LD_LIBRARY_PATH" in os.environ:
                    kwargs["env"]["LD_LIBRARY_PATH"] = os.environ["LD_LIBRARY_PATH"]
                return REAL_RUN([sys.executable, "-c",
                                 "import sys;sys.stdout.buffer.write(bytes.fromhex(sys.argv[1]));sys.exit(1)",
                                 FAILURE_TEXT.encode("utf-8").hex()], **kwargs)

            with ExitStack() as stack:
                stack.enter_context(patch.object(module, "source_binding", return_value=binding))
                stack.enter_context(patch.object(tempfile, "mkdtemp", side_effect=isolated_root))
                stack.enter_context(patch.object(Path, "open", windows_legacy_text_open))
                stack.enter_context(patch.object(subprocess, "_text_encoding", return_value="cp1252"))
                stack.enter_context(patch.object(subprocess, "run", side_effect=fixture_process))
                stack.enter_context(patch.object(subprocess, "check_output", return_value="b" * 40))
                if module is rich:
                    stack.enter_context(patch.object(rich.prepare_claude_cli, "current_platform",
                        return_value="darwin-arm64" if macos_registered else "linux-x64"))
                    if macos_registered:
                        verified_root = stack.enter_context(patch.object(rich, "registered_macos_root",
                            return_value=args.private_root))
                else:
                    stack.enter_context(patch.object(skill, "current_platform",
                        return_value="darwin-arm64" if macos_registered else "linux-x64"))
                    if macos_registered:
                        verified_root = stack.enter_context(patch.object(skill, "private_macos_root",
                            return_value=args.private_root))
                if missing_private_tmp:
                    stack.enter_context(patch.object(Path, "is_dir", lambda path:
                        False if str(path) == "/private/tmp" else REAL_IS_DIR(path)))
                stack.enter_context(redirect_stdout(io.StringIO()))
                with self.assertRaises(SystemExit) as stopped:
                    if module is rich:
                        module.run_production(args, {"platform": "offline-only"})
                    else:
                        stack.enter_context(patch.object(module, "verify_binary", return_value={}))
                        stack.enter_context(patch.object(module, "verify_version"))
                        arguments = ["probe", "--executable", str(binary), "--test-binary", str(binary),
                                     "--supervisor", str(binary), "--output", str(output)]
                        if macos_registered:
                            arguments.extend(["--private-root", str(args.private_root)])
                        if multi:
                            arguments.append("--multi")
                        stack.enter_context(patch.object(sys, "argv", arguments))
                        module.main()
                self.assertEqual(stopped.exception.code, 1)
            self.assertEqual((output / "test-output.txt").read_bytes(), FAILURE_TEXT.encode("utf-8"))
            receipt = json.loads((output / "receipt.json").read_text(encoding="utf-8"))
            self.assertFalse(receipt["acceptance_passed"] if module is rich else receipt["passed"])
            self.assertNotIn("PRIVATE_AUTH_CANARY", json.dumps(receipt))
            self.assertNotIn("private-account@example.invalid", json.dumps(receipt))
            if macos_registered:
                verified_root.assert_called_once_with(args.private_root)
                self.assertEqual(requested_dirs, [])
            if missing_private_tmp:
                self.assertEqual(requested_dirs, [None])

    def test_fixture_preserves_its_python_loader_environment(self):
        loader_path = os.pathsep.join(filter(None, (
            str(Path(tempfile.gettempdir()) / "offline-python-loader-fixture"),
            os.environ.get("LD_LIBRARY_PATH"))))
        real_run = REAL_RUN
        observed = []

        def checked_run(command, **kwargs):
            observed.append(kwargs["env"].get("LD_LIBRARY_PATH"))
            self.assertEqual(observed[-1], loader_path)
            return real_run(command, **kwargs)

        with patch.dict(os.environ, {"LD_LIBRARY_PATH": loader_path}):
            with patch(f"{__name__}.REAL_RUN", side_effect=checked_run):
                for module in (rich, skill):
                    with self.subTest(module=module.__name__):
                        self.failed_probe(module)
        self.assertEqual(len(observed), 2)

    def test_source_binding_git_queries_use_the_runner_repository(self):
        for module in (rich, skill):
            with self.subTest(module=module.__name__):
                with patch.object(subprocess, "check_output", return_value="b" * 40) as query:
                    module.source_binding()
                self.assertTrue(query.call_args_list)
                for call in query.call_args_list:
                    self.assertEqual(call.kwargs.get("cwd"), module.REPOSITORY)
                    self.assertEqual(call.kwargs.get("encoding"), "utf-8")

    def test_production_skill_probe_uses_platform_temp_without_private_tmp(self):
        self.failed_probe(skill, missing_private_tmp=True)

    def test_production_format_probe_uses_platform_temp_without_private_tmp(self):
        self.failed_probe(rich, missing_private_tmp=True)

    def test_production_format_uses_registered_short_macos_tmpdir(self):
        self.failed_probe(rich, macos_registered=True)

    def test_production_skill_uses_registered_short_macos_tmpdir(self):
        for multi in (False, True):
            with self.subTest(multi=multi):
                self.failed_probe(skill, macos_registered=True, multi=multi)

    def test_production_authentication_ignores_parent_private_configuration(self):
        with patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": "unrelated-parent-config",
                                    "ANTHROPIC_API_KEY": "UNRELATED_PARENT_AUTH"}):
            self.failed_probe(rich)

    def test_unavailable_default_account_cannot_launch_production_test(self):
        with tempfile.TemporaryDirectory(prefix="claude-auth-environment-offline-") as temporary:
            root = Path(temporary)
            binary = root / "never-executed"
            args = SimpleNamespace(output=root / "evidence", executable=binary, test_binary=binary,
                                   supervisor=binary, case="jpeg", model="claude-opus-5-5")

            def unauthenticated(command, **kwargs):
                self.assertEqual(command[1:], ["auth", "status", "--json"])
                self.assertNotIn("CLAUDE_CONFIG_DIR", kwargs["env"])
                self.assertNotIn("ANTHROPIC_API_KEY", kwargs["env"])
                return subprocess.CompletedProcess(command, 0, '{"loggedIn":false}', "")

            with ExitStack() as stack:
                stack.enter_context(patch.dict(os.environ, {
                    "CLAUDE_CONFIG_DIR": "logged-in-parent-config",
                    "ANTHROPIC_API_KEY": "UNRELATED_PARENT_AUTH"}))
                stack.enter_context(patch.object(rich.prepare_claude_cli, "current_platform",
                                                return_value="linux-x64"))
                stack.enter_context(patch.object(tempfile, "mkdtemp", return_value=str(root)))
                process = stack.enter_context(patch.object(subprocess, "run", side_effect=unauthenticated))
                with self.assertRaisesRegex(ValueError, "默认账户尚未登录"):
                    rich.run_production(args, {})
                self.assertEqual(process.call_count, 1)
                self.assertFalse((args.output / "events.ndjson").exists())

    def test_production_main_does_not_probe_authentication_in_parent_environment(self):
        arguments = ["probe", "--executable", "fixed-claude", "--test-binary", "libtest",
                     "--supervisor", "supervisor", "--case", "jpeg", "--output", "evidence"]
        with ExitStack() as stack:
            stack.enter_context(patch.object(sys, "argv", arguments))
            stack.enter_context(patch.object(rich.prepare_claude_cli, "verify_binary", return_value={}))
            stack.enter_context(patch.object(rich.prepare_claude_cli, "current_platform",
                                            return_value="linux-x64"))
            version = stack.enter_context(patch.object(subprocess, "check_output",
                                                      return_value="2.1.280 (Claude Code)\n"))
            production = stack.enter_context(patch.object(rich, "run_production"))
            rich.main()
            self.assertEqual(version.call_count, 1)
            self.assertEqual(version.call_args.args[0], ["fixed-claude", "--version"])
            production.assert_called_once()

    def test_macos_root_rejects_missing_or_external_round_path(self):
        for path in (None, Path("/Volumes/external/r-offline")):
            with self.subTest(path=path):
                with self.assertRaises(ValueError):
                    rich.registered_macos_root(path)

    @unittest.skipUnless(hasattr(os, "getuid"), "登记身份检查仅适用于 macOS")
    def test_macos_root_requires_matching_running_directory_record(self):
        with tempfile.TemporaryDirectory(prefix="claude-registered-root-offline-") as temporary:
            base = Path(temporary)
            root = base / "r-offline"
            root.mkdir(mode=0o700)
            records = base / "records"
            records.mkdir(mode=0o700)
            record = records / f"{root.name}.json"
            directory = root.stat()
            registered = {
                "run_id": root.name, "status": "running", "temporary_directory": str(root),
                "temporary_realpath": str(root.resolve()), "directory_device": directory.st_dev,
                "directory_inode": directory.st_ino, "directory_owner": directory.st_uid,
                "directory_mode": 0o700, "cleanup_ready": False,
            }
            with patch.object(rich, "private_macos_root", return_value=root):
                with self.assertRaises(FileNotFoundError):
                    rich.registered_macos_root(root)
                record.write_text(json.dumps(registered), encoding="utf-8")
                record.chmod(0o600)
                self.assertEqual(rich.registered_macos_root(root), root)
                for key, value in (("directory_inode", directory.st_ino + 1),
                                   ("temporary_realpath", "/Volumes/external/r-offline"),
                                   ("status", "cleaned"), ("cleanup_ready", True)):
                    with self.subTest(key=key):
                        changed = dict(registered, **{key: value})
                        record.write_text(json.dumps(changed), encoding="utf-8")
                        with self.assertRaisesRegex(ValueError, "登记状态或目录身份不匹配"):
                            rich.registered_macos_root(root)
                record.unlink()
                record.symlink_to(root)
                with self.assertRaisesRegex(ValueError, "登记文件身份或权限"):
                    rich.registered_macos_root(root)

    def test_production_format_failure_preserves_utf8_under_windows_codepage(self):
        self.failed_probe(rich)

    def test_production_skill_failure_preserves_utf8_under_windows_codepage(self):
        self.failed_probe(skill)

    def test_production_format_timeout_preserves_utf8_failure(self):
        self.failed_probe(rich, timed_out=True)

    def test_production_skill_timeout_preserves_utf8_failure(self):
        self.failed_probe(skill, timed_out=True)


if __name__ == "__main__":
    unittest.main()
