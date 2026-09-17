"""真实 Windows ConPTY 中运行固定 Codex hooks 候选；不验证普通 TUI 或模型生命周期。"""

import argparse
import ctypes
from ctypes import wintypes
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid

sys.dont_write_bytecode = True
from codex_windows_hook_command import source_text, windows_environment
from codex_windows_hook_inputs import obtain_inputs, require, sha256


MAX_OUTPUT = 16 * 1024 * 1024
OSC = re.compile(rb'\x1b\]777;notify;warp://cli-agent;(.*?)(?:\x07|\x1b\\)', re.DOTALL)


def shell_diagnostic_source():
    # 只在私有 BASH_ENV 中观察真实入口；不替换发布脚本、不读取 hook stdin、不写终端。
    return r'''case "$0" in
    */on-session-start.sh.fixture-original|*/on-prompt-submit.sh.fixture-original|*/warp-notify.sh)
    (
        set +e
        diagnostic_id=$BASHPID
        script_dir=${0%/*}
        gate=unavailable
        if [ -f "$script_dir/should-use-structured.sh" ]; then
            source "$script_dir/should-use-structured.sh"
            if should_use_structured; then gate=allowed; else gate=rejected; fi
        fi
        stdin_tty=false; stdout_tty=false; stderr_tty=false
        [ -t 0 ] && stdin_tty=true
        [ -t 1 ] && stdout_tty=true
        [ -t 2 ] && stderr_tty=true
        tty_error=$( (exec 9>/dev/tty) 2>&1)
        tty_status=$?
        windows_pid=$(cat "/proc/$$/winpid" 2>/dev/null)
        system=$(uname -s 2>/dev/null)
        notification=null
        if [[ "$0" == */warp-notify.sh ]]; then
            notification=$(printf '%s' "${2:-null}" | jq -c '
                if type == "object" then {v,agent,event,session_id,turn_id} else null end' 2>/dev/null)
            [ -n "$notification" ] || notification=null
        fi
        set -o noclobber
        jq -nc --arg script "$0" --arg case_dir "${INFINISHELL_HOOK_PROBE_CASE:-}" \
            --arg gate "$gate" --arg protocol "${WARP_CLI_AGENT_PROTOCOL_VERSION:-}" \
            --arg client "${WARP_CLIENT_VERSION:-}" --arg bash_version "$BASH_VERSION" \
            --arg system "$system" --arg msys_pid "$$" --arg windows_pid "$windows_pid" \
            --argjson stdin_tty "$stdin_tty" --argjson stdout_tty "$stdout_tty" \
            --argjson stderr_tty "$stderr_tty" --argjson tty_open_status "$tty_status" \
            --arg tty_open_error "$tty_error" --argjson notification "$notification" \
            '{version:1,script:$script,case_dir:$case_dir,structured_gate:$gate,
              protocol:$protocol,client:$client,bash_version:$bash_version,system:$system,
              msys_pid:$msys_pid,windows_pid:$windows_pid,stdin_tty:$stdin_tty,
              stdout_tty:$stdout_tty,stderr_tty:$stderr_tty,tty_open_status:$tty_open_status,
              tty_open_error:$tty_open_error,notification:$notification}' \
            > "$INFINISHELL_CONPTY_DIAGNOSTICS_DIR/$diagnostic_id.json"
    ) >/dev/null 2>/dev/null
    ;;
esac
'''


def read_shell_diagnostics(directory):
    files = sorted(directory.glob('*.json'))
    require(len(files) <= 64, '实际通知入口诊断超过固定上限')
    records = []
    for path in files:
        require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 65536,
                '实际通知入口诊断不是受控的小文件')
        value = json.loads(path.read_text(encoding='utf-8'))
        require(isinstance(value, dict) and value.get('version') == 1
                and value.get('structured_gate') in ('allowed', 'rejected', 'unavailable')
                and type(value.get('tty_open_status')) is int
                and all(type(value.get(key)) is bool for key in ('stdin_tty', 'stdout_tty', 'stderr_tty'))
                and (value.get('notification') is None or isinstance(value['notification'], dict)),
                '实际通知入口诊断字段不符合契约')
        records.append(value)
    return records


