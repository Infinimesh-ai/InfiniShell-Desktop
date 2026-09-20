"""固定文件工具证据、字节边界和失败清理离线回归，不启动 CLI 或模型。"""
import copy
from contextlib import nullcontext
import json
import os
import sys
from pathlib import Path
import subprocess
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_grok_files_policy as runner


def evidence():
    receipts, phases = {}, []
    for phase in runner.PHASES:
        row = {key:True for key in runner.PHASE_BOOLS}
        cancelled = phase == "pending_cancel"
        tools = 2 if phase in {"edit_allow", "edit_deny"} else 1
        receipts[phase] = {key:runner.fixed.sha((phase+key).encode()) for key in ("file_before_sha256", "file_after_sha256", "final_sha256")}
        row.update(receipts[phase],event="files_policy_phase",scope=runner.SCOPE,phase=phase,
            submitted=1,accepted=1,approvals=tools,approval_resolved=tools-int(cancelled),approval_cancelled=int(cancelled),
            native_session_sha256=runner.fixed.sha(b"same-session"),
            native_outcome="Completed" if phase in {"write_allow", "edit_allow", "cold_read"} else "Cancelled",failure_stage=None,approval_diagnostic=None)
        if row["native_outcome"] == "Cancelled":
            receipts[phase]["final_sha256"] = None
        phases.append(row)
    end = {key:False for key in runner.END_BOOLS}
    end.update(event="files_policy_finished",scope=runner.SCOPE,passed=True,system_managed_policies_apply=True)
    return [dict(runner.START),*phases,end], receipts


def passed(events,receipts,code=0):
    return runner.observation(code,events,receipts)["files_policy_passed"]


