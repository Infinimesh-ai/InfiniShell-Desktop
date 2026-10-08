# 两个准备脚本共用边界；本地模式必须显式选择，不修改或伪造 CI 环境。
function Assert-G09LocalDirectory([string]$Path) {
    if ($Path -notmatch '^[A-Za-z]:\\' -or $Path.Substring(2) -match '[:*?"<>|]' -or
        $Path -match '(^|\\)(\.{1,2}|[^\\]*[ .])($|\\)') {
        throw '准备根必须是无路径逃逸的绝对本地目录'
    }
    $full = [IO.Path]::GetFullPath($Path)
    if ($full.Length -le 3 -or
        -not [string]::Equals($Path.TrimEnd('\'), $full.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
        throw '准备根不得包含非规范路径段'
    }
    $current = Get-Item -LiteralPath $full -Force
    while ($null -ne $current) {
        if ($current -isnot [IO.DirectoryInfo] -or ($current.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw '准备根及所有祖先必须是普通本地目录'
        }
        $current = $current.Parent
    }
    return $full.TrimEnd('\')
}

function Get-G09SourcePaths {
    @('script/ci/g09-clr-fixture/preparation-context.ps1', 'script/ci/g09-clr-fixture/build-reader.ps1',
      'script/ci/g09-clr-fixture/prepare.ps1', 'script/ci/g09-clr-fixture/Fixture.cs',
      'script/ci/g09-clr-reader/reader.cpp', 'script/ci/g09-clr-reader/wire.h',
      'script/ci/g09-clr-reader/sos_layout.h', 'script/ci/g09-clr-reader/README.md',
      'script/ci/g09-clr-reader/sources.safe.json', 'script/ci/g09-clr-reader/vendor/clrdata.h',
      'script/ci/g09-clr-reader/vendor/xclrdata.h', 'script/ci/g09-clr-reader/vendor/sospriv.h',
      'script/ci/g09-clr-reader/vendor/LICENSE.TXT')
}

function Get-G09SourceIdentity([string]$Repository = (Join-Path $PSScriptRoot '..\..\..')) {
    $repository = [IO.Path]::GetFullPath($Repository)
    $actual = @(& git -C $repository rev-parse --show-toplevel)
    if ($LASTEXITCODE -ne 0 -or $actual.Count -ne 1 -or
        -not [string]::Equals([IO.Path]::GetFullPath($actual[0]), $repository, [StringComparison]::OrdinalIgnoreCase)) {
        throw '准备源码不属于预期 Git 工作区'
    }
    $commit = @(& git -C $repository rev-parse --verify HEAD)
    if ($LASTEXITCODE -ne 0 -or $commit.Count -ne 1 -or $commit[0] -cnotmatch '^[a-f0-9]{40}$') {
        throw '无法读取真实源码提交'
    }
    $status = @(& git -C $repository status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw '无法读取真实源码工作区状态' }
    # 安全收据只记录状态摘要及数量，不保存本地文件路径或环境内容。
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($status -join "`n")))
        $statusHash = ([BitConverter]::ToString($digest)).Replace('-', '').ToLowerInvariant()
    } finally { $sha.Dispose() }
    # Git 的 clean 过滤会掩盖 CRLF 等实际字节差异，必须绕过过滤独立核验固定输入。
    $files = @(foreach ($relative in (Get-G09SourcePaths)) {
        $path = Join-Path $repository $relative
        $null = Assert-G09LocalDirectory ([IO.Path]::GetDirectoryName($path))
        $item = Get-Item -LiteralPath $path -Force
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $item.Length -le 0 -or $item.Length -gt 67108864) {
            throw '固定源码不是边界内的普通文件'
        }
        $headBlob = @(& git -C $repository rev-parse --verify ($commit[0] + ':' + $relative))
        if ($LASTEXITCODE -ne 0 -or $headBlob.Count -ne 1 -or $headBlob[0] -cnotmatch '^[a-f0-9]{40}$') {
            throw '固定源码缺少提交原件'
        }
        $actualBlob = @(& git -C $repository hash-object --no-filters -- $path)
        if ($LASTEXITCODE -ne 0 -or $actualBlob.Count -ne 1 -or $actualBlob[0] -cnotmatch '^[a-f0-9]{40}$') {
            throw '无法读取固定源码的原始字节身份'
        }
        [ordered]@{
            path = $relative; head_blob = $headBlob[0]; actual_blob = $actualBlob[0]
            exact_to_head = $actualBlob[0] -ceq $headBlob[0]
        }
    })
    $commitAfter = @(& git -C $repository rev-parse --verify HEAD)
    if ($LASTEXITCODE -ne 0 -or $commitAfter.Count -ne 1 -or $commitAfter[0] -cne $commit[0]) {
        throw '源码提交在身份核验期间发生变化'
    }
    $mismatches = @($files | Where-Object { -not $_.exact_to_head }).Count
    return [ordered]@{
        commit = $commit[0]; dirty = $status.Count -ne 0 -or $mismatches -ne 0
        git_status_dirty = $status.Count -ne 0; byte_mismatch_count = $mismatches
        changed_entries = $status.Count; status_sha256 = $statusHash
        files_exact_to_head = $mismatches -eq 0; files = $files
    }
}

