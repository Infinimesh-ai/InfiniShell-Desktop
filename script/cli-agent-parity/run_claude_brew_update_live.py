#!/usr/bin/env python3
"""Claude 私有 cask 后端事务：官方固定映像，人工登记，零模型、零 brew/Ruby 执行。"""
import argparse
import hashlib
import json
import mmap
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import tempfile

SCOPE = "claude_brew_backend_no_model_v1"
MARKER = b"InfiniShell private cask backend fixture; synthetic registration; no credentials\n"
OLD, TARGET, TOKEN = "2.1.278", "2.1.280", "claude-code@latest"
CASES = ("updated", "swap_receipt_missing", "external_change_preserved", "candidate_changed_preserved")
TEST = "terminal::cli_agent_updates::sources::brew_transaction::live_tests::real_claude_brew_backend_without_model"
VOLUMES_ROOT = Path("/Volumes")
ENV_PATHS = {"HOME":"home", "USERPROFILE":"home", "CLAUDE_CONFIG_DIR":"home/.claude", "CODEX_HOME":"home/.codex",
             "GROK_HOME":"home/.grok", "XDG_CONFIG_HOME":"home/.config", "XDG_CACHE_HOME":"home/.cache",
             "XDG_DATA_HOME":"home/.local/share", "XDG_STATE_HOME":"home/.local/state", "TMPDIR":"tmp", "TMP":"tmp", "TEMP":"tmp"}
SOURCE_FILES = (
    "app/src/terminal/cli_agent_updates.rs",
    *["app/src/terminal/cli_agent_updates/" + name for name in (
        "sources.rs", "sources_brew.rs", "sources_brew_transaction.rs", "sources_npm_tree_unix.rs", "sources_brew_transaction_tests.rs", "sources_claude_brew_live_tests.rs")],
    *["app/src/ai/cli_agent_runtime/" + name for name in (
        "managed_process.rs", "managed_process_version_probe.rs", "managed_process_macos.rs", "managed_process_atomic_macos.rs",
        "managed_process_atomic_linux.rs", "managed_process_atomic_linux_glibc.rs")],
    "script/cli-agent-parity/claude_21280_brew_manifest.json", "script/cli-agent-parity/run_claude_brew_update_live.py",
)
CONTRACT_PATH = Path(__file__).with_name("claude_21280_brew_manifest.json")


def require(value, reason):
    if not value:
        raise ValueError(reason)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    raw = value if isinstance(value, bytes) else (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())


def regular(path):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and path.resolve(strict=True) == path,
            "regular_canonical_file_required")
    return info


def binding(path, expected):
    regular(path)
    actual = sha(path)
    require(re.fullmatch(r"[0-9a-f]{64}", expected) and actual == expected, "binary_sha_mismatch")
    return {"path":str(path), "sha256":actual}


def canonical_execution_path(path, *, strict):
    canonical = path.resolve(strict=strict)
    if canonical.is_relative_to(VOLUMES_ROOT) or not VOLUMES_ROOT.exists():
        return canonical
    for ancestor in (canonical, *canonical.parents):
        if ancestor.exists() and ancestor.samefile(VOLUMES_ROOT):
            return VOLUMES_ROOT / canonical.relative_to(ancestor)
    return canonical


def execution_volume(path):
    if path is None:
        return None
    require(platform.system() == "Darwin", "external_volume_requires_macos")
    volume = canonical_execution_path(path, strict=True)
    require(volume.parent == VOLUMES_ROOT and volume.is_dir() and os.path.ismount(volume), "external_volume_not_mounted_root")
    return {"path":str(volume), "st_dev":volume.stat().st_dev}


def verify_execution_path(path, volume, *, output=False):
    canonical = canonical_execution_path(path, strict=not output)
    if volume is None:
        require(not canonical.is_relative_to(VOLUMES_ROOT), "external_execution_requires_explicit_volume")
    elif output or canonical.is_relative_to(VOLUMES_ROOT):
        root = Path(volume["path"])
        require(canonical.is_relative_to(root), "execution_path_outside_allowed_volume")
        require(os.path.ismount(root) and root.stat().st_dev == volume["st_dev"], "execution_volume_changed")
        existing = canonical
        while not existing.exists():
            existing = existing.parent
        while existing != root:
            require(existing.stat().st_dev == volume["st_dev"] and not os.path.ismount(existing), "execution_volume_device_mismatch")
            existing = existing.parent
    return canonical


def derived_metadata(contract):
    require(contract.get("schema") == 1 and contract.get("scope") == SCOPE
            and (contract.get("old"), contract.get("target"), contract.get("token")) == (OLD, TARGET, TOKEN), "contract_identity")
    metadata = contract["metadata"]
    value = metadata["value"]
    require(metadata["kind"] == "derived_fixture" and value["token"] == TOKEN and value["version"] == TARGET,
            "derived_metadata_identity")
    raw = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    require(len(raw) == metadata["length"] and hashlib.sha256(raw).hexdigest() == metadata["sha256"], "derived_metadata_digest")
    return raw


