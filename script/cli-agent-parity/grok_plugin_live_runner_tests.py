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
    def test_panic_diagnostics_only_export_bound_source_location_and_error_class(self):
        source = 'app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs'
        output = ("thread 'private-task' panicked at C:\\private\\checkout\\" + source.replace('/', '\\')
                  + ':1792:14:\ncalled `Result::unwrap()` on an `Err` value: '
                  + 'PluginInstallError { log: "token=private-secret C:\\private\\config" }\n')
        diagnostics = runner.safe_failure_diagnostics(output)
        self.assertEqual(diagnostics['panic_locations'],
                         [{'source': source, 'line': 1792, 'column': 14, 'kind': 'unwrap_result',
                           'command_failure': None}])
        self.assertNotIn('private-secret', json.dumps(diagnostics))
        self.assertNotIn('private-task', json.dumps(diagnostics))
        self.assertNotIn('checkout', json.dumps(diagnostics))
        self.assertFalse(diagnostics['raw_output_exported'])
        self.assertFalse(diagnostics['private_paths_exported'])

    def test_panic_diagnostics_reject_unbound_paths_and_never_export_assertion_payload(self):
        output = ("thread 'worker' panicked at C:/private/config.rs:7:8:\nsecret payload\n"
                  "thread 'worker' panicked at app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs:2000:5:\n"
                  "assertion `left == right` failed: private-secret\n  left: private-left\n right: private-right\n")
        diagnostics = runner.safe_failure_diagnostics(output)
        self.assertEqual(len(diagnostics['panic_locations']), 1)
        self.assertEqual(diagnostics['panic_locations'][0]['kind'], 'assertion_failed')
        self.assertNotIn('private-', json.dumps(diagnostics))
        self.assertEqual(runner.safe_failure_diagnostics('no panic\n')['panic_locations'], [])

    def test_only_production_command_debug_result_identifies_timeout_or_spawn_error(self):
        prefix = ("thread 'worker' panicked at app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs:1996:70:\n"
                  'called `Result::unwrap()` on an `Err` value: PluginInstallError { message: "private", log: "')
        timeout = prefix + '$ private-command\\ncommand failed or timed out: Err(TimeoutError)\\n" }\n'
        error = runner.safe_failure_diagnostics(timeout)['panic_locations'][0]['command_failure']
        self.assertEqual(error, {'kind': 'timeout', 'os_code': None, 'native_exit_code': None})
        spawn = prefix + r'$ private-command\ncommand failed or timed out: Ok(Err(Os { code: 193, kind: Uncategorized, message: \"private-path\" }))\n" }' + '\n'
        error = runner.safe_failure_diagnostics(spawn)['panic_locations'][0]['command_failure']
        self.assertEqual(error, {'kind': 'spawn_io_error', 'os_code': 193, 'io_kind': 'Uncategorized',
                                'native_exit_code': None})
        self.assertNotIn('private', json.dumps(error))

    def test_unrecorded_native_exit_and_arbitrary_timeout_text_remain_unknown(self):
        prefix = ("thread 'worker' panicked at app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs:1996:70:\n"
                  'called `Result::unwrap()` on an `Err` value: PluginInstallError { message: "private", log: "')
        for payload in ('native process exited with code 1', 'Err(TimeoutError)', 'timeout',
                        'command failed or timed out: Err(UnknownError)'):
            output = prefix + payload + '" }\n'
            self.assertIsNone(runner.safe_failure_diagnostics(output)['panic_locations'][0]['command_failure'])

    def test_recorded_original_command_exports_exit_timeout_and_io_without_payload(self):
        for kind, code, os_code in [('native_nonzero', 17, None), ('timeout', None, None), ('spawn_io_error', None, 193)]:
            value = {'stage': 'windows_bridge_cmd', 'error_kind': kind, 'native_exit_code': code,
                     'os_code': os_code, 'private_log': 'private-secret', 'private_path': 'C:/private/config'}
            self.assertEqual(runner.safe_recorded_command(value), {
                'stage': 'windows_bridge_cmd', 'error_kind': kind, 'native_exit_code': code, 'os_code': os_code})

    def test_bridge_projection_keeps_known_exception_type_and_numeric_code_only(self):
        value = {'stage': 'windows_bridge_cmd', 'error_kind': 'native_nonzero', 'native_exit_code': 1,
                 'bridge': {'node_exit_code': None,
                            'failure': {'kind': 'win32', 'hresult': -2147467259, 'os_code': 2, 'message': 'private-secret'}}}
        result = runner.safe_recorded_command(value)
        self.assertEqual(result['bridge']['failure'], {'kind': 'win32', 'hresult': -2147467259, 'os_code': 2})
        self.assertNotIn('private-secret', json.dumps(result))
        value['bridge']['failure']['kind'] = 'private-type'
        self.assertIsNone(runner.safe_recorded_command(value)['bridge']['failure'])
        value['native_exit_code'] = 'private-secret'
        self.assertIsNone(runner.safe_recorded_command(value))
        self.assertIsNone(runner.safe_recorded_command({'stage': 'private-path', 'error_kind': None}))

    def test_shell_diagnostics_only_export_the_fixed_error_class_whitelist(self):
        value = {'stage': 'windows_bridge_cmd', 'error_kind': 'native_nonzero', 'native_exit_code': 1,
                 'bridge': {'powershell_clixml_observed': True, 'shell_error_kind': 'powershell_parser_error',
                            'message': 'private-secret', 'path': 'C:/private/config'}}
        safe = runner.safe_recorded_command(value)
        self.assertEqual(safe['bridge']['shell_error_kind'], 'powershell_parser_error')
        self.assertTrue(safe['bridge']['powershell_clixml_observed'])
        self.assertNotIn('private', json.dumps(safe))
        value['bridge']['shell_error_kind'] = 'private-type'
        self.assertNotIn('shell_error_kind', runner.safe_recorded_command(value)['bridge'])

    def test_production_failure_is_bound_to_its_stage_and_never_reuses_bridge_success(self):
        stage = 'production_upgrade_known_013_to_014'
        value = {'stage': stage, 'message_class': 'operation_failed', 'filesystem_error_observed': True,
                 'filesystem_os_code': 5, 'filesystem_os_code_parsed_from_display': True,
                 'commands': [{'operation': 'runtime_version', 'error_kind': None, 'native_exit_code': 0},
                              {'operation': 'plugin_install', 'error_kind': 'native_nonzero', 'native_exit_code': 17,
                               'private_payload': 'secret'}],
                 'migration': {'record': 'regular', 'record_os_code': None, 'registered_version': 'legacy_013',
                               'registered_cache_valid': True, 'legacy_source_valid': True, 'current_source_valid': True,
                               'private_path': 'private-secret'}}
        receipt = {'stage': stage, 'production_operation_failure': value,
                   'windows_bridge_last_command': {'stage': 'windows_bridge_powershell', 'error_kind': None, 'native_exit_code': 0}}
        result = runner.recorded_failure_diagnostics(receipt)
        self.assertEqual(result['production_operation_failure']['commands'][-1]['native_exit_code'], 17)
        self.assertEqual(result['production_operation_failure']['filesystem_os_code'], 5)
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('secret', json.dumps(result))
        self.assertNotIn('recorded_command', result)
        for changed in ('production_install', 'finished'):
            self.assertEqual(runner.recorded_failure_diagnostics(dict(receipt, stage=changed)), {})
        self.assertEqual(runner.recorded_failure_diagnostics(dict(receipt, production_operation_failure=None)), {})
        receipt['stage'] = 'windows_bridge_powershell_returned'
        self.assertIn('recorded_command', runner.recorded_failure_diagnostics(receipt))
        for mutation in ('category', 'code', 'boolean_code', 'too_many', 'state', 'operation'):
            bad = copy.deepcopy(value)
            if mutation == 'category': bad['message_class'] = 'private-secret'
            elif mutation == 'code': bad['filesystem_os_code'] = 2 ** 31
            elif mutation == 'boolean_code': bad['filesystem_os_code'] = True
            elif mutation == 'too_many': bad['commands'] *= 9
            elif mutation == 'state': bad['migration']['record'] = 'private-path'
            else: bad['commands'][0]['operation'] = 'private-command'
            self.assertIsNone(runner.safe_production_failure(bad, stage), mutation)

    def test_original_upgrade_failure_keeps_bridge_success_out_of_failure_diagnostics(self):
        receipt = {'stage': 'production_upgrade_known_013_to_014',
                   'windows_bridge_last_command': {'stage': 'windows_bridge_powershell',
                                                  'error_kind': None, 'native_exit_code': 0}}
        self.assertEqual(runner.recorded_failure_diagnostics(receipt), {})

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
            with patch.object(runner.sys, 'platform', 'linux'), patch.object(runner.os, 'pathsep', ':'):
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
