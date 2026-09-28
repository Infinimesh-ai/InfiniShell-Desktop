#!/usr/bin/env python3
"""V02/V05 证据门禁的合成输入回归；不代表真实桌面验收。"""

import importlib.util
import tempfile
import unittest
import zlib
from pathlib import Path
from unittest.mock import Mock, patch


MODULE_PATH = Path(__file__).with_name("verify_v02_v05_gui.py")
SPEC = importlib.util.spec_from_file_location("verify_v02_v05_gui", MODULE_PATH)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


def chunk(kind, payload):
    return len(payload).to_bytes(4, "big") + kind + payload + zlib.crc32(kind + payload).to_bytes(4, "big")


def png(path, label):
    # 合成图片只测试格式、路径与哈希门禁，真实截图另需人工视觉复核。
    header = (640).to_bytes(4, "big") + (400).to_bytes(4, "big") + bytes((8, 0, 0, 0, 0))
    rows = (b"\0" + bytes(640)) * 400
    data = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"tEXt", label.encode() + b"\0" + b"x" * 1100)
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


class V02V05EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = {
            "schema": 1,
            "platform": "linux",
            "source_commit": "a" * 40,
            "binary_sha256": "b" * 64,
            "same_commit_binary": True,
            "operator_visual_review": True,
            "v02": {
                "keyboard_source": "physical", "paste_used": False,
                "ime_engine": "pinyin", "input_desktop": "interactive", "surfaces": {},
            },
            "v05": {"actual_harness": "grok", "native_version": "1.0.41", "runs": []},
        }
        for surface in CHECK.SURFACES:
            movie = f"v02/{surface}.mkv"
            (self.root / "v02").mkdir(exist_ok=True)
            (self.root / movie).write_bytes((surface.encode() * 20000)[:120000])
            stages = {}
            for index, stage in enumerate(CHECK.IME_STAGES):
                image = f"v02/{surface}-{stage}.png"
                png(self.root / image, image)
                stages[stage] = {"observed": True, "timestamp_seconds": index + 1, "screenshot": image}
            self.manifest["v02"]["surfaces"][surface] = {
                "continuous_video": movie, "stages": stages,
                "draft_exact": "中文第一行 ASCII\n第二行 123",
                "enter_did_not_submit": True, "submission_count_before_explicit_send": 0,
            }
        for locale in CHECK.LOCALES:
            for size in CHECK.SIZES:
                prefix = f"v05/{locale}-{size}"
                states = {}
                scroll = {}
                for state in CHECK.GROK_STATES:
                    image = f"{prefix}/{state}.png"
                    png(self.root / image, image)
                    states[state] = image
                for position in CHECK.SCROLL:
                    image = f"{prefix}/scroll-{position}.png"
                    png(self.root / image, image)
                    scroll[position] = image
                self.manifest["v05"]["runs"].append({
                    "locale": locale, "size": size,
                    "window_width": 900 if size == "compact" else 1280,
                    "window_height": 650 if size == "compact" else 800,
                    "no_truncation": True, "no_overlap": True, "focus_visible": True,
                    "states": states, "scroll": scroll,
                })

    def test_complete_receipt_is_structural_only(self):
        result = CHECK.verify(self.root, self.manifest)
        self.assertTrue(result["evidence_complete"])
        self.assertEqual(result["script_checked"], "artifact_structure_and_hashes_only")
        self.assertEqual(len(result["artifacts"]), 50)

    def test_clipboard_or_noninteractive_session_cannot_claim_ime(self):
        self.manifest["v02"]["paste_used"] = True
        with self.assertRaisesRegex(CHECK.EvidenceError, "物理键盘"):
            CHECK.verify(self.root, self.manifest)
        self.manifest["v02"]["paste_used"] = False
        self.manifest["v02"]["input_desktop"] = "service"
        with self.assertRaisesRegex(CHECK.EvidenceError, "交互桌面"):
            CHECK.verify(self.root, self.manifest)

    def test_missing_preedit_or_enter_assertion_rejected(self):
        stages = self.manifest["v02"]["surfaces"]["rich_terminal"]["stages"]
        stages.pop("preedit")
        with self.assertRaisesRegex(CHECK.EvidenceError, "阶段不完整"):
            CHECK.verify(self.root, self.manifest)
        stages["preedit"] = {"observed": True, "timestamp_seconds": 0,
                             "screenshot": "v02/rich_terminal-preedit.png"}
        self.manifest["v02"]["surfaces"]["rich_terminal"]["enter_did_not_submit"] = False
        with self.assertRaisesRegex(CHECK.EvidenceError, "Enter"):
            CHECK.verify(self.root, self.manifest)

    def test_missing_locale_policy_or_scroll_rejected(self):
        self.manifest["v05"]["runs"].pop()
        with self.assertRaisesRegex(CHECK.EvidenceError, "双语"):
            CHECK.verify(self.root, self.manifest)
        self.manifest["v05"]["runs"].append({"locale": "zh-CN", "size": "normal"})
        with self.assertRaises(CHECK.EvidenceError):
            CHECK.verify(self.root, self.manifest)

    def test_symlink_and_broken_png_rejected(self):
        state = self.manifest["v05"]["runs"][0]["states"]
        path = self.root / state["inherit"]
        path.write_bytes(b"\x89PNG\r\n\x1a\n" + b"x" * 1200)
        with self.assertRaisesRegex(CHECK.EvidenceError, "PNG"):
            CHECK.verify(self.root, self.manifest)
        path.unlink()
        path.symlink_to(self.root / state["fixed_read"])
        with self.assertRaisesRegex(CHECK.EvidenceError, "链接"):
            CHECK.verify(self.root, self.manifest)

    def test_reused_frame_rejected_even_under_another_name(self):
        states = self.manifest["v05"]["runs"][0]["states"]
        (self.root / states["inherit"]).write_bytes((self.root / states["fixed_read"]).read_bytes())
        with self.assertRaisesRegex(CHECK.EvidenceError, "内容被跨场景复用"):
            CHECK.verify(self.root, self.manifest)

    def test_source_binding_checks_checkout_and_actual_binary(self):
        binary = self.root / "infinishell"
        binary.write_bytes(b"actual test binary")
        self.manifest["binary_sha256"] = CHECK.digest(binary)
        completed = lambda output: Mock(returncode=0, stdout=output)
        with patch.object(CHECK.subprocess, "run", side_effect=[
            completed(self.manifest["source_commit"] + "\n"), completed("")
        ]):
            CHECK.verify_source(self.root, binary, self.manifest)
        with patch.object(CHECK.subprocess, "run", side_effect=[
            completed(self.manifest["source_commit"] + "\n"), completed(" M app/src/lib.rs\n")
        ]):
            with self.assertRaisesRegex(CHECK.EvidenceError, "未提交变更"):
                CHECK.verify_source(self.root, binary, self.manifest)
        self.manifest["binary_sha256"] = "0" * 64
        with patch.object(CHECK.subprocess, "run", side_effect=[
            completed(self.manifest["source_commit"] + "\n"), completed("")
        ]):
            with self.assertRaisesRegex(CHECK.EvidenceError, "二进制摘要"):
                CHECK.verify_source(self.root, binary, self.manifest)

    def test_guide_preserves_physical_input_boundary(self):
        value = CHECK.guide()
        for phrase in ("preedit", "数字选词", "空格选词", "不要粘贴", "Session 0", "Grok"):
            self.assertIn(phrase, value)

    def test_new_template_is_unverified_and_complete_as_a_checklist(self):
        value = CHECK.template("windows", "a" * 40, "b" * 64)
        self.assertFalse(value["same_commit_binary"])
        self.assertFalse(value["operator_visual_review"])
        self.assertEqual(set(value["v02"]["surfaces"]), set(CHECK.SURFACES))
        self.assertEqual(len(value["v05"]["runs"]), 4)
        with self.assertRaisesRegex(CHECK.EvidenceError, "二进制"):
            CHECK.verify(self.root, value)

    def test_unfilled_timestamp_rejected_cleanly(self):
        state = self.manifest["v02"]["surfaces"]["rich_terminal"]["stages"]["preedit"]
        state["timestamp_seconds"] = None
        with self.assertRaisesRegex(CHECK.EvidenceError, "时间戳"):
            CHECK.verify(self.root, self.manifest)


if __name__ == "__main__":
    unittest.main()