def official_inputs(cache, contract, output):
    """只从五份固定原件构造明确标记的派生 JSON；绝不改写实时 API 响应。"""
    inputs = {}
    expected = contract["official_inputs"]
    require(len(expected) == 5, "original_input_set")
    for url, item in expected.items():
        require(Path(item["file"]).name == item["file"], "cache_path_escape")
        path = cache / item["file"]
        info = regular(path)
        require(info.st_size == item["length"] and sha(path) == item["sha256"], "original_input_digest")
        inputs[url] = {"path":str(path), "sha256":item["sha256"]}
    for version in (OLD, TARGET):
        url = f"https://downloads.claude.ai/claude-code-releases/{version}/"
        manifest = json.loads(Path(inputs[url + "manifest.json"]["path"]).read_bytes())
        native = expected[url + "darwin-arm64/claude"]
        require(manifest["version"] == version and manifest["platforms"]["darwin-arm64"] == {
            "binary":"claude", "checksum":native["sha256"], "size":native["length"]}, "release_manifest_native_binding")
    raw = derived_metadata(contract)
    value = contract["metadata"]["value"]
    # 整份 Ruby 先以固定摘要确认；这里只接受已人工审查的五个字段，不求值 Ruby。
    cask_url = "https://raw.githubusercontent.com/Homebrew/homebrew-cask/" + value["tap_git_head"] + "/Casks/c/claude-code@latest.rb"
    ruby = Path(inputs[cask_url]["path"]).read_text()
    require('cask "claude-code@latest" do' in ruby and 'version "2.1.280"' in ruby
            and 'binary "claude"' in ruby and ('sha256 arm:          "' + value["sha256"] + '"') in ruby
            and value["sha256"] == expected[value["url"]]["sha256"], "fixed_cask_fields")
    destination = output / "derived-cask.fixture.json"
    write(destination, raw)
    inputs[contract["metadata"]["url"]] = {"path":str(destination), "sha256":contract["metadata"]["sha256"]}
    write(output / "derived-cask.safe.json", {"kind":"derived_fixture", "sha256":sha(destination), "source_ruby":inputs[cask_url],
        "historical_api_original":False, "ruby_executed":False, "boundary":contract["boundary"]})
    return inputs


def environment(root, manifest, step):
    env = {name:str(root / relative) for name, relative in ENV_PATHS.items()}
    env.update(PATH="/usr/bin:/bin:/usr/sbin:/sbin", LANG="C", LC_ALL="C", DISABLE_AUTOUPDATER="1",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", INFINISHELL_CLAUDE_BREW_ALLOW=SCOPE,
        INFINISHELL_CLAUDE_BREW_STEP=step, INFINISHELL_CLAUDE_BREW_MANIFEST=str(root / "manifest.private.json"),
        INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=manifest["supervisor"]["path"])
    return env


