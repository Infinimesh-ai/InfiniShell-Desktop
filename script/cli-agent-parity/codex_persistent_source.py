#!/usr/bin/env python3
"""Codex 手动包的固定完整来源迁移；原生安装先在私有暂存 HOME 验证。"""

import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import tomllib

from apply_notification_patch import atomic_write, bundle_data, checked_file, digest, validate_tree

COMMIT = '31ce59d9011cfb1d78f265649a228dac5de58d76'
PLUGIN = 'warp@codex-warp'
MARKETPLACE = 'codex-warp'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def tree(root, ignore_git=False):
    result = {}
    require(not root.is_symlink(), '拒绝链接来源目录')
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in directories:
            require(not (Path(directory) / name).is_symlink(), '来源目录含链接')
        if ignore_git and Path(directory) == root:
            directories[:] = [name for name in directories if name != '.git']
            files = [name for name in files if name != '.codex-marketplace-install.json']
        for name in files:
            relative = (Path(directory) / name).relative_to(root).as_posix()
            result[relative] = digest(checked_file(root, relative).read_bytes())
    return result


def verify_modes(root, files):
    if os.name == 'posix':
        for name, entry in files.items():
            require(stat.S_IMODE(checked_file(root, name).stat().st_mode) == entry['mode'],
                    '完整来源模式位发生变化')


def source_bundle(bundle):
    root = bundle / 'codex'
    metadata_bytes = checked_file(root, 'SOURCE_METADATA.json').read_bytes()
    metadata = json.loads(metadata_bytes)
    require(metadata['upstream_commit'] == COMMIT and metadata['cli_contract_version'] == '0.147.0',
            '完整来源不符合固定受测契约')
    expected = {name: entry['sha256'] for name, entry in metadata['files'].items()}
    require(len(expected) == 36 and tree(root / 'source') == expected, '随附完整来源摘要不一致')
    verify_modes(root / 'source', metadata['files'])
    return metadata, metadata_bytes, expected


def config(home):
    path = home / 'config.toml'
    require(not path.is_symlink(), '配置不能是链接')
    data = checked_file(home, 'config.toml').read_bytes() if path.exists() else None
    document = tomllib.loads(data.decode('utf-8')) if data is not None else {}
    for parent in ('marketplaces', 'plugins'):
        require(parent not in document or isinstance(document[parent], dict), '配置父表类型不正确，保持原文件')
    marketplace = document.get('marketplaces', {}).get(MARKETPLACE)
    require(marketplace is None or isinstance(marketplace, dict), '目标 marketplace 类型不正确')
    for key in (PLUGIN, 'orchestration@codex-warp'):
        plugin = document.get('plugins', {}).get(key)
        require(plugin is None or isinstance(plugin, dict), '目标插件配置类型不正确')
        if plugin is not None:
            require('enabled' not in plugin or isinstance(plugin['enabled'], bool), '目标启用值不是布尔值')
    return data, document


def source_path(home, metadata):
    return home / 'plugins/infinishell-sources' / metadata['directory'] / 'source'


def verify_owned(home, bundle):
    metadata, metadata_bytes, expected = source_bundle(bundle)
    source = source_path(home, metadata)
    require(checked_file(source.parent, 'SOURCE_METADATA.json').read_bytes() == metadata_bytes and
            tree(source) == expected, '受控来源被修改、缺失或不完整')
    verify_modes(source, metadata['files'])
    return source


def previous_revision(bundle):
    data = checked_file(bundle / 'codex/revisions/rev3', 'SOURCE_METADATA.json').read_bytes()
    metadata = json.loads(data)
    require(metadata['upstream_commit'] == COMMIT and metadata['cli_contract_version'] == '0.147.0'
            and metadata['patch_revision'] == 3 and metadata['directory'] == 'codex-warp-0.4.0-rev3'
            and len(metadata['files']) == 36, '旧版本迁移契约不是固定 rev3')
    return metadata, data


def verify_previous(home, bundle):
    metadata, data = previous_revision(bundle)
    source = source_path(home, metadata)
    require(checked_file(source.parent, 'SOURCE_METADATA.json').read_bytes() == data
            and tree(source) == {name: entry['sha256'] for name, entry in metadata['files'].items()},
            '旧 rev3 来源有修改、缺失或额外文件')
    verify_modes(source, metadata['files'])
    return source


def verify_previous_cache(root, bundle):
    metadata, _ = previous_revision(bundle)
    files = {name.removeprefix('plugins/warp/'): entry for name, entry in metadata['files'].items()
             if name.startswith('plugins/warp/')}
    require(tree(root) == {name: entry['sha256'] for name, entry in files.items()},
            '旧缓存不是完整固定 rev3，拒绝新旧混合或自定义脚本')
    verify_modes(root, files)


