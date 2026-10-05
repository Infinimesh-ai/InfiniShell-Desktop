"""原生 PS 5.1 通知协议与真实 ConPTY 回归；离线检查不计作 Windows 实测。"""

import base64
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
from codex_windows_hook_command import (
    BUNDLE, PREFIX, SCRIPTS, encode, encode_legacy, native_windows_environment,
    source_text, verify_windows_syntax,
)

REPO = Path(__file__).resolve().parents[2]
FILES = ('on-notification.ps1', 'warp-notify.ps1')
OSC = re.compile(rb'\x1b\]777;notify;warp://cli-agent;(.*?)\x07', re.DOTALL)
SPECIAL_ROOT = "插件 😀 ' $(touch INJECTED) `touch INJECTED` & %PATH% !name! ^ ()"


def private_run(command, environment, directory, payload=None, *, no_console=True, timeout=45):
    """暂停创建后先加入私有 Job；超时仅回收本次任务的进程树。"""
    import _winapi
    from probe_codex_plugin_cache_refresh import WindowsProbeJob, suspended_creation
    job, process, trace = WindowsProbeJob(), None, {}
    try:
        with suspended_creation(_winapi, job, command, environment, directory, trace):
            process = subprocess.Popen(command, env=environment, cwd=directory,
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE,
                                       creationflags=subprocess.CREATE_NO_WINDOW if no_console else 0)
        stdout, stderr = process.communicate(payload, timeout=timeout)
        if job.wait_empty(5) != 0:
            raise AssertionError('原生通知子进程未自然退出')
        return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    finally:
        if job.active():
            job.terminate()
            if job.wait_empty(5) != 0:
                raise AssertionError('私有 Job 回收未完成')
        job.close()
        if process is not None:
            process.wait(timeout=5)
            for pipe in (process.stdin, process.stdout, process.stderr):
                pipe.close()


def hook_command(script, environment):
    return [environment['COMSPEC'], '/C', encode(script)]


def native_environment(plugin):
    environment = native_windows_environment()
    # 只留下 PS 5.1；即使 System32 装有 WSL bash.exe 也不能成为可用依赖。
    environment['PATH'] = str(Path(environment['SYSTEMROOT']) / 'System32/WindowsPowerShell/v1.0')
    environment.update(PLUGIN_ROOT=str(plugin), WARP_CLI_AGENT_PROTOCOL_VERSION='1',
                       WARP_CLIENT_VERSION='native-notification-test')
    return environment


class StaticNativeNotificationTests(unittest.TestCase):
    def test_native_launcher_has_no_shell_dependency_and_keeps_fixed_encoded_arguments(self):
        source = source_text()
        self.assertNotIn('Get-Command bash.exe', source)
        self.assertNotIn('Get-Command jq.exe', source)
        self.assertIn("Join-Path $PSHOME 'powershell.exe'", source)
        self.assertIn("'scripts/on-notification.ps1'", source)
        self.assertIn('ReparsePoint', source)
        for script in SCRIPTS:
            command = encode(script)
            self.assertLessEqual(len(command), 8000)
            self.assertTrue(command.isascii())
            self.assertFalse(any(character in command for character in '%!$`&^()'))
            decoded = base64.b64decode(command[len(PREFIX):]).decode('utf-16le')
            self.assertEqual(decoded, "$NotificationHook = '" + script + "'\n" + source)

    def test_legacy_candidate_keeps_frozen_shell_launcher(self):
        for script in SCRIPTS:
            legacy = encode_legacy(script)
            decoded = base64.b64decode(legacy[len(PREFIX):]).decode('utf-16le')
            self.assertIn('Get-Command bash.exe', decoded)
            self.assertNotIn('on-notification.ps1', decoded)
            self.assertNotEqual(legacy, encode(script))

    def test_writer_reuses_reviewed_native_console_implementation(self):
        legacy = Path(__file__).with_name('codex_windows_notify.ps1').read_text(encoding='utf-8')
        writer = (BUNDLE / 'scripts/warp-notify.ps1').read_text(encoding='utf-8')
        extract = lambda text: text.split("Add-Type -TypeDefinition @'\n", 1)[1].split("\n'@", 1)[0]
        self.assertEqual(extract(writer), extract(legacy))
        self.assertNotIn('SetConsoleMode', writer)
        self.assertNotIn('[Console]::Out', writer)
        for name in FILES:
            contents = (BUNDLE / 'scripts' / name).read_bytes()
            self.assertFalse(contents.startswith(b'\xef\xbb\xbf'))
            self.assertNotIn(b'\r\n', contents)