function Assert-G09SourceIdentity([bool]$Local, $Identity, [string]$ExpectedCommit) {
    if (-not $Local -and ($Identity.commit -cne $ExpectedCommit -or -not $Identity.files_exact_to_head)) {
        throw 'runner 源码 HEAD 或固定输入原始字节不匹配，禁止使用过滤后的 clean 状态代替同源证明'
    }
}

function Assert-G09SourceUnchanged($Before, $After) {
    if ($Before.commit -cne $After.commit -or @($Before.files).Count -ne @($After.files).Count) {
        throw '源码提交或固定输入集合在准备期间发生变化'
    }
    for ($i = 0; $i -lt $Before.files.Count; $i++) {
        if ($Before.files[$i].path -cne $After.files[$i].path -or
            $Before.files[$i].head_blob -cne $After.files[$i].head_blob -or
            $Before.files[$i].actual_blob -cne $After.files[$i].actual_blob) {
            throw '固定输入原始字节在准备期间发生变化'
        }
    }
}

function New-G09PreparationContext([bool]$Local, [string]$LocalRoot, [string]$LocalRunId, [string]$Kind) {
    if ([IntPtr]::Size -ne 8 -or $env:OS -ne 'Windows_NT' -or
        $PSVersionTable.PSEdition -ne 'Desktop' -or $PSVersionTable.PSVersion.Major -ne 5 -or
        $PSVersionTable.PSVersion.Minor -ne 1) {
        throw '准备入口仅支持 Windows x64 的 Windows PowerShell 5.1'
    }
    if ($Kind -cnotin @('build', 'fixture')) { throw '未知准备目录类型' }
    if ($Local) {
        if ($env:GITHUB_ACTIONS -eq 'true' -or [string]::IsNullOrWhiteSpace($LocalRoot) -or
            $LocalRunId -cnotmatch '^[a-z0-9][a-z0-9-]{0,47}$') {
            throw '本地准备必须显式指定既有 LocalRoot 和独立 LocalRunId，且不能在 Actions 内使用'
        }
        $base = Assert-G09LocalDirectory $LocalRoot
        $suffix = 'local-' + $LocalRunId
        $mode = 'local'
        $runId = $LocalRunId
        $attempt = $null
    } else {
        if (-not [string]::IsNullOrEmpty($LocalRoot) -or -not [string]::IsNullOrEmpty($LocalRunId) -or
            $env:GITHUB_ACTIONS -ne 'true' -or $env:GITHUB_RUN_ID -notmatch '^\d+$' -or
            $env:GITHUB_RUN_ATTEMPT -notmatch '^\d+$' -or [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or
            [string]::IsNullOrEmpty($env:GITHUB_ENV)) {
            throw '默认准备入口仅支持 Windows x64 CI；本地调试请显式使用 Local 参数'
        }
        $base = Assert-G09LocalDirectory $env:RUNNER_TEMP
        $suffix = $env:GITHUB_RUN_ID + '-' + $env:GITHUB_RUN_ATTEMPT
        $mode = 'runner'
        $runId = $env:GITHUB_RUN_ID
        $attempt = $env:GITHUB_RUN_ATTEMPT
    }
    $sourceIdentity = Get-G09SourceIdentity
    $root = [IO.Path]::GetFullPath((Join-Path $base ('g09-clr-' + $Kind + '-' + $suffix)))
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($root), $base.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase) -or
        (Test-Path -LiteralPath $root)) {
        throw '本轮准备目录已存在或逃逸指定根'
    }
    $directory = New-Item -ItemType Directory -Path $root
    # 只给刚新建的本轮目录设置私有 ACL；不修复或收紧既有目录的权限。
    $null = Assert-G09LocalDirectory $directory.FullName
    $owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetOwner($owner)
    $acl.SetAccessRuleProtection($true, $false)
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $owner, [Security.AccessControl.FileSystemRights]::FullControl,
        [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
        [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow))
    Set-Acl -LiteralPath $directory.FullName -AclObject $acl
    $null = Assert-G09LocalDirectory $directory.FullName
    $actualAcl = Get-Acl -LiteralPath $root
    $rules = @($actualAcl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
    if (-not $actualAcl.AreAccessRulesProtected -or $actualAcl.GetOwner([Security.Principal.SecurityIdentifier]) -ne $owner -or
        $rules.Count -ne 1 -or $rules[0].IdentityReference -ne $owner -or
        $rules[0].AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow -or
        $rules[0].FileSystemRights -ne [Security.AccessControl.FileSystemRights]::FullControl) {
        throw '本轮准备目录的私有 ACL 核验失败'
    }
    return [pscustomobject]@{
        Root = $root; Mode = $mode; RunId = $runId; RunAttempt = $attempt
        SourceIdentity = $sourceIdentity
    }
}

function Write-G09PreparationOutput($Context, [string]$Name, [string]$Value) {
    if ($Context.Mode -ceq 'runner') {
        "$Name=$Value" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    } else {
        # 本地输出供调用方读取，不修改调用进程环境或 GITHUB_ENV。
        Write-Output "$Name=$Value"
    }
}