def fixture(output, case, sources, binaries, inputs, contract):
    root = Path(tempfile.mkdtemp(prefix="infinishell-claude-brew-live-", dir=output)).resolve()
    for relative in set(ENV_PATHS.values()) | {"project", "state", "prefix/bin", "prefix/var/homebrew/locks"}:
        (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
    write(root / ".infinishell-cask-live", MARKER)
    write(root / "prefix/bin/brew", b"synthetic manager identity; never execute\n")
    (root / "prefix/bin/brew").chmod(0o400)
    package = root / "prefix/Caskroom" / TOKEN
    native = package / OLD / "claude"
    native.parent.mkdir(mode=0o755, parents=True)
    source = inputs[f"https://downloads.claude.ai/claude-code-releases/{OLD}/darwin-arm64/claude"]["path"]
    shutil.copyfile(source, native)
    native.chmod(0o750)
    metadata = package / ".metadata"
    metadata.mkdir(mode=0o755)
    casks = metadata / OLD / "20260928000000.000" / "Casks"
    casks.mkdir(mode=0o755, parents=True)
    write(casks / (TOKEN + ".json"), b"{}\n")
    write(metadata / "config.json", {"default_bottle_domain":"https://ghcr.io/v2/homebrew/core", "synthetic_fixture":True})
    write(metadata / "INSTALL_RECEIPT.json", {"homebrew_version":"7.0.4", "runtime_dependencies":{}, "uninstall_flight_blocks":False,
        "source":{"tap":"homebrew/cask", "version":OLD, "tap_git_head":None},
        "uninstall_artifacts":contract["metadata"]["value"]["artifacts"], "time":0})
    write(root / "home/.claude/settings.json", b'{"autoUpdatesChannel":"latest","fixture":"preserve"}\n')
    os.symlink("../Caskroom/" + TOKEN + "/" + OLD + "/claude", root / "prefix/bin/claude")
    manifest = dict(schema=1, scope=SCOPE, case=case, root=str(root), source_sha256=sources, inputs=inputs, **binaries)
    write(root / "manifest.private.json", manifest)
    write(root / "fixture-provenance.safe.json", {"registration":"synthetic_private_fixture", "brew_installed":False,
        "old_release":OLD,"target_release":TARGET,"token":TOKEN,"homebrew_version_field":"7.0.4 fixture contract, not executed brew",
        "consumer_source_discovery_covered":False,"real_package_manager_installation_covered":False})
    return root, manifest


def verify_embedded(binary, repo, *, worker):
    paths = SOURCE_FILES if worker else [name for name in SOURCE_FILES if not name.endswith("_tests.rs") and not name.startswith("script/")]
    with Path(binary).open("rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
        for name in paths:
            require(data.find((repo / name).read_bytes()) >= 0, "embedded_source_mismatch:" + name)


def run_test(binary, root, manifest, step):
    command = [binary, "--exact", TEST, "--ignored", "--nocapture", "--test-threads=1"]
    try:
        result = subprocess.run(command, cwd=root / "project", env=environment(root, manifest, step),
                                capture_output=True, timeout=900, check=False)
    except subprocess.TimeoutExpired as error:
        write(root / (step + ".stdout"), error.stdout or b"")
        write(root / (step + ".stderr"), error.stderr or b"")
        write(root / (step + ".timeout.safe.json"), {"timeout":True,"model_inputs_sent":0,"cleanup_confirmed":False,"descendant_cleanup":"unknown; stop and review retained receipts"})
        raise
    write(root / (step + ".stdout"), result.stdout)
    write(root / (step + ".stderr"), result.stderr)
    write(root / (step + ".safe.json"), {"returncode":result.returncode,"command":command,"model_inputs_sent":0})
    require(result.returncode == 0 and b"1 passed" in result.stdout, "product_test_failed")
    return json.loads((root / ("result-" + step + ".safe.json")).read_bytes())


def verify_snapshot(path, expected):
    """独立按真实 inode、权限及所有成员摘要核对 Rust 完整快照。"""
    def identity(item):
        metadata = item.lstat()
        require(not item.is_symlink() and metadata.st_uid == os.geteuid(), "snapshot_member_type")
        return {"device":metadata.st_dev, "inode":metadata.st_ino, "uid":metadata.st_uid,
                "gid":metadata.st_gid, "mode":metadata.st_mode}
    require(identity(path) == expected["root"], "snapshot_root")
    actual = {}
    for item in path.rglob("*"):
        info = item.lstat()
        require(stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode), "snapshot_special_member")
        actual[item.relative_to(path).as_posix()] = {"identity":identity(item), "length":info.st_size,
            "sha256":list(bytes.fromhex(sha(item))) if item.is_file() else None}
    # Rust 目录的 length 固定为零；文件保持真实长度。
    for name, node in actual.items():
        if node["sha256"] is None:
            node["length"] = 0
    require(actual == expected["nodes"], "snapshot_complete_tree")


def independent_check(root, case, contract):
    package = root / "prefix/Caskroom" / TOKEN
    snapshot = json.loads((root / "after-tree.safe.json").read_bytes())
    verify_snapshot(package, snapshot)
    expected = TARGET if case in ("updated", "external_change_preserved") else OLD
    public = root / "prefix/bin/claude"
    require(public.is_symlink() and public.resolve(strict=True) == package / expected / "claude", "public_link_target")
    native = contract["official_inputs"][f"https://downloads.claude.ai/claude-code-releases/{expected}/darwin-arm64/claude"]
    require(sha(public) == native["sha256"] and public.stat().st_size == native["length"], "public_native_digest")
    require((package / ".metadata/config.json").read_bytes() == (root / "before-cask-config.raw").read_bytes()
            and (root / "home/.claude/settings.json").read_bytes() == (root / "before-user-config.raw").read_bytes(), "preserved_configs")
    generation_root = root / "state/cli-agent-processes"
    generations = list(generation_root.iterdir()) if generation_root.exists() else []
    if case == "candidate_changed_preserved":
        require(not generations, "rejected_candidate_started")
        checkpoint = json.loads((root / "checkpoint-journal.safe.json").read_bytes())
        require(checkpoint["probe"] is None, "rejected_candidate_probe")
        stage = package.parent / (".infinishell-brew-" + checkpoint["id"])
        verify_snapshot(stage, json.loads((root / "mutated-stage.safe.json").read_bytes()))
    else:
        require(len(generations) == 1, "candidate_generation_count")
        receipt = json.loads((generations[0] / "exit.json").read_bytes())
        require(receipt["cleanup_confirmed"] is True and receipt["exit_code"] == 0, "candidate_exit_receipt")
    if case == "external_change_preserved":
        require((package / "external-cask-change").read_bytes() == b"external cask mutation must survive recovery\n", "external_marker")
        checkpoint = json.loads((root / "checkpoint-journal.safe.json").read_bytes())
        verify_snapshot(package.parent / (".infinishell-brew-" + checkpoint["id"]), json.loads((root / "before-tree.safe.json").read_bytes()))
    if case in ("external_change_preserved", "candidate_changed_preserved"):
        require((root / "state/claude-homebrew.json").read_bytes() == (root / "checkpoint-journal.safe.json").read_bytes(), "journal_bytes_changed")
    else:
        require(not (root / "state/claude-homebrew.json").exists(), "journal_not_removed")
    write(root / "independent-archive-check.safe.json", {"accepted":True,"complete_tree":True,"public_image_digest":native["sha256"],
        "candidate_generations":len(generations),"public_version_expected":expected,"registration":"synthetic_private_fixture",
        "real_homebrew_installation_covered":False,"consumer_source_discovery_covered":False})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repo", "output", "official-inputs", "test-binary", "supervisor"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("test-binary", "supervisor"):
        parser.add_argument("--" + name + "-sha256", required=True)
    parser.add_argument("--allow-external-volume", type=Path)
    parser.add_argument("--case", choices=CASES, action="append")
    args = parser.parse_args()
    require(platform.system() == "Darwin" and platform.machine() in ("arm64", "aarch64"), "macos_arm64_only")
    repo = args.repo.resolve(strict=True)
    cache = args.official_inputs.resolve(strict=True)
    volume = execution_volume(args.allow_external_volume)
    require(not args.output.exists() and not args.output.is_symlink(), "output_must_be_new")
    output = verify_execution_path(args.output, volume, output=True)
    binaries = {role:binding(verify_execution_path(getattr(args, arg), volume), getattr(args, arg + "_sha256"))
                for role, arg in (("worker", "test_binary"), ("supervisor", "supervisor"))}
    source = {name:sha(repo / name) for name in SOURCE_FILES}
    require(source[SOURCE_FILES[-1]] == sha(Path(__file__).resolve()), "runner_source_binding")
    verify_embedded(binaries["worker"]["path"], repo, worker=True)
    verify_embedded(binaries["supervisor"]["path"], repo, worker=False)
    output.mkdir(mode=0o700, parents=True)
    require(shutil.disk_usage(output).free >= 4 * 1024**3, "fixture_disk_space")
    require(verify_execution_path(output, volume, output=True) == output, "output_path_changed")
    signature = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", binaries["supervisor"]["path"]], capture_output=True, check=False)
    write(output / "supervisor-signature.stdout", signature.stdout)
    write(output / "supervisor-signature.stderr", signature.stderr)
    require(signature.returncode == 0, "supervisor_signature")
    contract = json.loads((repo / "script/cli-agent-parity/claude_21280_brew_manifest.json").read_bytes())
    inputs = official_inputs(cache, contract, output)
    revision = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    write(output / "source.safe.json", {"commit":revision,"source_sha256":source,"binaries":binaries,"inputs":inputs,
        "metadata_kind":"derived_fixture","real_homebrew_installation_covered":False,"consumer_source_discovery_covered":False,"execution_volume":volume})
    results = []
    for case in args.case or CASES:
        root, manifest = fixture(output, case, source, binaries, inputs, contract)
        result = run_test(binaries["worker"]["path"], root, manifest, "execute")
        if case in ("swap_receipt_missing", "external_change_preserved"):
            require(result.get("needs_cold_recovery") is True and result.get("accepted") is False, "checkpoint_not_observed")
            result = run_test(binaries["worker"]["path"], root, manifest, "recover")
        require(result.get("accepted") is True, "case_not_accepted")
        independent_check(root, case, contract)
        results.append({"case":case,"root":str(root),"result":result})
    require(all(sha(repo / name) == digest for name, digest in source.items()), "source_changed_during_run")
    require(all(sha(item["path"]) == item["sha256"] for item in [*binaries.values(), *inputs.values()]), "frozen_input_changed")
    require(verify_execution_path(output, volume, output=True) == output, "output_changed_during_run")
    for item in binaries.values():
        require(str(verify_execution_path(Path(item["path"]), volume)) == item["path"], "binary_path_changed")
    write(output / "summary.safe.json", {"scope":SCOPE,"accepted":True,"cases":results,"model_inputs_sent":0,
        "real_homebrew_installation_covered":False,"consumer_source_discovery_covered":False,
        "busy_plugin_runtime_covered":False,"g09_closed":False,"metadata_kind":"derived_fixture","execution_volume":volume})


if __name__ == "__main__":
    main()
