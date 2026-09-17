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
from codex_windows_formal import (EVENTS, authorize_config, exact_plugin_tree, formal_transport_expectations,
                                  native_hooks, probe_bundle, restore_config, same_native_registration,
                                  split_trigger_hooks, tree, validate_native_hooks)


HOOK_COMPLETION_TIMEOUT = 45
HOOK_PROMPTS = (
    "中文输入\nEnglish ' $() `literal` %PATH% !name! &",
    "中文输入\r\nEnglish 保留原始 CRLF 和字面 \\r\\n ' $() `literal` %PATH% !name! &",
)


def binary_query_candidate(original):
    command = "jq -r '.prompt // empty'"
    require(original.count(command) == 1, 'UserPromptSubmit 不是固定上游 query 提取配方')
    # 仅临时插件使用原生 jq 的二进制输出，既不引入 CR，也不删除用户已有的 CR。
    candidate = original.replace(command, "jq --binary -r '.prompt // empty'")
    return candidate, {'original_sha256': hashlib.sha256(original.encode('utf-8')).hexdigest(),
                       'candidate_sha256': hashlib.sha256(candidate.encode('utf-8')).hexdigest(),
                       'product_recipe_modified': False, 'newline_normalization_applied': False}


def verify_jq_newline_boundary(environment, evidence=None):
    executable = shutil.which('jq', path=environment['PATH'])
    require(executable is not None, '缺少实际 jq 可执行文件')
    version = subprocess.run([executable, '--version'], env=environment, capture_output=True, timeout=10, check=True)
    cases = []
    evidence = {} if evidence is None else evidence
    evidence.update(jq_executable=executable, jq_version=version.stdout.decode('utf-8').strip(),
                    cases=cases, normalization_used=False)
    for label, prompt in zip(('LF', 'CRLF'), HOOK_PROMPTS):
        source = json.dumps({'prompt': prompt}, ensure_ascii=False).encode('utf-8')
        observed = {}
        case = {'input_line_ending': label, 'input_utf8_hex': prompt.encode('utf-8').hex(),
                'outputs': observed, 'binary_preserves_original_bytes': False}
        cases.append(case)
        for mode, flags in (('text', []), ('binary', ['--binary'])):
            result = subprocess.run([executable, *flags, '-r', '.prompt // empty'],
                                    input=source, env=environment, capture_output=True, timeout=10, check=True)
            observed[mode] = {'stdout_hex': result.stdout.hex(), 'stderr_hex': result.stderr.hex()}
            if mode == 'binary':
                require(result.stdout == prompt.encode('utf-8') + b'\n' and not result.stderr,
                        '原生 jq 二进制输出没有逐字保留测试输入')
                case['binary_preserves_original_bytes'] = True
    return evidence


def hook_completion_snapshot(events, thread_id, turn_id, expected):
    started, completed, turns = {}, {}, []
    ignored = 0
    for event in events:
        message = event.get('value')
        if event.get('channel') != 'stdout' or not isinstance(message, dict):
            continue
        method = message.get('method')
        if method not in ('hook/started', 'hook/completed', 'turn/completed'):
            continue
        params = message.get('params', {})
        related_turn = params.get('turn', {}).get('id') if method == 'turn/completed' else params.get('turnId')
        if params.get('threadId') != thread_id or related_turn != turn_id:
            ignored += 1
            continue
        if method == 'turn/completed':
            turn = params['turn']
            require(turn.get('status') == 'completed' and turn.get('error') is None,
                    '原生阻断回合没有正常结束')
            if turns:
                require(turns[0] == turn, '同一原生回合终态内容冲突')
            else:
                turns.append(turn)
            continue
        run = params.get('run', {})
        order = run.get('displayOrder')
        require(order in expected, '观察到未授权或重复注册的 hook')
        contract = expected[order]
        require(run.get('eventName') == contract['eventName'] and run.get('sourcePath') == contract['sourcePath']
                and isinstance(run.get('id'), str) and run['id'], 'hook 身份与原生注册表不一致')
        entry = {'id': run['id'], 'status': run.get('status'), 'eventName': run['eventName'],
                 'emitted_at_ms': message.get('emittedAtMs'), 'duration_ms': run.get('durationMs'),
                 'observed_elapsed_ms': event.get('observed_elapsed_ms')}
        target = completed if method == 'hook/completed' else started
        if order in target:
            require(target[order]['id'] == entry['id'] and target[order]['status'] == entry['status'],
                    '同一 hook 注册项出现冲突执行')
        target[order] = entry
        if method == 'hook/completed':
            require(run.get('status') == contract['expected_status'], '原生 hook 返回失败或非预期终态')
        else:
            require(run.get('status') == 'running', 'hook 开始事件没有运行状态')
    complete = (set(started) == set(expected) and set(completed) == set(expected) and len(turns) == 1)
    if complete:
        require(all(started[order]['id'] == completed[order]['id'] for order in expected),
                'hook 开始与完成没有关联到同一次执行')
    return {'complete': complete, 'started': list(started.values()), 'completed': list(completed.values()),
            'turn_completed': bool(turns), 'ignored_unrelated_events': ignored,
            'pending_display_orders': sorted(set(expected) - set(completed))}


