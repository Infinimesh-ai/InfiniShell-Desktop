#!/usr/bin/env python3
"""隔离验证 Codex 原生 hook、控制终端和通知字节；固定输入必须在模型请求前被阻断。"""

import argparse
import base64
import errno
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid


CODEX_COMMIT = 'be6e8eac029b183056b7e4402879f15d2c85f61b'
PLUGIN_ID = 'transport-probe@infinishell-transport-probe'
PROMPT = 'INFINISHELL_HOOK_TRANSPORT_PROBE_DO_NOT_RUN_MODEL'
PROVIDER = 'local_hook_transport_probe'
ENV_KEYS = ('SHELL', 'PATH', 'WARP_CLI_AGENT_PROTOCOL_VERSION', 'WARP_CLIENT_VERSION',
            'TERM_PROGRAM', 'TMUX', 'PLUGIN_ROOT', 'PLUGIN_DATA', 'CLAUDE_PLUGIN_ROOT')

# 只在原生安装的临时插件中运行；PLUGIN_ROOT 必须由 Codex 注入，宿主不设置该变量。
HOOK_SOURCE = r'''import errno
import json
import os
from pathlib import Path
import subprocess
import sys

payload = json.load(sys.stdin)
mode = sys.argv[1]
event = payload['hook_event_name']
if mode == 'block':
    if event != 'UserPromptSubmit':
        raise SystemExit('unexpected blocking event')
    print(json.dumps({'continue': False, 'stopReason': 'isolated hook transport probe'}))
    raise SystemExit(0)

record = {'event': event, 'mode': mode, 'pid': os.getpid(), 'ppid': os.getppid(),
          'pgid': os.getpgrp(), 'sid': os.getsid(0),
          'isatty': {str(fd): os.isatty(fd) for fd in (0, 1, 2)},
          'environment': {key: os.environ.get(key) for key in __ENV_KEYS__},
          'input': payload}
try:
    tty = os.open('/dev/tty', os.O_WRONLY | os.O_NOCTTY)
except OSError as error:
    record['tty'] = {'opened': False, 'errno': error.errno, 'error': os.strerror(error.errno)}
else:
    try:
        record['tty'] = {'opened': True, 'isatty': os.isatty(tty),
                         'foreground_pgid': os.tcgetpgrp(tty)}
        if mode == 'diagnostic':
            marker = os.environ['INFINISHELL_HOOK_TRANSPORT_TOKEN'] + ':' + event
            sequence = ('\x1b]777;notify;infinishell-transport-probe;' + marker + '\x07').encode()
            written = 0
            while written < len(sequence):
                written += os.write(tty, sequence[written:])
            record['tty']['written_bytes'] = written
    finally:
        os.close(tty)

if mode == 'reference':
    script = {'SessionStart': 'on-session-start.sh',
              'UserPromptSubmit': 'on-prompt-submit.sh'}[event]
    original = Path(os.environ['PLUGIN_ROOT']) / 'reference/scripts' / script
    result = subprocess.run(['bash', str(original)], input=json.dumps(payload),
                            text=True, capture_output=True, timeout=5)
    record['reference'] = {'script': script, 'exit_code': result.returncode,
                           'stdout': result.stdout, 'stderr': result.stderr}
output = Path(os.environ['INFINISHELL_HOOK_TRANSPORT_OUTPUT']) / (event + '-' + mode + '.json')
output.write_text(json.dumps(record), encoding='utf-8')
'''.replace('__ENV_KEYS__', repr(ENV_KEYS))


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def tree(root):
    result = {}
    for path in sorted(root.rglob('*')):
        require(not path.is_symlink(), '测试来源不能包含符号链接')
        if path.is_file():
            result[path.relative_to(root).as_posix()] = digest(path)
    return result


