"""Windows 通知候选的编码与 Unix 保持回归；原生控制台必须由 ConPTY 探针验收。"""

import base64
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import codex_windows_notify as candidate
import tmux_notification_tests as unix_probe
from codex_windows_hook_command import SCRIPTS, encode

ASSETS = Path(__file__).resolve().parents[2] / 'app/assets/bundled/cli-agent-plugins'


class NotifyCandidateTests(unittest.TestCase):
    def original(self):
        return (ASSETS / 'codex/revisions/rev3/scripts/warp-notify.sh').read_text(encoding='utf-8')

    def test_rev6_resources_keep_reviewable_windows_payload_and_five_native_commands(self):
        notify = (ASSETS / 'codex/scripts/warp-notify.sh').read_text(encoding='utf-8')
        encoded = re.findall(r'-EncodedCommand ([A-Za-z0-9+/=]+)', notify)
        self.assertEqual(len(encoded), 1)
        self.assertEqual(base64.b64decode(encoded[0], validate=True).decode('utf-16le'),
                         candidate.source_text())
        self.assertNotIn('2>/dev/null || true', notify)
        self.assertIn('windows_console_write_failed', notify)
        self.assertIn('windows_console_not_found', notify)
        hooks = json.loads((ASSETS / 'codex/hooks/hooks.json').read_text())['hooks']
        handlers = [handler for groups in hooks.values() for group in groups for handler in group['hooks']]
        self.assertEqual(len(handlers), 5)
        self.assertEqual({handler['commandWindows'] for handler in handlers}, {encode(name) for name in SCRIPTS})
        self.assertNotIn('continue', json.dumps(hooks))
        for name in ('hooks/hooks.json', 'scripts/warp-notify.sh', 'scripts/on-prompt-submit.sh'):
            self.assertEqual((ASSETS / 'codex' / name).read_bytes(),
                             (ASSETS / 'codex/source/plugins/warp' / name).read_bytes())
        metadata = json.loads((ASSETS / 'codex/PATCH_METADATA.json').read_text())
        self.assertEqual(metadata['patch_revision'], 6)
        self.assertFalse(metadata['windows_product_enabled'])
        self.assertFalse(metadata['windows_uninstrumented_hooks_verified'])

    @unittest.skipUnless(os.name == 'posix', 'Unix 兼容分支由真实 Bash/jq 回放')
    def test_unix_query_keeps_lf_crlf_without_requiring_jq_binary_option(self):
        real_jq = shutil.which('jq')
        self.assertIsNotNone(real_jq)
        with tempfile.TemporaryDirectory(prefix='rev4-query-') as temporary:
            root = Path(temporary)
            scripts = root / 'scripts'
            shutil.copytree(ASSETS / 'codex/source/plugins/warp/scripts', scripts)
            (scripts / 'warp-notify.sh').write_text('#!/bin/bash\nprintf "%s" "$2"\n')
            (scripts / 'warp-notify.sh').chmod(0o755)
            shim = root / 'bin'
            shim.mkdir()
            (shim / 'jq').write_text('#!/bin/bash\nfor arg do [ "$arg" != "--binary" ] || exit 93; done\nexec "$REV4_REAL_JQ" "$@"\n')
            (shim / 'jq').chmod(0o755)
            environment = {**os.environ, 'PATH': str(shim) + os.pathsep + os.environ['PATH'],
                           'REV4_REAL_JQ': real_jq, 'WARP_CLI_AGENT_PROTOCOL_VERSION': '1',
                           'WARP_CLIENT_VERSION': 'rev4-test'}
            for prompt in ('中文\nEnglish', '中文\r\nEnglish', r'字面 \r\n'):
                result = subprocess.run(['bash', str(scripts / 'on-prompt-submit.sh')],
                    input=json.dumps({'prompt': prompt, 'session_id': 'session', 'turn_id': 'turn', 'cwd': str(root)}).encode(),
                    env=environment, capture_output=True, check=True, timeout=10)
                self.assertEqual(json.loads(result.stdout)['query'], prompt)

    def test_encoded_program_roundtrip_and_bash_syntax(self):
        body = (ASSETS / 'codex/scripts/warp-notify.sh').read_text(encoding='utf-8')
        encoded = re.findall(r'-EncodedCommand ([A-Za-z0-9+/=]+)', body)
        self.assertEqual(len(encoded), 1)
        self.assertEqual(base64.b64decode(encoded[0], validate=True).decode('utf-16le'), candidate.source_text())
        bash = shutil.which('bash')
        if os.name == 'nt':
            candidates = [Path(bash)] if bash else []
            candidates += [Path(os.environ.get('PROGRAMFILES', '')) / 'Git/usr/bin/bash.exe',
                           Path(os.environ.get('LOCALAPPDATA', '')) / 'Programs/Git/usr/bin/bash.exe']
            bash = next((str(path) for path in candidates if path.is_file()
                         and (path.parent / 'msys-2.0.dll').is_file()), None)
        self.assertIsNotNone(bash)
        result = subprocess.run([bash, '-n'], input=body.encode(), capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_unknown_transport_recipe_is_rejected_before_replacing_it(self):
        for body in ('', self.original().replace('/dev/tty', '/other'), self.original() + self.original()):
            with self.subTest(body_length=len(body)), self.assertRaises(ValueError):
                candidate.candidate_notify(body)

    def test_source_line_endings_do_not_change_encoded_program(self):
        source = candidate.source_text()
        with patch.object(Path, 'read_text', return_value=source.replace('\n', '\r\n')):
            self.assertEqual(candidate.source_text(), source)

    @unittest.skipUnless(os.name == 'posix', '此项需要真实 Unix 控制终端，不能代替 Windows 原生控制台验证')
    def test_direct_control_terminal_bytes_preserved(self):
        with tempfile.TemporaryDirectory(prefix='candidate-notify-') as temporary:
            root = Path(temporary)
            shutil.copytree(ASSETS / 'codex/scripts', root / 'codex/scripts')
            with patch.object(unix_probe, 'ASSETS', root):
                raw, received, stdout = unix_probe.TmuxNotificationTests().notify('codex', False)
                self.assertEqual(received, raw)
                self.assertEqual(stdout, b'')


if __name__ == '__main__':
    unittest.main()