def wait_for_hook_completion(recorder, thread_id, turn_id, native_hooks, blocker, requests, evidence):
    expected = {hook['displayOrder']: {'eventName': hook['eventName'], 'sourcePath': hook['sourcePath'],
        'expected_status': 'stopped' if hook['command'] == blocker else 'completed'}
        for hook in native_hooks if hook['eventName'] in ('sessionStart', 'userPromptSubmit')}
    require(sorted(item['expected_status'] for item in expected.values()) == ['completed', 'completed', 'stopped'],
            '等待对象不是两个通知 hook 和一个独立阻断 hook')
    started = time.monotonic()
    deadline = started + HOOK_COMPLETION_TIMEOUT
    wait_record = {'timeout_seconds': HOOK_COMPLETION_TIMEOUT, 'thread_id': thread_id, 'turn_id': turn_id}
    evidence['hook_completion_wait'] = wait_record
    try:
        while True:
            recorder.events_changed.clear()
            snapshot = hook_completion_snapshot(list(recorder.events), thread_id, turn_id, expected)
            wait_record.update(snapshot, elapsed_ms=round((time.monotonic() - started) * 1000),
                               process_exit_code=recorder.process.poll(), reader_errors=list(recorder.reader_errors),
                               model_http_requests=len(requests))
            require(not requests, '等待原生 hook 时出现模型 HTTP 请求')
            require(not recorder.reader_errors, '等待原生 hook 时读取输出失败')
            if snapshot['complete']:
                return
            require(recorder.process.poll() is None, '原生进程在完整 hook 终态前退出')
            remaining = deadline - time.monotonic()
            require(remaining > 0, '原生 hook 或阻断回合未在有界等待中完整结束')
            recorder.events_changed.wait(min(remaining, 0.25))
    except BaseException as error:
        wait_record['failure'] = {'type': type(error).__name__, 'message': str(error)}
        raise


class NativeRecorder:
    def __init__(self, command, env, directory, events):
        self.events = events
        self.reader_errors = []
        self.queue = queue.Queue()
        self.events_changed = threading.Event()
        started = time.monotonic()
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
                        self.events.append({'channel': channel, 'value': value,
                                            'observed_elapsed_ms': round((time.monotonic() - started) * 1000)})
                        self.events_changed.set()
                        if channel == 'stdout':
                            self.queue.put(value)
                except Exception as error:
                    self.reader_errors.append(str(error))
                finally:
                    self.events_changed.set()
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
        self.close_receipt = {'root_exited_naturally': False, 'termination_requested': False,
                              'kill_requested': False, 'output_readers_eof': False}
        try:
            self.process.stdin.close()
        except (BrokenPipeError, OSError):
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.close_receipt['termination_requested'] = True
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.close_receipt['kill_requested'] = True
                self.process.kill()
                self.process.wait(timeout=3)
        self.close_receipt.update(root_exit_code=self.process.returncode,
            root_exited_naturally=not self.close_receipt['termination_requested'])
        for reader in self.readers:
            reader.join(timeout=2)
        require(all(not reader.is_alive() for reader in self.readers), '原生输出读取没有结束')
        for stream in (self.process.stdout, self.process.stderr):
            stream.close()
        require(not self.reader_errors, '原生输出读取失败: ' + repr(self.reader_errors))
        self.close_receipt['output_readers_eof'] = True


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


