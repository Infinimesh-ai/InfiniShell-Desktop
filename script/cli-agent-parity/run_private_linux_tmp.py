"""以真实账号私有短 TMPDIR 运行一条 Linux 门禁；清理证据不足只保留，不改变命令退出码。"""

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import stat
import subprocess
import sys
import tempfile
import time


def sha256(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def identity(info):
    return {"device": info.st_dev, "inode": info.st_ino, "owner": info.st_uid,
            "mode": stat.S_IMODE(info.st_mode)}


def check_ancestors(path, owner):
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise ValueError("临时根祖先必须是真实绝对路径")
    for ancestor in (path, *path.parents):
        info = ancestor.lstat()
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, owner)
                or stat.S_IMODE(info.st_mode) & 0o022):
            raise ValueError("临时根祖先权限或所有者不可信")


def create_root(created):
    owner = os.geteuid()
    home = Path(pwd.getpwuid(owner).pw_dir)
    check_ancestors(home, owner)
    before = home.lstat()
    if before.st_uid != owner or len(os.fsencode(home / "t-12345678")) > 45:
        raise ValueError("系统账号目录所有者无效或短 TMPDIR 超过 45 字节预算")
    path = Path(tempfile.mkdtemp(prefix="t-", dir=home))
    created(path)
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        info = os.fstat(descriptor)
        check_ancestors(path, owner)
        if (identity(before) != identity(home.lstat()) or info.st_dev != before.st_dev
                or info.st_uid != owner or stat.S_IMODE(info.st_mode) != 0o700
                or len(os.fsencode(path)) > 45):
            raise ValueError("新建临时目录身份不符合合同，保留待核查")
        return path, descriptor, identity(info)
    except BaseException:
        os.close(descriptor)
        raise


def process(pid):
    raw = Path(f"/proc/{pid}/stat").read_text()
    fields = raw[raw.rindex(")") + 1:].split()
    return {"pid": pid, "start_time_ticks": int(fields[19]),
            "parent_pid": int(fields[1]), "state": fields[0]}


def process_key(value):
    return value["pid"], value["start_time_ticks"]


def mount_id(descriptor):
    for line in Path(f"/proc/self/fdinfo/{descriptor}").read_text().splitlines():
        if line.startswith("mnt_id:\t"):
            return int(line.split(":", 1)[1])
    raise ValueError("目录句柄缺少内核挂载身份")


def descendants(table, parent_pid):
    selected = {parent_pid}
    while True:
        expanded = selected | {pid for pid, value in table.items()
                               if value["parent_pid"] in selected}
        if expanded == selected:
            return [value for pid, value in table.items() if pid in selected and pid != parent_pid]
        selected = expanded


def process_table():
    table, incomplete = {}, []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            value = process(int(entry.name))
            table[value["pid"]] = value
        except (FileNotFoundError, ProcessLookupError):
            pass
        except (OSError, ValueError, IndexError):
            incomplete.append(int(entry.name))
    return table, incomplete


def reap_adopted(records):
    # 子收割者使 setsid、双 fork 后代仍归本包装器；仅 wait 自己的退出子进程，不发信号。
    while True:
        try:
            result = os.waitid(os.P_ALL, 0, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        except ChildProcessError:
            return
        if result is None:
            return
        value = process(result.si_pid)
        _, status = os.waitpid(result.si_pid, os.WNOHANG)
        records[process_key(value)] = {**value, "reaped_status": status}


def inspect_tree(descriptor, device, mount, entries):
    if mount_id(descriptor) != mount:
        raise ValueError("临时目录包含挂载点，保留")
    for name in os.listdir(descriptor):
        before = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
        if before.st_dev != device or stat.S_ISLNK(before.st_mode):
            raise ValueError("临时目录包含链接或跨设备节点，保留")
        entries.add((before.st_dev, before.st_ino))
        if stat.S_ISDIR(before.st_mode):
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                            dir_fd=descriptor)
            try:
                if identity(os.fstat(child)) != identity(before):
                    raise ValueError("临时子目录身份发生变化")
                inspect_tree(child, device, mount, entries)
            finally:
                os.close(child)
        elif not stat.S_ISREG(before.st_mode):
            raise ValueError("临时目录仍有非常规节点，保留")


