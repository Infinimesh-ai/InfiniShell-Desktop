#!/usr/bin/env python3
"""隔离验证固定 Codex 原生插件注册表生命周期；不调用模型或产品安装器。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib

sys.dont_write_bytecode = True
from codex_windows_hook_inputs import PLUGIN_COMMIT, fetch_file, plugin_base, require, verify_plugin
from prepare_codex_cli import DEFAULT_VERSION, SUPPORTED_VERSIONS, release_contract, require_cli_version
from probe_codex_windows_hooks import NativeRecorder


PLUGIN = 'warp@codex-warp'
INITIAL_CONFIG = ('model = "gpt-5.4"\napproval_policy = "on-request"\nsandbox_mode = "read-only"\n'
                  '[history]\npersistence = "none"\n')


def file_hash(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def tree_hashes(root):
    require(root.is_dir() and not root.is_symlink(), '快照目录不存在或为链接')
    output = {}
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in directories:
            require(not (Path(directory) / name).is_symlink(), '快照目录包含链接')
        for name in files:
            path = Path(directory) / name
            require(path.is_file() and not path.is_symlink(), '快照文件不是普通文件')
            output[path.relative_to(root).as_posix()] = file_hash(path)
    return output


def require_same_tree(root, expected):
    require(tree_hashes(root) == expected, '原生操作意外修改、增加或删除插件文件')


def user_settings(config):
    # 只读取本探测写出的隔离配置；原生插件和 marketplace 注册是预期的独立变化。
    parsed = tomllib.loads(config.read_text(encoding='utf-8'))
    return {key: value for key, value in parsed.items() if key not in ('plugins', 'marketplaces')}


def registry_entry(value):
    entries = [item for item in value['installed'] if item['pluginId'] == PLUGIN]
    require(len(entries) <= 1, '原生注册表出现重复插件')
    return entries[0] if entries else None


def safe_command(arguments):
    # 验证任务不能扩展到 thread、turn、exec 或模型入口。
    allowed = [('--version',), ('plugin', '--help'), ('plugin', 'list'), ('plugin', 'add'),
               ('plugin', 'remove'), ('plugin', 'marketplace', 'add'),
               ('plugin', 'marketplace', 'remove'), ('plugin', 'marketplace', 'upgrade')]
    if tuple(arguments) in (('plugin', 'disable', '--help'), ('plugin', 'enable', '--help'),
                            ('plugin', 'update', '--help')):
        return
    require(any(tuple(arguments[:len(prefix)]) == prefix for prefix in allowed), '拒绝非注册表命令')


class Probe:
    def __init__(self, executable, directory, report):
        self.executable = executable
        self.directory = directory
        self.report = report
        self.cli_home = directory / 'codex'
        self.home = directory / 'home'
        self.cli_home.mkdir()
        self.home.mkdir()
        self.config = self.cli_home / 'config.toml'
        self.config.write_text(INITIAL_CONFIG, encoding='utf-8', newline='\n')
        self.settings = user_settings(self.config)
        self.env = {key: os.environ[key] for key in ('PATH', 'TMPDIR', 'TMP', 'TEMP', 'SYSTEMROOT', 'WINDIR')
                    if key in os.environ}
        self.env.update(HOME=str(self.home), USERPROFILE=str(self.home), CODEX_HOME=str(self.cli_home),
                        APPDATA=str(self.home / 'AppData/Roaming'), LOCALAPPDATA=str(self.home / 'AppData/Local'),
                        GIT_CONFIG_NOSYSTEM='1', GIT_TERMINAL_PROMPT='0')
        for key in ('APPDATA', 'LOCALAPPDATA'):
            Path(self.env[key]).mkdir(parents=True)

    def command(self, arguments, expected_code=0):
        safe_command(arguments)
        completed = subprocess.run([str(self.executable), *arguments], env=self.env, cwd=self.directory,
                                   capture_output=True, text=True, encoding='utf-8', errors='strict', timeout=30)
        record = {'arguments': arguments, 'exit_code': completed.returncode,
                  'stdout': completed.stdout, 'stderr': completed.stderr}
        self.report['commands'].append(record)
        require(completed.returncode == expected_code, f'原生命令退出码不符: {arguments}: {completed.stderr}')
        require(user_settings(self.config) == self.settings, '原生命令改变了无关的隔离用户设置')
        return completed.stdout

    def listing(self):
        return json.loads(self.command(['plugin', 'list', '--available', '--json']))

    def state(self, name, installed, enabled=None):
        value = self.listing()
        entry = registry_entry(value)
        require((entry is not None) == installed, f'{name} 原生安装状态不符')
        if installed:
            require(entry['installed'] is True and entry['enabled'] is enabled and entry['version'] == '0.4.0',
                    f'{name} 原生版本或启用状态不符')
        self.report['states'][name] = value

    def configure_enabled(self, enabled):
        events = []
        self.report['config_api_traces'].append(events)
        recorder = NativeRecorder([str(self.executable), 'app-server', '--stdio'],
                                  self.env, self.directory, events)
        try:
            initialization = recorder.rpc('initialize', {'clientInfo': {
                'name': 'infinishell_plugin_lifecycle_probe', 'version': '0.1.0'},
                'capabilities': {'experimentalApi': True}}, 1)
            require(Path(initialization['codexHome']).resolve() == self.cli_home.resolve(), '原生配置作用域错误')
            recorder.send({'method': 'initialized'})
            response = recorder.rpc('config/value/write', {'keyPath': 'plugins."warp@codex-warp".enabled',
                'value': enabled, 'mergeStrategy': 'upsert', 'filePath': str(self.config)}, 2)
            require(response['status'] == 'ok' and Path(response['filePath']).resolve() == self.config.resolve(),
                    '原生配置写入未确认或越出私有 HOME')
            require(user_settings(self.config) == self.settings, '原生配置写入改变了无关设置')
            observed = tomllib.loads(self.config.read_text(encoding='utf-8'))
            require(observed['plugins'][PLUGIN]['enabled'] is enabled, '原生配置确认与落盘值不一致')
            hooks = recorder.rpc('hooks/list', {'cwds': [str(self.directory)]}, 3)
            self.report['hook_listings'].append({'enabled': enabled, 'response': hooks})
            if not enabled:
                require(all(not group['errors'] and not any(hook.get('pluginId') == PLUGIN for hook in group['hooks'])
                            for group in hooks['data']), '禁用后原生仍发现该插件的活跃 hook，或发现过程出错')
        finally:
            recorder.close()


def run_probe(executable, upstream, directory, report, version):
    probe = Probe(executable, directory, report)
    report['cli'] = probe.command(['--version']).strip()
    require_cli_version(report['cli'], version)
    report['executable_sha256'] = file_hash(executable)
    probe.command(['plugin', '--help'])
    for operation in ('disable', 'enable', 'update'):
        probe.command(['plugin', operation, '--help'], expected_code=2)
    report['unsupported_cli_subcommands'] = ['disable', 'enable', 'update']
    probe.state('missing', False)
    before_missing = probe.config.read_bytes()
    probe.command(['plugin', 'add', PLUGIN, '--json'], expected_code=1)
    require(probe.config.read_bytes() == before_missing, '缺失 marketplace 的失败安装改变了配置')

    marketplace = directory / 'marketplace'
    plugin = marketplace / 'plugins/warp'
    shutil.copytree(upstream, plugin)
    index = marketplace / '.agents/plugins/marketplace.json'
    index.parent.mkdir(parents=True)
    index.write_text(json.dumps({'name': 'codex-warp', 'plugins': [{'name': 'warp', 'source': './plugins/warp',
        'version': '0.4.0', 'policy': {'installation': 'AVAILABLE', 'authentication': 'ON_INSTALL'}}]}), encoding='utf-8')
    original_source = tree_hashes(plugin)
    report['original_plugin_tree_sha256'] = original_source
    probe.command(['plugin', 'marketplace', 'add', str(marketplace), '--json'])
    probe.state('available_before_install', False)
    installed = json.loads(probe.command(['plugin', 'add', PLUGIN, '--json']))
    require(installed['pluginId'] == PLUGIN and installed['version'] == '0.4.0', '原生安装结果不匹配')
    cache = Path(installed['installedPath'])
    require(cache.resolve().is_relative_to(probe.cli_home.resolve()), '原生安装越出私有 CODEX_HOME')
    require_same_tree(cache, original_source)
    probe.state('installed_enabled', True, True)

    probe.configure_enabled(False)
    probe.state('disabled', True, False)
    disabled_config = probe.config.read_bytes()
    disabled_tree = tree_hashes(cache)
    # 故障只注入临时来源；注册表和已安装缓存始终由真实 Codex 操作，绝不手写其结果。
    manifest = plugin / '.codex-plugin/plugin.json'
    original_manifest = manifest.read_bytes()
    try:
        manifest.write_bytes(b'{broken-json')
        probe.command(['plugin', 'add', PLUGIN, '--json'], expected_code=1)
        require(probe.config.read_bytes() == disabled_config, '失败的原生重装改变了用户配置或禁用状态')
        require_same_tree(cache, disabled_tree)
        probe.state('failed_reinstall_preserved_disabled', True, False)
    finally:
        manifest.write_bytes(original_manifest)
    require_same_tree(plugin, original_source)
    probe.command(['plugin', 'add', PLUGIN, '--json'])
    require_same_tree(cache, disabled_tree)
    # 原生 add 会重新启用插件；记录真实语义，再经原生配置 API 恢复用户原来的禁用选择。
    probe.state('native_reinstall_reenabled', True, True)
    probe.configure_enabled(False)
    probe.state('recovered_original_disabled_state', True, False)
    require(probe.config.read_bytes() == disabled_config, '恢复禁用后配置没有逐字回到失败前状态')
    require_same_tree(cache, disabled_tree)
    report['config_before_failure_sha256'] = hashlib.sha256(disabled_config).hexdigest()
    report['config_after_recovery_sha256'] = file_hash(probe.config)
    report['checks'].update({'missing_install_preserves_config': True, 'native_disable_persisted': True,
        'failed_native_reinstall_preserves_config_and_cache': True, 'native_add_reenables_disabled_plugin': True,
        'explicit_native_disable_restores_original_config_bytes': True, 'source_manifest_restored': True})

    probe.command(['plugin', 'remove', PLUGIN, '--json'])
    probe.state('removed', False)
    require(not cache.exists(), '原生移除后缓存仍然存在')
    probe.command(['plugin', 'marketplace', 'remove', 'codex-warp', '--json'])
    require_same_tree(plugin, original_source)
    require(user_settings(probe.config) == probe.settings, '生命周期结束后用户设置改变')
    report['checks']['native_remove_deleted_cache_and_preserved_source'] = True
    report['checks']['unrelated_isolated_user_settings_unchanged'] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex-executable', type=Path, required=True)
    parser.add_argument('--codex-version', choices=SUPPORTED_VERSIONS, default=DEFAULT_VERSION)
    parser.add_argument('--upstream-plugin', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    require(args.codex_executable.is_absolute() and args.output.is_absolute(), 'CLI 和输出必须为绝对路径')
    require(not args.output.resolve().is_relative_to(repo), '证据输出必须位于源树外')
    require(not Path(tempfile.gettempdir()).resolve().is_relative_to(repo), '临时 HOME 必须位于源树外')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    release = release_contract(args.codex_version)
    report = {'passed': False, 'host_os': sys.platform, 'expected_codex_version': release['version'],
              'codex_release_tag': release['tag'], 'codex_source_commit': release['commit'],
              'plugin_source_commit': PLUGIN_COMMIT, 'commands': [], 'states': {}, 'config_api_traces': [],
              'hook_listings': [], 'checks': {}, 'credentials_provided': False, 'model_entrypoints_called': [],
              'app_server_rpc_methods': ['initialize', 'config/value/write', 'hooks/list'],
              'scope': 'native_registry_and_failed_reinstall_only', 'product_installer_exercised': False,
              'native_plugin_update_subcommand_supported': False, 'git_marketplace_upgrade_verified': False,
              'new_plugin_version_upgrade_verified': False, 'native_hook_execution_verified': False}
    try:
        with tempfile.TemporaryDirectory(prefix='infinishell-plugin-lifecycle-') as temporary:
            directory = Path(temporary).resolve()
            base = plugin_base(repo)
            upstream = args.upstream_plugin
            if upstream is None:
                upstream = directory / 'fixed-upstream'
                for name, digest in base['tree_sha256'].items():
                    fetch_file(f'https://raw.githubusercontent.com/warpdotdev/codex-warp/{PLUGIN_COMMIT}/plugins/warp/{name}',
                               upstream / name, digest)
            require(upstream.is_absolute(), '上游原始目录必须是绝对路径')
            verify_plugin(upstream, base)
            run_probe(args.codex_executable.resolve(), upstream, directory, report, args.codex_version)
            verify_plugin(upstream, base)
        report['passed'] = True
    except Exception as error:
        report['failure'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