def validate_source(home, settings, bundle):
    metadata, _, _ = source_bundle(bundle)
    entry = settings.get('marketplaces', {}).get(MARKETPLACE)
    if entry is None:
        return False
    if entry.get('source_type') == 'local' and entry.get('source') == str(source_path(home, metadata)):
        verify_owned(home, bundle)
        return True
    previous, _ = previous_revision(bundle)
    if entry.get('source_type') == 'local' and entry.get('source') == str(source_path(home, previous)):
        verify_previous(home, bundle)
        # 旧版可以迁移，但检查命令不能把它报告为当前配方。
        return False
    require(entry.get('source_type') == 'git' and entry.get('source') in (
        'https://github.com/warpdotdev/codex-warp.git', 'https://github.com/warpdotdev/codex-warp',
        'warpdotdev/codex-warp') and entry.get('ref') in (None, COMMIT) and
        entry.get('last_revision') in (None, COMMIT) and 'sparse_paths' not in entry,
        '自定义或未知 marketplace 保持原样，不自动替换')
    snapshot = home / '.tmp/marketplaces/codex-warp'
    if snapshot.exists():
        require(tree(snapshot, True) == {name: entry['upstream_sha256'] for name, entry in metadata['files'].items()},
                '原始 marketplace 有用户修改或不是固定版本')
        verify_modes(snapshot, metadata['files'])
    return False


def validate_caches(home, bundle):
    metadata, _ = bundle_data(bundle, 'codex')
    root = home / 'plugins/cache/codex-warp/warp'
    if root.exists():
        entries = list(root.iterdir())
        require(len(entries) == 1 and not entries[0].is_symlink(), '目标缓存版本不唯一')
        manifest = json.loads(checked_file(entries[0], '.codex-plugin/plugin.json').read_bytes())
        require(manifest['name'] == 'warp' and manifest['version'] == entries[0].name, '插件版本目录不一致')
        try:
            validate_tree(entries[0], manifest['version'], metadata)
        except ValueError:
            require(manifest['version'] == '0.4.0', '旧缓存版本不是受测版本')
            verify_previous_cache(entries[0], bundle)
    orchestration = home / 'plugins/cache/codex-warp/orchestration'
    if orchestration.exists():
        source, _, _ = source_bundle(bundle)
        expected = {name.removeprefix('plugins/orchestration/'): entry['sha256'] for name, entry in source['files'].items()
                    if name.startswith('plugins/orchestration/')}
        require([path.name for path in orchestration.iterdir()] == ['0.4.0'] and
                tree(orchestration / '0.4.0') == expected, '原有编排插件存在未知版本或修改')


def materialize(home, bundle):
    metadata, metadata_bytes, expected = source_bundle(bundle)
    source = source_path(home, metadata)
    for ancestor in source.parents:
        require(not ancestor.is_symlink(), '拒绝通过链接写入来源')
    if source.parent.exists():
        return verify_owned(home, bundle)
    source.parent.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.prepare-', dir=source.parent.parent) as temporary:
        stage = Path(temporary)
        for name, entry in metadata['files'].items():
            target = stage / 'source' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(checked_file(bundle / 'codex/source', name).read_bytes())
            target.chmod(entry['mode'])
        (stage / 'SOURCE_METADATA.json').write_bytes(metadata_bytes)
        require(tree(stage / 'source') == expected, '完整来源写入验证失败')
        require(not source.parent.exists(), '来源发布前出现并发版本目录')
        os.rename(stage, source.parent)
    return verify_owned(home, bundle)


def native(executable, home, arguments):
    environment = os.environ.copy()
    environment['CODEX_HOME'] = str(home)
    result = subprocess.run([executable, 'plugin', *arguments], env=environment,
                            capture_output=True, text=True, encoding='utf-8', timeout=60)
    require(result.returncode == 0, f'原生暂存插件命令失败：{result.stderr[:4096]}')


def unchanged_except_target(before, after):
    # 只允许原生改动单一 marketplace 及目标 enabled；其他用户内容必须语义相等。
    import copy
    before = copy.deepcopy(before)
    after = copy.deepcopy(after)
    for document in (before, after):
        marketplaces = document.get('marketplaces', {})
        marketplaces.pop(MARKETPLACE, None)
        if not marketplaces:
            document.pop('marketplaces', None)
        plugins = document.get('plugins', {})
        target = plugins.get(PLUGIN, {})
        target.pop('enabled', None)
        if not target:
            plugins.pop(PLUGIN, None)
        if not plugins:
            document.pop('plugins', None)
    return before == after


def cache_snapshot(root):
    return {name: value + ':' + str(stat.S_IMODE(checked_file(root, name).stat().st_mode))
            for name, value in tree(root).items()}


def record_phase(stage, phase):
    path = stage / 'state.json'
    state = json.loads(path.read_bytes())
    state['phase'] = phase
    atomic_write(path, (json.dumps(state, indent=2) + '\n').encode(), 0o600)


