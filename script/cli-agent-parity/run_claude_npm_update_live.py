#!/usr/bin/env python3
"""固定 Claude npm 私有安装的零模型产品事务验收；只读 npm prefix，不运行安装脚本。"""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

SCOPE = "claude_npm_transaction_no_model_v1"
MARKER = b"InfiniShell private npm transaction fixture; no credentials\n"
OLD, TARGET = "2.1.278", "2.1.280"
CASES = ("updated", "swap_receipt_missing", "external_change_preserved", "candidate_changed_preserved", "unreviewed_downgrade_rejected")
TEST = "terminal::cli_agent_updates::sources::npm_transaction::live_tests::real_claude_npm_update_without_model"
BUSY_TESTS = (
    "terminal::cli_agent_updates::tests::manual_update_still_obeys_busy_and_single_operation_guards",
    "terminal::cli_agent_updates::tests::pty_session_events_defer_update_until_last_local_session_exits",
)
SOURCE_FILES = (
    "app/src/terminal/cli_agent_updates.rs",
    *["app/src/terminal/cli_agent_updates/" + name for name in (
        "sources.rs", "sources_npm.rs", "sources_npm_release.rs", "sources_npm_transaction.rs", "sources_npm_tree_unix.rs", "sources_npm_live_tests.rs")],
    *["app/src/ai/cli_agent_runtime/" + name for name in (
        "managed_process.rs", "managed_process_version_probe.rs", "managed_process_atomic_macos.rs", "managed_process_atomic_linux.rs", "managed_process_atomic_linux_glibc.rs")],
    "script/cli-agent-parity/run_claude_npm_update_live.py",
)
MAX_ARCHIVE = 512 * 1024 * 1024
MAX_EXPANDED = 1024 * 1024 * 1024
ENV_PATHS = {
    "HOME":"home", "USERPROFILE":"home", "CLAUDE_CONFIG_DIR":"home/.claude", "CODEX_HOME":"home/.codex", "GROK_HOME":"home/.grok",
    "XDG_CONFIG_HOME":"home/.config", "XDG_CACHE_HOME":"home/.cache", "XDG_DATA_HOME":"home/.local/share", "XDG_STATE_HOME":"home/.local/state",
    "TMPDIR":"tmp", "TMP":"tmp", "TEMP":"tmp",
}


def require(value, reason):
    if not value:
        raise ValueError(reason)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    raw = value if isinstance(value, bytes) else (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())


def get(url, maximum):
    require(url.startswith("https://registry.npmjs.org/"), "registry_not_official")
    with urllib.request.urlopen(url, timeout=120) as response:
        require(response.url == url, "registry_redirect")
        raw = response.read(maximum + 1)
    require(0 < len(raw) <= maximum, "registry_size")
    return raw


def archive_members(raw, integrity):
    require(re.fullmatch(r"sha512-[A-Za-z0-9+/]+={0,2}", integrity), "sri_not_sha512")
    expected = base64.b64decode(integrity[7:], validate=True)
    require(len(expected) == 64 and hashlib.sha512(raw).digest() == expected, "sri_mismatch")
    result, total = {}, 0
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
        for member in archive:
            name = member.name
            require(member.isfile() and not member.pax_headers, "archive_non_regular")
            require(name.startswith("package/") and "\\" not in name and ":" not in name and "\x00" not in name, "archive_path")
            parts = name.split("/")
            require(all(part and part not in (".", "..") for part in parts), "archive_escape")
            relative = PurePosixPath(*parts[1:]).as_posix()
            require(relative not in result and len(result) < 256, "archive_duplicate_or_limit")
            total += member.size
            require(total <= MAX_EXPANDED and 0 <= member.size <= MAX_EXPANDED, "expanded_limit")
            content = archive.extractfile(member).read(member.size + 1)
            require(len(content) == member.size, "member_length")
            result[relative] = (content, bool(member.mode & 0o111))
    require("package.json" in result, "package_manifest_missing")
    return result


def official_package(package, version, cache):
    name = package.rsplit("/", 1)[1]
    slug = package.replace("@", "").replace("/", "-") + "-" + version
    metadata_bytes = get("https://registry.npmjs.org/" + package + "/" + version, 1024 * 1024)
    metadata = json.loads(metadata_bytes)
    require(metadata.get("name") == package and metadata.get("version") == version, "registry_identity")
    url = metadata["dist"]["tarball"]
    require(url == "https://registry.npmjs.org/" + package + "/-/" + name + "-" + version + ".tgz", "tarball_identity")
    raw = get(url, MAX_ARCHIVE)
    members = archive_members(raw, metadata["dist"]["integrity"])
    embedded = json.loads(members["package.json"][0])
    require(embedded.get("name") == package and embedded.get("version") == version, "embedded_identity")
    for field in ("bin", "scripts", "dependencies", "optionalDependencies", "os", "cpu"):
        require(embedded.get(field) == metadata.get(field), "metadata_manifest_contract")
    write(cache / (slug + ".metadata.json"), metadata_bytes)
    write(cache / (slug + ".tgz"), raw)
    return members, embedded


