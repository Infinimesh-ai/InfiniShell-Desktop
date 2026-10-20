"""执行实际工作流范围校验，防止退出诊断混入普通发布门禁。"""

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
WORKFLOW = ".github/workflows/cross-platform-preflight.yml"
BASE = "f4dfff885e797fd4db8be9e02f48233d33b35844"


def parse_yaml(text):
    try:
        import yaml
    except ModuleNotFoundError:
        return json.loads(subprocess.check_output(
            ["ruby", "-ryaml", "-rjson", "-e", "puts JSON.generate(YAML.safe_load(STDIN.read))"],
            input=text, text=True,
        ))
    return yaml.safe_load(text)


DATA = parse_yaml((ROOT / WORKFLOW).read_text(encoding="utf-8"))
TRIGGER = DATA.get("on", DATA.get(True, DATA.get("true")))
INPUTS = TRIGGER["workflow_dispatch"]["inputs"]
CODE = DATA["jobs"]["validate_scope"]["steps"][0]["run"]
VALIDATOR = compile(CODE.split("python3 - <<'PY'\n", 1)[1].rsplit("\nPY", 1)[0], "scope", "exec")


def defaults():
    return {name: spec["default"] for name, spec in INPUTS.items()}


def accepted(values):
    with patch.dict(os.environ, {"PREFLIGHT_INPUTS": json.dumps(values)}):
        with contextlib.redirect_stdout(io.StringIO()):
            try:
                exec(VALIDATOR, {})
            except SystemExit as error:
                return error.code in (0, None)
    return True


def selected(values, event="workflow_dispatch"):
    result = []
    for name, job in DATA["jobs"].items():
        if name == "validate_scope":
            continue
        expression = job["if"].removeprefix("${{").removesuffix("}}").strip()
        expression = expression.replace("&&", " and ").replace("||", " or ")
        expression = re.sub(r"!(?!=)", "not ", expression)
        if eval(expression, {"__builtins__": {}}, {
            "github": SimpleNamespace(event_name=event),
            "inputs": SimpleNamespace(**values),
            "startsWith": lambda value, prefix: value.startswith(prefix),
        }):
            result.append(name)
    return result


class ExitDiagnosticScopeTests(unittest.TestCase):
    def diagnostic(self):
        return dict(defaults(), run_linux=False, run_windows=True,
                    windows_atomic_debug_scope="workspace_exit")

    def test_only_explicit_windows_diagnostic_is_selected(self):
        values = self.diagnostic()
        self.assertTrue(accepted(values))
        self.assertEqual(selected(values), ["windows_workspace_exit"])
        self.assertEqual(selected(defaults()), ["linux", "windows"])
        self.assertEqual(selected({}, "push"), ["linux", "windows"])
        self.assertEqual(selected(dict(defaults(), full_workspace_tests=True)), ["linux", "windows"])

    def test_each_conflicting_mode_is_rejected_before_a_runner_starts(self):
        values = self.diagnostic()
        for name, spec in INPUTS.items():
            if spec["type"] == "boolean":
                with self.subTest(name=name):
                    self.assertFalse(accepted(dict(values, **{name: not values[name]})))
            elif spec["type"] == "choice":
                for value in spec["options"]:
                    if value != spec["default"]:
                        with self.subTest(name=name, value=value):
                            self.assertFalse(accepted(dict(values, **{name: value})))

    def test_ordinary_jobs_and_input_defaults_are_preserved(self):
        original = parse_yaml(subprocess.check_output(
            ["git", "show", f"{BASE}:{WORKFLOW}"], cwd=ROOT, text=True,
        ))
        old_trigger = original.get("on", original.get(True, original.get("true")))
        self.assertEqual(TRIGGER, old_trigger)
        for name, job in original["jobs"].items():
            if name == "validate_scope":
                continue
            with self.subTest(job=name):
                current = dict(DATA["jobs"][name])
                if name == "windows":
                    current["if"] = current["if"].replace(
                        "inputs.windows_atomic_debug_scope != 'workspace_exit' && ", "",
                    )
                self.assertEqual(current, job)
        self.assertEqual(DATA["permissions"], {"contents": "read"})
        self.assertEqual(DATA["jobs"]["windows_workspace_exit"]["needs"], "validate_scope")
        # 身份校验比较到三代前的固定 f4d，干净浅克隆也必须包含该基线。
        self.assertGreaterEqual(DATA["jobs"]["windows_workspace_exit"]["steps"][0]["with"]["fetch-depth"], 4)
        self.assertEqual(DATA["jobs"]["windows_workspace_exit"]["steps"][-1]["if"], "${{ always() }}")


if __name__ == "__main__":
    unittest.main()