def referenced_by_proc(path, entries, own_descriptor):
    reasons = []
    table, incomplete = process_table()
    if incomplete:
        reasons.append("proc_process_visibility_incomplete")
    prefix = str(path) + "/"
    for pid, before in table.items():
        try:
            directory = Path(f"/proc/{pid}")
            targets = [directory / "cwd", directory / "root", directory / "exe"]
            targets.extend((directory / "fd").iterdir())
            for target in targets:
                if pid == os.getpid() and target == directory / "fd" / str(own_descriptor):
                    continue
                try:
                    link = os.readlink(target)
                    info = target.stat()
                    if (link == str(path) or link.startswith(prefix)
                            or (info.st_dev, info.st_ino) in entries):
                        reasons.append(f"proc_reference:{pid}")
                        break
                except (FileNotFoundError, ProcessLookupError):
                    continue
            if process_key(process(pid)) != process_key(before):
                reasons.append(f"proc_generation_changed:{pid}")
        except (FileNotFoundError, ProcessLookupError):
            continue
        except (OSError, ValueError, IndexError):
            reasons.append(f"proc_reference_visibility_incomplete:{pid}")
    return sorted(set(reasons))


def remove_contents(descriptor, device, mount):
    # 只相对已打开的目录 FD 操作；每项都不跟随链接，也不跨设备。
    if mount_id(descriptor) != mount:
        raise ValueError("清理期间出现挂载点")
    for name in os.listdir(descriptor):
        before = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
        if before.st_dev != device or stat.S_ISLNK(before.st_mode):
            raise ValueError("清理期间出现链接或跨设备节点")
        if stat.S_ISDIR(before.st_mode):
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                            dir_fd=descriptor)
            try:
                if identity(os.fstat(child)) != identity(before):
                    raise ValueError("清理期间子目录身份改变")
                remove_contents(child, device, mount)
                if identity(os.stat(name, dir_fd=descriptor, follow_symlinks=False)) != identity(before):
                    raise ValueError("清理期间子目录被替换")
                os.rmdir(name, dir_fd=descriptor)
            finally:
                os.close(child)
        elif stat.S_ISREG(before.st_mode):
            if identity(os.stat(name, dir_fd=descriptor, follow_symlinks=False)) != identity(before):
                raise ValueError("清理期间文件被替换")
            os.unlink(name, dir_fd=descriptor)
        else:
            raise ValueError("清理期间出现非常规节点")


def cleanup(path, descriptor, expected, records, before_remove):
    check_ancestors(path, os.geteuid())
    if identity(path.lstat()) != expected or identity(os.fstat(descriptor)) != expected:
        raise ValueError("临时根身份改变")
    reap_adopted(records)
    table, incomplete = process_table()
    remaining = descendants(table, os.getpid())
    for value in remaining:
        records[process_key(value)] = value
    if remaining or incomplete:
        return {"cleanup_ready": False, "status": "retained",
                "reasons": ["descendants_or_process_visibility_incomplete"]}
    entries = {(expected["device"], expected["inode"])}
    mount = mount_id(descriptor)
    inspect_tree(descriptor, expected["device"], mount, entries)
    reasons = referenced_by_proc(path, entries, descriptor)
    if reasons:
        return {"cleanup_ready": False, "status": "retained", "reasons": reasons}
    check_ancestors(path, os.geteuid())
    if identity(path.lstat()) != expected or identity(os.fstat(descriptor)) != expected:
        raise ValueError("清理前临时根身份改变")
    parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        if (mount_id(parent) != mount
                or identity(os.stat(path.name, dir_fd=parent, follow_symlinks=False)) != expected):
            raise ValueError("清理前临时根目录项被替换")
        # 先把退出、无引用和目录身份的收尾证明写到根外收据，再删除临时内容。
        before_remove()
        remove_contents(descriptor, expected["device"], mount)
        if identity(os.stat(path.name, dir_fd=parent, follow_symlinks=False)) != expected:
            raise ValueError("清理后临时根目录项被替换")
        os.rmdir(path.name, dir_fd=parent)
    finally:
        os.close(parent)
    return {"cleanup_ready": True, "status": "cleaned", "reasons": []}