def write_members(root, members):
    for relative, (content, executable) in members.items():
        path = root / relative
        path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        write(path, content)
        path.chmod(0o755 if executable else 0o644)


def prepare_old(cache):
    require(platform.system() in ("Darwin", "Linux"), "unix_only")
    cpu = {"arm64":"arm64", "aarch64":"arm64", "x86_64":"x64"}.get(platform.machine())
    require(cpu is not None, "unsupported_arch")
    target = ("darwin" if platform.system() == "Darwin" else "linux") + "-" + cpu
    package = "@anthropic-ai/claude-code"
    top, metadata = official_package(package, OLD, cache)
    require(metadata.get("bin") == {"claude":"bin/claude.exe"}, "old_public_layout")
    require(metadata.get("dependencies") in (None, {}), "unexpected_dependencies")
    require(metadata.get("scripts", {}).get("postinstall") == "node install.cjs", "unknown_postinstall_contract")
    dependency = package + "-" + target
    require(metadata.get("optionalDependencies", {}).get(dependency) == OLD, "platform_dependency")
    native, _ = official_package(dependency, OLD, cache)
    require("claude" in native, "platform_native_missing")
    destination = cache / "old-package"
    destination.mkdir(mode=0o755)
    write_members(destination, top)
    write_members(destination / "node_modules" / dependency, native)
    # 复制独立校验的官方 native；从未执行 postinstall 或 npm install。
    public = destination / "bin/claude.exe"
    public.write_bytes(native["claude"][0])
    public.chmod(0o755)
    write(cache / "old-materialization.safe.json", {"version":OLD,"target":target,"public_sha256":sha(public),"install_scripts_executed":False})
    return destination


def binding(path, expected=None):
    path = Path(path).resolve(strict=True)
    actual = sha(path)
    require(expected is None or actual == expected, "binary_expected_sha_mismatch")
    return {"path":str(path), "sha256":actual}


def environment(root, manifest, step):
    result = {name:str(root / part) for name, part in ENV_PATHS.items()}
    result.update(PATH=str(root / "tools") + ":/usr/bin:/bin:/usr/sbin:/sbin", LANG="C", LC_ALL="C",
        DISABLE_AUTOUPDATER="1", CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1",
        NPM_CONFIG_USERCONFIG=str(root / "npm/user.npmrc"), NPM_CONFIG_GLOBALCONFIG=str(root / "npm/global.npmrc"), NPM_CONFIG_CACHE=str(root / "npm/cache"),
        INFINISHELL_CLI_NPM_UPDATE_ALLOW=SCOPE, INFINISHELL_CLI_NPM_UPDATE_STEP=step,
        INFINISHELL_CLI_NPM_UPDATE_MANIFEST=str(root / "manifest.private.json"),
        INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=manifest["supervisor"]["path"])
    return result


def run_test(binary, test, root, env, output, ignored=False):
    command = [binary, "--exact", test, "--nocapture", "--test-threads=1"] + (["--ignored"] if ignored else [])
    try:
        result = subprocess.run(command, cwd=root, env=env, capture_output=True, timeout=900, check=False)
        write(output.with_suffix(".stdout"), result.stdout)
        write(output.with_suffix(".stderr"), result.stderr)
        write(output.with_suffix(".safe.json"), {"returncode":result.returncode,"command":command,"model_inputs_sent":0})
        require(result.returncode == 0 and b"1 passed" in result.stdout, "product_test_failed")
    except subprocess.TimeoutExpired as error:
        write(output.with_suffix(".timeout.safe.json"), {"timeout":True,"model_inputs_sent":0})
        write(output.with_suffix(".stdout"), error.stdout or b"")
        write(output.with_suffix(".stderr"), error.stderr or b"")
        raise


