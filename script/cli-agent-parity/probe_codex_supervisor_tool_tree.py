#!/usr/bin/env python3
"""真实 Codex 工具崩溃验收：绑定 macOS 资源域、退出事件和生产清理证明。"""

import argparse
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import queue
import select
import shlex
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time
import uuid

from probe_protocol import Recorder
from probe_supervisor_startup import receive
from probe_darwin_process_apis import UniqueInfo


CODEX_ARGUMENTS = ["app-server", "--stdio", "--disable", "shell_snapshot"]
CONTAINMENT = "macos_resource_coalition"
RECORDS = ("manifest.json", "macos-coalition.json", "macos-native.json")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def private_record(path):
    # 与生产 read_record 保持同一文件边界；拒绝部分文件和链接，不跟随替换后的目标。
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as source:
        information = os.fstat(source.fileno())
        require(stat.S_ISREG(information.st_mode) and information.st_nlink == 1
                and information.st_mode & 0o077 == 0 and information.st_size <= 65536,
                "托管证明不是私有、完整的普通文件")
        data = source.read(65537)
    require(len(data) <= 65536, "托管证明超过生产大小限制")
    return json.loads(data), hashlib.sha256(data).hexdigest()


def valid_identity(identity):
    require(isinstance(identity, dict), "缺少进程身份")
    for key in ("pid", "pid_version", "unique_id", "resource_cid"):
        require(type(identity.get(key)) is int and identity[key] > 0, "进程身份字段无效")
    require(identity["pid"] <= 0x7fffffff and identity["pid_version"] <= 0xffffffff,
            "进程身份超出原生范围")
    require(identity["unique_id"] <= 0xffffffffffffffff and identity["resource_cid"] <= 0xffffffffffffffff,
            "进程唯一身份超出原生范围")


def match_identity(current, saved, allow_exec=False):
    valid_identity(current)
    valid_identity(saved)
    keys = ("pid", "unique_id", "resource_cid") if allow_exec else ("pid", "pid_version", "unique_id", "resource_cid")
    require(all(current[key] == saved[key] for key in keys), "进程 PID、exec 版本或资源域已经改变")


def launch_records(state, generation, codex, project):
    records, hashes = {}, {}
    for name in RECORDS:
        records[name], hashes[name] = private_record(state / name)
    manifest, claim, native = (records[name] for name in RECORDS)
    require(state.name == generation and manifest.get("generation") == generation
            and type(manifest.get("version")) is int and manifest["version"] == 1 and manifest.get("launch_allowed") is True,
            "启动 manifest 代次不匹配")
    expected = [{"Unix": list(os.fsencode(value))} for value in CODEX_ARGUMENTS]
    require(manifest.get("executable") == str(codex) and manifest.get("arguments") == expected
            and manifest.get("cwd") == str(project), "manifest 不是本次固定 Codex 命令")
    require(type(claim.get("version")) is int and claim["version"] == 1 and claim.get("generation") == generation
            and claim.get("label") == "dev.infinishell.cli-agent." + generation
            and claim.get("manifest_sha256") == hashes["manifest.json"], "首次 claim 未绑定本次 manifest")
    require(str(uuid.UUID(claim.get("boot_session", ""))) == claim["boot_session"].lower(), "首次 boot 身份无效")
    require(native.get("generation") == generation and native.get("claim_sha256") == hashes["macos-coalition.json"],
            "native claim 未绑定首次资源域")
    valid_identity(claim.get("wrapper"))
    valid_identity(native.get("identity"))
    require(native["identity"]["resource_cid"] == claim["wrapper"]["resource_cid"], "native 属于其他资源域")
    return {"records": records, "sha256": hashes}


def parse_argv(data):
    # KERN_PROCARGS2：argc、可执行文件路径、对齐零字节、argc 个 argv；绝不保存后面的环境。
    require(len(data) >= 5, "内核 argv 过短")
    count = struct.unpack_from("=i", data)[0]
    require(0 < count <= 4096, "内核 argc 无效")
    end = data.find(b"\0", 4)
    require(end >= 4, "内核可执行路径不完整")
    offset = end + 1
    while offset < len(data) and data[offset] == 0:
        offset += 1
    arguments = []
    for _ in range(count):
        end = data.find(b"\0", offset)
        require(end >= offset, "内核 argv 不完整")
        arguments.append(os.fsdecode(data[offset:end]))
        offset = end + 1
    return arguments


