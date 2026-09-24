#!/usr/bin/env python3
"""在无账号私有 HOME 中验证固定 Claude 插件升级与补丁失败；不提交模型输入。"""

import argparse
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
import apply_notification_patch as patch
from probe_claude_plugin_cache_refresh import clean, hashes, native, require, snapshot


OLD_COMMIT = "bb6c1cf2f5cd7eb609678a2b175e7f4504ca61c2"
NEW_COMMIT = "8c28e936ae51cbb23a1a5657fca2bfd30cf06f12"
PLUGIN_ID = "warp@claude-code-warp"
RUST_TEST = "terminal::cli_agent_sessions::plugin_manager::claude::tests::real_claude_plugin_transaction"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--claude-version", choices=("2.1.273", "2.1.280"), default="2.1.273")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--rust-case", choices=("upgrade", "patch-failure", "crash-retry"), default="upgrade")
    args = parser.parse_args()
    directory = Path(tempfile.mkdtemp(prefix="infinishell-claude-upgrade-transaction-")).resolve()
    report = {"commands": [], "scope": {"model_requests": 0, "credentials_copied": False,
        "rust_installer_executed": False, "fixed_versions": ["2.1.0", "2.2.0"]}}
    try:
        execute(args, directory, report)
    except BaseException as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
        if isinstance(error, (subprocess.TimeoutExpired, subprocess.CalledProcessError)):
            report["failure"].update(command=error.cmd,
                stdout=output_text(error.stdout), stderr=output_text(error.stderr))
        raise
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(clean(report, directory), ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"private_directory": str(directory), "report": str(args.output)}))


def output_text(value):
    return value.decode(errors="replace") if isinstance(value, bytes) else value or ""


