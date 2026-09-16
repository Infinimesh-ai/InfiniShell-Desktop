#!/usr/bin/env python3
"""在无账号私有 HOME 中核对 Claude 插件缓存修补；不提交模型输入。"""

import argparse
import atexit
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
from apply_notification_patch import apply_files, bundle_data, default_bundle, installation, validate_tree
from probe_protocol import Recorder


PLUGIN_ID = "warp@claude-code-warp"
COMMIT = "8c28e936ae51cbb23a1a5657fca2bfd30cf06f12"


def require(value, message):
    if not value:
        raise ValueError(message)


def hashes(root):
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(root.rglob("*")) if path.is_file()}


def clean(value, directory):
    if isinstance(value, dict):
        return {key: clean(item, directory) for key, item in value.items()}
    if isinstance(value, list):
        return [clean(item, directory) for item in value]
    if isinstance(value, str):
        return value.replace(str(directory), "<isolated-probe>")
    return value


def native(executable, args, env, directory, report):
    require(args[0] in ("--version", "plugin"), "拒绝模型执行命令")
    completed = subprocess.run([str(executable), *args], env=env, cwd=directory,
                               capture_output=True, text=True, timeout=150)
    report["commands"].append({"arguments": args, "exit_code": completed.returncode,
                               "stdout": completed.stdout, "stderr": completed.stderr})
    require(completed.returncode == 0, f"原生命令失败：{args}: {completed.stderr}")
    return completed.stdout


def snapshot(home, cache):
    return {"tree_sha256": hashes(cache), "configuration": {
        name: json.loads((home / name).read_text()) for name in (
            "settings.json", "plugins/known_marketplaces.json", "plugins/installed_plugins.json")}}


def initialize(executable, env, project, directory, name, wait_seconds, report):
    # 只发送 initialize；即使握手成功也不发送 user、工具请求或模型提示词。
    debug_path = directory / f"{name}.debug.log"
    trace_path = directory / f"{name}.ndjson"
    command = [str(executable), "--print", "--input-format", "stream-json", "--output-format",
               "stream-json", "--verbose", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
               "--debug-file", str(debug_path)]
    with trace_path.open("w") as output:
        recorder = Recorder(command, env, project, output)
        try:
            recorder.send({"type": "control_request", "request_id": name,
                           "request": {"subtype": "initialize"}})
            response = recorder.until(lambda item: isinstance(item, dict) and
                item.get("type") == "control_response" and
                item.get("response", {}).get("request_id") == name, 30)
            require(response is not None, "没有收到真实 initialize 响应")
            time.sleep(wait_seconds)
        finally:
            recorder.close()
    trace = [json.loads(line) for line in trace_path.read_text().splitlines()]
    require(all(row.get("message", {}).get("type") != "user" for row in trace
                if row["direction"] == "stdin"), "发现意外模型输入")
    debug = debug_path.read_text() if debug_path.exists() else ""
    report["sessions"].append({"stage": name, "wait_seconds": wait_seconds, "events": trace,
        "plugin_debug_lines": [line for line in debug.splitlines()
            if any(term in line.lower() for term in ("plugin", "marketplace", "autoupdat"))]})


