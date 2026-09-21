#!/usr/bin/env python3
"""在异机 SSH/tmux 中触发固定 Codex 原生 hook；原始字节与安全收据分开保存。

这个探针只证明真实远端 Codex、SSH、tmux 与 OSC 传输。产品接收链由读取
``--private-output`` 的 Rust 定向测试独立验证；两者都不等同于 GUI 生命周期验收。
"""

import argparse
import base64
import errno
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import selectors
import shlex
import shutil
import struct
import subprocess
import sys
import tarfile
import termios
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
from probe_codex_hook_transport import digest, require, tree
from probe_codex_windows_hooks import NativeRecorder


PROMPT = "INFINISHELL_REMOTE_SSH_TMUX_NATIVE_HOOK_PROBE_DO_NOT_RUN_MODEL"
PLUGIN_ID = "ssh-transport-probe@infinishell-ssh-transport-probe"
MODES = ("direct", "tmux-on", "tmux-off")
OSC = re.compile(rb"\x1b\]777;notify;warp://cli-agent;([^\x07]*)\x07")
SAFE_REMOTE_ROOT = re.compile(
    r"^/root/\.cache/infinishell-parity-ssh-tmux\.[A-Za-z0-9_-]+$"
)
SAFE_HOST = re.compile(r"^[A-Za-z0-9_.-]+$")


HOOK_SOURCE = r'''
import json,os,pathlib,subprocess,sys
payload=json.load(sys.stdin)
event=payload['hook_event_name']
expected={'session':'SessionStart','prompt':'UserPromptSubmit'}[sys.argv[1]]
if event!=expected:
    raise SystemExit('unexpected hook event')
if event=='UserPromptSubmit' and payload.get('prompt')!=__PROMPT__:
    raise SystemExit('unexpected prompt')
root=pathlib.Path(os.environ['PLUGIN_ROOT'])
private_root=pathlib.Path(__REMOTE_ROOT__)
mode=pathlib.Path(payload['cwd']).name
tokens=__TOKENS__
if mode not in tokens or pathlib.Path(payload['cwd']).parent != private_root/'work':
    raise SystemExit('unexpected private work directory')
script='on-session-start.sh' if event=='SessionStart' else 'on-prompt-submit.sh'
def process_info(pid):
    text=pathlib.Path('/proc')/str(pid)/'stat'
    value=text.read_text(); end=value.rfind(')'); fields=value[end+2:].split()
    return {'pid':pid,'name':value[value.find('(')+1:end],'ppid':int(fields[1]),
            'pgid':int(fields[2]),'sid':int(fields[3]),'tty_nr':int(fields[4])}
ancestry=[]; ancestor=os.getpid()
for _ in range(8):
    item=process_info(ancestor); ancestry.append(item); ancestor=item['ppid']
    if ancestor<=1: break
tool_env=os.environ.copy()
tool_env.update(PATH=str(private_root/'pkg/usr/bin')+':/usr/local/bin:/usr/bin:/bin',
                LD_LIBRARY_PATH=str(private_root/'pkg/usr/lib/x86_64-linux-gnu'),
                WARP_CLI_AGENT_PROTOCOL_VERSION='1',WARP_CLIENT_VERSION='remote-live-probe',
                TERM_PROGRAM='WarpTerminal')
try:
    control_tty=os.open('/dev/tty',os.O_WRONLY|os.O_NOCTTY)
except OSError as error:
    hook_tty={'opened':False,'errno':error.errno}
else:
    hook_tty={'opened':True,'isatty':os.isatty(control_tty)}; os.close(control_tty)
original=subprocess.run(['bash',str(root/'reference/scripts'/script)],input=json.dumps(payload),
                        capture_output=True,text=True,timeout=8,env=tool_env)
codex_parent=next(item for item in ancestry if item['name']=='codex')
record={'input':payload,'pid':os.getpid(),'ppid':os.getppid(),'pgid':os.getpgrp(),'sid':os.getsid(0),
        'stdio_isatty':{str(fd):os.isatty(fd) for fd in (0,1,2)},'ancestry':ancestry,
        'hook_control_tty':hook_tty,'codex_parent':codex_parent,
        'official_reference_exit':original.returncode,
        'official_reference_stdout':original.stdout,'official_reference_stderr':original.stderr,
        'diagnostic_parent_tty_adapter':False,
        'reference_script':script,'reference_exit':original.returncode,
        'reference_stdout':original.stdout,'reference_stderr':original.stderr,
        'environment':{key:os.environ.get(key) for key in ('PLUGIN_ROOT','TMUX','TMUX_PANE',
            'SSH_TTY','WARP_CLI_AGENT_PROTOCOL_VERSION','WARP_CLIENT_VERSION','TERM_PROGRAM')}}
if event=='UserPromptSubmit':
    record['block_output']={'continue':False,'stopReason':tokens[mode]}
output=private_root/'reports'/mode/(event+'.json')
output.write_text(json.dumps(record),encoding='utf-8')
if original.returncode:
    raise SystemExit(original.returncode)
if event=='UserPromptSubmit':
    print(json.dumps(record['block_output']),flush=True)
'''.replace("__PROMPT__", repr(PROMPT))


