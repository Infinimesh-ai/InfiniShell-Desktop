param([Parameter(Mandatory = $true)][string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-PlainPath([string]$Path) {
    $item = Get-Item -LiteralPath $Path
    while ($null -ne $item) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw '诊断路径包含重解析点'
        }
        $item = if ($item -is [IO.DirectoryInfo]) { $item.Parent } else { $item.Directory }
    }
}

function Get-FileIdentity([string]$Path) {
    Assert-PlainPath $Path
    $before = Get-Item -LiteralPath $Path
    if ($before.PSIsContainer -or $before.Length -le 0) { throw '构建输入不是非空普通文件' }
    $length = $before.Length
    $writeTicks = $before.LastWriteTimeUtc.Ticks
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    $after = Get-Item -LiteralPath $Path
    if ($after.Length -ne $length -or $after.LastWriteTimeUtc.Ticks -ne $writeTicks) {
        throw '计算摘要期间文件发生变化'
    }
    return [ordered]@{ path = $after.FullName; bytes = $length; sha256 = $hash; last_write_ticks = $writeTicks }
}

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$root = [IO.Path]::GetFullPath($env:INFINISHELL_EXIT_DIAGNOSTIC_ROOT)
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $root.StartsWith($runnerTemp + '\', [StringComparison]::OrdinalIgnoreCase) -or
    $output -cne (Join-Path $root 'build')) { throw '必须使用本轮 RUNNER_TEMP 下的独占构建目录' }
Assert-PlainPath $runnerTemp
Assert-PlainPath (Split-Path -Parent $root)
if (Test-Path -LiteralPath $root) { throw '本轮诊断根已存在，拒绝复用或覆盖' }
New-Item -ItemType Directory -Path $root | Out-Null
New-Item -ItemType Directory -Path $output | Out-Null
Assert-PlainPath $output

