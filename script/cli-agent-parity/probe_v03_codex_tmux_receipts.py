#!/usr/bin/env python3
"""在固定 Linux Codex SSH 探针上增加 tmux pane 内侧原始字节收据。"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile
import uuid

sys.dont_write_bytecode = True
import probe_codex_remote_ssh_tmux as base

CODEX_VERSION = "0.156.1"
CODEX_RELATIVE = "codex/runtime-0.156.1-linux-x64/bin/codex"


def verify_fixed_codex_hashes(repository, binary_sha256, archive_sha256):
    manifest = json.loads((repository / "script/cli-agent-parity/codex_0156_package_manifest.json").read_text())
    package = manifest["packages"]["linux-x64"]
    base.require(manifest["version"] == CODEX_VERSION
        and package["entrypoint"] == "bin/codex"
        and binary_sha256 == package["files"]["bin/codex"][1]
        and archive_sha256 == package["sha256"], "Codex 0.156.1 完整包摘要不符")


def replace_once(source, before, after):
    base.require(source.count(before) == 1, "V03 插桩锚点已变化")
    return source.replace(before, after)


def remote_source():
    source = base.REMOTE_SOURCE
    source = replace_once(source, "import hashlib,json,os,pathlib,shlex,subprocess,sys",
        "import base64,hashlib,json,os,pathlib,re,shlex,subprocess,sys,time")
    source = replace_once(source, "if action=='kill-tmux':", '''if action=='inner':
    mode=sys.argv[3]
    if mode!='tmux-off': raise SystemExit('inner capture is only for tmux-off')
    raw=(root/'reports'/mode/'inner-pane.raw').read_bytes()
    if len(raw)>4*1024*1024: raise SystemExit('inner capture exceeds limit')
    print(json.dumps({'base64':base64.b64encode(raw).decode(),
                      'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest()}))
    raise SystemExit(0)
if action=='kill-tmux':''')
    native_start = "process=subprocess.Popen([str(codex),'--no-alt-screen'," + repr(base.PROMPT) + "],env=env,cwd=work)"
    source = replace_once(source, native_start,
        '''if mode=='tmux-off':
    pane=os.environ.get('TMUX_PANE','')
    if re.fullmatch(r'%[0-9]+',pane) is None: raise SystemExit('invalid tmux pane')
    socket=config['sockets'][mode]
    option=subprocess.check_output([str(tmux),'-S',socket,'show-option','-gv',
                                    'allow-passthrough'],env=env,text=True).strip()
    if option!='off': raise SystemExit('tmux passthrough is not off')
    pane_tty=subprocess.check_output([str(tmux),'-S',socket,'display-message','-p',
                                      '-t',pane,'#{pane_tty}'],env=env,text=True).strip()
    if pane_tty!=os.ttyname(0): raise SystemExit('tmux pane tty mismatch')
    capture=reports/'inner-pane.raw'
    command='/bin/cat > '+shlex.quote(str(capture))
    subprocess.run([str(tmux),'-S',socket,'pipe-pane','-O','-t',pane,command],
                   env=env,check=True,capture_output=True)
    (reports/'inner-capture.json').write_text(json.dumps({
        'pane':pane,'pane_tty':pane_tty,'allow_passthrough':option,
        'installed_before_codex_start':True}))
''' + native_start)
    source = replace_once(source,
        "(reports/'native-exit.json').write_text(json.dumps({'pid':process.pid,'exit_code':code,'forced':forced}))",
        '''if mode=='tmux-off':
    subprocess.run([str(tmux),'-S',socket,'pipe-pane','-t',pane],env=env,
                   check=True,capture_output=True)
    deadline=time.monotonic()+2
    while time.monotonic()<deadline:
        if capture.exists() and capture.read_bytes().count(b'\x1bPtmux;')>=2: break
        time.sleep(.02)
(reports/'native-exit.json').write_text(json.dumps({'pid':process.pid,'exit_code':code,'forced':forced}))''')
    return source


def inner_notifications(raw):
    messages = []
    for frame in re.findall(rb"\x1bPtmux;(.*?)\x1b\\", raw, re.DOTALL):
        decoded = frame.replace(b"\x1b\x1b", b"\x1b")
        for body in base.OSC.findall(decoded):
            try:
                messages.append(json.loads(body))
            except (UnicodeError, ValueError):
                raise RuntimeError("内层原生 OSC 内容无效") from None
    return messages


def validate_inner_outer(case, native, capture, inner):
    base.require(capture == {"pane": native["SessionStart"]["environment"]["TMUX_PANE"],
        "pane_tty": native["native-start"]["tty"], "allow_passthrough": "off",
        "installed_before_codex_start": True}, "内层 pipe-pane 时序或身份不符")
    session = native["UserPromptSubmit"]["input"]["session_id"]
    turn = native["UserPromptSubmit"]["input"]["turn_id"]
    work = native["UserPromptSubmit"]["input"]["cwd"]
    messages = inner_notifications(inner)
    base.require(len(messages) == 2 and all(value.get("session_id") == session for value in messages)
        and all(value.get("agent") == "codex" and value.get("v") == 1
                and value.get("cwd") == work for value in messages)
        and {value.get("event") for value in messages} == {"session_start", "prompt_submit"}
        and next(value for value in messages if value["event"] == "prompt_submit").get("turn_id") == turn,
        "内层未独立观察到同一原生 session/turn 的两次发送")
    outer = base64.b64decode(case["ssh_stdout_base64"], validate=True)
    base.require(hashlib.sha256(outer).hexdigest() == case["ssh_stdout_sha256"]
        and len(outer) == case["ssh_stdout_bytes"]
        and b"warp://cli-agent" not in outer,
        "外层 SSH 收据不完整或收到通知")
    return {"native_hook_records": 2, "inner_events": ["session_start", "prompt_submit"],
        "inner_bytes": len(inner), "inner_sha256": hashlib.sha256(inner).hexdigest(),
        "outer_bytes": len(outer), "outer_sha256": hashlib.sha256(outer).hexdigest(),
        "outer_notification_count": 0, "same_native_session_and_turn": True,
        "pipe_pane_before_codex_start": True}


def safe_report(report, remote_root, tokens):
    safe = base.sanitize(report, remote_root, tokens)
    safe.pop("authorization_rpc", None)
    safe.pop("native_install", None)
    for case in safe["cases"]:
        for key in ("native", "notifications", "native_session_id", "native_turn_id",
                    "inner_pane_base64"):
            case.pop(key, None)
    safe["safe_receipt_contains_raw_terminal_bytes"] = False
    return safe


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True)
    parser.add_argument("--remote-root", required=True)
    parser.add_argument("--codex-relative", default=CODEX_RELATIVE)
    parser.add_argument("--tmux-relative", default="pkg/usr/bin/tmux")
    parser.add_argument("--jq-relative", default="pkg/usr/bin/jq")
    parser.add_argument("--library-relative", default="pkg/usr/lib/x86_64-linux-gnu")
    parser.add_argument("--expected-codex-sha256", required=True)
    parser.add_argument("--expected-codex-package-sha256", required=True)
    parser.add_argument("--expected-tmux-sha256", required=True)
    parser.add_argument("--private-output", required=True, type=Path)
    parser.add_argument("--safe-output", required=True, type=Path)
    args = parser.parse_args()
    base.validate_remote_root(args.remote_root)
    for relative in (args.codex_relative, args.tmux_relative, args.jq_relative, args.library_relative):
        path = PurePosixPath(relative)
        base.require(not path.is_absolute() and ".." not in path.parts, "远端相对路径越界")
    base.require(not args.private_output.exists() and not args.safe_output.exists(), "输出收据已经存在")
    repository = Path(__file__).resolve().parents[2]
    verify_fixed_codex_hashes(repository, args.expected_codex_sha256,
        args.expected_codex_package_sha256)
    tokens = {mode: uuid.uuid4().hex for mode in base.MODES}
    ssh = base.ssh_base(args.host)
    report = {"source_commit": subprocess.check_output(["git", "-C", str(repository),
        "rev-parse", "HEAD"], text=True).strip(), "host_alias": args.host,
        "source_tree_dirty": bool(subprocess.check_output(["git", "-C", str(repository),
            "status", "--porcelain"], text=True)),
        "target": "remote_linux_codex_0.156.1_tmux_3.6", "cases": [], "passed": False,
        "product_ssh_ui_verified": False, "model_lifecycle_verified": False,
        "bidirectional_interaction_verified": False, "disconnect_recovery_verified": False,
        "cancellation_verified": False}
    try:
        with tempfile.TemporaryDirectory(prefix="infinishell-v03-") as temporary:
            staging = Path(temporary)
            expected_tree, command = base.prepare_tree(staging, repository, args.remote_root, args, tokens)
            (staging / "remote_driver.py").write_text(remote_source())
            base.transfer_tree(ssh, staging, args.remote_root)
            verify = base.remote_json(ssh, "python3", args.remote_root + "/remote_driver.py",
                args.remote_root, "verify")
            base.require(verify["root"] == args.remote_root
                and verify["codex_sha256"] == args.expected_codex_sha256
                and verify["codex_package_sha256"] == args.expected_codex_package_sha256
                and verify["tmux_sha256"] == args.expected_tmux_sha256
                and verify["codex_version"] == f"codex-cli {CODEX_VERSION}"
                and verify["tmux_version"].startswith("tmux 3.6"), "目标远端固定输入不符")
            report["remote_inputs"] = verify
            report["reference_tree_sha256"] = expected_tree
            report["native_install"] = base.remote_json(ssh, "python3",
                args.remote_root + "/remote_driver.py", args.remote_root, "install")
            base.authorize(ssh, args.remote_root, command, report)
            for mode in base.MODES:
                case = {"mode": mode, "passed": False}
                report["cases"].append(case)
                case.update(base.capture_case(base.remote_command(ssh, "python3",
                    args.remote_root + "/remote_driver.py", args.remote_root,
                    "run", mode, force_tty=True), tokens[mode]))
                native = base.remote_json(ssh, "python3", args.remote_root + "/remote_driver.py",
                    args.remote_root, "reports", mode)
                capture = native.pop("inner-capture", None)
                base.validate_case(mode, case, native, tokens[mode], CODEX_VERSION)
                if mode == "tmux-off":
                    inner = base.remote_json(ssh, "python3", args.remote_root + "/remote_driver.py",
                        args.remote_root, "inner", mode)
                    raw = base64.b64decode(inner["base64"], validate=True)
                    base.require(len(raw) == inner["bytes"]
                        and hashlib.sha256(raw).hexdigest() == inner["sha256"], "内层原始字节摘要不符")
                    case["dual_end_receipt"] = validate_inner_outer(case, native, capture, raw)
                    case["inner_pane_base64"] = inner["base64"]
                    case["inner_capture"] = capture
            report["passed"] = all(case["passed"] for case in report["cases"])
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        try:
            cleanup = subprocess.run(base.remote_command(ssh, "python3",
                args.remote_root + "/remote_driver.py", args.remote_root, "kill-tmux"),
                capture_output=True, timeout=15)
            report["tmux_cleanup_verified"] = cleanup.returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            report["tmux_cleanup_verified"] = False
        report["passed"] = report["passed"] and report["tmux_cleanup_verified"]
    base.write_exclusive(args.private_output, report)
    safe = safe_report(report, args.remote_root, tokens)
    safe["private_receipt_sha256"] = base.digest(args.private_output)
    base.write_exclusive(args.safe_output, safe)
    print(json.dumps({"passed": report["passed"], "safe_output": str(args.safe_output),
        "error": report.get("error")}, ensure_ascii=False))
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
