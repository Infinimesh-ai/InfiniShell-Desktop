#!/usr/bin/env python3
"""核对 V05 合成审批真实窗口截图与来源；不宣称原生审批完成。"""

import argparse
import hashlib
import json
import re
from pathlib import Path

from verify_grok_static_viewport import SIZES, digest, png_dimensions, require


NAMES = (
    "allow-ready.png",
    "allow-pending.png",
    "deny-ready.png",
    "deny-pending.png",
)


def verify(root, binary, grok, source, platform, locale, size):
    require(re.fullmatch(r"[0-9a-f]{40}", source), "源码提交必须为完整 SHA")
    require(platform in ("linux", "windows") and locale in ("en", "zh-CN")
            and size in SIZES, "平台、语言或尺寸错误")
    require(root.is_dir() and not root.is_symlink(), "本轮证据目录不存在")
    receipts = list(root.rglob("receipt.safe.json"))
    require(len(receipts) == 1, "本轮必须恰有一份安全收据")
    receipt = json.loads(receipts[0].read_text(encoding="utf-8-sig"))
    require(receipt.get("schema") == 1
            and receipt.get("test") == "test_cli_grok_approval_viewport"
            and receipt.get("platform") == platform
            and receipt.get("ui_locale") == locale
            and receipt.get("requested_size") == size
            and (receipt.get("window_width"), receipt.get("window_height")) == SIZES[size]
            and receipt.get("source_commit") == source
            and receipt.get("binary_sha256") == digest(binary)
            and receipt.get("grok_sha256") == digest(grok)
            and receipt.get("native_version") == "1.0.41"
            and receipt.get("model_inputs") == 0
            and receipt.get("synthetic_approval_cards") == 2
            and receipt.get("app_approval_actions") == ["AllowOnce", "DenyOnce"]
            and receipt.get("native_approval_requested") is False
            and receipt.get("native_approval_resolved") is False
            and receipt.get("visual_review_required") is True
            and receipt.get("v05_complete") is False,
            "合成审批收据与来源或未完成边界不符")
    screenshots = receipt.get("screenshots")
    require(isinstance(screenshots, list) and len(screenshots) == len(NAMES),
            "合成审批截图数量错误")
    require([item.get("file") for item in screenshots] == list(NAMES),
            "合成审批截图名称或顺序错误")
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
    require(hashes[0] != hashes[1] and hashes[2] != hashes[3],
            "审批点击前后窗口截图没有可区分变化")
    return {"platform": platform, "locale": locale, "size": size,
            "screenshots": len(hashes), "synthetic_approval_exercised": True,
            "native_approval_exercised": False, "v05_complete": False}


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
