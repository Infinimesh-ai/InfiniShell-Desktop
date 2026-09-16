#!/usr/bin/env python3
"""固定 Codex 原生 TUI 经隔离回环 SSH/tmux 发送通知；固定提示在模型请求前阻断。"""

import argparse
import base64
import getpass
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid

sys.dont_write_bytecode = True
from probe_codex_hook_transport import CODEX_COMMIT, digest, require, tree
from probe_codex_windows_hooks import NativeRecorder

PROMPT = 'INFINISHELL_SSH_TMUX_NATIVE_HOOK_PROBE_DO_NOT_RUN_MODEL'
PLUGIN_ID = 'ssh-transport-probe@infinishell-ssh-transport-probe'
MODES = ('direct', 'tmux-on', 'tmux-off')

# 此脚本仅由本次原生安装的诊断插件调用；参考通知脚本逐字保持受控来源内容。
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
script='on-session-start.sh' if event=='SessionStart' else 'on-prompt-submit.sh'
tty=os.open('/dev/tty',os.O_WRONLY|os.O_NOCTTY)
try:
    terminal={'path':os.ttyname(tty),'isatty':os.isatty(tty),'foreground_pgid':os.tcgetpgrp(tty)}
finally:
    os.close(tty)
result=subprocess.run(['bash',str(root/'reference/scripts'/script)],input=json.dumps(payload),
                      capture_output=True,text=True,timeout=8)
record={'input':payload,'pid':os.getpid(),'ppid':os.getppid(),'pgid':os.getpgrp(),'sid':os.getsid(0),
        'tty':terminal,'reference_script':script,'reference_exit':result.returncode,
        'reference_stdout':result.stdout,'reference_stderr':result.stderr,
        'environment':{key:os.environ.get(key) for key in ('PLUGIN_ROOT','TMUX','TMUX_PANE',
            'WARP_CLI_AGENT_PROTOCOL_VERSION','WARP_CLIENT_VERSION','TERM_PROGRAM')}}
if event=='UserPromptSubmit':
    record['block_output']={'continue':False,'stopReason':os.environ['INFINISHELL_SSH_PROBE_TOKEN']}
output=pathlib.Path(os.environ['INFINISHELL_SSH_PROBE_REPORTS'])/(event+'.json')
output.write_text(json.dumps(record),encoding='utf-8')
if result.returncode:
    raise SystemExit(result.returncode)
if event=='UserPromptSubmit':
    print(json.dumps(record['block_output']),flush=True)
'''.replace('__PROMPT__', repr(PROMPT))

# 直接继承 sshd 或 tmux pane 的 PTY；不调用 openpty/setsid，也不转发旁路通知字节。
REMOTE_SOURCE = r'''
import fcntl,json,os,pathlib,shlex,signal,struct,subprocess,sys,termios,time
config=json.loads(pathlib.Path(sys.argv[1]).read_text())
case=sys.argv[2]
mode=case.removesuffix(':inner')
if mode not in ('direct','tmux-on','tmux-off') or case not in (mode,mode+':inner'):
    raise SystemExit('unknown probe command')
entry=config['cases'][mode]
os.environ.update(entry['environment'])
os.chdir(entry['work'])
if not all(os.isatty(fd) for fd in (0,1,2)):
    raise SystemExit('remote CLI must inherit SSH PTY')
fcntl.ioctl(0,termios.TIOCSWINSZ,struct.pack('HHHH',40,140,0,0))
if mode!='direct' and not case.endswith(':inner'):
    arguments=[config['tmux'],'-f',entry['tmux_config'],'-S',entry['socket'],'new-session',
               '-s','parity','-n','probe','-x','140','-y','40',
               shlex.join([sys.executable,__file__,sys.argv[1],mode+':inner'])]
    os.execv(config['tmux'],arguments)
report=pathlib.Path(entry['reports'])
terminal={'pid':os.getpid(),'pgid':os.getpgrp(),'sid':os.getsid(0),'tty':os.ttyname(0),
          'tmux':os.environ.get('TMUX'),'tmux_pane':os.environ.get('TMUX_PANE')}
