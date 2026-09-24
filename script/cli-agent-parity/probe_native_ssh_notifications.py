#!/usr/bin/env python3
"""为固定 Mac CLI 准备独占 SSH/tmux 前台通知验收；真实输入由产品终端发出。"""

import argparse
from collections import Counter
import getpass
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid

import apply_notification_patch as patch
import run_claude_adapter_live as claude
import run_grok_official_adapter_live as grok
from probe_codex_windows_hooks import NativeRecorder

VERSIONS = {"codex": "codex-cli 0.156.1", "claude": "2.1.280 (Claude Code)",
            "grok": "grok 1.0.41 (4220f3b224a6)"}
MODES = ("local", "direct", "tmux-on", "tmux-off")
OSC = re.compile(rb"\x1b\]777;notify;warp://cli-agent;([^\x07]*)\x07")


def write_private(path, contents):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as target:
        target.write(contents)


def run(arguments, env, cwd, timeout=30):
    value = subprocess.run([str(item) for item in arguments], env=env, cwd=cwd,
                           capture_output=True, timeout=timeout)
    if value.returncode:
        raise RuntimeError(f"native_command_exit_{value.returncode}")
    return value.stdout


def auth_stat(path):
    value = path.stat()
    return tuple(getattr(value, field) for field in ("st_ino", "st_size", "st_mtime_ns", "st_mode"))


def grok_frontend_sandbox(root, credential_directory, port, case):
    profile = grok.shared.sandbox_profile(root, credential_directory, port, case / "leader.sock")
    profile += '(allow file-write* (regex #"^/dev/(tty|ttys[0-9]+|ptmx)$"))'
    if case.name in ("tmux-on", "tmux-off"):
        # 原生 worker 只访问当前 pane 的 tmux 服务，不能放开其他 socket 或绕过 pane 解析。
        socket_path = json.dumps(str(case / "tmux.sock"))
        profile += f'(allow network-outbound (remote unix-socket (path-literal {socket_path})))'
    return profile


def plugin_copy(agent, root, claude_plugin=None):
    destination = root / "plugin"
    assets = patch.default_bundle() / agent
    source = (assets / "source/plugins/warp" if agent == "codex" else
              claude_plugin if agent == "claude" else assets)
    if source is None:
        raise ValueError("verified_claude_plugin_required")
    shutil.copytree(source, destination)
    if agent != "grok":
        metadata, replacements = patch.bundle_data(patch.default_bundle(), agent)
        patch.validate_tree(destination, patch.CONTRACTS[agent][1], metadata)
        patch.apply_files(destination, metadata, replacements)
    return destination


def install_codex(executable, root, environment, plugin):
    marketplace = root / "marketplace"
    (marketplace / ".agents/plugins").mkdir(parents=True)
    shutil.copytree(plugin, marketplace / "plugins/warp")
    write_private(marketplace / ".agents/plugins/marketplace.json", json.dumps({
        "name": "infinishell-native-notification", "plugins": [{"name": "warp",
        "source": "./plugins/warp", "version": "0.4.0",
        "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"}}]}))
    run([executable, "plugin", "marketplace", "add", marketplace, "--json"], environment, root)
    installed = json.loads(run([executable, "plugin", "add",
        "warp@infinishell-native-notification", "--json"], environment, root))
    cache = Path(installed["installedPath"]).resolve()
    if not cache.is_relative_to(root / "home/.codex"):
        raise ValueError("plugin_cache_outside_private_home")
    reference = {str(p.relative_to(plugin)): p.read_bytes() for p in plugin.rglob("*") if p.is_file()}
    actual = {str(p.relative_to(cache)): p.read_bytes() for p in cache.rglob("*") if p.is_file()}
    if actual != reference:
        raise ValueError("plugin_cache_content_mismatch")
    recorder = NativeRecorder([str(executable), "app-server", "--stdio"], environment, root, [])
    try:
        recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_native_notifications",
            "version": "0.1.0"}, "capabilities": {"experimentalApi": True}}, 1)
        recorder.send({"method": "initialized"})
        hooks = recorder.rpc("hooks/list", {"cwds": [str(root / "project")]}, 2)
        native = [hook for group in hooks["data"] for hook in group["hooks"]]
        commands = {h["command"] for groups in json.loads((cache / "hooks/hooks.json").read_text())["hooks"].values()
                    for group in groups for h in group["hooks"]}
        if not native or any(Path(h["sourcePath"]).resolve() != cache / "hooks/hooks.json"
                             or h["command"] not in commands for h in native):
            raise ValueError("unexpected_native_hook")
        for number, hook in enumerate(native, 10):
            recorder.rpc("config/value/write", {"keyPath": "hooks.state." + json.dumps(hook["key"]),
                "value": {"enabled": True, "trusted_hash": hook["currentHash"]},
                "mergeStrategy": "upsert", "filePath": str(root / "home/.codex/config.toml")}, number)
    finally:
        recorder.close()