def fixture(args, case, old, sources, binaries):
    root = Path(tempfile.mkdtemp(prefix="infinishell-npm-live-", dir=args.output)).resolve()
    for part in set(ENV_PATHS.values()) | {"project","tools","npm/cache","prefix/bin"}:
        (root / part).mkdir(mode=0o700, parents=True, exist_ok=True)
    write(root / ".infinishell-npm-live", MARKER)
    write(root / "npm/user.npmrc", ("prefix=" + str(root / "prefix") + "\nregistry=https://registry.npmjs.org/\nignore-scripts=true\naudit=false\nfund=false\n").encode())
    write(root / "npm/global.npmrc", b"")
    write(root / "home/.claude/settings.json", b'{"autoUpdatesChannel":"latest"}\n')
    package = root / "prefix/lib/node_modules/@anthropic-ai/claude-code"
    package.parent.mkdir(mode=0o755, parents=True)
    shutil.copytree(old, package)
    # 使用非默认但安全的权限，验证事务确实保留既有 mode。
    (package / "bin/claude.exe").chmod(0o750)
    if (package / "README.md").exists():
        (package / "README.md").chmod(0o400)
    os.symlink("../lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe", root / "prefix/bin/claude")
    os.symlink(binaries["node"]["path"], root / "tools/node")
    os.symlink(binaries["npm_cli"]["path"], root / "tools/npm")
    manifest = dict(schema=1, scope=SCOPE, case=case, root=str(root), old_public_sha256=sha(package / "bin/claude.exe"), source_sha256=sources, **binaries)
    write(root / "manifest.private.json", manifest)
    return root, manifest


def verify_embedded(supervisor, repo):
    # 与现有 runner 一样逐块核源码字节；候选 supervisor 不得混用旧模块。
    paths = [relative for relative in SOURCE_FILES if not relative.endswith("sources_npm_live_tests.rs") and not relative.endswith(".py")]
    needles = [(repo / relative).read_bytes() for relative in paths]
    found = [False] * len(needles)
    maximum = max(map(len, needles))
    with Path(supervisor).open("rb") as stream:
        tail = b""
        while block := stream.read(1024 * 1024):
            data = tail + block
            for index, needle in enumerate(needles):
                found[index] |= needle in data
            tail = data[-maximum:]
    require(all(found), "supervisor_source_binding")