REMOTE_SOURCE = r'''
import hashlib,json,os,pathlib,shlex,subprocess,sys
root=pathlib.Path(sys.argv[1]).resolve()
config=json.loads((root/'remote-probe.json').read_text())
action=sys.argv[2]
codex=(root/config['codex_relative']).resolve()
tmux=(root/config['tmux_relative']).resolve()
jq=(root/config['jq_relative']).resolve()
libraries=(root/config['library_relative']).resolve()
home=root/'home'
common={'HOME':str(home),'CODEX_HOME':str(home/'.codex'),'SHELL':'/bin/bash','TERM':'xterm-256color',
        'PATH':str(jq.parent)+':/usr/local/bin:/usr/bin:/bin','LD_LIBRARY_PATH':str(libraries),
        'WARP_CLI_AGENT_PROTOCOL_VERSION':'1','WARP_CLIENT_VERSION':'remote-live-probe',
        'TERM_PROGRAM':'WarpTerminal','GIT_CONFIG_NOSYSTEM':'1','GIT_TERMINAL_PROMPT':'0'}
def environment(mode):
    value=os.environ.copy(); value.update(common)
    value.update(INFINISHELL_SSH_PROBE_PYTHON='/usr/bin/python3',
        INFINISHELL_SSH_PROBE_REPORTS=str(root/'reports'/mode),
        INFINISHELL_SSH_PROBE_TOKEN=config['tokens'][mode])
    return value
if action=='verify':
    def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
    packages={path.name:sha(path) for path in sorted((root/'debs').glob('*.deb'))}
    values={'root':str(root),'home':str(pathlib.Path.home().resolve()),'kernel':os.uname().sysname,
        'architecture':os.uname().machine,'codex_path':str(codex),'codex_sha256':sha(codex),
        'codex_package_sha256':sha(root/'codex.tgz'),'codex_version':subprocess.check_output(
            [str(codex),'--version'],env=common,text=True).strip(),'tmux_path':str(tmux),
        'tmux_sha256':sha(tmux),'tmux_version':subprocess.check_output(
            [str(tmux),'-V'],env=common,text=True).strip(),'jq_path':str(jq),
        'jq_version':subprocess.check_output([str(jq),'--version'],env=common,text=True).strip(),
        'package_sha256':packages}
    print(json.dumps(values)); raise SystemExit(0)
if action=='install':
    (home/'.codex').mkdir(parents=True,exist_ok=True)
    (root/'work').mkdir(exist_ok=True); (root/'reports').mkdir(exist_ok=True)
    for mode in config['modes']:
        (root/'work'/mode).mkdir(exist_ok=True); (root/'reports'/mode).mkdir(exist_ok=True)
    (home/'.codex/config.toml').write_text('cli_auth_credentials_store = "file"\nmodel = "gpt-5.4"\n'
        'model_provider = "isolated_ssh_probe"\napproval_policy = "on-request"\nsandbox_mode = "read-only"\n'
        '[model_providers.isolated_ssh_probe]\nname = "Isolated SSH probe"\n'
        'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
        'requires_openai_auth = false\nsupports_websockets = false\n')
    records=[]
    for arguments in (['plugin','marketplace','add',str(root/'marketplace'),'--json'],
                      ['plugin','add',config['plugin_id'],'--json']):
        value=subprocess.run([str(codex),*arguments],env=common,cwd=root/'work',
            capture_output=True,text=True,timeout=30)
        records.append({'arguments':arguments,'exit_code':value.returncode,
            'stdout':value.stdout,'stderr':value.stderr})
        if value.returncode: raise SystemExit('plugin install failed')
    print(json.dumps(records)); raise SystemExit(0)
if action=='app-server':
    os.chdir(root/'work')
    os.execve(codex,[str(codex),'app-server','--stdio'],common)
if action=='reports':
    mode=sys.argv[3]
    values={path.stem:json.loads(path.read_text()) for path in sorted((root/'reports'/mode).glob('*.json'))}
    print(json.dumps(values)); raise SystemExit(0)
if action=='kill-tmux':
    for mode in ('tmux-on','tmux-off'):
        subprocess.run([str(tmux),'-S',config['sockets'][mode],'kill-server'],env=common,
                       capture_output=True,timeout=5)
    raise SystemExit(0)
if action not in ('run','run-inner'):
    raise SystemExit('unknown action')
mode=sys.argv[3]
if mode not in config['modes']:
    raise SystemExit('unknown mode')
env=environment(mode); work=root/'work'/mode; os.chdir(work)
if action=='run' and mode!='direct':
    tmux_config=root/(mode+'.conf')
    tmux_config.write_text('set -g status off\nset -g default-shell /bin/sh\nset -g allow-passthrough '
                           +('on' if mode=='tmux-on' else 'off')+'\n')
    command=shlex.join(['/usr/bin/python3',__file__,str(root),'run-inner',mode])
    os.execve(tmux,[str(tmux),'-f',str(tmux_config),'-S',config['sockets'][mode],
        'new-session','-s','parity','-n','probe','-x','140','-y','40',command],env)
if not all(os.isatty(fd) for fd in (0,1,2)):
    raise SystemExit('Codex must inherit SSH/tmux PTY')
reports=root/'reports'/mode
terminal={'pid':os.getpid(),'pgid':os.getpgrp(),'sid':os.getsid(0),'tty':os.ttyname(0),
          'tmux':os.environ.get('TMUX'),'tmux_pane':os.environ.get('TMUX_PANE')}
process=subprocess.Popen([str(codex),'--no-alt-screen',__PROMPT__],env=env,cwd=work)
terminal['codex_pid']=process.pid
(reports/'native-start.json').write_text(json.dumps(terminal))
forced=False
try:
    code=process.wait(timeout=45)
except subprocess.TimeoutExpired:
    forced=True; process.terminate()
    try: code=process.wait(timeout=3)
    except subprocess.TimeoutExpired: process.kill(); code=process.wait(timeout=3)
(reports/'native-exit.json').write_text(json.dumps({'pid':process.pid,'exit_code':code,'forced':forced}))
raise SystemExit(code if code>=0 else 128-code)
'''.replace("__PROMPT__", repr(PROMPT))