class ControlledPtyRecorder:
    def __init__(self, executable, env, directory, trace):
        self.trace = trace
        self.queue = queue.Queue()
        self.errors = []
        self.pty_bytes = bytearray()
        self.master, slave = os.openpty()
        self.tty_path = os.ttyname(slave)

        # 固定启动器在新的单线程进程中建立控制终端，避免多线程宿主的 preexec_fn 死锁。
        # CLI 路径和 fd 作为 argv 数据传递，不插值进 Python 或 shell 源码。
        bootstrap = ('import fcntl,os,sys,termios; os.setsid(); '
                     'fcntl.ioctl(int(sys.argv[2]),termios.TIOCSCTTY,0); '
                     'os.execv(sys.argv[1],[sys.argv[1],"app-server","--stdio",'
                     '"--disable","shell_snapshot"])')
        try:
            self.process = subprocess.Popen(
                [sys.executable, '-c', bootstrap, str(executable), str(slave)],
                env=env, cwd=directory, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True, encoding='utf-8', errors='strict',
                bufsize=1, pass_fds=(slave,))
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)
        self.readers = []
        for channel in ('stdout', 'stderr'):
            def read(channel=channel):
                try:
                    for line in getattr(self.process, channel):
                        try:
                            value = json.loads(line)
                        except json.JSONDecodeError:
                            value = line.rstrip()
                        trace.append({'channel': channel, 'value': value})
                        if channel == 'stdout':
                            self.queue.put(value)
                except Exception as error:
                    self.errors.append(str(error))
            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            self.readers.append(reader)

        def read_terminal():
            try:
                while data := os.read(self.master, 65536):
                    self.pty_bytes.extend(data)
                    require(len(self.pty_bytes) <= 1024 * 1024, 'PTY 输出超过探测上限')
            except OSError as error:
                if error.errno not in (errno.EIO, errno.EBADF):
                    self.errors.append(str(error))
            except Exception as error:
                self.errors.append(str(error))
        reader = threading.Thread(target=read_terminal, daemon=True)
        reader.start()
        self.readers.append(reader)

    def send(self, value):
        method = value.get('method')
        require(method in ('initialize', 'initialized', 'hooks/list', 'config/value/write',
                           'thread/start', 'turn/start'), '拒绝探测范围外 RPC')
        if method == 'turn/start':
            require(value['params'].get('input') == [
                {'type': 'text', 'text': PROMPT, 'text_elements': []}], '拒绝非固定受阻输入')
        self.trace.append({'channel': 'request', 'value': value})
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()

    def rpc(self, method, params, request_id):
        self.send({'id': request_id, 'method': method, 'params': params})
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                value = self.queue.get(timeout=0.1)
            except queue.Empty:
                require(self.process.poll() is None, '原生进程提前退出')
                continue
            if isinstance(value, dict) and value.get('id') == request_id:
                require('error' not in value, f'原生 RPC 失败：{value.get("error")}')
                return value['result']
        raise RuntimeError(f'{method} 没有在期限内确认')

    def close(self):
        try:
            self.process.stdin.close()
        except (BrokenPipeError, OSError):
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        for reader in self.readers:
            reader.join(timeout=2)
        os.close(self.master)
        require(not self.errors and all(not reader.is_alive() for reader in self.readers),
                '原生输出读取未正常结束：' + repr(self.errors))


def completed_hooks(trace):
    return [item['value']['params']['run'] for item in trace
            if isinstance(item.get('value'), dict) and item['value'].get('method') == 'hook/completed']


