"""固定输入和编码的独立回归；原生 Windows 执行由 probe 单独验证，不用 mock 冒充。"""

import base64
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from codex_windows_hook_command import (PREFIX, SCRIPTS, cmd_line, encode, source_text,
                                       verify_encoding, verify_windows_argv)
from codex_windows_hook_inputs import (CODEX_COMMIT, RELEASE_ASSETS, fetch_file, plugin_base,
                                       regular_file, sha256, verify_codex, verify_plugin)


class CommandTests(unittest.TestCase):
    def test_each_command_roundtrips_exact_fixed_source(self):
        for script in SCRIPTS:
            with self.subTest(script=script):
                command = encode(script)
                decoded = base64.b64decode(command[len(PREFIX):], validate=True).decode('utf-16le')
                self.assertEqual(decoded, "$NotificationHook = '" + script + "'\n" + source_text())
                self.assertLessEqual(len(command), 8000)
        self.assertTrue(verify_encoding()['encoding_roundtrip'])

    def test_platform_checkout_newlines_generate_identical_commands(self):
        plain = source_text()
        with patch.object(Path, 'read_text', return_value=plain.replace('\n', '\r\n')):
            self.assertEqual(source_text(), plain)

    def test_script_name_cannot_be_shell_source(self):
        for name in ('../on-stop.sh', "on-stop.sh'; whoami; '", 'on-stop.sh\n', 'unknown.sh'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                encode(name)

    def test_overlong_fixed_source_is_rejected(self):
        with patch('codex_windows_hook_command.source_text', return_value=' ' * 9000):
            with self.assertRaises(ValueError):
                encode(SCRIPTS[0])

    def test_cmd_boundary_accounts_for_utf16_and_executable(self):
        executable = Path('C:/含😀空格/Windows/System32/cmd.exe')
        command = encode(SCRIPTS[0])
        count = len(cmd_line(command, executable).encode('utf-16le')) // 2
        boundary = command + ' ' * (8191 - count)
        self.assertEqual(len(cmd_line(boundary, executable).encode('utf-16le')) // 2, 8191)
        with self.assertRaises(ValueError):
            cmd_line(boundary + ' ', executable)

    def test_encoded_command_contains_no_environment_interpolation(self):
        for script in SCRIPTS:
            command = encode(script)
            self.assertTrue(command.isascii())
            self.assertFalse(any(character in command for character in '%!$`&^()'))

    def test_native_argv_failure_preserves_fixed_expected_and_actual_values(self):
        observed = {}

        def collapsed_result(plain, environment):
            values = json.loads(Path(environment['PROBE_VALUES']).read_text(encoding='utf-8'))
            observed['expected'] = values
            observed['actual'] = [' '.join(values)]
            Path(environment['PROBE_OUTPUT']).write_text(json.dumps(observed['actual']), encoding='utf-8')

        # 模拟旧 PS 数组转换的合并结果，只验证失败证据；不冒充 Windows 进程实测。
        with patch('codex_windows_hook_command.run_powershell', side_effect=collapsed_result):
            with self.assertRaises(ValueError) as failure:
                verify_windows_argv({})
        self.assertEqual(json.loads(str(failure.exception).split(': ', 1)[1]), observed)
        self.assertEqual(observed['expected'][0], '')
        self.assertEqual(len(observed['expected']), 8)

    def test_native_argv_validation_preserves_empty_and_quoted_fixture_values(self):
        def unchanged_result(plain, environment):
            values = json.loads(Path(environment['PROBE_VALUES']).read_text(encoding='utf-8'))
            Path(environment['PROBE_OUTPUT']).write_text(json.dumps(values), encoding='utf-8')

        with patch('codex_windows_hook_command.run_powershell', side_effect=unchanged_result):
            verify_windows_argv({})


class FixedInputTests(unittest.TestCase):
    def test_official_pins_are_specific_release_assets(self):
        self.assertEqual(len(CODEX_COMMIT), 40)
        for architecture, (name, size, digest) in RELEASE_ASSETS.items():
            self.assertEqual(name, f'codex-{architecture}-pc-windows-msvc.exe')
            self.assertGreater(size, 200_000_000)
            self.assertEqual(len(bytes.fromhex(digest)), 32)
        self.assertEqual(len(plugin_base(Path(__file__).resolve().parents[2])['tree_sha256']), 10)

    def test_external_executable_cannot_pass_by_version_string(self):
        with tempfile.TemporaryDirectory() as tmp:
            executable = Path(tmp) / 'codex.exe'
            executable.write_bytes(b'codex-cli 0.147.0')
            with self.assertRaises(ValueError):
                verify_codex(executable, 'x86_64')

    def test_raw_tree_rejects_modified_missing_and_extra_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / 'scripts/hook.sh'
            path.parent.mkdir()
            contents = b'#!/bin/bash\nprintf original\n'
            path.write_bytes(contents)
            base = {'tree_sha256': {'scripts/hook.sh': hashlib.sha256(contents).hexdigest()}}
            self.assertEqual(verify_plugin(root, base), base['tree_sha256'])
            path.write_bytes(contents + b'changed')
            with self.assertRaises(ValueError):
                verify_plugin(root, base)
            path.write_bytes(contents)
            extra = root / 'custom.txt'
            extra.touch()
            with self.assertRaises(ValueError):
                verify_plugin(root, base)
            extra.unlink()
            path.unlink()
            with self.assertRaises(ValueError):
                verify_plugin(root, base)

    def test_hardlinked_input_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'input'
            path.write_bytes(b'fixed')
            os.link(path, Path(tmp) / 'alias')
            with self.assertRaises(ValueError):
                regular_file(path)

    def test_existing_download_is_verified_without_network_or_overwrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'fixed'
            path.write_bytes(b'fixed')
            digest = sha256(path)
            url = 'https://github.com/openai/codex/releases/download/rust-v0.147.0/test.exe'
            with patch('urllib.request.urlopen', side_effect=AssertionError('不应联网')):
                fetch_file(url, path, digest, 5)
                with self.assertRaises(ValueError):
                    fetch_file(url, path, '0' * 64, 5)
            self.assertEqual(path.read_bytes(), b'fixed')

    def test_latest_or_uncontrolled_download_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            for url in ('https://github.com/openai/codex/releases/latest/download/codex.exe',
                        'http://example.com/codex.exe'):
                with self.subTest(url=url), self.assertRaises(ValueError):
                    fetch_file(url, Path(tmp) / 'exe', '0' * 64)


if __name__ == '__main__':
    unittest.main()
