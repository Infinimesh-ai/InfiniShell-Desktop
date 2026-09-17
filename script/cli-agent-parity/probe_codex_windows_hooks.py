"""Windows 原生 Codex hook 验证；只改临时副本，不证明 ConPTY 通知与完整生命周期。"""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
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

sys.dont_write_bytecode = True
from codex_windows_hook_command import (SCRIPTS, encode, encode_source, verify_encoding,
                                        verify_windows_argv, verify_windows_bytes_and_boundary,
                                        verify_windows_syntax, windows_environment)
from codex_windows_hook_inputs import (CODEX_COMMIT, RELEASE_ASSETS, obtain_inputs,
                                       plugin_base, require, verify_plugin)


class NativeRecorder:
    def __init__(self, command, env, directory, events):
        self.events = events
        self.reader_errors = []
        self.queue = queue.Queue()
        self.process = subprocess.Popen(command, env=env, cwd=directory, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        text=True, encoding='utf-8', errors='strict', bufsize=1)
        self.readers = []
        for name in ('stdout', 'stderr'):
            def read(channel=name):
                try:
                    for line in getattr(self.process, channel):
                        try:
                            value = json.loads(line)
                        except json.JSONDecodeError:
                            value = line.rstrip()
                        self.events.append({'channel': channel, 'value': value})
                        if channel == 'stdout':
                            self.queue.put(value)
                except Exception as error:
                    self.reader_errors.append(str(error))
            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            self.readers.append(reader)

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()

    def rpc(self, method, params, request_id):
        self.send({'id': request_id, 'method': method, 'params': params})
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                item = self.queue.get(timeout=0.1)
            except queue.Empty:
                if self.process.poll() is not None:
                    break
                continue
            if isinstance(item, dict) and item.get('id') == request_id:
                if 'error' in item:
                    raise RuntimeError(item['error'])
                return item['result']
        raise RuntimeError(f'{method} 没有原生确认')

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
        require(not self.reader_errors and all(not reader.is_alive() for reader in self.readers),
                '原生输出读取失败或没有结束: ' + repr(self.reader_errors))


def start_codex(executable, env, directory, traces):
    events = []
    traces.append(events)
    recorder = NativeRecorder([str(executable), 'app-server', '--stdio', '--disable', 'shell_snapshot'],
                              env, directory, events)
    try:
        recorder.rpc('initialize', {'clientInfo': {'name': 'windows_hook_verification', 'version': '0.1.0'},
                                    'capabilities': {'experimentalApi': True}}, 1)
        recorder.send({'method': 'initialized'})
        return recorder
    except BaseException:
        recorder.close()
        raise


