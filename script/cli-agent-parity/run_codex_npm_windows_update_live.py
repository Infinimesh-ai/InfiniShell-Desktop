#!/usr/bin/env python3
"""Windows x64 固定 Codex npm 私有登记、真实公共入口升级及两步发布冷恢复验收。"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import tempfile

from run_claude_npm_update_live import MAX_ARCHIVE, archive_members, get, require, run_test, sha, write

SCOPE = "codex_windows_npm_transaction_no_model_v1"
MARKER = b"InfiniShell private Windows npm transaction fixture; no credentials\n"
OLD, TARGET = "0.155.1", "0.156.1"
PACKAGE = "@openai/codex"
DEPENDENCY = "node_modules/@openai/codex-win32-x64/"
CASES = ("updated", "old_moved", "published_receipt_missing", "external_change_preserved", "candidate_changed_preserved")
COLD_CASES = CASES[1:4]
TEST = "terminal::cli_agent_updates::sources::npm_windows::live_tests::real_codex_windows_npm_update_without_model"
SOURCE_FILES = (
    "app/src/terminal/cli_agent_updates.rs",
    *["app/src/terminal/cli_agent_updates/" + name for name in (
        "sources.rs", "sources_npm.rs", "sources_npm_release.rs", "sources_npm_codex_windows.rs",
        "sources_npm_codex_windows_tree.rs", "sources_npm_codex_windows_contract.rs",
        "sources_codex_npm_windows_live_tests.rs")],
    *["app/src/ai/cli_agent_runtime/" + name for name in (
        "managed_process.rs", "managed_process_version_probe.rs", "managed_process_atomic_windows.rs",
        "managed_process_npm_probe_windows.rs")],
    "crates/command/src/windows_appcontainer.rs",
    "script/cli-agent-parity/codex_0156_package_manifest.json",
    "script/cli-agent-parity/run_claude_npm_update_live.py",
    "script/cli-agent-parity/run_codex_npm_windows_update_live.py",
)
# 完整官方 registry 固定版本 SRI；平台包仍注册为 @openai/codex 的 npm alias。
INTEGRITIES = {
    OLD: "sha512-02fAAGyBtlA1zPjEo3kTj/bOSYbPz5DvjLwRZJdV7weFFEDzNFOMjQGmZ/+5CuirYV0hE+AZTrnjzwXYU4AdAQ==",
    OLD + "-win32-x64": "sha512-MO+cCZrgU0Ec7lJP/5NsTe5obJ9/qtRMkQUK0jYWTY1omxLA3lp5IOD2IAmsejlEJB931XRo51LZ7hl178CDjA==",
    TARGET: "sha512-nI1iVl/n2SO2lSvlwEsJx63zdSI4C4Me2gR7AG0OWMJiGSakz2tY2hx43E39Zq5aEoeB5bZjJXzp5Sqhog6vyA==",
    TARGET + "-win32-x64": "sha512-MJyLxbBs2zzp5kbaR/99Zwe7SmbrwUkveTcT+ayYlO48V0nYh0eU+h2lalBwvC7VJ/ya/bXnUtISJfJKhGCD/g==",
}
ENV_PATHS = {
    "HOME":"home", "USERPROFILE":"home", "APPDATA":"home/AppData/Roaming",
    "LOCALAPPDATA":"home/AppData/Local", "CODEX_HOME":"home/.codex",
    "CLAUDE_CONFIG_DIR":"home/.claude", "GROK_HOME":"home/.grok",
    "XDG_CONFIG_HOME":"home/.config", "XDG_DATA_HOME":"home/.local/share",
    "XDG_CACHE_HOME":"home/.cache", "XDG_STATE_HOME":"home/.local/state",
    "TMP":"tmp", "TEMP":"tmp", "TMPDIR":"tmp",
    "NPM_CONFIG_USERCONFIG":"npm/user.npmrc", "NPM_CONFIG_GLOBALCONFIG":"npm/global.npmrc",
    "NPM_CONFIG_CACHE":"npm/cache",
}


def canonical(path):
    value = str(Path(path).resolve(strict=True))
    # 与 Rust canonicalize 返回的 Windows 路径一致，禁止网络共享验收目录。
    if os.name == "nt":
        dos = value[4:] if value.startswith("\\\\?\\") else value
        require(re.match(r"^[A-Za-z]:\\", dos), "network_path")
        value = "\\\\?\\" + dos
    return Path(value)


def plain(path):
    info = path.lstat()
    require(not path.is_symlink() and not getattr(info, "st_file_attributes", 0)
            & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400), "reparse_point")
    return info


def inventory(root):
    result = {}
    plain(root)
    for path in sorted(root.rglob("*")):
        info = plain(path)
        if stat.S_ISREG(info.st_mode):
            require(info.st_nlink == 1, "hardlinked_file")
            result[path.relative_to(root).as_posix()] = {"length":info.st_size, "sha256":sha(path)}
        else:
            require(stat.S_ISDIR(info.st_mode), "non_regular_file")
    return result


def package_contract(metadata, members, version):
    require(version in INTEGRITIES and metadata.get("name") == PACKAGE
            and metadata.get("version") == version, "registry_identity")
    dist = metadata.get("dist", {})
    require(dist.get("integrity") == INTEGRITIES[version], "fixed_registry_integrity")
    embedded = json.loads(members["package.json"][0])
    for key in ("name", "version", "bin", "scripts", "dependencies", "optionalDependencies", "os", "cpu", "libc"):
        require(embedded.get(key) == metadata.get(key), "metadata_manifest_contract")
    require(embedded.get("scripts") in (None, {}) and embedded.get("dependencies") in (None, {}), "package_lifecycle")
    require(dist.get("fileCount") == len(members)
            and dist.get("unpackedSize") == sum(len(item[0]) for item in members.values()), "complete_inventory")
    if version in (OLD, TARGET):
        require(set(members) == {"package.json", "README.md", "bin/codex.js"}
                and embedded.get("bin") == {"codex":"bin/codex.js"}, "public_wrapper_layout")
        require(embedded.get("optionalDependencies", {}).get("@openai/codex-win32-x64")
                == "npm:@openai/codex@" + version + "-win32-x64", "platform_alias")
    else:
        require(embedded.get("os") == ["win32"] and embedded.get("cpu") == ["x64"]
                and not embedded.get("optionalDependencies"), "platform_contract")
        require("vendor/x86_64-pc-windows-msvc/bin/codex.exe" in members, "native_entry_missing")
    return embedded


def official_package(version, output, reused=None):
    require(version in INTEGRITIES, "version_not_fixed")
    stem = "openai-codex-" + version
    metadata_url = "https://registry.npmjs.org/@openai/codex/" + version
    archive_url = "https://registry.npmjs.org/@openai/codex/-/codex-" + version + ".tgz"
    metadata_path, archive_path = output / (stem + ".metadata.json"), output / (stem + ".tgz")
    reuse_meta = reused / metadata_path.name if reused else None
    raw_meta = reuse_meta.read_bytes() if reuse_meta and reuse_meta.is_file() else get(metadata_url, 1024 * 1024)
    write(metadata_path, raw_meta)
    require(0 < len(raw_meta) <= 1024 * 1024, "registry_size")
    metadata = json.loads(raw_meta)
    require(metadata.get("name") == PACKAGE and metadata.get("version") == version
            and metadata.get("dist", {}).get("integrity") == INTEGRITIES[version]
            and metadata["dist"].get("tarball") == archive_url, "official_registry_contract")
    reuse_archive = reused / archive_path.name if reused else None
    raw = reuse_archive.read_bytes() if reuse_archive and reuse_archive.is_file() else get(archive_url, MAX_ARCHIVE)
    write(archive_path, raw)
    require(0 < len(raw) <= MAX_ARCHIVE, "archive_size")
    members = archive_members(raw, INTEGRITIES[version])
    package_contract(metadata, members, version)
    write(output / (stem + ".origin.safe.json"), {"metadata_url":metadata_url,"archive_url":archive_url,
        "metadata_sha256":sha(metadata_path),"archive_sha256":sha(archive_path),"integrity":INTEGRITIES[version]})
    return members


def expected_files(wrapper, platform_package):
    return {name:{"length":len(value[0]),"sha256":hashlib.sha256(value[0]).hexdigest()}
            for name,value in list(wrapper.items()) + [(DEPENDENCY + key,value) for key,value in platform_package.items()]}


def environment(root, binaries, system_root, step):
    result = {name:str(root / relative) for name,relative in ENV_PATHS.items()}
    result.update(SYSTEMROOT=str(system_root), WINDIR=str(system_root), COMSPEC=str(system_root / "System32/cmd.exe"),
        PATH=os.pathsep.join((str(Path(binaries["node"]["path"]).parent), str(system_root / "System32"),
                             str(system_root / "System32/WindowsPowerShell/v1.0"))),
        PATHEXT=".COM;.EXE;.BAT;.CMD", LANG="C", LC_ALL="C", DISABLE_AUTOUPDATER="1",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", INFINISHELL_CLI_CODEX_WINDOWS_NPM_ALLOW=SCOPE,
        INFINISHELL_CLI_CODEX_WINDOWS_NPM_STEP=step,
        INFINISHELL_CLI_CODEX_WINDOWS_NPM_MANIFEST=str(root / "manifest.private.json"),
        INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=binaries["supervisor"]["path"])
    return result


def npm_install_arguments(binaries, prefix, archive):
    # shim 由绑定的真实 npm 生成；不执行生命周期，也不向用户的全局 prefix 安装。
    return [binaries["node"]["path"], binaries["npm_cli"]["path"], "install", "--global", "--prefix", str(prefix),
            "--ignore-scripts", "--no-audit", "--no-fund", "--install-strategy=nested", "--package-lock=false",
            "--registry=https://registry.npmjs.org/", str(archive)]


def verify_registration(root, old, receipt):
    package = root / "prefix/node_modules/@openai/codex"
    require(receipt.get("exit_code") == 0 and receipt.get("operation") == "real_npm_private_install"
            and receipt.get("scripts_disabled") is True, "npm_registration_receipt")
    actual = inventory(package)
    require(actual == old, "npm_installed_complete_official_tree")
    # 这里只保存 npm 原产物；最终逐字节 shim 合同由生产 Owner::capture 核验。
    shims = {}
    for name in ("codex", "codex.cmd", "codex.ps1"):
        info = plain(root / "prefix" / name)
        require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, "npm_generated_shim_missing")
        shims[name] = sha(root / "prefix" / name)
    return actual, shims


def fixture(args, case, binaries, sources, old, manager_tree, system_root):
    root = canonical(tempfile.mkdtemp(prefix="infinishell-codex-windows-npm-", dir=args.output))
    for name,relative in ENV_PATHS.items():
        if name not in ("NPM_CONFIG_USERCONFIG", "NPM_CONFIG_GLOBALCONFIG"):
            (root / relative).mkdir(parents=True, exist_ok=True)
    for relative in ("prefix", "project", "journal"):
        (root / relative).mkdir()
    write(root / ".infinishell-windows-npm-live", MARKER)
    write(root / "npm/user.npmrc", ("prefix=" + str(root / "prefix").replace("\\", "/")
        + "\nregistry=https://registry.npmjs.org/\nignore-scripts=true\naudit=false\nfund=false\n").encode())
    write(root / "npm/global.npmrc", b"")
    write(root / "home/.codex/config.toml", b"# private update acceptance; no credentials\n")
    write(root / "before-config.raw", (root / "home/.codex/config.toml").read_bytes())
    arguments = npm_install_arguments(binaries, root / "prefix", args.output / "official-inputs/openai-codex-0.155.1.tgz")
    env = environment(root, binaries, system_root, "install")
    with (root / "npm-install.stdout").open("xb") as stdout, (root / "npm-install.stderr").open("xb") as stderr:
        # 超时保留现场且失败，不声称 npm 后代已清理；不尝试继续产品更新。
        result = subprocess.run(arguments, cwd=root / "project", env=env, stdin=subprocess.DEVNULL,
            stdout=stdout, stderr=stderr, timeout=900, check=False, creationflags=subprocess.CREATE_NO_WINDOW)
    receipt = {"operation":"real_npm_private_install","arguments":arguments,"exit_code":result.returncode,
        "scripts_disabled":True,"private_prefix":str(root / "prefix"),"node":binaries["node"],
        "npm_cli":binaries["npm_cli"],"wrapper_archive_sha256":sha(Path(arguments[-1])),
        "stdout_sha256":sha(root / "npm-install.stdout"),"stderr_sha256":sha(root / "npm-install.stderr")}
    write(root / "npm-install.safe.json", receipt)
    actual, shims = verify_registration(root, old, receipt)
    write(root / "npm-generated-shims.safe.json", {
        name:{"sha256":digest,"raw_base64":base64.b64encode((root / "prefix" / name).read_bytes()).decode("ascii")}
        for name,digest in shims.items()
    })
    manifest = dict(schema=1,scope=SCOPE,case=case,root=str(root),source_sha256=sources,
        old_public_sha256=sha(root / "prefix/node_modules/@openai/codex/bin/codex.js"),
        npm_install_sha256=sha(root / "npm-install.safe.json"),npm_tree=manager_tree,shims=shims,**binaries)
    write(root / "npm-registered-tree.safe.json", actual)
    write(root / "manifest.private.json", manifest)
    return root, manifest


def verify_embedded(supervisor, repo):
    names = [name for name in SOURCE_FILES if not name.endswith(("_live_tests.rs", ".py"))]
    needles = [(repo / name).read_bytes() for name in names]
    found, maximum = [False] * len(needles), max(map(len, needles))
    with Path(supervisor).open("rb") as stream:
        tail = b""
        while block := stream.read(1024 * 1024):
            data = tail + block
            for index,needle in enumerate(needles):
                found[index] |= needle in data
            tail = data[-maximum:]
    require(all(found), "supervisor_source_binding")


def verify_product(root, case, old, target):
    wrapper_meta = json.loads((root / "verified-wrapper.metadata.json").read_bytes())
    platform_meta = json.loads((root / "verified-platform.metadata.json").read_bytes())
    wrapper = archive_members((root / "verified-wrapper.tgz").read_bytes(), INTEGRITIES[TARGET])
    native = archive_members((root / "verified-platform.tgz").read_bytes(), INTEGRITIES[TARGET + "-win32-x64"])
    package_contract(wrapper_meta, wrapper, TARGET)
    package_contract(platform_meta, native, TARGET + "-win32-x64")
    require(expected_files(wrapper, native) == target, "downloaded_target_changed")
    journal = json.loads((root / "prepared-journal.safe.json").read_bytes())
    prepared = {name.replace("\\", "/"):{"length":value["length"],"sha256":bytes(value["digest"]).hex()}
                for name,value in journal["prepared"]["members"].items() if not value["directory"]}
    require(prepared == target, "prepared_tree_contract")
    actual = inventory(root / "prefix/node_modules/@openai/codex")
    if case == "external_change_preserved":
        marker = actual.pop("external-npm-change")
        require(marker["sha256"] == hashlib.sha256(b"later external install must survive\n").hexdigest(), "external_changed")
    require(actual == (old if case in ("old_moved", "published_receipt_missing", "candidate_changed_preserved") else target),
            "published_tree_contract")
    require((root / "before-config.raw").read_bytes() == (root / "home/.codex/config.toml").read_bytes(), "private_config_changed")
    write(root / "independent-archive-check.safe.json", {"full_sri_verified":True,"prepared_files":len(prepared),
        "published_files":len(actual),"private_config_bytes_unchanged":True,"official_versions":[OLD,TARGET]})


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--repo", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    result.add_argument("--official-inputs", type=Path)
    for role in ("test-binary", "supervisor", "node", "npm-cli"):
        result.add_argument("--" + role, type=Path, required=True)
        result.add_argument("--" + role + "-sha256", required=True)
    result.add_argument("--case", choices=CASES, action="append")
    return result


def main():
    args = parser().parse_args()
    require(platform.system() == "Windows" and platform.machine().lower() in ("amd64", "x86_64"), "windows_x64_only")
    args.repo = canonical(args.repo)
    args.official_inputs = canonical(args.official_inputs) if args.official_inputs else None
    require(not args.output.exists(), "output_must_be_new")
    args.output.mkdir(parents=True)
    args.output = canonical(args.output)
    require(not any(char in str(args.output) for char in '\n\r\x00%&|<>"'), "fixture_path_metacharacters")
    require(shutil.disk_usage(args.output).free > 4 * 1024**3, "fixture_space_insufficient")
    binaries = {}
    for key,role in (("worker","test_binary"),("supervisor","supervisor"),("node","node"),("npm_cli","npm_cli")):
        path = canonical(getattr(args, role))
        expected = getattr(args, role + "_sha256").lower()
        require(re.fullmatch(r"[0-9a-f]{64}", expected) and sha(path) == expected, "binary_binding")
        binaries[key] = {"path":str(path),"sha256":expected}
    node, npm_cli = Path(binaries["node"]["path"]), Path(binaries["npm_cli"]["path"])
    require(node.name.lower() == "node.exe" and npm_cli == node.parent / "node_modules/npm/bin/npm-cli.js", "same_node_npm_installation")
    manager_root = npm_cli.parent.parent
    manager_manifest = json.loads((manager_root / "package.json").read_bytes())
    require(manager_manifest.get("name") == "npm" and manager_manifest.get("bin", {}).get("npm") == "bin/npm-cli.js", "npm_registration")
    manager_tree = inventory(manager_root)
    shim_sources = []
    for relative,identity in manager_tree.items():
        if relative.endswith("cmd-shim/package.json"):
            metadata = json.loads((manager_root / relative).read_bytes())
            shim_sources.append({"path":relative,"name":metadata.get("name"),"version":metadata.get("version"),
                                 "sha256":identity["sha256"]})
    system_root = canonical(os.environ["SystemRoot"])
    sources = {name:sha(args.repo / name) for name in SOURCE_FILES}
    require(sources[SOURCE_FILES[-1]] == sha(Path(__file__)), "runner_source_binding")
    verify_embedded(binaries["supervisor"]["path"], args.repo)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.repo, text=True, encoding="utf-8").strip()
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "source_commit_invalid")
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=args.repo, text=True, encoding="utf-8")
    write(args.output / "source.safe.json", {"commit":revision,"source_sha256":sources,"binaries":binaries,
        "working_tree_dirty":bool(status.strip()),"npm_version":manager_manifest.get("version"),"cmd_shim_registrations":shim_sources,
        "platform":"windows-x64","consumer_channel_discovery_covered":False})
    cache = args.output / "official-inputs"
    cache.mkdir()
    packages = {version:official_package(version, cache, args.official_inputs) for version in INTEGRITIES}
    old = expected_files(packages[OLD], packages[OLD + "-win32-x64"])
    target = expected_files(packages[TARGET], packages[TARGET + "-win32-x64"])
    cases = []
    for case in args.case or CASES:
        require(shutil.disk_usage(args.output).free > 4 * 1024**3, "fixture_space_insufficient")
        root, manifest = fixture(args, case, binaries, sources, old, manager_tree, system_root)
        run_test(binaries["worker"]["path"], TEST, root / "project", environment(root, binaries, system_root, "execute"), root / "execute", True)
        result = json.loads((root / "result-execute.safe.json").read_bytes())
        if case in COLD_CASES:
            require(result.get("needs_cold_recovery") is True and result.get("accepted") is False, "checkpoint_not_observed")
            run_test(binaries["worker"]["path"], TEST, root / "project", environment(root, binaries, system_root, "recover"), root / "recover", True)
            result = json.loads((root / "result-recover.safe.json").read_bytes())
        require(result.get("accepted") is True, "case_not_accepted")
        require(result.get("candidate_probe", {}).get("public_modes") == ([] if case == "candidate_changed_preserved" else ["cmd", "powershell"]),
                "both_public_entries_required")
        verify_product(root, case, old, target)
        require(all(sha(value["path"]) == value["sha256"] for value in binaries.values())
                and all(sha(args.repo / name) == digest for name,digest in sources.items())
                and inventory(manager_root) == manager_tree, "source_or_manager_changed_during_run")
        cases.append({"case":case,"root":str(root),"result":result})
    write(args.output / "summary.safe.json", {"scope":SCOPE,"accepted":True,"cases":cases,"model_inputs_sent":0,
        "fixed_release_test_hook_used":True,"consumer_channel_discovery_covered":False,"gui_covered":False,
        "busy_or_plugin_recheck_covered":False,"private_npm_registration_executed":True,
        "npm_lifecycle_executed":False,"npm_installer_tree_cleanup_covered":False,
        "global_installation_changed":False,"g09_closed":False})


if __name__ == "__main__":
    main()
