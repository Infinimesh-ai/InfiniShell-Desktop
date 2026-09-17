"""Windows 通知候选的编码与 Unix 保持回归；原生控制台必须由 ConPTY 探针验收。"""

import base64
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

ASSETS = Path(__file__).resolve().parents[2] / 'app/assets/bundled/cli-agent-plugins'


class NotifyCandidateTests(unittest.TestCase):
    def original(self):
        return (ASSETS / 'codex/scripts/warp-notify.sh').read_text(encoding='utf-8')

    def test_encoded_program_roundtrip_and_bash_syntax(self):
        body = candidate.candidate_notify(self.original())
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
    def test_direct_and_tmux_control_terminal_bytes_preserved(self):
        with tempfile.TemporaryDirectory(prefix='candidate-notify-') as temporary:
            root = Path(temporary)
            shutil.copytree(ASSETS / 'codex/scripts', root / 'codex/scripts')
            (root / 'codex/scripts/warp-notify.sh').write_text(candidate.candidate_notify(self.original()), encoding='utf-8')
            with patch.object(unix_probe, 'ASSETS', root):
                for tmux in (False, True):
                    with self.subTest(tmux=tmux):
                        raw, received, stdout = unix_probe.TmuxNotificationTests().notify('codex', tmux)
                        expected = b'\x1bPtmux;' + raw.replace(b'\x1b', b'\x1b\x1b') + b'\x1b\\' if tmux else raw
                        self.assertEqual(received, expected)
                        self.assertEqual(stdout, b'')


if __name__ == '__main__':
    unittest.main()
