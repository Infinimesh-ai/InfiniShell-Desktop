#!/usr/bin/env python3
"""原生通知 SSH 运行器的安全边界回归，不调用真实 CLI 或模型。"""

import json
from pathlib import Path
import tempfile
import unittest
import subprocess
from unittest.mock import patch

import probe_native_ssh_notifications as runner


class NativeSshNotificationTests(unittest.TestCase):
    def test_grok_tmux_sandbox_allows_only_its_own_pane_socket(self):
        root = Path("/private/tmp/native-hook-test")
        for mode in runner.MODES:
            profile = runner.grok_frontend_sandbox(root, Path("/private/auth"), 1234, root / mode)
            own_socket = f'(allow network-outbound (remote unix-socket (path-literal "{root / mode / "tmux.sock"}")))'
            self.assertEqual(own_socket in profile, mode in ("tmux-on", "tmux-off"))
            self.assertIn("(deny network*)", profile)
            for other in set(runner.MODES) - {mode}:
                self.assertNotIn(str(root / other / "tmux.sock"), profile)

    def test_grok_bridge_main_uses_the_same_guarded_mapper_on_each_desktop(self):
        bridge = Path(__file__).resolve().parents[2] / "app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs"
        script = r'''const fs=require("node:fs"),vm=require("node:vm"),paths=require("node:path");
const source=fs.readFileSync(process.argv[1],"utf8");const results=[];
for(const platform of ["darwin","linux","win32","freebsd"]){
  for(const mode of ["allowed","disabled","untrusted","override"]){
    const path=platform==="win32"?paths.win32:paths.posix;
    const home=platform==="win32"?"C:\\固定 路径\\.grok":"/private/固定 路径/.grok";
    const plugin=path.join(home,"installed-plugins","cache");let calls=0;
    const state={grokVersion:"1.0.41",projectTrusted:mode!=="untrusted",permissions:{},
      plugins:[{name:"infinishell-grok",enabled:true,path:plugin,provides:{hooks:true}}],
      configSources:{layers:[{role:"user",path:path.join(home,"config.toml")}]}};
    const reader=p=>{if(p.endsWith(".metadata_version"))return "1.0.41";
      if(p.endsWith("disabled-hooks"))return "";
      if(p===path.join(home,"config.toml"))return mode==="disabled"?
        "[plugins]\ndisabled=['infinishell-grok']\n":"[plugins]\nenabled=['infinishell-grok']\n";
      throw Error("unexpected read");};
    const req=name=>{if(name==="node:fs")return {readFileSync:reader};if(name==="node:path")return path;
      if(name==="node:child_process")return {execFileSync:(exe,args,options)=>{
        if(exe!=="fixed-grok"||JSON.stringify(args)!=='["inspect","--json"]'||options.windowsHide!==true)throw Error("invalid inspect");
        return JSON.stringify(state)}};
      if(name===path.join(plugin,"hooks/notify.cjs"))return {main:()=>{calls++}};
      throw Error("unexpected module");};
    const context={require:req,module:{exports:{}},process:{platform,env:{
      GROK_HOOK_NAME:"global/infinishell-1.0.41:notification[0].hooks[0]",
      ...(mode==="override"?{GROK_CONFIG_PATH:"external"}:{})}}};
    vm.runInNewContext(source,context);context.module.exports.main("fixed-grok",plugin);results.push(calls);
  }
}
console.log(JSON.stringify(results));'''
        output = subprocess.check_output(["node", "-e", script, str(bridge)], timeout=5)
        self.assertEqual(json.loads(output), [1, 0, 0, 0] * 3 + [0, 0, 0, 0])

    def test_grok_bridge_checks_every_effective_native_config_layer(self):
        bridge = Path(__file__).resolve().parents[2] / "app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs"
        script = '''const {configLayersAllowPlugin:allows}=require(process.argv[1]);
const home="/private/grok", user={role:"user",path:home+"/config.toml"};
const enabled='[plugins]\\nenabled=["infinishell-grok"]\\n';
const disabled='[plugins]\\ndisabled=["infinishell-grok"]\\n';
const sources=layers=>({configSources:{layers}});
const results=[];
for(const role of ["managed","requirements","system_managed","system_requirements","project"]){
  const layer={role,path:"/private/"+role+"/config.toml"};
  for(const config of [disabled,'[plugins]\\nenabled=[]\\n','[plugins]\\npaths=[]\\n',
      '[permission]\\nrules=[]\\n','["plugins"]\\ndisabled=["infinishell-grok"]\\n']){
    results.push(allows(sources([user,layer]),home,p=>p===user.path?enabled:config));
  }
}
results.push(allows(sources([user,{role:"project",path:"/repo/config.toml"},
  {role:"project",path:"/repo/current/config.toml"}]),home,p=>p==="/repo/config.toml"?disabled:enabled));
results.push(allows(sources([user]),home,()=>{throw Error("unreadable")}));
results.push(allows(sources([user,{role:"env_overlay",path:"$GROK_CONFIG"}]),home,()=>enabled));
results.push(allows(sources([{role:"user",path:"/foreign/config.toml"}]),home,()=>enabled));
results.push(allows({},home,()=>enabled));
console.log(JSON.stringify(results));'''
        output = subprocess.check_output(["node", "-e", script, str(bridge)], timeout=5)
        self.assertEqual(json.loads(output), [False, False, True, True, False] * 5 + [False] * 5)

    def test_grok_bridge_respects_native_per_hook_disable_and_unknown_identity(self):
        bridge = Path(__file__).resolve().parents[2] / "app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs"
        script = '''const {hookIsEnabled:allows}=require(process.argv[1]);
const name="global/infinishell-1.0.41:session_start[0].hooks[0]";
const original="plugin/infinishell-grok/hooks:session_start[0].hooks[0]";
console.log(JSON.stringify([
  allows("",name),allows(name+"\\n",name),allows(original+"\\r\\n",name),
  allows("plugin/other/hooks:session_start[0].hooks[0]\\n",name),
  allows("plugin/infinishell-grok/hooks:stop[0].hooks[0]\\n",name),
  allows("",undefined),allows("",name.replace("[0]","[1]")),allows("\\0",name)]));'''
        output = subprocess.check_output(["node", "-e", script, str(bridge)], timeout=5)
        self.assertEqual(json.loads(output), [True, False, False, True, True, False, False, False])

    def test_grok_bridge_native_config_disable_overrides_incorrect_inspect_enabled(self):
        bridge = Path(__file__).resolve().parents[2] / "app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs"
        configs = [
            '[plugins]\nenabled = ["infinishell-grok"]\n',
            "[plugins]\nenabled = [\n'infinishell-grok',\n]\ndisabled=[]\n",
            '[plugins]\nenabled=[]\ndisabled=["infinishell-grok"]\n',
            '[plugins]\nenabled=["infinishell-grok"]\ndisabled=["source/infinishell-grok"]\n',
            '[plugins]\nenabled=["infinishell-grok"]\n"disabled"=["infinishell-grok"]\n',
            '[plugins]\n[[other]]\nenabled=["infinishell-grok"]\n',
            '[plugins]\nenabled=["infinishell-grok"]\nenabled=[]\n',
            '[plugins]\nenabled=["infinishell-grok"]\ndisabled="invalid"\n',
            'prompt="""\n[plugins]\nenabled=["infinishell-grok"]\n"""\n',
        ]
        script = 'const b=require(process.argv[1]);console.log(JSON.stringify(JSON.parse(process.argv[2]).map(b.configEnablesPlugin)));'
        output = subprocess.check_output(["node", "-e", script, str(bridge), json.dumps(configs)], timeout=5)
        self.assertEqual(json.loads(output), [True, True, False, False, False, False, False, False, False])

    def test_grok_bridge_requires_exact_native_version_trust_and_enabled_registration(self):
        bridge = Path(__file__).resolve().parents[2] / "app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs"
        script = '''const {permitsNotification}=require(process.argv[1]);
const plugin={name:"infinishell-grok",enabled:true,path:"/verified/plugin",provides:{hooks:true}};
const state={grokVersion:"1.0.41",projectTrusted:true,plugins:[plugin],permissions:{}};
const cases=[state,{...state,grokVersion:"1.0.42"},{...state,projectTrusted:false},
{...state,plugins:[{...plugin,enabled:false}]},{...state,plugins:[plugin,plugin]},
{...state,plugins:[{...plugin,path:"/foreign/plugin"}]},null,
{...state,permissions:undefined},{...state,permissions:{enforced:[{setting:"nonManagedHooks",enabled:false}]}}];
console.log(JSON.stringify(cases.map(value=>permitsNotification(value,"/verified/plugin"))));'''
        output = subprocess.check_output(["node", "-e", script, str(bridge)], timeout=5)
        self.assertEqual(json.loads(output), [True, False, False, False, False, False, False, False, False])

    def test_output_refuses_overwriting_previous_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "receipt"
            runner.write_private(target, "first")
            with self.assertRaises(FileExistsError):
                runner.write_private(target, "second")
            self.assertEqual(target.read_text(), "first")
            self.assertEqual(target.stat().st_mode & 0o777, 0o600)

    def test_summary_does_not_publish_prompt_response_or_native_id(self):
        body = {"agent": "codex", "event": "stop", "session_id": "private-session",
                "query": "private prompt", "response": "private response"}
        raw = b'\x1bPtmux;\x1b\x1b]777;notify;warp://cli-agent;' + json.dumps(body).encode() + b'\x07\x1b\\'
        summary = runner.notification_summary(raw, "codex")
        self.assertEqual(summary["events"], {"stop": 1})
        self.assertEqual(summary["native_session_count"], 1)
        self.assertTrue(summary["native_cli_hook_triggered"])
        self.assertFalse(summary["product_ssh_ui_verified"])
        self.assertFalse(summary["outer_tmux_passthrough_verified"])
        for value in ("private-session", "private prompt", "private response"):
            self.assertNotIn(value, json.dumps(summary))

    def test_unknown_agent_or_malformed_notification_does_not_pass(self):
        raw = b'\x1b]777;notify;warp://cli-agent;{bad}\x07'
        raw += b'\x1b]777;notify;warp://cli-agent;{"agent":"other","event":"stop"}\x07'
        self.assertFalse(runner.notification_summary(raw, "grok")["native_cli_hook_triggered"])

    def test_product_log_pairs_only_the_same_private_case_session_and_turn(self):
        prefix = '2026-09-24T03:00:00Z [INFO] Received OSC 777 notification: title=Some("warp://cli-agent"), body='
        root = Path("/private/tmp/notification-case")
        common = {"v": 1, "agent": "codex", "cwd": str(root / "direct/project"),
                  "session_id": "private-session", "query": "private prompt", "response": "private response"}
        payloads = [dict(common, event="prompt_submit", turn_id="turn-1"),
                    dict(common, event="stop", turn_id="turn-1"),
                    dict(common, event="prompt_submit", turn_id="failed-turn"),
                    dict(common, event="stop", turn_id="failed-turn", session_id="other-session"),
                    dict(common, event="stop", turn_id="failed-turn", cwd="/private/tmp/unrelated"),
                    dict(common, event="stop", turn_id="failed-turn", agent="claude")]
        log = "\n".join(prefix + json.dumps(value) for value in payloads).encode()
        receipt = runner.product_notification_summary(log, root, "codex")
        self.assertEqual(receipt["cases"]["direct"]["events"], {"prompt_submit": 2, "stop": 2})
        self.assertEqual(receipt["cases"]["direct"]["paired_prompt_stop_turns"], 1)
        self.assertEqual(receipt["cases"]["direct"]["unpaired_prompt_turns"], 1)
        self.assertFalse(receipt["cases"]["tmux-off"]["product_received_notifications"])
        for value in ("private-session", "other-session", "private prompt", "private response", "failed-turn"):
            self.assertNotIn(value, json.dumps(receipt))

    def test_unknown_remote_mode_is_rejected_before_reading_configuration(self):
        with self.assertRaisesRegex(ValueError, "unknown_case"):
            runner.remote(Path("/does-not-exist"), "direct;touch injected")

    def test_authentication_probe_never_starts_a_cli(self):
        with patch.object(runner.os, "execve") as execute, patch("builtins.print") as output:
            runner.remote(Path("/does-not-exist"), "verify")
            execute.assert_not_called()
            self.assertEqual(json.loads(output.call_args.args[0]),
                {"ssh_authenticated": True, "cli_started": False, "model_inputs": 0})

    def test_native_frontend_worker_receives_the_current_connection_tty(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "local/project").mkdir(parents=True)
            runner.write_private(root / "private-config.json", json.dumps({
                "environment": {"WARP_CLI_AGENT_TTY": "/dev/stale-tty"},
                "commands": {"local": ["/fixed/grok"]}}))
            with patch.object(runner.os, "isatty", return_value=True), \
                    patch.object(runner.os, "ttyname", return_value="/dev/ttys017"), \
                    patch.object(runner.os, "chdir"), \
                    patch.object(runner, "process_identity", return_value=None), \
                    patch.object(runner.os, "execve", side_effect=RuntimeError("process_replaced")) as execute:
                with self.assertRaisesRegex(RuntimeError, "process_replaced"):
                    runner.remote(root, "local")
                self.assertEqual(execute.call_args.args[2]["WARP_CLI_AGENT_TTY"], "/dev/ttys017")

    def test_cleanup_cannot_signal_reused_pid_or_shared_process_group(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "direct").mkdir()
            runner.write_private(root / "direct/process.safe.json",
                json.dumps({"pid": 12, "pgid": 12, "started": "old"}))
            with patch.object(runner, "process_identity", return_value={"pid": 12, "pgid": 12, "started": "new"}), \
                    patch.object(runner, "private_native_pids", return_value=[]), \
                    patch.object(runner.os, "killpg") as kill:
                self.assertTrue(runner.cleanup_native(root, Path("/fixed/cli")))
                kill.assert_not_called()


if __name__ == "__main__":
    unittest.main()