def controlled_update(executable, directory, report, wait_seconds):
    """2.2.1-fixture 只属于本地实验仓库，不代表 Warp 发布了新版本。"""
    home = directory / "home"
    config = home / ".claude"
    project = directory / "project"
    for path in (config, project):
        path.mkdir(parents=True)
    env = {key: os.environ[key] for key in ("PATH", "TMPDIR", "TMP", "TEMP", "SYSTEMROOT", "WINDIR") if key in os.environ}
    env.update(HOME=str(home), USERPROFILE=str(home), CLAUDE_CONFIG_DIR=str(config),
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(directory / "empty-gitconfig"),
               GIT_TERMINAL_PROMPT="0", DISABLE_AUTOUPDATER="1", FORCE_AUTOUPDATE_PLUGINS="1", TERM="dumb")
    (directory / "empty-gitconfig").touch()
    (directory / "environment.json").write_text(json.dumps(env, indent=2))
    version = native(executable, ["--version"], env, project, report).strip()
    require(version == "2.1.273 (Claude Code)", "必须使用受测 Claude 2.1.273")
    report["cli_version"] = version
    source = directory / "fixture-source"
    subprocess.run(["git", "clone", "https://github.com/warpdotdev/claude-code-warp.git", str(source)],
                   env=env, capture_output=True, check=True, timeout=150)
    commit = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], env=env, text=True).strip()
    require(commit == COMMIT, "夹具初始来源不在固定提交")
    report["fixture_base_commit"] = commit
    marketplace_path = source / ".claude-plugin/marketplace.json"
    marketplace = json.loads(marketplace_path.read_text())
    marketplace["name"] = "infinishell-claude-update-fixture"
    marketplace_path.write_text(json.dumps(marketplace, indent=2) + "\n")

    def commit_fixture(message):
        subprocess.run(["git", "-C", str(source), "add", "."], env=env, check=True, capture_output=True)
        subprocess.run(["git", "-C", str(source), "-c", "user.name=InfiniShell Probe", "-c",
                        "user.email=probe@example.invalid", "commit", "-m", message],
                       env=env, check=True, capture_output=True)
        return subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], env=env, text=True).strip()

    report["fixture_initial_commit"] = commit_fixture("仅更名本地审计 marketplace")
    public = directory / "public"
    public.mkdir()
    remote = public / "fixture.git"
    subprocess.run(["git", "clone", "--bare", str(source), str(remote)],
                   env=env, check=True, capture_output=True)
    subprocess.run(["git", "--git-dir", str(remote), "update-server-info"],
                   env=env, check=True, capture_output=True)
    # 原生拒绝 file:// marketplace；只公开本次夹具的只读 Git 数据，不公开私有配置。
    class FixtureHandler(BaseHTTPRequestHandler):
        def log_message(self, format_string, *args):
            report.setdefault("fixture_http_requests", []).append(format_string % args)

        def do_GET(self):
            self.git_request()

        def do_POST(self):
            self.git_request()

        def git_request(self):
            url = urlsplit(self.path)
            if not url.path.startswith("/fixture.git/") or ".." in url.path:
                self.send_error(404)
                return
            size = int(self.headers.get("Content-Length", 0))
            if not 0 <= size <= 10 * 1024 * 1024:
                self.send_error(413)
                return
            request_env = dict(env, GIT_PROJECT_ROOT=str(public), GIT_HTTP_EXPORT_ALL="1",
                PATH_INFO=url.path, QUERY_STRING=url.query, REQUEST_METHOD=self.command,
                CONTENT_TYPE=self.headers.get("Content-Type", ""), CONTENT_LENGTH=str(size))
            response = subprocess.run(["git", "http-backend"], env=request_env,
                input=self.rfile.read(size) if size else b"", capture_output=True, timeout=30)
            if response.returncode or b"\r\n\r\n" not in response.stdout:
                self.send_error(500)
                return
            headers, body = response.stdout.split(b"\r\n\r\n", 1)
            self.send_response(200)
            for line in headers.decode().split("\r\n"):
                key, value = line.split(":", 1)
                self.send_header(key.strip(), value.strip())
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    atexit.register(server.shutdown)
    fixture_id = "warp@infinishell-claude-update-fixture"
    source_url = f"http://127.0.0.1:{server.server_port}/fixture.git"
    native(executable, ["plugin", "marketplace", "add", source_url], env, project, report)
    native(executable, ["plugin", "install", fixture_id], env, project, report)
    registry = json.loads((config / "plugins/installed_plugins.json").read_text())
    entry = registry["plugins"][fixture_id][0]
    cache = Path(entry["installPath"])
    require(cache.resolve().is_relative_to(config / "plugins/cache"), "夹具被原地加载，不能验证缓存更新")
    require(entry["version"] == "2.2.0", "夹具初始版本不匹配")
    metadata, replacements = bundle_data(default_bundle(), "claude")
    validate_tree(cache, "2.2.0", metadata)
    apply_files(cache, metadata, replacements)
    report["fixture_patched_before_update"] = snapshot(config, cache)
    # 显式只改私有测试配置，使用原生文档中的 autoUpdate 字段；产品没有此写入。
    for name in ("settings.json", "plugins/known_marketplaces.json"):
        path = config / name
        value = json.loads(path.read_text())
        entry = (value["extraKnownMarketplaces"] if name == "settings.json" else value)[marketplace["name"]]
        entry["autoUpdate"] = True
        path.write_text(json.dumps(value, indent=2) + "\n")
    manifest_path = source / "plugins/warp/.claude-plugin/plugin.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["version"] = "2.2.1-fixture"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    next(plugin for plugin in marketplace["plugins"] if plugin["name"] == "warp")["version"] = "2.2.1-fixture"
    marketplace_path.write_text(json.dumps(marketplace, indent=2) + "\n")
    report["fixture_changed_commit"] = commit_fixture("仅用于审计的新版本 2.2.1-fixture")
    subprocess.run(["git", "-C", str(source), "push", str(remote), "HEAD"],
                   env=env, check=True, capture_output=True)
    subprocess.run(["git", "--git-dir", str(remote), "update-server-info"],
                   env=env, check=True, capture_output=True)
    report["fixture_new_source_tree"] = hashes(source / "plugins/warp")
    initialize(executable, env, project, directory, "fixture_auto_update_startup", wait_seconds, report)
    report["fixture_after_auto_update_window"] = snapshot(config, cache)
    auto_registry = report["fixture_after_auto_update_window"]["configuration"]["plugins/installed_plugins.json"]
    report["automatic_update_observed"] = auto_registry["plugins"][fixture_id][0]["version"] == "2.2.1-fixture"
    native(executable, ["plugin", "marketplace", "update", marketplace["name"]], env, project, report)
    native(executable, ["plugin", "update", fixture_id, "--json"], env, project, report)
    registry = json.loads((config / "plugins/installed_plugins.json").read_text())
    current = registry["plugins"][fixture_id][0]
    new_cache = Path(current["installPath"])
    require(current["version"] == "2.2.1-fixture", "原生 update 未切换到受控新版本")
    report["fixture_after_native_new_version_update"] = snapshot(config, new_cache)
    report["fixture_old_cache_after_native_update"] = hashes(cache)
    before = report["fixture_patched_before_update"]["tree_sha256"]
    after = report["fixture_old_cache_after_native_update"]
    report["observations"] = {
        "different_version_native_update_verified": True,
        "new_active_cache_equals_unpatched_fixture_source": hashes(new_cache) == report["fixture_new_source_tree"],
        "old_cache_original_files_preserved": all(after.get(name) == digest for name, digest in before.items()),
        "old_cache_added_files": sorted(set(after) - set(before)),
        "model_requests": 0,
        "upstream_new_version_used": False,
    }


