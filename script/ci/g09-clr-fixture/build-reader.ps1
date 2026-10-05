$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([IntPtr]::Size -ne 8 -or $env:OS -ne 'Windows_NT' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or $PSVersionTable.PSVersion.Major -ne 5 -or
    $PSVersionTable.PSVersion.Minor -ne 1 -or $env:GITHUB_ACTIONS -ne 'true' -or
    $env:GITHUB_RUN_ID -notmatch '^\d+$' -or $env:GITHUB_RUN_ATTEMPT -notmatch '^\d+$' -or
    [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or [string]::IsNullOrEmpty($env:GITHUB_ENV)) {
    throw 'reader 构建仅支持 Windows x64 CI 的 Windows PowerShell 5.1'
}

function Get-RegularFileHash([string]$Path) {
    $item = Get-Item -LiteralPath $Path
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $item.Length -le 0 -or $item.Length -gt 67108864) {
        throw '构建原件不是固定边界内的普通文件'
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$base = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
if ($base.StartsWith('\\') -or -not [IO.Directory]::Exists($base)) {
    throw '构建根必须是既有本地 runner 临时目录'
}
$root = Join-Path $base ('g09-clr-build-' + $env:GITHUB_RUN_ID + '-' + $env:GITHUB_RUN_ATTEMPT)
if (Test-Path -LiteralPath $root) { throw '本轮 reader 构建目录已存在' }
$directory = New-Item -ItemType Directory -Path $root
"INFINISHELL_CLR_BUILD_ROOT=$root" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\g09-clr-reader'))
$snapshot = Join-Path $root 'source'
$executable = Join-Path $root 'reader.exe'
$inputs = @()
$failure = $null
$postFailure = $false
$receipt = [ordered]@{
    schema = 1; status = 'started'; stage = 'private_directory'; source_commit = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID; run_attempt = $env:GITHUB_RUN_ATTEMPT
    native_execution_started = $false; dac_loaded = $false; cleanup_ready = $false
    inputs = @(); executable = $null
    scope = '仅编译固定只读 reader；不执行 reader、DAC、CLR 夹具或任何 CLI'
}
try {
    # 仅给本轮新建目录设置私有 ACL，不修改安装树或既有 runner 目录。
    $owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetOwner($owner)
    $acl.SetAccessRuleProtection($true, $false)
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $owner, [Security.AccessControl.FileSystemRights]::FullControl,
        [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
        [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow))
    Set-Acl -LiteralPath $directory.FullName -AclObject $acl
    New-Item -ItemType Directory -Path $snapshot | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $snapshot 'vendor') | Out-Null

    $receipt.stage = 'official_source_binding'
    # 独立固定官方原件，不允许同时改 provenance 和 vendor 摘要来绕过误更检查。
    $official = @(
        @{ path = 'vendor/clrdata.h'; bytes = 45641; sha256 = '66ef8f73507485e20c299a4e2b4f553ed3e5a523a4949330b026ec90e3878f69' },
        @{ path = 'vendor/xclrdata.h'; bytes = 289218; sha256 = 'a3729f85c323adb1996c1cfbd6a32bb7c0d6429a50fe45c394ef0ece7d599984' },
        @{ path = 'vendor/LICENSE.TXT'; bytes = 1116; sha256 = 'cfc21f5e8bd655ae997eec916138b707b1d290b83272c02a95c9f821b8c87310' }
    )
    $provenancePath = Join-Path $sourceRoot 'sources.safe.json'
    $provenanceHash = Get-RegularFileHash $provenancePath
    $provenance = [IO.File]::ReadAllText($provenancePath, [Text.Encoding]::UTF8) | ConvertFrom-Json
    if ($provenance.schema -ne 1 -or $provenance.upstream -ne 'https://github.com/dotnet/runtime' -or
        $provenance.commit -ne '5535e31a712343a63f5d7d796cd874e563e5ac14' -or
        $provenance.tag -ne 'v8.0.0' -or @($provenance.files).Count -ne 3) {
        throw '官方 ABI 来源记录不匹配'
    }
    foreach ($item in $official) {
        $path = Join-Path $sourceRoot $item.path
        $declared = @($provenance.files | Where-Object { $_.path -ceq $item.path })
        if ($declared.Count -ne 1 -or $declared[0].sha256 -cne $item.sha256 -or
            $declared[0].bytes -ne $item.bytes -or (Get-Item -LiteralPath $path).Length -ne $item.bytes -or
            (Get-RegularFileHash $path) -cne $item.sha256) {
            throw '官方 vendor 原件字节或来源摘要不匹配'
        }
    }
    foreach ($name in @('reader.cpp', 'wire.h', 'README.md', 'sources.safe.json',
                         'vendor/clrdata.h', 'vendor/xclrdata.h', 'vendor/LICENSE.TXT')) {
        $path = Join-Path $sourceRoot $name
        $copied = Join-Path $snapshot $name
        $hash = Get-RegularFileHash $path
        $expected = @($official | Where-Object { $_.path -ceq $name })
        if (($expected.Count -eq 1 -and $hash -cne $expected[0].sha256) -or
            ($name -ceq 'sources.safe.json' -and $hash -cne $provenanceHash)) {
            throw '已核官方原件在保全过程中发生变化'
        }
        Copy-Item -LiteralPath $path -Destination $copied
        if ((Get-RegularFileHash $copied) -cne $hash) { throw '私有源码副本不匹配' }
        $inputs += [ordered]@{ path = $name; bytes = (Get-Item -LiteralPath $path).Length; sha256_before = $hash }
    }
    $scriptHash = Get-RegularFileHash $PSCommandPath
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $root 'build-reader.ps1')
    $receipt.build_script_sha256 = $scriptHash
    $receipt.provenance_sha256 = $provenanceHash

    $receipt.stage = 'toolchain'
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $receipt.vswhere_sha256 = Get-RegularFileHash $vswhere
    $installations = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if ($LASTEXITCODE -ne 0 -or $installations.Count -ne 1 -or [string]::IsNullOrWhiteSpace($installations[0])) {
        throw '缺少唯一既有 MSVC x64 工具链'
    }
    $vcvars = Join-Path $installations[0].Trim() 'VC\Auxiliary\Build\vcvars64.bat'
    $receipt.vcvars_sha256 = Get-RegularFileHash $vcvars
    $source = Join-Path $snapshot 'reader.cpp'
    $object = Join-Path $root 'reader.obj'
    $build = Join-Path $root 'build.cmd'
    foreach ($path in @($vcvars, $root, $source)) {
        if ($path -match '[%!"&|<>^]' -or $path -match '[^\x20-\x7e]') {
            throw 'CMD 构建路径必须为不含元字符的 ASCII 路径'
        }
    }
    @(
        '@echo off'
        "call `"$vcvars`" >nul || exit /b 1"
        'set "CL="'
        'set "_CL_="'
        'set "LINK="'
        'set "_LINK_="'
        "cl.exe /nologo /std:c++17 /utf-8 /W4 /WX /O2 /MT /EHsc `"$source`" /Fo`"$object`" /Fe`"$executable`" /link bcrypt.lib || exit /b 1"
        'exit /b 0'
    ) | Set-Content -LiteralPath $build -Encoding ascii
    $receipt.stage = 'compile'
    & cmd.exe /d /v:off /c "`"$build`"" *> (Join-Path $root 'compiler.log')
    $receipt.compiler_exit = $LASTEXITCODE
    if ($LASTEXITCODE -ne 0) { throw 'reader 构建失败，完整 compiler.log 已保留' }
    $receipt.executable = [ordered]@{
        file = 'reader.exe'; sha256 = (Get-RegularFileHash $executable)
        bytes = (Get-Item -LiteralPath $executable).Length
    }
    $receipt.status = 'compiled'
    $receipt.stage = 'source_recheck'
} catch {
    $failure = $_
    $receipt.status = 'failed'
    $receipt.error_kind = $_.Exception.GetType().FullName
    $receipt.error_hresult = $_.Exception.HResult
} finally {
    # 编译失败也重核已保全输入；不让后续清理或再次构建覆盖首次失败。
    foreach ($item in $inputs) {
        try {
            $item.sha256_after = Get-RegularFileHash (Join-Path $sourceRoot $item.path)
            $item.archived_sha256 = Get-RegularFileHash (Join-Path $snapshot $item.path)
            $item.unchanged = $item.sha256_after -ceq $item.sha256_before -and $item.archived_sha256 -ceq $item.sha256_before
            if (-not $item.unchanged) { $postFailure = $true }
        } catch {
            $postFailure = $true
            $item.unchanged = $false
            $item.recheck_error_kind = $_.Exception.GetType().FullName
            $item.recheck_error_hresult = $_.Exception.HResult
        }
    }
    $receipt.inputs = $inputs
    $receipt.inputs_rechecked = $inputs.Count
    $receipt.sources_unchanged = $inputs.Count -eq 7 -and -not $postFailure
    if ($postFailure) { $receipt.status = 'failed' }
    if ($receipt.status -eq 'compiled') { $receipt.stage = 'complete' }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 8))
    $file = [IO.File]::Open((Join-Path $root 'build.safe.json'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush() } finally { $file.Dispose() }
}
if ($null -ne $failure) { throw $failure }
if ($postFailure) { throw '构建输入发生变化，拒绝发布 reader' }
"INFINISHELL_CLR_READER=$executable" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
