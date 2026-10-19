"""仅为 Windows PATH 回归准备固定编译器；不安装工具或执行产品。"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import urllib.parse
import urllib.request
import zipfile

sys.dont_write_bytecode = True
_source = Path(__file__).with_name('reissue_release_installer.py')
_spec = importlib.util.spec_from_file_location('environment_compiler_source', _source)
_reissue = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_reissue)
COMPILER = _reissue.COMPILER

# 与 verify_release_installer.ps1 的只读解包工具保持同一冻结来源。
TOOL = {
    'version': '2.71.1',
    'url': 'https://raw.githubusercontent.com/jrathlev/InnoUnpacker-Windows-GUI/b94b34935ff4b1019359462c154509de9fd951b9/innounp-2/bin/innounp-2.zip',
    'archive_sha256': 'f2f037fdbc63de31248efae9ccb294398d160dc0cbad5f36d14c2f159e17bbf5',
    'archive_size': 618116,
    'exe_sha256': '94377351f03257b06e221285b16d41108040151ab672b96faf1d3d874a7d0a32',
    'exe_size': 1790976,
}
TOOL_MEMBERS = {'innounp-cl.txt', 'innounp.css', 'innounp.exe', 'innounp.htm', 'innounp.ico'}
ISCC_SIZE = 1455248
MAX_COMPILER_BYTES = 128 * 1024**2


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def physical(path):
    path = Path(path)
    require(path.is_absolute() and '..' not in path.parts, '必须使用无父级跳转的绝对路径')
    if os.name == 'nt':
        require(re.fullmatch(r'[A-Za-z]:', path.drive) is not None, '只支持本机盘符路径')
    for entry in (path, *path.parents):
        try:
            info = entry.lstat()
        except FileNotFoundError:
            continue
        require(not stat.S_ISLNK(info.st_mode)
                and not getattr(info, 'st_file_attributes', 0) & 0x400,
                '拒绝符号链接或重解析点')
        require(stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode), '拒绝特殊文件')
        if stat.S_ISREG(info.st_mode):
            require(info.st_nlink == 1, '拒绝硬链接文件')
    return path


def sha(path):
    with physical(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def verified_file(path, size, digest):
    physical(path)
    require(path.is_file() and path.stat().st_size == size and sha(path) == digest,
            '固定文件大小或 SHA-256 不符')


def save(path, value):
    with physical(path).open('x', encoding='utf-8', newline='\n') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write('\n')


class HttpsRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        require(urllib.parse.urlsplit(newurl).scheme == 'https', '下载跳转必须保持 HTTPS')
        return super().redirect_request(request, fp, code, message, headers, newurl)


def download(url, path, size, digest):
    require(urllib.parse.urlsplit(url).scheme == 'https', '下载地址必须使用 HTTPS')
    physical(path)
    request = urllib.request.Request(url, headers={'User-Agent': 'InfiniShell-PATH-test-compiler'})
    opener = urllib.request.build_opener(HttpsRedirect())
    # 只下载公开固定来源，不读取或发送 GitHub 凭据，也不保存临时重定向 URL。
    with opener.open(request, timeout=120) as response, path.open('xb') as stream:
        require(urllib.parse.urlsplit(response.url).scheme == 'https', '最终下载地址必须使用 HTTPS')
        total = 0
        while chunk := response.read(min(1024**2, size - total + 1)):
            total += len(chunk)
            require(total <= size, '下载超过固定大小')
            stream.write(chunk)
        stream.flush()
        os.fsync(stream.fileno())
    verified_file(path, size, digest)


def unpack_tool(archive_path, directory):
    physical(directory).mkdir()
    with zipfile.ZipFile(physical(archive_path)) as archive:
        entries = archive.infolist()
        names = [entry.filename for entry in entries]
        require(len(names) == len(set(name.casefold() for name in names))
                and set(names) == TOOL_MEMBERS, '解包器 ZIP 必须精确匹配五个固定成员')
        require(sum(entry.file_size for entry in entries) <= 4 * 1024**2, '解包器 ZIP 内容过大')
        for entry in entries:
            require(not entry.is_dir() and not entry.flag_bits & 1
                    and stat.S_IFMT(entry.external_attr >> 16) in (0, stat.S_IFREG)
                    and not entry.external_attr & 0x400, '解包器 ZIP 包含特殊、加密或重解析成员')
        require(archive.testzip() is None, '解包器 ZIP CRC 不符')
        for entry in entries:
            target = physical(directory / entry.filename)
            with archive.open(entry) as source, target.open('xb') as output:
                while chunk := source.read(1024**2):
                    output.write(chunk)
    verified_file(directory / 'innounp.exe', TOOL['exe_size'], TOOL['exe_sha256'])


def compiler_members(text):
    members = _reissue.application_members(text)
    require(len(members) <= 4096, '编译器成员数量超过上限')
    names = set()
    for member in members:
        parts = member.replace('\\', '/').split('/')
        for part in parts[1:]:
            require(not any(ord(char) < 32 or char in '<>:"|?*' for char in part)
                    and part.split('.')[0].upper() not in
                    {'CON', 'PRN', 'AUX', 'NUL', *('COM' + str(i) for i in range(1, 10)),
                     *('LPT' + str(i) for i in range(1, 10))}, '编译器成员包含 Windows 非法文件名')
        normalized = '/'.join(parts)
        require(normalized.casefold() not in names, '编译器成员存在别名冲突')
        names.add(normalized.casefold())
    require('{app}/iscc.exe' in names, '编译器成员缺少 ISCC.exe')
    return {member.replace('\\', '/') for member in members}


def inventory(directory, expected):
    physical(directory)
    files = {}
    seen = set()
    directories = {str(Path(name).parent).replace('\\', '/') for name in expected}
    directories |= {parent.as_posix() for name in expected for parent in Path(name).parents
                    if parent != Path('.')}
    for parent, subdirs, names in os.walk(directory, followlinks=False):
        for name in [*subdirs, *names]:
            path = physical(Path(parent) / name)
            relative = path.relative_to(directory).as_posix()
            require(relative.casefold() not in seen, '提取内容存在大小写冲突')
            seen.add(relative.casefold())
            if path.is_dir():
                require(relative in directories, '提取结果包含未知目录')
            else:
                require(relative in expected, '提取结果包含未知文件')
                files[relative] = {'size': path.stat().st_size, 'sha256': sha(path)}
    require(set(files) == expected, '编译器提取成员集合不符')
    require(sum(row['size'] for row in files.values()) <= MAX_COMPILER_BYTES, '提取编译器超过大小上限')
    return files


def run_tool(root, label, arguments):
    tool = root / 'tool/innounp.exe'
    verified_file(tool, TOOL['exe_size'], TOOL['exe_sha256'])
    distribution = root / 'compiler-distribution.exe'
    verified_file(distribution, COMPILER['distribution_size'], COMPILER['distribution_sha256'])
    stdout = root / (label + '.stdout.log')
    stderr = root / (label + '.stderr.log')
    with physical(stdout).open('xb') as out, physical(stderr).open('xb') as err:
        result = subprocess.run([str(tool), *arguments], cwd=root, stdin=subprocess.DEVNULL,
                                stdout=out, stderr=err, timeout=120, check=False,
                                creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    require(result.returncode == 0, '只读解包器执行失败')
    require(stdout.stat().st_size <= 2 * 1024**2 and stderr.stat().st_size <= 2 * 1024**2,
            '只读解包器日志超过上限')
    verified_file(tool, TOOL['exe_size'], TOOL['exe_sha256'])
    verified_file(distribution, COMPILER['distribution_size'], COMPILER['distribution_sha256'])
    return stdout


def prepare(output):
    root = physical(output)
    require(root.parent.is_dir(), '输出父目录必须已经存在')
    require(not root.exists(), '输出目录已存在；拒绝覆盖或复用')
    root.mkdir(mode=0o700)
    result = {'schema': 'infinishell-environment-test-compiler-v1', 'status': 'failed',
              'compiler': COMPILER, 'unpacker': TOOL,
              'script_sha256': sha(Path(__file__).absolute()), 'source_helper_sha256': sha(_source.absolute()),
              'product_execution': False, 'tool_installation': False, 'registry_modified': False}
    try:
        archive = root / 'innounp.zip'
        download(TOOL['url'], archive, TOOL['archive_size'], TOOL['archive_sha256'])
        unpack_tool(archive, root / 'tool')
        distribution = root / 'compiler-distribution.exe'
        download(COMPILER['url'], distribution, COMPILER['distribution_size'], COMPILER['distribution_sha256'])
        listing = run_tool(root, 'compiler-members', ['-s', '-h', '-b', '-o', '-u',
                                                    str(distribution), r'{app}\*'])
        expected = compiler_members(listing.read_text(encoding='utf-8-sig'))
        destination = root / 'compiler'
        physical(destination).mkdir()
        run_tool(root, 'compiler-extract', ['-x', '-b', '-q', '-o', '-u', '-d' + str(destination),
                                          str(distribution), r'{app}\*'])
        files = inventory(destination, expected)
        compiler = destination / '{app}/ISCC.exe'
        verified_file(compiler, ISCC_SIZE, COMPILER['iscc_sha256'])
        save(root / 'compiler-files.json', {'files': files, 'count': len(files)})
        result.update(status='passed', compiler_path=str(compiler),
                      compiler_files_sha256=sha(root / 'compiler-files.json'))
    except Exception as error:
        result['error_type'] = type(error).__name__
        raise
    finally:
        save(root / 'result.json', result)
    return compiler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    require(os.name == 'nt', '固定编译器只在 Windows runner 准备')
    print(prepare(args.output))


if __name__ == '__main__':
    main()