def one_case(args, directory, requests, evidence):
    from apply_notification_patch import apply_files, bundle_data, default_bundle, validate_tree
    metadata, replacements = bundle_data(default_bundle(), 'codex')
    cli_home = directory / 'codex'
    cli_home.mkdir(parents=True)
    user_home = directory / 'home'
    user_home.mkdir()
    env = args.native_environment.copy()
    env.update(CODEX_HOME=str(cli_home), HOME=str(user_home), USERPROFILE=str(user_home),
               APPDATA=str(user_home / 'AppData/Roaming'), LOCALAPPDATA=str(user_home / 'AppData/Local'),
               INFINISHELL_HOOK_PROBE_CASE=str(directory))
    for key in ('APPDATA', 'LOCALAPPDATA'):
        Path(env[key]).mkdir(parents=True)
    marketplace = directory / 'marketplace'
    plugin = marketplace / 'plugins/warp'
    verify_plugin(args.upstream_plugin, plugin_base(args.repo))
    shutil.copytree(args.upstream_plugin, plugin)
    validate_tree(plugin, '0.4.0', metadata)
    apply_files(plugin, metadata, replacements)
    from codex_windows_notify import candidate_notify, evidence as notify_evidence
    notify_path = plugin / 'scripts/warp-notify.sh'
    original_notify = notify_path.read_text(encoding='utf-8')
    notification_candidate = candidate_notify(original_notify)
    notify_path.write_text(notification_candidate, encoding='utf-8', newline='\n')
    evidence['notification_transport_candidate'] = notify_evidence(original_notify, notification_candidate)
    hooks_path = plugin / 'hooks/hooks.json'
    hooks = json.loads(hooks_path.read_text(encoding='utf-8'))
    for groups in hooks['hooks'].values():
        for group in groups:
            for handler in group['hooks']:
                script = next(script for script in SCRIPTS if script in handler['command'])
                handler['commandWindows'] = encode(script)
    # 只有临时测试插件增加阻断 hook；不生成模型请求，也不绕过原生 hook 信任。
    stop_source = "[Console]::Out.WriteLine('{\"continue\":false,\"stopReason\":\"isolated hook probe\"}')"
    blocker = encode_source(stop_source)
    hooks['hooks']['UserPromptSubmit'].append({'hooks': [{'type': 'command', 'command': 'false',
                                                        'commandWindows': blocker}]})
    hooks_path.write_text(json.dumps(hooks), encoding='utf-8')
    evidence['temporary_hooks_sha256'] = hashlib.sha256(hooks_path.read_bytes()).hexdigest()
    original_hashes = {}
    for script in ('on-session-start.sh', 'on-prompt-submit.sh'):
        path = plugin / 'scripts' / script
        original_hashes[script] = hashlib.sha256(path.read_bytes()).hexdigest()
        original = path.with_name(script + '.fixture-original')
        path.rename(original)
        # 插桩只发生于临时副本；原脚本逐字保留并实际调用，不把插桩当发布文件。
        wrapper = '''#!/bin/bash
set -e
input=$(cat)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
printf '%s' "$input" | jq -c --arg root "$PLUGIN_ROOT" '{hook_event_name,turn_id,session_id,cwd,prompt,plugin_root:$root}' > "$INFINISHELL_HOOK_PROBE_CASE/__FIXTURE_SCRIPT__.json"
printf '%s' "$input" | bash -- "$SCRIPT_DIR/__FIXTURE_SCRIPT__.fixture-original"
'''.replace('__FIXTURE_SCRIPT__', script)
        path.write_text(wrapper, encoding='utf-8', newline='\n')
    index = marketplace / '.agents/plugins/marketplace.json'
    index.parent.mkdir(parents=True)
    index.write_text(json.dumps({'name': 'codex-warp', 'plugins': [{'name': 'warp',
        'source': './plugins/warp', 'version': '0.4.0',
        'policy': {'installation': 'AVAILABLE', 'authentication': 'ON_INSTALL'}}]}), encoding='utf-8')
    config = cli_home / 'config.toml'
    config.write_text('cli_auth_credentials_store = "file"\nmodel = "gpt-5.4"\n'
        'model_provider = "local_hook_probe"\n'
        '[model_providers.local_hook_probe]\nname = "Local hook probe"\n'
        f'base_url = "http://127.0.0.1:{args.port}/v1"\n'
        'wire_api = "responses"\nrequires_openai_auth = false\nsupports_websockets = false\n', encoding='utf-8')
    for command in (['plugin', 'marketplace', 'add', str(marketplace), '--json'],
                    ['plugin', 'add', 'warp@codex-warp', '--json']):
        subprocess.run([str(args.codex_executable), *command], env=env, cwd=directory,
                       capture_output=True, check=True, timeout=30)
    recorder = start_codex(args.codex_executable, env, directory, evidence['traces'])
    try:
        listed = recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 2)
        native_hooks = [hook for group in listed['data'] for hook in group['hooks']
                        if hook.get('pluginId') == 'warp@codex-warp']
        require(len(native_hooks) == 6, '原生 hook 注册数量不匹配')
        require(all(hook['trustStatus'] == 'untrusted' for hook in native_hooks), '初始 hook 必须未受信任')
        require({hook['command'] for hook in native_hooks} == {encode(s) for s in SCRIPTS} | {blocker},
                'Windows 原生没有选择预期的 commandWindows')
        installed_root = Path(native_hooks[0]['sourcePath']).parent.parent
        require(installed_root.resolve().is_relative_to(cli_home.resolve()), '原生插件目录不属于隔离 HOME')
        expected_tree = {path.relative_to(plugin).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                         for path in plugin.rglob('*') if path.is_file()}
        actual_tree = {path.relative_to(installed_root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                       for path in installed_root.rglob('*') if path.is_file()}
        require(actual_tree == expected_tree, '授权前原生安装副本与临时受控插件不一致')
        evidence['native_initial_hooks'] = native_hooks
    finally:
        recorder.close()
    with config.open('a', encoding='utf-8') as target:
        for hook in native_hooks:
            # 仅授权本测试生成的临时插件，待授权脚本已在复制前完整验证。
            target.write(f'\n[hooks.state.{json.dumps(hook["key"])}]\nenabled = true\n'
                         f'trusted_hash = {json.dumps(hook["currentHash"])}\n')
    before = config.read_bytes()
    recorder = start_codex(args.codex_executable, env, directory, evidence['traces'])
    prompt = "中文输入\nEnglish ' $() `literal` %PATH% !name! &"
    try:
        trusted = recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 3)
        trusted_hooks = [hook for group in trusted['data'] for hook in group['hooks']
                         if hook.get('pluginId') == 'warp@codex-warp']
        require(len(trusted_hooks) == 6, '信任后 hook 数量不匹配')
        require(all(hook['enabled'] and hook['trustStatus'] == 'trusted' for hook in trusted_hooks),
                '临时 hook 没有得到原生信任确认')
        thread = recorder.rpc('thread/start', {'cwd': str(directory), 'sessionStartSource': 'startup',
            'ephemeral': False, 'sandbox': 'read-only', 'approvalPolicy': 'on-request',
            'modelProvider': 'local_hook_probe'}, 4)
        require(thread['modelProvider'] == 'local_hook_probe', '原生会话没有使用隔离的本机 provider')
        turn = recorder.rpc('turn/start', {'threadId': thread['thread']['id'], 'input': [
            {'type': 'text', 'text': prompt, 'text_elements': []}]}, 5)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            complete = [event['value'].get('params', {}).get('run', {}) for event in recorder.events
                        if isinstance(event['value'], dict) and event['value'].get('method') == 'hook/completed']
            if len(complete) >= 3 and any(run.get('eventName') == 'userPromptSubmit' and run.get('status') == 'stopped'
                                         for run in complete):
                break
            time.sleep(0.05)
        require(any(run.get('eventName') == 'userPromptSubmit' and run.get('status') == 'stopped'
                    for run in complete), '原生阻断 hook 未确认执行')
        require([run['status'] for run in complete if run['eventName'] == 'sessionStart'] == ['completed'],
                '原始 SessionStart 脚本没有成功完成')
        require(sorted(run['status'] for run in complete if run['eventName'] == 'userPromptSubmit') ==
                ['completed', 'stopped'], '原始 UserPromptSubmit 或独立阻断脚本没有成功完成')
        markers = {script: json.loads((directory / (script + '.json')).read_text(encoding='utf-8'))
                   for script in ('on-session-start.sh', 'on-prompt-submit.sh')}
        require(markers['on-session-start.sh']['hook_event_name'] == 'SessionStart', 'SessionStart 未原生执行')
        require(markers['on-prompt-submit.sh']['prompt'] == prompt, '原生 prompt 字节语义被更改')
        require(markers['on-prompt-submit.sh']['hook_event_name'] == 'UserPromptSubmit' and
                markers['on-prompt-submit.sh']['session_id'] == thread['thread']['id'] and
                markers['on-prompt-submit.sh']['turn_id'] == turn['turn']['id'], '原生输入关联字段丢失')
        evidence['markers'] = markers
        require(all(Path(marker['plugin_root']) == installed_root for marker in markers.values()),
                'PLUGIN_ROOT 没有指向原生已安装插件')
        require(all(hashlib.sha256((installed_root / 'scripts' / (s + '.fixture-original')).read_bytes()).hexdigest() == h
                    for s, h in original_hashes.items()), '插桩后的原脚本没有逐字保留')
        require(not requests, f'阻断前出现模型 HTTP 请求: {requests}')
        require(not (directory / 'INJECTED').exists(), '目录数据被错误作为 shell 源码执行')
        require(config.read_bytes() == before, '原生执行意外修改了已核对的隔离配置')
        return {'name': directory.name, 'native_session_start': True, 'native_user_prompt_submit': True,
                'native_prompt_blocked': True, 'native_hooks': trusted_hooks,
                'model_http_requests': len(requests), 'model_generation_verified': False,
                'native_conpty_notifications_verified': False, 'full_lifecycle_verified': False}
    finally:
        recorder.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--architecture', choices=RELEASE_ASSETS, default='x86_64')
    for name in ('codex-executable', 'upstream-plugin', 'download-dir'):
        parser.add_argument('--' + name, type=Path)
    for name in ('bash-executable', 'jq-executable', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    # 所有写入都在源树之外；-B / dont_write_bytecode 阻止导入产生源树 pycache。
    args.repo = args.repo.resolve()
    for name in ('codex_executable', 'upstream_plugin', 'download_dir', 'bash_executable', 'jq_executable', 'output'):
        path = getattr(args, name)
        if path is not None:
            require(path.is_absolute(), f'{name} 必须是绝对路径')
            # 输入先保留链接身份，交给摘要校验拒绝；写入路径解析后用于源树边界判断。
            setattr(args, name, path.resolve() if name in ('download_dir', 'output') else path)
    for path in (args.output, args.download_dir, Path(tempfile.gettempdir()).resolve()):
        require(path is None or not path.is_relative_to(args.repo), '禁止将产物、下载或临时 HOME 写入源树')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {'passed': False, 'official_commit': CODEX_COMMIT, 'cases': [], 'checks': {},
              'scope': 'native_cmd_powershell_bash_hooks_and_blocking_only',
              'credentials_provided': False, 'native_conpty_notifications_verified': False,
              'full_lifecycle_verified': False, 'model_generation_verified': False}
    requests = []
    server = None
    worker = None
    phase = 'platform'
    try:
        require(os.name == 'nt', '本脚本必须在 Windows 原生执行；不能用 Bash 回放代替')
        phase = 'fixed_input_hashes'
        args.codex_executable, args.upstream_plugin, report['fixed_inputs'] = obtain_inputs(
            args.repo, args.download_dir, args.architecture, args.codex_executable, args.upstream_plugin)
        phase = 'encoding_roundtrip'
        report['checks']['encoding'] = verify_encoding()
        phase = 'native_prerequisites'
        args.native_environment = windows_environment(args.bash_executable, args.jq_executable)
        version = subprocess.run([str(args.codex_executable), '--version'], env=args.native_environment,
                                 capture_output=True, text=True, encoding='utf-8', check=True, timeout=10).stdout.strip()
        require(version == 'codex-cli 0.147.0', '原生 Codex 版本不匹配')
        report['cli'] = version
        phase = 'powershell_51_syntax'
        verify_windows_syntax(args.native_environment)
        report['checks'][phase] = True
        phase = 'native_argv_roundtrip'
        verify_windows_argv(args.native_environment)
        report['checks'][phase] = True
        phase = 'native_bytes_and_cmd_boundary'
        report['checks'][phase] = verify_windows_bytes_and_boundary(args.native_environment)

        class RejectModel(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append({'method': self.command, 'path': self.path})
                self.send_error(503, 'No model available in this fixture')
            do_POST = do_GET
            do_PUT = do_GET
            do_DELETE = do_GET
            do_PATCH = do_GET
            do_OPTIONS = do_GET
            do_HEAD = do_GET
            def log_message(self, *arguments):
                pass

        server = ThreadingHTTPServer(('127.0.0.1', 0), RejectModel)
        args.port = server.server_port
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        phase = 'native_codex_hooks'
        with tempfile.TemporaryDirectory(prefix='infinishell-native-windows-hooks-') as tmp:
            for name in ('插件 空 格', "插件 ' $(touch INJECTED) `touch INJECTED` & %PATH% !name! ^ ()"):
                evidence = {'name': name, 'passed': False, 'traces': []}
                report['cases'].append(evidence)
                evidence.update(one_case(args, Path(tmp) / name, requests, evidence))
                evidence['passed'] = True
        phase = 'zero_model_requests'
        require(not requests, f'原生进程关闭前出现模型 HTTP 请求: {requests}')
        report['checks'][phase] = True
        phase = 'source_inputs_unchanged'
        verify_plugin(args.upstream_plugin, plugin_base(args.repo))
        require(verify_encoding() == report['checks']['encoding'], '探测期间固定源码被并发修改')
        report['checks'][phase] = True
        report['passed'] = True
    except Exception as error:
        report['failure'] = {'phase': phase, 'type': type(error).__name__, 'message': str(error)}
        if isinstance(error, subprocess.CalledProcessError):
            report['failure']['stderr'] = (error.stderr.decode('utf-8', errors='replace')
                                            if isinstance(error.stderr, bytes) else error.stderr)
        raise
    finally:
        if server is not None:
            server.shutdown()
            server.server_close()
            worker.join(timeout=2)
        report['model_http_requests'] = requests
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
