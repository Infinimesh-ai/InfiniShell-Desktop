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

    def test_powershell_reader_build_is_required_but_does_not_run_the_fixed_fixture(self):
        steps = DATA["jobs"]["windows"]["steps"]
        build = next(step for step in steps if step.get("id") == "ps_clr_reader")
        native = next(step for step in steps if step.get("id") == "codex_npm_updates")
        preserve = next(step for step in steps if step.get("name") == "Preserve PowerShell npm reader build evidence")
        self.assertIn("inputs.windows_atomic_debug_scope == 'g09_npm_powershell'", build["if"])
        self.assertIn("inputs.run_windows_npm_native_witness", build["if"])
        self.assertEqual(build["shell"], "powershell")
        self.assertEqual(build["run"], "./script/ci/g09-clr-fixture/build-reader.ps1")
        self.assertIn("steps.ps_clr_reader.outcome == 'success'", native["if"])
        self.assertIn("inputs.windows_atomic_debug_scope != 'g09_npm_powershell'", native["if"])
        self.assertIn("if ($witnessMode -eq 'powershell')", native["run"])
        self.assertIn("'--clr-reader', $env:INFINISHELL_CLR_READER, '--clr-reader-sha256'", native["run"])
        self.assertIn("sources::npm_windows::live_tests::windows_npm_witness_", native["run"])
        self.assertIn("atomic_windows::native_clr::tests::", native["run"])
        self.assertIn("always()", preserve["if"])
        self.assertIn("steps.ps_clr_reader.outcome == 'success' || steps.ps_clr_reader.outcome == 'failure'", preserve["if"])
        self.assertIn("g09-clr-build-${{ github.run_id }}-${{ github.run_attempt }}/", preserve["with"]["path"])
        self.assertEqual(preserve["with"]["if-no-files-found"], "error")
        self.assertFalse(any("framework_exception_chain_uses_original_event_thread_and_reaps_reader" in step.get("run", "") for step in steps))

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


class PowerShellContractScopeTests(unittest.TestCase):
    def values(self):
        return dict(static_inputs(), windows_atomic_debug_scope="g09_powershell_contract")

    def test_only_standalone_windows_collection_is_selected(self):
        values = self.values()
        self.assertEqual(len(INPUTS), 25)
        self.assertTrue(accepted(values))
        self.assertFalse(accepted(dict(values, run_windows=False)))
        self.assertEqual([name for name, job in DATA["jobs"].items()
                          if name != "validate_scope" and enabled(job["if"], values)],
                         ["windows_g09_powershell_contract"])
        condition = DATA["jobs"]["windows_g09_powershell_contract"]["if"]
        self.assertFalse(enabled(condition, {}, "push"))
        self.assertFalse(enabled(condition, defaults()))

    def test_collection_rejects_every_other_boolean_and_nondefault_choice(self):
        values = self.values()
        for name, spec in INPUTS.items():
            if spec["type"] == "boolean" and name != "run_windows":
                with self.subTest(forbidden=name):
                    self.assertFalse(accepted(dict(values, **{name: True})))
            elif spec["type"] == "choice" and name != "windows_atomic_debug_scope":
                for value in spec["options"]:
                    if value != spec["default"]:
                        with self.subTest(forbidden=name, value=value):
                            self.assertFalse(accepted(dict(values, **{name: value})))

    def test_actual_windows_powershell_host_and_always_preserved_evidence(self):
        job = DATA["jobs"]["windows_g09_powershell_contract"]
        self.assertEqual(job["needs"], "validate_scope")
        self.assertEqual(job["runs-on"], ["self-hosted", "windows", "x64", "infinishell-ci"])
        self.assertEqual(len(job["steps"]), 3)
        self.assertTrue(job["steps"][0]["uses"].startswith("actions/checkout@"))
        self.assertEqual(job["steps"][1]["shell"], "powershell")
        self.assertEqual(job["steps"][1]["run"], "./script/ci/collect_g09_powershell_contract.ps1")
        artifact = job["steps"][2]
        self.assertEqual(artifact["if"], "${{ always() }}")
        self.assertTrue(artifact["uses"].startswith("actions/upload-artifact@"))
        self.assertEqual(artifact["with"]["if-no-files-found"], "error")
        self.assertEqual(artifact["with"]["path"],
                         "${{ runner.temp }}/g09-powershell-contract-${{ github.run_id }}-${{ github.run_attempt }}/")


