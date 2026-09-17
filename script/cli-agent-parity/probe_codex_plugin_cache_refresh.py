#!/usr/bin/env python3
"""复现 Codex 原生后台覆盖缓存修补，并验证同 ID 本地来源迁移；只操作私有 HOME。"""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import tomllib

sys.dont_write_bytecode = True
from apply_notification_patch import apply_files, bundle_data, default_bundle
from codex_windows_hook_inputs import CODEX_COMMIT, PLUGIN_COMMIT, plugin_base, require, verify_plugin
from probe_codex_plugin_lifecycle import tree_hashes
from probe_codex_windows_hooks import NativeRecorder


PLUGIN_ID = 'warp@codex-warp'
UPSTREAM_URL = 'https://github.com/warpdotdev/codex-warp.git'


def failure_record(error):
    return {'type': type(error).__name__, 'message': str(error)}


class WindowsProbeJob:
    """仅此探针的私有 Job；不允许后台 Git 脱离，也不将回收当作自然退出。"""

    def __init__(self):
        import ctypes
        from ctypes import wintypes
        self.ctypes = ctypes
        self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        class Limits(ctypes.Structure):
            _fields_ = [('process_time', ctypes.c_int64), ('job_time', ctypes.c_int64),
                        ('flags', wintypes.DWORD), ('minimum', ctypes.c_size_t),
                        ('maximum', ctypes.c_size_t), ('active', wintypes.DWORD),
                        ('affinity', ctypes.c_size_t), ('priority', wintypes.DWORD),
                        ('scheduling', wintypes.DWORD)]
        class Extended(ctypes.Structure):
            _fields_ = [('basic', Limits), ('io', ctypes.c_uint64 * 6),
                        ('process_memory', ctypes.c_size_t), ('job_memory', ctypes.c_size_t),
                        ('peak_process', ctypes.c_size_t), ('peak_job', ctypes.c_size_t)]
        class Accounting(ctypes.Structure):
            _fields_ = [('times', ctypes.c_int64 * 4), ('faults', wintypes.DWORD),
                        ('total', wintypes.DWORD), ('active', wintypes.DWORD),
                        ('terminated', wintypes.DWORD)]
        self.accounting = Accounting
        signatures = {
            'CreateJobObjectW': ([ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
            'SetInformationJobObject': ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
            'AssignProcessToJobObject': ([wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
            'QueryInformationJobObject': ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p], wintypes.BOOL),
            'TerminateJobObject': ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            'TerminateProcess': ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            'ResumeThread': ([wintypes.HANDLE], wintypes.DWORD),
            'WaitForSingleObject': ([wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD),
            'CloseHandle': ([wintypes.HANDLE], wintypes.BOOL),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result
        self.handle = self.kernel.CreateJobObjectW(None, None)
        self.check(self.handle)
        try:
            limits = Extended()
            limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE，不授予 breakaway。
            self.check(self.kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)))
        except BaseException as error:
            try:
                self.close()
            except BaseException as cleanup_error:
                error.add_note('关闭新建私有 Job 失败: ' + str(cleanup_error))
            raise

    def check(self, value):
        if not value:
            raise self.ctypes.WinError(self.ctypes.get_last_error())

    def attach_and_resume(self, process, thread):
        self.check(self.kernel.AssignProcessToJobObject(self.handle, process))
        result = self.kernel.ResumeThread(thread)
        if result == 0xffffffff:
            raise self.ctypes.WinError(self.ctypes.get_last_error())
        require(result == 1, '私有进程主线程的暂停计数不符合创建契约')

    def reclaim_suspended(self, process):
        self.check(self.kernel.TerminateProcess(process, 1))
        require(self.kernel.WaitForSingleObject(process, 5000) == 0, '暂停进程没有确认退出')

    def active(self):
        data = self.accounting()
        self.check(self.kernel.QueryInformationJobObject(self.handle, 1, self.ctypes.byref(data), self.ctypes.sizeof(data), None))
        return data.active

    def terminate(self):
        self.check(self.kernel.TerminateJobObject(self.handle, 1))

    def wait_empty(self, timeout):
        deadline = time.monotonic() + timeout
        while True:
            active = self.active()
            if active == 0 or time.monotonic() >= deadline:
                return active
            time.sleep(0.02)

    def close(self):
        if self.handle:
            self.check(self.kernel.CloseHandle(self.handle))
            self.handle = None


@contextmanager
def suspended_creation(api, job, command, env, directory, trace):
    # CPython 3.13 的 Popen 会立即关闭 hThread；只在精确创建调用返回前附加 Job。
    original = api.CreateProcess
    owner = threading.get_ident()
    invoked = False
    def create(*arguments):
        nonlocal invoked
        if threading.get_ident() != owner:
            return original(*arguments)
        require(not invoked and len(arguments) == 9 and arguments[1] == subprocess.list2cmdline(command)
                and arguments[6] is env and arguments[7] == os.fspath(directory),
                '拒绝不匹配的 CPython 私有创建调用')
        invoked = True
        changed = list(arguments)
        changed[5] |= 0x00000004  # CREATE_SUSPENDED，避免先运行再附加的竞态。
        handles = original(*changed)
        process, thread, pid, _ = handles
        trace['pid'] = pid
        try:
            job.attach_and_resume(process, thread)
            trace['assigned_before_resume'] = True
        except BaseException as error:
            trace['startup_failure'] = failure_record(error)
            try:
                job.reclaim_suspended(process)
                trace['suspended_process_exit_confirmed'] = True
            except BaseException as cleanup_error:
                trace['startup_cleanup_failure'] = failure_record(cleanup_error)
            finally:
                for handle in (thread, process):
                    try:
                        api.CloseHandle(handle)
                    except BaseException as cleanup_error:
                        trace.setdefault('handle_close_failures', []).append(failure_record(cleanup_error))
            raise
        return handles
    api.CreateProcess = create
    try:
        yield
        require(invoked, '没有观察到预期的 CPython 进程创建')
    finally:
        api.CreateProcess = original


class CacheRefreshRecorder(NativeRecorder):
    def __init__(self, command, env, directory, events, trace):
        self.trace = trace
        self.job = None
        self.readers = []
        self.reader_errors = []
        if os.name == 'nt':
            import _winapi
            require(sys.implementation.name == 'cpython', 'Windows 探针需要已核实的 CPython 创建接口')
            try:
                self.job = WindowsProbeJob()
                with suspended_creation(_winapi, self.job, command, env, directory, trace):
                    super().__init__(command, env, directory, events)
            except BaseException as error:
                trace.setdefault('startup_failure', failure_record(error))
                try:
                    if hasattr(self, 'process'):
                        self.close()
                    elif self.job is not None:
                        # Popen 负责关闭创建失败时尚未移交的管道句柄。
                        self.job.close()
                except BaseException as cleanup_error:
                    trace.setdefault('startup_cleanup_failure', failure_record(cleanup_error))
                raise
        else:
            super().__init__(command, env, directory, events)

    def close(self):
        trace = self.trace
        started = time.monotonic()
        trace.update(root_exited_naturally=False, root_forced=False, descendants_forced=False,
                     readers_eof=False, cleanup_confirmed=False)
        try:
            try:
                self.process.stdin.close()
            except (BrokenPipeError, OSError) as error:
                trace['stdin_close_error'] = failure_record(error)
            try:
                self.process.wait(timeout=5)
                trace['root_exited_naturally'] = True
            except subprocess.TimeoutExpired:
                trace['root_forced'] = True
                if self.job is not None:
                    self.job.terminate()
                else:
                    self.process.kill()
                self.process.wait(timeout=5)
            trace['root_exit_code'] = self.process.returncode
            if self.job is not None:
                active = self.job.wait_empty(5)
                trace['job_active_after_grace'] = active
                if active:
                    trace['descendants_forced'] = True
                    self.job.terminate()
                    active = self.job.wait_empty(5)
                trace['job_active_after_cleanup'] = active
                require(active == 0, '私有 Job 中仍有未确认退出的进程')
            for reader in self.readers:
                reader.join(timeout=2)
            trace['reader_errors'] = list(self.reader_errors)
            trace['readers_eof'] = all(not reader.is_alive() for reader in self.readers)
            require(not self.reader_errors and trace['readers_eof'],
                    '原生输出读取失败或没有结束: ' + repr(self.reader_errors))
            trace['cleanup_confirmed'] = True
            require(trace['root_exited_naturally'] and self.process.returncode == 0,
                    '原生主进程没有自然正常退出；回收不能替代成功')
        finally:
            primary = sys.exc_info()[1]
            close_error = None
            if self.job is not None:
                try:
                    self.job.close()
                except BaseException as error:
                    trace['job_close_failure'] = failure_record(error)
                    trace['cleanup_confirmed'] = False
                    close_error = error
            # 只有读取线程确已退出时才能关闭文本流，避免阻塞在其内部锁。
            if all(not reader.is_alive() for reader in self.readers):
                for stream in (self.process.stdout, self.process.stderr):
                    try:
                        stream.close()
                    except BaseException as error:
                        trace.setdefault('stream_close_failures', []).append(failure_record(error))
                        trace['cleanup_confirmed'] = False
                        if close_error is None:
                            close_error = error
            trace['elapsed_ms'] = round((time.monotonic() - started) * 1000)
            if close_error is not None and primary is None:
                raise close_error


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
    trace = {'stage': stage, 'events': [], 'process_cleanup': {}}
    report['app_server_traces'].append(trace)
    recorder = CacheRefreshRecorder([str(executable), 'app-server', '--stdio'], env, directory,
                                    trace['events'], trace['process_cleanup'])
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
                    if background_refresh_published(cache, expected_revert, Path(env['CODEX_HOME'])):
                        trace['background_reverted_to_upstream'] = True
                        trace['background_revision_published'] = PLUGIN_COMMIT
                        break
                except (FileNotFoundError, ValueError):
                    # 原生原子替换目录的短窗口不属于最终结果。
                    pass
                time.sleep(0.02)
            require(trace.get('background_reverted_to_upstream') is True,
                    '没有在期限内复现真实后台还原；不能把源码推断当实证')
        return hooks
    finally:
        primary = sys.exc_info()[1]
        try:
            recorder.close()
        except BaseException as error:
            trace['close_failure'] = failure_record(error)
            if primary is None:
                raise


def background_refresh_published(cache, expected_tree, home):
    # 固定上游先发布 revision，再刷新缓存；两项都观察到才进入退出阶段。
    return (tree_hashes(cache) == expected_tree and
            configuration(home).get('marketplaces', {}).get('codex-warp', {}).get('last_revision') == PLUGIN_COMMIT)


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


def run_and_cleanup(executable, directory, report):
    try:
        run(executable, directory, report)
    except BaseException as error:
        report['failure'] = failure_record(error)
        # 保留失败现场；不得在活跃后台 Git 的目录上盲删并覆盖最初异常。
        report['private_directory_removed'] = False
        report['retained_private_directory'] = str(directory)
        raise
    try:
        shutil.rmtree(directory)
        report['private_directory_removed'] = True
    except BaseException as error:
        report['cleanup_failure'] = failure_record(error)
        report['private_directory_removed'] = False
        report['retained_private_directory'] = str(directory)
        raise


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
        directory = Path(tempfile.mkdtemp(prefix='infinishell-cache-refresh-')).resolve()
        require(not directory.is_relative_to(repo), '隔离 HOME 必须在源树外')
        run_and_cleanup(args.codex_executable.resolve(), directory, report)
        report['passed'] = True
    except Exception as error:
        report.setdefault('failure', failure_record(error))
        raise
    finally:
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
