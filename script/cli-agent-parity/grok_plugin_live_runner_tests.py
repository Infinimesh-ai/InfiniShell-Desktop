#!/usr/bin/env python3
"""Grok 无凭据通知安装验收的严格收据与环境边界。"""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_grok_plugin_live as runner


def valid_receipt():
    export = {'installed_hook_export_version_verified': True, 'plugin_version': '0.1.4', 'main_entry_verified': False}
    steps = [dict(step=name, passed=True) for name in runner.STEPS]
    steps[0]['installed_hook_export'] = export
    steps[1].update(installed_hook_export=export, previous_plugin_version='0.1.3', current_plugin_version='0.1.4',
                    legacy_source_unchanged=True, production_update_call=True)
    steps[2]['config_and_registry_unchanged'] = True
    steps[3]['config_and_files_unchanged'] = True
    steps[4].update(production_apply_call=False, fault_after_first_replacement=True)
    return {'passed': True, 'grok_version': '1.0.41', 'grok_sha256': 'g' * 64, 'node_sha256': 'n' * 64,
            'test_binary_sha256': 't' * 64, 'credentials_provided': False, 'model_input_submitted': False,
            'native_inspect_verified': True, 'native_hook_execution_verified': False, 'steps': steps,
            'windows_bridge_argv': {'cmd_verified': True, 'powershell_51_verified': True,
                'space_unicode_and_shell_metacharacters_preserved': True, 'native_hook_execution_verified': False}}


class GrokPluginLiveRunnerTests(unittest.TestCase):
    def accept(self, receipt, host='linux', **kwargs):
        args = dict(exit_code=0, timed_out=False, output='test result: ok. 1 passed; 0 failed; 0 ignored;',
                    receipt=receipt, native_sha='g' * 64, node_sha='n' * 64, test_sha='t' * 64, host_platform=host)
        args.update(kwargs)
        return runner.verified_receipt(**args)

    def test_only_exact_version_and_binary_bound_completed_receipt_passes(self):
        for host in ('linux', 'win32'):
            self.assertTrue(self.accept(valid_receipt(), host))
        for field, value in [('grok_version', '1.0.42'), ('grok_sha256', 'x'), ('node_sha256', 'x'),
                             ('test_binary_sha256', 'x'), ('credentials_provided', True), ('model_input_submitted', True),
                             ('native_hook_execution_verified', True), ('native_inspect_verified', False), ('passed', False)]:
            receipt = valid_receipt(); receipt[field] = value
            self.assertFalse(self.accept(receipt), field)
        self.assertFalse(self.accept(valid_receipt(), 'darwin'))

    def test_failure_timeout_partial_or_reordered_steps_never_pass(self):
        for change in ({'exit_code': 1}, {'timed_out': True}, {'output': ''}):
            self.assertFalse(self.accept(valid_receipt(), **change))
        for mutation in ('missing', 'order', 'failed', 'source_changed', 'disabled_changed', 'rollback_relabelled'):
            receipt = valid_receipt()
            if mutation == 'missing': receipt['steps'].pop()
            elif mutation == 'order': receipt['steps'].reverse()
            elif mutation == 'failed': receipt['steps'][2]['passed'] = False
            elif mutation == 'source_changed': receipt['steps'][1]['legacy_source_unchanged'] = False
            elif mutation == 'disabled_changed': receipt['steps'][3]['config_and_files_unchanged'] = False
            else: receipt['steps'][4]['production_apply_call'] = True
            self.assertFalse(self.accept(receipt), mutation)

    def test_windows_requires_real_both_shell_argument_receipts(self):
        for field in ('cmd_verified', 'powershell_51_verified', 'space_unicode_and_shell_metacharacters_preserved'):
            receipt = valid_receipt(); receipt['windows_bridge_argv'][field] = False
            self.assertFalse(self.accept(receipt, 'win32'))
        receipt = valid_receipt(); del receipt['windows_bridge_argv']
        self.assertTrue(self.accept(receipt, 'linux'))
        self.assertFalse(self.accept(receipt, 'win32'))

    def test_private_environment_drops_auth_proxies_hook_overrides_and_injection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            host = {'PATH': '/foreign/bin', 'HOME': '/real', 'GROK_API_KEY': 'secret',
                    'HTTPS_PROXY': 'private', 'NODE_OPTIONS': '--require bad', 'GROK_CONFIG_PATH': '/external',
                    'GROK_SHELL': 'foreign', 'LANG': 'C.UTF-8'}
            with patch.object(runner.sys, 'platform', 'linux'):
                env = runner.isolated_environment(root, root / '程序 bin', host)
            self.assertEqual(env['PATH'], str(root / '程序 bin') + ':/usr/bin:/bin')
            for key in ('GROK_API_KEY', 'HTTPS_PROXY', 'NODE_OPTIONS', 'GROK_CONFIG_PATH', 'GROK_SHELL'):
                self.assertNotIn(key, env)
            for key in ('HOME', 'USERPROFILE', 'GROK_HOME', 'LOCALAPPDATA', 'APPDATA', 'TMPDIR', 'TMP', 'TEMP'):
                self.assertTrue(Path(env[key]).is_relative_to(root))
            self.assertNotIn('secret', json.dumps(env))

    def test_windows_requires_system_environment_instead_of_importing_user_settings(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner.sys, 'platform', 'win32'):
            with self.assertRaises(ValueError):
                runner.isolated_environment(Path(directory), Path(directory) / 'bin', {})


if __name__ == '__main__':
    unittest.main()