def install_grok_startup_bridge(executable, root, environment):
    installed = json.loads(run([executable, "plugin", "list", "--json"], environment, root))
    matches = [entry for entry in installed if entry.get("name") == "infinishell-grok"]
    if len(matches) != 1 or matches[0].get("version") != "0.1.4":
        raise ValueError("unexpected_grok_plugin_registration")
    plugin = Path(matches[0]["path"]).resolve()
    if not plugin.is_relative_to(root / "home/.grok/installed-plugins"):
        raise ValueError("grok_plugin_outside_private_home")
    bridge_source = Path(__file__).resolve().parents[2] / "app/src/terminal/cli_agent_sessions/plugin_manager/grok_native_hook_bridge.cjs"
    hooks = root / "home/.grok/hooks"
    hooks.mkdir(mode=0o700)
    bridge = hooks / "infinishell-1.0.41.cjs"
    write_private(bridge, bridge_source.read_text())
    config = json.loads((plugin / "hooks/hooks.json").read_text())
    command = shlex.join([shutil.which("node", path=environment["PATH"]), str(bridge), str(executable), str(plugin)])
    for groups in config["hooks"].values():
        for group in groups:
            for hook in group["hooks"]:
                hook["command"] = command
    write_private(hooks / "infinishell-1.0.41.json", json.dumps(config, indent=2))


def install_grok_with_production_test(binary, executable, root, environment):
    # 在复制认证以前调用原 Rust 安装器；其真实升级/修复测试最终保留可用的受控安装。
    paths = {"HOME": "home", "USERPROFILE": "home", "GROK_HOME": "home/.grok",
        "APPDATA": "home/AppData/Roaming", "LOCALAPPDATA": "home/AppData/Local",
        "XDG_CONFIG_HOME": "home/.config", "XDG_DATA_HOME": "home/.local/share",
        "XDG_CACHE_HOME": "home/.cache", "TMPDIR": "tmp"}
    for relative in (*paths.values(), "home/Library/Application Support", "work", "installer-bin"):
        (root / relative).mkdir(parents=True, exist_ok=True, mode=0o700)
    node = Path(shutil.which("node", path=environment["PATH"])).resolve()
    (root / "installer-bin/grok").symlink_to(executable)
    (root / "installer-bin/node").symlink_to(node)
    write_private(root / ".infinishell-grok-plugin-live", "isolated unauthenticated Grok plugin verification\n")
    installer_env = dict(environment, **{key: str(root / relative) for key, relative in paths.items()})
    installer_env.update(PATH=str(root / "installer-bin") + ":/usr/bin:/bin",
        GROK_AUTO_UPDATE="0", GROK_DISABLE_AUTOUPDATER="1",
        INFINISHELL_GROK_PLUGIN_LIVE_ROOT=str(root),
        INFINISHELL_GROK_PLUGIN_LIVE_GROK_SHA256=hashlib.sha256(executable.read_bytes()).hexdigest(),
        INFINISHELL_GROK_PLUGIN_LIVE_NODE_SHA256=hashlib.sha256(node.read_bytes()).hexdigest(),
        INFINISHELL_GROK_PLUGIN_LIVE_NODE_VERSION=run([node, "--version"], environment, root).decode().strip())
    case = "terminal::cli_agent_sessions::plugin_manager::grok::tests::live_grok_production_installer_repairs_and_preserves_disable"
    result = subprocess.run([str(binary.resolve()), "--exact", case, "--ignored", "--test-threads=1", "--nocapture"],
        env=installer_env, cwd=root / "work", capture_output=True, timeout=200)
    write_private(root / "production-installer-output.private.txt",
        (result.stdout + result.stderr).decode("utf-8", errors="replace"))
    if result.returncode:
        raise RuntimeError("grok_production_installer_failed")
    receipt = json.loads((root / "grok-production-installer.json").read_text())
    if receipt.get("passed") is not True or receipt.get("grok_version") != "1.0.41":
        raise ValueError("grok_production_installer_receipt_mismatch")
    return {"passed": True, "test_binary_sha256": receipt["test_binary_sha256"],
            "production_steps": len(receipt["steps"]), "credentials_provided": False}