def ssh_base(host):
    require(SAFE_HOST.fullmatch(host) is not None, "SSH 别名包含不安全字符")
    return ["ssh", "-F", str(Path.home() / ".ssh/config"), "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=8", "-o", "ServerAliveInterval=5",
            "-o", "ServerAliveCountMax=2", host]


def validate_remote_root(remote_root):
    require(
        SAFE_REMOTE_ROOT.fullmatch(remote_root) is not None,
        "远端根目录不在受控私有缓存范围",
    )


def remote_command(ssh, *arguments, force_tty=False):
    require(arguments, "远端命令不能为空")
    command = shlex.join([str(argument) for argument in arguments])
    if force_tty:
        return [*ssh[:-1], "-tt", ssh[-1], command]
    return [*ssh, command]


def remote_json(ssh, *arguments, input_bytes=None, timeout=30):
    result = subprocess.run(remote_command(ssh, *arguments), input=input_bytes,
                            capture_output=True, timeout=timeout)
    require(result.returncode == 0,
            "远端命令失败：" + result.stderr.decode("utf-8", errors="replace")[-1000:])
    return json.loads(result.stdout)


def write_exclusive(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as output:
        json.dump(value, output, ensure_ascii=False, indent=2)
        output.write("\n")


def transfer_tree(ssh, source, remote_root):
    validate_remote_root(remote_root)
    preflight = remote_json(
        ssh,
        "python3",
        "-c",
        (
            "import json,pathlib,sys; "
            "path=pathlib.Path(sys.argv[1]); "
            "resolved=path.resolve(strict=True); "
            "(path.is_dir() and resolved == path) or sys.exit('unsafe remote root'); "
            "print(json.dumps({'root':str(resolved)}))"
        ),
        remote_root,
    )
    require(preflight == {"root": remote_root}, "远端私有根目录不是现有真实目录")
    payload = io.BytesIO()
    with tarfile.open(fileobj=payload, mode="w") as archive:
        for path in sorted(source.rglob("*")):
            require(not path.is_symlink(), "拒绝上传符号链接")
            archive.add(path, arcname=path.relative_to(source), recursive=False)
    result = subprocess.run(remote_command(ssh, "tar", "-xf", "-", "-C", remote_root),
                            input=payload.getvalue(), capture_output=True, timeout=30)
    require(result.returncode == 0,
            "上传私有探针失败：" + result.stderr.decode("utf-8", errors="replace")[-1000:])


def prepare_tree(root, repository, remote_root, args, tokens):
    reference = repository / "app/assets/bundled/cli-agent-plugins/codex"
    metadata = json.loads((reference / "SOURCE_METADATA.json").read_text())
    expected = {name.removeprefix("plugins/warp/"): value["sha256"]
                for name, value in metadata["files"].items() if name.startswith("plugins/warp/")}
    require(tree(reference / "source/plugins/warp") == expected, "受控参考插件摘要不符")
    plugin = root / "marketplace/plugins/ssh-transport-probe"
    (plugin / ".codex-plugin").mkdir(parents=True)
    (plugin / "hooks").mkdir()
    shutil.copytree(reference / "source/plugins/warp", plugin / "reference")
    hook_source = (HOOK_SOURCE.replace("__REMOTE_ROOT__", repr(remote_root))
                   .replace("__TOKENS__", repr(tokens)))
    (plugin / "native_hook.py").write_text(hook_source, encoding="utf-8")
    (plugin / ".codex-plugin/plugin.json").write_text(json.dumps(
        {"name": "ssh-transport-probe", "version": "0.1.0"}))
    command = '/usr/bin/python3 "$PLUGIN_ROOT/native_hook.py" '
    (plugin / "hooks/hooks.json").write_text(json.dumps({"hooks": {
        event: [{"hooks": [{"type": "command", "command": command + mode, "timeout": 10}]}]
        for event, mode in (("SessionStart", "session"), ("UserPromptSubmit", "prompt"))}}))
    marketplace = root / "marketplace/.agents/plugins"
    marketplace.mkdir(parents=True)
    (marketplace / "marketplace.json").write_text(json.dumps({
        "name": "infinishell-ssh-transport-probe", "plugins": [{
            "name": "ssh-transport-probe", "source": "./plugins/ssh-transport-probe",
            "version": "0.1.0", "policy": {"installation": "AVAILABLE",
            "authentication": "ON_INSTALL"}}]}))
    (root / "remote_driver.py").write_text(REMOTE_SOURCE, encoding="utf-8")
    socket_prefix = "/tmp/isp-" + hashlib.sha256(remote_root.encode()).hexdigest()[:12]
    configuration = {"codex_relative": args.codex_relative,
        "tmux_relative": args.tmux_relative, "jq_relative": args.jq_relative,
        "library_relative": args.library_relative, "plugin_id": PLUGIN_ID,
        "modes": list(MODES), "tokens": tokens,
        "sockets": {mode: socket_prefix + "-" + mode + ".sock"
                    for mode in MODES if mode != "direct"}}
    (root / "remote-probe.json").write_text(json.dumps(configuration))
    return expected, command


def authorize(ssh, remote_root, command, report):
    events = report.setdefault("authorization_rpc", [])
    recorder = NativeRecorder(remote_command(
        ssh, "python3", remote_root + "/remote_driver.py", remote_root, "app-server"),
        os.environ.copy(),
        Path.cwd(), events)
    try:
        initialized = recorder.rpc("initialize", {"clientInfo": {
            "name": "infinishell_remote_ssh_probe", "version": "0.1.0"},
            "capabilities": {"experimentalApi": True}}, 1)
        require(PurePosixPath(initialized["codexHome"]) == PurePosixPath(remote_root) / "home/.codex",
                "原生 Codex Home 越过远端私有域")
        recorder.send({"method": "initialized"})
        hooks = recorder.rpc("hooks/list", {"cwds": [remote_root + "/work"]}, 2)
        selected = [hook for group in hooks["data"] for hook in group["hooks"]]
        require(len(selected) == 2, "必须只有两个原生测试 hook")
        for index, hook in enumerate(selected):
            require(hook["pluginId"] == PLUGIN_ID and hook["trustStatus"] == "untrusted"
                    and hook["command"] in {command + "session", command + "prompt"}
                    and PurePosixPath(hook["sourcePath"]).is_relative_to(
                        PurePosixPath(remote_root) / "home/.codex"),
                    "拒绝授权非本次精确定义")
            result = recorder.rpc("config/value/write", {
                "keyPath": "hooks.state." + json.dumps(hook["key"]),
                "value": {"enabled": True, "trusted_hash": hook["currentHash"]},
                "mergeStrategy": "upsert",
                "filePath": remote_root + "/home/.codex/config.toml"}, 10 + index)
            require(result["status"] == "ok", "原生测试授权未确认")
        report["authorized_hooks"] = [{key: hook.get(key) for key in
            ("pluginId", "eventName", "command", "sourcePath", "trustStatus", "currentHash")}
            for hook in selected]
    finally:
        recorder.close()
    require(recorder.process.returncode == 0, "远端原生授权进程没有正常退出")


def capture_case(command, token, timeout=40):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
    bootstrap = ("import fcntl,os,sys,termios; fd=int(sys.argv[1]); os.setsid(); "
                 "fcntl.ioctl(fd,termios.TIOCSCTTY,0); "
                 "[os.dup2(fd,target) for target in (0,1,2)]; os.close(fd); "
                 "os.execvp(sys.argv[2],sys.argv[2:])")
    process = subprocess.Popen([sys.executable, "-c", bootstrap, str(slave), *command],
                               pass_fds=(slave,))
    os.close(slave)
    selector = selectors.DefaultSelector()
    stream = bytearray()
    selector.register(master, selectors.EVENT_READ)
    answered = {}
    trusted = False
    kept_existing_model = False
    quit_started = None
    second_quit = False
    deadline = time.monotonic() + timeout
    capture_error = None
    try:
        while selector.get_map():
            require(time.monotonic() < deadline, "远端原生 Codex TUI 超时")
            for _, _ in selector.select(0.05):
                try:
                    data = os.read(master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b""
                if data:
                    stream.extend(data)
                    require(len(stream) <= 4 * 1024 * 1024, "远端 SSH 输出超过限制")
                else:
                    selector.unregister(master)
            raw = bytes(stream)
            for query, answer in ((b"\x1b[6n", b"\x1b[1;1R"),
                                  (b"\x1b[c", b"\x1b[?1;2c"),
                                  (b"\x1b[>c", b"\x1b[>0;0;0c"),
                                  (b"\x1b[?u", b"\x1b[?0u"),
                                  (b"\x1b]10;?\x1b\\", b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\"),
                                  (b"\x1b]11;?\x1b\\", b"\x1b]11;rgb:0000/0000/0000\x1b\\")):
                count = raw.count(query)
                if count > answered.get(query, 0) and process.poll() is None:
                    os.write(master, answer * (count - answered.get(query, 0)))
                    answered[query] = count
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", raw).decode(
                "utf-8", errors="replace")
            compact = re.sub(r"\s+", "", plain)
            if not trusted and "Doyoutrustthecontentsofthisdirectory?" in compact \
                    and "Yes,continue" in compact:
                os.write(master, b"1\r"); trusted = True
            if not kept_existing_model and "isnolongeravailable" in compact \
                    and "Useexistingmodel" in compact:
                os.write(master, b"\x1b[B\r"); kept_existing_model = True
            if token in plain and quit_started is None:
                quit_started = time.monotonic()
            if quit_started is not None and time.monotonic() - quit_started > 0.8 \
                    and not second_quit and process.poll() is None:
                os.write(master, b"\x03\x03"); second_quit = True
        process.wait(timeout=3)
    except Exception as error:
        capture_error = {"type": type(error).__name__, "message": str(error)}
    finally:
        selector.close()
        if process.poll() is None:
            process.kill(); process.wait(timeout=3)
        os.close(master)
    return {"ssh_exit": process.returncode, "trust_selected_in_native_tui": trusted,
            "kept_existing_model_in_native_tui": kept_existing_model,
            "capture_error": capture_error,
            "terminal_query_responses": {key.hex(): value for key, value in answered.items()},
            "ssh_stdout_base64": base64.b64encode(stream).decode(),
            "ssh_stdout_sha256": hashlib.sha256(stream).hexdigest(),
            "ssh_stdout_bytes": len(stream), "ssh_stderr": "<merged-into-control-pty>"}


def validate_case(mode, case, native, token):
    require(case["capture_error"] is None, "远端原生捕获失败：" + str(case["capture_error"]))
    require(set(native) == {"native-start", "native-exit", "SessionStart", "UserPromptSubmit"},
            "缺少远端真实原生执行或退出记录")
    require(case["ssh_exit"] == 0 and native["native-exit"]["exit_code"] == 0
            and not native["native-exit"]["forced"], "SSH 或远端原生进程没有正常退出")
    submit = native["UserPromptSubmit"]["input"]
    session_id, turn_id = submit["session_id"], submit["turn_id"]
    require(submit["prompt"] == PROMPT and session_id and turn_id, "远端原生提示或关联身份不符")
    for event in ("SessionStart", "UserPromptSubmit"):
        record = native[event]
        require(record["input"]["session_id"] == session_id
                and record["official_reference_exit"] == 0
                and record["official_reference_stderr"] == ""
                and record["reference_exit"] == 0 and record["reference_stderr"] == "",
                "远端参考 hook 执行失败或关联错误")
        require(native["native-start"]["codex_pid"] in {
                    ancestor["pid"] for ancestor in record["ancestry"]}
                and record["ancestry"][0]["pid"] == record["pid"],
                "hook 祖先链不能关联远端原生 Codex")
        require(not record["hook_control_tty"]["opened"]
                and record["codex_parent"]["tty_nr"] != 0
                and not record["diagnostic_parent_tty_adapter"],
                "Codex 0.155.1 hook 的无控制终端特征或正式路径状态不符")
        require(bool(record["environment"]["TMUX"]) == (mode != "direct"), "tmux 环境不符")
    raw = base64.b64decode(case["ssh_stdout_base64"])
    plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", raw).decode("utf-8", errors="replace")
    block = native["UserPromptSubmit"]["block_output"]
    require(block == {"continue": False, "stopReason": token} and token in plain,
            "未取得远端原生 TUI 的阻断结果")
    notifications = []
    for body in OSC.findall(raw):
        try:
            value = json.loads(body)
        except (ValueError, UnicodeError):
            continue
        if value.get("session_id") == session_id:
            notifications.append(value)
    expected = 0 if mode == "tmux-off" else 2
    require(len(notifications) == expected, "SSH 客户端收到的远端真实通知数不符")
    if expected:
        require({value["event"] for value in notifications} == {"session_start", "prompt_submit"},
                "远端通知事件不符")
        prompt = next(value for value in notifications if value["event"] == "prompt_submit")
        require(prompt.get("turn_id") == turn_id and prompt.get("query") == PROMPT,
                "远端 SSH 通知不能关联原生输入")
    case.update(native=native, native_session_id=session_id, native_turn_id=turn_id,
                notifications=notifications, native_cli_hook_triggered=True,
                native_tui_blocked_observed=True, transport_received=bool(expected), passed=True,
                official_reference_transport_compatible=True,
                transport_via_diagnostic_parent_tty_adapter=False,
                result="blocking_observed" if not expected else "transport_received")


def sanitize(value, remote_root, tokens):
    if isinstance(value, dict):
        return {key: sanitize(item, remote_root, tokens) for key, item in value.items()
                if key not in {"ssh_stdout_base64", "reference_stdout", "block_output"}}
    if isinstance(value, list):
        return [sanitize(item, remote_root, tokens) for item in value]
    if isinstance(value, str):
        result = value.replace(remote_root, "<remote-private-root>")
        for token in tokens.values():
            result = result.replace(token, "<probe-token>")
        return result
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True)
    parser.add_argument("--remote-root", required=True)
    parser.add_argument("--codex-relative", default="codex/package/vendor/x86_64-unknown-linux-musl/bin/codex")
    parser.add_argument("--tmux-relative", default="pkg/usr/bin/tmux")
    parser.add_argument("--jq-relative", default="pkg/usr/bin/jq")
    parser.add_argument("--library-relative", default="pkg/usr/lib/x86_64-linux-gnu")
    parser.add_argument("--expected-codex-sha256", required=True)
    parser.add_argument("--expected-codex-package-sha256", required=True)
    parser.add_argument("--expected-tmux-sha256", required=True)
    parser.add_argument("--private-output", required=True, type=Path)
    parser.add_argument("--safe-output", required=True, type=Path)
    args = parser.parse_args()
    validate_remote_root(args.remote_root)
    for relative in (args.codex_relative, args.tmux_relative, args.jq_relative, args.library_relative):
        path = PurePosixPath(relative)
        require(not path.is_absolute() and ".." not in path.parts, "远端相对路径越界")
    require(not args.private_output.exists() and not args.safe_output.exists(), "输出收据已经存在")
    repository = Path(__file__).resolve().parents[2]
    source_commit = subprocess.check_output(
        ["git", "-C", str(repository), "rev-parse", "HEAD"], text=True).strip()
    source_tree_dirty = bool(subprocess.check_output(
        ["git", "-C", str(repository), "status", "--porcelain"], text=True))
    ssh = ssh_base(args.host)
    tokens = {mode: uuid.uuid4().hex for mode in MODES}
    report = {"source_commit": source_commit, "source_tree_dirty": source_tree_dirty,
        "host_alias": args.host, "remote_root_scope": "private_cache",
        "prompt": PROMPT, "credentials_read_or_copied": False,
        "product_ssh_ui_verified": False, "product_osc_parser_verified": False,
        "model_lifecycle_verified": False, "cases": []}
    try:
        with tempfile.TemporaryDirectory(prefix="infinishell-remote-ssh-probe-") as temporary:
            staging = Path(temporary)
            expected_tree, command = prepare_tree(staging, repository, args.remote_root, args, tokens)
            transfer_tree(ssh, staging, args.remote_root)
            verify = remote_json(ssh, "python3", args.remote_root + "/remote_driver.py",
                                 args.remote_root, "verify")
            require(verify["root"] == args.remote_root, "远端私有根目录解析不一致")
            require(verify["codex_sha256"] == args.expected_codex_sha256
                    and verify["codex_package_sha256"] == args.expected_codex_package_sha256
                    and verify["tmux_sha256"] == args.expected_tmux_sha256,
                    "远端固定输入摘要不符")
            require(verify["codex_version"] == "codex-cli 0.155.1", "远端 Codex 版本不符")
            require(verify["tmux_version"].startswith("tmux 3.6"), "远端 tmux 版本不符")
            report["remote_inputs"] = verify
            report["reference_tree_sha256"] = expected_tree
            report["native_install"] = remote_json(
                ssh, "python3", args.remote_root + "/remote_driver.py", args.remote_root, "install")
            authorize(ssh, args.remote_root, command, report)
            for mode in MODES:
                case = {"mode": mode, "passed": False}
                report["cases"].append(case)
                case.update(capture_case(remote_command(
                    ssh, "python3", args.remote_root + "/remote_driver.py", args.remote_root,
                    "run", mode, force_tty=True), tokens[mode]))
                native = remote_json(ssh, "python3", args.remote_root + "/remote_driver.py",
                                     args.remote_root, "reports", mode)
                validate_case(mode, case, native, tokens[mode])
        report["native_cli_hook_triggered"] = all(case["native_cli_hook_triggered"] for case in report["cases"])
        report["direct_transport_passed"] = next(case for case in report["cases"]
                                                   if case["mode"] == "direct")["transport_received"]
        report["tmux_passthrough_passed"] = next(case for case in report["cases"]
                                                  if case["mode"] == "tmux-on")["transport_received"]
        report["tmux_blocking_observed"] = not next(case for case in report["cases"]
                                                    if case["mode"] == "tmux-off")["transport_received"]
        report["official_codex_hook_control_tty_compatible"] = all(
            case["official_reference_transport_compatible"] for case in report["cases"])
        report["remote_transport_passed"] = (
            report["native_cli_hook_triggered"] and report["direct_transport_passed"]
            and report["tmux_passthrough_passed"] and report["tmux_blocking_observed"])
        report["passed"] = (report["remote_transport_passed"]
                            and report["official_codex_hook_control_tty_compatible"])
        report["result"] = "passed" if report["passed"] else "remote_transport_failed"
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        report["passed"] = False
    finally:
        subprocess.run(remote_command(
            ssh, "python3", args.remote_root + "/remote_driver.py", args.remote_root,
            "kill-tmux"), capture_output=True, timeout=15)
    write_exclusive(args.private_output, report)
    safe = sanitize(report, args.remote_root, tokens)
    safe["private_receipt"] = {"path_recorded": False,
        "sha256": digest(args.private_output), "contains_raw_ssh_bytes": True}
    safe["safe_receipt_contains_raw_ssh_bytes"] = False
    write_exclusive(args.safe_output, safe)
    print(json.dumps({"passed": report["passed"], "safe_output": str(args.safe_output),
                      "private_sha256": safe["private_receipt"]["sha256"],
                      "error": report.get("error")}, ensure_ascii=False))
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