def verify_product_archives(root, case, old):
    wrapper_meta = json.loads((root / "verified-wrapper.metadata.json").read_bytes())
    platform_meta = json.loads((root / "verified-platform.metadata.json").read_bytes())
    require(wrapper_meta["name"] == "@anthropic-ai/claude-code" and wrapper_meta["version"] == TARGET
            and platform_meta["name"].startswith("@anthropic-ai/claude-code-") and platform_meta["version"] == TARGET,
            "product_registry_identity")
    wrapper = archive_members((root / "verified-wrapper.tgz").read_bytes(), wrapper_meta["dist"]["integrity"])
    native = archive_members((root / "verified-platform.tgz").read_bytes(), platform_meta["dist"]["integrity"])
    expected = dict(wrapper)
    expected.update({"node_modules/" + platform_meta["name"] + "/" + path: member for path, member in native.items()})
    expected["bin/claude.exe"] = native["claude"]
    journal = json.loads((root / "prepared-journal.safe.json").read_bytes())
    require(bytes(journal["wrapper_archive_sha256"]).hex() == sha(root / "verified-wrapper.tgz")
            and bytes(journal["platform_archive_sha256"]).hex() == sha(root / "verified-platform.tgz"), "product_archive_receipt_binding")
    prepared = journal["prepared"]["nodes"]
    prepared_files = {name:node for name, node in prepared.items() if node["sha256"] is not None}
    require(set(prepared_files) == set(expected), "product_prepared_member_set")
    for name, (content, _) in expected.items():
        require(prepared_files[name]["length"] == len(content)
                and bytes(prepared_files[name]["sha256"]) == hashlib.sha256(content).digest(), "product_prepared_member_digest")
    package = root / "prefix/lib/node_modules/@anthropic-ai/claude-code"
    if case in ("swap_receipt_missing", "candidate_changed_preserved"):
        expected = {str(path.relative_to(old)):(path.read_bytes(), False) for path in old.rglob("*") if path.is_file()}
    actual = set()
    for path in package.rglob("*"):
        require(not path.is_symlink(), "published_tree_contains_link")
        if path.is_file():
            relative = str(path.relative_to(package))
            if case == "external_change_preserved" and relative == "external-npm-change":
                require(path.read_bytes() == b"later external install must survive\n", "external_change_bytes")
                continue
            actual.add(relative)
            require(relative in expected and sha(path) == hashlib.sha256(expected[relative][0]).hexdigest(), "published_member_digest")
    require(actual == set(expected), "published_member_set")
    write(root / "independent-archive-check.safe.json", {"sri_verified":True,"prepared_members":len(prepared_files),
        "published_members":len(actual),"target":TARGET,"old":OLD,"global_installation_changed":False})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    for role in ("test-binary", "supervisor", "node", "npm-cli"):
        parser.add_argument("--" + role, type=Path, required=True)
        parser.add_argument("--" + role + "-sha256", required=True)
    parser.add_argument("--case", choices=CASES, action="append")
    args = parser.parse_args()
    args.repo = args.repo.resolve(strict=True)
    require(not args.output.exists(), "output_must_be_new")
    args.output.mkdir(mode=0o700, parents=True)
    args.output = args.output.resolve(strict=True)
    require(all(ord(character) >= 32 and character not in "\r\n" for character in str(args.output)), "prefix_has_control_characters")
    require(not str(args.output).startswith("/Volumes/"), "runtime_requires_internal_disk")
    require(shutil.disk_usage(args.output).free > 3 * 1024**3, "fixture_space_insufficient")
    binaries = {key:binding(getattr(args, option), getattr(args, option + "_sha256")) for key, option in (("worker","test_binary"),("supervisor","supervisor"),("node","node"),("npm_cli","npm_cli"))}
    require(all(not record["path"].startswith("/Volumes/") for record in binaries.values()), "executables_require_internal_disk")
    npm_manifest = Path(binaries["npm_cli"]["path"]).parent.parent / "package.json"
    npm_metadata = json.loads(npm_manifest.read_bytes())
    require(npm_metadata.get("name") == "npm" and npm_metadata.get("bin", {}).get("npm") == "bin/npm-cli.js", "npm_registration")
    source = {relative:sha(args.repo / relative) for relative in SOURCE_FILES}
    require(source[SOURCE_FILES[-1]] == sha(Path(__file__)), "runner_source_binding")
    verify_embedded(binaries["supervisor"]["path"], args.repo)
    if platform.system() == "Darwin":
        checked = subprocess.run(["/usr/bin/codesign","--verify","--strict",binaries["supervisor"]["path"]], capture_output=True, check=False)
        write(args.output / "supervisor-signature.stdout", checked.stdout)
        write(args.output / "supervisor-signature.stderr", checked.stderr)
        require(checked.returncode == 0, "supervisor_signature")
    revision = subprocess.run(["git", "-C", str(args.repo), "rev-parse", "HEAD"], capture_output=True, check=True, text=True).stdout.strip()
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "source_commit_invalid")
    dirty = bool(subprocess.run(["git", "-C", str(args.repo), "status", "--porcelain", "--untracked-files=no"], capture_output=True, check=True).stdout)
    write(args.output / "source.safe.json", {"source_sha256":source,"binaries":binaries,"commit":revision,"working_tree_dirty":dirty,
        "platform":{"system":platform.system(),"machine":platform.machine()},"mode":"official_npm_single_package_tree",
        "old_version":OLD,"target_version":TARGET,"base_patch":"c06688d3c246f08eb49838cade5e24223a243ea1169f0205281c0bacb141f72f"})
    cache = args.output / "official-inputs"
    cache.mkdir(mode=0o700)
    old = prepare_old(cache)
    results = []
    for case in args.case or CASES:
        root, manifest = fixture(args, case, old, source, binaries)
        env = environment(root, manifest, "execute")
        if not results:
            for index, test in enumerate(BUSY_TESTS):
                run_test(binaries["worker"]["path"], test, root / "project", env, args.output / ("busy-gate-" + str(index)))
        run_test(binaries["worker"]["path"], TEST, root / "project", env, root / "execute", True)
        result = json.loads((root / "result-execute.safe.json").read_text())
        if case in ("swap_receipt_missing", "external_change_preserved"):
            require(result.get("needs_cold_recovery") is True and result.get("accepted") is False, "checkpoint_not_observed")
            run_test(binaries["worker"]["path"], TEST, root / "project", environment(root, manifest, "recover"), root / "recover", True)
            result = json.loads((root / "result-recover.safe.json").read_text())
        require(result.get("accepted") is True, "case_not_accepted")
        verify_product_archives(root, case, old)
        results.append({"case":case,"root":str(root),"result":result})
    write(args.output / "summary.safe.json", {"scope":SCOPE,"accepted":True,"cases":results,"model_inputs_sent":0,
        "busy_scope":"existing product model state-machine tests only", "plugin_recheck_covered":False,
        "g09_closed":False,"consumer_channels_unchanged":True})

if __name__ == "__main__":
    main()
