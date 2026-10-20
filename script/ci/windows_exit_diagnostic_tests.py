"""验证诊断器拒绝错配或损坏的符号身份，避免用错误 PDB 解释现场栈。"""

import importlib.util
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import uuid


SPEC = importlib.util.spec_from_file_location(
    "exit_diagnostic", Path(__file__).with_name("run_windows_exit_diagnostic.py"),
)
DIAGNOSTIC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DIAGNOSTIC)
GUID = uuid.UUID("b381ac58-248c-4dd9-8488-f3d35696a73f")


def pe_bytes():
    data = bytearray(1536)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 132, 0x8664, 1, 0, 0, 0, 240, 0)
    struct.pack_into("<H", data, 152, 0x20B)
    struct.pack_into("<I", data, 152 + 108, 16)
    struct.pack_into("<II", data, 152 + 112 + 6 * 8, 0x1000, 28)
    data[392:400] = b".rdata\0\0"
    struct.pack_into("<IIII", data, 400, 512, 0x1000, 512, 1024)
    codeview = b"RSDS" + GUID.bytes_le + struct.pack("<I", 2) + b"warp.pdb\0"
    struct.pack_into("<IIHHIIII", data, 1024, 0, 0, 0, 0, 2, len(codeview), 0x1020, 1056)
    data[1056:1056 + len(codeview)] = codeview
    return data


def pdb_bytes(guid=GUID, age=2):
    data = bytearray(2560)
    data[:32] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0"
    struct.pack_into("<IIIIII", data, 32, 512, 1, 5, 16, 0, 4)
    struct.pack_into("<I", data, 2048, 2)
    struct.pack_into("<IIII", data, 1024, 2, 0, 28, 3)
    struct.pack_into("<III", data, 1536, 20000404, 0, age)
    data[1548:1564] = guid.bytes_le
    return data


class BuildEnvironmentTests(unittest.TestCase):
    def test_absent_flag_overrides_preserve_config_flags(self):
        with patch.dict(os.environ, {"RUST_BACKTRACE": "1"}, clear=True):
            self.assertEqual(DIAGNOSTIC.environment_contract()["unexpected_nonempty_overrides"], [])

    def test_present_empty_rust_flags_are_reported_as_overrides(self):
        for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
            with self.subTest(key=key):
                with patch.dict(os.environ, {"RUST_BACKTRACE": "1", key: ""}, clear=True):
                    self.assertEqual(DIAGNOSTIC.environment_contract()["unexpected_nonempty_overrides"], [key])


class SymbolIdentityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def write(self, name, contents):
        path = self.root / name
        path.write_bytes(contents)
        return path

    def test_matching_pe_and_pdb_identity_is_accepted(self):
        pe = DIAGNOSTIC.read_pe_codeview(self.write("warp.exe", pe_bytes()))
        pdb = DIAGNOSTIC.read_pdb_identity(self.write("warp.pdb", pdb_bytes()))
        self.assertTrue(DIAGNOSTIC.validate_pdb_match(pe, pdb))
        self.assertEqual(pe["age"], 2)
        self.assertEqual(pe["pdb_name"], "warp.pdb")

    def test_same_filename_with_wrong_guid_or_age_is_rejected(self):
        pe = DIAGNOSTIC.read_pe_codeview(self.write("warp.exe", pe_bytes()))
        for contents in (pdb_bytes(uuid.UUID(int=1)), pdb_bytes(age=3)):
            with self.subTest(contents=contents[1540:1564].hex()):
                pdb = DIAGNOSTIC.read_pdb_identity(self.write("warp.pdb", contents))
                with self.assertRaises(DIAGNOSTIC.DiagnosticError):
                    DIAGNOSTIC.validate_pdb_match(pe, pdb)

    def test_truncated_symbol_records_are_rejected(self):
        for reader, name, contents in (
            (DIAGNOSTIC.read_pe_codeview, "warp.exe", pe_bytes()[:1060]),
            (DIAGNOSTIC.read_pdb_identity, "warp.pdb", pdb_bytes()[:1549]),
        ):
            with self.subTest(name=name):
                with self.assertRaises(DIAGNOSTIC.DiagnosticError):
                    reader(self.write(name, contents))

    def test_pdb_directory_cannot_claim_unbounded_pages(self):
        data = pdb_bytes()
        struct.pack_into("<I", data, 44, 0xFFFFFFF0)
        with self.assertRaises(DIAGNOSTIC.DiagnosticError):
            DIAGNOSTIC.read_pdb_identity(self.write("warp.pdb", data))


class ObservationDecisionTests(unittest.TestCase):
    def successful_summary(self):
        return {
            "mode": "observe", "child_pid": 421, "child_exit_code": 0,
            "observer_exit_code": 0, "stdout_eof": True, "stderr_eof": True,
            "eof_deadline_recorded": True,
            "libtest_success_marker": True, "libtest_test_ok": True,
            "libtest_summary_one_passed": True, "sample_count": 0,
            "resumed_threads": 0, "sampler_failures": 0, "observer_error": None,
            "timed_out": False, "eof_timeout": False, "stop_reason": None,
            "job_terminated": False, "job_empty": True, "active_processes_final": 0,
        }

    def test_zero_exit_requires_both_output_streams_to_close(self):
        for stream in ("stdout_eof", "stderr_eof"):
            summary = self.successful_summary()
            summary[stream] = False
            with self.subTest(stream=stream):
                decision = DIAGNOSTIC.validate_observer_summary(summary, "observe", 0, False)
                self.assertFalse(decision["continue"])

    def test_stop_or_sampling_cannot_be_erased_by_later_zero_exit(self):
        summary = self.successful_summary()
        self.assertFalse(DIAGNOSTIC.validate_observer_summary(summary, "observe", 0, True)["continue"])
        summary["sample_count"] = 2
        self.assertFalse(DIAGNOSTIC.validate_observer_summary(summary, "observe", 0, False)["continue"])

    def test_nonzero_child_cannot_be_reported_as_success(self):
        summary = self.successful_summary()
        summary.update(child_exit_code=7, observer_exit_code=7)
        self.assertFalse(DIAGNOSTIC.validate_observer_summary(summary, "observe", 7, False)["continue"])

    def test_unknown_or_nonempty_job_cannot_continue_experiment(self):
        summary = self.successful_summary()
        summary.update(job_empty=False, active_processes_final=1)
        self.assertFalse(DIAGNOSTIC.validate_observer_summary(summary, "observe", 0, False)["continue"])

    def test_nonzero_protocol_validates_real_exit_without_success_markers(self):
        summary = self.successful_summary()
        summary.update(mode="protocol-nonzero", child_exit_code=7, observer_exit_code=7,
                       stop_reason="child_nonzero", libtest_success_marker=False,
                       libtest_test_ok=False, libtest_summary_one_passed=False)
        self.assertTrue(DIAGNOSTIC.validate_observer_summary(summary, "protocol-nonzero", 7, True)["continue"])


if __name__ == "__main__":
    unittest.main()