def notification_summary(raw, agent):
    # tmux 内侧日志仍有 DCS 转义；公开仅记录原生身份摘要和事件数量。
    values = []
    for body in OSC.findall(raw.replace(b"\x1b\x1b", b"\x1b")):
        try:
            value = json.loads(body)
        except (ValueError, UnicodeError):
            continue
        if value.get("agent") == agent:
            values.append(value)
    return {"events": dict(Counter(v.get("event", "unknown") for v in values)),
            "native_session_count": len({v.get("session_id") for v in values}),
            "native_session_sha256": sorted({hashlib.sha256(str(v.get("session_id")).encode()).hexdigest()
                                              for v in values}),
            "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest(),
            "native_cli_hook_triggered": bool(values),
            "product_ssh_ui_verified": False, "outer_tmux_passthrough_verified": False}


def product_notification_summary(log, root, agent):
    # Codex 可能直接写 SSH_TTY，内层 script 录制不包含外层真正收到的 OSC。
    # 仅从产品原生接收日志取固定字段；用户输入、回复、原生 ID 均不投影。
    records = {mode: [] for mode in MODES}
    events = {"session_start", "prompt_submit", "stop", "stop_failure", "cancelled",
              "tool_complete", "permission_request", "permission_replied", "question_asked",
              "idle_prompt", "notification"}
    projects = {str((root / mode / "project").resolve()): mode for mode in MODES}
    for line in log.decode("utf-8", errors="replace").splitlines():
        marker = 'Received OSC 777 notification: title=Some("warp://cli-agent"), body='
        if marker not in line:
            continue
        try:
            value = json.JSONDecoder().raw_decode(line.split(marker, 1)[1])[0]
        except (ValueError, TypeError):
            continue
        if (not isinstance(value, dict) or value.get("agent") != agent or value.get("v", 1) != 1
                or not isinstance(value.get("event"), str) or value["event"] not in events
                or not isinstance(value.get("cwd"), str)):
            continue
        mode = projects.get(value.get("cwd"))
        if mode is None:
            continue
        session = value.get("session_id")
        turn = value.get("turn_id") or value.get("prompt_id")
        timestamp = re.match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", line)
        records[mode].append({"event": value["event"],
            "timestamp": timestamp.group() if timestamp else None,
            "native_session_sha256": hashlib.sha256(session.encode()).hexdigest() if isinstance(session, str) and session else None,
            "turn_sha256": hashlib.sha256(turn.encode()).hexdigest() if isinstance(turn, str) and turn else None})
    cases = {}
    for mode, observed in records.items():
        submitted = {(e["native_session_sha256"], e["turn_sha256"]) for e in observed
                     if e["event"] == "prompt_submit" and e["native_session_sha256"] and e["turn_sha256"]}
        stopped = {(e["native_session_sha256"], e["turn_sha256"]) for e in observed
                   if e["event"] == "stop" and e["native_session_sha256"] and e["turn_sha256"]}
        cases[mode] = {"events": dict(Counter(e["event"] for e in observed)),
            "native_session_count": len({e["native_session_sha256"] for e in observed if e["native_session_sha256"]}),
            "paired_prompt_stop_turns": len(submitted & stopped),
            "unpaired_prompt_turns": len(submitted - stopped),
            "product_received_notifications": bool(observed), "records": observed}
    return {"agent": agent, "source": "isolated_product_terminal_log", "cases": cases,
            "log_bytes": len(log), "log_sha256": hashlib.sha256(log).hexdigest(),
            "raw_payloads_included": False, "different_os_host_verified": False,
            "model_response_content_verified": False}


