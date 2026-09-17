"""由可审查的固定 PowerShell 源码构造 Windows 通知候选；原生验证后才纳入产品配方。"""

import base64
import hashlib
from pathlib import Path

from codex_windows_hook_inputs import require

SOURCE = Path(__file__).with_suffix('.ps1')


def source_text():
    return SOURCE.read_text(encoding='utf-8').replace('\r\n', '\n')


def candidate_notify(original):
    # 保留入口 gate、OSC 正文和 tmux 编码；只替换最后的设备写入。
    unix_write = "(printf '%s' \"$SEQ\" > /dev/tty) 2>/dev/null || true"
    require(original.count(unix_write) == 1, '通知脚本不是受测 Unix 输出配方')
    encoded = base64.b64encode(source_text().encode('utf-16le')).decode('ascii')
    require(len(encoded) <= 16000, '通知助手超过固定命令大小上限')
    transport = r'''case "$(uname -s)" in
        MINGW*|MSYS*)
            # Git Bash 无 /dev/tty 时，仍可通过已附着的 Windows 控制台输出。
            windows_root="${SYSTEMROOT:-${SystemRoot:-}}"
            windows_root="${windows_root//\\//}"
            powershell="$windows_root/System32/WindowsPowerShell/v1.0/powershell.exe"
            if [ -n "$windows_root" ] && [ -f "$powershell" ]; then
                (printf '%s' "$SEQ" | "$powershell" -NoLogo -NoProfile -NonInteractive -EncodedCommand __ENCODED__) 2>/dev/null || true
            fi
            ;;
        *)
            (printf '%s' "$SEQ" > /dev/tty) 2>/dev/null || true
            ;;
    esac'''.replace('__ENCODED__', encoded)
    # tmux 分支也使用同一设备；封套形成后才交给目标平台。
    tmux_write = "(printf '\\033Ptmux;%s\\033\\\\' \"$SEQ\" > /dev/tty) 2>/dev/null || true"
    require(original.count(tmux_write) == 1, '通知脚本没有受测 tmux 封套')
    body = original.replace(tmux_write, "SEQ=$(printf '\\033Ptmux;%s\\033\\\\' \"$SEQ\")")
    body = body.replace('else\n    ' + unix_write + '\nfi', 'fi\n' + transport + '\n')
    require(body.count('-EncodedCommand ' + encoded) == 1 and body != original, '候选设备替换失败')
    return body


def evidence(original, candidate):
    return {'transport': 'candidate_write_console_w',
            'powershell_source_sha256_lf_utf8': hashlib.sha256(source_text().encode()).hexdigest(),
            'original_notify_sha256_lf_utf8': hashlib.sha256(original.encode()).hexdigest(),
            'candidate_notify_sha256_lf_utf8': hashlib.sha256(candidate.encode()).hexdigest(),
            'product_recipe_modified': False}