class DarwinProcesses:
    def __init__(self):
        self.lib = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.system = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
        self.lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
        self.lib.proc_pidinfo.restype = ctypes.c_int
        self.lib.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
        self.lib.proc_pidpath.restype = ctypes.c_int
        self.lib.proc_signal_with_audittoken.argtypes = [ctypes.POINTER(ctypes.c_uint32 * 8), ctypes.c_int]
        self.lib.proc_signal_with_audittoken.restype = ctypes.c_int
        self.system.coalition_info_resource_usage.argtypes = [ctypes.c_uint64, ctypes.c_void_p, ctypes.c_size_t]
        self.system.coalition_info_resource_usage.restype = ctypes.c_int
        self.system.sysctl.argtypes = [ctypes.POINTER(ctypes.c_int), ctypes.c_uint, ctypes.c_void_p,
                                      ctypes.POINTER(ctypes.c_size_t), ctypes.c_void_p, ctypes.c_size_t]
        self.system.sysctl.restype = ctypes.c_int
        self.system.sysctlbyname.argtypes = [ctypes.c_char_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
                                            ctypes.c_void_p, ctypes.c_size_t]
        self.system.sysctlbyname.restype = ctypes.c_int
        require(ctypes.sizeof(UniqueInfo) == 56, "Darwin unique ID ABI 不匹配")

    def read(self, pid, flavor, value):
        require(self.lib.proc_pidinfo(pid, flavor, 0, ctypes.byref(value), ctypes.sizeof(value)) == ctypes.sizeof(value),
                "内核进程身份读取不完整")
        return value

    def identity(self, pid):
        first = self.read(pid, 17, UniqueInfo())
        coalition = self.read(pid, 20, (ctypes.c_uint64 * 5)())
        second = self.read(pid, 17, UniqueInfo())
        require((first.unique_id, first.pid_version) == (second.unique_id, second.pid_version), "查询期间 PID/exec 身份改变")
        value = {"pid": pid, "pid_version": first.pid_version & 0xffffffff,
                 "unique_id": first.unique_id, "resource_cid": coalition[0],
                 "parent_unique_id": first.parent_unique_id}
        valid_identity(value)
        return value

    def boot(self):
        buffer = ctypes.create_string_buffer(128)
        size = ctypes.c_size_t(len(buffer))
        require(self.system.sysctlbyname(b"kern.bootsessionuuid", buffer, ctypes.byref(size), None, 0) == 0,
                "无法读取系统启动身份")
        require(1 < size.value <= len(buffer) and buffer.raw[size.value - 1] == 0, "系统启动身份长度无效")
        value = buffer.value.decode("ascii")
        uuid.UUID(value)
        return value

    def inspect(self, pid):
        before = self.identity(pid)
        # 本机 SDK 的 PROC_PIDTBSDINFO 为 136 字节，前 12 项 uint32 含 PID/PPID/UID。
        bsd = bytes(self.read(pid, 3, (ctypes.c_ubyte * 136)()))
        header = struct.unpack_from("=12I", bsd)
        require(header[3] == pid and header[5] == os.geteuid(), "进程 PID/用户身份不匹配")
        image = ctypes.create_string_buffer(4096)
        require(self.lib.proc_pidpath(pid, image, len(image)) > 0, "无法读取真实进程 image")
        buffer = ctypes.create_string_buffer(1024 * 1024)
        size = ctypes.c_size_t(len(buffer))
        mib = (ctypes.c_int * 3)(1, 49, pid)
        require(self.system.sysctl(mib, 3, buffer, ctypes.byref(size), None, 0) == 0
                and 0 < size.value <= len(buffer), "无法完整读取真实进程 argv")
        argv = parse_argv(buffer.raw[:size.value])
        match_identity(self.identity(pid), before)
        return {**before, "ppid": header[4], "image": os.fsdecode(image.value), "argv": argv,
                "pgid": os.getpgid(pid), "sid": os.getsid(pid)}

    def kill(self, expected, boot):
        require(self.boot() == boot, "旧 boot 不得发送信号")
        match_identity(self.identity(expected["pid"]), expected)
        token = (ctypes.c_uint32 * 8)()
        token[5], token[7] = expected["pid"], expected["pid_version"]
        ctypes.set_errno(0)
        require(self.lib.proc_signal_with_audittoken(ctypes.byref(token), signal.SIGKILL) == 0,
                "audit-token SIGKILL 未成功；拒绝改用裸 PID")

    def destroyed(self, cid, boot):
        require(cid > 0 and self.boot() == boot, "不能查询零 CID 或旧 boot")
        usage = (ctypes.c_uint64 * 2)()
        ctypes.set_errno(0)
        result = self.system.coalition_info_resource_usage(cid, usage, ctypes.sizeof(usage))
        if result == 0:
            return False
        require(ctypes.get_errno() == errno.ESRCH, "资源查询错误不能代替域销毁")
        return True