def run(args, directory, port, report):
    repo = Path(__file__).resolve().parents[2]
    source = repo / 'app/assets/bundled/cli-agent-plugins/codex'
    metadata = json.loads((source / 'SOURCE_METADATA.json').read_text(encoding='utf-8'))
    reference = source / 'source/plugins/warp'
    reference_tree = tree(reference)
    require(reference_tree == {name.removeprefix('plugins/warp/'): value['sha256']
            for name, value in metadata['files'].items() if name.startswith('plugins/warp/')},
            '随附完整参考插件不匹配受控元数据')
    report['reference_tree_sha256'] = reference_tree
    user_home, cli_home = directory / 'home', directory / 'codex'
    user_home.mkdir()
    cli_home.mkdir()
    reports = directory / 'hook-reports'
    reports.mkdir()
    env = {key: os.environ[key] for key in ('PATH', 'TMPDIR', 'TMP', 'TEMP', 'LANG', 'LC_ALL')
           if key in os.environ}
    env.update(HOME=str(user_home), CODEX_HOME=str(cli_home), SHELL=args.shell,
               WARP_CLI_AGENT_PROTOCOL_VERSION='1', WARP_CLIENT_VERSION='local',
               TERM_PROGRAM='WarpTerminal', INFINISHELL_HOOK_TRANSPORT_OUTPUT=str(reports),
               INFINISHELL_HOOK_TRANSPORT_TOKEN=uuid.uuid4().hex,
               INFINISHELL_HOOK_TRANSPORT_PYTHON=sys.executable,
               GIT_CONFIG_NOSYSTEM='1', GIT_TERMINAL_PROMPT='0')
    config = cli_home / 'config.toml'
    config.write_text('cli_auth_credentials_store = "file"\nmodel = "gpt-5.4"\n'
        f'model_provider = "{PROVIDER}"\napproval_policy = "on-request"\nsandbox_mode = "read-only"\n'
        f'[model_providers.{PROVIDER}]\nname = "Isolated hook transport probe"\n'
        f'base_url = "http://127.0.0.1:{port}/v1"\nwire_api = "responses"\n'
        'requires_openai_auth = false\nsupports_websockets = false\n', encoding='utf-8')
    marketplace = directory / 'marketplace'
    plugin = marketplace / 'plugins/transport-probe'
    for child in ('.codex-plugin', 'hooks'):
        (plugin / child).mkdir(parents=True)
    shutil.copytree(reference, plugin / 'reference')
    (plugin / 'transport_probe.py').write_text(HOOK_SOURCE, encoding='utf-8')
    (plugin / '.codex-plugin/plugin.json').write_text(json.dumps({
        'name': 'transport-probe', 'description': 'Isolated native hook transport probe',
        'version': '0.1.0'}), encoding='utf-8')
    command = '"$INFINISHELL_HOOK_TRANSPORT_PYTHON" "$PLUGIN_ROOT/transport_probe.py" '
    hooks = {'hooks': {event: [{'hooks': [{'type': 'command', 'command': command + mode,
                                         'timeout': 10} for mode in modes]}]
                       for event, modes in [('SessionStart', ['diagnostic', 'reference']),
                                            ('UserPromptSubmit', ['diagnostic', 'reference', 'block'])]}}
    (plugin / 'hooks/hooks.json').write_text(json.dumps(hooks), encoding='utf-8')
    index = marketplace / '.agents/plugins/marketplace.json'
    index.parent.mkdir(parents=True)
    index.write_text(json.dumps({'name': 'infinishell-transport-probe', 'plugins': [{
        'name': 'transport-probe', 'source': './plugins/transport-probe', 'version': '0.1.0',
        'policy': {'installation': 'AVAILABLE', 'authentication': 'ON_INSTALL'}}]}), encoding='utf-8')
    report['diagnostic_source_sha256'] = digest(plugin / 'transport_probe.py')

    for arguments in (['--version'], ['plugin', 'marketplace', 'add', str(marketplace), '--json'],
                      ['plugin', 'add', PLUGIN_ID, '--json']):
        result = subprocess.run([str(args.codex_executable), *arguments], cwd=directory, env=env,
                                capture_output=True, text=True, timeout=30)
        report['commands'].append({'arguments': arguments, 'exit_code': result.returncode,
                                   'stdout': result.stdout, 'stderr': result.stderr})
        require(result.returncode == 0, '原生命令执行失败')
        if arguments == ['--version']:
            require(result.stdout.strip() == 'codex-cli 0.147.0', '只允许固定 Codex 0.147.0')
        if arguments[:2] == ['plugin', 'add']:
            cache = Path(json.loads(result.stdout)['installedPath'])
    require(cache.resolve().is_relative_to(cli_home) and tree(cache) == tree(plugin),
            '原生安装副本与受控测试插件不一致')

    def start(label):
        trace = []
        report['traces'].append({'phase': label, 'events': trace})
        recorder = ControlledPtyRecorder(args.codex_executable, env, directory, trace)
        try:
            initialized = recorder.rpc('initialize', {'clientInfo': {
                'name': 'infinishell_hook_transport_probe', 'version': '0.1.0'},
                'capabilities': {'experimentalApi': True}}, 1)
            require(Path(initialized['codexHome']).resolve() == cli_home, '原生配置作用域越界')
            recorder.send({'method': 'initialized'})
            return recorder
        except BaseException:
            recorder.close()
            raise

    recorder = start('native_authorization')
    try:
        listed = recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 2)
        native_hooks = [hook for group in listed['data'] for hook in group['hooks']]
        require(len(native_hooks) == 5 and all(hook.get('pluginId') == PLUGIN_ID and
                hook['trustStatus'] == 'untrusted' for hook in native_hooks), '原生测试 hook 清单不符')
        expected_commands = {command + mode for mode in ('diagnostic', 'reference', 'block')}
        for index, hook in enumerate(native_hooks):
            require(hook['command'] in expected_commands and
                    Path(hook['sourcePath']).resolve() == cache / 'hooks/hooks.json',
                    '拒绝授权未知或越界 hook')
            # 授权只作用于刚生成、逐字校验的本测试定义，绝不复制用户的既有信任。
            result = recorder.rpc('config/value/write', {
                'keyPath': f'hooks.state.{json.dumps(hook["key"])}',
                'value': {'enabled': True, 'trusted_hash': hook['currentHash']},
                'mergeStrategy': 'upsert', 'filePath': str(config)}, 10 + index)
            require(result['status'] == 'ok', '原生测试授权未确认')
    finally:
        recorder.close()

    recorder = start('native_controlled_pty')
    try:
        listed = recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 2)
        require(all(hook['trustStatus'] == 'trusted' and hook['enabled']
                    for group in listed['data'] for hook in group['hooks']), '原生信任未生效')
        thread = recorder.rpc('thread/start', {'cwd': str(directory), 'ephemeral': False,
            'sessionStartSource': 'startup', 'sandbox': 'read-only', 'approvalPolicy': 'on-request',
            'modelProvider': PROVIDER}, 3)
        require(thread['modelProvider'] == PROVIDER, '拒绝使用非本机测试模型 provider')
        time.sleep(1)
        report['hooks_after_thread_start_before_input'] = completed_hooks(recorder.trace)
        report['model_input_submitted'] = True
        turn = recorder.rpc('turn/start', {'threadId': thread['thread']['id'], 'input': [
            {'type': 'text', 'text': PROMPT, 'text_elements': []}]}, 4)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            complete = completed_hooks(recorder.trace)
            if len(complete) == 5 and len(list(reports.glob('*.json'))) == 4:
                break
            time.sleep(0.05)
        report['completed_hooks'] = complete
        report['hook_reports'] = {path.stem: json.loads(path.read_text(encoding='utf-8'))
                                  for path in sorted(reports.glob('*.json'))}
        report['native_process'] = {'pid': recorder.process.pid, 'sid': os.getsid(recorder.process.pid),
                                     'pgid': os.getpgid(recorder.process.pid), 'tty': recorder.tty_path}
        require(len(complete) == 5, '原生 hook 没有全部完成；不能把握手当执行成功')
        require(sorted(run['status'] for run in complete) == ['completed'] * 4 + ['stopped'],
                '原生诊断、对照或阻断 hook 执行失败')
        require(any(run['eventName'] == 'userPromptSubmit' and run['status'] == 'stopped'
                    for run in complete), '固定输入未在原生 hook 阶段被阻断')
        for event in ('SessionStart', 'UserPromptSubmit'):
            for mode in ('diagnostic', 'reference'):
                record = report['hook_reports'][event + '-' + mode]
                require(record['environment']['PLUGIN_ROOT'] == str(cache), '原生 PLUGIN_ROOT 错误')
                require(record['input']['session_id'] == thread['thread']['id'], '原生会话关联错误')
                require(record['tty']['opened'], f'{event}/{mode} 无法打开控制终端')
                if mode == 'reference':
                    require(record['reference']['exit_code'] == 0, '未修改的原脚本执行失败')
            marker = (f'\x1b]777;notify;infinishell-transport-probe;'
                      f'{env["INFINISHELL_HOOK_TRANSPORT_TOKEN"]}:{event}\x07').encode()
            require(bytes(recorder.pty_bytes).count(marker) == 1, '唯一诊断 OSC 没有到达真实 PTY')
        report['native_turn_id'] = turn['turn']['id']
    finally:
        recorder.close()
        report['pty_bytes_base64'] = base64.b64encode(recorder.pty_bytes).decode()
        report['pty_bytes_sha256'] = hashlib.sha256(recorder.pty_bytes).hexdigest()
        report['pty_byte_count'] = len(recorder.pty_bytes)
        report['native_exit_code'] = recorder.process.returncode
    raw = bytes(recorder.pty_bytes)
    report['reference_notifications'] = []
    prefix = b'\x1b]777;notify;warp://cli-agent;'
    for tail in raw.split(prefix)[1:]:
        body = tail.split(b'\x07', 1)[0]
        report['reference_notifications'].append(json.loads(body))
    require(sorted(item['event'] for item in report['reference_notifications']) ==
            ['prompt_submit', 'session_start'], '原脚本的两条完整通知未到达 PTY')
    require(tree(reference) == reference_tree and tree(cache) == tree(plugin), '探测意外修改了来源或缓存')
    require(digest(args.codex_executable) == args.expected_codex_sha256, '探测期间原生 CLI 文件发生变化')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex-executable', type=Path, required=True)
    parser.add_argument('--expected-codex-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--shell', default='/bin/zsh' if sys.platform == 'darwin' else '/bin/bash')
    args = parser.parse_args()
    require(os.name == 'posix', '此探针仅验证 Unix 控制终端，不替代 Windows ConPTY 验收')
    require(args.codex_executable.is_absolute() and args.output.is_absolute(), 'CLI 和输出必须为绝对路径')
    require(Path(args.shell).is_absolute() and os.access(args.shell, os.X_OK), '测试 shell 必须可执行')
    repo = Path(__file__).resolve().parents[2]
    require(not args.output.resolve().is_relative_to(repo), '原始证据必须保存到源树外')
    require(digest(args.codex_executable) == args.expected_codex_sha256, '固定 CLI 摘要不匹配')
    requests = []

    class RejectModel(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append({'method': self.command, 'path': self.path})
            self.send_error(503, 'No model is available in this probe')
        do_POST = do_GET
        do_PUT = do_GET
        do_DELETE = do_GET
        do_PATCH = do_GET
        do_OPTIONS = do_GET
        do_HEAD = do_GET

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), RejectModel)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    report = {'passed': False, 'codex_version': '0.147.0', 'codex_source_commit': CODEX_COMMIT,
              'executable_sha256': args.expected_codex_sha256, 'host_os': sys.platform,
              'credentials_provided': False, 'model_input_submitted': False,
              'model_generation_verified': False, 'full_lifecycle_verified': False,
              'gui_receiver_verified': False, 'commands': [], 'traces': []}
    with tempfile.TemporaryDirectory(prefix='infinishell-hook-transport-') as temporary:
        directory = Path(temporary).resolve()
        try:
            run(args, directory, server.server_port, report)
            require(not requests, '出现模型 HTTP 请求，禁止计为通过')
            report['passed'] = True
        except Exception as error:
            report['error'] = str(error)
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=2)
            report['model_http_requests'] = requests
            report['model_http_request_count'] = len(requests)
            if requests:
                report['passed'] = False
            # 原始报告也移除临时 HOME 路径；PTY 原始字节仅含本测试生成的公开夹具。
            rendered = json.dumps(report, ensure_ascii=False, indent=2).replace(str(directory), '<PROBE_ROOT>')
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered + '\n', encoding='utf-8')
    print(json.dumps({'passed': report['passed'], 'error': report.get('error'),
                      'model_http_request_count': len(requests)}))
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
