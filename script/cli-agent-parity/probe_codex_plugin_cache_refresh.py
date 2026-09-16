#!/usr/bin/env python3
"""复现 Codex 原生后台覆盖缓存修补，并验证同 ID 本地来源迁移；只操作私有 HOME。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

sys.dont_write_bytecode = True
from apply_notification_patch import apply_files, bundle_data, default_bundle
from codex_windows_hook_inputs import CODEX_COMMIT, PLUGIN_COMMIT, plugin_base, require, verify_plugin
from probe_codex_plugin_lifecycle import tree_hashes
from probe_codex_windows_hooks import NativeRecorder


PLUGIN_ID = 'warp@codex-warp'
UPSTREAM_URL = 'https://github.com/warpdotdev/codex-warp.git'


def configuration(home):
    return tomllib.loads((home / 'config.toml').read_text(encoding='utf-8'))


def expected_patch_tree(metadata):
    tree = metadata['compatible_bases'][0]['tree_sha256'].copy()
    for name, hashes in metadata['files'].items():
        tree[name] = hashes['replacement_sha256']
    return tree


def cli(executable, arguments, env, directory, report, expected_code=0):
    require(arguments[0] in ('--version', 'plugin'), '拒绝模型或会话执行命令')
    completed = subprocess.run([str(executable), *arguments], env=env, cwd=directory,
                               capture_output=True, text=True, encoding='utf-8', timeout=45)
    record = {'arguments': arguments, 'exit_code': completed.returncode,
              'stdout': completed.stdout, 'stderr': completed.stderr}
    report['commands'].append(record)
    require(completed.returncode == expected_code, f'原生命令失败: {arguments}: {completed.stderr}')
    return completed.stdout


def native_listing(executable, env, directory, report, stage, cache, expected_revert=None, disable_id=None):
    trace = {'stage': stage, 'events': []}
    report['app_server_traces'].append(trace)
    recorder = NativeRecorder([str(executable), 'app-server', '--stdio'], env, directory, trace['events'])
    try:
        initialized = recorder.rpc('initialize', {'clientInfo': {
            'name': 'infinishell_cache_refresh_probe', 'version': '0.1.0'},
            'capabilities': {'experimentalApi': True}}, 1)
        require(Path(initialized['codexHome']).resolve() == Path(env['CODEX_HOME']).resolve(),
                '原生作用域不是隔离 CODEX_HOME')
        recorder.send({'method': 'initialized'})
        if disable_id is not None:
            require(disable_id in (PLUGIN_ID, 'orchestration@codex-warp'), '不能修改测试范围外的插件')
            result = recorder.rpc('config/value/write', {'keyPath': f'plugins."{disable_id}".enabled',
                'value': False, 'mergeStrategy': 'upsert', 'filePath': str(Path(env['CODEX_HOME']) / 'config.toml')}, 2)
            require(result['status'] == 'ok', '原生禁用没有确认')
        hooks = recorder.rpc('hooks/list', {'cwds': [str(directory)]}, 3)
        trace['hooks'] = hooks
        if expected_revert is not None:
            # 等待实际文件变化，不把初始化成功误当作后台刷新已经完成。
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                try:
                    if tree_hashes(cache) == expected_revert:
                        trace['background_reverted_to_upstream'] = True
                        break
                except (FileNotFoundError, ValueError):
                    # 原生原子替换目录的短窗口不属于最终结果。
                    pass
                time.sleep(0.02)
            require(trace.get('background_reverted_to_upstream') is True,
                    '没有在期限内复现真实后台还原；不能把源码推断当实证')
        return hooks
    finally:
        recorder.close()


def verify_complete_git_source(root, env):
    def git(arguments):
        return subprocess.run(['git', '-C', str(root), *arguments], env=env,
                              capture_output=True, check=True, timeout=15).stdout
    require(git(['rev-parse', 'HEAD']).decode().strip() == PLUGIN_COMMIT, 'marketplace 不在固定提交')
    require(not git(['diff', '--name-only', 'HEAD', '--']).strip(), '固定来源有被修改的跟踪文件')
    untracked = git(['ls-files', '--others', '--exclude-standard']).decode().splitlines()
    require(set(untracked) <= {'.codex-marketplace-install.json'}, '固定来源存在额外未跟踪文件')
    tracked = git(['ls-files', '-z']).decode().split('\0')
    tracked = [name for name in tracked if name]
    for name in tracked:
        require(not (root / name).is_symlink(), '固定来源含未支持的符号链接')
    return {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in tracked}


def run(executable, directory, report):
    repo = Path(__file__).resolve().parents[2]
    metadata, replacements = bundle_data(default_bundle(), 'codex')
    raw_tree = plugin_base(repo)['tree_sha256']
    patched_tree = expected_patch_tree(metadata)
    home = directory / 'codex'
    user_home = directory / 'home'
    home.mkdir()
    user_home.mkdir()
    env = {key: os.environ[key] for key in ('PATH', 'TMPDIR', 'TMP', 'TEMP', 'SYSTEMROOT', 'WINDIR') if key in os.environ}
    env.update(HOME=str(user_home), USERPROFILE=str(user_home), CODEX_HOME=str(home),
               APPDATA=str(user_home / 'AppData/Roaming'), LOCALAPPDATA=str(user_home / 'AppData/Local'),
               GIT_CONFIG_NOSYSTEM='1', GIT_TERMINAL_PROMPT='0')
    for key in ('APPDATA', 'LOCALAPPDATA'):
        Path(env[key]).mkdir(parents=True)
    (home / 'config.toml').write_text('approval_policy = "on-request"\nsandbox_mode = "read-only"\n', encoding='utf-8')
    version = cli(executable, ['--version'], env, directory, report).strip()
    require(version == 'codex-cli 0.147.0', '必须使用固定 Codex 0.147.0')
    report['cli'] = version
    with executable.open('rb') as source:
        report['executable_sha256'] = hashlib.file_digest(source, 'sha256').hexdigest()

    added = json.loads(cli(executable, ['plugin', 'marketplace', 'add', UPSTREAM_URL,
        '--ref', PLUGIN_COMMIT, '--json'], env, directory, report))
    git_source = Path(added['installedRoot'])
    complete_source = verify_complete_git_source(git_source, env)
    verify_plugin(git_source / 'plugins/warp', plugin_base(repo))
    installed = json.loads(cli(executable, ['plugin', 'add', PLUGIN_ID, '--json'], env, directory, report))
    cache = Path(installed['installedPath'])
    require(cache.resolve().is_relative_to(home), '缓存越出私有 HOME')
    verify_plugin(cache, plugin_base(repo))
    apply_files(cache, metadata, replacements)
    require(tree_hashes(cache) == patched_tree, '缓存补丁初始校验失败')
    report['cache_only_before_restart'] = {'tree': tree_hashes(cache), 'config': configuration(home)}
    require(configuration(home)['marketplaces']['codex-warp'].get('last_revision') is None,
            '原生首次 add 的配置与已知触发前提不符')
    native_listing(executable, env, directory, report, 'cache_only_restart', cache, expected_revert=raw_tree)
    report['cache_only_after_restart'] = {'tree': tree_hashes(cache), 'config': configuration(home)}
    require(configuration(home)['marketplaces']['codex-warp']['last_revision'] == PLUGIN_COMMIT,
            '原生后台没有记录实际刷新修订')
    require(verify_complete_git_source(git_source, env) == complete_source, '后台来源偏离固定提交')

    orchestration = json.loads(cli(executable, ['plugin', 'add', 'orchestration@codex-warp', '--json'],
                                   env, directory, report))
    orchestration_cache = Path(orchestration['installedPath'])
    require(orchestration_cache.resolve().is_relative_to(home), '编排缓存越出私有 HOME')
    orchestration_tree = tree_hashes(orchestration_cache)
    require(orchestration_tree == tree_hashes(git_source / 'plugins/orchestration'), '原生编排安装来源不一致')
    native_listing(executable, env, directory, report, 'explicit_orchestration_disable', cache,
                   disable_id='orchestration@codex-warp')

    # 保留完整固定 marketplace 的全部插件；不能只留下 warp 而破坏同来源的 orchestration。
    owned_source = directory / 'owned-source/codex-0.147.0-rev3/codex-warp'
    owned_source.mkdir(parents=True)
    for name in complete_source:
        target = owned_source / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(git_source / name, target)
    require(tree_hashes(owned_source) == complete_source, '完整本地来源复制不一致')
    apply_files(owned_source / 'plugins/warp', metadata, replacements)
    owned_tree = tree_hashes(owned_source)
    require({name: digest for name, digest in owned_tree.items() if not name.startswith('plugins/warp/')} ==
            {name: digest for name, digest in complete_source.items() if not name.startswith('plugins/warp/')},
            '本地来源改动了其他插件或 marketplace 索引')

    other = directory / 'other-marketplace'
    (other / '.agents/plugins').mkdir(parents=True)
    (other / '.agents/plugins/marketplace.json').write_text(json.dumps({'name': 'fixture-other', 'plugins': []}), encoding='utf-8')
    cli(executable, ['plugin', 'marketplace', 'add', str(other), '--json'], env, directory, report)
    before_migration = configuration(home)
    before_conflict = (home / 'config.toml').read_bytes()
    cli(executable, ['plugin', 'marketplace', 'add', str(owned_source), '--json'], env, directory, report, expected_code=1)
    require((home / 'config.toml').read_bytes() == before_conflict, '原生来源冲突错误改变了配置')
    cli(executable, ['plugin', 'marketplace', 'remove', 'codex-warp', '--json'], env, directory, report)
    require(configuration(home)['plugins'] == before_migration['plugins'] and cache.exists(),
            '仅移除 marketplace 意外改变插件启用或已安装缓存')
    cli(executable, ['plugin', 'marketplace', 'add', str(owned_source), '--json'], env, directory, report)
    cli(executable, ['plugin', 'add', PLUGIN_ID, '--json'], env, directory, report)
    require(tree_hashes(cache) == patched_tree, '原生安装没有使用完整已修补来源')
    migrated = configuration(home)
    require(migrated['marketplaces']['codex-warp']['source_type'] == 'local', '原生配置没有选中本地来源')
    require(migrated['marketplaces']['fixture-other'] == before_migration['marketplaces']['fixture-other'],
            '迁移改变了其他 marketplace')
    require(migrated['plugins'] == before_migration['plugins'], '迁移改变了已有插件 ID 或启用值')
    require(migrated['plugins']['orchestration@codex-warp']['enabled'] is False and
            tree_hashes(orchestration_cache) == orchestration_tree, '迁移改变了已装编排插件或其显式禁用')
    report['same_id_migration'] = {'config': migrated, 'owned_source_tree': owned_tree}

    hooks = native_listing(executable, env, directory, report, 'owned_source_restart', cache)
    native_hooks = [hook for group in hooks['data'] for hook in group['hooks'] if hook.get('pluginId') == PLUGIN_ID]
    require(len(native_hooks) == 5 and all(hook['trustStatus'] == 'untrusted' for hook in native_hooks),
            '迁移后 hooks 缺失、重复或被意外授信')
    require(tree_hashes(cache) == patched_tree, '原生重启覆盖了本地来源修补')
    upgraded = json.loads(cli(executable, ['plugin', 'marketplace', 'upgrade', '--json'], env, directory, report))
    require(not upgraded['selectedMarketplaces'] and not upgraded['upgradedRoots'] and not upgraded['errors'],
            '本地来源意外进入 Git 自动更新路径')
    # 主动要求原生重新复制缓存，证明持久性来自来源本身，而非恰好尚未触发复制。
    cli(executable, ['plugin', 'add', PLUGIN_ID, '--json'], env, directory, report)
    native_listing(executable, env, directory, report, 'owned_source_reinstall_then_restart', cache)
    require(tree_hashes(cache) == patched_tree and tree_hashes(owned_source) == owned_tree,
            '重装或再次启动后来源及缓存不一致')
    native_listing(executable, env, directory, report, 'explicit_native_disable', cache, disable_id=PLUGIN_ID)
    disabled_config = (home / 'config.toml').read_bytes()
    hooks = native_listing(executable, env, directory, report, 'disabled_restart', cache)
    require(all(not group['hooks'] and not group['errors'] for group in hooks['data']), '禁用后仍加载 hook 或发生错误')
    require((home / 'config.toml').read_bytes() == disabled_config, '重启改变了显式禁用或其他配置')
    require(tree_hashes(cache) == patched_tree and tree_hashes(owned_source) == owned_tree, '禁用重启改变了受控文件')
    require(b'trusted_hash' not in disabled_config, '探测不应写入原生信任哈希')

    # 原生移除和添加是两个操作：证明中途失败留下空缺，再由显式原生命令恢复来源。
    disabled_before_switch = configuration(home)
    cli(executable, ['plugin', 'marketplace', 'remove', 'codex-warp', '--json'], env, directory, report)
    invalid_source = directory / 'missing-marketplace'
    cli(executable, ['plugin', 'marketplace', 'add', str(invalid_source), '--json'],
        env, directory, report, expected_code=1)
    require('codex-warp' not in configuration(home).get('marketplaces', {}), '失败添加意外写入来源')
    report['native_switch_failure_before_recovery'] = {'config': configuration(home),
                                                       'requires_explicit_recovery': True}
    cli(executable, ['plugin', 'marketplace', 'add', str(owned_source), '--json'], env, directory, report)
    recovered = configuration(home)
    require(recovered['plugins'] == disabled_before_switch['plugins'] and
            recovered['marketplaces']['fixture-other'] == disabled_before_switch['marketplaces']['fixture-other'],
            '显式来源恢复改变了禁用或其他 marketplace')
    # 禁用插件不能通过 plugin add 恢复缓存；原生 add 会重新启用，已有缓存应直接保留。
    hooks = native_listing(executable, env, directory, report, 'disabled_source_recovery_restart', cache)
    require(all(not group['hooks'] and not group['errors'] for group in hooks['data']), '来源恢复重新启用了 hook')
    require(tree_hashes(cache) == patched_tree and tree_hashes(orchestration_cache) == orchestration_tree and
            tree_hashes(owned_source) == owned_tree, '来源恢复改变了原有缓存或完整来源')
    report['native_switch_recovery'] = {'config': recovered, 'plugin_add_used': False}
    report['checks'] = {'cache_only_patch_reverted_by_native_startup': True,
        'same_commit_first_refresh_reinstalled_cache': True, 'complete_other_plugin_sources_preserved': True,
        'installed_orchestration_cache_and_disabled_config_preserved': True,
        'same_plugin_id_preserved': True, 'other_marketplace_preserved': True,
        'owned_source_survives_native_reinstall_and_restarts': True,
        'explicit_disabled_state_survives_restart': True, 'no_automatic_hook_trust': True,
        'native_remove_add_failure_gap_observed': True, 'disabled_native_source_recovery_preserved_cache': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex-executable', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    require(args.output.is_absolute() and not args.output.resolve().is_relative_to(repo), '输出必须在源树外')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {'passed': False, 'codex_source_commit': CODEX_COMMIT, 'plugin_source_commit': PLUGIN_COMMIT,
              'host_os': sys.platform, 'commands': [], 'app_server_traces': [], 'credentials_provided': False,
              'thread_or_turn_created': False, 'product_installer_fixed': False, 'native_pty_verified': False}
    try:
        with tempfile.TemporaryDirectory(prefix='infinishell-cache-refresh-') as temporary:
            directory = Path(temporary).resolve()
            require(not directory.is_relative_to(repo), '隔离 HOME 必须在源树外')
            run(args.codex_executable.resolve(), directory, report)
        report['passed'] = True
    except Exception as error:
        report['failure'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