$receipt = [ordered]@{
    schema = 1; status = 'started'; source_sha = $null; source_sha256 = $null
    executable = $null; compiler_exit_code = $null; cleanup_ready = $false
    uploaded_build_files = @('build-receipt.json', 'compiler.log')
    scope = '仅构建本轮直接libtest退出观察器；不执行产品、协议夹具或测试，不改变共享配置'
}
$failed = $false
$source = Join-Path $PSScriptRoot 'windows_exit_observer.cpp'
$executable = Join-Path $output 'windows-exit-observer.exe'
$sourceBefore = $null
$scriptBefore = $null
try {
    $head = & git -C $repo rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $head.Trim() -cne $env:GITHUB_SHA) { throw '构建源码与派发身份不一致' }
    $receipt.source_sha = $head.Trim()
    $sourceBefore = Get-FileIdentity $source
    $scriptBefore = Get-FileIdentity $PSCommandPath
    $receipt.source_sha256 = $sourceBefore.sha256
    $receipt.build_script_sha256 = $scriptBefore.sha256
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $receipt.vswhere = Get-FileIdentity $vswhere
    $installations = @(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if ($LASTEXITCODE -ne 0 -or $installations.Count -ne 1 -or [string]::IsNullOrWhiteSpace($installations[0])) {
        throw '找不到唯一既有 MSVC x64 工具链'
    }
    $vcvars = Join-Path $installations[0].Trim() 'VC\Auxiliary\Build\vcvars64.bat'
    $receipt.vcvars64 = Get-FileIdentity $vcvars
    $object = Join-Path $output 'windows-exit-observer.obj'
    $compilerPdb = Join-Path $output 'compiler.pdb'
    $linkerPdb = Join-Path $output 'windows-exit-observer.pdb'
    $build = Join-Path $output 'build.cmd'
    $compilerPath = Join-Path $output 'compiler-path.txt'
    foreach ($path in @($vcvars, $source, $output)) {
        if ($path -match '[%!"&|<>^]' -or $path -match '[^\x20-\x7e]') {
            throw 'CMD 构建路径不能包含元字符或非 ASCII 字符'
        }
    }
    # 只在独立 cmd 子进程加载工具链，清除隐式编译选项，不修改共享环境。
    @(
        '@echo off'
        "call `"$vcvars`" >nul || exit /b 1"
        'set "CL="'
        'set "_CL_="'
        'set "LINK="'
        'set "_LINK_="'
        "where cl.exe > `"$compilerPath`" || exit /b 1"
        "cl.exe /nologo /std:c++17 /EHsc /utf-8 /W4 /O2 /Zi `"$source`" /Fo`"$object`" /Fd`"$compilerPdb`" /Fe`"$executable`" /link /DEBUG /PDB:`"$linkerPdb`" Dbghelp.lib Psapi.lib || exit /b 1"
        'exit /b 0'
    ) | Set-Content -LiteralPath $build -Encoding ascii
    $receipt.compiler_options = @('/std:c++17', '/EHsc', '/utf-8', '/W4', '/O2', '/Zi', '/link', '/DEBUG', 'Dbghelp.lib', 'Psapi.lib')
    & $env:ComSpec /d /v:off /c "`"$build`"" *> (Join-Path $output 'compiler.log')
    $receipt.compiler_exit_code = $LASTEXITCODE
    if ($LASTEXITCODE -ne 0) { throw '观察器编译失败，原编译日志保留在本轮目录' }
    $compilerPaths = @([IO.File]::ReadAllLines($compilerPath) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($compilerPaths.Count -lt 1) { throw '缺少实际编译器路径' }
    $compiler = $compilerPaths[0].Trim()
    if (-not $compiler.StartsWith($installations[0].Trim() + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw '实际编译器不在所选 Visual Studio 内'
    }
    $receipt.compiler = Get-FileIdentity $compiler
    $receipt.compiler_file_version = [Diagnostics.FileVersionInfo]::GetVersionInfo($compiler).FileVersion
    $receipt.executable = Get-FileIdentity $executable
    $receipt.status = 'compiled'
} catch {
    $failed = $true
    $receipt.status = 'failed'
    $receipt.error_kind = $_.Exception.GetType().FullName
    $receipt.error_hresult = $_.Exception.HResult
} finally {
    # 原源码与脚本再核摘要；失败也只留收据，不自动重建或删除唯一证据。
    try {
        if ($null -eq $sourceBefore -or $null -eq $scriptBefore) { throw '缺少构建前身份' }
        $sourceAfter = Get-FileIdentity $source
        $scriptAfter = Get-FileIdentity $PSCommandPath
        $headAfter = & git -C $repo rev-parse HEAD
        if ($LASTEXITCODE -ne 0 -or $headAfter.Trim() -cne $receipt.source_sha -or
            $sourceAfter.sha256 -cne $sourceBefore.sha256 -or $scriptAfter.sha256 -cne $scriptBefore.sha256) {
            throw '构建期间源码身份变化'
        }
        $receipt.source_unchanged = $true
    } catch {
        $failed = $true
        $receipt.status = 'failed'
        $receipt.source_unchanged = $false
        $receipt.recheck_error_kind = $_.Exception.GetType().FullName
    }
    $receipt.finished_at_utc = [DateTime]::UtcNow.ToString('o')
    try {
        $compilerLog = Join-Path $output 'compiler.log'
        if (Test-Path -LiteralPath $compilerLog) {
            Assert-PlainPath $compilerLog
            $receipt.compiler_log = [ordered]@{
                path = $compilerLog; bytes = (Get-Item -LiteralPath $compilerLog).Length
                sha256 = (Get-FileHash -LiteralPath $compilerLog -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        }
    } catch {
        $failed = $true
        $receipt.status = 'failed'
        $receipt.compiler_log_error_kind = $_.Exception.GetType().FullName
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 8))
    $stream = [IO.File]::Open((Join-Path $output 'build-receipt.json'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush() } finally { $stream.Dispose() }
}
if ($failed) { throw '观察器构建或身份复核失败；不得继续产品实验' }