def capture_case_markers(directory, evidence):
    markers = {}
    for script in ('on-session-start.sh', 'on-prompt-submit.sh'):
        path = directory / (script + '.json')
        if path.exists():
            require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 65536,
                    '原生 hook marker 不是受控小文件')
            markers[script] = json.loads(path.read_text(encoding='utf-8'))
    evidence['markers'] = markers
    return markers


def prepare_case(args, directory, evidence, mode):
    from apply_notification_patch import apply_files, validate_tree
    metadata, replacements, source_hash = probe_bundle(args.repo, mode)
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
    evidence.update(mode=mode, patch_revision=metadata['patch_revision'],
                    source_metadata_sha256=source_hash, resource_tree_sha256=exact_plugin_tree(plugin, metadata))
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
    return cli_home, env, marketplace, plugin, config


def install_case(args, directory, env, marketplace):
    for command in (['plugin', 'marketplace', 'add', str(marketplace), '--json'],
                    ['plugin', 'add', 'warp@codex-warp', '--json']):
        subprocess.run([str(args.codex_executable), *command], env=env, cwd=directory,
                       capture_output=True, check=True, timeout=30)


def close_case(recorder, evidence):
    # 保留首个失败，同时明确记录清理是否完成；关闭失败不能计入通过。
    primary = sys.exc_info()[1]
    try:
        recorder.close()
        evidence.setdefault('process_closes', []).append(dict(recorder.close_receipt,
            all_descendants_job_verified=False))
        require(recorder.close_receipt['root_exited_naturally'] and recorder.process.returncode == 0,
                '原生 app-server 未正常退出，不能把监督终止计入通过')
    except BaseException as error:
        evidence['close_failure'] = {'type': type(error).__name__, 'message': str(error)}
        evidence['failed_close_receipt'] = getattr(recorder, 'close_receipt', {})
        if primary is None:
            raise


def rollback_case(config, before, written, evidence):
    primary = sys.exc_info()[1]
    try:
        restore_config(config, before, written, evidence.setdefault('config_rollback', {}))
    except BaseException as error:
        evidence['config_rollback_failure'] = {'type': type(error).__name__, 'message': str(error)}
        if primary is None:
            raise


def formal_registration(args, directory, evidence):
    evidence.update(passed=False, traces=[], scope='uninstrumented_formal_five_hooks',
                    model_generation_requested_by_probe=False, native_hook_events_verified=[],
                    native_hook_events_not_verified=list(EVENTS))
    cli_home, env, marketplace, plugin, config = prepare_case(args, directory, evidence, 'formal')
    install_case(args, directory, env, marketplace)
    before = config.read_bytes()
    recorder = start_codex(args.codex_executable, env, directory, evidence['traces'])
    try:
        hooks = native_hooks(recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 2))
        evidence['untrusted_hooks'] = hooks
        validate_native_hooks(hooks, cli_home, plugin, {encode(script) for script in SCRIPTS}, 'untrusted')
        require({hook['eventName'] for hook in hooks} == set(EVENTS), '正式五项事件集合改变')
        require(config.read_bytes() == before, '未授权注册检查修改了隔离配置')
    finally:
        close_case(recorder, evidence)
    require(config.read_bytes() == before, '未授权 app-server 退出时改变了隔离配置')
    before, written = authorize_config(config, hooks)
    try:
        recorder = start_codex(args.codex_executable, env, directory, evidence['traces'])
        try:
            trusted = native_hooks(recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 3))
            evidence['trusted_hooks'] = trusted
            validate_native_hooks(trusted, cli_home, plugin, {encode(script) for script in SCRIPTS}, 'trusted')
            same_native_registration(hooks, trusted)
            require(tree(plugin) == evidence['resource_tree_sha256'], '正式来源字节在采集中改变')
            require(config.read_bytes() == written, '正式信任检查修改了隔离配置')
        finally:
            close_case(recorder, evidence)
    finally:
        rollback_case(config, before, written, evidence)
    evidence.update(passed=True, hook_count=5, scripts_instrumented=False,
                    source_manifest_modified=False, native_trust_confirmed=True,
                    no_thread_or_turn_started=True, windows_product_enabled=False)
    return hooks


