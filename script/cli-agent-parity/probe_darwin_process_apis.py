#!/usr/bin/env python3
"""实测 Darwin 子树追踪限制与带 PID version 的信号；只操作自己创建的无模型夹具。"""

import argparse
from contextlib import closing
import ctypes
import errno
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time


class UniqueInfo(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16), ("unique_id", ctypes.c_uint64),
                ("parent_unique_id", ctypes.c_uint64), ("pid_version", ctypes.c_int32),
                ("original_parent_pid_version", ctypes.c_int32),
                ("reserved2", ctypes.c_uint64), ("reserved3", ctypes.c_uint64)]


def kqueue_probe():
    code = "import subprocess,sys,time; sys.stdin.read(1); child=subprocess.Popen(['/bin/sleep','.3']); print(child.pid,flush=True); time.sleep(.1)"
    process = subprocess.Popen([sys.executable, "-c", code], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    events = []
    result = {"root_pid": process.pid}
    try:
        with closing(select.kqueue()) as watcher:
            try:
                watcher.control([select.kevent(process.pid, filter=select.KQ_FILTER_PROC,
                                                flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                                fflags=select.KQ_NOTE_TRACK | select.KQ_NOTE_FORK | select.KQ_NOTE_EXIT)], 0, 0)
                result["note_track_registered"] = True
            except OSError as error:
                result.update(note_track_registered=False, note_track_errno=error.errno, note_track_error=str(error))
            watcher.control([select.kevent(process.pid, filter=select.KQ_FILTER_PROC,
                                            flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                            fflags=select.KQ_NOTE_FORK | select.KQ_NOTE_EXIT)], 0, 0)
            process.stdin.write(b"1")
            process.stdin.flush()
            child_pid = int(process.stdout.readline())
            result["child_pid"] = child_pid
            deadline = time.monotonic() + 1
            while time.monotonic() < deadline:
                for event in watcher.control(None, 8, .1):
                    events.append({"ident": event.ident, "filter": event.filter, "flags": event.flags, "fflags": event.fflags, "data": event.data})
            process.wait(timeout=2)
        result.update(events=events, note_fork_received=any(event["fflags"] & select.KQ_NOTE_FORK for event in events),
                      child_pid_delivered=any(event["ident"] == child_pid or event["data"] == child_pid for event in events))
        result["expected_limitation_confirmed"] = result.get("note_track_errno") == errno.ENOTSUP and result["note_fork_received"] and not result["child_pid_delivered"]
        return result
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdin.close()
        process.stdout.close()


def audit_signal_probe():
    library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    if not hasattr(library, "proc_signal_with_audittoken"):
        return {"available": False, "passed": False}
    library.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    library.proc_pidinfo.restype = ctypes.c_int
    token_type = ctypes.c_uint32 * 8
    library.proc_signal_with_audittoken.argtypes = [ctypes.POINTER(token_type), ctypes.c_int]
    library.proc_signal_with_audittoken.restype = ctypes.c_int
    process = subprocess.Popen(["/bin/sleep", "5"])
    try:
        info = UniqueInfo()
        count = library.proc_pidinfo(process.pid, 17, 0, ctypes.byref(info), ctypes.sizeof(info))
        if count != 56:
            raise RuntimeError("固定 PROC_PIDUNIQIDENTIFIERINFO 结构未返回完整数据")
        token = token_type()
        token[5], token[7] = process.pid, info.pid_version
        wrong = token_type(*token)
        wrong[7] ^= 0x40000000
        ctypes.set_errno(0)
        wrong_result = library.proc_signal_with_audittoken(ctypes.byref(wrong), signal.SIGKILL)
        wrong_errno = ctypes.get_errno()
        still_running = process.poll() is None
        ctypes.set_errno(0)
        correct_result = library.proc_signal_with_audittoken(ctypes.byref(token), signal.SIGKILL)
        correct_errno = ctypes.get_errno()
        status = process.wait(timeout=3)
        return {"available": True, "pid": process.pid, "pid_version": info.pid_version,
                "wrong_version_result": wrong_result, "wrong_version_errno": wrong_errno,
                "original_alive_after_wrong_version": still_running, "correct_version_result": correct_result,
                "correct_version_errno": correct_errno, "exit_status": status,
                "passed": wrong_result != 0 and wrong_errno == errno.ESRCH and still_running and correct_result == 0 and status == -signal.SIGKILL,
                "full_tree_tracking_proven": False}
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("只支持 Darwin 实测，其他平台不能据此计为通过")
    report = {"platform": sys.platform, "os_version": subprocess.run(["sw_vers", "-productVersion"], capture_output=True, text=True, check=True).stdout.strip(),
              "model_requests": False, "kqueue": kqueue_probe(), "audit_token_signal": audit_signal_probe(), "complete_descendant_tracking_available": False}
    report["probe_expectations_passed"] = report["kqueue"]["expected_limitation_confirmed"] and report["audit_token_signal"]["passed"]
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False))
    if not report["probe_expectations_passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
