#!/usr/bin/env python3
"""G08 回环探针的离线字节编码回归。"""

import importlib.util
from pathlib import Path
import struct
import sys
import unittest
import zlib


sys.dont_write_bytecode = True
SOURCE = Path(__file__).with_name("probe_g08_image_loopback.py")
SPEC = importlib.util.spec_from_file_location("probe_g08_image_loopback", SOURCE)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class ProbeCodecTests(unittest.TestCase):
    def test_binary_png_is_valid_and_stable(self):
        image = PROBE.image_bytes()
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(len(image), 70)
        offset = 8
        kinds = []
        while offset < len(image):
            length, = struct.unpack(">I", image[offset:offset + 4])
            content = image[offset + 4:offset + 8 + length]
            crc, = struct.unpack(">I", image[offset + 8 + length:offset + 12 + length])
            self.assertEqual(zlib.crc32(content), crc)
            kinds.append(content[:4])
            offset += 12 + length
        self.assertEqual(offset, len(image))
        self.assertEqual(kinds, [b"IHDR", b"IDAT", b"IEND"])

    def test_scope_and_binary_fields_round_trip(self):
        image = PROBE.image_bytes()
        payload = PROBE.item(1, "含 空格") + PROBE.item(2, len(image)) + PROBE.item(3, image)
        decoded = PROBE.fields(payload)
        self.assertEqual(decoded[1], ["含 空格".encode()])
        self.assertEqual(decoded[2], [len(image)])
        self.assertEqual(decoded[3], [image])

    def test_truncated_protobuf_is_rejected(self):
        with self.assertRaises(RuntimeError):
            PROBE.fields(b"\x0a\x05ab")
        with self.assertRaises(RuntimeError):
            PROBE.fields(b"\x0a\x80")

    def test_packed_capabilities(self):
        self.assertEqual(PROBE.packed_numbers(b"\x01\x03\x04\x05\xac\x02"),
                         [1, 3, 4, 5, 300])


if __name__ == "__main__":
    unittest.main()
