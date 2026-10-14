#requires -Version 5.1
<# 仅在 preflight 读取正式 CI 原件、Authenticode 与精确内嵌资源。
不安装或运行产品，不修改注册表、系统服务、用户配置或已冻结发行产物。 #>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^[1-9][0-9]*$')][string]$ReleaseRun,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT' -or $env:GITHUB_REPOSITORY -cne 'Infinimesh-ai/InfiniShell-Desktop' -or $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$') { throw '仅支持正式仓库 Windows preflight 的确切控制器提交。' }
function Assert-Physical([string]$Path) {
    $cursor = [IO.Path]::GetFullPath($Path)
    if ($cursor -notmatch '^[A-Za-z]:\\') { throw '必须使用本机盘符路径。' }
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            if (((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw '证据路径存在重解析点。' }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Write-NewJson([string]$Path, $Value) {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 30))
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
function File-Sha([string]$Path) {
    Assert-Physical $Path
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
$root = [IO.Path]::GetFullPath($OutputDirectory)
Assert-Physical $root
if (Test-Path -LiteralPath $root) { throw '证据目录已存在；保留旧轮次，不覆盖或自动重跑。' }
$null = New-Item -ItemType Directory -Path $root -ErrorAction Stop
$python = (Get-Command python.exe -ErrorAction Stop).Source
$acquirerPath = Join-Path $root 'artifact-controller.py'
$acquirer = @'
"""固定仓库的只读正式工件来源、提供方 ZIP 与安全解包门禁。"""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import subprocess
import sys
import urllib.error
import urllib.request
import zipfile

REPO = 'Infinimesh-ai/InfiniShell-Desktop'
API = 'https://api.github.com/repos/' + REPO + '/'
ROOT = Path(sys.argv[2]).resolve(strict=True)
RUN = int(sys.argv[3])
CONTROLLER = os.environ['GITHUB_SHA']
NAMES = ['infinishell-linux-x86_64.tar.gz', 'infinishell-linux-aarch64.tar.gz', 'infinishell-macos-x86_64.tar.gz', 'infinishell-macos-aarch64.tar.gz', 'infinishell-windows-x86_64.zip', 'infinishell-windows-aarch64.zip']
ROLES = {
    'windows': ('infinishell-windows-x64', 'Build Release (Windows x64)', 'Upload installer artifact'),
    'bundle': ('infinishell-bundled-remote-server-resources', 'Prepare bundled remote-server resources', 'Upload bundled remote-server resources'),
    'linux-x86_64': ('infinishell-cli-linux-x86_64', 'Build remote-server (Linux)', 'Upload x86_64 remote-server artifact'),
    'linux-aarch64': ('infinishell-cli-linux-aarch64', 'Build remote-server (Linux)', 'Upload aarch64 remote-server artifact'),
    'macos-x86_64': ('infinishell-cli-macos-x86_64', 'Build remote-server (macOS x86_64)', 'Upload remote-server artifact'),
    'macos-aarch64': ('infinishell-cli-macos-aarch64', 'Build remote-server (macOS aarch64)', 'Upload remote-server artifact'),
    'windows-x86_64': ('infinishell-cli-windows-x86_64', 'Build remote-server (Windows x86_64)', 'Upload remote-server artifact'),
    'windows-aarch64': ('infinishell-cli-windows-aarch64', 'Build remote-server (Windows aarch64)', 'Upload remote-server artifact'),
}
TOOL = {'version': '2.71.1', 'url': 'https://raw.githubusercontent.com/jrathlev/InnoUnpacker-Windows-GUI/b94b34935ff4b1019359462c154509de9fd951b9/innounp-2/bin/innounp-2.zip', 'archive_sha256': 'f2f037fdbc63de31248efae9ccb294398d160dc0cbad5f36d14c2f159e17bbf5', 'archive_size': 618116, 'exe_sha256': '94377351f03257b06e221285b16d41108040151ab672b96faf1d3d874a7d0a32', 'exe_size': 1790976}

def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(name, value):
    with (ROOT / name).open('x', encoding='utf-8') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write('\n')


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def request(url, authenticated):
    headers = {'User-Agent': 'InfiniShell-readonly-release-verification', 'Accept': 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28'}
    if authenticated:
        require(url.startswith(API), '认证请求超出固定仓库 API')
        headers['Authorization'] = 'Bearer ' + os.environ['GH_TOKEN']
    opener = urllib.request.build_opener(NoRedirect())
    try:
        return opener.open(urllib.request.Request(url, headers=headers), timeout=120)
    except urllib.error.HTTPError as exc:
        if exc.code in (301, 302, 303, 307, 308):
            target = exc.headers.get('Location', '')
            require(target.startswith('https://'), '下载跳转必须为 HTTPS')
            # 不把 API token 发给工件存储或任何重定向地址，也不归档带签名的 URL。
            return urllib.request.urlopen(urllib.request.Request(target, headers={'User-Agent': headers['User-Agent']}), timeout=120)
        raise


def api(path, filename):
    with request(API + path, True) as response:
        raw = response.read(16 * 1024 * 1024 + 1)
    require(len(raw) <= 16 * 1024 * 1024, '官方 JSON 超过上限')
    (ROOT / filename).write_bytes(raw)
    return json.loads(raw)


def pages(path, key, prefix):
    values = []
    for number in range(1, 21):
        data = api(path + ('&' if '?' in path else '?') + f'per_page=100&page={number}', f'{prefix}-{number}.json')
        values += data[key]
        if len(data[key]) < 100:
            require(len(values) == data['total_count'], '官方分页数量不完整')
            return values
    raise RuntimeError('官方分页超过本轮有界范围')


def download(url, path, limit, authenticated):
    with request(url, authenticated) as response, path.open('xb') as output:
        total = 0
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            require(total <= limit, '下载超过冻结大小或证据上限')
            output.write(chunk)
        output.flush()
        os.fsync(output.fileno())
    return total


def unzip(path, directory, expected, max_total):
    directory.mkdir()
    result = []
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        names = [entry.filename for entry in entries]
        require(len(names) == len(set(name.casefold() for name in names)) and set(names) == set(expected), 'ZIP 成员必须精确匹配白名单，不允许重复')
        require(sum(entry.file_size for entry in entries) <= max_total, 'ZIP 解压总量超过上限')
        for entry in entries:
            mode = entry.external_attr >> 16
            require(not entry.is_dir() and not entry.flag_bits & 1 and stat.S_IFMT(mode) in (0, stat.S_IFREG), 'ZIP 目录、加密或特殊文件被拒绝')
            require(entry.filename in expected and not any(part in ('', '.', '..') or ':' in part or '\\' in part for part in entry.filename.split('/')), 'ZIP 路径不安全')
        require(archive.testzip() is None, 'ZIP CRC 不符')
        for entry in entries:
            target = directory / entry.filename
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(entry) as source, target.open('xb') as output:
                while chunk := source.read(1024 * 1024):
                    output.write(chunk)
            require(target.stat().st_size == entry.file_size and entry.file_size > 0, 'ZIP 解包长度不符')
            result.append({'name': entry.filename, 'size': entry.file_size, 'sha256': sha(target)})
    return result


def run_identity(run):
    require(run['id'] == RUN and run['repository']['full_name'] == REPO and run['event'] == 'push' and run['path'] == '.github/workflows/infinishell_release.yml', '不是指定仓库正式 tag 构建')
    require(run['status'] == 'completed' and run['conclusion'] == 'success', '正式构建必须已完整成功')
    require(re.fullmatch(r'[0-9a-f]{40}', run['head_sha']) and re.fullmatch(r'v[0-9]+(?:\.[0-9]+)+\.oss_[0-9]+', run['head_branch']), '正式 source/tag 格式不符')
    return {key: run[key] for key in ('id', 'head_sha', 'head_branch', 'run_attempt', 'event', 'path', 'status', 'conclusion')}


def artifact_identity(artifact, run, expected_name):
    require(artifact['name'] == expected_name and artifact['expired'] is False and type(artifact['id']) is int and artifact['id'] > 0, '工件角色、ID 或有效性不符')
    require(artifact['workflow_run']['id'] == RUN and artifact['workflow_run']['head_sha'] == run['head_sha'] and artifact['workflow_run']['head_branch'] == run['head_branch'], '工件 run/source/tag 不符')
    require(re.fullmatch(r'sha256:[0-9a-f]{64}', artifact['digest']) and 0 < artifact['size_in_bytes'] <= 2 * 1024**3, '工件提供方摘要或大小无效')
    return {key: artifact[key] for key in ('id', 'name', 'digest', 'size_in_bytes', 'expired', 'created_at', 'updated_at', 'workflow_run')}


def job_identity(job, run, role, artifact):
    require(job['run_id'] == RUN and job['head_sha'] == run['head_sha'] and job['name'] == ROLES[role][1], '生产 job 来源不符')
    require(job['status'] == 'completed' and job['conclusion'] == 'success' and 1 <= job['run_attempt'] <= run['run_attempt'], '生产 job 未成功或 attempt 不属本 run')
    steps = [step for step in job['steps'] if step['name'] == ROLES[role][2]]
    require(len(steps) == 1 and steps[0]['status'] == 'completed' and steps[0]['conclusion'] == 'success', '生产上传步骤未明确成功')
    require(job['started_at'] <= artifact['created_at'] <= job['completed_at'], '工件时间不属于生产 job')
    return {key: job[key] for key in ('id', 'run_id', 'run_attempt', 'head_sha', 'name', 'status', 'conclusion', 'started_at', 'completed_at', 'steps')}


def tag_identity(run, suffix):
    tag = run['head_branch']
    ref = api('git/ref/tags/' + tag, 'tag-ref-' + suffix + '.json')
    require(ref['object']['type'] == 'tag', '必须为 annotated tag')
    obj = api('git/tags/' + ref['object']['sha'], 'tag-object-' + suffix + '.json')
    require(obj['sha'] == ref['object']['sha'] and obj['tag'] == tag and obj['object']['type'] == 'commit' and obj['object']['sha'] == run['head_sha'], 'annotated tag 未绑定正式 source')
    return {'ref': ref, 'object': obj}


def acquire():
    run = api(f'actions/runs/{RUN}', 'run-before.json')
    bound_run = run_identity(run)
    tags = tag_identity(run, 'before')
    head = subprocess.run(['git', 'rev-parse', 'HEAD'], check=True, capture_output=True, text=True).stdout.strip()
    require(head == CONTROLLER, 'checkout 与 controller SHA 不符')
    ancestor = subprocess.run(['git', 'merge-base', '--is-ancestor', run['head_sha'], head], capture_output=True)
    require(ancestor.returncode == 0, 'artifact source 必须为 controller SHA 的祖先或同一提交')
    artifacts = pages(f'actions/runs/{RUN}/artifacts', 'artifacts', 'artifacts-before')
    jobs = pages(f'actions/runs/{RUN}/jobs?filter=all', 'jobs', 'jobs-before')
    sources, logs = {}, {}
    for role, (name, job_name, _) in ROLES.items():
        matches = [item for item in artifacts if item['name'] == name and not item['expired']]
        require(len(matches) == 1, '角色必须对应唯一有效工件：' + role)
        artifact = api(f'actions/artifacts/{matches[0]["id"]}', role + '-artifact-before.json')
        artifact_identity(artifact, run, name)
        producers = [job for job in jobs if job['name'] == job_name and job['status'] == 'completed' and job['conclusion'] == 'success' and job['started_at'] <= artifact['created_at'] <= job['completed_at']]
        require(len(producers) == 1, '工件必须对应唯一成功生产 job：' + role)
        job = api(f'actions/jobs/{producers[0]["id"]}', role + '-job-before.json')
        job_identity(job, run, role, artifact)
        if job['id'] not in logs:
            log_path = ROOT / f'producer-{job["id"]}.log'
            download(API + f'actions/jobs/{job["id"]}/logs', log_path, 64 * 1024**2, True)
            logs[job['id']] = log_path.read_text(encoding='utf-8-sig')
        log = logs[job['id']]
        finalized = r'(?m)^.*Artifact ' + re.escape(name) + r'\.zip successfully finalized\. Artifact ID\s+' + str(artifact['id']) + r'\s*$'
        digest_line = r'(?m)^.*SHA256 digest of uploaded artifact zip is ' + re.escape(artifact['digest'][7:]) + r'\s*$'
        require(len(re.findall(finalized, log)) == 1 and len(re.findall(digest_line, log)) == 1, '上传原始日志没有唯一绑定名称、ID 和 ZIP 摘要：' + role)
        sources[role] = {'artifact': artifact_identity(artifact, run, name), 'producer': job_identity(job, run, role, artifact), 'upload_log': f'producer-{job["id"]}.log', 'upload_log_sha256': sha(ROOT / f'producer-{job["id"]}.log')}
    bundle_log = logs[sources['bundle']['producer']['id']]
    downloaded_ids = [int(x) for x in re.findall(r'\(ID:\s*(\d+),', bundle_log)]
    cli_ids = sorted(value['artifact']['id'] for role, value in sources.items() if role not in ('bundle', 'windows'))
    require(len(downloaded_ids) == 6 and sorted(downloaded_ids) == cli_ids, 'bundle 下载日志没有绑定精确六件来源 ID')
    for role in ROLES:
        if role not in ('bundle', 'windows'):
            require(ROLES[role][0] in bundle_log, 'bundle 下载日志缺少 CLI 角色名称')
    desktop_downloads = [int(x) for x in re.findall(r'\(ID:\s*(\d+),', logs[sources['windows']['producer']['id']])]
    require(desktop_downloads == [sources['bundle']['artifact']['id']], 'Windows producer 未使用已绑定 bundle')
    for role, expected, limit in [('windows', ['InfiniShellSetup.exe'], 2 * 1024**3), ('bundle', ['remote-server/manifest.json'] + ['remote-server/' + name for name in NAMES], 2 * 1024**3)]:
        artifact = sources[role]['artifact']
        path = ROOT / (role + '.artifact.zip')
        size = download(API + f'actions/artifacts/{artifact["id"]}/zip', path, artifact['size_in_bytes'], True)
        require(size == artifact['size_in_bytes'] and 'sha256:' + sha(path) == artifact['digest'], '提供方 ZIP 摘要或大小不符：' + role)
        sources[role]['extracted'] = unzip(path, ROOT / role, expected, limit)
    manifest_path = ROOT / 'bundle/remote-server/manifest.json'
    require(manifest_path.stat().st_size <= 65536, 'CI manifest 超过上限')
    manifest = json.loads(manifest_path.read_bytes())
    require(manifest['version'] == run['head_branch'] and len(manifest['artifacts']) == 6, 'CI manifest 未绑定新 tag 和精确六资源')
    require({row['file'] for row in manifest['artifacts']} == set(NAMES), 'CI manifest 成员集合不符')
    for row in manifest['artifacts']:
        require(re.fullmatch(r'[0-9a-f]{64}', row['sha256']) and sha(ROOT / 'bundle/remote-server' / row['file']) == row['sha256'], 'CI manifest 内部摘要不符')
    (ROOT / 'ci-original-manifest.json').write_bytes(manifest_path.read_bytes())
    tool_zip = ROOT / 'innounp.zip'
    require(download(TOOL['url'], tool_zip, TOOL['archive_size'], False) == TOOL['archive_size'] and sha(tool_zip) == TOOL['archive_sha256'], '固定 innounp ZIP 来源不符')
    unzip(tool_zip, ROOT / 'tool', ['innounp-cl.txt', 'innounp.css', 'innounp.exe', 'innounp.htm', 'innounp.ico'], 4 * 1024**2)
    require(sha(ROOT / 'tool/innounp.exe') == TOOL['exe_sha256'] and (ROOT / 'tool/innounp.exe').stat().st_size == TOOL['exe_size'], '固定 innounp EXE 不符')
    identity = {'schema': 'infinishell-release-installer-source-v1', 'controller_sha': CONTROLLER, 'artifact_source_sha': run['head_sha'], 'source_sha': run['head_sha'], 'tag': run['head_branch'], 'tag_object': tags['object']['sha'], 'release_run': RUN, 'release_run_attempt': run['run_attempt'], 'run': bound_run, 'sources': sources, 'tool': TOOL, 'ci_manifest_sha256': sha(manifest_path), 'controller_script_sha256': sha(Path(__file__)), 'product_execution': 'not_performed'}
    save('candidate-identity.json', identity)
    save('source-acquisition-result.json', {'status': 'provider_digest_crc_exact_members_and_source_verified', 'identity_sha256': sha(ROOT / 'candidate-identity.json'), 'artifact_source_sha': run['head_sha'], 'controller_sha': CONTROLLER})


def recheck():
    before = json.loads((ROOT / 'candidate-identity.json').read_text())
    require(before['controller_sha'] == CONTROLLER and before['release_run'] == RUN, '后核 controller/run 不符')
    run = api(f'actions/runs/{RUN}', 'run-after.json')
    require(run_identity(run) == before['run'], '前后正式 run 身份变化')
    tags = tag_identity(run, 'after')
    require(tags['object']['sha'] == before['tag_object'], '前后 annotated tag 变化')
    for role, source in before['sources'].items():
        artifact = api(f'actions/artifacts/{source["artifact"]["id"]}', role + '-artifact-after.json')
        require(artifact_identity(artifact, run, ROLES[role][0]) == source['artifact'], '前后工件提供方身份变化：' + role)
        job = api(f'actions/jobs/{source["producer"]["id"]}', role + '-job-after.json')
        require(job_identity(job, run, role, artifact) == source['producer'], '前后生产 job 身份变化：' + role)
    for role in ('windows', 'bundle'):
        source = before['sources'][role]
        require('sha256:' + sha(ROOT / (role + '.artifact.zip')) == source['artifact']['digest'], '验证期间 ZIP 原件变化')
        for row in source['extracted']:
            path = ROOT / role / row['name']
            require(path.stat().st_size == row['size'] and sha(path) == row['sha256'], '验证期间 CI 提取原件变化')
    require(sha(ROOT / 'tool/innounp.exe') == TOOL['exe_sha256'], '验证期间工具变化')
    save('official-recheck-result.json', {'status': 'official_source_and_local_inputs_unchanged', 'artifact_source_sha': run['head_sha'], 'controller_sha': CONTROLLER, 'identity_sha256': sha(ROOT / 'candidate-identity.json')})


try:
    require(os.name == 'nt' and sys.version_info[:3] == (3, 13, 15) and struct.calcsize('P') == 8, '必须使用已固定准备的 Windows Python 3.13.15 x64')
    require(os.environ.get('GITHUB_REPOSITORY') == REPO and re.fullmatch('[0-9a-f]{40}', CONTROLLER), '控制器环境不符')
    require(sys.argv[1] in ('acquire', 'recheck'), '未知只读模式')
    acquire() if sys.argv[1] == 'acquire' else recheck()
except BaseException as exc:
    # 不归档网络异常字符串，避免签名下载 URL 或认证信息进入日志。
    save(sys.argv[1] + '-error.json', {'status': 'failed', 'exception_type': type(exc).__name__, 'http_status': getattr(exc, 'code', None), 'reason': str(exc) if isinstance(exc, RuntimeError) else '详见已保存阶段原件；未输出 URL、请求头或凭据。'})
    print('正式工件只读门禁失败；原件保留，未执行产品。', file=sys.stderr)
    raise SystemExit(1)
'@

[IO.File]::WriteAllText($acquirerPath, $acquirer, [Text.UTF8Encoding]::new($false))
$commands = [Collections.Generic.List[object]]::new()
function Invoke-ReadOnlyProcess([string]$Label, [string]$Executable, [string[]]$Arguments, [int]$TimeoutSeconds) {
    foreach ($value in @($Executable) + $Arguments) { if ($value -match '["\r\n]') { throw '工具参数含非法字符。' } }
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.Arguments = (($Arguments | ForEach-Object { '"' + $_ + '"' }) -join ' ')
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WorkingDirectory = $root
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    if ($Label -in @('acquire', 'recheck')) { $info.WorkingDirectory = $env:GITHUB_WORKSPACE }
    $out = [IO.File]::Open((Join-Path $root ($Label + '.stdout.log')), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $err = [IO.File]::Open((Join-Path $root ($Label + '.stderr.log')), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $process = [Diagnostics.Process]::new()
    try {
        $process.StartInfo = $info
        if (-not $process.Start()) { throw '只读工具未启动。' }
        $stdoutTask = $process.StandardOutput.BaseStream.CopyToAsync($out)
        $stderrTask = $process.StandardError.BaseStream.CopyToAsync($err)
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $commands.Add(@{ label = $Label; pid = $process.Id; status = 'timed_out_retained'; executable = $Executable })
            throw '只读工具超时；保留原始进程与目录，不重跑工具或产品。'
        }
        $null = $stdoutTask.GetAwaiter().GetResult()
        $null = $stderrTask.GetAwaiter().GetResult()
        $out.Flush($true); $err.Flush($true)
        $commands.Add(@{ label = $Label; executable = $Executable; arguments = $Arguments; pid = $process.Id; exit_code = $process.ExitCode; process_exited = $process.HasExited })
        if ($process.ExitCode -ne 0) { throw ('只读工具失败：' + $Label + '，退出码 ' + $process.ExitCode + '；原始输出已保留。') }
    } finally { $out.Dispose(); $err.Dispose(); $process.Dispose() }
}
function Stream-Sha($Stream) {
    $Stream.Position = 0
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($hasher.ComputeHash($Stream))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose(); $Stream.Position = 0 }
}
function Certificate-Summary($Certificate) {
    if ($null -eq $Certificate) { return $null }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return @{ thumbprint_sha1 = $Certificate.Thumbprint.ToLowerInvariant(); certificate_sha256 = ([BitConverter]::ToString($hasher.ComputeHash($Certificate.RawData))).Replace('-', '').ToLowerInvariant() } }
    finally { $hasher.Dispose() }
}
$result = [ordered]@{
    schema = 'infinishell-release-installer-verification-v1'
    status = 'failed'
    release_run = $ReleaseRun
    controller_sha = $env:GITHUB_SHA
    artifact_source_sha = $null
    controller_script_sha256 = File-Sha $PSCommandPath
    started_utc = [DateTime]::UtcNow.ToString('o')
    finished_utc = $null
    signature = $null
    embedded = $null
    error = $null
    commands = $commands
    product_execution = 'not_performed'
    installer_execution = 'not_performed'
    gui = 'not_performed'
    user_lifecycle = 'pending_user_acceptance'
    system_installation = 'not_performed'
}
$locks = [Collections.Generic.List[object]]::new()
try {
    Invoke-ReadOnlyProcess 'acquire' $python @('-I', $acquirerPath, 'acquire', $root, $ReleaseRun) 1800
    $identityPath = Join-Path $root 'candidate-identity.json'
    $identity = Get-Content -LiteralPath $identityPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($identity.controller_sha -cne $env:GITHUB_SHA -or [string]$identity.release_run -cne $ReleaseRun) { throw '下载身份未绑定本控制器与请求。' }
    $result.artifact_source_sha = $identity.artifact_source_sha
    $result['tag'] = $identity.tag
    $result['tag_object'] = $identity.tag_object
    $result['release_run_attempt'] = $identity.release_run_attempt
    $result['identity_sha256'] = File-Sha $identityPath
    $result['ci_manifest_sha256'] = $identity.ci_manifest_sha256
    $installer = Join-Path $root 'windows\InfiniShellSetup.exe'
    $tool = Join-Path $root 'tool\innounp.exe'
    $expectedSetup = @($identity.sources.windows.extracted | Where-Object { $_.name -ceq 'InfiniShellSetup.exe' })
    if ($expectedSetup.Count -ne 1) { throw '缺少唯一 Setup 原件。' }
    $lockPaths = @($installer, $tool, $identityPath, $acquirerPath, (Join-Path $root 'windows.artifact.zip'), (Join-Path $root 'bundle.artifact.zip'))
    foreach ($row in $identity.sources.bundle.extracted) { $lockPaths += Join-Path $root ('bundle\' + $row.name.Replace('/', '\')) }
    foreach ($path in $lockPaths) {
        Assert-Physical $path
        $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $locks.Add(@{ path = $path; stream = $stream; size = $stream.Length; sha256 = Stream-Sha $stream })
    }
    if ((File-Sha $installer) -cne $expectedSetup[0].sha256 -or (Get-Item -LiteralPath $installer).Length -ne $expectedSetup[0].size -or (File-Sha $tool) -cne $identity.tool.exe_sha256) { throw '安装器或只读工具在使用前变化。' }
    $signature = Get-AuthenticodeSignature -LiteralPath $installer -ErrorAction Stop
    if ($signature.Status.ToString() -cnotin @('Valid', 'NotSigned')) { throw ('安装器 Authenticode 状态不可接受：' + $signature.Status) }
    if ($signature.Status.ToString() -ceq 'Valid' -and $null -eq $signature.SignerCertificate) { throw 'Valid 状态缺少签名者证书。' }
    if ($signature.Status.ToString() -ceq 'NotSigned' -and $null -ne $signature.SignerCertificate) { throw 'NotSigned 状态与签名者不一致。' }
    $signatureReceipt = [ordered]@{
        status = 'authenticode_read_verified'
        authenticode_status = $signature.Status.ToString()
        installer_sha256 = $expectedSetup[0].sha256
        installer_size_bytes = $expectedSetup[0].size
        signer = Certificate-Summary $signature.SignerCertificate
        timestamp_signer = Certificate-Summary $signature.TimeStamperCertificate
        source_sha = $identity.artifact_source_sha
        controller_sha = $identity.controller_sha
        tag = $identity.tag
        tag_object = $identity.tag_object
        release_run = $identity.release_run
        artifact_id = $identity.sources.windows.artifact.id
        producer_job_id = $identity.sources.windows.producer.id
        producer_run_attempt = $identity.sources.windows.producer.run_attempt
        identity_sha256 = $result.identity_sha256
        captured_utc = [DateTime]::UtcNow.ToString('o')
    }
    Write-NewJson (Join-Path $root 'signature-result.json') $signatureReceipt
    $result.signature = $signatureReceipt
    Invoke-ReadOnlyProcess 'installer-info' $tool @('-b', '-o', '-u', $installer) 600
    Invoke-ReadOnlyProcess 'installer-members' $tool @('-s', '-h', '-b', '-o', '-u', $installer) 600
    $info = [IO.File]::ReadAllText((Join-Path $root 'installer-info.stdout.log'))
    if ($info -cnotmatch ('(?m)^Application version:\s+' + [regex]::Escape($identity.tag) + '\s*$')) { throw '安装器内部版本不是本轮 tag。' }
    $names = @('manifest.json', 'infinishell-linux-x86_64.tar.gz', 'infinishell-linux-aarch64.tar.gz', 'infinishell-macos-x86_64.tar.gz', 'infinishell-macos-aarch64.tar.gz', 'infinishell-windows-x86_64.zip', 'infinishell-windows-aarch64.zip')
    $members = @('{app}\infinishell.exe') + @($names | ForEach-Object { '{app}\resources\remote-server\' + $_ })
    $listed = @([IO.File]::ReadAllLines((Join-Path $root 'installer-members.stdout.log')) | ForEach-Object { $_.Trim() })
    foreach ($member in $members) {
        if (@($listed | Where-Object { $_ -ceq $member }).Count -ne 1) { throw '安装器列表没有精确包含唯一目标成员。' }
    }
    $embedded = Join-Path $root 'embedded'
    $null = New-Item -ItemType Directory -Path $embedded -ErrorAction Stop
    Invoke-ReadOnlyProcess 'embedded-extract' $tool (@('-e', '-b', '-q', '-o', '-u', ('-d' + $embedded), $installer) + $members) 600
    $expectedNames = @('infinishell.exe') + $names
    $actualNames = @(Get-ChildItem -LiteralPath $embedded -Force | ForEach-Object { $_.Name })
    if ((@($actualNames | Sort-Object) -join '|') -cne (@($expectedNames | Sort-Object) -join '|') -or $actualNames.Count -ne 8) { throw '实际只读解包输出不是精确八件。' }
    $rows = @()
    foreach ($name in $expectedNames) {
        $path = Join-Path $embedded $name
        Assert-Physical $path
        $item = Get-Item -LiteralPath $path -Force
        if ($item.PSIsContainer) { throw '内嵌目标不是普通文件。' }
        $hash = File-Sha $path
        if ($name -ne 'infinishell.exe') {
            $source = Join-Path $root ('bundle\remote-server\' + $name)
            if ($hash -cne (File-Sha $source) -or $item.Length -ne (Get-Item -LiteralPath $source).Length) { throw ('内嵌资源不等于 CI 原件：' + $name) }
        }
        $rows += @{ name = $name; size = $item.Length; sha256 = $hash }
    }
    $main = Join-Path $embedded 'infinishell.exe'
    $stream = [IO.File]::OpenRead($main)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw '主 EXE 无 DOS 标识。' }
        $stream.Position = 0x3c
        $pe = $reader.ReadInt32()
        if ($pe -lt 64 -or $pe -gt $stream.Length - 26) { throw '主 EXE PE 偏移越界。' }
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) { throw '内嵌主 EXE 必须为 AMD64 PE；不约束 Setup 引擎为 PE32+。' }
    } finally { $reader.Dispose(); $stream.Dispose() }
    $embeddedReceipt = [ordered]@{
        status = 'exact_eight_members_and_ci_original_six_verified'
        source_sha = $identity.artifact_source_sha
        controller_sha = $identity.controller_sha
        tag = $identity.tag
        tag_object = $identity.tag_object
        release_run = $identity.release_run
        identity_sha256 = $result.identity_sha256
        installer_sha256 = $expectedSetup[0].sha256
        ci_manifest_sha256 = $identity.ci_manifest_sha256
        bundle_artifact_id = $identity.sources.bundle.artifact.id
        bundle_producer_job_id = $identity.sources.bundle.producer.id
        bundle_producer_run_attempt = $identity.sources.bundle.producer.run_attempt
        members = $members
        extracted = $rows
        main_pe_machine = 'AMD64'
        tool = $identity.tool
        product_execution = 'not_performed'
    }
    Write-NewJson (Join-Path $root 'installer-embedded-resources-result.json') $embeddedReceipt
    $result.embedded = $embeddedReceipt
    Invoke-ReadOnlyProcess 'recheck' $python @('-I', $acquirerPath, 'recheck', $root, $ReleaseRun) 900
    foreach ($entry in $locks) {
        if ($entry.stream.Length -ne $entry.size -or (Stream-Sha $entry.stream) -cne $entry.sha256) { throw '验证期间持锁输入原件发生变化。' }
    }
    $result.status = 'readonly_final_installer_artifact_verification_passed'
} catch { $result.error = $_.Exception.Message }
finally {
    foreach ($entry in $locks) { $entry.stream.Dispose() }
    $result.finished_utc = [DateTime]::UtcNow.ToString('o')
    Write-NewJson (Join-Path $root 'result.json') $result
    $evidence = @()
    foreach ($item in @(Get-ChildItem -LiteralPath $root -File | Where-Object { $_.Extension -in @('.json', '.log', '.py') })) {
        $evidence += @{ file = $item.Name; size = $item.Length; sha256 = File-Sha $item.FullName }
    }
    Write-NewJson (Join-Path $root 'evidence-index.json') @{ schema = 'infinishell-readonly-installer-evidence-v1'; files = $evidence; boundary = '只上传小证据；CI 原始 ZIP 和产品文件保留在独占作业目录，不运行产品。' }
}
Write-Output ($result | ConvertTo-Json -Depth 30 -Compress)
if ($result.status -cne 'readonly_final_installer_artifact_verification_passed') { exit 1 }
exit 0