def execute(argv, receipt_path):
    if sys.platform != "linux" or not argv:
        raise ValueError("仅允许 Linux 和明确的非空命令参数")
    source = Path(__file__).resolve(strict=True)
    executable = shutil.which(argv[0])
    if executable is None:
        raise ValueError("命令文件不存在")
    executable = Path(executable).resolve(strict=True)
    receipt_path = receipt_path.absolute()
    # 新收据不覆盖旧证据；后续只写同一 FD，不因目录项替换而重开文件。
    descriptor = os.open(receipt_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
                         | os.O_CLOEXEC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        receipt = {"schema_version": 1, "platform": "linux", "status": "preparing",
                   "cleanup_ready": False, "argv": argv,
                   "argv_sha256": hashlib.sha256(json.dumps(argv, ensure_ascii=False,
                                                             separators=(",", ":")).encode()).hexdigest(),
                   "source": str(source), "source_sha256": sha256(source),
                   "executable": str(executable), "executable_sha256": sha256(executable),
                   "wrapper": process(os.getpid()), "descendants": []}

        def save():
            if identity(os.fstat(output.fileno())) != identity(receipt_path.lstat()):
                raise ValueError("收据文件身份改变")
            output.seek(0)
            json.dump(receipt, output, ensure_ascii=False, indent=2)
            output.write("\n")
            output.truncate()
            output.flush()
            os.fsync(output.fileno())

        save()
        def created(path):
            receipt["tmpdir"] = str(path)
            save()

        try:
            path, root_fd, root_identity = create_root(created)
        except Exception as error:
            receipt.update(status="retained", runner_error=type(error).__name__,
                           command_exit_code=None, reasons=["root_setup_failed"])
            save()
            return 2
        records, command, exit_code = {}, None, 2
        receipt.update(tmpdir=str(path), directory_identity=root_identity)
        try:
            save()
            # 不改变 HOME/CODEX_HOME，不改变全局环境，也不建立后台监控服务。
            if ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) != 0:
                raise OSError(ctypes.get_errno(), "不能建立后代退出证据")
            environment = dict(os.environ, TMPDIR=str(path))
            command = subprocess.Popen(argv, env=environment)
            value = process(command.pid)
            records[process_key(value)] = value
            receipt.update(status="running", child=value)
            save()
            while True:
                table, incomplete = process_table()
                for value in descendants(table, os.getpid()):
                    records[process_key(value)] = value
                if incomplete:
                    receipt["process_observation_incomplete"] = True
                result = command.poll()
                if result is not None:
                    exit_code = result
                    records[process_key(receipt["child"])]["exit_code"] = result
                    break
                time.sleep(0.1)
        except KeyboardInterrupt:
            exit_code = command.returncode if command and command.returncode is not None else 130
            receipt["interrupted"] = True
        except Exception as error:
            # 收尾失败与测试失败分别记录；已取得的测试退出码不能被清理错误覆盖。
            if command is not None and command.poll() is not None:
                exit_code = command.returncode
            receipt["runner_error"] = type(error).__name__
        finally:
            receipt["command_exit_code"] = exit_code
            try:
                if "runner_error" in receipt:
                    raise ValueError("运行器取证中断，保留临时目录")
                receipt["source_unchanged"] = sha256(source) == receipt["source_sha256"]
                if not receipt["source_unchanged"]:
                    raise ValueError("包装器源码在运行中改变，保留临时目录")
                receipt["directory_identity_after"] = identity(path.lstat())

                def before_remove():
                    receipt.update(status="cleanup_ready", cleanup_ready=True,
                                   descendants=sorted(records.values(), key=process_key),
                                   cleanup_proof={"descendants_remaining": 0,
                                                  "proc_references": 0,
                                                  "tree_symlinks_or_mounts": 0})
                    save()

                receipt.update(cleanup(path, root_fd, root_identity, records, before_remove))
            except Exception as error:
                receipt.update(status="retained", cleanup_ready=False,
                               reasons=["cleanup_evidence_incomplete:" + type(error).__name__])
            finally:
                os.close(root_fd)
            receipt["descendants"] = sorted(records.values(), key=process_key)
            try:
                save()
            except Exception:
                # 归档异常也不能把已完成命令的退出码换成另一个结果。
                print("Linux TMPDIR 收据保存失败，请保留先前记录中的目录", file=sys.stderr)
        return exit_code if exit_code >= 0 else 128 - exit_code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    return execute(command, args.receipt)


if __name__ == "__main__":
    raise SystemExit(main())
