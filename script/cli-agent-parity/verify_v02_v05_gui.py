#!/usr/bin/env python3
"""核验 V02/V05 真实桌面会话的证据完整性；不代替视觉复核。"""

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path


SHA256 = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
SURFACES = ("rich_terminal", "managed_composer")
IME_STAGES = ("preedit", "number_candidate", "space_candidate", "mixed_multiline_enter")
LOCALES = ("en", "zh-CN")
SIZES = ("compact", "normal")
GROK_STATES = (
    "inherit", "fixed_read", "fixed_files", "skill_disabled",
    "approval_pending", "approval_allowed", "approval_denied",
)
SCROLL = ("top", "middle", "bottom")


class EvidenceError(ValueError):
    pass


def require(condition, reason):
    if not condition:
        raise EvidenceError(reason)


def artifact(root, relative, kind):
    require(isinstance(relative, str) and relative, f"{kind}: 缺少相对路径")
    path = Path(relative)
    require(not path.is_absolute() and ".." not in path.parts, f"{kind}: 非私有相对路径")
    target = root / path
    require(target.is_file() and not target.is_symlink(), f"{kind}: 文件不存在或为链接")
    require(target.resolve().is_relative_to(root.resolve()), f"{kind}: 路径逃逸")
    return target


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def png_dimensions(path):
    data = path.read_bytes()
    require(data[:8] == b"\x89PNG\r\n\x1a\n", f"{path.name}: 非 PNG")
    offset = 8
    size = None
    image_data = bytearray()
    complete = False
    while offset + 12 <= len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        require(length <= len(data) - offset - 12, f"{path.name}: PNG 数据截断")
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        require(zlib.crc32(kind + payload) == crc, f"{path.name}: PNG 校验失败")
        if kind == b"IHDR":
            require(size is None and length == 13, f"{path.name}: PNG 头部错误")
            size = struct.unpack_from(">II", payload)
        elif kind == b"IDAT":
            image_data.extend(payload)
        elif kind == b"IEND":
            complete = True
            break
        offset += 12 + length
    require(size is not None and image_data and complete, f"{path.name}: PNG 数据不完整")
    try:
        require(bool(zlib.decompress(image_data)), f"{path.name}: PNG 像素为空")
    except zlib.error as error:
        raise EvidenceError(f"{path.name}: PNG 像素无效") from error
    width, height = size
    require(width >= 640 and height >= 400, f"{path.name}: 截图尺寸不足")
    return [width, height]


def screenshot(root, relative, received, label):
    path = artifact(root, relative, label)
    require(path.stat().st_size >= 1024, f"{label}: 截图过小")
    dimensions = png_dimensions(path)
    received.append({"file": relative, "sha256": digest(path), "dimensions": dimensions})


def video(root, relative, received, label):
    path = artifact(root, relative, label)
    require(path.suffix.lower() in (".mkv", ".mp4", ".webm"), f"{label}: 需原始录屏")
    require(path.stat().st_size >= 100_000, f"{label}: 录屏过小")
    received.append({"file": relative, "sha256": digest(path), "bytes": path.stat().st_size})


def verify_ime(root, value, received):
    require(isinstance(value, dict), "V02: 缺少会话")
    require(value.get("keyboard_source") == "physical" and value.get("paste_used") is False,
            "V02: 必须记录物理键盘且未粘贴")
    require(value.get("ime_engine") and value.get("ime_engine") not in ("none", "clipboard"),
            "V02: 必须记录实际输入法引擎")
    require(value.get("input_desktop") == "interactive", "V02: 需要交互桌面，服务会话不算")
    surfaces = value.get("surfaces")
    require(isinstance(surfaces, dict) and set(surfaces) == set(SURFACES), "V02: 输入面不完整")
    for surface in SURFACES:
        item = surfaces[surface]
        require(isinstance(item, dict), f"V02/{surface}: 格式错误")
        video(root, item.get("continuous_video"), received, f"V02/{surface}")
        stages = item.get("stages")
        require(isinstance(stages, dict) and set(stages) == set(IME_STAGES), f"V02/{surface}: 输入阶段不完整")
        for stage in IME_STAGES:
            state = stages[stage]
            require(isinstance(state, dict), f"V02/{surface}/{stage}: 格式错误")
            timestamp = state.get("timestamp_seconds")
            require(state.get("observed") is True and type(timestamp) in (int, float) and timestamp >= 0,
                    f"V02/{surface}/{stage}: 缺少带时间戳的实际观察")
            screenshot(root, state.get("screenshot"), received, f"V02/{surface}/{stage}")
        require(item.get("draft_exact") == "中文第一行 ASCII\n第二行 123",
                f"V02/{surface}: 最终草稿内容不符")
        require(item.get("enter_did_not_submit") is True and item.get("submission_count_before_explicit_send") == 0,
                f"V02/{surface}: Enter 误提交或未观察提交数")


