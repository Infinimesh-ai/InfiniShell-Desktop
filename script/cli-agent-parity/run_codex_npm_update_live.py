#!/usr/bin/env python3
"""固定 Codex npm 私有安装的真实 Node 升级与冷恢复验收；不运行模型或安装脚本。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile

from run_claude_npm_update_live import (
    ENV_PATHS, MAX_ARCHIVE, archive_members, binding, get, require, run_test, sha,
    write, write_members,
)

SCOPE = "codex_npm_transaction_no_model_v1"
MARKER = b"InfiniShell private npm transaction fixture; no credentials\n"
OLD, TARGET = "0.155.1", "0.156.1"
PACKAGE = "@openai/codex"
CASES = ("updated", "swap_receipt_missing", "external_change_preserved", "candidate_changed_preserved")
TEST = "terminal::cli_agent_updates::sources::npm_transaction::codex_live_tests::real_codex_npm_update_without_model"
SOURCE_FILES = (
    "app/src/terminal/cli_agent_updates.rs",
    *["app/src/terminal/cli_agent_updates/" + name for name in (
        "sources.rs", "sources_npm.rs", "sources_npm_release.rs", "sources_npm_transaction.rs",
        "sources_npm_tree_unix.rs", "sources_codex_npm_live_tests.rs", "sources_npm_codex.rs")],
    *["app/src/ai/cli_agent_runtime/" + name for name in (
        "managed_process.rs", "managed_process_version_probe.rs", "managed_process_atomic_macos.rs",
        "managed_process_atomic_linux.rs", "managed_process_atomic_linux_glibc.rs",
        "managed_process_npm_probe.rs", "managed_process_npm_probe_macos.rs")],
    "script/cli-agent-parity/run_claude_npm_update_live.py",
    "script/cli-agent-parity/codex_0156_package_manifest.json",
    "script/cli-agent-parity/run_codex_npm_update_live.py",
)
# 原始官方 registry 元数据给出的固定 SRI；校验完整 tarball，不把版本号当作字节身份。
INTEGRITIES = {
    "0.155.1": "sha512-02fAAGyBtlA1zPjEo3kTj/bOSYbPz5DvjLwRZJdV7weFFEDzNFOMjQGmZ/+5CuirYV0hE+AZTrnjzwXYU4AdAQ==",
    "0.155.1-darwin-arm64": "sha512-cYxzGcRRoBrncyHlR8ed4yXwcoVJZC1pipGULSyJkGFKXJw/Uu57BklvzayuAptjJIipamnOk32CfUkk1F0bLw==",
    "0.155.1-linux-x64": "sha512-atv3HF0mubqB0J/XkQ2JopqKzXJ+/7aQtTB2MkJ9MrraujMIz8zbCCLylLkN3PzpVGTJzzQFN/wD1oq8oJPJKg==",
    "0.156.1": "sha512-nI1iVl/n2SO2lSvlwEsJx63zdSI4C4Me2gR7AG0OWMJiGSakz2tY2hx43E39Zq5aEoeB5bZjJXzp5Sqhog6vyA==",
    "0.156.1-darwin-arm64": "sha512-Jg6wbdV+wmMZczhwE74GSxOYEZlViKXn6KyCw/yfrz3PAKFD14xljuPopmdhWC1+8IKU2WdN5fdmXNPt2q4HPA==",
    "0.156.1-linux-x64": "sha512-2ePo0wgOcnONKsuzp8vBjOmNY+IdsKaouaDDIdiKq9HOWNuV/GI22OeXft2A/1GaoL71ictO0/pLAsCEnQ6wew==",
}


def target_platform():
    value = (platform.system(), platform.machine())
    if value == ("Darwin", "arm64"):
        return "darwin-arm64", "aarch64-apple-darwin"
    if value == ("Linux", "x86_64"):
        return "linux-x64", "x86_64-unknown-linux-musl"
    raise ValueError("platform_not_calibrated")


def package_contract(metadata, members, version):
    require(version in INTEGRITIES and metadata.get("name") == PACKAGE
            and metadata.get("version") == version, "registry_identity")
    require(metadata.get("dist", {}).get("integrity") == INTEGRITIES[version], "fixed_registry_integrity")
    embedded = json.loads(members["package.json"][0])
    require(embedded.get("name") == PACKAGE and embedded.get("version") == version, "embedded_identity")
    for field in ("bin", "scripts", "dependencies", "optionalDependencies", "os", "cpu", "libc"):
        require(embedded.get(field) == metadata.get(field), "metadata_manifest_contract")
    require(embedded.get("scripts") in (None, {}) and embedded.get("dependencies") in (None, {}),
            "unexpected_scripts_or_dependencies")
    require(metadata["dist"].get("fileCount") == len(members)
            and metadata["dist"].get("unpackedSize") == sum(len(value[0]) for value in members.values()),
            "archive_complete_inventory")
    return embedded


def official_package(version, cache, reused=None):
    require(version in INTEGRITIES, "version_not_fixed")
    stem = "openai-codex-" + version
    metadata_name, archive_name = stem + ".metadata.json", stem + ".tgz"
    metadata_url = "https://registry.npmjs.org/@openai/codex/" + version
    archive_url = "https://registry.npmjs.org/@openai/codex/-/codex-" + version + ".tgz"
    metadata_bytes = ((reused / metadata_name).read_bytes()
                      if reused and (reused / metadata_name).is_file() else get(metadata_url, 1024 * 1024))
    # 先封存原始响应，校验失败也保留，不覆盖既有归档。
    write(cache / metadata_name, metadata_bytes)
    metadata = json.loads(metadata_bytes)
    require(metadata.get("name") == PACKAGE and metadata.get("version") == version
            and metadata.get("dist", {}).get("tarball") == archive_url
            and metadata["dist"].get("integrity") == INTEGRITIES[version], "registry_contract")
    raw = ((reused / archive_name).read_bytes()
           if reused and (reused / archive_name).is_file() else get(archive_url, MAX_ARCHIVE))
    write(cache / archive_name, raw)
    members = archive_members(raw, INTEGRITIES[version])
    embedded = package_contract(metadata, members, version)
    write(cache / (stem + ".origin.safe.json"), {
        "metadata_url":metadata_url, "archive_url":archive_url,
        "metadata_sha256":hashlib.sha256(metadata_bytes).hexdigest(),
        "archive_sha256":hashlib.sha256(raw).hexdigest(), "integrity":INTEGRITIES[version],
        "reuse_directory":str(reused) if reused else None,
    })
    return members, embedded


def prepare_old(cache, reused=None):
    target, triple = target_platform()
    wrapper, metadata = official_package(OLD, cache, reused)
    require(set(wrapper) == {"package.json", "README.md", "bin/codex.js"}
            and metadata.get("bin") == {"codex":"bin/codex.js"}, "old_public_layout")
    dependency = PACKAGE + "-" + target
    require(metadata.get("optionalDependencies", {}).get(dependency) == "npm:" + PACKAGE + "@" + OLD + "-" + target,
            "platform_alias_contract")
    native, embedded = official_package(OLD + "-" + target, cache, reused)
    require(embedded.get("os") == [target.split("-")[0]] and embedded.get("cpu") == [target.split("-")[1]],
            "platform_os_cpu")
    require("vendor/" + triple + "/bin/codex" in native and "vendor/" + triple + "/codex-package.json" in native,
            "platform_complete_native_tree_missing")
    destination = cache / "old-package"
    destination.mkdir(mode=0o755)
    write_members(destination, wrapper)
    write_members(destination / "node_modules" / dependency, native)
    write(cache / "old-materialization.safe.json", {
        "version":OLD,"target":target,"public_sha256":sha(destination / "bin/codex.js"),
        "platform_alias":dependency,"platform_registered_name":PACKAGE,"install_scripts_executed":False,
        "native_resources_preserved":True,
    })
    return destination


def clone_copy(source, destination):
    if platform.system() == "Darwin":
        subprocess.run(["/bin/cp", "-c", "-p", str(source), str(destination)], check=True, capture_output=True)
    else:
        shutil.copy2(source, destination)
    return destination


def environment(root, manifest, step):
    result = {name:str(root / part) for name, part in ENV_PATHS.items()}
    result.update(PATH=str(root / "tools") + ":/usr/bin:/bin:/usr/sbin:/sbin", LANG="C", LC_ALL="C",
        DISABLE_AUTOUPDATER="1", CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1",
        NPM_CONFIG_USERCONFIG=str(root / "npm/user.npmrc"), NPM_CONFIG_GLOBALCONFIG=str(root / "npm/global.npmrc"),
        NPM_CONFIG_CACHE=str(root / "npm/cache"), INFINISHELL_CLI_CODEX_NPM_UPDATE_ALLOW=SCOPE,
        INFINISHELL_CLI_CODEX_NPM_UPDATE_STEP=step,
        INFINISHELL_CLI_CODEX_NPM_UPDATE_MANIFEST=str(root / "manifest.private.json"),
        INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=manifest["supervisor"]["path"])
    return result


def fixture(args, case, old, sources, binaries):
    root = Path(tempfile.mkdtemp(prefix="infinishell-codex-npm-live-", dir=args.output)).resolve()
    for part in set(ENV_PATHS.values()) | {"project", "tools", "npm/cache", "prefix/bin"}:
        (root / part).mkdir(mode=0o700, parents=True, exist_ok=True)
    write(root / ".infinishell-npm-live", MARKER)
    write(root / "npm/user.npmrc", ("prefix=" + str(root / "prefix")
        + "\nregistry=https://registry.npmjs.org/\nignore-scripts=true\naudit=false\nfund=false\n").encode())
    write(root / "npm/global.npmrc", b"")
    package = root / "prefix/lib/node_modules/@openai/codex"
    package.parent.mkdir(mode=0o755, parents=True)
    shutil.copytree(old, package, copy_function=clone_copy)
    (package / "bin/codex.js").chmod(0o750)
    (package / "README.md").chmod(0o400)
    os.symlink("../lib/node_modules/@openai/codex/bin/codex.js", root / "prefix/bin/codex")
    os.symlink(binaries["node"]["path"], root / "tools/node")
    os.symlink(binaries["npm_cli"]["path"], root / "tools/npm")
    manifest = dict(schema=1, scope=SCOPE, case=case, root=str(root),
                    old_public_sha256=sha(package / "bin/codex.js"), source_sha256=sources, **binaries)
    write(root / "manifest.private.json", manifest)
    return root, manifest


def verify_embedded(supervisor, repo):
    paths = [name for name in SOURCE_FILES if not name.endswith(("_live_tests.rs", ".py"))]
    needles = [(repo / name).read_bytes() for name in paths]
    found, maximum = [False] * len(needles), max(map(len, needles))
    with Path(supervisor).open("rb") as source:
        tail = b""
        while block := source.read(1024 * 1024):
            data = tail + block
            for index, needle in enumerate(needles):
                found[index] |= needle in data
            tail = data[-maximum:]
    require(all(found), "supervisor_source_binding")


def verify_product_archives(root, case, old):
    target, _ = target_platform()
    wrapper_meta = json.loads((root / "verified-wrapper.metadata.json").read_bytes())
    platform_meta = json.loads((root / "verified-platform.metadata.json").read_bytes())
    wrapper = archive_members((root / "verified-wrapper.tgz").read_bytes(), INTEGRITIES[TARGET])
    native = archive_members((root / "verified-platform.tgz").read_bytes(), INTEGRITIES[TARGET + "-" + target])
    package_contract(wrapper_meta, wrapper, TARGET)
    package_contract(platform_meta, native, TARGET + "-" + target)
    expected = dict(wrapper)
    expected.update({"node_modules/" + PACKAGE + "-" + target + "/" + name:member for name, member in native.items()})
    journal = json.loads((root / "prepared-journal.safe.json").read_bytes())
    require(bytes(journal["wrapper_archive_sha256"]).hex() == sha(root / "verified-wrapper.tgz")
            and bytes(journal["platform_archive_sha256"]).hex() == sha(root / "verified-platform.tgz"),
            "product_archive_receipt_binding")
    prepared = {name:node for name,node in journal["prepared"]["nodes"].items() if node["sha256"] is not None}
    require(set(prepared) == set(expected), "product_prepared_member_set")
    for name, (content, _) in expected.items():
        require(prepared[name]["length"] == len(content)
                and bytes(prepared[name]["sha256"]) == hashlib.sha256(content).digest(), "product_prepared_member_digest")
    if case in ("swap_receipt_missing", "candidate_changed_preserved"):
        expected = {str(path.relative_to(old)):(path.read_bytes(), False) for path in old.rglob("*") if path.is_file()}
    package = root / "prefix/lib/node_modules/@openai/codex"
    actual = set()
    for path in package.rglob("*"):
        require(not path.is_symlink(), "published_tree_contains_link")
        if path.is_file():
            relative = path.relative_to(package).as_posix()
            if case == "external_change_preserved" and relative == "external-npm-change":
                require(path.read_bytes() == b"later external install must survive\n", "external_change_bytes")
                continue
            actual.add(relative)
            require(relative in expected and sha(path) == hashlib.sha256(expected[relative][0]).hexdigest(),
                    "published_member_digest")
    require(actual == set(expected), "published_member_set")
    write(root / "independent-archive-check.safe.json", {
        "sri_verified":True,"prepared_members":len(prepared),"published_members":len(actual),
        "target":TARGET,"old":OLD,"public_entry":"bin/codex.js","native_resources_verified":True,
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--official-inputs", type=Path,
                        help="复用只读官方材料目录，缺失材料仍从官方 registry 下载并另存原始字节")
    for role in ("test-binary", "supervisor", "node", "npm-cli"):
        parser.add_argument("--" + role, type=Path, required=True)
        parser.add_argument("--" + role + "-sha256", required=True)
    parser.add_argument("--case", choices=CASES, action="append")
    args = parser.parse_args()
    target_platform()
    args.repo = args.repo.resolve(strict=True)
    if args.official_inputs:
        args.official_inputs = args.official_inputs.resolve(strict=True)
    require(not args.output.exists(), "output_must_be_new")
    args.output.mkdir(mode=0o700, parents=True)
    args.output = args.output.resolve(strict=True)
    require(all(ord(character) >= 32 for character in str(args.output)), "prefix_has_control_characters")
    require(not str(args.output).startswith("/Volumes/"), "runtime_requires_internal_disk")
    require(shutil.disk_usage(args.output).free > 3 * 1024**3, "fixture_space_insufficient")
    binaries = {key:binding(getattr(args, option), getattr(args, option + "_sha256"))
                for key,option in (("worker","test_binary"),("supervisor","supervisor"),("node","node"),("npm_cli","npm_cli"))}
    require(all(not record["path"].startswith("/Volumes/") for record in binaries.values()), "executables_require_internal_disk")
    npm_manifest = Path(binaries["npm_cli"]["path"]).parent.parent / "package.json"
    npm_metadata = json.loads(npm_manifest.read_bytes())
    require(npm_metadata.get("name") == "npm" and npm_metadata.get("bin", {}).get("npm") == "bin/npm-cli.js",
            "npm_registration")
    sources = {name:sha(args.repo / name) for name in SOURCE_FILES}
    require(sources[SOURCE_FILES[-1]] == sha(Path(__file__)), "runner_source_binding")
    verify_embedded(binaries["supervisor"]["path"], args.repo)
    if platform.system() == "Darwin":
        signature = subprocess.run(["/usr/bin/codesign","--verify","--strict",binaries["supervisor"]["path"]],
                                   capture_output=True, check=False)
        write(args.output / "supervisor-signature.stdout", signature.stdout)
        write(args.output / "supervisor-signature.stderr", signature.stderr)
        require(signature.returncode == 0, "supervisor_signature")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.repo, text=True, encoding="utf-8").strip()
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "source_commit_invalid")
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=args.repo, text=True, encoding="utf-8")
    write(args.output / "source.safe.json", {"source_sha256":sources,"binaries":binaries,"commit":revision,
        "working_tree_dirty":bool(status.strip()),"platform":{"system":platform.system(),"machine":platform.machine()},
        "mode":"fixed_official_npm_node_transaction","old_version":OLD,"target_version":TARGET,
        "consumer_channel_discovery_covered":False})
    cache = args.output / "official-inputs"
    cache.mkdir(mode=0o700)
    old = prepare_old(cache, args.official_inputs)
    results = []
    for case in args.case or CASES:
        root, manifest = fixture(args, case, old, sources, binaries)
        run_test(binaries["worker"]["path"], TEST, root / "project", environment(root, manifest, "execute"), root / "execute", True)
        result = json.loads((root / "result-execute.safe.json").read_bytes())
        if case in ("swap_receipt_missing", "external_change_preserved"):
            require(result.get("needs_cold_recovery") is True and result.get("accepted") is False, "checkpoint_not_observed")
            run_test(binaries["worker"]["path"], TEST, root / "project", environment(root, manifest, "recover"), root / "recover", True)
            result = json.loads((root / "result-recover.safe.json").read_bytes())
        require(result.get("accepted") is True, "case_not_accepted")
        if case != "candidate_changed_preserved":
            require(result.get("candidate_probe", {}).get("public_node_launcher_executed") is True,
                    "native_version_alone_is_not_acceptance")
        verify_product_archives(root, case, old)
        require(all(sha(record["path"]) == record["sha256"] for record in binaries.values())
                and all(sha(args.repo / name) == digest for name,digest in sources.items()), "inputs_changed_during_run")
        results.append({"case":case,"root":str(root),"result":result})
    write(args.output / "summary.safe.json", {"scope":SCOPE,"accepted":True,"cases":results,"model_inputs_sent":0,
        "fixed_release_test_hook_used":True,"consumer_channel_discovery_covered":False,
        "busy_or_plugin_recheck_covered":False,"gui_covered":False,"g09_closed":False,
        "consumer_channels_unchanged":True,"global_installation_changed":False})


if __name__ == "__main__":
    main()