def process_identity(pid):
    value = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "pgid=,lstart="],
                           capture_output=True, text=True, timeout=5)
    fields = value.stdout.strip().split(maxsplit=1)
    return {"pid": pid, "pgid": int(fields[0]), "started": fields[1]} if len(fields) == 2 else None


def private_native_pids(root, executable):
    # 仅选择明确携带本次私有路径的指定二进制，进程参数不写入回执。
    result = subprocess.run(["/bin/ps", "-axo", "pid=,command="],
                            capture_output=True, text=True, timeout=5)
    return [int(fields[0]) for line in result.stdout.splitlines()
            if len(fields := line.strip().split(maxsplit=1)) == 2
            and str(executable) in fields[1] and str(root) + "/" in fields[1]]


def cleanup_native(root, executable):
    for mode in MODES:
        path = root / mode / "process.safe.json"
        if not path.is_file():
            continue
        previous = json.loads(path.read_text())
        if process_identity(previous["pid"]) == previous and previous["pgid"] == previous["pid"]:
            try:
                os.killpg(previous["pgid"], signal.SIGTERM)
            except ProcessLookupError:
                pass
    for pid in private_native_pids(root, executable):
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + 8
    while private_native_pids(root, executable) and time.monotonic() < deadline:
        time.sleep(.1)
    return not private_native_pids(root, executable)


def remote(root, mode):
    if mode == "verify":
        print(json.dumps({"ssh_authenticated": True, "cli_started": False, "model_inputs": 0}))
        return
    if mode not in MODES:
        raise ValueError("unknown_case")
    data = json.loads((root / "private-config.json").read_text())
    case = root / mode
    environment = data["environment"]
    environment.update({key: os.environ[key] for key in ("SSH_TTY", "SSH_CONNECTION", "SSH_CLIENT") if key in os.environ})
    if os.isatty(0):
        # Grok leader 派生的 hook 无控制终端，绑定本次连接的真实 PTY 供原生 worker 回退。
        environment["WARP_CLI_AGENT_TTY"] = os.ttyname(0)
    command = data["commands"][mode]
    if mode in ("local", "direct"):
        write_private(case / "started.once", "native_frontend\n")
        write_private(case / "process.safe.json", json.dumps(process_identity(os.getpid())))
        os.chdir(case / "project")
        os.execve("/usr/bin/script", ["/usr/bin/script", "-q", str(case / "terminal.raw"), *command], environment)
    tmux = data["tmux"]
    tmux_socket = str(case / "tmux.sock")
    base = [tmux, "-S", tmux_socket]
    existing = subprocess.run([*base, "has-session", "-t", "parity"], env=environment, capture_output=True)
    if existing.returncode == 0:
        # 断线只接回同一 pane；绝不重复启动或重放输入。
        os.execve(tmux, [*base, "attach-session", "-t", "parity"], environment)
    write_private(case / "started.once", "native_frontend\n")
    write_private(case / "pane.raw", "")
    payload = shlex.join(["/usr/bin/script", "-q", str(case / "terminal.raw"), *command])
    # 原生 worker 写 pane_tty 会绕过内层 script；在 pane 解析透传开关之前保留真实字节。
    recorder = "/bin/cat > " + shlex.quote(str(case / "pane.raw"))
    payload = shlex.join([tmux, "-S", tmux_socket, "pipe-pane", "-O", "-t", "parity:0.0", recorder]) + " && exec " + payload
    os.execve(tmux, [*base, "-f", str(case / "tmux.conf"), "new-session", "-s", "parity",
                     "-c", str(case / "project"), payload], environment)


