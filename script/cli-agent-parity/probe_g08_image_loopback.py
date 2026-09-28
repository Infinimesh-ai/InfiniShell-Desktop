#!/usr/bin/env python3
"""在隔离的 macOS 回环 SSH 上验收产品远端图片暂存、恢复与释放。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import time
import uuid
import zlib


def require(value, reason):
    if not value:
        raise RuntimeError(reason)


def varint(value):
    require(value >= 0, "负数 protobuf varint")
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def item(number, value):
    if isinstance(value, int):
        return varint(number << 3) + varint(value)
    if isinstance(value, str):
        value = value.encode()
    return varint(number << 3 | 2) + varint(len(value)) + value


def fields(data):
    result = {}
    offset = 0

    def number():
        nonlocal offset
        value = shift = 0
        while offset < len(data):
            byte = data[offset]
            offset += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
            require(shift < 70, "protobuf varint 过长")
        raise RuntimeError("protobuf varint 截断")

    while offset < len(data):
        tag = number()
        kind = tag & 7
        key = tag >> 3
        require(key != 0, "protobuf 字段编号为零")
        if kind == 0:
            value = number()
        elif kind == 2:
            length = number()
            require(offset + length <= len(data), "protobuf 字段截断")
            value = data[offset:offset + length]
            offset += length
        else:
            raise RuntimeError("未知 protobuf wire type")
        result.setdefault(key, []).append(value)
    return result


def one(message, number):
    values = fields(message).get(number, [])
    require(len(values) == 1, f"protobuf 字段 {number} 数量不符")
    return values[0]


def packed_numbers(data):
    result = []
    offset = 0
    while offset < len(data):
        value = shift = 0
        while True:
            require(offset < len(data) and shift < 70, "packed protobuf varint 截断")
            byte = data[offset]
            offset += 1
            value |= (byte & 127) << shift
            if byte < 128:
                break
            shift += 7
        result.append(value)
    return result


def image_bytes():
    def chunk(kind, body):
        body = kind + body
        return struct.pack(">I", len(body) - 4) + body + struct.pack(">I", zlib.crc32(body))

    raw = b"\0" + b"\xff\0\x00\xff"
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def remote_mode(config_path):
    config = json.loads(Path(config_path).read_text())
    command = os.environ.get("SSH_ORIGINAL_COMMAND")
    os.environ.clear()
    os.environ.update(HOME=config["home"], PATH="/usr/bin:/bin:/opt/homebrew/bin",
                      TMPDIR=config["tmpdir"], LANG="en_US.UTF-8")
    if command == "proxy":
        os.execve(config["product"], [config["product"], "remote-server-proxy",
                                       "--identity-key", config["identity"]], os.environ)
    if command == "read":
        marker = Path(config["published_marker"])
        path = Path(marker.read_text())
        require(path.is_relative_to(Path(config["home"])), "已发布路径越出私有 HOME")
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        try:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid(), "原图身份不符")
            while data := os.read(fd, 65536):
                os.write(1, data)
        finally:
            os.close(fd)
        return
    raise RuntimeError("拒绝未知 SSH 命令")


class Connection:
    def __init__(self, ssh, archive, label):
        self.requests = bytearray()
        self.responses = bytearray()
        self.stderr_path = archive / f"{label}.ssh-stderr.txt"
        self.stderr = self.stderr_path.open("wb")
        self.process = subprocess.Popen([*ssh, "proxy"], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.stderr)
        self.buffer = bytearray()
        self.sequence = 0

    def read_exact(self, length):
        deadline = time.monotonic() + 20
        while len(self.buffer) < length:
            require(time.monotonic() < deadline, "SSH 响应超时")
            ready, _, _ = select.select([self.process.stdout], [], [], .25)
            if ready:
                piece = os.read(self.process.stdout.fileno(), 65536)
                require(piece, "SSH 响应提前结束")
                self.buffer.extend(piece)
        output = bytes(self.buffer[:length])
        del self.buffer[:length]
        return output

    def receive(self):
        header = self.read_exact(4)
        size, = struct.unpack("<I", header)
        require(size <= 64 * 1024 * 1024, "SSH 响应超出协议上限")
        payload = self.read_exact(size)
        self.responses.extend(header + payload)
        return fields(payload)

    def send(self, wrapper, field, payload, response=True):
        self.sequence += 1
        request_id = f"g08-{self.sequence}"
        message = item(1, request_id) + item(wrapper, item(field, payload))
        frame = struct.pack("<I", len(message)) + message
        self.requests.extend(frame)
        self.process.stdin.write(frame)
        self.process.stdin.flush()
        if not response:
            return None
        for _ in range(10):
            received = self.receive()
            if received.get(1, [b""])[0] == request_id.encode():
                return received
        raise RuntimeError("未收到对应请求的响应")

    def close(self, archive, label):
        self.process.stdin.close()
        self.process.wait(timeout=10)
        self.stderr.close()
        (archive / f"{label}.client-frames.bin").write_bytes(self.requests)
        (archive / f"{label}.server-frames.bin").write_bytes(self.responses)
        return self.process.returncode


def scope(host, epoch, native, generation=None, submission=None):
    return (item(1, host) + item(2, 7) + item(3, native)
            + item(4, generation or str(uuid.uuid4()))
            + item(5, submission or str(uuid.uuid4())) + item(6, epoch))


def request(connection, identity, revision, action, payload):
    body = item(1, identity) + item(2, revision) + item(action, payload)
    response = connection.send(3, 7, body)
    require(44 in response, "缺少图片暂存响应")
    return fields(response[44][0])


def result(response, field):
    require(field in response, f"图片暂存结果错误：{list(response)}")
    return response[field][0]


def initialize(connection, epoch):
    response = connection.send(3, 1, b"")
    require(2 in response, "初始化响应缺失")
    init = fields(response[2][0])
    capabilities = set()
    for value in init.get(3, []):
        capabilities.update(packed_numbers(value) if isinstance(value, bytes) else [value])
    require({3, 4, 5}.issubset(capabilities), "远端图片能力未启用")
    connection.send(4, 4, item(1, 7) + item(2, "bash") + item(3, "/bin/bash")
                    + item(4, epoch) + item(5, 1), response=False)
    return one(response[2][0], 2).decode()


def start_sshd(root, config):
    for name in ("host-key", "client-key"):
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name)],
                       check=True, capture_output=True)
    authorized = root / "authorized_keys"
    authorized.write_text("restrict,pty " + (root / "client-key.pub").read_text())
    authorized.chmod(0o600)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    known = root / "known_hosts"
    known.write_text(f"[127.0.0.1]:{port} " + (root / "host-key.pub").read_text())
    sshd = root / "sshd_config"
    sshd.write_text("\n".join([
        "ListenAddress 127.0.0.1", f"Port {port}", f"HostKey {root}/host-key",
        f"PidFile {root}/sshd.pid", f"AuthorizedKeysFile {authorized}",
        "StrictModes yes", "PasswordAuthentication no", "KbdInteractiveAuthentication no",
        "UsePAM no", "PermitRootLogin no", "AllowTcpForwarding no", "X11Forwarding no",
        "PermitTunnel no", "PrintMotd no", "LogLevel ERROR",
        f"ForceCommand /usr/bin/python3 {Path(__file__).resolve()} --remote {config}", "",
    ]))
    subprocess.run(["/usr/sbin/sshd", "-t", "-f", str(sshd)], check=True, capture_output=True)
    with (root / "sshd.stderr").open("wb") as errors:
        server = subprocess.Popen(["/usr/sbin/sshd", "-D", "-e", "-f", str(sshd)],
                                  stdout=subprocess.DEVNULL, stderr=errors)
    deadline = time.monotonic() + 5
    while True:
        require(server.poll() is None and time.monotonic() < deadline, "隔离 sshd 未就绪")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=.1):
                break
        except OSError:
            time.sleep(.03)
    ssh = ["ssh", "-F", "/dev/null", "-T", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes",
           "-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=" + str(known),
           "-o", "ConnectTimeout=3", "-i", str(root / "client-key"), "-p", str(port),
           subprocess.check_output(["id", "-un"], text=True).strip() + "@127.0.0.1"]
    return server, ssh, port


def private_root():
    root = Path("/Users/zhishi/InfiniShell-Tests")
    value = Path(os.environ["TMPDIR"])
    require(value.parent == root and value.name.startswith("r-"), "TMPDIR 不是本轮私有目录")
    for path in (Path("/Users"), Path("/Users/zhishi"), root, value):
        info = path.lstat()
        require(stat.S_ISDIR(info.st_mode) and not path.is_symlink()
                and info.st_dev == Path("/Users").stat().st_dev
                and not info.st_mode & 0o022, "测试目录祖先身份不安全")
    info = value.stat()
    require(not os.path.ismount(value) and info.st_uid == os.getuid()
            and stat.S_IMODE(info.st_mode) == 0o700,
            "TMPDIR 所有者或权限错误")
    return value


def cleanup_daemon(home, product, identity):
    pids = list((home / ".infinishell").rglob("server*.pid"))
    require(len(pids) <= 1, "私有 HOME 中存在多个 daemon PID")
    if not pids:
        return {"pid_file_count": 0, "daemon_gone": True}
    pid = int(pids[0].read_text())
    command = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "command="],
                             capture_output=True, text=True)
    require(command.returncode == 0 and product in command.stdout
            and "remote-server-daemon" in command.stdout and identity in command.stdout,
            "拒绝停止身份不符的 daemon")
    os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if subprocess.run(["/bin/ps", "-p", str(pid)], capture_output=True).returncode != 0:
            return {"pid": pid, "pid_file_count": 1, "daemon_gone": True}
        time.sleep(.05)
    return {"pid": pid, "pid_file_count": 1, "daemon_gone": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--product", type=Path)
    parser.add_argument("--expected-product-sha256")
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--remote", type=Path)
    args = parser.parse_args()
    if args.remote:
        remote_mode(args.remote)
        return
    require(args.product and args.archive and args.expected_product_sha256,
            "需要指定产品二进制、摘要与归档目录")
    os.umask(0o077)
    root = private_root()
    product = str(args.product.resolve(strict=True))
    require(args.archive.is_dir() and not args.archive.is_symlink()
            and stat.S_IMODE(args.archive.stat().st_mode) == 0o700
            and not any(args.archive.iterdir()), "归档目录必须为私有空目录")
    archive = args.archive.resolve(strict=True)
    home = root / "h"
    home.mkdir(mode=0o700)
    identity = "g08-" + uuid.uuid4().hex[:8]
    config = root / "remote.json"
    marker = root / "published-path.txt"
    config.write_text(json.dumps({"home": str(home), "tmpdir": str(root), "product": product,
                                  "identity": identity, "published_marker": str(marker)}))
    product_sha = hashlib.sha256(Path(product).read_bytes()).hexdigest()
    require(product_sha == args.expected_product_sha256, "固定产品二进制摘要不符")
    receipt = {"target": "macos_arm64_loopback_ssh_product_image_staging",
               "product_sha256": product_sha, "source_commit": subprocess.check_output(
                   ["git", "rev-parse", "HEAD"], text=True).strip(),
               "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"])),
               "auth_provided": False, "product_gui_verified": False,
               "native_cli_consumption_verified": False, "passed": False}
    server = None
    connections = []
    try:
        server, ssh, port = start_sshd(root, config)
        receipt["ssh"] = {"loopback": True, "isolated_identity": True, "port": port,
                          "production_config_read": False, "binary_stdio_no_pty": True}
        original = image_bytes()
        (archive / "source.png").write_bytes(original)
        receipt["source"] = {"bytes": len(original), "sha256": hashlib.sha256(original).hexdigest()}
        first = Connection(ssh, archive, "first")
        connections.append((first, "first"))
        epoch = str(uuid.uuid4())
        host = initialize(first, epoch)
        receipt["host_id"] = host
        native = "g08-isolated-native"
        current = scope(host, epoch, native)
        activated = request(first, current, 0, 3, b"")
        require(3 in activated and activated.get(2) == [1], "图片作用域激活失败")
        transfer = str(uuid.uuid4())
        key = str(uuid.uuid4())
        spec = item(1, len(original)) + item(2, hashlib.sha256(original).digest())
        result(request(first, current, 1, 4, item(1, transfer) + item(2, spec)), 4)
        mid = len(original) // 2
        for offset, data in ((0, original[:mid]), (mid, original[mid:])):
            progress = fields(result(request(first, current, 1, 5,
                                            item(1, transfer) + item(2, offset) + item(3, data)), 4))
            require(progress.get(2) == [offset + len(data)], "图片分块偏移不符")
        verified = fields(result(request(first, current, 1, 6, item(1, transfer) + item(2, spec)), 5))
        require(one(verified[2][0], 2) == hashlib.sha256(original).digest(), "远端 SHA 不符")
        published = fields(result(request(first, current, 1, 9,
                                          item(1, transfer) + item(2, key)), 9))
        remote_path = published[2][0].decode()
        marker.write_text(remote_path)
        read = subprocess.run([*ssh, "read"], capture_output=True, timeout=15)
        require(read.returncode == 0 and read.stdout == original, "SSH 远端原始 PNG 字节不一致")
        (archive / "remote-read.png").write_bytes(read.stdout)
        receipt["uploaded"] = {"verify_sha256": one(verified[2][0], 2).hex(),
                               "remote_read_sha256": hashlib.sha256(read.stdout).hexdigest(),
                               "published_path_private": remote_path.startswith(str(home))}
        require(receipt["uploaded"]["published_path_private"], "发布路径不在私有 HOME")
        sibling = Connection(ssh, archive, "sibling")
        connections.append((sibling, "sibling"))
        require(initialize(sibling, epoch) == host, "并发连接主机身份变化")
        sibling_scope = scope(host, epoch, native)
        require(3 in request(sibling, sibling_scope, 0, 3, b""), "并发作用域激活失败")
        sibling_verify = request(sibling, sibling_scope, 1, 6,
                                 item(1, transfer) + item(2, spec))
        require(7 in sibling_verify, "并发连接读取了其他连接的未投递传输")
        receipt.setdefault("connection_exits", {})["sibling"] = sibling.close(archive, "sibling")
        require(receipt["connection_exits"]["sibling"] == 0, "并发 SSH 代理异常退出")
        connections.pop()
        transient = str(uuid.uuid4())
        result(request(first, current, 1, 4, item(1, transient) + item(2, spec)), 4)
        result(request(first, current, 1, 5,
                       item(1, transient) + item(2, 0) + item(3, original[:mid])), 4)
        staging_directory = Path(remote_path).parent
        require(len([entry for entry in staging_directory.iterdir() if entry.is_file()]) == 2,
                "未发布与已发布文件未同时留存")
        receipt.setdefault("connection_exits", {})["first"] = first.close(archive, "first")
        require(receipt["connection_exits"]["first"] == 0, "首条 SSH 代理异常退出")
        connections.clear()
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            remaining = [entry for entry in staging_directory.iterdir() if entry.is_file()]
            if remaining == [Path(remote_path)]:
                break
            time.sleep(.05)
        require(remaining == [Path(remote_path)], "断连后未发布图片未精确回收")
        receipt["unpublished_disconnect_cleanup"] = True
        second = Connection(ssh, archive, "second")
        connections.append((second, "second"))
        new_epoch = str(uuid.uuid4())
        require(initialize(second, new_epoch) == host, "重连主机身份变化")
        newer = scope(host, new_epoch, native)
        recovered = fields(result(request(second, newer, 1, 11,
                                          item(1, transfer) + item(2, key)), 11))
        require(2 in recovered and recovered[2][0] and not recovered.get(3, [0])[0],
                "断连后引用查询失败")
        old = request(second, current, 1, 11, item(1, transfer) + item(2, key))
        require(one(old[7][0], 1) == 3, "旧终端回调未拒绝")
        wrong = request(second, newer, 1, 11,
                        item(1, transfer) + item(2, str(uuid.uuid4())))
        require(7 in wrong, "错误恢复凭据未拒绝")
        released_receipt = fields(result(request(second, newer, 1, 11,
                                                 item(1, transfer) + item(2, key) + item(3, 1)), 11))
        require(released_receipt.get(3) == [1], "重连后恢复释放未确认")
        released = subprocess.run([*ssh, "read"], capture_output=True, timeout=15)
        require(released.returncode != 0 and not Path(remote_path).exists(), "释放后远端原图仍可读")
        receipt["recovery"] = {"same_host": True, "old_epoch_rejected": True,
                               "wrong_key_rejected": True, "release_removed_original": True}
        receipt["sibling_transfer_denied"] = True
        receipt["passed"] = True
    except Exception as error:
        receipt["error"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        for connection, label in connections:
            try:
                receipt.setdefault("connection_exits", {})[label] = connection.close(archive, label)
            except Exception as error:
                receipt.setdefault("cleanup_errors", []).append(f"{label}: {error}")
        if server:
            if server.poll() is None:
                server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                receipt.setdefault("cleanup_errors", []).append("隔离 sshd 未退出")
            receipt["isolated_sshd_gone"] = server.poll() is not None
        try:
            receipt["daemon_cleanup"] = cleanup_daemon(home, product, identity)
        except Exception as error:
            receipt.setdefault("cleanup_errors", []).append(f"daemon: {error}")
        receipt["passed"] = bool(receipt["passed"] and receipt.get("isolated_sshd_gone")
                                 and receipt.get("daemon_cleanup", {}).get("daemon_gone")
                                 and not receipt.get("cleanup_errors"))
        (archive / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    if not receipt["passed"]:
        raise RuntimeError("G08 回环 SSH 图片验收未通过；详见归档收据")


if __name__ == "__main__":
    main()