def execute(args, directory, report):
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

    def git(arguments):
        return subprocess.run(["git", *arguments], env=env, capture_output=True, text=True,
                              check=True, timeout=120).stdout.strip()

    version = native(args.executable, ["--version"], env, project, report).strip()
    require(version == f"{args.claude_version} (Claude Code)"
            and (args.claude_version != "2.1.280" or sys.platform == "darwin"),
            "CLI 版本或平台不符合显式选择的固定通知契约")
    report.update(cli_version=version, executable_sha256=hashlib.sha256(args.executable.read_bytes()).hexdigest())
    public = directory / "public"
    public.mkdir()
    remote = public / "fixture.git"
    git(["clone", "--bare", "https://github.com/warpdotdev/claude-code-warp.git", str(remote)])
    for commit, version in ((OLD_COMMIT, "2.1.0"), (NEW_COMMIT, "2.2.0")):
        manifest = json.loads(git(["--git-dir", str(remote), "show", f"{commit}:plugins/warp/.claude-plugin/plugin.json"]))
        require(manifest["version"] == version, "固定提交版本不匹配")
    git(["--git-dir", str(remote), "update-ref", "refs/heads/parity-probe", OLD_COMMIT])
    git(["--git-dir", str(remote), "symbolic-ref", "HEAD", "refs/heads/parity-probe"])
    gate_requested = threading.Event()
    gate_entered = threading.Event()
    gate_release = threading.Event()
    gate_lock = threading.Lock()

    class FixtureHandler(BaseHTTPRequestHandler):
        def log_message(self, format_string, *arguments):
            report.setdefault("loopback_requests", []).append(format_string % arguments)

        def do_GET(self):
            self.git_request()

        def do_POST(self):
            self.git_request()

        def git_request(self):
            url = urlsplit(self.path)
            size = int(self.headers.get("Content-Length", 0))
            if not url.path.startswith("/fixture.git/") or ".." in url.path or not 0 <= size <= 10 * 1024 * 1024:
                self.send_error(404)
                return
            with gate_lock:
                gated = gate_requested.is_set() and not gate_entered.is_set()
                if gated:
                    gate_entered.set()
            if gated and not gate_release.wait(180):
                self.send_error(504)
                return
            request_env = dict(env, GIT_PROJECT_ROOT=str(public), GIT_HTTP_EXPORT_ALL="1",
                PATH_INFO=url.path, QUERY_STRING=url.query, REQUEST_METHOD=self.command,
                CONTENT_TYPE=self.headers.get("Content-Type", ""), CONTENT_LENGTH=str(size))
            result = subprocess.run(["git", "http-backend"], env=request_env,
                input=self.rfile.read(size) if size else b"", capture_output=True, timeout=30)
            if result.returncode or b"\r\n\r\n" not in result.stdout:
                self.send_error(500)
                return
            headers, body = result.stdout.split(b"\r\n\r\n", 1)
            self.send_response(200)
            for line in headers.decode().split("\r\n"):
                key, value = line.split(":", 1)
                self.send_header(key.strip(), value.strip())
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        source_url = f"http://127.0.0.1:{server.server_port}/fixture.git"
        if args.test_binary:
            # 固定提交经只读回环 Git 提供；原生登记仍使用官方 GitHub source 结构。
            for url in ("https://github.com/warpdotdev/claude-code-warp.git", "git@github.com:warpdotdev/claude-code-warp.git"):
                git(["config", "--file", str(directory / "empty-gitconfig"), "--add", f"url.{source_url}.insteadOf", url])
        native(args.executable, ["plugin", "marketplace", "add", "warpdotdev/claude-code-warp" if args.test_binary else source_url], env, project, report)
        native(args.executable, ["plugin", "install", PLUGIN_ID], env, project, report)
        registry_path = config / "plugins/installed_plugins.json"
        old_entry = json.loads(registry_path.read_text())["plugins"][PLUGIN_ID][0]
        require(old_entry["version"] == "2.1.0", "旧版本安装失败")
        old_cache = Path(old_entry["installPath"])
        metadata, replacements = patch.bundle_data(patch.default_bundle(), "claude")
        patch.validate_tree(old_cache, "2.1.0", metadata)
        report["before_native_update"] = snapshot(config, old_cache)
        git(["--git-dir", str(remote), "update-ref", "refs/heads/parity-probe", NEW_COMMIT])
        if args.test_binary:
            binary = args.test_binary.resolve(strict=True)
            cli_bin = directory / "bin"
            cli_bin.mkdir()
            (cli_bin / "claude").symlink_to(args.executable.resolve(strict=True))
            env["PATH"] = str(cli_bin) + os.pathsep + env.get("PATH", "")
            env["INFINISHELL_CLAUDE_TRANSACTION_ROOT"] = str(directory)
            env["INFINISHELL_CLAUDE_TRANSACTION_CASE"] = "upgrade" if args.rust_case == "crash-retry" else args.rust_case
            (directory / "private-no-model-fixture").write_text("claude-plugin-transaction\n")
            report["scope"]["rust_installer_executed"] = True
            report["rust_test"] = {"name": RUST_TEST, "case": args.rust_case,
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
            test_command = [str(binary), "--exact", RUST_TEST, "--ignored", "--nocapture", "--test-threads=1"]

            def run_test(label):
                try:
                    result = subprocess.run(test_command, env=env, cwd=project, capture_output=True, text=True, timeout=180)
                except subprocess.TimeoutExpired as error:
                    report["rust_test"][label] = {"timed_out":True, "timeout_seconds":error.timeout,
                        "stdout":output_text(error.stdout), "stderr":output_text(error.stderr)}
                    raise
                report["rust_test"][label] = {"exit_code":result.returncode, "stdout":result.stdout, "stderr":result.stderr}
                require(result.returncode == 0 and "1 passed; 0 failed" in result.stdout,
                        "真实 Rust 测试未通过，不能将零测试计为成功")

            if args.rust_case == "crash-retry":
                before_registry = registry_path.read_bytes()
                first_log = directory / "killed-rust-parent.log"
                gate_requested.set()
                with first_log.open("w") as output:
                    worker = subprocess.Popen(test_command, env=env, cwd=project, stdout=output, stderr=subprocess.STDOUT)
                    try:
                        require(gate_entered.wait(45), "没有进入真实暂存原生命令，拒绝盲杀")
                        stage_base = config / "plugins/infinishell-claude-staging"
                        before_stages = set(stage_base.iterdir())
                        require(len(before_stages) == 1, "首轮暂存目录不唯一")
                        require(registry_path.read_bytes() == before_registry, "暂存期间真实 registry 被提前改写")
                        worker.kill()
                        worker.wait(10)
                        report["rust_test"]["killed_parent_exit_code"] = worker.returncode
                        # 第一条原生 Git 请求仍被阻塞，新进程必须能用独立目录完成操作。
                        run_test("retry_while_original_request_blocked")
                        new_cache, _ = patch.installation(config, "claude")
                        after_retry = snapshot(config, new_cache)
                        gate_release.set()
                        time.sleep(3)
                        require(snapshot(config, new_cache) == after_retry, "旧原生命令释放后改写了新发布状态")
                        require(len(set(stage_base.iterdir()) - before_stages) == 1, "重试复用了旧暂存目录")
                        report["rust_test"]["crash_isolation"] = {"original_git_request_held_during_retry":True,
                            "old_native_process_liveness_verified":False,
                            "distinct_new_stage":True, "old_stage_preserved":all(path.exists() for path in before_stages),
                            "published_state_unchanged_after_release":True,
                            "observation_after_release_seconds":3,
                            "phase":"staged native marketplace clone; not registry publication"}
                    finally:
                        gate_release.set()
                        try:
                            if worker.poll() is None:
                                worker.kill()
                                worker.wait(10)
                        finally:
                            output.flush()
                            report["rust_test"]["killed_parent_output"] = first_log.read_text(errors="replace")
            else:
                run_test("run")
            report["rust_test"]["assertions"] = json.loads((directory / f"rust-{env['INFINISHELL_CLAUDE_TRANSACTION_CASE']}-result.json").read_text())
            entry = json.loads(registry_path.read_text())["plugins"][PLUGIN_ID][0]
            report["after_rust_test"] = snapshot(config, Path(entry["installPath"]))
            return
        native(args.executable, ["plugin", "marketplace", "update", "claude-code-warp"], env, project, report)
        native(args.executable, ["plugin", "update", PLUGIN_ID, "--json"], env, project, report)
        new_cache, version = patch.installation(config, "claude")
        patch.validate_tree(new_cache, version, metadata)
        report["after_native_update"] = snapshot(config, new_cache)
        registry_after_update = registry_path.read_bytes()
        original_tree = hashes(new_cache)
        atomic_write = patch.atomic_write
        writes = 0

        def fail_third_write(path, contents, mode, *, staging_dir=None):
            nonlocal writes
            writes += 1
            if writes == 3:
                raise OSError("独立探针注入：第三次受控替换失败")
            atomic_write(path, contents, mode, staging_dir=staging_dir)

        patch.atomic_write = fail_third_write
        try:
            patch.apply_files(new_cache, metadata, replacements)
            raise AssertionError("注入的补丁失败未被观察到")
        except OSError as error:
            report["patch_failure"] = str(error)
        finally:
            patch.atomic_write = atomic_write
        require(hashes(new_cache) == original_tree, "受控文件未恢复为新版本原始内容")
        require(registry_path.read_bytes() == registry_after_update, "现有补丁不应修改原生登记")
        report["after_failed_patch"] = snapshot(config, new_cache)
        report["old_cache_after_update"] = hashes(old_cache)
        report["observed_failure"] = {"active_version_after_patch_failure": version,
            "old_active_version_restored": False, "new_cache_controlled_files_rolled_back": True,
            "old_cache_orphan_marker": (old_cache / ".orphaned_at").exists()}
        patch.apply_files(new_cache, metadata, replacements)
        report["after_explicit_retry"] = snapshot(config, new_cache)
        require(all(hashes(new_cache)[name] == metadata["files"][name]["replacement_sha256"]
                    for name in replacements), "明确重试后补丁不完整")
        native(args.executable, ["plugin", "disable", PLUGIN_ID], env, project, report)
        disabled_settings = (config / "settings.json").read_bytes()
        native(args.executable, ["plugin", "update", PLUGIN_ID, "--json"], env, project, report)
        require((config / "settings.json").read_bytes() == disabled_settings, "禁用配置被改写")
        report["disabled_native_update_preserved"] = True
        report["source_commits"] = {"2.1.0": OLD_COMMIT, "2.2.0": NEW_COMMIT}
    finally:
        gate_release.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
