"""直接执行工作流的前置范围代码，确认静态取证不能混入原生或源码门禁。"""
import contextlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/cross-platform-preflight.yml"
# 沿用本机工作流审计的 Ruby 标准库 YAML，不要求为此安装 Python 依赖。
DATA = json.loads(subprocess.check_output([
    "ruby", "-r", "yaml", "-r", "json", "-e",
    "puts JSON.generate(YAML.load_file(ARGV[0]))", str(WORKFLOW),
], text=True))
TRIGGER = DATA.get("on", DATA.get("true"))
INPUTS = TRIGGER["workflow_dispatch"]["inputs"]
RUN = DATA["jobs"]["validate_scope"]["steps"][0]["run"]
SCOPE = compile(RUN.split("python3 - <<'PY'\n", 1)[1].rsplit("\nPY", 1)[0], "workflow-scope", "exec")


def accepted(values):
    with patch.dict(os.environ, {"PREFLIGHT_INPUTS": json.dumps(values)}):
        with contextlib.redirect_stdout(io.StringIO()):
            try:
                exec(SCOPE, {})
            except SystemExit as error:
                return error.code in (0, None)
    return True


def defaults():
    return {name: spec["default"] for name, spec in INPUTS.items()}


def static_inputs():
    values = defaults()
    values.update(run_linux=False, run_windows=True, windows_atomic_debug_scope="g09_image_contract")
    return values


def enabled(condition, values, event="workflow_dispatch"):
    expression = condition.removeprefix("${{").removesuffix("}}").strip()
    expression = expression.replace("&&", " and ").replace("||", " or ")
    expression = re.sub(r"!(?!=)", "not ", expression)
    return bool(eval(expression, {"__builtins__": {}}, {
        "github": SimpleNamespace(event_name=event), "inputs": SimpleNamespace(**values),
    }))


class ImageContractScopeTests(unittest.TestCase):
    def test_standalone_windows_and_input_limit(self):
        self.assertEqual(len(INPUTS), 25)
        self.assertTrue(accepted(static_inputs()))
        self.assertFalse(accepted(dict(static_inputs(), run_windows=False)))

    def test_each_other_boolean_is_rejected(self):
        for name, spec in INPUTS.items():
            if spec["type"] == "boolean" and name != "run_windows":
                with self.subTest(name=name):
                    self.assertFalse(accepted(dict(static_inputs(), **{name: True})))

    def test_each_nondefault_other_choice_is_rejected(self):
        for name, spec in INPUTS.items():
            if spec["type"] != "choice" or name == "windows_atomic_debug_scope":
                continue
            for value in spec["options"]:
                if value != spec["default"]:
                    with self.subTest(name=name, value=value):
                        self.assertFalse(accepted(dict(static_inputs(), **{name: value})))

    def test_existing_default_push_full_and_source_modes(self):
        for values in ({}, defaults(), dict(defaults(), full_workspace_tests=True), dict(defaults(), source_gate_only=True)):
            with self.subTest(values=values):
                self.assertTrue(accepted(values))
        self.assertFalse(accepted(dict(defaults(), source_gate_only=True, full_workspace_tests=True)))

    def test_existing_hooks_and_witness_modes(self):
        base = defaults()
        base["run_linux"] = False
        hooks = dict(base, run_atomic_windows_debug=True, windows_atomic_debug_scope="codex_hooks")
        witness = dict(base, windows_grok_installer_only=True, run_windows_codex_npm_updates=True, run_windows_npm_native_witness=True)
        self.assertTrue(accepted(hooks))
        self.assertTrue(accepted(witness))
        self.assertFalse(accepted(dict(hooks, run_windows_npm_native_witness=True)))
        self.assertFalse(accepted(dict(witness, source_gate_only=True)))

    def test_powershell_witness_uses_existing_scope_and_only_the_original_four_flags(self):
        base = defaults()
        base.update(run_linux=False, run_windows=True, windows_grok_installer_only=True,
                    run_windows_codex_npm_updates=True, run_windows_npm_native_witness=True,
                    windows_atomic_debug_scope="g09_npm_powershell")
        self.assertEqual(len(INPUTS), 25)
        self.assertTrue(accepted(base))
        self.assertEqual([name for name, job in DATA["jobs"].items()
                          if name != "validate_scope" and enabled(job["if"], base)], ["windows"])
        for name in ("run_windows", "windows_grok_installer_only", "run_windows_codex_npm_updates",
                     "run_windows_npm_native_witness"):
            with self.subTest(required=name):
                self.assertFalse(accepted(dict(base, **{name:False})))
        for name, spec in INPUTS.items():
            if spec["type"] == "boolean" and not base[name]:
                with self.subTest(forbidden=name):
                    self.assertFalse(accepted(dict(base, **{name:True})))
        self.assertFalse(accepted(dict(base, windows_atomic_debug_scope="codex_hooks")))
        self.assertFalse(accepted(dict(base, windows_atomic_debug_scope="unknown")))

    def test_independent_job_has_only_checkout_collection_and_preservation(self):
        job = DATA["jobs"]["windows_g09_image_contract"]
        self.assertEqual(job["needs"], "validate_scope")
        self.assertIn("inputs.windows_atomic_debug_scope == 'g09_image_contract'", job["if"])
        self.assertEqual(len(job["steps"]), 3)
        self.assertTrue(job["steps"][0]["uses"].startswith("actions/checkout@"))
        self.assertEqual(job["steps"][1]["run"], "./script/ci/collect_g09_windows_images.ps1")
        self.assertEqual(job["steps"][1]["shell"], "pwsh")
        self.assertTrue(job["steps"][2]["uses"].startswith("actions/upload-artifact@"))
        self.assertEqual(job["steps"][2]["if"], "${{ always() }}")
        self.assertIn("inputs.windows_atomic_debug_scope != 'g09_image_contract'", DATA["jobs"]["windows"]["if"])
        # 不能使用 job 级 always/!cancelled 绕过前置范围拒绝。
        for name, item in DATA["jobs"].items():
            if name == "validate_scope":
                continue
            self.assertNotIn("always()", item.get("if", ""))
            self.assertNotIn("cancelled()", item.get("if", ""))

    def test_only_static_job_selected_and_push_does_not_select_it(self):
        selected = [name for name, job in DATA["jobs"].items()
                    if name != "validate_scope" and enabled(job["if"], static_inputs())]
        self.assertEqual(selected, ["windows_g09_image_contract"])
        condition = DATA["jobs"]["windows_g09_image_contract"]["if"]
        self.assertFalse(enabled(condition, {}, "push"))
        self.assertFalse(enabled(condition, defaults()))


if __name__ == "__main__":
    unittest.main(verbosity=2)