def cleanup_private_cases(directory, cases, evidence):
    evidence.update(attempted=False, deleted=False, descendants_job_verified=False)
    require(cases and all(case['passed'] for case in cases), '失败现场不能自动清理')
    for case in cases:
        phases = [case, case['formal_registration']] if case.get('mode') == 'formal' else [case]
        for phase in phases:
            closes = phase.get('process_closes', [])
            require(len(closes) == 2 and all(close['root_exited_naturally'] and close['root_exit_code'] == 0
                    and close['output_readers_eof'] for close in closes) and not phase.get('close_failure')
                    and phase.get('config_rollback', {}).get('restored'), '缺少退出、EOF 或配置回滚证据，保留现场')
    require(directory.is_dir() and not directory.is_symlink(), '私有临时目录已被替换，拒绝清理')
    evidence['attempted'] = True
    try:
        # 不更改未知只读属性；任何文件占用或权限错误都保留原始失败。
        shutil.rmtree(directory)
        evidence['deleted'] = True
    except Exception as error:
        evidence['failure'] = {'type': type(error).__name__, 'message': str(error),
                               'winerror': getattr(error, 'winerror', None), 'filename': getattr(error, 'filename', None)}
        raise


def verify_case_resources(repo, cases, mode):
    metadata, _, source_hash = probe_bundle(repo, mode)
    expected = dict(metadata['compatible_bases'][0]['tree_sha256'])
    expected.update({name: entry['replacement_sha256'] for name, entry in metadata['files'].items()})
    for case in cases:
        require(case['source_metadata_sha256'] == source_hash and case['resource_tree_sha256'] == expected,
                '探测期间配方或正式资源发生变化')


def instrument_candidate(plugin, evidence):
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
    hooks_path.write_text(json.dumps(hooks), encoding='utf-8')
    original_hashes = {}
    for script in ('on-session-start.sh', 'on-prompt-submit.sh'):
        path = plugin / 'scripts' / script
        original_hashes[script] = hashlib.sha256(path.read_bytes()).hexdigest()
        original = path.with_name(script + '.fixture-original')
        path.rename(original)
        entry = original.name
        if script == 'on-prompt-submit.sh':
            candidate, evidence['query_binary_candidate'] = binary_query_candidate(original.read_text(encoding='utf-8'))
            entry = script + '.fixture-candidate'
            original.with_name(entry).write_text(candidate, encoding='utf-8', newline='\n')
        # 插桩只发生于临时副本；原文件逐字保留，query 候选的唯一改动和摘要另行记录。
        wrapper = '''#!/bin/bash
set -e
input=$(cat)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
printf '%s' "$input" | jq -c --arg root "$PLUGIN_ROOT" '{hook_event_name,turn_id,session_id,cwd,prompt,plugin_root:$root}' > "$INFINISHELL_HOOK_PROBE_CASE/__FIXTURE_SCRIPT__.json"
printf '%s' "$input" | bash -- "$SCRIPT_DIR/__FIXTURE_ENTRY__"
'''.replace('__FIXTURE_SCRIPT__', script).replace('__FIXTURE_ENTRY__', entry)
        path.write_text(wrapper, encoding='utf-8', newline='\n')
    return original_hashes