def verify_grok(root, value, received):
    require(isinstance(value, dict), "V05: 缺少会话")
    require(value.get("actual_harness") == "grok" and value.get("native_version") == "1.0.41",
            "V05: 必须是固定 1.0.41 的实际 Grok 专属视口")
    runs = value.get("runs")
    require(isinstance(runs, list) and len(runs) == len(LOCALES) * len(SIZES),
            "V05: 必须分别覆盖双语与紧凑/常规窗口")
    seen = set()
    for run in runs:
        require(isinstance(run, dict), "V05: 视口记录格式错误")
        key = (run.get("locale"), run.get("size"))
        require(key[0] in LOCALES and key[1] in SIZES and key not in seen,
                "V05: 语言/窗口尺寸重复或缺失")
        seen.add(key)
        width, height = run.get("window_width"), run.get("window_height")
        require(type(width) is int and type(height) is int and width >= 640 and height >= 400,
                f"V05/{key}: 未记录实际窗口尺寸")
        require(run.get("no_truncation") is True and run.get("no_overlap") is True
                and run.get("focus_visible") is True, f"V05/{key}: 视觉检查未通过")
        states = run.get("states")
        require(isinstance(states, dict) and set(states) == set(GROK_STATES),
                f"V05/{key}: 策略与审批状态不完整")
        for state in GROK_STATES:
            screenshot(root, states[state], received, f"V05/{key}/{state}")
        scroll = run.get("scroll")
        require(isinstance(scroll, dict) and set(scroll) == set(SCROLL),
                f"V05/{key}: 未覆盖完整滚动区")
        for position in SCROLL:
            screenshot(root, scroll[position], received, f"V05/{key}/scroll/{position}")
    require(seen == {(locale, size) for locale in LOCALES for size in SIZES}, "V05: 缺少组合")


def verify(root, manifest):
    require(manifest.get("schema") == 1, "证据版本错误")
    require(manifest.get("platform") in ("linux", "windows"), "仅验 Linux/Windows")
    require(isinstance(manifest.get("source_commit"), str)
            and COMMIT.fullmatch(manifest["source_commit"]), "缺少完整源码提交")
    require(isinstance(manifest.get("binary_sha256"), str)
            and SHA256.fullmatch(manifest["binary_sha256"]), "缺少真实应用二进制摘要")
    require(manifest.get("same_commit_binary") is True, "应用二进制未绑定本次提交")
    require(manifest.get("operator_visual_review") is True, "需要人工逐帧复核原图和录屏")
    received = []
    verify_ime(root, manifest.get("v02"), received)
    verify_grok(root, manifest.get("v05"), received)
    require(len({item["file"] for item in received}) == len(received), "原图/录屏被跨场景复用")
    require(len({item["sha256"] for item in received}) == len(received), "原图/录屏内容被跨场景复用")
    return {
        "schema": 1,
        "platform": manifest["platform"],
        "source_commit": manifest["source_commit"],
        "binary_sha256": manifest["binary_sha256"],
        "evidence_complete": True,
        "script_checked": "artifact_structure_and_hashes_only",
        "visual_review_recorded": True,
        "artifacts": received,
    }


