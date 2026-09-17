"""正式五项注册与额外阻断 hook 的证据边界；不生成或预置原生信任摘要。"""

import hashlib
import json
from pathlib import Path
import re
import stat

from apply_notification_patch import atomic_write, bundle_data, checked_file
from codex_persistent_source import source_bundle, tree
from codex_windows_hook_inputs import require


EVENTS = ('sessionStart', 'userPromptSubmit', 'stop', 'permissionRequest', 'postToolUse')
NATIVE_FIELDS = ('key', 'eventName', 'handlerType', 'command', 'timeoutSec', 'source', 'pluginId', 'currentHash')


def probe_bundle(repo, mode):
    bundle = repo / 'app/assets/bundled/cli-agent-plugins'
    if mode == 'formal':
        metadata, replacements = bundle_data(bundle, 'codex')
        require(metadata['patch_revision'] == 4, '正式直验要求固定 rev4 配方')
        source, source_bytes, hashes = source_bundle(bundle)
        require(source['patch_revision'] == 4, '正式完整来源不是 rev4')
        for name, contents in replacements.items():
            require(hashes['plugins/warp/' + name] == hashlib.sha256(contents).hexdigest(),
                    '正式替换件与完整来源不一致')
        return metadata, replacements, hashlib.sha256(source_bytes).hexdigest()
    require(mode == 'candidate', '未知通知验收模式')
    # 旧候选从保留的 rev3 原字节出发，禁止向正式 rev4 再次应用候选变换。
    previous = bundle / 'codex/revisions/rev3'
    metadata = json.loads(checked_file(previous, 'PATCH_METADATA.json').read_bytes())
    require(metadata['patch_revision'] == 3 and len(metadata['files']) == 4, '候选基线不是固定 rev3')
    replacements = {}
    for name, entry in metadata['files'].items():
        root = previous if (previous / name).exists() else bundle / 'codex'
        contents = checked_file(root, name).read_bytes()
        require(hashlib.sha256(contents).hexdigest() == entry['replacement_sha256'], 'rev3 候选基线摘要不匹配')
        replacements[name] = contents
    return metadata, replacements, None


def exact_plugin_tree(plugin, metadata):
    expected = dict(metadata['compatible_bases'][0]['tree_sha256'])
    expected.update({name: entry['replacement_sha256'] for name, entry in metadata['files'].items()})
    require(tree(plugin) == expected, '正式插件完整树与固定配方不一致')
    return expected


def native_hooks(listed):
    return [hook for group in listed['data'] for hook in group['hooks']
            if hook.get('pluginId') == 'warp@codex-warp']


def validate_native_hooks(hooks, cli_home, plugin, expected_commands, trust):
    require(len(hooks) == len(expected_commands), '原生 hook 注册数量不匹配')
    require({hook['command'] for hook in hooks} == set(expected_commands), '原生没有选择预期的 Windows 命令')
    require(len({hook['key'] for hook in hooks}) == len(hooks), '原生 hook key 重复')
    require(all(hook['enabled'] and hook['trustStatus'] == trust and hook['source'] == 'plugin'
                and hook['handlerType'] == 'command' and re.fullmatch(r'sha256:[0-9a-f]{64}', hook['currentHash'])
                for hook in hooks), '原生 hook 类型、摘要或信任状态不符合预期')
    sources = {Path(hook['sourcePath']) for hook in hooks}
    require(len(sources) == 1, '原生 hook 来源不唯一')
    source = sources.pop()
    installed = source.parent.parent
    require(source == installed / 'hooks/hooks.json' and installed.resolve().is_relative_to(cli_home.resolve()),
            '原生插件不属于隔离 HOME')
    require(tree(installed) == tree(plugin), '原生已安装完整树与受控临时来源不一致')
    return installed


def same_native_registration(before, after):
    def entries(hooks):
        return sorted((tuple(hook[field] for field in NATIVE_FIELDS) for hook in hooks), key=lambda item: item[0])
    require(entries(before) == entries(after), '原生信任变更或阻断 hook 改变了正式注册项')


def split_trigger_hooks(formal, runtime, blocker):
    require(len(formal) == 5 and {hook['eventName'] for hook in formal} == set(EVENTS),
            '正式注册必须恰为五个通知事件')
    require(len(runtime) == 6, '触发验证必须为五项正式注册加一项测试阻断')
    original = [hook for hook in runtime if hook['command'] != blocker]
    extra = [hook for hook in runtime if hook['command'] == blocker]
    require(len(extra) == 1 and extra[0]['eventName'] == 'userPromptSubmit'
            and extra[0]['key'] not in {hook['key'] for hook in formal}, '额外阻断 hook 边界不明确')
    same_native_registration(formal, original)
    return {'formal_hook_count': 5, 'test_only_hook_count': 1, 'formal_registration_unchanged': True,
            'test_only_blocker': extra[0], 'formal_hooks': original,
            'native_events_verified': [], 'native_events_not_verified': list(EVENTS)}


def authorize_config(path, hooks):
    before = checked_file(path.parent, path.name).read_bytes()
    additions = ''.join(f'\n[hooks.state.{json.dumps(hook["key"])}]\nenabled = true\n'
                        f'trusted_hash = {json.dumps(hook["currentHash"])}\n' for hook in hooks).encode('utf-8')
    written = before + additions
    atomic_write(path, written, stat.S_IMODE(path.stat().st_mode))
    return before, written


def restore_config(path, before, written, evidence):
    evidence.update(attempted=True, restored=False, concurrent_change_detected=False)
    current = checked_file(path.parent, path.name).read_bytes()
    if current != written:
        evidence['concurrent_change_detected'] = True
        raise ValueError('隔离配置被并发修改，保留现场而不覆盖未知内容')
    atomic_write(path, before, stat.S_IMODE(path.stat().st_mode))
    require(path.read_bytes() == before, '隔离配置回滚字节不一致')
    evidence['restored'] = True


def formal_transport_expectations(directory, session, turn, prompt):
    require(all(isinstance(value, str) and value for value in (session, turn, prompt)), '缺少原生会话或回合标识')
    return {'provenance': 'app_server_request_response', 'cwd': str(directory),
            'session_id': session, 'turn_id': turn, 'prompt': prompt,
            'query_normalization_applied': False}