def run(executable, directory, report, wait_seconds):
    home = directory / "home"
    config = home / ".claude"
    project = directory / "project"
    for path in (config, project):
        path.mkdir(parents=True)
    # 不继承账号、API 密钥、代理认证或用户 Git 配置；禁止更新共享 CLI 二进制。
    env = {key: os.environ[key] for key in ("PATH", "TMPDIR", "TMP", "TEMP", "SYSTEMROOT", "WINDIR") if key in os.environ}
    env.update(HOME=str(home), USERPROFILE=str(home), CLAUDE_CONFIG_DIR=str(config),
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(directory / "empty-gitconfig"),
               GIT_TERMINAL_PROMPT="0", DISABLE_AUTOUPDATER="1", FORCE_AUTOUPDATE_PLUGINS="1", TERM="dumb")
    (directory / "empty-gitconfig").touch()
    (directory / "environment.json").write_text(json.dumps(env, indent=2))
    version = native(executable, ["--version"], env, project, report).strip()
    require(version == "2.1.273 (Claude Code)", "必须使用受测 Claude 2.1.273")
    report["cli_version"] = version
    with executable.open("rb") as source:
        report["executable_sha256"] = hashlib.file_digest(source, "sha256").hexdigest()
    native(executable, ["plugin", "marketplace", "add", "warpdotdev/claude-code-warp"], env, project, report)
    native(executable, ["plugin", "install", PLUGIN_ID], env, project, report)
    marketplace = config / "plugins/marketplaces/claude-code-warp"
    observed_commit = subprocess.check_output(["git", "-C", str(marketplace), "rev-parse", "HEAD"], env=env, text=True).strip()
    require(observed_commit == COMMIT, "原生来源偏离已验证提交，停止固定版本审计")
    report["upstream_commit"] = observed_commit
    metadata, replacements = bundle_data(default_bundle(), "claude")
    cache, plugin_version = installation(config, "claude")
    validate_tree(cache, plugin_version, metadata)
    report["upstream_install"] = snapshot(config, cache)
    apply_files(cache, metadata, replacements)
    report["patched_before_restart"] = snapshot(config, cache)
    initialize(executable, env, project, directory, "default_restart", wait_seconds, report)
    report["after_default_restart"] = snapshot(config, cache)
    native(executable, ["plugin", "marketplace", "update", "claude-code-warp"], env, project, report)
    report["after_marketplace_update"] = snapshot(config, cache)
    native(executable, ["plugin", "update", PLUGIN_ID, "--json"], env, project, report)
    report["after_same_version_plugin_update"] = snapshot(config, cache)
    native(executable, ["plugin", "install", PLUGIN_ID], env, project, report)
    report["after_same_version_plugin_install"] = snapshot(config, cache)
    # 用户显式关闭的插件不能因刷新或再启动而重新启用。
    native(executable, ["plugin", "disable", PLUGIN_ID], env, project, report)
    report["disabled_before_restart"] = snapshot(config, cache)
    initialize(executable, env, project, directory, "disabled_restart", wait_seconds, report)
    native(executable, ["plugin", "update", PLUGIN_ID, "--json"], env, project, report)
    report["disabled_after_restart_and_update"] = snapshot(config, cache)
    try:
        installation(config, "claude")
    except ValueError as error:
        report["product_preflight_disabled_error"] = str(error)
    else:
        raise ValueError("产品预检意外接受已禁用插件")
    report["native_listing"] = json.loads(native(executable, ["plugin", "list", "--json"], env, project, report))
    patched = report["patched_before_restart"]["tree_sha256"]
    report["observations"] = {
        "same_version_cache_preserved": {name: report[name]["tree_sha256"] == patched for name in (
            "after_default_restart", "after_marketplace_update", "after_same_version_plugin_update",
            "after_same_version_plugin_install", "disabled_after_restart_and_update")},
        "disabled_remains_false": report["disabled_after_restart_and_update"]["configuration"]["settings.json"]["enabledPlugins"][PLUGIN_ID] is False,
        "model_requests": 0,
        "different_version_automatic_update_verified": False,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wait-seconds", type=int, default=20)
    parser.add_argument("--controlled-update", action="store_true")
    args = parser.parse_args()
    require(1 <= args.wait_seconds <= 60, "单次握手观察窗口应在 1 至 60 秒")
    directory = Path(tempfile.mkdtemp(prefix="infinishell-claude-cache-native-")).resolve()
    report = {"commands": [], "sessions": [], "scope": "无账号、无模型，真实原生命令与缓存文件"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        action = controlled_update if args.controlled_update else run
        action(args.executable.resolve(), directory, report, args.wait_seconds)
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (directory / "raw-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
        args.output.write_text(json.dumps(clean(report, directory), ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"private_directory": str(directory), "report": str(args.output)}), flush=True)


if __name__ == "__main__":
    main()