class ClrFixtureScopeTests(unittest.TestCase):
    def values(self):
        return dict(static_inputs(), windows_atomic_debug_scope="g09_clr_fixture")

    def test_only_standalone_fixed_fixture_job_is_selected(self):
        values = self.values()
        self.assertEqual(len(INPUTS), 25)
        self.assertTrue(accepted(values))
        self.assertFalse(accepted(dict(values, run_windows=False)))
        self.assertEqual([name for name, job in DATA["jobs"].items()
                          if name != "validate_scope" and enabled(job["if"], values)],
                         ["windows_g09_clr_fixture"])
        condition = DATA["jobs"]["windows_g09_clr_fixture"]["if"]
        self.assertFalse(enabled(condition, {}, "push"))
        self.assertFalse(enabled(condition, defaults()))

    def test_fixture_rejects_every_other_mode(self):
        values = self.values()
        for name, spec in INPUTS.items():
            if spec["type"] == "boolean" and name != "run_windows":
                with self.subTest(forbidden=name):
                    self.assertFalse(accepted(dict(values, **{name: True})))
            elif spec["type"] == "choice" and name != "windows_atomic_debug_scope":
                for value in spec["options"]:
                    if value != spec["default"]:
                        with self.subTest(forbidden=name, value=value):
                            self.assertFalse(accepted(dict(values, **{name: value})))

    def test_native_fixture_runs_once_after_local_checks_with_original_evidence(self):
        job = DATA["jobs"]["windows_g09_clr_fixture"]
        self.assertEqual(job["needs"], "validate_scope")
        self.assertEqual(job["runs-on"], ["self-hosted", "windows", "x64", "infinishell-ci"])
        self.assertEqual(job["steps"][0]["with"]["ref"], "${{ github.sha }}")
        commands = [step for step in job["steps"] if "run" in step]
        self.assertEqual(commands[0]["run"], "cargo check --locked -p command --tests")
        self.assertIn("--retries 0", commands[1]["run"])
        self.assertIn("test(windows::clr_fixture_tests::) | test(windows::clr_reader::tests::)", commands[1]["run"])
        self.assertEqual(commands[2]["shell"], "powershell")
        self.assertEqual(commands[2]["run"], "./script/ci/g09-clr-fixture/build-reader.ps1")
        self.assertEqual(commands[3]["shell"], "powershell")
        self.assertEqual(commands[3]["run"], "./script/ci/g09-clr-fixture/prepare.ps1 -Reader $env:INFINISHELL_CLR_READER")
        self.assertEqual(commands[4]["timeout-minutes"], 3)
        self.assertEqual(commands[4]["run"], "cargo nextest run --locked --no-fail-fast --retries 0 --run-ignored only -p command --lib -E 'test(windows::clr_fixture_tests::framework_exception_chain_uses_original_event_thread_and_reaps_reader)'")
        artifact = job["steps"][-1]
        self.assertEqual(artifact["if"], "${{ always() }}")
        self.assertEqual(artifact["with"]["if-no-files-found"], "error")
        for directory in ("g09-clr-build", "g09-clr-fixture"):
            self.assertIn("${{ runner.temp }}/" + directory + "-${{ github.run_id }}-${{ github.run_attempt }}/", artifact["with"]["path"])
        self.assertEqual(len(commands), 5)