def commit(home, stage, original_bytes, installed_bytes, bundle, before_step=lambda step: None):
    target = home / 'plugins/cache/codex-warp/warp'
    staged = stage / 'codex/plugins/cache/codex-warp/warp'
    backup = stage / 'previous-cache'
    before_tree = cache_snapshot(target) if target.exists() else None
    after_tree = cache_snapshot(staged)
    (stage / 'state.json').write_text(json.dumps({'plugin': PLUGIN, 'phase': 'prepared',
        'old_config_sha256': digest(original_bytes) if original_bytes is not None else None,
        'installed_config_sha256': digest(installed_bytes), 'original_cache': before_tree,
        'installed_cache': after_tree}, indent=2) + '\n')
    before_step(0)
    require(config(home)[0] == original_bytes, '配置发生并发变化，未迁入')
    validate_caches(home, bundle)
    target.parent.mkdir(parents=True, exist_ok=True)
    if before_tree is not None:
        require(cache_snapshot(target) == before_tree, '缓存发生并发变化，未迁入')
        os.rename(target, backup)
    cache_written = False
    config_written = False
    try:
        os.rename(staged, target)
        cache_written = True
        record_phase(stage, 'cache_written')
        before_step(1)
        require(config(home)[0] == original_bytes, '提交前配置或禁用状态变化，未覆盖')
        mode = stat.S_IMODE((home / 'config.toml').stat().st_mode) if original_bytes is not None else 0o600
        atomic_write(home / 'config.toml', installed_bytes, mode)
        config_written = True
        record_phase(stage, 'configuration_written')
        before_step(2)
        require(config(home)[0] == installed_bytes and cache_snapshot(target) == after_tree, '提交后发现配置或缓存漂移')
        verify_owned(home, bundle)
        record_phase(stage, 'verified')
    except Exception as error:
        recovery_errors = []
        if config_written:
            if config(home)[0] == installed_bytes:
                if original_bytes is None:
                    (home / 'config.toml').unlink()
                else:
                    atomic_write(home / 'config.toml', original_bytes, mode)
            else:
                recovery_errors.append('配置被并发修改')
        if cache_written:
            if cache_snapshot(target) == after_tree:
                shutil.rmtree(target)
            else:
                recovery_errors.append('缓存被并发修改')
        if backup.exists() and not target.exists():
            os.rename(backup, target)
        record_phase(stage, 'needs_review' if recovery_errors else 'rolled_back')
        if recovery_errors:
            raise ValueError(f'恢复未完成，保留并发变化和 {stage}：' + '、'.join(recovery_errors)) from error
        raise


def install_source(home, executable, bundle, check=False, before_step=lambda step: None):
    home = home.resolve()
    home.mkdir(parents=True, exist_ok=True)
    original_bytes, settings = config(home)
    require(settings.get('plugins', {}).get(PLUGIN, {}).get('enabled') is not False, '插件已被用户禁用，保持禁用')
    current = validate_source(home, settings, bundle)
    validate_caches(home, bundle)
    preserve_previous_cache = False
    previous_cache = home / 'plugins/cache/codex-warp/warp/0.4.0'
    if previous_cache.exists():
        try:
            verify_previous_cache(previous_cache, bundle)
            preserve_previous_cache = True
        except ValueError:
            pass
    if check:
        require(current and settings.get('plugins', {}).get(PLUGIN, {}).get('enabled') is True,
                '尚未启用完整持久来源')
        return
    source = materialize(home, bundle)
    transactions = home / 'plugins/infinishell-transactions'
    transactions.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix='manual-', dir=transactions))
    staged_home = stage / 'codex'
    staged_home.mkdir()
    # 私有目录为 0700，不记录配置全文；暂存原生写入可保留用户格式及不相关设置。
    if original_bytes is not None:
        (staged_home / 'config.toml').write_bytes(original_bytes)
    try:
        if MARKETPLACE in settings.get('marketplaces', {}):
            native(executable, staged_home, ['marketplace', 'remove', MARKETPLACE])
        native(executable, staged_home, ['marketplace', 'add', str(source)])
        native(executable, staged_home, ['add', PLUGIN])
        installed_bytes, installed = config(staged_home)
        require(installed_bytes is not None and unchanged_except_target(settings, installed), '原生暂存配置改变了不相关字段')
        require(installed['plugins'][PLUGIN]['enabled'] is True and
                installed['marketplaces'][MARKETPLACE].get('source_type') == 'local' and
                installed['marketplaces'][MARKETPLACE].get('source') == str(source), '原生暂存来源不是最终持久路径')
        require(tree(staged_home / 'plugins/cache/codex-warp/warp/0.4.0') == tree(source / 'plugins/warp'),
                '原生暂存缓存与完整来源不一致')
        commit(home, stage, original_bytes, installed_bytes, bundle, before_step)
    except Exception as error:
        raise ValueError(f'{error}；事务资料保留于 {stage}') from error
    # 成功的旧版升级也留下 previous-cache/state.json；不删除用户已有的受控旧树。
    if not preserve_previous_cache:
        shutil.rmtree(stage)
