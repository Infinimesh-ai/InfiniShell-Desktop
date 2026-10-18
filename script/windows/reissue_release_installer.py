"""正式旧版本安装器重发门禁；不运行产品、不更改发行页。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import urllib.request

APP_SOURCE = '5068dae9c75b79628865a0872684113647c204d8'
APP_VERSION = 'v0.2026.10.14.16.45.oss_00'
ORIGINAL_SHA = '1ad878fb098c8cc7e49c7dc29aa6afec7ceb34d80fcb9fc8220643f35ec3086a'
HELPER_SHA = 'dea6f701a22b3dff9197401410ddedeaaa70d40c909fc5e7ee01b6626e322cfd'
LEGACY_SHA = '630fb260603f0279b97d1e206bbb91688f5178aa64806832166b29366a01307a'
COMPILER = {
    'version': '6.7.1',
    'url': 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_1/innosetup-6.7.1.exe',
    'distribution_size': 10619024,
    'distribution_sha256': '4d11e8050b6185e0d49bd9e8cc661a7a59f44959a621d31d11033124c4e8a7b0',
    'iscc_sha256': 'eb6f4410c8db367a5f74127e8025ad2ccacc0afabbe783959d237df3050f97fb',
}
INPUTS = [
    'script/windows/windows-installer.iss', 'script/windows/environment.iss',
    'script/windows/windows-registration.iss',
    'script/windows/installer-images/warp-logo.bmp', 'script/windows/installer-images/warp-banner.bmp',
    'app/channels/oss/icon/padded/icon.ico',
    *['app/assets/windows/x64/' + name for name in (
        'conpty.dll', 'OpenConsole.exe', 'vcruntime140.dll', 'vcruntime140_1.dll',
        'msvcp140.dll', 'dxcompiler.dll', 'dxil.dll')],
]
TESTS = ['script/windows/' + name for name in (
    'environment-tests.iss', 'environment-action-tests.iss', 'environment-legacy-r4.iss',
    'test_windows_environment.ps1', 'test_windows_environment_reissue.ps1',
    'reissue_release_installer.ps1', 'reissue_release_installer.py', 'reissue_release_installer_tests.py',
    'verify_release_installer.ps1')]


def require(ok, reason):
    if not ok:
        raise RuntimeError(reason)


def physical(path):
    path = Path(path).absolute()
    for item in [path, *path.parents]:
        if item.exists():
            require(not item.is_symlink() and not item.stat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT,
                    '路径存在重解析点：' + str(item))
    return path


def sha(path):
    with physical(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(physical(path).read_text(encoding='utf-8-sig'))


def save(path, value):
    physical(path).parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8', newline='\n') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write('\n')


def copy(source, target):
    physical(source)
    physical(target).parent.mkdir(parents=True, exist_ok=True)
    require(not target.exists(), '拒绝覆盖：' + str(target))
    shutil.copy2(source, target)


def row(path, root):
    return {'file': path.relative_to(root).as_posix(), 'size': path.stat().st_size, 'sha256': sha(path)}


def inventory(directory):
    files = {}
    for path in sorted(directory.rglob('*')):
        physical(path)
        if path.is_file():
            name = path.relative_to(directory).as_posix()
            require(name.casefold() not in {key.casefold() for key in files}, '文件名大小写冲突')
            files[name] = {'size': path.stat().st_size, 'sha256': sha(path)}
    return files


def verify_source(verified):
    result = read(verified / 'result.json')
    identity = read(verified / 'candidate-identity.json')
    require(result['status'] == 'readonly_final_installer_artifact_verification_passed' and result['error'] is None,
            '原正式工件验证尚未成功')
    require(identity['controller_sha'] == result['controller_sha'] == os.environ['GITHUB_SHA'], '控制器提交不匹配')
    require(identity['artifact_source_sha'] == result['artifact_source_sha'] == APP_SOURCE and identity['tag'] == APP_VERSION,
            '本模式仅重发已验收的旧应用版本')
    require(identity['release_run'] == 38537451605 and identity['tag_object'] == '27d40029759b541dc8903a0deb9ec0948e9670fa',
            '原正式 run/tag 身份不符')
    require(result['identity_sha256'] == sha(verified / 'candidate-identity.json'), '来源身份摘要不符')
    require(read(verified / 'official-recheck-result.json')['status'] == 'official_source_and_local_inputs_unchanged', '缺少来源后核')
    require(sha(verified / 'windows/InfiniShellSetup.exe') == ORIGINAL_SHA, '原始安装器字节不符')
    require(sha(verified / 'tool/innounp.exe') == identity['tool']['exe_sha256'] ==
            '94377351f03257b06e221285b16d41108040151ab672b96faf1d3d874a7d0a32', '解包工具摘要不符')
    for role in ('windows', 'bundle'):
        source = identity['sources'][role]
        require('sha256:' + sha(verified / (role + '.artifact.zip')) == source['artifact']['digest'], '来源 ZIP 已变化')
        for item in source['extracted']:
            path = verified / role / item['name']
            require(path.stat().st_size == item['size'] and sha(path) == item['sha256'], '原始提取载荷已变化')
    require(sha(verified / 'ci-original-manifest.json') == identity['ci_manifest_sha256'], '原始 manifest 已变化')
    return identity


def prepare(args):
    root, verified, repo = args.output, args.verified, args.repo
    identity = verify_source(verified)
    head = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=repo, capture_output=True, text=True, check=True).stdout.strip()
    require(head == os.environ['GITHUB_SHA'], 'checkout 与控制器提交不匹配')
    require(subprocess.run(['git', 'merge-base', '--is-ancestor', APP_SOURCE, head], cwd=repo).returncode == 0, '旧应用不是控制器祖先')
    tracked = subprocess.run(['git', 'ls-files', '--error-unmatch', '--', *INPUTS, *TESTS], cwd=repo, capture_output=True)
    require(tracked.returncode == 0, '构建与测试输入必须全部已提交')
    require(subprocess.run(['git', 'diff', '--quiet', 'HEAD', '--', *INPUTS, *TESTS], cwd=repo).returncode == 0, '构建与测试输入有本地改动')
    require(sha(repo / INPUTS[1]) == HELPER_SHA and sha(repo / 'script/windows/environment-legacy-r4.iss') == LEGACY_SHA,
            '新旧 helper 不是已验收版本')
    source_rows = []
    for name in INPUTS + TESTS:
        target = root / 'source' / name
        copy(repo / name, target)
        source_rows.append(row(target, root))
    save(root / 'evidence/source-inputs.json', {'controller_sha': head, 'installer_inputs': INPUTS, 'files': source_rows})
    for path in sorted(verified.iterdir()):
        if path.is_file() and path.suffix in ('.json', '.log', '.py'):
            copy(path, root / 'evidence/original-verification' / path.name)
    save(root / 'evidence/source-binding.json', {
        'controller_sha': head, 'application_source_sha': APP_SOURCE, 'application_version': APP_VERSION,
        'original_installer_sha256': ORIGINAL_SHA, 'identity': identity,
        'verified_result_sha256': sha(verified / 'result.json'), 'compiler': COMPILER,
        'expected_installer_sha256': args.expected,
    })
    target = root / 'compiler-distribution.exe'
    request = urllib.request.Request(COMPILER['url'], headers={'User-Agent': 'InfiniShell-installer-reissue'})
    # 下载公开固定版本，不发送 API token；摘要与大小双重校验后才调用解包器。
    with urllib.request.urlopen(request, timeout=120) as response, target.open('xb') as stream:
        require(response.url.startswith('https://'), '编译器下载必须为 HTTPS')
        size = 0
        while chunk := response.read(1024 * 1024):
            size += len(chunk)
            require(size <= COMPILER['distribution_size'], '编译器下载超出固定大小')
            stream.write(chunk)
    require(target.stat().st_size == COMPILER['distribution_size'] and sha(target) == COMPILER['distribution_sha256'], '编译器发行原件不符')


def application_members(text):
    # -h 只隐藏程序横幅；-s 仍列出归档说明和未被提取掩码选择的成员。
    members, all_names = [], set()
    for line in text.splitlines():
        name = line.strip()
        if not name or re.fullmatch(r'Inno Setup archive:\s+(?:compiler-distribution|InfiniShellSetup)\.exe', name) or re.fullmatch(r'Inno Setup version detected:\s+[0-9.]+ \(Unicode\)', name):
            continue
        require(name.casefold() not in all_names, '成员列表重名')
        all_names.add(name.casefold())
        if name == 'install_script.iss':
            continue
        parts = name.replace('\\', '/').split('/')
        require(parts[0] in ('{app}', '{tmp}') and len(parts) > 1 and
                all(part not in ('', '.', '..') and ':' not in part and not part.endswith((' ', '.')) for part in parts),
                '拒绝未知或不安全成员：' + name)
        if parts[0] == '{app}':
            members.append(name)
    require(members, '应用成员列表为空')
    return members


def listing(args):
    text = (args.output / 'evidence' / (args.label + '.stdout.log')).read_text(encoding='utf-8-sig')
    members = application_members(text)
    if args.label != 'compiler-members':
        require(len(members) == 150, '应用载荷必须精确为 150 件')
    save(args.output / 'evidence' / (args.label + '.json'), {'members': members, 'count': len(members)})


def stage(args):
    root = args.output
    compiler = root / 'compiler/{app}/ISCC.exe'
    require(sha(compiler) == COMPILER['iscc_sha256'], 'ISCC 字节不符')
    files = inventory(root / 'compiler')
    require(set(files) == {name.replace('\\', '/') for name in read(root / 'evidence/compiler-members.json')['members']}, '编译器提取集合不符')
    save(root / 'evidence/compiler-files.json', {'compiler': COMPILER, 'files': files})
    original = inventory(root / 'original-payload/{app}')
    require(len(original) == 150, '原应用载荷数量不符')
    require(set(original) == {name.replace('\\', '/')[6:] for name in read(root / 'evidence/original-members.json')['members']}, '原应用提取集合不符')
    # 仓库运行库/图标必须与原包完全相同；只编译安装逻辑，不生成远端资源或主程序。
    for name in INPUTS[5:]:
        relative = 'icon.ico' if name.endswith('/icon.ico') else ('x64/OpenConsole.exe' if name.endswith('/OpenConsole.exe') else Path(name).name)
        require({'size': (root / 'source' / name).stat().st_size, 'sha256': sha(root / 'source' / name)} == original[relative],
                '仓库运行库或图标已改变：' + name)
    require(original['resources/remote-server/manifest.json']['sha256'] == read(root / 'evidence/source-binding.json')['identity']['ci_manifest_sha256'], '提取的内嵌 manifest 不符')
    save(root / 'evidence/original-payload.json', {'count': len(original), 'files': original})


def validate_tests(args):
    root = args.output
    full = read(root / 'test-output/full/result.json')
    fixed = {'status': 'passed', 'error': None, 'native_matrix_skipped': False, 'native_matrix_cases': 200,
             'native_edge_cases': 38, 'assertions': 1615, 'native_actions': 1378, 'native_step_assertions': 2544,
             'real_user_path_unchanged': True, 'cleanup_passed': True}
    for key, value in fixed.items():
        require(full.get(key) == value, '完整 PATH 回归字段不符：' + key)
    require(len(full['verified_cases']) == 362 and {item['case'] for item in full['known_limits']} ==
            {'native-limit-future-old-install', 'native-limit-missing-history'}, '完整回归场景/已知边界不符')
    reissue = read(root / 'test-output/reissue/result.json')
    fixed = {'status': 'passed', 'execution_error': None, 'positive_cases': 40, 'negative_control_cases': 2,
             'native_actions': 342, 'independent_step_snapshots': 342, 'assertions': 2476,
             'execution_context_hkcu_environment_path_unchanged': True, 'desktop_environment_path_verified': False,
             'cleanup_passed': True, 'positive_test_registry_cleanup_passed': True,
             'negative_registry_evidence_preserved': True, 'negative_actual_failed': 1, 'negative_actual_passed': 1}
    for key, value in fixed.items():
        require(reissue.get(key) == value, '重发回归字段不符：' + key)
    require(len(reissue['verified_cases']) == 40 and not reissue['failed_assertions'], '重发正例或断言不符')
    controls = reissue['negativecontrols']
    require(len(controls) == 2 and {item['name'] for item in controls} == {'negative-absent', 'negative-empty-expand'}, '负对照集合不符')
    for control in controls:
        absent = control['name'] == 'negative-absent'
        expected = 'failed' if absent else 'passed'
        require(control['role'] == 'unmodified_R4_future_installer' and
                control['expected_restoration_status'] == control['actual_restoration_status'] == expected and
                control['expected_actual_mismatch'] is absent and control['mismatch_observed'] is absent,
                '负对照不可混入修复正例')
        case = control['case_result']
        require(case['operations'] == ['old,add,A', 'new,add,B', 'new,remove,A', 'old,add,C', 'old,remove,B', 'old,remove,C'] and
                case['actual_final']['path'] == {'kind': 2, 'value': ''}, '原始旧版反例未保留')
    for folder in ('full', 'reissue'):
        require(sha(root / 'test-output' / folder / 'source/environment.iss') == HELPER_SHA and
                sha(root / 'test-output' / folder / 'source/environment-legacy-r4.iss') == LEGACY_SHA, '测试没有使用绑定的新旧 helper')
    save(root / 'evidence/path-regression-binding.json', {
        'status': 'passed', 'helper_sha256': HELPER_SHA, 'frozen_R4_sha256': LEGACY_SHA,
        'full_result': row(root / 'test-output/full/result.json', root),
        'reissue_result': row(root / 'test-output/reissue/result.json', root),
        'full_matrix_cases': 200, 'full_edge_cases': 38, 'full_known_limits': full['known_limits'],
        'reissue_positive_cases': 40, 'reissue_negativecontrols': controls,
        'execution_context_hkcu_environment_unchanged': True, 'desktop_path_verified': False,
    })


def finish(args):
    root = args.output
    identity = verify_source(args.verified)
    before = read(root / 'evidence/source-inputs.json')
    for item in before['files']:
        require(row(root / item['file'], root) == item, '冻结源码在构建期间改变')
        require(sha(args.repo / item['file'][7:]) == item['sha256'], 'checkout 输入在构建期间改变')
    require(inventory(root / 'compiler') == read(root / 'evidence/compiler-files.json')['files'], '编译器文件在使用期间改变')
    original = read(root / 'evidence/original-payload.json')['files']
    rebuilt = inventory(root / 'rebuilt-payload/{app}')
    require(original == inventory(root / 'original-payload/{app}') == rebuilt and len(rebuilt) == 150, '150 件应用载荷未逐字节保持原样')
    require(set(rebuilt) == {name.replace('\\', '/')[6:] for name in read(root / 'evidence/rebuilt-members.json')['members']}, '重编提取集合不符')
    setup = root / 'candidate/InfiniShellSetup.exe'
    actual = sha(setup)
    comparison = {'status': 'identical', 'file_count': 150, 'files': rebuilt,
                  'original_installer_sha256': ORIGINAL_SHA, 'rebuilt_installer_sha256': actual,
                  'expected_installer_sha256': args.expected, 'installer_matches_expected': actual == args.expected,
                  'ci_manifest_sha256': identity['ci_manifest_sha256']}
    save(root / 'evidence/payload-verification.json', comparison)
    require(actual == args.expected, '重编安装器不等于固定已验收候选；保留新字节，需独立候选与桌面验收')
    signature = read(root / 'evidence/installer-signature.json')
    require(signature['authenticode_status'] in ('Valid', 'NotSigned') and signature['installer_sha256'] == actual, '安装器签名回执不符')
    validate_binding = read(root / 'evidence/path-regression-binding.json')
    require(validate_binding['status'] == 'passed', '缺少本轮完整回归')
    result = {
        'schema': 'infinishell-windows-installer-reissue-v1', 'status': 'remote_reissue_build_and_regression_passed',
        'controller_sha': os.environ['GITHUB_SHA'], 'application_source_sha': APP_SOURCE, 'application_version': APP_VERSION,
        'original_release_run': identity['release_run'], 'original_tag_object': identity['tag_object'],
        'original_artifact': identity['sources']['windows']['artifact'], 'original_installer_sha256': ORIGINAL_SHA,
        'installer_sha256': actual, 'installer_size_bytes': setup.stat().st_size,
        'helper_sha256': HELPER_SHA, 'application_payload_files_unchanged': 150,
        'compiler': COMPILER, 'authenticode_status': signature['authenticode_status'],
        'test_scope': validate_binding, 'delivery': 'delivery/InfiniShellSetup.exe',
        'product_execution': 'not_performed', 'installer_execution': 'not_performed',
        'desktop_six_stage_acceptance': 'not_performed_by_this_job', 'release_mutation': 'not_performed',
        'guarantee_boundary': '未来新增产品使用重发安装/卸载逻辑；未修改 R4 EXE 的失败对照保持失败。仅串行隔离回归，无原地升级或 all-users 声明。',
        'evidence_bindings': [row(root / 'evidence' / name, root) for name in (
            'source-inputs.json', 'source-binding.json', 'compiler-files.json', 'compiler-signature.json',
            'path-regression-binding.json', 'payload-verification.json', 'installer-signature.json')],
    }
    copy(setup, root / 'delivery/InfiniShellSetup.exe')
    save(root / 'evidence/result.json', result)
    copy(root / 'evidence/result.json', root / 'delivery/reissue-binding.json')
    with (root / 'delivery/InfiniShellSetup.exe.sha256').open('x', encoding='ascii', newline='\n') as stream:
        stream.write(actual + '  InfiniShellSetup.exe\n')


def archive_evidence(args):
    root = args.output
    # 即使失败也归档完整文本证据，保留失败动作；排除编译器、产品与测试 EXE。
    copied = []
    for directory in ('source', 'test-output'):
        base = root / directory
        if base.exists():
            for path in sorted(base.rglob('*')):
                if path.is_file() and path.suffix.lower() in ('.json', '.jsonl', '.log', '.txt', '.iss', '.ps1', '.py', '.ini', '.exit'):
                    target = root / 'evidence' / path.relative_to(root)
                    copy(path, target)
                    copied.append(row(path, root))
    save(root / 'evidence/text-evidence-sources.json', {'files': copied})
    save(root / 'evidence/evidence-index.json', {'files': [row(path, root) for path in sorted((root / 'evidence').rglob('*')) if path.is_file()]})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['prepare', 'listing', 'stage', 'tests', 'finish', 'archive'])
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verified', type=Path, required=True)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--expected', required=True)
    parser.add_argument('--label')
    args = parser.parse_args()
    require(os.name == 'nt' and os.environ.get('GITHUB_REPOSITORY') == 'Infinimesh-ai/InfiniShell-Desktop' and
            re.fullmatch('[0-9a-f]{40}', os.environ.get('GITHUB_SHA', '')) and re.fullmatch('[0-9a-f]{64}', args.expected),
            '仅支持固定仓库 Windows CI 和确切提交/候选摘要')
    args.output, args.verified, args.repo = map(physical, (args.output, args.verified, args.repo))
    {'prepare': prepare, 'listing': listing, 'stage': stage, 'tests': validate_tests,
     'finish': finish, 'archive': archive_evidence}[args.action](args)


if __name__ == '__main__':
    main()