class ExitWatch:
    def __init__(self, api, identities):
        self.queue = select.kqueue()
        self.identities = identities
        self.exited = set()
        try:
            for identity in identities:
                match_identity(api.identity(identity["pid"]), identity)
                self.queue.control([select.kevent(identity["pid"], filter=select.KQ_FILTER_PROC,
                                                  flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                                  fflags=select.KQ_NOTE_EXIT)], 0, 0)
                match_identity(api.identity(identity["pid"]), identity)
            self.collect()
            require(not self.exited, "注入前进程已退出，不能归因于本次强杀")
        except Exception:
            self.close()
            raise

    def collect(self):
        for event in self.queue.control(None, 64, 0):
            require(not (event.flags & select.KQ_EV_ERROR), "退出观察通道失败")
            if event.fflags & select.KQ_NOTE_EXIT:
                self.exited.add(event.ident)
        return sorted(self.exited)

    def close(self):
        self.queue.close()


def verify_proof(state, initial, generation):
    for name, expected in initial["sha256"].items():
        require(private_record(state / name)[1] == expected, "首次授权记录在运行中被替换")
    receipt, receipt_hash = private_record(state / "exit.json")
    proof, proof_hash = private_record(state / "macos-cleanup.json")
    require(type(receipt.get("version")) is int and receipt["version"] == 1 and receipt.get("generation") == generation
            and receipt.get("manifest_sha256") == initial["sha256"]["manifest.json"]
            and receipt.get("containment") == CONTAINMENT and receipt.get("cleanup_confirmed") is True,
            "退出回执没有确认本次资源域清理")
    require(type(proof.get("version")) is int and proof["version"] == 1 and proof.get("generation") == generation
            and proof.get("claim_sha256") == initial["sha256"]["macos-coalition.json"]
            and proof.get("native_sha256") == initial["sha256"]["macos-native.json"]
            and proof.get("job_removed") is True and proof.get("resource_cid_destroyed") is True,
            "生产清理 proof 缺失、部分成功或属于其他代次")
    wait = proof.get("native_wait_status")
    code = receipt.get("exit_code")
    require(code is None or type(code) is int and -0x80000000 <= code <= 0x7fffffff, "原生退出码类型无效")
    if proof.get("execution_failed") is True:
        require(wait is None and receipt.get("exit_code") is None, "失败执行不能伪造原生退出码")
    else:
        require(proof.get("execution_failed") is False and type(wait) is int
                and -0x80000000 <= wait <= 0x7fffffff
                and (os.WIFEXITED(wait) or os.WIFSIGNALED(wait)), "原生 wait 状态没有终止")
        code = os.WEXITSTATUS(wait) if os.WIFEXITED(wait) else None
        require(code == receipt.get("exit_code"), "wait 与生产回执退出码不一致")
    require(receipt.get("exit_reason") in ("native_exit", "stop_requested", "host_disconnected", "stdio_closed"),
            "退出原因不属于生产协议")
    return {"receipt": receipt, "proof": proof, "receipt_sha256": receipt_hash, "proof_sha256": proof_hash}