def verify_source(repo, binary, manifest):
    require(repo.is_dir() and binary.is_file() and not binary.is_symlink(), "源码或真实应用二进制不存在")
    head = subprocess.run(("git", "-C", str(repo), "rev-parse", "HEAD"),
                          capture_output=True, text=True, check=False)
    require(head.returncode == 0 and head.stdout.strip() == manifest["source_commit"],
            "当前源码提交与会话记录不一致")
    status = subprocess.run(("git", "-C", str(repo), "status", "--porcelain"),
                            capture_output=True, text=True, check=False)
    require(status.returncode == 0 and not status.stdout.strip(), "源码工作区含未提交变更")
    require(digest(binary) == manifest["binary_sha256"], "真实应用二进制摘要与会话记录不一致")


def template(platform, source_commit, binary_sha256):
    require(platform in ("linux", "windows"), "仅验 Linux/Windows")
    surfaces = {}
    for surface in SURFACES:
        surfaces[surface] = {
            "continuous_video": f"v02/{surface}.mkv",
            "stages": {
                stage: {
                    "observed": False, "timestamp_seconds": None,
                    "screenshot": f"v02/{surface}-{stage}.png",
                } for stage in IME_STAGES
            },
            "draft_exact": "", "enter_did_not_submit": False,
            "submission_count_before_explicit_send": None,
        }
    runs = []
    for locale in LOCALES:
        for size in SIZES:
            prefix = f"v05/{locale}-{size}"
            runs.append({
                "locale": locale, "size": size, "window_width": 0, "window_height": 0,
                "no_truncation": False, "no_overlap": False, "focus_visible": False,
                "states": {state: f"{prefix}/{state}.png" for state in GROK_STATES},
                "scroll": {position: f"{prefix}/scroll-{position}.png" for position in SCROLL},
            })
    return {
        "schema": 1, "platform": platform, "source_commit": source_commit,
        "binary_sha256": binary_sha256, "same_commit_binary": False,
        "operator_visual_review": False,
        "v02": {
            "keyboard_source": "", "paste_used": None, "ime_engine": "",
            "input_desktop": "", "surfaces": surfaces,
        },
        "v05": {"actual_harness": "", "native_version": "", "runs": runs},
    }


def initialize(root, repo, binary):
    require(not root.exists(), "证据目录已存在，拒绝覆盖")
    require(not root.resolve().is_relative_to(repo.resolve()), "证据目录须位于源码仓库外")
    require(repo.is_dir() and binary.is_file() and not binary.is_symlink(), "源码或应用二进制不存在")
    head = subprocess.run(("git", "-C", str(repo), "rev-parse", "HEAD"),
                          capture_output=True, text=True, check=False)
    require(head.returncode == 0 and COMMIT.fullmatch(head.stdout.strip()), "无法读取源码提交")
    manifest = template("windows" if sys.platform == "win32" else sys.platform,
                        head.stdout.strip(), digest(binary))
    verify_source(repo, binary, manifest)
    root.mkdir(parents=True)
    (root / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
                                       encoding="utf-8")
    return root / "manifest.json"


def preflight():
    """只检查会话前提，不能据此声称输入法已经组合输入。"""
    if sys.platform.startswith("linux"):
        session = os.environ.get("XDG_SESSION_TYPE", "")
        display = os.environ.get("DISPLAY", "")
        bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
        engines = []
        for command in (("ibus", "engine"), ("fcitx5-remote", "-n")):
            if shutil.which(command[0]):
                result = subprocess.run(command, capture_output=True, text=True, timeout=3, check=False)
                if result.returncode == 0 and result.stdout.strip():
                    engines.append({"provider": command[0], "engine": result.stdout.strip()})
        return {
            "platform": "linux", "interactive_x11": session == "x11" and bool(display) and bool(bus),
            "session_type": session, "display_set": bool(display), "session_bus_set": bool(bus),
            "ime_engines": engines,
            "ready_for_operator_check": session == "x11" and bool(display) and bool(bus) and bool(engines),
            "ime_composition_verified": False,
        }
    if sys.platform == "win32":
        import ctypes

        ctypes.windll.user32.OpenInputDesktop.restype = ctypes.c_void_p
        ctypes.windll.user32.CloseDesktop.argtypes = [ctypes.c_void_p]
        session_id = ctypes.c_ulong()
        current = ctypes.windll.kernel32.GetCurrentProcessId()
        session_ok = bool(ctypes.windll.kernel32.ProcessIdToSessionId(current, ctypes.byref(session_id)))
        desktop = ctypes.windll.user32.OpenInputDesktop(0, False, 0x0100)
        if desktop:
            ctypes.windll.user32.CloseDesktop(desktop)
        layout = ctypes.windll.user32.GetKeyboardLayout(0) & 0xFFFF
        chinese_layout = layout in (0x0404, 0x0804, 0x0C04, 0x1004)
        return {
            "platform": "windows", "session_id": session_id.value if session_ok else None,
            "input_desktop_opened": bool(desktop), "chinese_layout_active": chinese_layout,
            "ready_for_operator_check": session_ok and session_id.value != 0 and bool(desktop) and chinese_layout,
            "ime_composition_verified": False,
        }
    raise EvidenceError("只支持 Linux X11 与 Windows 交互桌面")


