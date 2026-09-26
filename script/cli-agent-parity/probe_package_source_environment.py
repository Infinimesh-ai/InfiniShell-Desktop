#!/usr/bin/env python3
"""只读探测包管理器验收环境；标准输出为安全 JSON，不代表事务验收通过。"""

import argparse
import ctypes
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import stat
import subprocess
import sys


BREW_PREFIX = "/home/linuxbrew/.linuxbrew"
MAX_CHILD_BYTES = 16384
NAMESPACE_TIMEOUT = 10


def error_code(error):
    # 不输出异常正文，避免将路径、账号或子进程输出带入收据。
    return getattr(error, "winerror", None) or getattr(error, "errno", None)


def path_identity(path):
    try:
        value = os.lstat(path)
    except FileNotFoundError:
        return {"state": "absent"}
    except OSError as error:
        return {"state": "unreadable", "error_code": error_code(error)}
    return {
        "state": "present",
        "directory": stat.S_ISDIR(value.st_mode),
        "symlink": stat.S_ISLNK(value.st_mode),
        "uid": value.st_uid,
        "gid": value.st_gid,
        "mode": stat.S_IMODE(value.st_mode),
    }


def id_map(path):
    try:
        with open(path, "rb") as source:
            raw = source.read(4097)
        if len(raw) > 4096:
            return {"state": "invalid"}
        rows = [[int(part) for part in row.split()] for row in raw.splitlines()]
        if len(rows) > 32 or any(
            len(row) != 3 or any(value < 0 or value > 0xFFFFFFFF for value in row)
            for row in rows
        ):
            return {"state": "invalid"}
        return {"state": "read", "ranges": rows}
    except (ValueError, OSError):
        return {"state": "unreadable"}


def namespace_identity(name):
    try:
        value = os.stat(f"/proc/self/ns/{name}")
        return {"state": "read", "device": value.st_dev, "inode": value.st_ino}
    except OSError as error:
        return {"state": "unreadable", "error_code": error_code(error)}


def linux_identity():
    return {
        "uid": os.getuid(),
        "euid": os.geteuid(),
        "gid": os.getgid(),
        "egid": os.getegid(),
        "uid_map": id_map("/proc/self/uid_map"),
        "gid_map": id_map("/proc/self/gid_map"),
        "namespaces": {name: namespace_identity(name) for name in ("user", "mnt")},
        "standard_path_identity": {
            path: path_identity(path)
            for path in ("/", "/home", "/home/linuxbrew", BREW_PREFIX)
        },
    }


def landlock_abi():
    # 本批只支持 Linux x64；444 是该架构的 landlock_create_ruleset。
    if platform.machine().lower() not in ("x86_64", "amd64"):
        return {"state": "unsupported_architecture"}
    try:
        library = ctypes.CDLL(None, use_errno=True)
        syscall = library.syscall
        syscall.restype = ctypes.c_long
        ctypes.set_errno(0)
        result = syscall(ctypes.c_long(444), ctypes.c_void_p(), ctypes.c_size_t(0), ctypes.c_uint(1))
        if result < 0:
            return {"state": "unavailable", "error_code": ctypes.get_errno()}
        return {"state": "available", "abi": result, "abi3_available": result >= 3}
    except (AttributeError, OSError) as error:
        return {"state": "query_failed", "error_code": error_code(error)}


def namespace_command(unshare):
    # 不调用 mount、mkdir 或 shell；命名空间内只运行本脚本的身份读取分支。
    return [
        unshare, "--user", "--map-root-user", "--mount", "--propagation", "unchanged",
        "--fork", "--", sys.executable, "-I", "-B", str(Path(__file__).resolve()),
        "--namespace-child",
    ]


def validate_child(value, template):
    # 只允许子分支约定的数值和状态字段，拒绝额外字段或自由文本。
    if isinstance(template, dict):
        if not isinstance(value, dict) or value.keys() != template.keys():
            raise ValueError("child_schema")
        for key in template:
            validate_child(value[key], template[key])
    elif isinstance(template, list):
        if not isinstance(value, list) or len(value) > 32:
            raise ValueError("child_schema")
        for row in value:
            if not isinstance(row, list) or len(row) != 3 or any(
                type(number) is not int or not 0 <= number <= 0xFFFFFFFF for number in row
            ):
                raise ValueError("child_schema")
    elif type(template) is bool:
        if type(value) is not bool:
            raise ValueError("child_schema")
    elif type(template) is int:
        if type(value) is not int or not 0 <= value < 1 << 64:
            raise ValueError("child_schema")
    elif template is None:
        if value is not None and type(value) is not int:
            raise ValueError("child_schema")
    elif value != template:
        raise ValueError("child_schema")