class FilesPolicyRunnerTests(unittest.TestCase):
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

    def test_approval_failure_projection_is_bounded_and_cannot_pass(self):
        diagnostic={key:False for key in runner.APPROVAL_FLAGS}
        diagnostic.update({key:"string" for key in runner.APPROVAL_TYPES})
        diagnostic.update(missing_parameter_count=1,extra_parameter_count=1,
            parameter_types={key:"absent" for key in runner.PARAMETER_KEYS})
        self.assertTrue(runner.approval_summary(diagnostic))
        events,receipts=evidence();events[3]["approval_diagnostic"]=diagnostic
        self.assertFalse(passed(events,receipts))
        events[3].update(passed=False,failure_stage="approval_requested")
        self.assertTrue(runner.validate_event(events[3]))
        for key,value in (("raw_input","OFFLINE_PRIVATE"),("name_type","OFFLINE_PRIVATE"),
                ("missing_parameter_count",True),("parameters_match",1),("parameter_types",{"secret":"string"})):
            changed=diagnostic|{key:value}
            self.assertFalse(runner.approval_summary(changed))

    def test_json_budget_and_duplicate_keys_are_checked_on_all_platforms(self):
        events,_=evidence()
        for raw,valid in (("".join(json.dumps(event)+"\n" for event in events),True),
                ("{}\n"*9,False),('{"event":1,"event":2}\n',False),('{"counter":NaN}\n',False)):
            with patch.object(runner.isolation,"private_bytes",return_value=raw.encode()):
                if valid:self.assertEqual(runner.read_events(Path("synthetic.json")),events)
                else:
                    with self.assertRaises(ValueError):runner.read_events(Path("synthetic.json"))

    def test_exact_six_input_lifecycle_and_scope(self):
        events,receipts=evidence();result=runner.observation(0,events,receipts)
        self.assertTrue(result["files_policy_passed"])
        self.assertTrue(result["cold_native_restore_verified"])
        for key in ("spawn_verified","coordinator_verified","app_restart_verified","filesystem_sandbox_verified",
                "native_effective_policy_verified","shell_build_tools_verified","full_cli_parity_acceptance_passed"):
            self.assertFalse(result[key])

    def test_missing_duplicate_reordered_or_extra_input_fails(self):
        events,receipts=evidence()
        self.assertFalse(passed(events[:-1],receipts))
        self.assertFalse(passed(events+[events[-1]],receipts))
        altered=copy.deepcopy(events);altered[2],altered[3]=altered[3],altered[2]
        self.assertFalse(passed(altered,receipts))
        for value in (2,True,1.0):
            altered=copy.deepcopy(events);altered[3]["submitted"]=value
            self.assertFalse(passed(altered,receipts))
        self.assertFalse(passed(events,receipts,False))

    def test_native_catalog_tool_termination_and_file_bytes_are_all_required(self):
        for index in range(1,7):
            for key in ("native_catalog_verified","native_tool_terminal","file_bytes_match","final_history_verified","same_saved_profile","same_native_session","no_replay_before_input"):
                with self.subTest(index=index,key=key):
                    events,receipts=evidence();events[index][key]=False
                    self.assertFalse(passed(events,receipts))
        events,receipts=evidence();events[6]["native_session_sha256"]=runner.fixed.sha(b"different")
        self.assertFalse(passed(events,receipts))

    def test_permission_denial_and_pending_cancel_cannot_be_claimed_completed(self):
        for index in (2,4,5):
            events,receipts=evidence();events[index]["native_outcome"]="Completed"
            self.assertFalse(passed(events,receipts))
        for key,value in (("approval_resolved",1),("approval_cancelled",0),("approvals",2)):
            events,receipts=evidence();events[5][key]=value
            self.assertFalse(passed(events,receipts))

    def test_receipt_hashes_and_cleanup_cannot_be_forged(self):
        for key in ("file_before_sha256","file_after_sha256","final_sha256"):
            events,receipts=evidence();events[3][key]=runner.fixed.sha(b"wrong")
            self.assertFalse(passed(events,receipts))
        for key in ("cleanup_confirmed","managed_auth_removed","transport_closed","hook_absent_at_ready","hook_absent_after_shutdown"):
            events,receipts=evidence();events[5][key]=False
            self.assertFalse(passed(events,receipts))

    def test_cancelled_receipts_preserve_real_hash_without_inventing_empty_output(self):
        for index in (2,4,5):
            with self.subTest(index=index):
                events,receipts=evidence()
                observed=runner.fixed.sha("审批前已有当前回合文本".encode())
                events[index]["final_sha256"]=observed
                self.assertTrue(passed(events,receipts))
                self.assertEqual(events[index]["final_sha256"],observed)
                for invalid in (None,"",True,"not-a-hash"):
                    altered=copy.deepcopy(events);altered[index]["final_sha256"]=invalid
                    self.assertFalse(passed(altered,receipts))
                receipts[events[index]["phase"]]["final_sha256"]=runner.fixed.sha(b"")
                self.assertFalse(passed(events,receipts))

    def test_projection_rejects_unexpected_text_and_malformed_fields(self):
        for key,value in (("raw_input","private"),("native_outcome",[]),("failure_stage",{}),("phase",[])):
            events,receipts=evidence();events[1][key]=value
            self.assertFalse(runner.validate_event(events[1]))
            self.assertFalse(passed(events,receipts))

    def test_real_synthetic_files_distinguish_allowed_denied_and_cancelled_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/"project").mkdir()
            cases=runner.fixture_cases(root);receipts=runner.expected_receipts(cases)
            self.assertFalse(runner.files_match(root,cases))
            for phase in ("write_allow","edit_allow"):
                row=cases[phase];(root/"project"/row["name"]).write_bytes(row["after"].encode())
            self.assertTrue(runner.files_match(root,cases))
            for phase in ("write_deny","edit_deny","pending_cancel"):
                row=cases[phase];path=root/"project"/row["name"]
                self.assertEqual(receipts[phase]["file_before_sha256"],receipts[phase]["file_after_sha256"])
                path.write_bytes(row["proposed"].encode())
                self.assertFalse(runner.files_match(root,cases))
                if row["before"] is None:path.unlink()
                else:path.write_bytes(row["before"].encode())
            self.assertEqual(cases["cold_read"]["before"],cases["write_allow"]["after"])

    def test_six_input_budget_is_exact_and_deadline_is_bounded(self):
        args=SimpleNamespace(max_native_inputs=6,timeout=900,test_binary=Path("test"),grok=Path("grok"),supervisor=Path("supervisor"),official_grok_home=Path("source"),output=Path("output"))
        with patch.object(runner.fixed.isolation,"validate_paths"):
            runner.fixed.validate_paths(args,max_native_inputs=6,max_deadline=runner.MAX_DEADLINE)
        for budget,deadline in ((7,900),(True,900),(6,901),(6,True)):
            args.max_native_inputs=budget;args.timeout=deadline
            with self.assertRaises(ValueError):
                runner.fixed.validate_paths(args,max_native_inputs=6,max_deadline=runner.MAX_DEADLINE)

    @unittest.skipUnless(os.name == "posix", "真实私有文件读取验证依赖 POSIX 权限和 O_NOFOLLOW")
    def test_eight_events_are_bounded_and_duplicate_keys_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/"evidence";events,_=evidence()
            path.write_text("".join(json.dumps(row)+"\n" for row in events));path.chmod(0o600)
            self.assertEqual(runner.read_events(path),events)
            path.write_text('{"scope":"a","scope":"b"}\n')
            with self.assertRaises(ValueError):runner.read_events(path)
            path.write_text("".join(json.dumps(row)+"\n" for row in events+[events[-1]]))
            with self.assertRaises(ValueError):runner.read_events(path)

    def test_timeout_removes_private_auth_and_never_passes(self):
        self.run_cleanup_failure(False)

    def test_tunnel_close_error_still_removes_auth_and_preserves_failure(self):
        self.run_cleanup_failure(True)

    def run_cleanup_failure(self, close_error):
        with tempfile.TemporaryDirectory() as temporary:
            base=Path(temporary);root=base/"workspace";root.mkdir()
            executable=base/"binary";executable.write_bytes(b"offline-binary")
            args=SimpleNamespace(output=base/"safe.ndjson",grok=executable,test_binary=executable,supervisor=executable,official_grok_home=base/"source",timeout=900)
            tunnel=SimpleNamespace(deadline=time.monotonic()+900,forwarded=0,bytes=0,events=[])
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
                (target/"auth.json").write_text("offline-private-canary")
            with patch.object(runner.isolation,"reserve_artifacts"),patch.object(runner.tempfile,"mkdtemp",return_value=str(root)),patch.object(runner.fixed,"setup_project_sentinel",side_effect=lambda path:(path/"project").mkdir() or {}),patch.object(runner.lease,"auth_identity",return_value=(1,2,3)),patch.object(runner.isolation,"bounded_tunnel",return_value=nullcontext(tunnel)),patch.object(runner.official,"copy_private_auth",side_effect=copy_auth),patch.object(runner.official,"official_environment",return_value={}),patch.object(runner.subprocess,"Popen",return_value=process),patch.object(runner,"read_events",return_value=[]):
                self.assertEqual(runner.run(args),1)
            self.assertFalse((root/"home/.grok/auth.json").exists())
            metadata=json.loads(args.output.with_suffix('.metadata.json').read_text())
            self.assertTrue(metadata["private_auth_copy_removed"])
            self.assertFalse(metadata["files_policy_passed"])
            if close_error:
                self.assertEqual(metadata["tunnel_cleanup_error_type"], "OSError")
                self.assertFalse(metadata["tunnels_stopped"])
                self.assertNotIn("OFFLINE_PRIVATE_CLOSE_CANARY", json.dumps(metadata))
                self.assertEqual(len(attempts),1)
            else:
                self.assertTrue(metadata["timed_out"])
                self.assertEqual(len(attempts),2)


if __name__ == '__main__':unittest.main()