def notifications(raw):
    return [json.loads(match.group(1).decode('utf-8', errors='strict')) for match in OSC.finditer(raw)]


def verify_transport(raw, cases):
    require(cases and all(case.get('passed') for case in cases), '原生 hook 场景未全部通过')
    observed = notifications(raw)
    matched = []
    for case in cases:
        markers = case['markers']
        session = markers['on-session-start.sh']['session_id']
        turn = markers['on-prompt-submit.sh']['turn_id']
        for event in ('session_start', 'prompt_submit'):
            found = [item for item in observed if item.get('v') == 1 and item.get('agent') == 'codex'
                     and item.get('event') == event and item.get('session_id') == session
                     and (event != 'prompt_submit' or item.get('turn_id') == turn)]
            require(len(found) == 1, f'{case["name"]} 的 {event} 必须真实且仅出现一次，实际 {len(found)}')
            matched.append(found[0])
    return matched


def require_candidate_contract():
    source = source_text()
    for assignment in ('$info.UseShellExecute = $false', '$info.CreateNoWindow = $false',
                       '$info.RedirectStandardInput = $false', '$info.RedirectStandardOutput = $false',
                       '$info.RedirectStandardError = $false'):
        require(source.count(assignment) == 1, '候选句柄/控制台契约改变: ' + assignment)
    return hashlib.sha256(source.encode('utf-8')).hexdigest()


def environment_block(environment):
    require(all('\0' not in key + value and '=' not in key for key, value in environment.items()),
            '环境块含无效字段')
    require(len({key.upper() for key in environment}) == len(environment), '环境键存在大小写重复')
    return '\0'.join(key + '=' + value for key, value in sorted(environment.items(), key=lambda item: item[0].upper())) + '\0\0'


class COORD(ctypes.Structure):
    _fields_ = [('X', ctypes.c_short), ('Y', ctypes.c_short)]


class STARTUPINFO(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('reserved', wintypes.LPWSTR), ('desktop', wintypes.LPWSTR),
                ('title', wintypes.LPWSTR), ('x', wintypes.DWORD), ('y', wintypes.DWORD),
                ('xsize', wintypes.DWORD), ('ysize', wintypes.DWORD), ('xchars', wintypes.DWORD),
                ('ychars', wintypes.DWORD), ('fill', wintypes.DWORD), ('flags', wintypes.DWORD),
                ('show', wintypes.WORD), ('reserved2size', wintypes.WORD), ('reserved2', ctypes.c_void_p),
                ('stdin', wintypes.HANDLE), ('stdout', wintypes.HANDLE), ('stderr', wintypes.HANDLE)]


class STARTUPINFOEX(ctypes.Structure):
    _fields_ = [('info', STARTUPINFO), ('attributes', ctypes.c_void_p)]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [('process', wintypes.HANDLE), ('thread', wintypes.HANDLE),
                ('pid', wintypes.DWORD), ('tid', wintypes.DWORD)]


