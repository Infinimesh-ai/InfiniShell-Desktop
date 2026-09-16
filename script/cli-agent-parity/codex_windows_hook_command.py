"""从固定明文生成 commandWindows，并分别验证编码、PS 5.1、原生 argv 和 cmd 边界。"""

import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from codex_windows_hook_inputs import require


SOURCE = Path(__file__).with_suffix(".ps1")
SCRIPTS = ("on-session-start.sh", "on-prompt-submit.sh", "on-stop.sh",
           "on-permission-request.sh", "on-post-tool-use.sh")
PREFIX = "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand "


def source_text():
    # 仓库将 ps1 检出为 CRLF；统一 LF 后编码，保证各平台派生出完全相同的命令。
    return SOURCE.read_text(encoding="utf-8").replace("\r\n", "\n")


def encode_source(plain):
    encoded = base64.b64encode(plain.encode("utf-16le")).decode("ascii")
    require(base64.b64decode(encoded, validate=True).decode("utf-16le") == plain, "固定源码编码往返失败")
    return PREFIX + encoded


def encode(script):
    require(script in SCRIPTS, "未知固定通知入口")
    command = encode_source("$NotificationHook = '" + script + "'\n" + source_text())
    require(len(command) <= 8000, "固定启动器超过保守的 cmd 命令长度上限")
    return command


