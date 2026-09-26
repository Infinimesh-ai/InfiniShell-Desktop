#!/usr/bin/env python3
"""固定 Grok 1.0.41 生产通知安装窄验收；只用私有无凭据 HOME，不提交模型输入。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile

from prepare_grok_cli import verify_binary
from run_codex_source_live import digest, stop_group

TEST_NAME = ('terminal::cli_agent_sessions::plugin_manager::grok::tests::'
             'live_grok_production_installer_repairs_and_preserves_disable')
# 升级步骤键保留历史收据的稳定标识；实际目标版本在下方严格校验为 0.1.5。
STEPS = ['production_install', 'production_upgrade_known_013_to_014',
         'production_same_version_update', 'production_disabled_update_rejected',
         'file_transaction_failure_rollback', 'production_update_after_rollback']


def safe_failure_diagnostics(output):
    # 仅导出已知源码位置和固定错误类别；panic 正文可能含配置、命令和私有路径。
    sources = ('app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs',
               'app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs')
    locations = []
    for match in re.finditer(r"(?m)^thread [^\r\n]{1,512} panicked at ([^\r\n]+):(\d{1,9}):(\d{1,9}):\s*\n([^\r\n]*)", output):
        path, line, column, message = match.groups()
        path = path.replace('\\', '/')
        source = next((name for name in sources if path == name or path.endswith('/' + name)), None)
        if source is None:
            continue
        kind = 'panic'
        if message.startswith('called `Result::unwrap()` on an `Err` value:'):
            kind = 'unwrap_result'
        elif message.startswith('called `Option::unwrap()` on a `None` value'):
            kind = 'unwrap_option'
        elif message.startswith('assertion ') and ' failed' in message:
            kind = 'assertion_failed'
        command_failure = None
        if message.startswith('called `Result::unwrap()` on an `Err` value: PluginInstallError {'):
            # 仅识别生产 run 自己追加的 Debug 结果；原生非零退出未记录 status，必须保持未知。
            if re.search(r'\\ncommand failed or timed out: Err\(TimeoutError\)\\n" \}$', message):
                command_failure = {'kind': 'timeout', 'os_code': None, 'native_exit_code': None}
            else:
                error = re.search(r'\\ncommand failed or timed out: Ok\(Err\(Os \{ code: (-?\d{1,10}), kind: '
                                  r'(NotFound|PermissionDenied|InvalidInput|Interrupted|Other|Uncategorized), message:', message)
                if error:
                    command_failure = {'kind': 'spawn_io_error', 'os_code': int(error[1]),
                                       'io_kind': error[2], 'native_exit_code': None}
        locations.append({'source': source, 'line': int(line), 'column': int(column), 'kind': kind,
                          'command_failure': command_failure})
        if len(locations) == 8:
            break
    return {'panic_locations': locations,
            'raw_output_exported': False, 'private_paths_exported': False}


def safe_recorded_command(value, allowed_stages=('windows_bridge_cmd', 'windows_bridge_powershell')):
    """只投影原调用产生的固定诊断；不导出日志、路径或错误正文。"""
    if not isinstance(value, dict) or value.get('error_kind') not in (None, 'native_nonzero', 'timeout', 'spawn_io_error'):
        return None
    if value.get('stage') not in allowed_stages:
        return None
    result = {'stage': value['stage'], 'error_kind': value.get('error_kind')}
    for key in ('native_exit_code', 'os_code'):
        number = value.get(key)
        if number is not None and (type(number) is not int or not -(2 ** 31) <= number < 2 ** 31):
            return None
        result[key] = number
    for key in ('stdout_bytes', 'stderr_bytes'):
        number = value.get(key)
        if type(number) is int and 0 <= number < 2 ** 63:
            result[key] = number
    for key in ('stdout_sha256', 'stderr_sha256'):
        digest = value.get(key)
        if isinstance(digest, str) and re.fullmatch('[0-9a-f]{64}', digest):
            result[key] = digest
    bridge = value.get('bridge')
    if isinstance(bridge, dict):
        safe = {'node_exit_code': None, 'failure': None,
                'node_module_not_found': bridge.get('node_module_not_found') is True}
        if bridge.get('powershell_clixml_observed') is True:
            safe['powershell_clixml_observed'] = True
        if bridge.get('shell_error_kind') in ('cmd_command_not_found', 'powershell_command_not_found',
                'powershell_parser_error', 'powershell_security_error', 'powershell_initialization_failure'):
            safe['shell_error_kind'] = bridge['shell_error_kind']
        code = bridge.get('node_exit_code')
        if type(code) is int and -(2 ** 31) <= code < 2 ** 31:
            safe['node_exit_code'] = code
        failure = bridge.get('failure')
        if (isinstance(failure, dict) and failure.get('kind') in ('win32', 'other')
                and type(failure.get('hresult')) is int and -(2 ** 31) <= failure['hresult'] < 2 ** 31):
            code = failure.get('os_code')
            if code is None or (type(code) is int and -(2 ** 31) <= code < 2 ** 31):
                safe['failure'] = {key: failure.get(key) for key in ('kind', 'hresult', 'os_code')}
        result['bridge'] = safe
    return result



def safe_production_failure(value, stage):
    """绑定当次生产操作；只保留固定类别、数值和迁移状态。"""
    if (stage not in STEPS or stage == 'file_transaction_failure_rollback'
            or not isinstance(value, dict) or value.get('stage') != stage
            or value.get('message_class') not in ('operation_failed', 'invalid_state', 'restored_previous',
                'restore_unverified', 'update_not_effective', 'incompatible', 'disabled', 'unknown')):
        return None
    result = {key: value[key] for key in ('stage', 'message_class')}
    for key in ('filesystem_error_observed', 'filesystem_os_code_parsed_from_display'):
        if type(value.get(key)) is not bool:
            return None
        result[key] = value[key]
    code = value.get('filesystem_os_code')
    if code is not None and (type(code) is not int or not -(2 ** 31) <= code < 2 ** 31):
        return None
    if value['filesystem_os_code_parsed_from_display'] != (code is not None):
        return None
    result['filesystem_os_code'] = code
    commands = value.get('commands')
    if not isinstance(commands, list) or len(commands) > 16:
        return None
    result['commands'] = []
    for command in commands:
        if (not isinstance(command, dict) or command.get('operation') not in
                ('runtime_version', 'plugin_validate', 'plugin_install', 'plugin_uninstall', 'unknown')):
            return None
        safe = safe_recorded_command(dict(command, stage=stage), allowed_stages=(stage,))
        if safe is None:
            return None
        safe.pop('stage'); safe.pop('bridge', None)
        safe['operation'] = command['operation']
        result['commands'].append(safe)
    migration = value.get('migration')
    if (not isinstance(migration, dict)
            or migration.get('record') not in ('regular', 'not_regular', 'absent', 'io_error')
            or migration.get('registered_version') not in ('legacy_013', 'current_014', 'other', 'absent', 'unreadable_or_invalid')
            or (migration.get('registered_cache_valid') is not None and type(migration['registered_cache_valid']) is not bool)
            or any(type(migration.get(key)) is not bool for key in ('legacy_source_valid', 'current_source_valid'))):
        return None
    code = migration.get('record_os_code')
    if code is not None and (type(code) is not int or not -(2 ** 31) <= code < 2 ** 31):
        return None
    result['migration'] = {key: migration.get(key) for key in ('record', 'record_os_code', 'registered_version',
        'registered_cache_valid', 'legacy_source_valid', 'current_source_valid')}
    return result


def recorded_failure_diagnostics(receipt):
    """桥接诊断只属于桥接阶段，不能填入后续生产升级失败。"""
    stage = receipt.get('stage')
    failure = safe_production_failure(receipt.get('production_operation_failure'), stage)
    if failure is not None:
        return {'production_operation_failure': failure}
    command = safe_recorded_command(receipt.get('windows_bridge_last_command'))
    if command is not None and stage in (command['stage'], command['stage'] + '_returned'):
        return {'recorded_command': command}
    return {}


def isolated_environment(root, programs, host_environment=None):
    host = os.environ if host_environment is None else host_environment
    allowed = {'LANG', 'LC_ALL', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'PATHEXT'}
    environment = {key.upper(): value for key, value in host.items() if key.upper() in allowed}
    system_paths = ['/usr/bin', '/bin']
    if sys.platform == 'win32':
        if not all(environment.get(key) for key in ('SYSTEMROOT', 'COMSPEC', 'PATHEXT')):
            raise ValueError('Windows 安装验收缺少系统环境')
        system = Path(environment['SYSTEMROOT']) / 'System32'
        system_paths = [str(system / 'WindowsPowerShell/v1.0'), str(system)]
    environment['PATH'] = os.pathsep.join([str(programs), *system_paths])
    paths = {'HOME': 'home', 'USERPROFILE': 'home', 'GROK_HOME': 'home/.grok',
             'APPDATA': 'home/AppData/Roaming', 'LOCALAPPDATA': 'home/AppData/Local',
             'XDG_CONFIG_HOME': 'home/.config', 'XDG_DATA_HOME': 'home/.local/share',
             'XDG_CACHE_HOME': 'home/.cache', 'TMPDIR': 'tmp', 'TMP': 'tmp', 'TEMP': 'tmp'}
    for key, relative in paths.items():
        path = root / relative
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
        environment[key] = str(path)
    (root / 'home/Library/Application Support').mkdir(parents=True, exist_ok=True)
    (root / 'work').mkdir()
    (root / '.infinishell-grok-plugin-live').write_text(
        'isolated unauthenticated Grok plugin verification\n', encoding='utf-8', newline='\n')
    environment.update(INFINISHELL_GROK_PLUGIN_LIVE_ROOT=str(root),
                       GROK_AUTO_UPDATE='0', GROK_DISABLE_AUTOUPDATER='1')
    return environment


def verified_receipt(exit_code, timed_out, output, receipt, native_sha, node_sha, test_sha, host_platform):
    if (timed_out or exit_code != 0 or not re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', output)
            or not isinstance(receipt, dict) or receipt.get('passed') is not True
            or receipt.get('grok_version') != '1.0.41' or receipt.get('grok_sha256') != native_sha
            or receipt.get('node_sha256') != node_sha or receipt.get('test_binary_sha256') != test_sha
            or receipt.get('credentials_provided') is not False or receipt.get('model_input_submitted') is not False
            or receipt.get('native_inspect_verified') is not True or receipt.get('native_hook_execution_verified') is not False):
        return False
    steps = receipt.get('steps')
    if (not isinstance(steps, list) or any(not isinstance(item, dict) for item in steps)
            or [item.get('step') for item in steps] != STEPS
            or any(item.get('passed') is not True for item in steps)):
        return False
    if (steps[1].get('previous_plugin_version') != '0.1.3'
            or steps[1].get('current_plugin_version') != '0.1.5'
            or steps[1].get('legacy_source_unchanged') is not True
            or steps[1].get('production_update_call') is not True
            or steps[2].get('config_and_registry_unchanged') is not True
            or steps[3].get('config_and_files_unchanged') is not True
            or steps[4].get('production_apply_call') is not False
            or steps[4].get('fault_after_first_replacement') is not True):
        return False
    for item in steps[:2]:
        export = item.get('installed_hook_export', {})
        if (export.get('installed_hook_export_version_verified') is not True
                or export.get('plugin_version') != '0.1.5' or export.get('main_entry_verified') is not False):
            return False
    if host_platform == 'win32':
        args = receipt.get('windows_bridge_argv', {})
        if (args.get('cmd_verified') is not True or args.get('powershell_51_verified') is not True
                or args.get('space_unicode_and_shell_metacharacters_preserved') is not True
                or args.get('native_hook_execution_verified') is not False):
            return False
    return host_platform in ('linux', 'win32')


def source_identity():
    repo = Path(__file__).resolve().parents[2]
    files = ['app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs',
             'app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs',
             'app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs',
             'app/assets/bundled/cli-agent-plugins/grok/hooks/notify.cjs',
             'app/assets/bundled/cli-agent-plugins/grok/.grok-plugin/plugin.json',
             'script/cli-agent-parity/codex_windows_hook_command.ps1']
    return {name: digest(repo / name) for name in files}


def run(args):
    if sys.platform not in ('linux', 'win32') or platform.machine().lower() not in ('x86_64', 'amd64'):
        raise ValueError('此窄入口只覆盖固定 Linux/Windows x64；Mac 沿既有验收')
    native = verify_binary(args.grok, f'{sys.platform}-x64', '1.0.41')
    for path in (args.test_binary, args.node):
        if not path.is_file() or path.is_symlink():
            raise ValueError('必须提供普通同提交 libtest 与 Node 可执行文件')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.output.exists():
        raise FileExistsError('不能覆盖此前失败或成功收据')
    root = Path(tempfile.mkdtemp(prefix='grok 通知安装 ', dir=args.fixture_parent)).resolve()
    report = {'schema_version': 1, 'passed': False, 'fixed_version': '1.0.41',
              'platform': sys.platform, 'native_sha256': native['sha256'],
              'test_binary_sha256': digest(args.test_binary), 'node_sha256': digest(args.node),
              'runner_sha256': digest(Path(__file__)), 'source_sha256': source_identity(), 'credentials_provided': False,
              'model_inputs': 0, 'native_hook_execution_verified': False,
              'worker_conout_verified': False, 'gui_verified': False,
              'private_root_removed': False, 'timed_out': False}
    try:
        programs = root / '程序 bin'; programs.mkdir()
        suffix = '.exe' if sys.platform == 'win32' else ''
        for source, name in [(args.grok, 'grok'), (args.node, 'node')]:
            shutil.copy2(source, programs / (name + suffix))
        env = isolated_environment(root, programs)
        env.update(INFINISHELL_GROK_PLUGIN_LIVE_GROK_SHA256=report['native_sha256'],
                   INFINISHELL_GROK_PLUGIN_LIVE_NODE_SHA256=report['node_sha256'])
        node = subprocess.run([str(programs / ('node' + suffix)), '--version'], cwd=root / 'work',
                              env=env, capture_output=True, timeout=5, check=True)
        env['INFINISHELL_GROK_PLUGIN_LIVE_NODE_VERSION'] = node.stdout.decode().strip()
        command = [str(args.test_binary), '--exact', TEST_NAME, '--ignored', '--test-threads=1', '--nocapture']
        options = {'creationflags': 0x00000200} if sys.platform == 'win32' else {'start_new_session': True}
        process = subprocess.Popen(command, cwd=root / 'work', env=env, **options,
                                   text=True, encoding='utf-8', errors='replace', stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            output = process.communicate(timeout=220)[0]
        except subprocess.TimeoutExpired:
            output = stop_group(process, env)[0]; report['timed_out'] = True
        except BaseException:
            stop_group(process, env); raise
        report.update(exit_code=process.returncode, output_bytes=len(output.encode()),
                      output_sha256=hashlib.sha256(output.encode()).hexdigest())
        report['failure_diagnostics'] = safe_failure_diagnostics(output)
        receipt_path = root / 'grok-production-installer.json'
        receipt = json.loads(receipt_path.read_text()) if receipt_path.exists() else {}
        report['receipt'] = receipt
        report['failure_diagnostics'].update(recorded_failure_diagnostics(receipt))
        report['passed'] = verified_receipt(process.returncode, report['timed_out'], output, receipt,
            report['native_sha256'], report['node_sha256'], report['test_binary_sha256'], sys.platform)
        report['source_unchanged'] = source_identity() == report['source_sha256']
        report['passed'] = report['passed'] and report['source_unchanged']
        if report['passed']:
            shutil.rmtree(root); report['private_root_removed'] = True
        else:
            # 无凭据失败现场保留在 runner 私有目录；公开收据只记录摘要，不输出载荷或路径。
            (root / 'test-output.private.txt').write_text(output, encoding='utf-8')
    except Exception as error:
        report['passed'] = False
        report['error_kind'] = type(error).__name__
    finally:
        with args.output.open('x', encoding='utf-8') as target:
            json.dump(report, target, ensure_ascii=False, indent=2); target.write('\n')
    return 0 if report['passed'] and report['private_root_removed'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('test-binary', 'grok', 'node', 'fixture-parent', 'output'):
        parser.add_argument('--' + name, required=True, type=lambda value: Path(value).resolve())
    return run(parser.parse_args())


if __name__ == '__main__':
    sys.exit(main())