class WinApi:
    def __init__(self):
        require(os.name == 'nt' and ctypes.sizeof(ctypes.c_void_p) == 8, '只允许原生 Windows x64 Python')
        self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        signatures = {
            'CreatePipe': ([ctypes.POINTER(wintypes.HANDLE), ctypes.POINTER(wintypes.HANDLE), ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
            'CloseHandle': ([wintypes.HANDLE], wintypes.BOOL),
            'InitializeProcThreadAttributeList': ([ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(ctypes.c_size_t)], wintypes.BOOL),
            'UpdateProcThreadAttribute': ([ctypes.c_void_p, wintypes.DWORD, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_void_p], wintypes.BOOL),
            'DeleteProcThreadAttributeList': ([ctypes.c_void_p], None),
            'CreateProcessW': ([wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p, wintypes.BOOL,
                                wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(STARTUPINFO), ctypes.POINTER(PROCESS_INFORMATION)], wintypes.BOOL),
            'ReadFile': ([wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p], wintypes.BOOL),
            'WaitForSingleObject': ([wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD),
            'TerminateProcess': ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            'GetExitCodeProcess': ([wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            'GetConsoleProcessList': ([ctypes.POINTER(wintypes.DWORD), wintypes.DWORD], wintypes.DWORD),
            'OpenProcess': ([wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE),
            'QueryFullProcessImageNameW': ([wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            'GetProcessTimes': ([wintypes.HANDLE, ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME)], wintypes.BOOL),
            'CreateFileW': ([wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE], wintypes.HANDLE),
            'GetConsoleMode': ([wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            'SetConsoleMode': ([wintypes.HANDLE, wintypes.DWORD], wintypes.BOOL),
            'WriteConsoleW': ([wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p], wintypes.BOOL),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result

    def check(self, result):
        if not result:
            raise ctypes.WinError(ctypes.get_last_error())

    def console_members(self):
        values = (wintypes.DWORD * 256)()
        count = self.kernel.GetConsoleProcessList(values, len(values))
        self.check(count)
        require(count <= len(values), '私有控制台成员超过固定探针容量')
        members = []
        for pid in values[:count]:
            handle = self.kernel.OpenProcess(0x1000, False, pid)
            if not handle:
                continue
            try:
                name, length = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768)
                times = [wintypes.FILETIME() for _ in range(4)]
                if self.kernel.QueryFullProcessImageNameW(handle, 0, name, ctypes.byref(length)) and self.kernel.GetProcessTimes(handle, *(ctypes.byref(value) for value in times)):
                    members.append({'pid': pid, 'image': name.value,
                                    'created': (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime})
            finally:
                self.kernel.CloseHandle(handle)
        return members

    def console_diagnostic(self, nonce):
        handle = self.kernel.CreateFileW('CONOUT$', 0xC0000000, 3, None, 3, 0, None)
        require(handle not in (None, ctypes.c_void_p(-1).value), '附着 driver 不能打开 CONOUT$')
        mode = wintypes.DWORD()
        try:
            self.check(self.kernel.GetConsoleMode(handle, ctypes.byref(mode)))
            self.check(self.kernel.SetConsoleMode(handle, mode.value | 4))
            # 仅诊断私有控制台的 VT 输出；随后恢复原模式，不能代替原生 hook 的验收。
            text = '\x1b]777;notify;warp://cli-agent;' + json.dumps({'diagnostic_nonce': nonce}) + '\x07'
            written = wintypes.DWORD()
            self.check(self.kernel.WriteConsoleW(handle, ctypes.c_wchar_p(text), len(text), ctypes.byref(written), None))
            require(written.value == len(text), 'CONOUT$ 诊断写入不完整')
        finally:
            try:
                self.check(self.kernel.SetConsoleMode(handle, mode.value))
            finally:
                self.kernel.CloseHandle(handle)
        return {'opened': True, 'original_mode': mode.value, 'mode_restored_before_native_hooks': True}


def run_driver(configuration):
    # 此入口只能由本探针附着到私有 HPCON；原生 RPC 仍使用独立标准管道。
    import types
    import probe_codex_windows_hooks as native
    config = json.loads(configuration.read_text(encoding='utf-8'))
    api = WinApi()
    report = {'passed': False, 'cases': [], 'model_http_requests': [], 'attachments': [],
              'native_codex_attachments': [], 'credentials_provided': False}
    stop = threading.Event()
    server = None
    shell_diagnostics = Path(config['private_dir']) / 'shell-diagnostics'
    shell_diagnostics.mkdir(parents=True)
    bash_environment = shell_diagnostics / 'observe-entry.sh'
    bash_environment.write_text(shell_diagnostic_source(), encoding='utf-8', newline='\n')
    report['shell_diagnostics_source_sha256'] = sha256(bash_environment)
    observed, observation_errors = {}, []

    def observe():
        while not stop.wait(0.005):
            try:
                for member in api.console_members():
                    observed[(member['pid'], member['created'])] = member
            except Exception as error:
                observation_errors.append(str(error))
                return

    observer = threading.Thread(target=observe, daemon=True)
    observer.start()
    original_recorder = native.NativeRecorder

    class AttachedRecorder(original_recorder):
        def rpc(self, method, params, request_id):
            result = super().rpc(method, params, request_id)
            if method == 'initialize':
                # 真实握手返回后再查附着关系，避免把 CreateProcess 返回当作用户态已就绪。
                members = api.console_members()
                require(any(member['pid'] == self.process.pid for member in members), '真实 Codex 未附着当前私有 ConPTY')
                report['native_codex_attachments'].append({'pid': self.process.pid, 'members': members})
            return result

    class RejectModel(BaseHTTPRequestHandler):
        def do_GET(self):
            report['model_http_requests'].append({'method': self.command, 'path': self.path})
            self.send_error(503)
        do_POST = do_PUT = do_DELETE = do_PATCH = do_OPTIONS = do_HEAD = do_GET
        def log_message(self, *arguments):
            pass

    try:
        report['conout_diagnostic'] = api.console_diagnostic(config['nonce'])
        native.NativeRecorder = AttachedRecorder
        environment = windows_environment(Path(config['bash']), Path(config['jq']))
        environment.update(WARP_CLI_AGENT_PROTOCOL_VERSION='1', WARP_CLIENT_VERSION='conpty-probe', TERM_PROGRAM='WarpTerminal')
        environment.update(BASH_ENV=bash_environment.as_posix(),
                           INFINISHELL_CONPTY_DIAGNOSTICS_DIR=shell_diagnostics.as_posix())
        server = ThreadingHTTPServer(('127.0.0.1', 0), RejectModel)
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        args = types.SimpleNamespace(repo=Path(config['repo']), upstream_plugin=Path(config['plugin']),
            codex_executable=Path(config['codex']), native_environment=environment, port=server.server_port)
        for name in ('插件 空 格', "插件 ' $(touch INJECTED) `touch INJECTED` & %PATH% !name! ^ ()"):
            evidence = {'name': name, 'passed': False, 'traces': []}
            report['cases'].append(evidence)
            evidence.update(native.one_case(args, Path(config['private_dir']) / name,
                                            report['model_http_requests'], evidence))
            evidence['passed'] = True
        require(not report['model_http_requests'], '出现模型请求，不能通过无模型验收')
        report['passed'] = True
    except Exception as error:
        report['failure'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        native.NativeRecorder = original_recorder
        if server is not None:
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=2)
        stop.set()
        observer.join(timeout=2)
        report['attachments'] = list(observed.values())
        report['attachment_observation_errors'] = observation_errors
        report['attachment_snapshot_is_not_complete_process_history'] = True
        try:
            report['shell_diagnostics'] = read_shell_diagnostics(shell_diagnostics)
        except Exception as error:
            report['shell_diagnostics_error'] = str(error)
        Path(config['native_report']).write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def run_conpty(dll_path, command, environment, cwd, output, timeout):
    api = WinApi()
    library = ctypes.WinDLL(str(dll_path), use_last_error=True)
    create = library.CreatePseudoConsole
    create.argtypes = [COORD, wintypes.HANDLE, wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)]
    create.restype = ctypes.c_long
    close = library.ClosePseudoConsole
    close.argtypes, close.restype = [wintypes.HANDLE], None
    show, release = library.ConptyShowHidePseudoConsole, library.ConptyReleasePseudoConsole
    show.argtypes, show.restype = [wintypes.HANDLE, ctypes.c_bool], ctypes.c_long
    release.argtypes, release.restype = [wintypes.HANDLE], ctypes.c_long
    handles, errors = [], []
    hpc, process = wintypes.HANDLE(), PROCESS_INFORMATION()
    reader = None
    attributes = None
    try:
        input_read, input_write, output_read, output_write = (wintypes.HANDLE() for _ in range(4))
        api.check(api.kernel.CreatePipe(ctypes.byref(input_read), ctypes.byref(input_write), None, 0))
        handles.extend([input_read, input_write])
        api.check(api.kernel.CreatePipe(ctypes.byref(output_read), ctypes.byref(output_write), None, 0))
        handles.extend([output_read, output_write])
        result = create(COORD(120, 40), input_read, output_write, 0, ctypes.byref(hpc))
        require(result >= 0, f'CreatePseudoConsole HRESULT={result:#x}')
        show_result = show(hpc, True)

        def read_output():
            total = 0
            try:
                with output.open('xb') as target:
                    buffer, count = ctypes.create_string_buffer(65536), wintypes.DWORD()
                    while api.kernel.ReadFile(output_read, buffer, len(buffer), ctypes.byref(count), None):
                        if not count.value:
                            break
                        total += count.value
                        require(total <= MAX_OUTPUT, 'ConPTY 输出超过固定上限')
                        target.write(buffer.raw[:count.value])
                        target.flush()
                    error = ctypes.get_last_error()
                    require(error in (0, 109, 233), f'ConPTY ReadFile 错误: {error}')
            except Exception as error:
                errors.append(str(error))

        reader = threading.Thread(target=read_output, daemon=True)
        reader.start()
        size = ctypes.c_size_t()
        api.kernel.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(size))
        storage = ctypes.create_string_buffer(size.value)
        api.check(api.kernel.InitializeProcThreadAttributeList(storage, 1, 0, ctypes.byref(size)))
        attributes = storage
        api.check(api.kernel.UpdateProcThreadAttribute(storage, 0, 0x20016, hpc, ctypes.sizeof(hpc), None, None))
        startup = STARTUPINFOEX()
        startup.info.cb = ctypes.sizeof(startup)
        startup.attributes = ctypes.addressof(storage)
        env = ctypes.create_unicode_buffer(environment_block(environment))
        argv = ctypes.create_unicode_buffer(subprocess.list2cmdline(command))
        # 与产品一样不设 STARTF_USESTDHANDLES；不允许继承父进程的无关句柄。
        flags = 0x80000 | 0x400 | 0x1000000
        api.check(api.kernel.CreateProcessW(command[0], argv, None, None, False, flags,
                    env, str(cwd), ctypes.byref(startup.info), ctypes.byref(process)))
        release_result = release(hpc)
        api.kernel.CloseHandle(process.thread)
        process.thread = None
        for handle in (input_read, output_write):
            api.kernel.CloseHandle(handle)
            handles.remove(handle)
        wait = api.kernel.WaitForSingleObject(process.process, int(timeout * 1000))
        require(wait == 0, f'附着 driver 未在 {timeout}s 内退出，WaitForSingleObject={wait}')
        code = wintypes.DWORD()
        api.check(api.kernel.GetExitCodeProcess(process.process, ctypes.byref(code)))
        return {'pid': process.pid, 'exit_code': code.value, 'show_hresult': show_result,
                'release_hresult': release_result, 'create_flags': flags}
    finally:
        if process.process:
            if api.kernel.WaitForSingleObject(process.process, 0) == 258:
                if not api.kernel.TerminateProcess(process.process, 99):
                    errors.append(f'driver TerminateProcess 失败: {ctypes.get_last_error()}')
                elif api.kernel.WaitForSingleObject(process.process, 5000) != 0:
                    errors.append('driver 终止未确认')
            api.kernel.CloseHandle(process.process)
        if process.thread:
            api.kernel.CloseHandle(process.thread)
        if attributes is not None:
            api.kernel.DeleteProcThreadAttributeList(attributes)
        if hpc.value:
            closing = threading.Thread(target=close, args=(hpc,), daemon=True)
            closing.start()
            closing.join(timeout=10)
            if closing.is_alive():
                errors.append('ClosePseudoConsole 没有在 10s 内返回')
        # 先关闭本端的多余写句柄，读线程才能看到真正 EOF。
        for handle in handles:
            if handle != output_read:
                api.kernel.CloseHandle(handle)
        if reader:
            reader.join(timeout=5)
            if reader.is_alive():
                errors.append('ConPTY 输出没有 EOF')
        if output_read in handles:
            api.kernel.CloseHandle(output_read)
        require(not errors, '; '.join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--driver-config', type=Path, help=argparse.SUPPRESS)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    for name in ('bash-executable', 'jq-executable', 'download-dir', 'output'):
        parser.add_argument('--' + name, type=Path)
    args = parser.parse_args()
    if args.driver_config:
        run_driver(args.driver_config)
        return
    require(os.name == 'nt', '必须在 Windows 原生执行；离线测试不能替代 ConPTY')
    for name in ('bash_executable', 'jq_executable', 'download_dir', 'output'):
        path = getattr(args, name)
        require(path is not None and path.is_absolute(), name + ' 必须是绝对路径')
    repo = args.repo.resolve()
    for path in (args.download_dir.resolve(), args.output.resolve(), Path(tempfile.gettempdir()).resolve()):
        require(not path.is_relative_to(repo), '禁止在源码树中写探针产物、下载或 HOME')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    require(not args.output.exists(), '输出已经存在，不能覆盖先前证据')
    report = {'passed': False, 'native_conpty_notifications_verified': False,
              'scope': 'candidate_windows_hooks_actual_conpty_transport', 'ordinary_codex_tui_verified': False,
              'product_ui_notifications_verified': False, 'full_lifecycle_verified': False,
              'model_generation_verified': False, 'credentials_provided': False}
    root = Path(tempfile.mkdtemp(prefix='infinishell-codex-conpty-'))
    report['private_dir'] = str(root)
    try:
        report['source_commit'] = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip()
        source_files = [Path(__file__).resolve(),
            *(repo / 'script/cli-agent-parity' / name for name in (
                'probe_codex_windows_hooks.py', 'codex_windows_hook_command.py',
                'codex_windows_hook_command.ps1', 'codex_windows_hook_inputs.py',
                'apply_notification_patch.py'))]
        report['source_sha256'] = {str(path.relative_to(repo)): sha256(path) for path in source_files}
        report['candidate_lf_sha256'] = require_candidate_contract()
        codex, plugin, report['fixed_inputs'] = obtain_inputs(repo, args.download_dir, 'x86_64')
        environment = windows_environment(args.bash_executable, args.jq_executable)
        host = root / 'host'
        (host / 'x64').mkdir(parents=True)
        report['conpty_assets'] = {}
        for source_name, target_name in (('conpty.dll', 'conpty.dll'), ('OpenConsole.exe', 'x64/OpenConsole.exe')):
            source = repo / 'app/assets/windows/x64' / source_name
            target = host / target_name
            shutil.copyfile(source, target)
            require(sha256(source) == sha256(target), '私有 ConPTY 资产复制摘要不符')
            report['conpty_assets'][source_name] = sha256(target)
        native_report = args.output.with_suffix('.native.json')
        raw_output = args.output.with_suffix('.pty.bin')
        require(not native_report.exists() and not raw_output.exists(), '派生证据路径已存在')
        report['native_report'] = str(native_report)
        report['raw_output'] = {'path': str(raw_output)}
        config = {'nonce': str(uuid.uuid4()), 'repo': str(repo), 'bash': str(args.bash_executable),
                  'jq': str(args.jq_executable), 'codex': str(codex), 'plugin': str(plugin),
                  'private_dir': str(root / 'cases'), 'native_report': str(native_report)}
        config_file = root / 'driver.json'
        config_file.write_text(json.dumps(config), encoding='utf-8')
        environment.update(HOME=str(root / 'home'), USERPROFILE=str(root / 'home'),
            APPDATA=str(root / 'home/AppData/Roaming'), LOCALAPPDATA=str(root / 'home/AppData/Local'))
        for key in ('HOME', 'APPDATA', 'LOCALAPPDATA'):
            Path(environment[key]).mkdir(parents=True, exist_ok=True)
        report['driver'] = run_conpty(host / 'conpty.dll', [sys.executable, '-B', str(Path(__file__).resolve()),
            '--driver-config', str(config_file)], environment, root, raw_output, 120)
        native = json.loads(native_report.read_text(encoding='utf-8'))
        report['raw_output'] = {'path': str(raw_output), 'sha256': sha256(raw_output), 'bytes': raw_output.stat().st_size}
        require(report['driver']['exit_code'] == 0 and native['passed'], '附着原生 hook 场景失败')
        raw = raw_output.read_bytes()
        report['conout_canary_observed'] = any(item.get('diagnostic_nonce') == config['nonce'] for item in notifications(raw))
        report['matched_notifications'] = verify_transport(raw, native['cases'])
        require(not native['model_http_requests'], '发生模型请求')
        require(report['candidate_lf_sha256'] == require_candidate_contract(), '候选脚本在执行中改变')
        require(report['source_sha256'] == {str(path.relative_to(repo)): sha256(path) for path in source_files},
                '探针源码在执行中改变')
        report['native_conpty_notifications_verified'] = True
        report['passed'] = True
    except Exception as error:
        report['failure'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        # 保留所有失败现场；目录内没有用户认证，后续清理由调用方处理。
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
