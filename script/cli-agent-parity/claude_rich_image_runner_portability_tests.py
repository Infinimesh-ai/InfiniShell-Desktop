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

    def failed_probe(self, module, missing_private_tmp=False, timed_out=False):
        with tempfile.TemporaryDirectory(prefix="claude-image-runner-offline-") as temporary:
            root = Path(temporary)
            binary = root / "未执行的二进制"
            binary.write_bytes(b"offline fixture only")
            output = root / "收据"
            args = SimpleNamespace(output=output, executable=binary, test_binary=binary,
                                   supervisor=binary, case="jpeg", model="claude-opus-5-5")
            binding = {"critical_source_sha256": {"offline-fixture": "a" * 64}}
            requested_dirs = []

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
                self.assertEqual(kwargs.get("cwd"), module.REPOSITORY)
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
                        stack.enter_context(patch.object(sys, "argv", ["probe", "--executable", str(binary),
                            "--test-binary", str(binary), "--supervisor", str(binary), "--output", str(output)]))
                        module.main()
                self.assertEqual(stopped.exception.code, 1)
            self.assertEqual((output / "test-output.txt").read_bytes(), FAILURE_TEXT.encode("utf-8"))
            receipt = json.loads((output / "receipt.json").read_text(encoding="utf-8"))
            self.assertFalse(receipt["acceptance_passed"] if module is rich else receipt["passed"])
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