def serve(args):
    if sys.platform != "darwin" or not 60 <= args.lifetime <= 7200:
        raise ValueError("unsupported_platform_or_budget")
    root = args.root.resolve()
    root.mkdir(mode=0o700)
    os.umask(0o077)
    for name in ("home/.codex", "home/.grok", "tmp", "project"):
        (root / name).mkdir(parents=True, mode=0o700)
    environment = {"PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(root / "home"),
        "TMPDIR": str(root / "tmp"), "SHELL": "/bin/bash", "TERM": "xterm-256color", "LANG": "en_US.UTF-8",
        "WARP_CLI_AGENT_PROTOCOL_VERSION": "1", "WARP_CLIENT_VERSION": "native-ssh-parity",
        "TERM_PROGRAM": "WarpTerminal", "DISABLE_AUTOUPDATER": "1", "DISABLE_UPDATES": "1",
        "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_TERMINAL_PROMPT": "0"}
    executable = args.executable.resolve()
    if run([executable, "--version"], environment, root).decode().strip() != VERSIONS[args.agent]:
        raise ValueError("fixed_cli_version_mismatch")
    plugin = plugin_copy(args.agent, root, args.claude_plugin)
    tunnel = server = None
    keys = None
    auth_source = auth_copy = auth_before = None
    cleanup = {"credential_copy_removed": None, "credential_source_stat_unchanged": None}
    production_installer = None
    try:
        if args.agent == "codex":
            environment["CODEX_HOME"] = str(root / "home/.codex")
            write_private(root / "home/.codex/config.toml", 'cli_auth_credentials_store="file"\nmodel="gpt-6-luna"\napproval_policy="on-request"\nsandbox_mode="read-only"\n')
            install_codex(executable, root, environment, plugin)
            auth_source = args.auth_source.resolve()
            auth_before = auth_stat(auth_source)
            auth_copy = grok.copy_private_auth(auth_source.parent, root / "home/.codex")
        elif args.agent == "claude":
            environment.update(claude.authorized_default_account_environment(root))
            environment.pop("CLAUDE_CODE_ENTRYPOINT", None)
            # 仅隔离前台主题、会话和引导状态；认证仍由原生 CLI 访问已有默认钥匙串。
            config = root / "home/.claude"
            config.mkdir(mode=0o700)
            write_private(config / ".claude.json", json.dumps({
                "hasCompletedOnboarding": True, "theme": "dark"}))
            environment.update(CLAUDE_CONFIG_DIR=str(config), CLAUDE_SECURESTORAGE_CONFIG_DIR="")
            claude.probe_authorized_default_account(executable, environment, root / "project")
        else:
            if args.installer_test_binary is not None:
                production_installer = install_grok_with_production_test(
                    args.installer_test_binary, executable, root, environment)
            auth_source = args.auth_source.resolve()
            auth_before = auth_stat(auth_source)
            auth_copy = grok.copy_private_auth(auth_source.parent, root / "home/.grok")
            tunnel = grok.OfficialTunnel(args.lifetime)
            port = tunnel.start()
            environment.update(grok.official_environment(root, port))
            environment["WARP_CLI_AGENT_NOTIFY_EXECUTABLE"] = str(args.worker.resolve())
            # 固定 41 的 leader 会在退出 hook 执行前关闭会话；前台链使用原生独立会话。
            native_config = '[cli]\nuse_leader=false\nauto_update=false\n[models]\ndefault="grok-4.7"\n[features]\nturn_summary=false\ntitle_refresh=false\n[[permission.rules]]\naction="ask"\ntool="any"\n'
            config_path = root / "home/.grok/config.toml"
            if production_installer is None:
                write_private(config_path, native_config)
                run([executable, "plugin", "install", "--trust", plugin], environment, root)
                install_grok_startup_bridge(executable, root, environment)
            else:
                previous = config_path.read_text()
                if {"cli", "models", "features", "permission"} & tomllib.loads(previous).keys():
                    raise ValueError("unexpected_private_installer_configuration")
                config_path.write_text(native_config + previous)
        (root / "bin").mkdir(mode=0o700)
        (root / "bin/tmux").symlink_to(args.tmux.resolve())
        environment["PATH"] = str(root / "bin") + os.pathsep + environment["PATH"]
        commands = {}
        for mode in MODES:
            case = root / mode
            (case / "project").mkdir(parents=True, mode=0o700)
            write_private(case / "tmux.conf", "set -g status off\nset -g default-shell /bin/bash\nset -g allow-passthrough " + ("on" if mode == "tmux-on" else "off") + "\n")
            if args.agent == "codex":
                command = [str(executable), "--no-alt-screen", "--disable", "shell_snapshot"]
            elif args.agent == "claude":
                command = [str(executable), "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--plugin-dir", str(plugin), "--session-id", str(uuid.uuid4())]
            else:
                endpoint = case / "leader.sock"
                profile = grok_frontend_sandbox(root, auth_source.parent, port, case)
                write_private(case / "sandbox.sb", profile)
                command = ["/usr/bin/sandbox-exec", "-f", str(case / "sandbox.sb"), str(executable),
                    "--minimal", "--no-alt-screen", "--leader-socket", str(endpoint), "--session-id", str(uuid.uuid4()),
                    "--no-subagents", "--disable-web-search", "--model", "grok-4.7"]
            commands[mode] = command
        write_private(root / "private-config.json", json.dumps({"environment": environment,
            "commands": commands, "tmux": str(args.tmux.resolve())}))
        # OpenSSH StrictModes 拒绝 /private/tmp 的可写祖先；认证文件保存在系统私有用户临时目录。
        user_temp = run(["/usr/bin/getconf", "DARWIN_USER_TEMP_DIR"], environment, root).decode().strip()
        keys = Path(tempfile.mkdtemp(prefix="infinishell-native-ssh-keys-", dir=user_temp)).resolve()
        for name in ("host-key", "client-key"):
            run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", keys / name], environment, root)
        write_private(keys / "authorized_keys", "restrict,pty " + (keys / "client-key.pub").read_text())
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        write_private(keys / "known_hosts", f"[127.0.0.1]:{port} " + (keys / "host-key.pub").read_text())
        launcher = root / "force-command.sh"
        write_private(launcher, "#!/bin/sh\nexec " + shlex.join([str(Path(sys.executable).resolve()),
            str(Path(__file__).resolve()), "remote", "--root", str(root), "--mode"]) + ' "$SSH_ORIGINAL_COMMAND"\n')
        launcher.chmod(0o700)
        write_private(root / "sshd_config", "\n".join(["ListenAddress 127.0.0.1", f"Port {port}",
            f"HostKey {keys}/host-key", f"PidFile {root}/sshd.pid", f"AuthorizedKeysFile {keys}/authorized_keys",
            "StrictModes yes", "PasswordAuthentication no", "KbdInteractiveAuthentication no", "UsePAM no",
            "AllowUsers " + getpass.getuser(), "PermitRootLogin no", "AllowTcpForwarding no", "X11Forwarding no",
            "PermitTunnel no", "PrintMotd no", "LogLevel VERBOSE", f"ForceCommand {launcher}"]) + "\n")
        run(["/usr/sbin/sshd", "-t", "-f", root / "sshd_config"], environment, root)
        with (root / "sshd.log").open("xb") as log:
            server = subprocess.Popen(["/usr/sbin/sshd", "-D", "-e", "-f", str(root / "sshd_config")], stdout=log, stderr=log)
        write_private(root / "client_config", "\n".join(["Host parity", " HostName 127.0.0.1",
            " User " + getpass.getuser(), f" Port {port}", f" IdentityFile {keys}/client-key",
            f" UserKnownHostsFile {keys}/known_hosts", " StrictHostKeyChecking yes",
            " IdentitiesOnly yes", " BatchMode yes"]) + "\n")
        ssh = ["ssh", "-F", str(root / "client_config"), "-tt", "parity"]
        time.sleep(.2)
        verification = json.loads(run([*ssh, "verify"], environment, root, timeout=15))
        if verification != {"ssh_authenticated": True, "cli_started": False, "model_inputs": 0}:
            raise ValueError("ssh_zero_input_verification_failed")
        ready = {"agent": args.agent, "version": VERSIONS[args.agent], "cli_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
            "root": str(root), "sshd_pid": server.pid,
            "local_command": shlex.join([str(Path(sys.executable).resolve()), str(Path(__file__).resolve()),
                "remote", "--root", str(root), "--mode", "local"]),
            "ssh_commands": {mode: shlex.join([*ssh, mode]) for mode in MODES if mode != "local"},
            "max_model_inputs_per_case": 2, "automatic_model_inputs": 0, "different_os_host_verified": False,
            "tmux_unicode_layout_verified": False, "product_ssh_ui_verified": False,
            "ssh_zero_input_verification": verification}
        if args.agent == "grok":
            # 原生来源已自动配置；是否收到通知仍由真实 CLI 启动和产品接收回执确认。
            ready.update(native_frontend="standalone", native_hook_activation="managed_global_hook_bridge",
                         automatic_startup_notification_verified=False,
                         production_installer=production_installer)
        write_private(root / "ready.safe.json", json.dumps(ready, ensure_ascii=False, indent=2))
        print(json.dumps({"ready": True, "root": str(root), "agent": args.agent}), flush=True)
        deadline = time.monotonic() + args.lifetime
        while time.monotonic() < deadline and not (root / "stop").exists():
            if server.poll() is not None:
                raise RuntimeError("isolated_sshd_exited")
            time.sleep(.2)
    finally:
        for mode in MODES:
            subprocess.run([str(args.tmux), "-S", str(root / mode / "tmux.sock"), "kill-server"], capture_output=True)
        cleanup["identified_private_native_processes_gone"] = cleanup_native(root, executable)
        if server is not None and server.poll() is None:
            server.terminate()
            server.wait(timeout=10)
        if tunnel is not None:
            cleanup["tunnel_drained"] = tunnel.close()
        if auth_copy is not None:
            auth_copy.unlink(missing_ok=True)
            cleanup["credential_copy_removed"] = not auth_copy.exists()
            cleanup["credential_source_stat_unchanged"] = auth_stat(auth_source) == auth_before
        if keys is not None:
            shutil.rmtree(keys)
            cleanup["ssh_private_keys_removed"] = not keys.exists()
        cleanup["cases"] = {mode: notification_summary((root / mode / "terminal.raw").read_bytes(), args.agent)
            for mode in MODES if (root / mode / "terminal.raw").is_file()}
        cleanup["process_tree_cleanup_verified"] = False
        write_private(root / "summary.safe.json", json.dumps(cleanup, ensure_ascii=False, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("serve", "remote", "summarize"))
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--mode", choices=(*MODES, "verify"))
    parser.add_argument("--agent", choices=VERSIONS)
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--tmux", type=Path)
    parser.add_argument("--worker", type=Path)
    parser.add_argument("--claude-plugin", type=Path)
    parser.add_argument("--auth-source", type=Path)
    parser.add_argument("--installer-test-binary", type=Path)
    parser.add_argument("--lifetime", type=int, default=1800)
    parser.add_argument("--app-log", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.action == "remote":
        remote(args.root.resolve(), args.mode)
    elif args.action == "summarize":
        if args.agent is None or args.app_log is None or args.output is None:
            parser.error("summarize requires --agent, --app-log and --output")
        receipt = product_notification_summary(args.app_log.read_bytes(), args.root.resolve(), args.agent)
        write_private(args.output, json.dumps(receipt, ensure_ascii=False, indent=2))
        print(json.dumps({"agent": args.agent, "output": str(args.output),
            "event_counts": {mode: case["events"] for mode, case in receipt["cases"].items()}}))
    else:
        serve(args)


if __name__ == "__main__":
    main()
