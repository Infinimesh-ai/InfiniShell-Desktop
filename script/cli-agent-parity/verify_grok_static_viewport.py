#!/usr/bin/env python3
"""核对单轮 V05 静态真实窗口截图与构建来源；不代替人工审图或审批验收。"""

import argparse
import hashlib
import json
import re
import struct
import zlib
from pathlib import Path


NAMES = (
    "inherit.png", "fixed-read.png", "fixed-files.png", "skill-disabled.png",
    "scroll-top.png", "scroll-middle.png", "scroll-bottom.png",
)
SIZES = {"compact": (800, 600), "normal": (1280, 800)}
PNG = b"\x89PNG\r\n\x1a\n"


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(path):
    require(path.is_file() and not path.is_symlink(), f"缺少普通文件：{path}")
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def png_dimensions(data, name):
    require(data[:8] == PNG, f"截图不是 PNG：{name}")
    offset = 8
    dimensions = None
    compressed = bytearray()
    complete = False
    while offset + 12 <= len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        require(length <= len(data) - offset - 12, f"PNG 数据截断：{name}")
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        require(zlib.crc32(kind + payload) == crc, f"PNG 校验失败：{name}")
        if kind == b"IHDR":
            require(dimensions is None and length == 13, f"PNG 头部错误：{name}")
            dimensions = struct.unpack_from(">II", payload)
        elif kind == b"IDAT":
            compressed.extend(payload)
        elif kind == b"IEND":
            complete = True
            break
        offset += 12 + length
    require(dimensions is not None and compressed and complete, f"PNG 不完整：{name}")
    try:
        require(bool(zlib.decompress(compressed)), f"PNG 像素为空：{name}")
    except zlib.error as error:
        raise ValueError(f"PNG 像素无效：{name}") from error
    return dimensions


def verify(root, binary, grok, source, platform, locale, size):
    require(re.fullmatch(r"[0-9a-f]{40}", source), "源码提交必须为完整 SHA")
    require(platform in ("linux", "windows") and locale in ("en", "zh-CN")
            and size in SIZES, "平台、语言或尺寸错误")
    require(root.is_dir() and not root.is_symlink(), "本轮证据目录不存在")
    receipts = list(root.rglob("receipt.safe.json"))
    require(len(receipts) == 1, "本轮必须恰有一份安全收据")
    receipt = json.loads(receipts[0].read_text(encoding="utf-8-sig"))
    bottom_scroll = receipt.get("bottom_scroll_px")
    require(type(bottom_scroll) in (int, float) and bottom_scroll >= 0
            and (size != "compact" or bottom_scroll > 0)
            and receipt.get("scroll_mode") == (
                "scrollable" if bottom_scroll > 0 else "fully_visible"),
            "真实滚动范围或完整可见状态不符")
    require(receipt.get("schema") == 1
            and receipt.get("test") == "test_cli_grok_static_viewport"
            and receipt.get("platform") == platform
            and receipt.get("ui_locale") == locale
            and receipt.get("requested_size") == size
            and (receipt.get("window_width"), receipt.get("window_height")) == SIZES[size]
            and receipt.get("source_commit") == source
            and receipt.get("binary_sha256") == digest(binary)
            and receipt.get("grok_sha256") == digest(grok)
            and receipt.get("native_version") == "1.0.41"
            and receipt.get("model_inputs") == 0
            and receipt.get("approval_states_exercised") is False
            and receipt.get("visual_review_required") is True
            and receipt.get("v05_complete") is False,
            "静态收据与本轮来源或部分验收边界不符")
    screenshots = receipt.get("screenshots")
    require(isinstance(screenshots, list) and len(screenshots) == len(NAMES),
            "静态截图数量错误")
    require({item.get("file") for item in screenshots} == set(NAMES), "静态截图名称错误")
    hashes = []
    for item in screenshots:
        path = receipts[0].parent / item["file"]
        require(path.parent == receipts[0].parent, "截图路径越界")
        data = path.read_bytes() if path.is_file() and not path.is_symlink() else b""
        require(len(data) >= 1024, f"截图过小：{path}")
        width, height = png_dimensions(data, path.name)
        checksum = hashlib.sha256(data).hexdigest()
        require(width >= 640 and height >= 400
                and (item.get("width"), item.get("height")) == (width, height)
                and item.get("sha256") == checksum, f"截图摘要或尺寸不符：{path}")
        hashes.append(checksum)
    require(len(set(hashes[:4])) >= 3, "策略截图缺少可区分的变化")
    if bottom_scroll > 0:
        require(len(set(hashes[4:])) == 3, "上中下滚动截图缺少可区分的变化")
    return {"platform": platform, "locale": locale, "size": size,
            "screenshots": len(hashes), "scroll_mode": receipt["scroll_mode"],
            "v05_complete": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("root", "binary", "grok"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    for name in ("source", "platform", "locale", "size"):
        parser.add_argument(f"--{name}", required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.root, args.binary, args.grok, args.source,
                            args.platform, args.locale, args.size), ensure_ascii=False))


if __name__ == "__main__":
    main()