HEARTBEAT = '''import json,os,pathlib,subprocess,time
root=pathlib.Path(__file__).resolve().parent
child=subprocess.Popen(["/bin/sleep","60"],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
identity={"pid":os.getpid(),"ppid":os.getppid(),"pgid":os.getpgid(0),"sid":os.getsid(0),"sleep_pid":child.pid}
(root/"tool-identity.json").write_text(json.dumps(identity))
deadline=time.monotonic()+60
try:
    with (root/"heartbeat").open("a") as output:
        while time.monotonic()<deadline and not (root/"fixture-stop").exists():
            output.write(str(time.monotonic_ns())+"\\n"); output.flush(); os.fsync(output.fileno()); time.sleep(.1)
finally:
    if child.poll() is None: child.terminate()
    child.wait()
    (root/"fixture-finished").write_text("finished")
'''


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.chmod(0o600)
    temporary.replace(path)


def safe_approval(details, expected, project):
    command = details.get("command", "")
    if not isinstance(command, str):
        return False
    try:
        argv = shlex.split(command)
        if len(argv) == 3 and argv[0] in ("/bin/zsh", "/bin/bash", "/bin/sh") and argv[1] in ("-lc", "-c"):
            argv = shlex.split(argv[2])
        actions = details.get("commandActions", [])
        return (argv == expected and details.get("cwd") == str(project) and isinstance(actions, list)
                and len(actions) == 1 and isinstance(actions[0], dict)
                and isinstance(actions[0].get("command"), str) and shlex.split(actions[0]["command"]) == expected)
    except ValueError:
        return False