@unittest.skipUnless(os.name == 'nt', '必须在 Windows PS 5.1 执行，离线检查不替代原生证据')
class NativePowerShellNotificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        verify_windows_syntax(native_windows_environment())

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='codex-native-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.plugin = self.root / SPECIAL_ROOT
        (self.plugin / 'scripts').mkdir(parents=True)
        for name in FILES:
            shutil.copyfile(BUNDLE / 'scripts' / name, self.plugin / 'scripts' / name)
        self.environment = native_environment(self.plugin)
        self.assertIsNone(shutil.which('bash.exe', path=self.environment['PATH']))
        self.assertIsNone(shutil.which('jq.exe', path=self.environment['PATH']))

    def record_payload(self, script, value):
        # 仅协议单测替换 writer；真实设备写入另由未修改脚本的 ConPTY 测试验证。
        (self.plugin / 'scripts/warp-notify.ps1').write_text(
            'function Write-InfiniShellNotification([string] $Message) { '
            '[Console]::OpenStandardOutput().Write([Text.Encoding]::ASCII.GetBytes($Message), 0, $Message.Length) }\n',
            encoding='utf-8')
        result = self.run_hook(script, json.dumps(value, ensure_ascii=True).encode('ascii'))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b'')
        self.assertTrue(result.stdout.isascii())
        self.assertFalse(result.stdout.startswith(b'\xef\xbb\xbf'))
        self.assertFalse((self.plugin / 'INJECTED').exists())
        matches = OSC.findall(result.stdout)
        if not matches:
            self.assertEqual(result.stdout, b'')
            return None
        self.assertEqual(len(matches), 1)
        self.assertEqual(OSC.fullmatch(result.stdout).group(1), matches[0])
        return json.loads(matches[0])

    def run_hook(self, script, payload, environment=None):
        environment = self.environment if environment is None else environment
        return private_run(hook_command(script, environment), environment, self.root, payload)

    def test_all_five_events_keep_local_protocol_and_native_turn_fields(self):
        common = {'session_id': '会话😀', 'turn_id': '回合😀', 'cwd': 'C:\\项目 空格\\demo',
                  'prompt_id': 'do-not-forward', 'event_id': 'do-not-forward', 'sequence': 9}
        cases = (
            ('on-session-start.sh', {}, {'event': 'session_start', 'plugin_version': '0.4.0'}),
            ('on-prompt-submit.sh', {'prompt': '中文😀\nEnglish\r\n$() %PATH% !x! & `'},
             {'event': 'prompt_submit', 'query': '中文😀\nEnglish\r\n$() %PATH% !x! & `'}),
            ('on-stop.sh', {'last_assistant_message': '完成😀', 'transcript_path': 'C:\\日志\\a.jsonl'},
             {'event': 'stop', 'response': '完成😀', 'transcript_path': 'C:\\日志\\a.jsonl'}),
            ('on-permission-request.sh', {'tool_name': 'shell', 'tool_input': {'command': 'echo 中文😀', 'args': ['x', 1]}},
             {'event': 'permission_request', 'tool_name': 'shell',
              'tool_input': {'command': 'echo 中文😀', 'args': ['x', 1]}, 'summary': 'Wants to run shell: echo 中文😀'}),
            ('on-post-tool-use.sh', {'tool_name': 'shell'}, {'event': 'tool_complete', 'tool_name': 'shell'}),
        )
        for script, extra, expected in cases:
            with self.subTest(script=script):
                self.assertEqual(self.record_payload(script, {**common, **extra}),
                                 {'v': 1, 'agent': 'codex', 'session_id': '会话😀', 'turn_id': '回合😀',
                                  'cwd': 'C:\\项目 空格\\demo', 'project': 'demo', **expected})

    def test_query_and_response_truncation_never_split_surrogate_pairs(self):
        text = 'a' * 196 + '😀' + 'bcde'
        expected = 'a' * 196 + '😀...'
        query = self.record_payload('on-prompt-submit.sh', {'prompt': text})
        stop = self.record_payload('on-stop.sh', {'last_assistant_message': text, 'turn_id': 'turn'})
        self.assertEqual(query['query'], expected)
        self.assertEqual(stop['response'], expected)
        self.assertEqual(len(query['query']), 200)
        self.assertEqual(self.record_payload('on-prompt-submit.sh', {'prompt': '😀' * 200})['query'], '😀' * 200)

    def test_stop_schema_never_promotes_missing_or_untyped_native_turn(self):
        for turn in (None, '', False, True, 123, [], {}):
            with self.subTest(turn=turn):
                payload = self.record_payload('on-stop.sh', {'last_assistant_message': 'done', 'turn_id': turn})
                self.assertEqual(payload['event'], 'notification')
                self.assertTrue(payload['terminal_unverified'])
                self.assertEqual(payload['error_type'], 'uncorrelated_hook')
                self.assertNotIn('turn_id', payload)
        for value in (None, '', '\n\n', False, 123, [], {}):
            with self.subTest(response=value):
                payload = self.record_payload('on-stop.sh', {'last_assistant_message': value, 'turn_id': 'turn'})
                self.assertEqual(payload['event'], 'notification')
                self.assertEqual(payload['response'], '')
                self.assertNotIn('terminal_unverified', payload)
        for active in (True, 'true'):
            self.assertIsNone(self.record_payload('on-stop.sh', {'stop_hook_active': active, 'last_assistant_message': 'done'}))

    def test_missing_fields_and_false_defaults_match_shell_recipe(self):
        self.assertEqual(self.record_payload('on-prompt-submit.sh', {'prompt': False})['query'], '')
        permission = self.record_payload('on-permission-request.sh', {})
        self.assertEqual(permission['tool_input'], {})
        self.assertEqual(permission['summary'], 'Wants to run unknown: null')
        self.assertEqual(permission['session_id'], '')
        self.assertEqual(permission['cwd'], '')
        self.assertEqual(permission['project'], '')
        self.assertEqual(self.record_payload('on-post-tool-use.sh', {})['tool_name'], '')

    def test_control_characters_stay_in_json_and_trailing_lf_matches_shell(self):
        value = '前😀\x1b]777;notify;evil;{}\x07\r\n后\n'
        self.assertEqual(self.record_payload('on-prompt-submit.sh', {'prompt': value})['query'], value[:-1])
        self.assertEqual(self.record_payload('on-prompt-submit.sh', {'prompt': '文字\r\n'})['query'], '文字\r')
        self.assertEqual(self.record_payload('on-prompt-submit.sh', {'prompt': r'literal \ud800'})['query'], r'literal \ud800')

    def test_gate_disabled_does_not_consume_or_parse_input(self):
        for key in ('WARP_CLI_AGENT_PROTOCOL_VERSION', 'WARP_CLIENT_VERSION'):
            environment = {**self.environment, key: ''}
            result = self.run_hook('on-session-start.sh', b'not json', environment)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b'', b''))

    def test_malformed_utf8_size_and_surrogates_fail_without_osc(self):
        values = [b'\xff', b'[]', b'null', b'{', b' ' * 1048577]
        for field in ('session_id', 'cwd', 'turn_id', 'tool_name', 'tool_input', 'prompt'):
            values.append(json.dumps({field: '\ud800'}).encode('ascii'))
            values.append(json.dumps({field: '\udc00'}).encode('ascii'))
        values.append(b'{"tool_input":{"nested":["\\ud800"]}}')
        for payload in values:
            with self.subTest(length=len(payload), prefix=payload[:60]):
                result = self.run_hook('on-permission-request.sh', payload)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b'')
                self.assertIn(b'native_notification_failed', result.stderr)

    def test_absent_console_is_failure_without_stdout_fallback(self):
        result = self.run_hook('on-session-start.sh', b'{"session_id":"no-console"}')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b'')
        self.assertIn(b'native_notification_failed', result.stderr)

    def test_install_preflight_uses_only_system_powershell_and_cmd(self):
        # 安装预检还需解析 cmd.exe；System32 可能存在 WSL bash，但不能引入 Git Bash/jq。
        environment = native_windows_environment()
        system = Path(environment['SYSTEMROOT']) / 'System32'
        self.assertEqual(environment['PATH'], os.pathsep.join(map(str, (system, system / 'WindowsPowerShell/v1.0'))))
        self.assertIsNone(shutil.which('jq.exe', path=environment['PATH']))
        command = [str(system / 'WindowsPowerShell/v1.0/powershell.exe'), '-NoLogo', '-NoProfile',
                   '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File',
                   str(BUNDLE / 'windows-install-preflight.ps1')]
        result = private_run(command, environment, self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, b'infinishell-codex-windows-native-notifications-v2\r\n')
        self.assertEqual(result.stderr, b'')

    def test_relative_root_and_reparse_root_fail_before_execution(self):
        for value in ('relative', 'C:relative', '\\relative'):
            result = self.run_hook('on-session-start.sh', b'{}', {**self.environment, 'PLUGIN_ROOT': value})
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b'')
            self.assertIn(b'native_launcher_failed', result.stderr)
        junction = self.root / 'junction'
        result = private_run([self.environment['COMSPEC'], '/D', '/C', 'mklink', '/J', str(junction), str(self.plugin)],
                             self.environment, self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        try:
            result = self.run_hook('on-session-start.sh', b'{}', {**self.environment, 'PLUGIN_ROOT': str(junction)})
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b'native_launcher_failed', result.stderr)
        finally:
            junction.rmdir()

    def test_real_conpty_receives_all_five_notifications_with_clean_standard_streams(self):
        host = self.root / 'host'
        (host / 'x64').mkdir(parents=True)
        for source, destination in (('conpty.dll', 'conpty.dll'), ('OpenConsole.exe', 'x64/OpenConsole.exe')):
            shutil.copyfile(REPO / 'app/assets/windows/x64' / source, host / destination)
        configuration = self.root / 'configuration.json'
        configuration.write_text(json.dumps({'plugin': str(self.plugin), 'root': str(self.root), 'host': str(host)}), encoding='utf-8')
        result = private_run([sys.executable, '-B', str(Path(__file__).resolve()), '--conpty-host', str(configuration)],
                             self.environment, self.root, timeout=150)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads((self.root / 'driver.json').read_text(encoding='utf-8'))
        self.assertEqual(report['powershell_version'], '5.1')
        self.assertEqual(report['exit_codes'], [0, 0, 0, 0, 0])
        self.assertEqual(report['stdout'], ['', '', '', '', ''])
        self.assertEqual(report['stderr'], ['', '', '', '', ''])
        raw = (self.root / 'console.bin').read_bytes()
        notifications = [json.loads(value) for value in OSC.findall(raw)]
        self.assertEqual([item['event'] for item in notifications],
                         ['session_start', 'prompt_submit', 'stop', 'permission_request', 'tool_complete'])
        self.assertEqual(notifications[1]['query'], '中文😀\nEnglish\r\n$() ` %PATH% ! &')
        self.assertTrue(all(item['session_id'] == '原生😀' for item in notifications))
        self.assertFalse((self.plugin / 'INJECTED').exists())