class ClaudeMuslScopeTests(unittest.TestCase):
    def values(self):
        values = defaults()
        values.update(run_linux=True, run_windows=False, native_acceptance_only=True,
                      run_claude_npm_updates=True, linux_atomic_agent="claude_musl")
        return values

    def test_musl_requires_only_linux_native_claude_npm_and_keeps_25_inputs(self):
        values = self.values()
        self.assertEqual(len(INPUTS), 25)
        self.assertTrue(accepted(values))
        self.assertEqual([name for name, job in DATA["jobs"].items()
                          if name != "validate_scope" and enabled(job["if"], values)], ["linux"])
        self.assertEqual(DATA["jobs"]["linux"]["needs"], "validate_scope")
        for name in ("run_linux", "native_acceptance_only", "run_claude_npm_updates"):
            with self.subTest(required=name):
                self.assertFalse(accepted(dict(values, **{name: False})))

    def test_musl_rejects_every_other_boolean_and_nondefault_choice(self):
        values = self.values()
        for name, spec in INPUTS.items():
            if spec["type"] == "boolean" and not values[name]:
                with self.subTest(forbidden=name):
                    self.assertFalse(accepted(dict(values, **{name: True})))
            elif spec["type"] == "choice" and name != "linux_atomic_agent":
                for value in spec["options"]:
                    if value != spec["default"]:
                        with self.subTest(forbidden=name, value=value):
                            self.assertFalse(accepted(dict(values, **{name: value})))

    def test_existing_linux_agent_choices_keep_their_native_scope(self):
        self.assertEqual(INPUTS["linux_atomic_agent"]["default"], "all")
        for agent in ("all", "codex", "claude", "grok"):
            with self.subTest(agent=agent):
                values = defaults()
                values.update(run_windows=False, native_acceptance_only=True,
                              run_cli_atomic_updates=True, linux_atomic_agent=agent)
                self.assertTrue(accepted(values))

    def test_only_musl_passes_the_libc_argument_to_the_original_driver(self):
        step = next(step for step in DATA["jobs"]["linux"]["steps"]
                    if step.get("id") == "claude_npm_updates")
        self.assertEqual(step["timeout-minutes"], 45)
        command = step["run"][step["run"].index("libc_args=()") :]
        # 运行原参数组装片段，用Shell函数接收argv；不执行驱动、CLI或二进制。
        capture = "digest() { printf 'fixed-sha'; }; python3() { printf '%s\\0' \"$@\"; };\n"
        common = ["-B", "script/cli-agent-parity/run_claude_npm_update_live.py",
                  "--repo", "/source fixture", "--output", "/private fixture/evidence",
                  "--test-binary", "/private fixture/warp-libtest", "--test-binary-sha256", "fixed-sha",
                  "--supervisor", "/private fixture/supervisor", "--supervisor-sha256", "fixed-sha",
                  "--node", "/runtime fixture/node", "--node-sha256", "fixed-sha",
                  "--npm-cli", "/runtime fixture/npm", "--npm-cli-sha256", "fixed-sha"]
        for agent in INPUTS["linux_atomic_agent"]["options"]:
            with self.subTest(agent=agent):
                values = dict(self.values(), linux_atomic_agent=agent)
                environment = dict(os.environ, GITHUB_WORKSPACE="/source fixture",
                                   fixture_parent="/private fixture", node_path="/runtime fixture/node",
                                   npm_path="/runtime fixture/npm",
                                   CLAUDE_NPM_MUSL="true" if enabled(step["env"]["CLAUDE_NPM_MUSL"], values) else "false")
                result = subprocess.run(["bash", "-euo", "pipefail", "-c", capture + command],
                                        env=environment, capture_output=True, text=True, check=True)
                expected = common + (["--linux-libc", "musl"] if agent == "claude_musl" else [])
                self.assertEqual(result.stdout.split("\0"), expected + [""])
                self.assertEqual(result.stderr, "")


if __name__ == "__main__":
    unittest.main(verbosity=2)
