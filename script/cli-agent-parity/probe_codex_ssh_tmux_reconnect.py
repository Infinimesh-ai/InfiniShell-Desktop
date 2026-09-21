#!/usr/bin/env python3
"""验证真实 Codex 在 SSH 断连、tmux 分离与重连后的原生 hook 传输。

本探针复用 ``probe_codex_remote_ssh_tmux.py`` 已安装并精确授权的正式插件。
原始 PTY 字节只写私有收据；安全收据不包含凭据、探针令牌或原始字节。
"""

import argparse
import base64
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

sys.dont_write_bytecode = True
from probe_codex_hook_transport import digest, require
from probe_codex_remote_ssh_tmux import (
    OSC,
    PLUGIN_ID,
    PROMPT,
    SAFE_HOST,
    remote_command,
    remote_json,
    sanitize,
    ssh_base,
    transfer_tree,
    validate_remote_root,
    write_exclusive,
)


CASES = ("explicit-detach", "abrupt-disconnect")
SAFE_CASE = re.compile(r"^[a-z-]+$")


REMOTE_SOURCE = r'''
import json,os,pathlib,re,shlex,subprocess,sys
root=pathlib.Path(sys.argv[1]).resolve()
config=json.loads((root/'remote-probe.json').read_text())
action=sys.argv[2]
case=sys.argv[3]
if not re.fullmatch(r'[a-z-]+',case): raise SystemExit('invalid case')
codex=(root/config['codex_relative']).resolve()
tmux=(root/config['tmux_relative']).resolve()
jq=(root/config['jq_relative']).resolve()
libraries=(root/config['library_relative']).resolve()
home=root/'home'; mode='tmux-on'; reports=root/'reports'/mode
socket=config['sockets'][mode]; session='reconnect-'+case
common={'HOME':str(home),'CODEX_HOME':str(home/'.codex'),'SHELL':'/bin/bash','TERM':'xterm-256color',
        'PATH':str(jq.parent)+':/usr/local/bin:/usr/bin:/bin','LD_LIBRARY_PATH':str(libraries),
        'WARP_CLI_AGENT_PROTOCOL_VERSION':'1','WARP_CLIENT_VERSION':'remote-reconnect-probe',
        'TERM_PROGRAM':'WarpTerminal','GIT_CONFIG_NOSYSTEM':'1','GIT_TERMINAL_PROMPT':'0',
        'INFINISHELL_SSH_PROBE_PYTHON':'/usr/bin/python3',
        'INFINISHELL_SSH_PROBE_REPORTS':str(reports),
        'INFINISHELL_SSH_PROBE_TOKEN':config['tokens'][mode]}
def run_tmux(*arguments,check=True):
    return subprocess.run([str(tmux),'-S',socket,*arguments],env={**os.environ,**common},
                          capture_output=True,text=True,timeout=10,check=check)
if action=='prepare':
    if reports.exists():
        archive=root/'reports'/('tmux-on.archived-before-'+case)
        if archive.exists(): raise SystemExit('report archive already exists')
        reports.rename(archive)
    reports.mkdir(parents=True)
    tmux_config=root/('reconnect-'+case+'.conf')
    tmux_config.write_text('set -g status off\nset -g default-shell /bin/sh\nset -g allow-passthrough on\n')
    print(json.dumps({'case':case,'session':session,'socket':socket,
                      'report_directory':str(reports),'tmux_config':str(tmux_config)}))
    raise SystemExit(0)
if action=='start-attached':
    tmux_config=root/('reconnect-'+case+'.conf')
    command=shlex.join(['/usr/bin/python3',__file__,str(root),'run-inner',case])
    os.execve(tmux,[str(tmux),'-f',str(tmux_config),'-S',socket,'new-session',
        '-s',session,'-n','probe','-x','140','-y','40',command],{**os.environ,**common})
if action=='attach':
    os.execve(tmux,[str(tmux),'-S',socket,'attach-session','-t',session],
              {**os.environ,**common})
if action=='run-inner':
    if not all(os.isatty(fd) for fd in (0,1,2)): raise SystemExit('Codex must inherit tmux PTY')
    work=root/'work'/mode; os.chdir(work)
    terminal={'pid':os.getpid(),'pgid':os.getpgrp(),'sid':os.getsid(0),'tty':os.ttyname(0),
              'tmux':os.environ.get('TMUX'),'tmux_pane':os.environ.get('TMUX_PANE')}
    process=subprocess.Popen([str(codex),'--no-alt-screen',__PROMPT__],
                             env={**os.environ,**common},cwd=work)
    terminal['codex_pid']=process.pid
    (reports/'native-start.json').write_text(json.dumps(terminal))
    code=process.wait()
    (reports/'native-exit.json').write_text(json.dumps(
        {'pid':process.pid,'exit_code':code,'forced':False}))
    raise SystemExit(code if code>=0 else 128-code)
if action=='status':
    sessions=run_tmux('list-sessions','-F','#{session_name}|#{session_attached}',check=False)
    panes=run_tmux('list-panes','-t',session,'-F',
        '#{pane_id}|#{pane_dead}|#{pane_pid}|#{pane_current_command}|#{pane_tty}',check=False)
    clients=run_tmux('list-clients','-t',session,'-F','#{client_name}',check=False)
    values={'session_exit':sessions.returncode,'sessions':sessions.stdout.splitlines(),
            'pane_exit':panes.returncode,'panes':panes.stdout.splitlines(),
            'client_exit':clients.returncode,'clients':clients.stdout.splitlines(),
            'reports':sorted(path.name for path in reports.glob('*.json'))}
    print(json.dumps(values)); raise SystemExit(0)
if action=='reports':
    values={path.stem:json.loads(path.read_text()) for path in sorted(reports.glob('*.json'))}
    print(json.dumps(values)); raise SystemExit(0)
if action=='kill':
    run_tmux('kill-session','-t',session,check=False)
    raise SystemExit(0)
raise SystemExit('unknown action')
'''