def console_driver(configuration):
    config = json.loads(configuration.read_text(encoding='utf-8'))
    root, plugin = Path(config['root']), Path(config['plugin'])
    environment = native_environment(plugin)
    command = ['powershell.exe', '-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
               "[Console]::Out.Write(('{0}.{1}' -f $PSVersionTable.PSVersion.Major,$PSVersionTable.PSVersion.Minor))"]
    version = private_run(command, environment, root, no_console=False)
    if version.returncode or version.stdout != b'5.1':
        raise AssertionError('真实控制台 driver 未使用 PS 5.1')
    data = {'session_id': '原生😀', 'turn_id': 'turn-native', 'cwd': str(plugin),
            'prompt': '中文😀\nEnglish\r\n$() ` %PATH% ! &', 'last_assistant_message': '完成😀',
            'tool_name': 'shell', 'tool_input': {'command': 'echo 中文😀'}}
    report = {'powershell_version': '5.1', 'exit_codes': [], 'stdout': [], 'stderr': []}
    for script in SCRIPTS:
        result = private_run(hook_command(script, environment), environment, root,
                             json.dumps(data, ensure_ascii=False).encode('utf-8'), no_console=False)
        report['exit_codes'].append(result.returncode)
        report['stdout'].append(base64.b64encode(result.stdout).decode('ascii'))
        report['stderr'].append(base64.b64encode(result.stderr).decode('ascii'))
    (root / 'driver.json').write_text(json.dumps(report), encoding='utf-8')


def conpty_host(configuration):
    from probe_claude_windows_notifications import run_contained_conpty
    config = json.loads(configuration.read_text(encoding='utf-8'))
    root, host = Path(config['root']), Path(config['host'])
    command = [sys.executable, '-B', str(Path(__file__).resolve()), '--console-driver', str(configuration)]
    result = run_contained_conpty(host / 'conpty.dll', command, native_environment(Path(config['plugin'])),
                                 root, root / 'console.bin', 120, {})
    if result['exit_code'] != 0 or not result['console_close_and_output_eof_confirmed']:
        raise AssertionError('真实 ConPTY driver 或输出回收失败')


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--console-driver':
        console_driver(Path(sys.argv[2]))
    elif len(sys.argv) == 3 and sys.argv[1] == '--conpty-host':
        conpty_host(Path(sys.argv[2]))
    else:
        unittest.main()
