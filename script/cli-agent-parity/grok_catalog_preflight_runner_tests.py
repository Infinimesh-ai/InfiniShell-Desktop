"""SDK 目录零输入运行器的离线合同；不调用 CLI、模型或真实凭据。"""

from contextlib import nullcontext
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_grok_catalog_preflight as runner


def evidence():
    observed = {k: True for k in runner.OBS_BOOLS}
    observed.update({k: 0 for k in runner.OBS_COUNTS})
    observed.update(event="catalog_observed", scope=runner.SCOPE, catalogs=3,
        outbound_request_attempts=3, outbound_response_attempts=2,
        name_matches={name: True for name in runner.NAMES})
    end = {k: True for k in runner.END_BOOLS}
    end.update(event="catalog_preflight_finished", scope=runner.SCOPE, native_inputs=0,
        business_tool_dispatches=0, full_cli_parity_acceptance_passed=False)
    return [dict(runner.START), observed, end]


class CatalogPreflightRunnerTests(unittest.TestCase):
    def test_precise_six_names_and_zero_input_receipt_are_required(self):
        self.assertTrue(runner.observation(0, evidence())["catalog_union_passed"])
        self.assertFalse(runner.observation(0, evidence())["full_cli_parity_acceptance_passed"])
        for name in runner.NAMES:
            rows = evidence(); rows[1]["name_matches"][name] = False
            self.assertFalse(runner.observation(0, rows)["catalog_union_passed"])
        for key in ("native_inputs", "business_tool_dispatches"):
            rows = evidence(); rows[-1][key] = 1
            self.assertFalse(runner.observation(0, rows)["catalog_union_passed"])

    def test_unserved_wrong_shape_duplicate_unknown_and_cleanup_failure_are_rejected(self):
        for key, value in [("served_names_exact", False), ("union_seen", False), ("unknown_count", 1),
            ("duplicate_count", 1), ("non_string_count", 1), ("guard_rejections", 1),
            ("outbound_request_attempts", 9), ("outbound_response_attempts", 1)]:
            rows = evidence(); rows[1][key] = value
            self.assertFalse(runner.observation(0, rows)["catalog_union_passed"])
        for key in runner.END_BOOLS - {"full_cli_parity_acceptance_passed"}:
            rows = evidence(); rows[-1][key] = False
            self.assertFalse(runner.observation(0, rows)["catalog_union_passed"])

    def test_no_raw_names_extra_fields_wrong_types_or_replayed_events_are_published(self):
        for row in evidence():
            row["private_body"] = "not public"
            self.assertFalse(runner.validate_event(row))
        for bad in ([], {}, True, None, 3):
            rows = evidence(); rows[1]["name_matches"] = bad
            self.assertFalse(runner.observation(0, rows)["catalog_union_passed"])
        for code in (True, None, 101, 1):
            self.assertFalse(runner.observation(code, evidence())["catalog_union_passed"])
        self.assertFalse(runner.observation(0, evidence() * 2)["catalog_union_passed"])
        self.assertFalse(runner.observation(0, evidence()[:2])["catalog_union_passed"])

    def test_safe_reader_rejects_duplicate_keys_and_extra_event_count(self):
        for raw in (b'{"event":1,"event":2}', b'{}\n' * 4):
            with patch.object(runner.isolation, "private_bytes", return_value=raw):
                with self.assertRaises(ValueError): runner.read_events(Path("offline"))
        raw = "\n".join(json.dumps(row) for row in evidence()).encode()
        with patch.object(runner.isolation, "private_bytes", return_value=raw):
            self.assertEqual(runner.read_events(Path("offline")), evidence())

    def test_no_network_is_valid_but_cleanup_and_network_budgets_remain_required(self):
        metadata = dict(runner.fixed.SANDBOX_SCOPE_FIELDS, test_exit_code=0,
            **{k: True for k in ("tunnels_stopped", "private_auth_copy_removed", "original_auth_stat_unchanged",
                "project_snapshot_unchanged", "binary_unchanged")})
        tunnel = SimpleNamespace(forwarded=0, bytes=0, events=[])
        self.assertTrue(runner.boundary_passed(metadata, tunnel))
        metadata["tunnels_stopped"] = False
        self.assertFalse(runner.boundary_passed(metadata, tunnel))
        metadata["tunnels_stopped"] = True
        tunnel.bytes = runner.lease.MAX_TLS_BYTES + 1
        self.assertFalse(runner.boundary_passed(metadata, tunnel))

    def test_timeout_still_cleans_private_auth_and_reports_failure(self):
        self.run_cleanup_failure(False)

    def test_tunnel_close_exception_still_cleans_private_auth(self):
        self.run_cleanup_failure(True)

    def test_main_entrypoint_propagates_failure_to_real_python_process_exit(self):
        # 执行文件中真实的 __main__ 分支和参数解析，只替换有副作用的 run/路径预检。
        child = r"""
import ast, runpy, sys
from pathlib import Path
path=Path(sys.argv[1]); sys.path[:0]=[str(path.parent),sys.argv[2]]
code=int(sys.argv[3]); loaded=runpy.run_path(str(path),run_name="offline_runner_import")
scope=loaded["main"].__globals__; scope["__name__"]="__main__"
scope["fixed"].validate_paths=lambda *args,**kwargs: None
scope["run"]=lambda args: code
scope["subprocess"].Popen=lambda *args,**kwargs: (_ for _ in ()).throw(AssertionError("不能启动 CLI"))
sys.argv=[str(path),"--test-binary","offline","--grok","offline","--supervisor","offline","--official-grok-home","offline","--output","offline"]
tree=ast.parse(path.read_text(encoding="utf-8"))
entry=[node for node in tree.body if isinstance(node,ast.If) and isinstance(node.test,ast.Compare)
    and isinstance(node.test.left,ast.Name) and node.test.left.id=="__name__"]
assert len(entry)==1
exec(compile(ast.Module(body=entry,type_ignores=[]),str(path),"exec"),scope)
"""
        for code in (0,1,2):
            with self.subTest(code=code):
                result=subprocess.run([sys.executable,"-B","-c",child,str(Path(runner.__file__).resolve()),
                    str(Path(runner.lease.__file__).resolve().parent),str(code)],capture_output=True,text=True,timeout=15)
                self.assertEqual(result.returncode,code,result.stderr)
                self.assertEqual(result.stdout,"")

    def run_cleanup_failure(self, close_error):
        with tempfile.TemporaryDirectory() as temporary:
            base=Path(temporary);root=base/"workspace";root.mkdir()
            executable=base/"binary";executable.write_bytes(b"offline-binary")
            args=SimpleNamespace(output=base/"safe.ndjson",grok=executable,test_binary=executable,supervisor=executable,official_grok_home=base/"source",timeout=180)
            tunnel=SimpleNamespace(deadline=time.monotonic()+180,forwarded=0,bytes=0,events=[])
            tunnel.start=lambda:1234
            def close():
                if close_error: raise OSError("OFFLINE_PRIVATE_CLOSE_CANARY")
                return True
            tunnel.close=close
            process=SimpleNamespace(returncode=-9);process.kill=lambda:None
            attempts=[]
            def wait(timeout):
                attempts.append(timeout)
                if len(attempts)==1 and not close_error:raise subprocess.TimeoutExpired('offline',timeout)
                return -9
            process.wait=wait
            def copy_auth(source,target):
                (target/"auth.json").write_text("offline-private-canary", encoding="utf-8")
            with patch.object(runner.isolation,"reserve_artifacts"),patch.object(runner.tempfile,"mkdtemp",return_value=str(root)),patch.object(runner.fixed,"setup_project_sentinel",side_effect=lambda path:(path/"project").mkdir() or {}),patch.object(runner.lease,"auth_identity",return_value=(1,2,3)),patch.object(runner.isolation,"bounded_tunnel",return_value=nullcontext(tunnel)),patch.object(runner.official,"copy_private_auth",side_effect=copy_auth),patch.object(runner.official,"official_environment",return_value={}),patch.object(runner.subprocess,"Popen",return_value=process),patch.object(runner,"read_events",return_value=[]):
                self.assertEqual(runner.run(args),1)
            self.assertFalse((root/"home/.grok/auth.json").exists())
            metadata=json.loads(args.output.with_suffix('.metadata.json').read_text(encoding='utf-8'))
            self.assertTrue(metadata["private_auth_copy_removed"])
            self.assertFalse(metadata["catalog_union_passed"])
            if close_error:
                self.assertEqual(metadata["tunnel_cleanup_error_type"], "OSError")
                self.assertFalse(metadata["tunnels_stopped"])
                self.assertNotIn("OFFLINE_PRIVATE_CLOSE_CANARY", json.dumps(metadata))
                self.assertEqual(len(attempts),1)
            else:
                self.assertTrue(metadata["timed_out"])
                self.assertEqual(len(attempts),2)


if __name__ == "__main__":
    unittest.main()
