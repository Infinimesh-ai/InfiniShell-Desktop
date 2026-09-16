#!/usr/bin/env python3
"""隔离回环 SSH 的 CLI 上下文与真实 tmux 通知传输；不代表完整产品 SSH/UI 验收。"""

import argparse
import getpass
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time

from apply_notification_patch import apply_files, bundle_data, default_bundle, validate_tree


REMOTE = r'''
import datetime,json,os,pathlib,shlex,shutil,subprocess,sys,time
configuration=json.loads(pathlib.Path(sys.argv[1]).read_text())
case=sys.argv[2]
home=pathlib.Path(configuration["home"])
work=pathlib.Path(configuration["work"])
os.chdir(work)
if case=="context":
    versions={}
    for name in ("codex","claude","grok"):
        executable=shutil.which(name)
        result=subprocess.run([executable,"--version"],capture_output=True,text=True,timeout=8)
        versions[name]={"path":executable,"exit_code":result.returncode,"version":result.stdout.strip()}
    print(json.dumps({"home":os.environ["HOME"],"cwd":str(pathlib.Path.cwd()),"versions":versions,"codex_plugin_present":(home/".codex/plugins/cache/codex-warp/warp/0.4.0/.codex-plugin/plugin.json").exists(),"claude_registry_present":(home/".claude/plugins/installed_plugins.json").exists(),"grok_registry_present":(home/".grok/installed-plugins/registry.json").exists()}))
    raise SystemExit(0)
parts=case.split(":")
if len(parts)!=2 or parts[0] not in ("direct","tmux-on","tmux-off","emit") or parts[1] not in ("codex","claude","grok"):
    raise SystemExit("未知隔离探测命令")
mode,agent=parts
if mode.startswith("tmux-"):
    socket_path=configuration["socket_dir"]+"/tmux-"+mode+"-"+agent+".sock"
    config_path=configuration["root"]+"/tmux-"+mode+".conf"
    pathlib.Path(config_path).write_text("set -g status off\nset -g default-shell /bin/sh\nset -g allow-passthrough "+("on" if mode=="tmux-on" else "off")+"\n")
    command=shlex.join([sys.executable,__file__,sys.argv[1],"emit:"+agent])
    os.execv(configuration["tmux"],[configuration["tmux"],"-f",config_path,"-S",socket_path,"new-session","-s","parity","-n","probe","-x","100","-y","30",command])
root=pathlib.Path(configuration["plugins"][agent])
env=os.environ.copy()
session="ssh-probe-"+agent+"-"+mode+"-"+str(os.getpid())
env.update(WARP_CLI_AGENT_PROTOCOL_VERSION="1",WARP_CLIENT_VERSION="v0.2026.09.16.00.00.stable_00",CLAUDE_PLUGIN_ROOT=str(root),PLUGIN_ROOT=str(root),GROK_PLUGIN_ROOT=str(root),GROK_PLUGIN_DATA=str(home/"grok-plugin-data"))
payload={"hook_event_name":"SessionStart","session_id":session,"cwd":str(work)}
if agent=="grok":
    env.update(GROK_HOOK_EVENT="SessionStart",GROK_SESSION_ID=session)
    payload={"hookEventName":"SessionStart","sessionId":session,"cwd":str(work),"timestamp":datetime.datetime.now(datetime.timezone.utc).isoformat()}
hooks=json.loads((root/"hooks/hooks.json").read_text())["hooks"]["SessionStart"][0]["hooks"][0]
if "args" in hooks:
    args=[argument.replace("${CLAUDE_PLUGIN_ROOT}",str(root)) for argument in hooks["args"]]
    command=[hooks["command"],*args]
else:
    command=["/bin/sh","-c",hooks["command"]]
result=subprocess.run(command,input=json.dumps(payload).encode(),env=env,timeout=8)
print("INFINISHELL_HOOK_REPLAY_DONE",flush=True)
time.sleep(.15)
raise SystemExit(result.returncode)
'''


def capture(command, timeout=18):
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    selector = selectors.DefaultSelector()
    streams = {"stdout": bytearray(), "stderr": bytearray()}
    for name in streams:
        selector.register(getattr(process, name), selectors.EVENT_READ, name)
    deadline = time.monotonic() + timeout
    try:
        while selector.get_map():
            if time.monotonic() >= deadline:
                raise TimeoutError("隔离 SSH 命令超时")
            for key, _ in selector.select(.1):
                data = os.read(key.fileobj.fileno(), 65536)
                if data:
                    streams[key.data].extend(data)
                else:
                    selector.unregister(key.fileobj)
        process.wait(timeout=3)
        return process.returncode, bytes(streams["stdout"]), bytes(streams["stderr"])
    finally:
        selector.close()
        if process.poll() is None:
            process.kill()
            process.wait()
        if process.stdin:
            process.stdin.close()