process=subprocess.Popen([config['codex'],'--no-alt-screen',__PROMPT__])
terminal['codex_pid']=process.pid
(report/'native-start.json').write_text(json.dumps(terminal))
forced=False
try:
    code=process.wait(timeout=40)
except subprocess.TimeoutExpired:
    forced=True
    process.terminate()
    try:
        code=process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill();code=process.wait(timeout=3)
(report/'native-exit.json').write_text(json.dumps({'pid':process.pid,'exit_code':code,'forced':forced}))
raise SystemExit(code if code>=0 else 128-code)
'''.replace('__PROMPT__', repr(PROMPT))


def native_prepare(args, entry, report):
    directory = Path(entry['root'])
    env = entry['environment']
    home = Path(env['CODEX_HOME'])
    reference = Path(__file__).resolve().parents[2] / 'app/assets/bundled/cli-agent-plugins/codex'
    metadata = json.loads((reference / 'SOURCE_METADATA.json').read_text())
    expected = {name.removeprefix('plugins/warp/'): value['sha256']
                for name, value in metadata['files'].items() if name.startswith('plugins/warp/')}
    require(tree(reference / 'source/plugins/warp') == expected, '受控参考插件摘要不符')
    market = directory / 'marketplace'
    plugin = market / 'plugins/ssh-transport-probe'
    for child in ('.codex-plugin', 'hooks'):
        (plugin / child).mkdir(parents=True)
    shutil.copytree(reference / 'source/plugins/warp', plugin / 'reference')
    (plugin / 'native_hook.py').write_text(HOOK_SOURCE, encoding='utf-8')
    (plugin / '.codex-plugin/plugin.json').write_text(json.dumps(
        {'name': 'ssh-transport-probe', 'version': '0.1.0'}))
    command = '"$INFINISHELL_SSH_PROBE_PYTHON" "$PLUGIN_ROOT/native_hook.py" '
    (plugin / 'hooks/hooks.json').write_text(json.dumps({'hooks': {
        event: [{'hooks': [{'type': 'command', 'command': command + mode, 'timeout': 10}]}]
        for event, mode in [('SessionStart', 'session'), ('UserPromptSubmit', 'prompt')]}}))
    index = market / '.agents/plugins/marketplace.json'
    index.parent.mkdir(parents=True)
    index.write_text(json.dumps({'name': 'infinishell-ssh-transport-probe', 'plugins': [{
        'name': 'ssh-transport-probe', 'source': './plugins/ssh-transport-probe', 'version': '0.1.0',
        'policy': {'installation': 'AVAILABLE', 'authentication': 'ON_INSTALL'}}]}))
    for arguments in (['plugin', 'marketplace', 'add', str(market), '--json'],
                      ['plugin', 'add', PLUGIN_ID, '--json']):
        result = subprocess.run([str(args.codex), *arguments], env=env, cwd=directory,
                                capture_output=True, text=True, timeout=30)
        report.setdefault('native_install', []).append({'arguments': arguments,
            'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
        require(result.returncode == 0, '原生测试插件安装失败')
        if arguments[:2] == ['plugin', 'add']:
            cache = Path(json.loads(result.stdout)['installedPath'])
    require(cache.resolve().is_relative_to(home) and tree(cache) == tree(plugin), '原生安装副本不符')
    events = report.setdefault('authorization_rpc', [])
    recorder = NativeRecorder([str(args.codex), 'app-server', '--stdio'], env, directory, events)
    try:
        initialized = recorder.rpc('initialize', {'clientInfo': {'name': 'infinishell_ssh_probe',
            'version': '0.1.0'}, 'capabilities': {'experimentalApi': True}}, 1)
        require(Path(initialized['codexHome']).resolve() == home, '原生配置作用域越界')
        recorder.send({'method': 'initialized'})
        hooks = recorder.rpc('hooks/list', {'cwds': [entry['work']]}, 2)
        selected = [hook for group in hooks['data'] for hook in group['hooks']]
        require(len(selected) == 2, '必须只有两个原生测试 hook')
        for index, hook in enumerate(selected):
            require(hook['pluginId'] == PLUGIN_ID and hook['trustStatus'] == 'untrusted'
                    and hook['command'] in {command + 'session', command + 'prompt'}
                    and Path(hook['sourcePath']).resolve() == cache / 'hooks/hooks.json',
                    '拒绝授权非本次精确定义')
            result = recorder.rpc('config/value/write', {
                'keyPath': 'hooks.state.' + json.dumps(hook['key']),
                'value': {'enabled': True, 'trusted_hash': hook['currentHash']},
                'mergeStrategy': 'upsert', 'filePath': str(home / 'config.toml')}, 10 + index)
            require(result['status'] == 'ok', '原生测试授权未确认')
    finally:
        recorder.close()
    require(recorder.process.returncode == 0, '原生授权进程没有正常退出')
    report['reference_tree_sha256'] = expected
    report['native_cache'] = str(cache)


def capture_case(command, entry, report, model_requests):
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
    selector = selectors.DefaultSelector()
    streams = {name: bytearray() for name in ('stdout', 'stderr')}
    for name in streams:
        selector.register(getattr(process, name), selectors.EVENT_READ, name)
    answered = {}
    trusted = False
    kept_existing_model = False
    quit_started = None
    sent_second_quit = False
    deadline = time.monotonic() + 35
    try:
        while selector.get_map():
            require(not model_requests, '拒绝 provider 收到 HTTP 请求，不能计无模型通过')
            require(time.monotonic() < deadline, '原生 SSH TUI 超时')
            for key, _ in selector.select(.05):
                data = os.read(key.fileobj.fileno(), 65536)
                if not data:
                    selector.unregister(key.fileobj)
                    continue
                streams[key.data].extend(data)
                require(len(streams[key.data]) <= 4 * 1024 * 1024, 'SSH 输出超过限制')
            raw = bytes(streams['stdout'])
            # 经 SSH 返回标准终端查询应答；不是模型提示或自建 PTY。
            for query, answer in ((b'\x1b[6n', b'\x1b[1;1R'), (b'\x1b[c', b'\x1b[?1;2c'),
                                  (b'\x1b[>c', b'\x1b[>0;0;0c')):
                count = raw.count(query)
                if count > answered.get(query, 0) and process.poll() is None:
                    process.stdin.write(answer * (count - answered.get(query, 0)))
                    process.stdin.flush()
                    answered[query] = count
            plain = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', raw).decode('utf-8', errors='replace')
            compact = re.sub(r'\s+', '', plain)
            if not trusted and 'Doyoutrustthecontentsofthisdirectory?' in compact and 'Yes,continue' in compact:
                require(not any(Path(entry['work']).iterdir()), '仅允许明确选择信任本次空目录')
                require(str(Path(entry['work'])) in plain, '原生目录信任没有显示精确测试路径')
                process.stdin.write(b'1')
                process.stdin.flush()
                trusted = True
            if not kept_existing_model and 'GPT-5.4isnolongeravailable' in compact and 'Useexistingmodel' in compact:
                # 原生首次启动提示；保持已绑定本机拒绝 provider 的原模型，不接受替换。
                process.stdin.write(b'\x1b[B\r')
                process.stdin.flush()
                kept_existing_model = True
            reports = Path(entry['reports'])
            if (reports / 'UserPromptSubmit.json').exists() and quit_started is None:
                quit_started = time.monotonic()
            if quit_started is not None and time.monotonic() - quit_started > 1 and not sent_second_quit:
                process.stdin.write(b'\x03\x03')
                process.stdin.flush()
                sent_second_quit = True
        process.wait(timeout=3)
    finally:
        selector.close()
        report.update(ssh_exit=process.poll(), trust_selected_in_native_tui=trusted,
            kept_existing_model_in_native_tui=kept_existing_model,
            ssh_stdout_base64=base64.b64encode(streams['stdout']).decode(),
            ssh_stdout_sha256=hashlib.sha256(streams['stdout']).hexdigest(),
            ssh_stdout_bytes=len(streams['stdout']), ssh_stderr=streams['stderr'].decode(errors='replace'),
            terminal_query_responses={key.hex(): value for key, value in answered.items()})
        if process.poll() is None:
            process.kill(); process.wait(timeout=3)
        process.stdin.close()


def validate_case(entry, report):
    reports = Path(entry['reports'])
    required = ('native-start', 'native-exit', 'SessionStart', 'UserPromptSubmit')
    require(all((reports / (name + '.json')).is_file() for name in required), '缺少真实原生执行或退出记录')
    report['native'] = {name: json.loads((reports / (name + '.json')).read_text()) for name in required}
    native = report['native']
    require(report['ssh_exit'] == 0 and native['native-exit']['exit_code'] == 0
            and not native['native-exit']['forced'], 'SSH 或原生进程没有正常退出')
    require(report['trust_selected_in_native_tui'], '私有目录未在原生 TUI 明确确认')
    submit = native['UserPromptSubmit']['input']
    session_id, turn_id = submit['session_id'], submit['turn_id']
    require(submit['prompt'] == PROMPT and session_id and turn_id, '原生提示或关联身份不符')
    for event in ('SessionStart', 'UserPromptSubmit'):
        record = native[event]
        require(record['input']['session_id'] == session_id and record['input']['cwd'] == entry['work']
                and record['reference_exit'] == 0 and record['reference_stdout'] == ''
                and record['reference_stderr'] == '', '参考 hook 执行失败或关联错误')
        require(record['tty']['isatty'] and record['tty']['foreground_pgid'] == native['native-start']['pgid']
                and record['pgid'] == native['native-start']['pgid']
                and record['sid'] == native['native-start']['sid'], 'hook 未使用 SSH/tmux 原生控制终端')
        require(record['environment']['PLUGIN_ROOT'] == report['native_cache'], '插件根目录非原生缓存')
        require(bool(record['environment']['TMUX']) == (report['mode'] != 'direct'), 'tmux 环境不符')
    raw = base64.b64decode(report['ssh_stdout_base64'])
    plain = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', raw).decode('utf-8', errors='strict')
    # 原生 TUI 确认已处理本次阻断，不把诊断脚本写出 JSON 当成原生接收确认。
    block = native['UserPromptSubmit']['block_output']
    require(block['continue'] is False and block['stopReason'] == entry['environment']['INFINISHELL_SSH_PROBE_TOKEN']
            and block['stopReason'] in plain
            and 'UserPromptSubmithook(stopped)' in re.sub(r'\s+', '', plain)
            and 'codexresume' + session_id in re.sub(r'\s+', '', plain), '未取得原生 TUI 的本次阻断结果与会话')
    notifications = [json.loads(body) for body in re.findall(
        rb'\x1b\]777;notify;warp://cli-agent;([^\x07]*)\x07', raw)]
    correlated = [value for value in notifications if value.get('session_id') == session_id]
    expected = 0 if report['mode'] == 'tmux-off' else 2
    require(len(correlated) == expected, 'SSH 客户端收到的真实通知数不符')
    if expected:
        require({value['event'] for value in correlated} == {'session_start', 'prompt_submit'}, '通知事件不符')
        prompt = next(value for value in correlated if value['event'] == 'prompt_submit')
        require(prompt.get('turn_id') == turn_id and prompt.get('query') == PROMPT, 'SSH 通知不能关联原生输入')
    report.update(native_session_id=session_id, native_turn_id=turn_id, notifications=correlated,
                  native_cli_hook_triggered=True, native_hook_summary_obtained=False,
                  native_tui_blocked_observed=True,
                  model_http_request_count=0, passed=True,
                  result='blocking_observed' if not expected else 'transport_received')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex', required=True, type=Path)
    parser.add_argument('--expected-codex-sha256', required=True)
    parser.add_argument('--tmux', required=True, type=Path)
    parser.add_argument('--expected-tmux-sha256', required=True)
    parser.add_argument('--sshd', type=Path, default=Path('/usr/sbin/sshd'))
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    require(os.name == 'posix', '只允许 Unix 回环 SSH；不冒充 Windows/ConPTY')
    for name in ('codex', 'tmux', 'sshd'):
        setattr(args, name, getattr(args, name).resolve())
    require(digest(args.codex) == args.expected_codex_sha256, '固定 Codex 摘要不符')
    require(digest(args.tmux) == args.expected_tmux_sha256, '固定 tmux 摘要不符')
    require(subprocess.check_output([str(args.codex), '--version'], text=True).strip() == 'codex-cli 0.147.0', '版本不符')
    model_requests = []
    class RejectModel(BaseHTTPRequestHandler):
        def reject(self):
            model_requests.append({'method': self.command, 'path': self.path})
            self.send_error(503, 'No model is available')
        do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = do_OPTIONS = do_HEAD = reject
        def log_message(self, *_args):
            pass
    provider = ThreadingHTTPServer(('127.0.0.1', 0), RejectModel)
    provider_thread = threading.Thread(target=provider.serve_forever, daemon=True)
    provider_thread.start()
    repo = Path(__file__).resolve().parents[2]
    report = {'source_commit': subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip(),
        'source_tree_dirty': bool(subprocess.check_output(['git', '-C', str(repo), 'status', '--porcelain'], text=True)),
        'platform': sys.platform, 'codex_commit': CODEX_COMMIT, 'codex_sha256': digest(args.codex),
        'tmux_sha256': digest(args.tmux), 'prompt': PROMPT, 'credentials_provided': False,
        'product_ssh_ui_verified': False, 'keystroke_input_verified': False,
        'model_lifecycle_verified': False, 'native_hook_summary_obtained': False, 'cases': []}
    # SSH 密钥保留在用户私有临时目录，只有 tmux socket 使用短路径。
    with tempfile.TemporaryDirectory(prefix='infinishell-native-ssh-') as temporary, \
            tempfile.TemporaryDirectory(prefix='issh-', dir='/tmp') as short_sockets:
        root = Path(temporary).resolve()
        server = None
        cases = {}
        try:
            for mode in MODES:
                directory = root / mode
                home, work, reports = directory / 'home', directory / 'work', directory / 'reports'
                for path in (home / '.codex', work, reports):
                    path.mkdir(parents=True)
                env = {key: os.environ[key] for key in ('PATH', 'TMPDIR', 'TMP', 'TEMP', 'LANG', 'LC_ALL') if key in os.environ}
                env.update(HOME=str(home), CODEX_HOME=str(home / '.codex'), SHELL='/bin/bash', TERM='xterm-256color',
                    WARP_CLI_AGENT_PROTOCOL_VERSION='1', WARP_CLIENT_VERSION='local', TERM_PROGRAM='WarpTerminal',
                    INFINISHELL_SSH_PROBE_PYTHON=str(Path(sys.executable).resolve()),
                    INFINISHELL_SSH_PROBE_REPORTS=str(reports), INFINISHELL_SSH_PROBE_TOKEN=uuid.uuid4().hex,
                    GIT_CONFIG_NOSYSTEM='1', GIT_TERMINAL_PROMPT='0')
                (home / '.codex/config.toml').write_text('cli_auth_credentials_store = "file"\nmodel = "gpt-5.4"\n'
                    'model_provider = "isolated_ssh_probe"\napproval_policy = "on-request"\nsandbox_mode = "read-only"\n'
                    '[model_providers.isolated_ssh_probe]\nname = "Isolated SSH probe"\n'
                    f'base_url = "http://127.0.0.1:{provider.server_port}/v1"\nwire_api = "responses"\n'
                    'requires_openai_auth = false\nsupports_websockets = false\n')
                tmux_config = directory / 'tmux.conf'
                tmux_config.write_text('set -g status off\nset -g default-shell /bin/sh\nset -g allow-passthrough '
                                       + ('on' if mode == 'tmux-on' else 'off') + '\n')
                entry = {'root': str(directory), 'work': str(work), 'reports': str(reports),
                         'environment': env, 'tmux_config': str(tmux_config),
                         'socket': str(Path(short_sockets).resolve() / (mode + '.sock'))}
                cases[mode] = entry
                case_report = {'mode': mode, 'passed': False}
                report['cases'].append(case_report)
                native_prepare(args, entry, case_report)
            config = root / 'probe.json'
            config.write_text(json.dumps({'cases': cases, 'tmux': str(args.tmux), 'codex': str(args.codex)}))
            remote = root / 'remote.py'
            remote.write_text(REMOTE_SOURCE)
            for name in ('host-key', 'client-key'):
                subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)], check=True)
            authorized = root / 'authorized_keys'
            authorized.write_text('restrict,pty ' + (root / 'client-key.pub').read_text())
            authorized.chmod(0o600)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
            known = root / 'known_hosts'
            known.write_text('[127.0.0.1]:' + str(port) + ' ' + (root / 'host-key.pub').read_text())
            launcher = root / 'force-command.sh'
            launcher.write_text('#!/bin/sh\nexec ' + shlex.join(['/usr/bin/env', '-i', 'PATH=' + os.environ['PATH'],
                'HOME=' + str(root), str(Path(sys.executable).resolve()), str(remote), str(config)])
                + ' "$SSH_ORIGINAL_COMMAND"\n')
            launcher.chmod(0o700)
            sshd_config = root / 'sshd_config'
            sshd_config.write_text('\n'.join(['ListenAddress 127.0.0.1', f'Port {port}', f'HostKey {root}/host-key',
                f'PidFile {root}/sshd.pid', f'AuthorizedKeysFile {authorized}', 'StrictModes yes',
                'PasswordAuthentication no', 'KbdInteractiveAuthentication no', 'UsePAM no',
                'AllowUsers ' + getpass.getuser(), 'PermitRootLogin no', 'AllowTcpForwarding no',
                'X11Forwarding no', 'PermitTunnel no', 'PrintMotd no', 'LogLevel ERROR',
                'SetEnv ' + json.dumps('HOME=' + str(root)), 'ForceCommand ' + str(launcher)]) + '\n')
            subprocess.run([str(args.sshd), '-t', '-f', str(sshd_config)], capture_output=True, check=True)
            server = subprocess.Popen([str(args.sshd), '-D', '-e', '-f', str(sshd_config)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.monotonic() + 3
            while True:
                require(server.poll() is None and time.monotonic() < deadline, '隔离 sshd 未就绪')
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=.1):
                        break
                except OSError:
                    time.sleep(.03)
            ssh = ['ssh', '-F', '/dev/null', '-o', 'BatchMode=yes', '-o', 'IdentitiesOnly=yes',
                '-o', 'StrictHostKeyChecking=yes', '-o', 'UserKnownHostsFile=' + str(known),
                '-o', 'ConnectTimeout=3', '-i', str(root / 'client-key'), '-p', str(port), '-tt',
                getpass.getuser() + '@127.0.0.1']
            for case_report in report['cases']:
                mode = case_report['mode']
                try:
                    capture_case([*ssh, mode], cases[mode], case_report, model_requests)
                    validate_case(cases[mode], case_report)
                except Exception as error:
                    case_report['error'] = str(error)
                    case_report['native_partial'] = {path.stem: json.loads(path.read_text())
                        for path in Path(cases[mode]['reports']).glob('*.json')}
                    raise
            require(not model_requests, '本机 provider 收到请求')
        except Exception as error:
            report['error'] = str(error)
        finally:
            for entry in cases.values():
                subprocess.run([str(args.tmux), '-S', entry['socket'], 'kill-server'],
                               capture_output=True, timeout=5)
            if server is not None and server.poll() is None:
                server.terminate(); server.wait(timeout=5)
            # 只读检查本次原生进程记录；不根据旧 PID 对未知进程发送信号。
            recorded = []
            for entry in cases.values():
                for path in Path(entry['reports']).glob('*.json'):
                    value = json.loads(path.read_text())
                    recorded.extend(value[key] for key in ('pid', 'codex_pid') if key in value)
            live = []
            for pid in sorted(set(recorded)):
                try:
                    os.kill(pid, 0)
                    live.append(pid)
                except ProcessLookupError:
                    pass
            report['recorded_test_pids_still_alive'] = live
            report['isolated_sshd_exit'] = server.returncode if server else None
            report['model_http_requests'] = model_requests
            report['model_http_request_count'] = len(model_requests)
            report['passed'] = not report.get('error') and len(report['cases']) == 3 and all(
                case['passed'] for case in report['cases']) and not live and not model_requests
            report = json.loads(json.dumps(report).replace(str(root), '<isolated-probe>')
                                .replace(str(Path(short_sockets).resolve()), '<isolated-sockets>'))
    provider.shutdown(); provider.server_close(); provider_thread.join(timeout=3)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'passed': report['passed'], 'error': report.get('error'),
                      'cases': [{key: case.get(key) for key in ('mode', 'passed', 'error', 'result')} for case in report['cases']],
                      'model_http_request_count': report['model_http_request_count']}))
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