def host(configuration):
    options = json.loads(configuration.read_text())
    root, project = Path(options["root"]), Path(options["root"]) / "project"
    generation, token = uuid.UUID(options["generation"]), uuid.uuid4()
    state = root / "cli-agent-processes" / str(generation)
    state.mkdir(parents=True, mode=0o700)
    with socket.socket() as listener, (root / "protocol.ndjson").open("w") as evidence:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        listener.settimeout(10)
        args = CODEX_ARGUMENTS
        manifest = {"version": 1, "launch_allowed": True, "generation": str(generation), "token": str(token), "parent_control": "127.0.0.1:" + str(listener.getsockname()[1]), "executable": options["codex"], "arguments": [{"Unix": list(os.fsencode(value))} for value in args], "cwd": str(project)}
        manifest_path = state / "manifest.json"
        write(manifest_path, manifest)
        manifest_path.chmod(0o600)
        recorder = Recorder([options["supervisor"], "cli-agent-supervisor", str(manifest_path)], os.environ.copy(), project, evidence)
        write(root / "worker.json", {"pid": recorder.process.pid, "host_pid": os.getpid()})
        control, _ = listener.accept()
        with control:
            control.settimeout(10)
            if receive(control, 32) != generation.bytes + token.bytes:
                raise RuntimeError("监督握手身份不匹配")
            control.sendall(b"\x01")
            if receive(control, 1) != b"\x01":
                raise RuntimeError("监督进程没有就绪")
            response = recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_supervised_tool_probe", "version": "0.1.0"}}, 1)
            if not response or "result" not in response:
                raise RuntimeError("真实 Codex 初始化失败")
            recorder.send({"method": "initialized"})
            response = recorder.rpc("thread/start", {"cwd": str(project), "approvalPolicy": "untrusted", "sandbox": "workspace-write", "ephemeral": True}, 2)
            if not response or "result" not in response:
                raise RuntimeError("真实 Codex 新建会话失败")
            thread_id = response["result"]["thread"]["id"]
            expected = [options["python"], "heartbeat.py"]
            command = shlex.join(expected)
            prompt = "这是隔离临时项目里的进程清理测试。必须只调用 exec_command 一次，原样执行以下固定命令，不改文件、不检查其他目录、不用其他工具；获准后保持进程运行，不主动停止或重试。拒绝则结束。命令：\n" + command
            response = recorder.rpc("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]}, 3)
            if not response or "result" not in response:
                raise RuntimeError("真实 Codex 输入失败")
            write(root / "native.json", {"thread_id": thread_id, "turn_id": response["result"]["turn"]["id"]})
            turn_id = response["result"]["turn"]["id"]
            approved = 0
            requested = 0
            deadline = time.monotonic() + 100
            while time.monotonic() < deadline:
                try:
                    message = recorder.messages.get(timeout=.1)
                except queue.Empty:
                    if recorder.process.poll() is not None:
                        break
                    continue
                if isinstance(message, dict) and "id" in message and message.get("method", "").endswith("/requestApproval"):
                    requested += 1
                    details = message.get("params", {})
                    safe = (message["method"] == "item/commandExecution/requestApproval" and requested == 1
                            and details.get("threadId") == thread_id and details.get("turnId") == turn_id
                            and safe_approval(details, expected, project))
                    recorder.send({"id": message["id"], "result": {"decision": "accept" if safe else "decline"}})
                    approved += int(safe)
                    write(root / "approval.json", {"accepted": approved, "requested": requested,
                                                   "last_request_matched_exact_fixture": safe})
            # CLI 崩溃分支保留宿主，避免把随后宿主退出混入原生崩溃的清理证据。
            write(root / "worker-exited.json", {"exit_code": recorder.process.poll()})
            while time.monotonic() < deadline:
                time.sleep(.1)


def sample(path):
    data = path.read_bytes() if path.exists() else b""
    return {"bytes": len(data), "lines": data.count(b"\n"), "sha256": hashlib.sha256(data).hexdigest()}


def bind_tree(api, initial, worker, host_identity, tool, codex, supervisor, state, python):
    claim = initial["records"]["macos-coalition.json"]
    native = initial["records"]["macos-native.json"]["identity"]
    require(api.boot() == claim["boot_session"], "首次 claim 属于另一系统启动")
    wrapper = api.inspect(claim["wrapper"]["pid"])
    match_identity(wrapper, claim["wrapper"])
    cli = api.inspect(native["pid"])
    match_identity(cli, native, allow_exec=True)
    require(Path(cli["image"]).resolve() == codex and cli["argv"] == [str(codex), *CODEX_ARGUMENTS],
            "原生 PID 不是固定 Codex image/argv")
    require(cli["ppid"] == wrapper["pid"] and cli["parent_unique_id"] == wrapper["unique_id"],
            "Codex 不是已领取 wrapper 的同一原生子进程")
    require(Path(wrapper["image"]).resolve() == supervisor
            and wrapper["argv"] == [str(supervisor), "cli-agent-supervisor", str(state / "manifest.json"), "--execute"],
            "wrapper 不是本次固定生产入口")
    host = api.inspect(host_identity["pid"])
    match_identity(host, host_identity)
    owner = api.inspect(worker["pid"])
    require(owner["ppid"] == host["pid"] and owner["parent_unique_id"] == host["unique_id"]
            and Path(owner["image"]).resolve() == supervisor
            and owner["argv"] == [str(supervisor), "cli-agent-supervisor", str(state / "manifest.json")],
            "监督者不是本次宿主创建的固定 worker")
    cid = wrapper["resource_cid"]
    require(owner["resource_cid"] != cid and host["resource_cid"] != cid, "CLI 域与宿主共享")
    members = {cli["pid"]: cli, wrapper["pid"]: wrapper}
    for pid in (tool["pid"], tool["sleep_pid"]):
        child = api.inspect(pid)
        if pid == tool["pid"]:
            require(Path(child["image"]).resolve() == python and child["argv"] == [str(python), "heartbeat.py"],
                    "工具 PID 不是已授权的完整固定命令")
        else:
            require(child["image"] == "/bin/sleep" and child["argv"] == ["/bin/sleep", "60"]
                    and child["ppid"] == tool["pid"], "sleep 不是本次工具的固定子进程")
        visited = set()
        while child["pid"] != cli["pid"]:
            require(child["resource_cid"] == cid and child["pid"] not in visited and len(visited) < 24,
                    "工具不属于本次 Codex 的有界原生父链")
            visited.add(child["pid"])
            members[child["pid"]] = child
            parent = api.inspect(child["ppid"])
            require(child["parent_unique_id"] == parent["unique_id"], "工具父 PID 已复用")
            child = parent
        match_identity(child, cli)
    # 再读所有身份，拒绝构建父链期间退出或 exec 的对象；之后才能预注册退出事件。
    all_processes = [host, owner, *members.values()]
    require(len({item["pid"] for item in all_processes}) == len(all_processes), "测试对象身份重复")
    for item in all_processes:
        match_identity(api.identity(item["pid"]), item)
    return {"host": host, "supervisor": owner, "wrapper": wrapper, "codex": cli,
            "members": list(members.values()), "all": all_processes}


def classify_cleanup(report):
    receipt = report.get("receipt")
    identity = report.get("tool_identity", {})
    expected = {identity.get("pid"), identity.get("sleep_pid")}
    rows = [row for row in report.get("after_observation", []) if row["pid"] in expected]
    stopped = None not in expected and len(rows) == 2 and all(not row["present"] or row.get("state", "").startswith("Z") for row in rows)
    stopped = stopped and report.get("heartbeat_continued_after_receipt") is False
    associated = bool(receipt and report.get("manifest_digest_matches") and receipt.get("generation") == report.get("generation"))
    # 旧进程组即使声称清理成功，也不能被此资源域验收升级；保留旧字段仅供负向诊断。
    coalition = associated and receipt.get("containment") == CONTAINMENT
    if coalition:
        expected = set(report.get("required_note_exit_pids", []))
        stopped = bool(expected) and expected.issubset(report.get("note_exit_pids", []))
    confirmed = (coalition and receipt.get("cleanup_confirmed") is True
                 and report.get("production_proof_verified") is True
                 and report.get("same_boot_cid_esrch") is True
                 and report.get("exact_approval_verified") is True
                 and report.get("crash_target_verified") is True)
    stopped = stopped and report.get("heartbeat_continued_after_receipt") is False
    return {"known_tool_processes_stopped": stopped, "cleanup_failed": not (confirmed and stopped),
            "unsafe_recovery_prevented": associated and receipt.get("cleanup_confirmed") is False,
            "recovery_verification_scope": "production_proof_schema_and_live_CID; Rust confirmed_exit entrypoint tested separately",
            "passed": confirmed and stopped}


def run(args):
    report = {"platform": sys.platform, "scope": "real_codex_tool_via_supervisor_custom_host",
              "crash_target": args.crash_target, "rust_adapter_tested": False, "model_requested": False,
              "credentials_isolated": True, "supervisor_sha256": digest(args.supervisor),
              "codex_sha256": digest(args.codex), "complete_tree_mechanism": CONTAINMENT}
    api = DarwinProcesses()
    with tempfile.TemporaryDirectory(prefix="infinishell-codex-tree-") as temporary:
        root = Path(temporary).resolve()
        project, home, configuration = root / "project", root / "home", root / "codex"
        for directory in (project, home, configuration):
            directory.mkdir(mode=0o700)
        environment = {key: value for key, value in os.environ.items() if key in ("PATH", "TMPDIR", "LANG", "LC_ALL")}
        environment.update(HOME=str(home), CODEX_HOME=str(configuration))
        version = subprocess.run([str(args.codex), "--version"], env=environment, cwd=project,
                                 capture_output=True, text=True, check=True, timeout=5).stdout.strip()
        require(version == "codex-cli 0.147.0", "只接受固定 Codex 0.147.0")
        report["cli_version"] = version
        # 只复制用户明确提供的账号文件；模型探针之外不读取真实配置、插件或历史。
        credentials = configuration / "auth.json"
        with credentials.open("xb") as target:
            credentials.chmod(0o600)
            target.write(args.credential_source.read_bytes())
        (project / "heartbeat.py").write_text(HEARTBEAT)
        generation = str(uuid.uuid4())
        # Framework Python 会把 bin 启动器 exec 成 Python.app；直接固定内核确认的实际 image。
        python = Path(api.inspect(os.getpid())["image"]).resolve()
        options = root / "host-config.json"
        write(options, {"root": str(root), "generation": generation, "codex": str(args.codex),
                        "supervisor": str(args.supervisor), "python": str(python)})
        state = root / "cli-agent-processes" / generation
        identity, watcher, tree, host_identity = {}, None, None, None
        with (root / "host-stderr").open("wb") as errors:
            process = subprocess.Popen([str(python), str(Path(__file__).resolve()), "--host", str(options)],
                                       env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=errors)
            try:
                host_identity = api.identity(process.pid)
                report["model_requested"] = True
                deadline = time.monotonic() + 90
                while time.monotonic() < deadline and process.poll() is None:
                    if (project / "tool-identity.json").exists() and sample(project / "heartbeat")["lines"] >= 3:
                        break
                    time.sleep(.1)
                identity = json.loads((project / "tool-identity.json").read_text())
                worker = json.loads((root / "worker.json").read_text())
                report.update(native=json.loads((root / "native.json").read_text()), generation=generation,
                              tool_identity=identity, worker=worker)
                initial = launch_records(state, generation, args.codex, project)
                report["launch_record_sha256"] = initial["sha256"]
                report["claim"] = initial["records"]["macos-coalition.json"]
                report["native_claim"] = initial["records"]["macos-native.json"]
                boot, cid = report["claim"]["boot_session"], report["claim"]["wrapper"]["resource_cid"]
                tree = bind_tree(api, initial, worker, host_identity, identity, args.codex, args.supervisor, state, python)
                report["bound_processes"] = tree
                report["approval"] = json.loads((root / "approval.json").read_text())
                require(report["approval"] == {"accepted": 1, "requested": 1, "last_request_matched_exact_fixture": True},
                        "必须恰好一次完整固定工具审批")
                report["exact_approval_verified"] = True
                require(digest(args.codex) == report["codex_sha256"] and digest(args.supervisor) == report["supervisor_sha256"],
                        "授权后执行文件改变")
                watcher = ExitWatch(api, tree["all"])
                target = tree[args.crash_target]
                # 注册后再核对 image/argv 和全身份，期间 exec 变化不得沿用旧观察或旧信号目标。
                current = api.inspect(target["pid"])
                match_identity(current, target)
                require(current["image"] == target["image"] and current["argv"] == target["argv"], "强杀前 image/argv 改变")
                report["required_note_exit_pids"] = [item["pid"] for item in tree["all"]
                                                    if args.crash_target == "host" or item["pid"] != process.pid]
                report["note_exit_registered_before_signal"] = [item["pid"] for item in tree["all"]]
                report["heartbeat_before_kill"] = sample(project / "heartbeat")
                start = time.monotonic()
                api.kill(target, boot)
                report["crash_target_verified"] = True
                report["termination_mechanism"] = "darwin_proc_signal_with_audittoken"
                if args.crash_target == "host":
                    report["host_exit"] = process.wait(timeout=5)
                    require(report["host_exit"] == -signal.SIGKILL, "宿主未由本次 SIGKILL 退出")
                else:
                    report["host_alive_at_cli_kill"] = process.poll() is None
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline and not (state / "exit.json").exists():
                    watcher.collect()
                    time.sleep(.02)
                report["receipt_observed_after_sec"] = round(time.monotonic() - start, 3)
                report["receipt"] = private_record(state / "exit.json")[0] if (state / "exit.json").exists() else None
                report["heartbeat_at_receipt"] = sample(project / "heartbeat")
                report["host_alive_after_receipt"] = process.poll() is None
                time.sleep(1.2)
                report["heartbeat_later"] = sample(project / "heartbeat")
                report["heartbeat_continued_after_receipt"] = report["heartbeat_later"]["lines"] > report["heartbeat_at_receipt"]["lines"]
                report["note_exit_pids"] = watcher.collect()
                report["manifest_digest_matches"] = bool(report["receipt"] and report["receipt"].get("manifest_sha256") == initial["sha256"]["manifest.json"])
                report["tool_has_distinct_group_from_cli"] = tree["codex"]["pgid"] != next(item["pgid"] for item in tree["members"] if item["pid"] == identity["pid"])
                if args.crash_target == "codex":
                    require(process.poll() is None, "原生崩溃分支宿主也退出，无法独立归因")
                    match_identity(api.identity(process.pid), tree["host"])
                report.update(verify_proof(state, initial, generation))
                report["production_proof_verified"] = True
                report["same_boot_cid_esrch"] = api.destroyed(cid, boot)
                require(report["same_boot_cid_esrch"], "已知 CID 仍存在，不能靠无心跳判定清理")
                report.update(classify_cleanup(report))
            except Exception as error:
                report.update(error=str(error), passed=False)
                # 缺失、损坏或未确认的 proof 仍保留收到的原始回执，不能将错误转换成成功。
                report.update(classify_cleanup(report))
                report["passed"] = False
            finally:
                if process.poll() is None:
                    try:
                        expected = host_identity or api.identity(process.pid)
                        api.kill(expected, api.boot())
                        process.wait(timeout=5)
                    except Exception as error:
                        report["host_cleanup_error"] = str(error)
                        report["passed"] = False
                # 仅发本次固定夹具的正常停止标记；不按全局 PID/PGID 补杀未知进程。
                (project / "fixture-stop").write_text("stop")
                if watcher is not None:
                    try:
                        deadline = time.monotonic() + 5
                        expected = {identity.get("pid"), identity.get("sleep_pid")}
                        while time.monotonic() < deadline and not expected.issubset(watcher.exited):
                            watcher.collect()
                            time.sleep(.05)
                        report["fixture_cleanup_note_exit_pids"] = sorted(watcher.exited)
                    except Exception as error:
                        report["fixture_observer_cleanup_error"] = str(error)
                        report["passed"] = False
                    finally:
                        watcher.close()
        report["host_stderr"] = (root / "host-stderr").read_text(errors="replace")
        report["supervisor_unchanged_during_probe"] = digest(args.supervisor) == report["supervisor_sha256"]
        report["codex_unchanged_during_probe"] = digest(args.codex) == report["codex_sha256"]
        if (root / "approval.json").exists():
            report["final_approval"] = json.loads((root / "approval.json").read_text())
            if report["final_approval"] != {"accepted": 1, "requested": 1, "last_request_matched_exact_fixture": True}:
                report.update(passed=False, exact_approval_verified=False)
        cleaner = Recorder.__new__(Recorder)
        cleaner.directory = str(root)
        protocol, events = root / "protocol.ndjson", []
        if protocol.exists():
            for line in protocol.read_text().splitlines():
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    # 宿主 SIGKILL 可能截断最后一条诊断；保留原片段，不冒充完整协议事件。
                    report["partial_protocol_record"] = line
                    continue
                message = item.get("message")
                if isinstance(message, dict) and (message.get("method") in ("item/started", "item/completed", "item/commandExecution/requestApproval", "turn/started", "turn/completed", "error") or "result" in message and "decision" in message["result"]):
                    events.append(item)
        report["protocol_events"] = events
        report = cleaner.clean(report)
    report["passed"] = report.get("passed", False) and report["supervisor_unchanged_during_probe"] and report["codex_unchanged_during_probe"]
    write(args.output, report)
    print(json.dumps({key: report.get(key) for key in ("passed", "cleanup_failed", "unsafe_recovery_prevented", "same_boot_cid_esrch", "production_proof_verified", "error")}))
    return report["passed"]


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--host":
        host(Path(sys.argv[2]))
        return
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("codex", "supervisor", "credential-source", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--crash-target", choices=("host", "codex"), default="host")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("本次定向 probe 仅验证 macOS；其他平台须独立执行对应验收")
    require(not args.output.exists() and not args.output.is_symlink(), "拒绝覆盖已有验收证据或链接")
    for name in ("codex", "supervisor", "credential_source", "output"):
        setattr(args, name, getattr(args, name).resolve())
    require(not args.output.is_relative_to(Path(__file__).resolve().parents[2]), "真实探针输出必须位于源树外")
    try:
        passed = run(args)
    except Exception as error:
        # 准备或清理异常也要留负向记录；这里无法确认模型是否已提交，不推断为零。
        write(args.output, {"passed": False, "scope": "real_codex_tool_via_supervisor_custom_host",
                            "crash_target": args.crash_target, "error": str(error),
                            "model_requested": None, "preparation_or_finalization_failed": True})
        passed = False
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