def probe_namespaces(before):
    unshare = next((path for path in ("/usr/bin/unshare", "/bin/unshare")
                    if os.path.isfile(path) and os.access(path, os.X_OK)), None)
    if unshare is None:
        return {"state": "tool_absent"}
    try:
        process = subprocess.Popen(
            namespace_command(unshare), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, start_new_session=True,
            env={"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"},
        )
        try:
            raw, _ = process.communicate(timeout=NAMESPACE_TIMEOUT)
        except subprocess.TimeoutExpired:
            # 超时只清理本次独立进程组，不遗留 namespace 子进程。
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
            return {"state": "timeout"}
    except OSError as error:
        return {"state": "spawn_failed", "error_code": error_code(error)}
    if process.returncode != 0:
        return {"state": "unavailable", "exit_code": process.returncode}
    try:
        if len(raw) > MAX_CHILD_BYTES:
            raise ValueError("child_size")
        child = json.loads(raw)
        validate_child(child, before)
        distinct = all(
            child["namespaces"][name]["state"] == "read"
            and before["namespaces"][name]["state"] == "read"
            and child["namespaces"][name] != before["namespaces"][name]
            for name in ("user", "mnt")
        )
        return {"state": "created" if distinct else "identity_not_distinct", "child": child}
    except (ValueError, TypeError, KeyError):
        return {"state": "invalid_child_receipt"}


def windows_profile():
    # 从当前 token 查询 profile；只打开目录句柄，不枚举目录或读取配置内容。
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    security = ctypes.WinDLL("advapi32", use_last_error=True)
    userenv = ctypes.WinDLL("userenv", use_last_error=True)
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    security.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD,
                                        ctypes.POINTER(wintypes.HANDLE)]
    security.OpenProcessToken.restype = wintypes.BOOL
    userenv.GetUserProfileDirectoryW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR,
                                               ctypes.POINTER(wintypes.DWORD)]
    userenv.GetUserProfileDirectoryW.restype = wintypes.BOOL
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    token = wintypes.HANDLE()
    if not security.OpenProcessToken(kernel.GetCurrentProcess(), 0x0008, ctypes.byref(token)):
        return {"state": "token_unreadable", "error_code": ctypes.get_last_error()}
    try:
        length = wintypes.DWORD(32768)
        buffer = ctypes.create_unicode_buffer(length.value)
        if not userenv.GetUserProfileDirectoryW(token, buffer, ctypes.byref(length)):
            return {"state": "profile_unavailable", "error_code": ctypes.get_last_error()}
        handle = kernel.CreateFileW(buffer.value, 0x80000000, 7, None, 3, 0x02000000, None)
        if handle == ctypes.c_void_p(-1).value:
            return {"state": "profile_unreadable", "error_code": ctypes.get_last_error()}
        kernel.CloseHandle(handle)
        return {"state": "profile_readable"}
    finally:
        kernel.CloseHandle(token)


def probe_windows():
    import winreg

    # 不执行 winget，避免其日志、首次初始化或遥测写入；命令发现不等同实际执行成功。
    result = {"winget": {"command_found": shutil.which("winget.exe") is not None,
                         "execution_checked": False}}
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "", 0, winreg.KEY_READ):
            result["hkcu"] = {"state": "readable"}
    except OSError as error:
        result["hkcu"] = {"state": "unreadable", "error_code": error_code(error)}
    try:
        shell = ctypes.WinDLL("shell32", use_last_error=True)
        shell.IsUserAnAdmin.restype = ctypes.c_int
        result["effective_admin_membership"] = bool(shell.IsUserAnAdmin())
        result["profile"] = windows_profile()
    except (AttributeError, OSError) as error:
        result["platform_api"] = {"state": "unavailable", "error_code": error_code(error)}
    return result


def probe():
    system = platform.system()
    result = {
        "schema_version": 1,
        "platform": system,
        "architecture": platform.machine(),
        "transaction_execution_verified": False,
        "filesystem_mounts_performed": False,
        "registry_modified": False,
    }
    if system == "Linux":
        identity = linux_identity()
        result["linux"] = {
            "identity": identity,
            "landlock": landlock_abi(),
            "user_mount_namespace": probe_namespaces(identity),
            # rootless 映射可将宿主 root 显示为 65534；这里不判断生产祖先 UID 门禁。
            "production_uid_contract": "requires_validation",
            "bind_mount_capability": "not_tested",
        }
    elif system == "Windows":
        result["windows"] = probe_windows()
    elif system == "Darwin":
        result["macos"] = {"apple_silicon": platform.machine() == "arm64",
                           "standard_brew_prefix": path_identity("/opt/homebrew")}
    else:
        result["state"] = "unsupported_platform"
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--namespace-child", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.namespace_child:
        if platform.system() != "Linux":
            return 2
        result = linux_identity()
    else:
        result = probe()
    print(json.dumps(result, ensure_ascii=True, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