def cmd_line(command, executable):
    # 与 Codex 0.147 的 /C + raw_arg 外层引号一致；变量路径只在可执行文件 argv 位置。
    require('"' not in str(executable) and '"' not in command, "固定命令含意外双引号")
    result = f'"{executable}" /C "{command}"'
    require(len(result.encode("utf-16le")) // 2 <= 8191, "cmd.exe 命令行超过 8191 个 UTF-16 单元")
    return result


def verify_encoding():
    commands = {script: encode(script) for script in SCRIPTS}
    for script, command in commands.items():
        expected = "$NotificationHook = '" + script + "'\n" + source_text()
        require(base64.b64decode(command[len(PREFIX):], validate=True).decode("utf-16le") == expected,
                "生成结果无法从仓库明文逐字重建")
    return {"source_sha256_lf_utf8": hashlib.sha256(source_text().encode("utf-8")).hexdigest(),
            "command_characters": {name: len(command) for name, command in commands.items()},
            "encoding_roundtrip": True}


def windows_environment(bash, jq):
    require(os.name == "nt", "原生检查必须在 Windows 执行，不允许跳过当作通过")
    import winreg
    # Codex 默认 cmd /C 会执行 AutoRun；受控 runner 存在该配置时拒绝执行，不改全局配置。
    for hive in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        try:
            with winreg.OpenKey(hive, r"Software\Microsoft\Command Processor") as key:
                value, _ = winreg.QueryValueEx(key, "AutoRun")
                require(not value, "runner 的 cmd AutoRun 非空，不能证明隔离执行")
        except FileNotFoundError:
            pass
    require(bash.name.lower() == "bash.exe" and (bash.parent / "msys-2.0.dll").is_file(),
            "必须提供 Git usr/bin/bash.exe；不能使用 WSL shim")
    require(jq.name.lower() == "jq.exe" and jq.is_file(), "必须提供实际 jq.exe")
    env = {key: os.environ[key] for key in ("SYSTEMROOT", "WINDIR", "TEMP", "TMP") if key in os.environ}
    system = Path(env["SYSTEMROOT"]) / "System32"
    env["PATH"] = os.pathsep.join(map(str, (bash.parent, jq.parent, system, system / "WindowsPowerShell/v1.0")))
    env["COMSPEC"] = str(system / "cmd.exe")
    for executable in (bash, jq):
        subprocess.run([str(executable), "--version"], env=env, capture_output=True, check=True, timeout=10)
    return env


def run_powershell(plain, env):
    command = encode_source(plain)
    return subprocess.run(["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand",
                           command[len(PREFIX):]], env=env, capture_output=True, check=True, timeout=20)


def verify_windows_syntax(env):
    parser = r'''
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) {
    throw 'Windows PowerShell 5.1 is required'
}
$text = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:PROBE_SOURCE))
$tokens = $null
$errors = $null
$null = [Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw ($errors | Out-String) }
'''
    for script in SCRIPTS:
        plain = base64.b64decode(encode(script)[len(PREFIX):]).decode("utf-16le")
        run_powershell(parser, {**env, "PROBE_SOURCE": base64.b64encode(plain.encode("utf-8")).decode("ascii")})


def verify_windows_argv(env):
    values = ["", "中文 English", "a'b", "a&b%PATH%!NAME!^()$()`", 'a"b',
              "trailing\\", 'two\\\\"quoted', "C:\\插件 空格\\%PATH%!a!&b\\hook.sh"]
    with tempfile.TemporaryDirectory(prefix="infinishell-native-argv-") as tmp:
        root = Path(tmp)
        capture, target = root / "capture.py", root / "arguments.json"
        capture.write_text("import json,sys\nfrom pathlib import Path\n"
                           "Path(sys.argv[1]).write_text(json.dumps(sys.argv[2:]), encoding='utf-8')\n", encoding="utf-8")
        (root / "input.json").write_text(json.dumps(values), encoding="utf-8")
        plain = source_text().split("# BEGIN_NOTIFICATION_LAUNCH")[0] + r'''
# PS 5.1 把 JSON 数组作为单个管道对象；额外 @() 会制造嵌套数组并合并 argv。
$values = (Get-Content -LiteralPath $env:PROBE_VALUES -Raw | ConvertFrom-Json)
if ($values.Count -ne 8 -or $values[0] -isnot [string]) { throw 'Unexpected argv fixture shape' }
$arguments = @($env:PROBE_CAPTURE, $env:PROBE_OUTPUT) + $values
$info = New-Object System.Diagnostics.ProcessStartInfo
$info.FileName = $env:PROBE_PYTHON
$info.UseShellExecute = $false
$info.Arguments = (($arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' ')
$process = [Diagnostics.Process]::Start($info)
$process.WaitForExit()
exit $process.ExitCode
'''
        run_powershell(plain, {**env, "PROBE_PYTHON": sys.executable, "PROBE_CAPTURE": str(capture),
                              "PROBE_OUTPUT": str(target), "PROBE_VALUES": str(root / "input.json")})
        actual = json.loads(target.read_text(encoding="utf-8"))
        # 这里只记录本测试的固定参数；失败仍需保留原生收到的值，不能仅输出不匹配名称。
        require(actual == values, "原生 Windows argv 往返不一致: " +
                json.dumps({"expected": values, "actual": actual}, ensure_ascii=True))


def require_original_bytes(completed, captured, payload, case):
    expected_stdout = b"native-bytes-ok"
    if completed.stdout == expected_stdout and captured == payload:
        return
    # 这里只保存探针固定输入和占位脚本输出，不采集用户或真实模型内容。
    encoded = lambda value: None if value is None else base64.b64encode(value).decode("ascii")
    evidence = {"case": case, "expected_stdin_base64": encoded(payload),
                "actual_stdin_base64": encoded(captured), "expected_stdout_base64": encoded(expected_stdout),
                "actual_stdout_base64": encoded(completed.stdout), "stderr_base64": encoded(completed.stderr)}
    raise ValueError("cmd/PowerShell/Bash 原始 stdin 或 stdout 字节发生改变: " + json.dumps(evidence, ensure_ascii=True))


def verify_windows_bytes_and_boundary(env):
    payload = (json.dumps({"prompt": "中文\nEnglish ' $() ` % ! & ^", "extra": "反斜杠\\和空格"},
                          ensure_ascii=False) + "\r\n").encode("utf-8")
    with tempfile.TemporaryDirectory(prefix="infinishell-hook-bytes-") as tmp:
        root = Path(tmp) / "插件 ' $(touch INJECTED) `touch INJECTED` & %PATH% !name! ^ ()"
        (root / "scripts").mkdir(parents=True)
        target = root / "input.bin"
        for script in SCRIPTS:
            (root / "scripts" / script).write_text(
                '#!/bin/bash\nset -e\ncat > "$INFINISHELL_HOOK_BYTES"\nprintf "native-bytes-ok"\n',
                encoding="utf-8", newline="\n")
        local_env = {**env, "PLUGIN_ROOT": str(root), "INFINISHELL_HOOK_BYTES": str(target)}
        maximum = max((encode(script) for script in SCRIPTS), key=len)
        # 真正执行 8191 单元的边界命令，而不只检查 Python 字符串长度。
        padding = 8191 - len(cmd_line(maximum, env["COMSPEC"]).encode("utf-16le")) // 2
        boundary = maximum + " " * padding
        for case, command in [*((script, encode(script)) for script in SCRIPTS), ("cmd_8191_boundary", boundary)]:
            target.unlink(missing_ok=True)
            completed = subprocess.run(cmd_line(command, env["COMSPEC"]), executable=env["COMSPEC"],
                                       env=local_env, cwd=root, input=payload, capture_output=True,
                                       check=True, timeout=20)
            captured = target.read_bytes() if target.exists() else None
            require_original_bytes(completed, captured, payload, case)
            require(not (root / "INJECTED").exists(), "路径数据被作为 shell 源码执行")
        try:
            cmd_line(boundary + " ", env["COMSPEC"])
        except ValueError:
            pass
        else:
            raise ValueError("超过 cmd 边界的命令未被拒绝")
    return {"raw_stdin_stdout_roundtrip": True, "cmd_8191_executed": True,
            "cmd_8192_rejected_before_execution": True}