def one_case(args, directory, requests, evidence, prompt=HOOK_PROMPTS[0]):
    mode = getattr(args, 'mode', 'formal')
    formal = None
    if mode == 'formal':
        evidence['formal_registration'] = {}
        formal = formal_registration(args, directory / 'formal-registration', evidence['formal_registration'])
    cli_home, env, marketplace, plugin, config = prepare_case(args, directory, evidence, mode)
    if formal is not None:
        registration = evidence['formal_registration']
        require(evidence['resource_tree_sha256'] == registration['resource_tree_sha256']
                and evidence['source_metadata_sha256'] == registration['source_metadata_sha256'],
                '正式注册与触发验证的资源不是同一版字节')
    original_hashes = instrument_candidate(plugin, evidence) if mode == 'candidate' else {}
    hooks_path = plugin / 'hooks/hooks.json'
    hooks = json.loads(hooks_path.read_bytes())
    # 第六项仅阻断本次测试；正式五项摘要只能来自上面未改动资源的独立注册。
    stop_source = "[Console]::Out.WriteLine('{\"continue\":false,\"stopReason\":\"isolated hook probe\"}')"
    blocker = encode_source(stop_source)
    hooks['hooks']['UserPromptSubmit'].append({'hooks': [{'type': 'command', 'command': 'false',
                                                        'commandWindows': blocker}]})
    hooks_path.write_text(json.dumps(hooks), encoding='utf-8')
    expected_tree = tree(plugin)
    evidence['temporary_hooks_sha256'] = expected_tree['hooks/hooks.json']
    if formal is not None:
        require({name: digest for name, digest in expected_tree.items() if name != 'hooks/hooks.json'} ==
                {name: digest for name, digest in evidence['resource_tree_sha256'].items() if name != 'hooks/hooks.json'},
                '正式触发验证不得修改脚本或增加插桩文件')
        evidence['scripts_instrumented'] = False
    install_case(args, directory, env, marketplace)
    initial_config = config.read_bytes()
    recorder = start_codex(args.codex_executable, env, directory, evidence['traces'])
    try:
        listed = recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 2)
        initial_hooks = native_hooks(listed)
        evidence['native_initial_hooks'] = initial_hooks
        installed_root = validate_native_hooks(initial_hooks, cli_home, plugin,
                                              {encode(script) for script in SCRIPTS} | {blocker}, 'untrusted')
        if formal is not None:
            evidence['trigger_validation'] = split_trigger_hooks(formal, initial_hooks, blocker)
    finally:
        close_case(recorder, evidence)
    require(config.read_bytes() == initial_config, '未授权 app-server 改变了隔离配置')
    before, written = authorize_config(config, initial_hooks)
    evidence['expected_prompt'] = prompt
    recorder = None
    try:
        recorder = start_codex(args.codex_executable, env, directory, evidence['traces'])
        trusted_hooks = native_hooks(recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 3))
        validate_native_hooks(trusted_hooks, cli_home, plugin, {encode(script) for script in SCRIPTS} | {blocker}, 'trusted')
        same_native_registration(initial_hooks, trusted_hooks)
        if formal is not None:
            evidence['trigger_validation'] = split_trigger_hooks(formal, trusted_hooks, blocker)
        thread = recorder.rpc('thread/start', {'cwd': str(directory), 'sessionStartSource': 'startup',
            'ephemeral': False, 'sandbox': 'read-only', 'approvalPolicy': 'on-request',
            'modelProvider': 'local_hook_probe'}, 4)
        require(thread['modelProvider'] == 'local_hook_probe', '原生会话没有使用隔离的本机 provider')
        turn = recorder.rpc('turn/start', {'threadId': thread['thread']['id'], 'input': [
            {'type': 'text', 'text': prompt, 'text_elements': []}]}, 5)
        if formal is not None:
            evidence['transport_expectations'] = formal_transport_expectations(
                directory, thread['thread']['id'], turn['turn']['id'], prompt)
        wait_for_hook_completion(recorder, thread['thread']['id'], turn['turn']['id'], trusted_hooks,
                                 blocker, requests, evidence)
        if formal is not None:
            evidence['trigger_validation'].update(native_events_verified=['sessionStart', 'userPromptSubmit'],
                native_events_not_verified=['stop', 'permissionRequest', 'postToolUse'],
                notification_transport_verified=False, scripts_match_formal_resource=True)
        else:
            markers = capture_case_markers(directory, evidence)
            require(markers['on-session-start.sh']['hook_event_name'] == 'SessionStart', 'SessionStart 未原生执行')
            require(markers['on-prompt-submit.sh']['prompt'] == prompt, '原生 prompt 字节语义被更改')
            require(markers['on-prompt-submit.sh']['hook_event_name'] == 'UserPromptSubmit' and
                    markers['on-prompt-submit.sh']['session_id'] == thread['thread']['id'] and
                    markers['on-prompt-submit.sh']['turn_id'] == turn['turn']['id'], '原生输入关联字段丢失')
            require(all(Path(marker['plugin_root']) == installed_root for marker in markers.values()),
                    'PLUGIN_ROOT 没有指向原生已安装插件')
            require(all(hashlib.sha256((installed_root / 'scripts' / (s + '.fixture-original')).read_bytes()).hexdigest() == h
                        for s, h in original_hashes.items()), '插桩后的原脚本没有逐字保留')
        require(tree(installed_root) == expected_tree and tree(plugin) == expected_tree,
                '触发后插件来源或原生缓存被修改')
        require(not requests, f'阻断前出现模型 HTTP 请求: {requests}')
        require(not (directory / 'INJECTED').exists(), '目录数据被错误作为 shell 源码执行')
        require(config.read_bytes() == written, '原生执行意外修改了已核对的隔离配置')
        return {'name': directory.name, 'native_session_start': True, 'native_user_prompt_submit': True,
                'native_prompt_blocked': True, 'native_hooks': trusted_hooks,
                'model_http_requests': len(requests), 'model_generation_verified': False,
                'native_conpty_notifications_verified': False, 'full_lifecycle_verified': False}
    finally:
        try:
            if recorder is not None:
                close_case(recorder, evidence)
        finally:
            rollback_case(config, before, written, evidence)
            if mode == 'candidate':
                try:
                    capture_case_markers(directory, evidence)
                except Exception as error:
                    evidence['marker_capture_failure'] = {'type': type(error).__name__, 'message': str(error)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=('formal', 'candidate'), default='formal')
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
    report = {'schema_version': 2, 'mode': args.mode, 'passed': False, 'official_commit': CODEX_COMMIT, 'cases': [], 'checks': {},
              'scope': args.mode + '_native_registration_and_two_blocked_hook_events',
              'credentials_provided': False, 'windows_product_enabled': False, 'native_conpty_notifications_verified': False,
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
        phase = 'jq_newline_boundary'
        report['jq_newline_boundary'] = {}
        verify_jq_newline_boundary(args.native_environment, report['jq_newline_boundary'])

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
        private = Path(tempfile.mkdtemp(prefix='infinishell-native-windows-hooks-'))
        report['private_dir'] = str(private)
        for name, prompt in zip(('插件 空 格', "插件 ' $(touch INJECTED) `touch INJECTED` & %PATH% !name! ^ ()"), HOOK_PROMPTS):
            evidence = {'name': name, 'passed': False, 'traces': []}
            report['cases'].append(evidence)
            evidence.update(one_case(args, private / name, requests, evidence, prompt))
            evidence['passed'] = True
        phase = 'zero_model_requests'
        require(not requests, f'原生进程关闭前出现模型 HTTP 请求: {requests}')
        report['checks'][phase] = True
        phase = 'source_inputs_unchanged'
        verify_plugin(args.upstream_plugin, plugin_base(args.repo))
        verify_case_resources(args.repo, report['cases'], args.mode)
        require(verify_encoding() == report['checks']['encoding'], '探测期间固定源码被并发修改')
        report['checks'][phase] = True
        phase = 'private_directory_cleanup'
        report['private_directory_cleanup'] = {}
        cleanup_private_cases(private, report['cases'], report['private_directory_cleanup'])
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