def guide():
    return """V02/V05 实际桌面验收步骤：
1. 在有物理键盘或可发送逐键事件的交互登录桌面运行 --preflight。Linux 需要 X11、会话 D-Bus 与 ibus/fcitx5 中文引擎；Windows 需要非 Session 0、可打开 input desktop 且已切到中文输入法。服务会话、仅 Xvfb、剪贴板注入均不构成 V02 证据。
2. 从同一提交构建真实应用；将完整提交 SHA 和应用二进制 SHA-256 记入 manifest.json。使用隔离测试账号/配置，登录动作完成后再开始录屏，不把凭据、令牌或私人提示写进证据。
3. 分别在普通终端富输入和托管输入中用实际拼音键序做 preedit，录到候选出现；数字选词、空格选词；输入“中文第一行 ASCII”，按产品约定的换行键插入换行，再输入“第二行 123”。逐步截原图并记连续录屏时间戳。组合期间按 Enter 应只确认输入法候选，不能误提交；显式发送前任务数为 0。原生组合事件和最终草稿都须目视核对。不要粘贴。
4. 英文和简体中文分别冷启动固定 Grok 1.0.41 专属视口，在受支持的紧凑和常规窗口尺寸验证继承、固定读取、固定文件、技能禁用、审批待决/允许/拒绝各状态；长内容滚动到顶部、中部、底部，核对截断、遮挡、焦点，保存每一状态原始截图。
5. 运行“证据目录 --init --binary 本轮应用路径 --repo 源码仓库”创建未通过的 manifest.json 清单；按清单保存原始录屏/截图并填写实际观察。随后去掉 --init 运行核验。完整证据仍需独立视觉复核；核验输出不宣称产品通过。
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence_dir", type=Path, nargs="?", help="私有会话目录，含 manifest.json 和原始媒体")
    parser.add_argument("--binary", type=Path, help="本轮实际启动的应用二进制")
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2],
                        help="源码 Git 仓库；默认当前仓库")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--preflight", action="store_true", help="检查当前交互桌面与输入法前提")
    modes.add_argument("--guide", action="store_true", help="显示真实会话执行步骤")
    modes.add_argument("--init", action="store_true", help="为本轮真实 GUI 会话生成未通过的证据清单")
    parser.add_argument("--output", type=Path, help="安全摘要输出路径；默认只输出标准输出")
    args = parser.parse_args()
    if args.guide:
        print(guide(), end="")
        return
    if args.preflight:
        print(json.dumps(preflight(), ensure_ascii=False, indent=2))
        return
    if args.evidence_dir is None:
        parser.error("核验模式必须指定证据目录")
    if args.binary is None:
        parser.error("核验模式必须指定本轮应用二进制 --binary")
    if args.init:
        try:
            path = initialize(args.evidence_dir.absolute(), args.repo.resolve(), args.binary.absolute())
        except EvidenceError as error:
            parser.exit(1, f"无法初始化证据：{error}\n")
        print(path)
        return
    root = args.evidence_dir.resolve(strict=True)
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    try:
        verify_source(args.repo.resolve(), args.binary.absolute(), manifest)
        result = verify(root, manifest)
        result["checkout_and_binary_matched"] = True
    except EvidenceError as error:
        parser.exit(1, f"证据不完整：{error}\n")
    rendered = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")


if __name__ == "__main__":
    main()
