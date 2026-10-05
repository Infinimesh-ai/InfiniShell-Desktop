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

function Get-G09SourceIdentity {
    $repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
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
    return [ordered]@{
        commit = $commit[0]; dirty = $status.Count -ne 0
        changed_entries = $status.Count; status_sha256 = $statusHash
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
    if (-not $Local -and $sourceIdentity.commit -cne $env:GITHUB_SHA) {
        throw 'runner 声明的提交与真实源码 HEAD 不同'
    }
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