def notification_values(raw):
    values = []
    for body in OSC.findall(raw):
        try:
            value = json.loads(body)
        except (UnicodeError, ValueError):
            continue
        if value.get("agent") == "codex":
            values.append(value)
    return values


def answer_terminal_queries(master, raw, answered):
    for query, answer in (
        (b"\x1b[6n", b"\x1b[1;1R"),
        (b"\x1b[c", b"\x1b[?1;2c"),
        (b"\x1b[>c", b"\x1b[>0;0;0c"),
        (b"\x1b[?u", b"\x1b[?0u"),
        (b"\x1b]10;?\x1b\\", b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\"),
        (b"\x1b]11;?\x1b\\", b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
    ):
        count = raw.count(query)
        if count > answered.get(query, 0):
            os.write(master, answer * (count - answered.get(query, 0)))
            answered[query] = count


def capture(command, phase, disconnect_method=None, timeout=40):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
    bootstrap = (
        "import fcntl,os,sys,termios; fd=int(sys.argv[1]); os.setsid(); "
        "fcntl.ioctl(fd,termios.TIOCSCTTY,0); "
        "[os.dup2(fd,target) for target in (0,1,2)]; os.close(fd); "
        "os.execvp(sys.argv[2],sys.argv[2:])"
    )
    process = subprocess.Popen(
        [sys.executable, "-c", bootstrap, str(slave), *command], pass_fds=(slave,)
    )
    os.close(slave)
    selector = selectors.DefaultSelector()
    selector.register(master, selectors.EVENT_READ)
    stream = bytearray()
    answered = {}
    trusted = False
    kept_existing_model = False
    prompt_sent = False
    prompt_sent_at = None
    submit_key_attempts = []
    quit_sent = False
    completed_at = None
    started = time.monotonic()
    capture_error = None
    disconnected = False
    try:
        while selector.get_map():
            require(time.monotonic() - started < timeout, f"{phase} PTY 超时")
            for _, _ in selector.select(0.05):
                try:
                    data = os.read(master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b""
                if data:
                    stream.extend(data)
                    require(len(stream) <= 4 * 1024 * 1024, "SSH 输出超过限制")
                else:
                    selector.unregister(master)
            raw = bytes(stream)
            if process.poll() is None:
                answer_terminal_queries(master, raw, answered)
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", raw).decode(
                "utf-8", errors="replace"
            )
            compact = re.sub(r"\s+", "", plain)
            if not trusted and "Doyoutrustthecontentsofthisdirectory?" in compact \
                    and "Yes,continue" in compact and process.poll() is None:
                os.write(master, b"1\r")
                trusted = True
            if not kept_existing_model and "isnolongeravailable" in compact \
                    and "Useexistingmodel" in compact and process.poll() is None:
                os.write(master, b"\x1b[B\r")
                kept_existing_model = True
            events = [value.get("event") for value in notification_values(raw)]
            if phase == "initial" and {"session_start", "prompt_submit"}.issubset(events) \
                    and completed_at is None:
                completed_at = time.monotonic()
            if phase == "initial" and not disconnected and completed_at is not None \
                    and time.monotonic() - completed_at > 1.0 and process.poll() is None:
                if disconnect_method == "explicit-detach":
                    os.write(master, b"\x02d")
                elif disconnect_method == "abrupt-disconnect":
                    os.kill(process.pid, signal.SIGKILL)
                else:
                    raise AssertionError("未知断连方式")
                disconnected = True
                completed_at = None
            if phase == "reattach" and not prompt_sent \
                    and time.monotonic() - started > 1.5 and process.poll() is None:
                os.write(master, PROMPT.encode() + b"\r")
                prompt_sent = True
                prompt_sent_at = time.monotonic()
                submit_key_attempts.append("carriage-return")
            if phase == "reattach" and prompt_sent_at is not None \
                    and "prompt_submit" not in events and process.poll() is None:
                elapsed = time.monotonic() - prompt_sent_at
                if elapsed > 1.0 and "line-feed" not in submit_key_attempts:
                    os.write(master, b"\n")
                    submit_key_attempts.append("line-feed")
                elif elapsed > 2.0 and "kitty-enter" not in submit_key_attempts:
                    os.write(master, b"\x1b[13;1u")
                    submit_key_attempts.append("kitty-enter")
            if phase == "reattach" and not quit_sent \
                    and "prompt_submit" in events and completed_at is None:
                completed_at = time.monotonic()
            if phase == "reattach" and not quit_sent and completed_at is not None \
                    and time.monotonic() - completed_at > 1.0 and process.poll() is None:
                os.write(master, b"\x03\x03")
                quit_sent = True
                completed_at = None
            if process.poll() is not None and selector.get_map():
                time.sleep(0.05)
        process.wait(timeout=3)
    except Exception as error:
        capture_error = {"type": type(error).__name__, "message": str(error)}
    finally:
        selector.close()
        if process.poll() is None:
            process.kill()
            process.wait(timeout=3)
        os.close(master)
    return {
        "phase": phase,
        "disconnect_method": disconnect_method,
        "ssh_exit": process.returncode,
        "disconnect_initiated": disconnected,
        "trust_selected_in_native_tui": trusted,
        "kept_existing_model_in_native_tui": kept_existing_model,
        "prompt_sent_after_reattach": prompt_sent,
        "submit_key_attempts": submit_key_attempts,
        "capture_error": capture_error,
        "terminal_query_responses": {key.hex(): value for key, value in answered.items()},
        "ssh_stdout_base64": base64.b64encode(stream).decode(),
        "ssh_stdout_sha256": hashlib.sha256(stream).hexdigest(),
        "ssh_stdout_bytes": len(stream),
        "notifications": notification_values(bytes(stream)),
    }


def status_is_detached_and_alive(status, case_name):
    session = "reconnect-" + case_name
    return (
        status["session_exit"] == 0
        and status["pane_exit"] == 0
        and status["client_exit"] == 0
        and status["sessions"] == [session + "|0"]
        and status["clients"] == []
        and len(status["panes"]) == 1
        and status["panes"][0].split("|")[1] == "0"
    )


def validate_case(case, reports_before_reattach, reports, token):
    initial = case["initial"]
    reattach = case["reattach"]
    require(initial["capture_error"] is None and reattach["capture_error"] is None,
            "SSH/tmux 捕获失败")
    require(initial["disconnect_initiated"], "首个 SSH 客户端未执行断连")
    if case["method"] == "explicit-detach":
        require(initial["ssh_exit"] == 0, "显式 tmux detach 后 SSH 未正常结束")
    else:
        require(initial["ssh_exit"] == -signal.SIGKILL, "SSH 客户端未被真实 SIGKILL")
    require(status_is_detached_and_alive(case["after_disconnect"], case["method"]),
            "断连后 tmux 会话未保持无客户端且 pane 存活")
    require(reattach["ssh_exit"] == 0 and reattach["prompt_sent_after_reattach"],
            "重连 SSH/tmux 未正常提交提示并退出")
    require(set(reports_before_reattach) == {"SessionStart", "UserPromptSubmit", "native-start"},
            "断连后、重连前缺少原生 Codex 报告")
    require(set(reports) == {"SessionStart", "UserPromptSubmit", "native-start", "native-exit"},
            "重连执行缺少原生 Codex 报告")
    start = reports_before_reattach["SessionStart"]["input"]
    first_submit = reports_before_reattach["UserPromptSubmit"]["input"]
    submit = reports["UserPromptSubmit"]["input"]
    require(start["session_id"] == submit["session_id"] and submit["turn_id"],
            "重连前后未保持同一原生 session 或缺少 turn")
    require(first_submit["session_id"] == submit["session_id"]
            and first_submit["turn_id"] != submit["turn_id"],
            "相同提示的两次真实提交必须使用不同原生 turn")
    require(submit["prompt"] == PROMPT and reports["UserPromptSubmit"]["block_output"] == {
        "continue": False, "stopReason": token}, "重连后的真实 prompt hook 未阻断")
    require(reports["native-exit"] == {
        "pid": reports["native-start"]["codex_pid"], "exit_code": 0, "forced": False},
        "重连后的 Codex 未正常退出")
    initial_events = [value["event"] for value in initial["notifications"]
                      if value.get("session_id") == start["session_id"]]
    reattach_events = [value["event"] for value in reattach["notifications"]
                       if value.get("session_id") == start["session_id"]]
    require(initial_events == ["session_start", "prompt_submit"],
            "初始 SSH 收到的原生事件不符")
    require(reattach_events == ["prompt_submit"],
            "重连 SSH 必须只收到新 prompt，不能重放旧 session_start")
    prompt = next(value for value in reattach["notifications"]
                  if value.get("event") == "prompt_submit")
    require(prompt.get("turn_id") == submit["turn_id"], "重连通知不能关联原生 turn")
    case.update(
        native_session_id=start["session_id"],
        initial_native_turn_id=first_submit["turn_id"],
        native_turn_id=submit["turn_id"],
        native_cli_hook_triggered=True,
        tmux_survived_without_clients=True,
        reattach_prompt_transport_received=True,
        old_session_start_replayed_on_reattach=False,
        passed=True,
    )


def build_product_input(report):
    cases = []
    for case in report["cases"]:
        raw = base64.b64decode(case["initial"]["ssh_stdout_base64"])
        cases.append({
            "mode": "tmux-on",
            "ssh_stdout_base64": base64.b64encode(raw).decode(),
            "ssh_stdout_sha256": hashlib.sha256(raw).hexdigest(),
            "native_session_id": case["native_session_id"],
            "native_turn_id": case["initial_native_turn_id"],
            "disconnect_method": case["method"],
            "scope": "initial SSH bytes before detach; reattach is retained separately",
        })
    return {
        "source_commit": report["source_commit"],
        "source_tree_dirty": report["source_tree_dirty"],
        "host_alias": report["host_alias"],
        "remote_inputs": report["remote_inputs"],
        "official_codex_hook_control_tty_compatible": True,
        "remote_transport_passed": True,
        "cases": cases,
    }


def build_reattach_product_input(report):
    cases = []
    for case in report["cases"]:
        initial = base64.b64decode(case["initial"]["ssh_stdout_base64"])
        reattach = base64.b64decode(case["reattach"]["ssh_stdout_base64"])
        session_start = None
        for match in OSC.finditer(initial):
            value = json.loads(match.group(1))
            if value.get("event") == "session_start" \
                    and value.get("session_id") == case["native_session_id"]:
                session_start = match.group(0)
                break
        require(session_start is not None, "初始真实字节缺少 session_start 帧")
        raw = session_start + reattach
        cases.append({
            "mode": "tmux-on",
            "ssh_stdout_base64": base64.b64encode(raw).decode(),
            "ssh_stdout_sha256": hashlib.sha256(raw).hexdigest(),
            "native_session_id": case["native_session_id"],
            "native_turn_id": case["native_turn_id"],
            "disconnect_method": case["method"],
            "input_construction": {
                "session_start": "exact OSC frame selected from initial real SSH stream",
                "prompt_submit": "complete real reattach SSH stream",
                "complete_live_product_stream": False,
                "offline_product_parser_only": True,
                "session_start_frame_sha256": hashlib.sha256(session_start).hexdigest(),
                "reattach_stream_sha256": hashlib.sha256(reattach).hexdigest(),
            },
        })
    return {
        "source_commit": report["source_commit"],
        "source_tree_dirty": report["source_tree_dirty"],
        "host_alias": report["host_alias"],
        "remote_inputs": report["remote_inputs"],
        "official_codex_hook_control_tty_compatible": True,
        "remote_transport_passed": True,
        "scope": "selected actual session_start frame plus complete reattach SSH stream",
        "complete_live_product_stream": False,
        "offline_product_parser_only": True,
        "cases": cases,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True)
    parser.add_argument("--remote-root", required=True)
    parser.add_argument("--baseline-private", required=True, type=Path)
    parser.add_argument("--private-output", required=True, type=Path)
    parser.add_argument("--safe-output", required=True, type=Path)
    parser.add_argument("--product-private-output", required=True, type=Path)
    parser.add_argument("--reattach-product-private-output", required=True, type=Path)
    args = parser.parse_args()
    require(SAFE_HOST.fullmatch(args.host) is not None, "SSH 别名包含不安全字符")
    validate_remote_root(args.remote_root)
    require(not args.private_output.exists() and not args.safe_output.exists()
            and not args.product_private_output.exists()
            and not args.reattach_product_private_output.exists(), "输出收据已经存在")
    require(len({args.private_output, args.safe_output, args.product_private_output,
                 args.reattach_product_private_output}) == 4, "输出收据路径必须互不相同")
    baseline_bytes = args.baseline_private.read_bytes()
    baseline = json.loads(baseline_bytes)
    require(baseline["passed"] and baseline["host_alias"] == args.host
            and baseline["remote_inputs"]["root"] == args.remote_root
            and baseline["remote_inputs"]["codex_version"] == "codex-cli 0.155.1",
            "基线私有收据不属于当前固定远端")
    require(baseline["official_codex_hook_control_tty_compatible"], "基线正式插件未通过")
    tokens = {case["mode"]: baseline["cases"][index]["native"]["UserPromptSubmit"]
              ["block_output"]["stopReason"] for index, case in enumerate(baseline["cases"])}
    repository = Path(__file__).resolve().parents[2]
    source_commit = subprocess.check_output(
        ["git", "-C", repository, "rev-parse", "HEAD"], text=True).strip()
    source_tree_dirty = bool(subprocess.check_output(
        ["git", "-C", repository, "status", "--porcelain"], text=True))
    report = {
        "scope": "Codex 0.155.1 SSH disconnect and tmux detach/reattach",
        "source_commit": source_commit,
        "source_tree_dirty": source_tree_dirty,
        "host_alias": args.host,
        "baseline_private_receipt_sha256": hashlib.sha256(baseline_bytes).hexdigest(),
        "remote_inputs": baseline["remote_inputs"],
        "credentials_read_or_copied": False,
        "product_terminal_view_ui_verified": False,
        "cases": [],
    }
    ssh = ssh_base(args.host)
    with tempfile.TemporaryDirectory(prefix="infinishell-reconnect-probe-") as temporary:
        staging = Path(temporary)
        (staging / "reconnect_driver.py").write_text(
            REMOTE_SOURCE.replace("__PROMPT__", repr(PROMPT)), encoding="utf-8")
        transfer_tree(ssh, staging, args.remote_root)
    try:
        for method in CASES:
            require(SAFE_CASE.fullmatch(method) is not None, "内部 case 名无效")
            remote_json(ssh, "python3", args.remote_root + "/reconnect_driver.py",
                        args.remote_root, "prepare", method)
            case = {"method": method, "passed": False}
            report["cases"].append(case)
            initial_command = remote_command(
                ssh, "python3", args.remote_root + "/reconnect_driver.py", args.remote_root,
                "start-attached", method, force_tty=True)
            case["initial"] = capture(initial_command, "initial", method)
            time.sleep(0.5)
            case["after_disconnect"] = remote_json(
                ssh, "python3", args.remote_root + "/reconnect_driver.py",
                args.remote_root, "status", method)
            case["native_before_reattach"] = remote_json(
                ssh, "python3", args.remote_root + "/reconnect_driver.py",
                args.remote_root, "reports", method)
            attach_command = remote_command(
                ssh, "python3", args.remote_root + "/reconnect_driver.py", args.remote_root,
                "attach", method, force_tty=True)
            case["reattach"] = capture(attach_command, "reattach")
            reports = remote_json(ssh, "python3", args.remote_root + "/reconnect_driver.py",
                                  args.remote_root, "reports", method)
            case["native"] = reports
            validate_case(case, case["native_before_reattach"], reports, tokens["tmux-on"])
        report["ssh_disconnect_tmux_survival_verified"] = all(
            case["tmux_survived_without_clients"] for case in report["cases"])
        report["tmux_detach_reattach_verified"] = all(case["passed"] for case in report["cases"])
        report["old_event_transport_replay_observed"] = any(
            case["old_session_start_replayed_on_reattach"] for case in report["cases"])
        report["old_event_transport_boundary_verified"] = not report[
            "old_event_transport_replay_observed"]
        report["same_identity_duplicate_actual_cli_triggered"] = False
        report["out_of_order_actual_cli_triggered"] = False
        report["duplicate_and_out_of_order_result"] = "evidence_insufficient"
        report["duplicate_and_out_of_order_reason"] = (
            "Codex hooks expose no native sequence or event id; the official CLI emitted only "
            "session_start then prompt_submit, so exact-identity duplicate and out-of-order "
            "events were not fabricated."
        )
        report["lifecycle_passed"] = (
            report["ssh_disconnect_tmux_survival_verified"]
            and report["tmux_detach_reattach_verified"]
            and report["old_event_transport_boundary_verified"]
        )
        report["passed"] = False
        report["result"] = "partial_evidence_duplicate_out_of_order_insufficient"
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        report["lifecycle_passed"] = False
        report["passed"] = False
        report["result"] = "lifecycle_failed"
    finally:
        for method in CASES:
            subprocess.run(remote_command(
                ssh, "python3", args.remote_root + "/reconnect_driver.py", args.remote_root,
                "kill", method), capture_output=True, timeout=15)
    write_exclusive(args.private_output, report)
    safe = sanitize(report, args.remote_root, tokens)
    safe["private_receipt"] = {
        "path_recorded": False,
        "sha256": digest(args.private_output),
        "contains_raw_ssh_bytes": True,
    }
    safe["safe_receipt_contains_raw_ssh_bytes"] = False
    safe["auth_material_in_receipt"] = False
    write_exclusive(args.safe_output, safe)
    if report.get("lifecycle_passed"):
        write_exclusive(args.product_private_output, build_product_input(report))
        write_exclusive(
            args.reattach_product_private_output,
            build_reattach_product_input(report),
        )
    print(json.dumps({
        "lifecycle_passed": report.get("lifecycle_passed", False),
        "overall_passed": report["passed"],
        "safe_output": str(args.safe_output),
        "private_sha256": safe["private_receipt"]["sha256"],
        "product_private_output_written": args.product_private_output.exists(),
        "reattach_product_private_output_written": (
            args.reattach_product_private_output.exists()),
        "error": report.get("error"),
    }, ensure_ascii=False))
    if not report.get("lifecycle_passed"):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