def execute(args, root, socket_directory):
    home, work = root / "远端 配置", root / "远端 工作区"
    home.mkdir(); work.mkdir()
    for name in (".codex", ".claude", ".grok", "bin"):
        (home / name).mkdir()
    paths = {"codex": args.codex, "claude": args.claude, "grok": args.grok, "node": Path(shutil.which("node") or ""), "jq": Path(shutil.which("jq") or ""), "bash": Path(shutil.which("bash") or "")}
    for name, executable in paths.items():
        if not executable.is_file():
            raise RuntimeError("缺少实际 CLI/运行时：" + name)
        (home / "bin" / name).symlink_to(executable.resolve())
    plugins = {}
    for agent, source, version in (("codex", args.codex_plugin, "0.4.0"), ("claude", args.claude_plugin, "2.2.0")):
        plugin = home / (agent + " 插件")
        shutil.copytree(source, plugin)
        metadata, replacements = bundle_data(default_bundle(), agent)
        validate_tree(plugin, version, metadata)
        apply_files(plugin, metadata, replacements)
        plugins[agent] = str(plugin)
    grok = home / "grok 插件"
    shutil.copytree(default_bundle() / "grok", grok)
    plugins["grok"] = str(grok)
    local_marker = root / "local-control/.codex/plugins/cache/codex-warp/warp/0.4.0/.codex-plugin/plugin.json"
    local_marker.parent.mkdir(parents=True)
    local_marker.write_text('{"version":"0.4.0","synthetic_local_control":true}')
    configuration = root / "probe.json"
    configuration.write_text(json.dumps({"root": str(root), "socket_dir": str(socket_directory), "home": str(home), "work": str(work), "tmux": str(args.tmux), "plugins": plugins}))
    remote = root / "remote.py"
    remote.write_text(REMOTE)
    for name in ("host-key", "client-key"):
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name)], check=True)
    authorized = root / "authorized_keys"
    authorized.write_text("restrict,pty " + (root / "client-key.pub").read_text())
    authorized.chmod(0o600)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    known = root / "known_hosts"
    known.write_text("[127.0.0.1]:" + str(port) + " " + (root / "host-key.pub").read_text())
    # 固定 ForceCommand 只接受探测名称，SSH 原命令不会作为 shell 源码执行。
    environment = {"HOME": str(home), "PATH": str(home / "bin") + ":/usr/bin:/bin:/usr/sbin:/sbin", "CODEX_HOME": str(home / ".codex"), "CLAUDE_CONFIG_DIR": str(home / ".claude"), "GROK_HOME": str(home / ".grok"), "TERM": "xterm-256color", "LANG": "en_US.UTF-8", "GROK_DISABLE_AUTOUPDATER": "1", "GROK_AUTO_UPDATE": "0"}
    launcher = root / "force-command.sh"
    launcher.write_text("#!/bin/sh\nexec " + shlex.join(["/usr/bin/env", "-i", *[key + "=" + value for key, value in environment.items()], str(Path(sys.executable).resolve()), str(remote), str(configuration)]) + ' "$SSH_ORIGINAL_COMMAND"\n')
    launcher.chmod(0o700)
    config = root / "sshd_config"
    lines = ["ListenAddress 127.0.0.1", f"Port {port}", f"HostKey {root}/host-key", f"PidFile {root}/sshd.pid", f"AuthorizedKeysFile {authorized}", "StrictModes yes", "PasswordAuthentication no", "KbdInteractiveAuthentication no", "UsePAM no", "AllowUsers " + getpass.getuser(), "PermitRootLogin no", "AllowTcpForwarding no", "X11Forwarding no", "PermitTunnel no", "PrintMotd no", "LogLevel ERROR", "SetEnv " + json.dumps("HOME=" + str(home), ensure_ascii=False), "ForceCommand " + str(launcher)]
    config.write_text("\n".join(lines) + "\n")
    subprocess.run([str(args.sshd), "-t", "-f", str(config)], capture_output=True, check=True)
    server_log = (root / "sshd.log").open("wb")
    server = subprocess.Popen([str(args.sshd), "-D", "-e", "-f", str(config)], stdout=server_log, stderr=server_log)
    command = ["ssh", "-F", "/dev/null", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=" + str(known), "-o", "ConnectTimeout=3", "-i", str(root / "client-key"), "-p", str(port)]
    try:
        deadline = time.monotonic() + 3
        while True:
            if server.poll() is not None:
                raise RuntimeError("隔离 sshd 提前退出")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=.1):
                    break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.03)
        code, output, error = capture([*command, "-T", getpass.getuser() + "@127.0.0.1", "context"])
        if code:
            raise RuntimeError("SSH 上下文检查失败：" + error.decode(errors="replace"))
        context = json.loads(output)
        context_ok = context["home"] == str(home) and context["cwd"] == str(work) and not any(context[key] for key in ("codex_plugin_present", "claude_registry_present", "grok_registry_present")) and all(value["exit_code"] == 0 and value["path"].startswith(str(home / "bin") + "/") for value in context["versions"].values())
        cases = []
        for mode in ("direct", "tmux-off", "tmux-on"):
            for agent in ("codex", "claude", "grok"):
                code, output, error = capture([*command, "-tt", getpass.getuser() + "@127.0.0.1", mode + ":" + agent])
                notifications = []
                for body in re.findall(rb"\x1b\]777;notify;warp://cli-agent;([^\x07]*)\x07", output):
                    try:
                        notifications.append(json.loads(body))
                    except (ValueError, UnicodeError):
                        pass
                correlated = [item for item in notifications if item.get("agent") == agent and item.get("event") == "session_start" and item.get("cwd") == str(work)]
                completed = b"INFINISHELL_HOOK_REPLAY_DONE" in output
                cases.append({"mode": mode, "agent": agent, "claude_transport_path": "compatibility_tty_version_unknown" if agent == "claude" else None, "ssh_exit": code, "failure_output": output.decode(errors="replace")[-2000:] if code else None, "hook_runner_completed": completed, "received_notifications": len(correlated), "notifications": correlated, "raw_stream_sha256": hashlib.sha256(output).hexdigest(), "raw_stream_bytes": len(output), "stderr": error.decode(errors="replace"), "transport_received": code == 0 and completed and len(correlated) == 1})
        return {"context": context, "remote_context_verified": context_ok, "synthetic_local_install_marker_exists": local_marker.exists(), "cases": cases, "direct_transport_passed": all(item["transport_received"] for item in cases if item["mode"] == "direct"), "tmux_passthrough_passed": all(item["transport_received"] for item in cases if item["mode"] == "tmux-on"), "default_tmux_blocking_observed": all(item["ssh_exit"] == 0 and item["hook_runner_completed"] and item["received_notifications"] == 0 for item in cases if item["mode"] == "tmux-off")}
    finally:
        if server.poll() is None:
            server.terminate(); server.wait(timeout=5)
        server_log.close()
        for socket_path in socket_directory.glob("tmux-*.sock"):
            subprocess.run([str(args.tmux), "-S", str(socket_path), "kill-server"], capture_output=True, timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("tmux", "codex", "claude", "grok", "codex-plugin", "claude-plugin"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--sshd", type=Path, default=Path("/usr/sbin/sshd"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("此回环 sshd/PTY 探测需要 Unix；Windows 远端 SSH 场景须单独验收")
    for name in ("tmux", "codex", "claude", "grok", "codex_plugin", "claude_plugin", "sshd"):
        setattr(args, name, getattr(args, name).resolve())
    repository = Path(__file__).resolve().parents[2]
    commit = subprocess.run(["git", "-C", str(repository), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "-C", str(repository), "status", "--porcelain"], capture_output=True, text=True, check=True).stdout.strip())
    result = {"source_commit": commit, "source_tree_dirty": dirty, "tmux_sha256": hashlib.sha256(args.tmux.read_bytes()).hexdigest(), "host_platform": sys.platform, "ssh": subprocess.run(["ssh", "-V"], capture_output=True, text=True).stderr.strip(), "tmux": subprocess.run([str(args.tmux), "-V"], capture_output=True, text=True, check=True).stdout.strip(), "credentials_provided": False, "model_requests": False, "native_cli_hook_triggered": False, "notification_origin": "bundled_hook_replay", "product_ssh_ui_verified": False, "different_os_host_verified": False}
    result["notification_patch"] = {agent: {"revision": bundle_data(default_bundle(), agent)[0]["patch_revision"], "warp_notify_sha256": hashlib.sha256((default_bundle() / agent / "scripts/warp-notify.sh").read_bytes()).hexdigest()} for agent in ("claude", "codex")}
    with tempfile.TemporaryDirectory(prefix="infinishell-ssh-tmux-") as temporary, tempfile.TemporaryDirectory(prefix="istmux-", dir="/tmp") as short_sockets:
        root = Path(temporary).resolve()
        try:
            result.update(execute(args, root, Path(short_sockets).resolve()))
        except Exception as error:
            result["error"] = str(error)
        result = json.loads(json.dumps(result, ensure_ascii=False).replace(str(root), "<isolated-probe>"))
    if "error" in result:
        result["error"] = result["error"].replace(getpass.getuser() + "@127.0.0.1", "<local-user>@127.0.0.1")
    result["passed"] = result.get("remote_context_verified") is True and result.get("direct_transport_passed") is True and result.get("tmux_passthrough_passed") is True and result.get("default_tmux_blocking_observed") is True
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": result["passed"], "remote_context_verified": result.get("remote_context_verified"), "direct_transport_passed": result.get("direct_transport_passed"), "tmux_passthrough_passed": result.get("tmux_passthrough_passed"), "error": result.get("error")}))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
