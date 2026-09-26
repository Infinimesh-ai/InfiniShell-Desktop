#!/usr/bin/env python3
"""固定 Grok npm 私有安装的零模型事务验收；复用原包，不执行安装脚本。"""
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
import ctypes
import urllib.request
import stat

SCOPE = "grok_npm_transaction_no_model_v1"
MARKER = b"InfiniShell private npm transaction fixture; no credentials\n"
OLD, TARGET = "1.0.40", "1.0.41"
CASES = ("updated", "swap_receipt_missing", "external_change_preserved", "candidate_changed_preserved")
TEST = "terminal::cli_agent_updates::sources::npm_grok::live_tests::real_grok_npm_update_without_model"
BUSY_TESTS = (
    "terminal::cli_agent_updates::tests::manual_update_still_obeys_busy_and_single_operation_guards",
    "terminal::cli_agent_updates::tests::pty_session_events_defer_update_until_last_local_session_exits",
)
SOURCE_FILES = (
    "app/src/terminal/cli_agent_updates.rs",
    *["app/src/terminal/cli_agent_updates/" + name for name in (
        "sources.rs", "sources_npm.rs", "sources_npm_release.rs", "sources_npm_grok.rs", "sources_npm_grok_contract.rs",
        "sources_npm_grok_mirror.rs", "sources_npm_tree_unix.rs", "sources_grok_npm_live_tests.rs")],
    *["app/src/ai/cli_agent_runtime/" + name for name in (
        "managed_process.rs", "managed_process_version_probe.rs", "managed_process_atomic_macos.rs", "managed_process_atomic_linux.rs", "managed_process_atomic_linux_glibc.rs")],
    "script/cli-agent-parity/grok_1041_npm_manifest.json",
    "script/cli-agent-parity/run_grok_npm_update_live.py",
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


def binding(path, expected=None):
    path = Path(path).resolve(strict=True)
    actual = sha(path)
    require(expected is None or actual == expected, "binary_expected_sha_mismatch")
    return {"path":str(path), "sha256":actual}


def environment(root, manifest, step):
    result = {name:str(root / part) for name, part in ENV_PATHS.items()}
    result.update(PATH=str(root / "tools") + ":/usr/bin:/bin:/usr/sbin:/sbin", LANG="C", LC_ALL="C",
        NPM_CONFIG_USERCONFIG=str(root / "npm/user.npmrc"), NPM_CONFIG_GLOBALCONFIG=str(root / "npm/global.npmrc"), NPM_CONFIG_CACHE=str(root / "npm/cache"),
        INFINISHELL_GROK_NPM_UPDATE_ALLOW=SCOPE, INFINISHELL_GROK_NPM_UPDATE_STEP=step,
        INFINISHELL_GROK_NPM_UPDATE_MANIFEST=str(root / "manifest.private.json"),
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


def verify_embedded(supervisor, repo):
    # 与现有 runner 一样逐块核源码字节；候选 supervisor 不得混用旧模块。
    paths = [relative for relative in SOURCE_FILES if not relative.endswith("sources_grok_npm_live_tests.rs") and not relative.endswith(".py")]
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



def target_platform():
    target = {( "Darwin", "arm64"): "darwin-arm64", ("Linux", "x86_64"): "linux-x64"}.get((platform.system(), platform.machine()))
    require(target is not None, "only_macos_arm64_linux_x64")
    return target



def package_coordinates(target, version):
    return [("@xai-official/grok", "grok-1040" if version == OLD else "wrapper", ""),
            ("@xai-official/grok-" + target, "grok-" + target + ("-1040" if version == OLD else ""), "node_modules/@xai-official/grok-" + target),
            ("@iarna/toml", "toml", "node_modules/@iarna/toml")]


def download_inputs(destination, contract, target):
    # 云端没有历史档案时才获取固定原包；本机传 --official-inputs 时完全不调用这里。
    destination.mkdir(mode=0o700)
    for version in (OLD, TARGET):
        for package,slug,_ in package_coordinates(target, version):
            package_version = "3.0.0" if package == "@iarna/toml" else version
            metadata_path, archive_path = destination / (slug + "-metadata.json"), destination / (slug + ".tgz")
            if metadata_path.exists():
                continue
            expected = contract["packages"][package + "@" + package_version]
            metadata_url = "https://registry.npmjs.org/" + package + "/" + package_version
            archive_url = "https://registry.npmjs.org/" + package + "/-/" + package.rsplit("/", 1)[1] + "-" + package_version + ".tgz"
            download_exact(metadata_url, metadata_path, 1024 * 1024)
            metadata = json.loads(metadata_path.read_bytes())
            require(metadata.get("name") == package and metadata.get("version") == package_version and metadata["dist"]["integrity"] == expected["integrity"] and metadata["dist"]["tarball"] == archive_url, "download_fixed_metadata")
            download_exact(archive_url, archive_path, MAX_ARCHIVE)
    # 所有下载字节随后必须通过 official_inputs 的 SRI 和逐成员合同，不能仅信 HTTP 成功。
    return destination


def download_exact(url, destination, limit):
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, request, response, code, message, headers, new_url):
            raise ValueError("official_download_redirect")
    opener = urllib.request.build_opener(NoRedirect)
    with opener.open(url, timeout=120) as response:
        require(response.url == url and response.status == 200, "official_download_response")
        fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            length = 0
            while block := response.read(65536):
                length += len(block)
                require(length <= limit, "official_download_limit")
                stream.write(block)
            require(length > 0, "official_download_empty")
            stream.flush()
            os.fsync(stream.fileno())

def official_inputs(cache, contract, target):
    # 原始归档只读引用；逐文件和 SRI 都来自已审阅的固定合同。
    records, releases = {}, {}
    for version in (OLD, TARGET):
        keys = package_coordinates(target, version)
        members = {}
        archives = []
        for package, slug, prefix in keys:
            package_version = "3.0.0" if package == "@iarna/toml" else version
            expected = contract["packages"][package + "@" + package_version]
            metadata_path, archive_path = cache / (slug + "-metadata.json"), cache / (slug + ".tgz")
            metadata = json.loads(metadata_path.read_bytes())
            require(metadata.get("name") == package and metadata.get("version") == package_version, "official_metadata_identity")
            url = "https://registry.npmjs.org/" + package + "/-/" + package.rsplit("/", 1)[1] + "-" + package_version + ".tgz"
            require(metadata["dist"]["tarball"] == url and metadata["dist"]["integrity"] == expected["integrity"], "official_dist_binding")
            raw = archive_path.read_bytes()
            require(len(raw) <= MAX_ARCHIVE, "archive_size")
            files = archive_members(raw, expected["integrity"])
            embedded = json.loads(files["package.json"][0])
            require(all(embedded.get(key, {} if value == {} else None) == value for key,value in expected["manifest"].items()), "fixed_embedded_manifest")
            for field in ("bin", "scripts", "dependencies", "optionalDependencies", "os", "cpu", "libc"):
                require(metadata.get(field, {} if expected["manifest"].get(field) == {} else None) == expected["manifest"].get(field), "metadata_manifest_contract")
            require(set(files) == set(expected["files"]), "fixed_archive_member_set")
            for name, (content, executable) in files.items():
                wanted = expected["files"][name]
                require(len(content) == wanted["length"] and hashlib.sha256(content).hexdigest() == wanted["sha256"] and executable == wanted["executable"], "fixed_archive_member_digest")
                members[(prefix + "/" if prefix else "") + name] = (content, executable)
            records["https://registry.npmjs.org/" + package + "/" + package_version] = binding(metadata_path)
            records[url] = binding(archive_path)
            archives.append(records[url]["sha256"])
        releases[version] = {"members":members, "archives":archives, "native":contract["native"][version][target]}
    return records, releases


def clone_file(source, destination):
    # APFS clone 不共享 inode；失败才回退普通复制，绝不使用 hardlink。
    if platform.system() == "Darwin":
        library = ctypes.CDLL(None, use_errno=True)
        if library.clonefile(os.fsencode(source), os.fsencode(destination), 0) == 0:
            return str(destination)
        require(not Path(destination).exists(), "clone_partial_destination")
    shutil.copy2(source, destination)
    return str(destination)


def write_members(root, members):
    for relative, (content, executable) in members.items():
        path = root / relative
        path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        write(path, content)
        path.chmod(0o755 if executable else 0o644)


def prepare_old(root, release, target, node):
    destination = root / "old-package"
    destination.mkdir(mode=0o755)
    write_members(destination, release["members"])
    # 只用绑定 Node 的 zlib 解压已验 Brotli；不加载 npm 包或运行 postinstall。
    compressed = destination / ("node_modules/@xai-official/grok-" + target + "/bin/grok.br")
    native = destination / "bin/grok-native"
    script = "const fs=require('fs'),z=require('zlib');const b=z.brotliDecompressSync(fs.readFileSync(process.argv[1]),{maxOutputLength:Number(process.argv[3])});fs.writeFileSync(process.argv[2],b,{flag:'wx',mode:0o755});"
    env = {name:str(root / part) for name, part in ENV_PATHS.items()}
    for part in set(ENV_PATHS.values()):
        (root / part).mkdir(mode=0o700, parents=True, exist_ok=True)
    env.update(PATH="/usr/bin:/bin", LANG="C", LC_ALL="C")
    require(sha(node["path"]) == node["sha256"], "node_changed_before_decompression")
    result = subprocess.run([node["path"], "-e", script, str(compressed), str(native), str(release["native"]["length"])], cwd=root, env=env, capture_output=True, timeout=120, check=False)
    write(root / "brotli.stdout", result.stdout)
    write(root / "brotli.stderr", result.stderr)
    require(result.returncode == 0 and native.stat().st_size == release["native"]["length"] and sha(native) == release["native"]["sha256"], "brotli_native_binding")
    (destination / "bin/grok").unlink()
    os.symlink("./grok-native", destination / "bin/grok")
    write(root / "old-materialization.safe.json", {"version":OLD,"target":target,"public_sha256":sha(native),"install_scripts_executed":False,"node_sha256":node["sha256"]})
    return destination


def fixture(args, case, old, sources, binaries, inputs):
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-npm-live-", dir=args.output)).resolve()
    for part in set(ENV_PATHS.values()) | {"project","tools","npm/cache","prefix/bin","home/.grok/bin"}:
        (root / part).mkdir(mode=0o700, parents=True, exist_ok=True)
    write(root / ".infinishell-npm-live", MARKER)
    write(root / "npm/user.npmrc", ("prefix=" + str(root / "prefix") + "\nregistry=https://registry.npmjs.org/\nignore-scripts=true\naudit=false\nfund=false\n").encode())
    write(root / "npm/global.npmrc", b"")
    write(root / "home/.grok/config.toml", b'[cli]\ninstaller = "npm"\nchannel = "stable"\n')
    package = root / "prefix/lib/node_modules/@xai-official/grok"
    package.parent.mkdir(mode=0o755, parents=True)
    shutil.copytree(old, package, symlinks=True, copy_function=clone_file)
    (package / "bin/grok-native").chmod(0o750)
    (package / "README.md").chmod(0o400)
    clone_file(package / "bin/grok-native", root / ("home/.grok/bin/grok-" + OLD))
    os.symlink("grok-" + OLD, root / "home/.grok/bin/grok")
    # 无关历史文件验证多目录恢复不会顺带清理用户保留的数据。
    write(root / "home/.grok/bin/unrelated-history", b"preserve this unrelated history\n")
    os.symlink("../lib/node_modules/@xai-official/grok/bin/grok", root / "prefix/bin/grok")
    os.symlink(binaries["node"]["path"], root / "tools/node")
    os.symlink(binaries["npm_cli"]["path"], root / "tools/npm")
    manifest = dict(schema=1, scope=SCOPE, case=case, root=str(root), old_public_sha256=sha(package / "bin/grok-native"), source_sha256=sources, inputs=inputs, **binaries)
    write(root / "manifest.private.json", manifest)
    return root, manifest


def installed_contract(release):
    files = {name:{"length":len(content), "sha256":hashlib.sha256(content).hexdigest()} for name,(content, _) in release["members"].items() if name != "bin/grok"}
    files["bin/grok-native"] = release["native"]
    return files


def verify_tree(root, expected, external=False):
    actual = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            require(relative == "bin/grok" and os.readlink(path) == "./grok-native", "published_tree_link")
        elif stat.S_ISREG(metadata.st_mode):
            require(metadata.st_nlink == 1, "published_tree_hardlink")
            if external and relative == "external-npm-change":
                require(path.read_bytes() == b"later external install must survive\n", "external_change_bytes")
                continue
            actual.add(relative)
            require(relative in expected and metadata.st_size == expected[relative]["length"] and sha(path) == expected[relative]["sha256"], "published_member_digest")
        else:
            require(stat.S_ISDIR(metadata.st_mode), "published_tree_special")
    require(actual == set(expected) and (root / "bin/grok").is_symlink(), "published_member_set")
    return len(actual)


def verify_product_archives(root, case, releases):
    journal = json.loads((root / "prepared-journal.safe.json").read_bytes())
    require([bytes(value).hex() for value in journal["archives"]] == releases[TARGET]["archives"], "product_archive_receipt_binding")
    expected = installed_contract(releases[TARGET])
    prepared = journal["prepared"]["tree"]["nodes"]
    files = {name:node for name,node in prepared.items() if node["sha256"] is not None}
    require(set(files) == set(expected) and journal["prepared"]["link"] is not None, "product_prepared_member_set")
    for name, wanted in expected.items():
        require(files[name]["length"] == wanted["length"] and bytes(files[name]["sha256"]).hex() == wanted["sha256"], "product_prepared_member_digest")
    version = OLD if case in ("swap_receipt_missing", "candidate_changed_preserved") else TARGET
    count = verify_tree(root / "prefix/lib/node_modules/@xai-official/grok", installed_contract(releases[version]), case == "external_change_preserved")
    mirror = root / "home/.grok/bin"
    require(os.readlink(mirror / "grok") == "grok-" + version, "mirror_link_version")
    require(sha(mirror / ("grok-" + version)) == releases[version]["native"]["sha256"], "mirror_native_binding")
    require(sha(mirror / ("grok-" + OLD)) == releases[OLD]["native"]["sha256"], "old_mirror_removed_or_changed")
    require((mirror / "unrelated-history").read_bytes() == b"preserve this unrelated history\n", "unrelated_history_changed")
    if version == OLD:
        require(not (mirror / ("grok-" + TARGET)).exists(), "uncommitted_mirror_native_retained")
    require((root / "home/.grok/config.toml").read_bytes() == b'[cli]\ninstaller = "npm"\nchannel = "stable"\n', "user_config_changed")
    write(root / "independent-archive-check.safe.json", {"sri_verified":True,"prepared_members":len(files),"published_members":count,
        "target":TARGET,"old":OLD,"public_version":version,"user_bin_checked":True,"global_installation_changed":False})


def verify_run_bindings(repo, binaries, sources):
    require(all(sha(record["path"]) == record["sha256"] for record in binaries.values()), "binary_changed_during_run")
    require(all(sha(repo / relative) == digest for relative,digest in sources.items()), "source_changed_during_run")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repo", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--official-inputs", type=Path, help="已有原包目录；省略时从固定官方 URL 下载并封存")
    for role in ("test-binary", "supervisor", "node", "npm-cli"):
        parser.add_argument("--" + role, type=Path, required=True)
        parser.add_argument("--" + role + "-sha256", required=True)
    parser.add_argument("--case", choices=CASES, action="append")
    args = parser.parse_args()
    target = target_platform()
    args.repo = args.repo.resolve(strict=True)
    if args.official_inputs is not None:
        args.official_inputs = args.official_inputs.resolve(strict=True)
    require(not args.output.exists(), "output_must_be_new")
    args.output.mkdir(mode=0o700, parents=True)
    args.output = args.output.resolve(strict=True)
    require(all(ord(character) >= 32 for character in str(args.output)), "prefix_has_control_characters")
    require(not str(args.output).startswith("/Volumes/"), "runtime_requires_internal_disk")
    require(shutil.disk_usage(args.output).free > 3 * 1024**3, "fixture_space_insufficient")
    binaries = {key:binding(getattr(args, option), getattr(args, option + "_sha256")) for key,option in (("worker","test_binary"),("supervisor","supervisor"),("node","node"),("npm_cli","npm_cli"))}
    require(all(not value["path"].startswith("/Volumes/") for value in binaries.values()), "executables_require_internal_disk")
    npm_metadata = json.loads((Path(binaries["npm_cli"]["path"]).parent.parent / "package.json").read_bytes())
    require(npm_metadata.get("name") == "npm" and npm_metadata.get("bin", {}).get("npm") == "bin/npm-cli.js", "npm_registration")
    source = {relative:sha(args.repo / relative) for relative in SOURCE_FILES}
    require(source[SOURCE_FILES[-1]] == sha(Path(__file__)), "runner_source_binding")
    verify_embedded(binaries["supervisor"]["path"], args.repo)
    if platform.system() == "Darwin":
        result = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", binaries["supervisor"]["path"]], capture_output=True, check=False)
        write(args.output / "supervisor-signature.stdout", result.stdout)
        write(args.output / "supervisor-signature.stderr", result.stderr)
        require(result.returncode == 0, "supervisor_signature")
    revision = subprocess.run(["git", "-C", str(args.repo), "rev-parse", "HEAD"], capture_output=True, check=True, text=True).stdout.strip()
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "source_commit_invalid")
    dirty = bool(subprocess.run(["git", "-C", str(args.repo), "status", "--porcelain", "--untracked-files=no"], capture_output=True, check=True).stdout)
    contract = json.loads((args.repo / "script/cli-agent-parity/grok_1041_npm_manifest.json").read_bytes())
    if args.official_inputs is None:
        args.official_inputs = download_inputs(args.output / "official-inputs", contract, target)
    inputs, releases = official_inputs(args.official_inputs, contract, target)
    write(args.output / "source.safe.json", {"commit":revision,"working_tree_dirty":dirty,"source_sha256":source,"binaries":binaries,
        "official_inputs":inputs,"platform":target,"old_version":OLD,"target_version":TARGET,"consumer_channel_resolution_covered":False,
        "release_selection":"test-only fixed official 1.0.41; current npm latest not queried"})
    materialization = args.output / "materialization"
    materialization.mkdir(mode=0o700)
    old = prepare_old(materialization, releases[OLD], target, binaries["node"])
    results = []
    for case in args.case or CASES:
        root, manifest = fixture(args, case, old, source, binaries, inputs)
        env = environment(root, manifest, "execute")
        if not results:
            for index,test in enumerate(BUSY_TESTS):
                run_test(binaries["worker"]["path"], test, root / "project", env, args.output / ("busy-gate-" + str(index)))
        run_test(binaries["worker"]["path"], TEST, root / "project", env, root / "execute", True)
        result = json.loads((root / "result-execute.safe.json").read_bytes())
        if case in ("swap_receipt_missing", "external_change_preserved"):
            require(result.get("needs_cold_recovery") is True and result.get("accepted") is False, "checkpoint_not_observed")
            run_test(binaries["worker"]["path"], TEST, root / "project", environment(root, manifest, "recover"), root / "recover", True)
            result = json.loads((root / "result-recover.safe.json").read_bytes())
        require(result.get("accepted") is True, "case_not_accepted")
        verify_product_archives(root, case, releases)
        results.append({"case":case,"root":str(root),"result":result})
    # 原始归档如在本轮发生变化，不能把已生成的收据判为整轮成功。
    require(all(sha(value["path"]) == value["sha256"] for value in inputs.values()), "official_cache_changed")
    verify_run_bindings(args.repo, binaries, source)
    write(args.output / "summary.safe.json", {"scope":SCOPE,"accepted":True,"cases":results,"model_inputs_sent":0,
        "busy_scope":"existing product model state-machine tests only","plugin_recheck_covered":False,
        "g09_closed":False,"consumer_channel_resolution_covered":False,"consumer_channels_unchanged":True})


if __name__ == "__main__":
    main()
