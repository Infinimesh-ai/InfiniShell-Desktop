#!/usr/bin/env python3
"""固定 Grok 生产适配器图片与冷恢复；私有配置和认证副本均限定在本次验收目录。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from prepare_grok_cli import current_platform, verify_binary
from run_grok_official_adapter_live import copy_private_auth
from run_claude_adapter_live import sanitize


TEST = "ai::cli_agent_runtime::grok::managed_image_live_tests::real_grok_managed_image_resume"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def source_binding():
    repository = Path(__file__).resolve().parents[2]
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=repository, text=True)
    files = ['app/src/ai/cli_agent_runtime/grok.rs', 'app/src/ai/cli_agent_runtime/managed_input.rs', 'app/src/ai/cli_agent_runtime/grok_managed_image_live_tests.rs']
    return {"source_commit_is_baseline_only": bool(status.strip()),
            "source_worktree_dirty": bool(status.strip()), "source_status_porcelain": status,
            "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "critical_source_sha256": {name: hashlib.sha256((repository / name).read_bytes()).hexdigest()
                                       for name in files}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--credential-home", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("此认证副本运行器仅适用于 macOS/Linux；Windows 需单独验证原生 ACL 与认证来源。")
    executable = args.executable.resolve(strict=True)
    native = verify_binary(executable, current_platform("1.0.41"), "1.0.41")
    version = subprocess.check_output([str(executable), "--version"], text=True).strip()
    if version != "grok 1.0.41 (4220f3b224a6)":
        raise ValueError("Grok 版本不匹配")
    args.output.mkdir(parents=True, exist_ok=False)
    root = Path(tempfile.mkdtemp(prefix="gri-", dir="/private/tmp" if Path("/private/tmp").is_dir() else None))
    (root / ".infinishell-grok-managed-image-probe").write_text("isolated Grok managed image verification\n")
    for directory in ("home/.grok", "tmp", "project", "state"):
        (root / directory).mkdir(parents=True, mode=0o700)
    settings = root / "home/.grok/config.toml"
    settings.write_text('''[cli]
use_leader = true
auto_update = false
[models]
default = "grok-4.7"
session_summary = "grok-4.7"
web_search = "grok-4.7"
image_description = "grok-4.7"
[features]
turn_summary = false
title_refresh = false
support_permission = true
[[permission.rules]]
action = "ask"
tool = "any"
''')
    settings.chmod(0o600)
    original_settings = args.credential_home / "config.toml"
    original_digest = digest(original_settings) if original_settings.exists() else None
    auth = copy_private_auth(args.credential_home, root / "home/.grok")
    allowed = {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG", "LC_ALL", "LC_CTYPE",
               "USER", "USERNAME", "LOGNAME", "SHELL", "HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY"}
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    environment.update({"HOME": str(root / "home"), "GROK_HOME": str(root / "home/.grok"),
                        "XDG_CONFIG_HOME": str(root / "home/.config"), "XDG_DATA_HOME": str(root / "home/.local/share"),
                        "XDG_CACHE_HOME": str(root / "home/.cache"), "TMPDIR": str(root / "tmp"),
                        "TMP": str(root / "tmp"), "TEMP": str(root / "tmp"),
                        "GROK_DISABLE_API_KEY_AUTH": "1", "GROK_AUTO_UPDATE": "0", "GROK_DISABLE_AUTOUPDATER": "1",
                        "GROK_CLAUDE_HOOKS_ENABLED": "0", "GROK_CLAUDE_MCPS_ENABLED": "0",
                        "GROK_CODEX_HOOKS_ENABLED": "0", "GROK_CODEX_MCPS_ENABLED": "0",
                        "INFINISHELL_GROK_LIVE_ROOT": str(root),
                        "INFINISHELL_GROK_MANAGED_IMAGE_TRACE": "1",
                        "INFINISHELL_GROK_LIVE_EXECUTABLE": str(executable),
                        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor.resolve())})
    receipt = {"scope": "production_adapter_new_and_cold_resume", "native": native,
               "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
               "test_binary_sha256": digest(args.test_binary), "supervisor_sha256": digest(args.supervisor),
               "model": "grok-4.7", "workspace": str(root), "passed": False,
               "auth_file_copied_to_private_directory": True, "auth_values_recorded": False,
               "app_restart_or_gui_verified": False}
    receipt.update(source_binding())
    try:
        process = subprocess.run([str(args.test_binary.resolve()), TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                 capture_output=True, text=True, env=environment, timeout=510)
        output = sanitize(process.stdout + process.stderr, root, None, None)
        (args.output / "test-output.txt").write_text(output)
        events_path = root / "events.ndjson"
        raw = events_path.read_bytes() if events_path.exists() else b""
        (args.output / "events.ndjson").write_bytes(raw)
        events = [json.loads(line) for line in raw.splitlines()]
        ready = [event for event in events if event.get("event") == "ready"]
        prepared = [event for event in events if event.get("event") == "attachment_prepared"]
        results = [event for event in events if event.get("event") == "finished"]
        cleanup = [event for event in events if event.get("event") == "cleanup"]
        projections = [json.loads(line.split("GROK_NATIVE_IMAGE_HISTORY ", 1)[1])
                       for line in output.splitlines() if "GROK_NATIVE_IMAGE_HISTORY " in line]
        (args.output / "native-image-projections.json").write_text(json.dumps(projections, indent=2) + "\n")
        native_images_match = len(projections) == len(results) == len(prepared) == 2
        for result, attachment in zip(results, prepared):
            matches = [projection for projection in projections
                       if projection.get("runtime_generation") == result["generation"]
                       and projection.get("session_id") == result["native_session_id"]
                       and projection.get("turn_id") == result["turn_id"]]
            if len(matches) != 1 or matches[0].get("source") != "verified_native_final_history":
                native_images_match = False
                continue
            images = matches[0].get("images", [])
            native_images_match = native_images_match and len(images) == 1 and (
                images[0].get("image_sha256") == attachment["image_sha256"]
                and images[0].get("image_bytes") == attachment["image_bytes"]
                and images[0].get("mime_type") == "image/png"
                and images[0].get("native_content_sha256") == attachment["native_image_content_sha256"])
        receipt["native_image_history_matches_prepared_bytes"] = native_images_match
        receipt.update(exit_code=process.returncode, event_bytes_sha256=hashlib.sha256(raw).hexdigest())
        receipt["passed"] = (native_images_match and process.returncode == 0 and "1 passed" in output and len(ready) == len(results) == len(cleanup) == len(prepared) == 2
            and ready[0]["native_session_id"] == ready[1]["native_session_id"]
            and ready[0]["generation"] != ready[1]["generation"]
            and prepared[0]["image_sha256"] != prepared[1]["image_sha256"]
            and [event["result"].strip() for event in results] == ["RED BLUE", "GREEN YELLOW"]
            and all(event["cleanup_confirmed"] and event["same_id_replay_did_not_start_another_turn"] for event in cleanup)
            and events[-1].get("event") == "acceptance_passed")
    except subprocess.TimeoutExpired as error:
        receipt["timed_out"] = True
        output = (error.stdout or b"").decode("utf-8", "replace") + (error.stderr or b"").decode("utf-8", "replace")
        (args.output / "test-output.txt").write_text(sanitize(output, root, None, None))
        events_path = root / "events.ndjson"
        if events_path.exists():
            (args.output / "events.ndjson").write_bytes(events_path.read_bytes())
    finally:
        auth.unlink(missing_ok=True)
        receipt["private_auth_copy_removed"] = not auth.exists()
        receipt["original_settings_unchanged"] = (digest(original_settings) if original_settings.exists() else None) == original_digest
        receipt["native_binary_unchanged"] = verify_binary(executable, current_platform("1.0.41"), "1.0.41") == native
        receipt["critical_sources_unchanged_during_run"] = receipt["critical_source_sha256"] == source_binding()["critical_source_sha256"]
        receipt["passed"] = receipt["passed"] and receipt["critical_sources_unchanged_during_run"] and receipt["private_auth_copy_removed"] and receipt["original_settings_unchanged"] and receipt["native_binary_unchanged"]
        (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "passed": receipt["passed"]}))
    if not receipt["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
