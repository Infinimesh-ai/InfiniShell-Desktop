param([Parameter(Mandatory = $true)][string]$LocalRoot)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$testScriptHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$contextScript = Join-Path $PSScriptRoot 'preparation-context.ps1'
$contextScriptHash = (Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant()
. $contextScript
if ((Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $contextScriptHash) {
    throw '受控准备入口在加载期间发生变化'
}
if ($env:GITHUB_ACTIONS -eq 'true') { throw '本地入口测试不在 runner 内运行' }
$results = [Collections.Generic.List[string]]::new()
function Assert-Rejected([string]$Name, [scriptblock]$Action) {
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw "入口未拒绝测试：$Name" }
    $results.Add($Name)
}

# 这些测试只检查路径、ACL 和源码身份，不构建或执行原生夹具。
Assert-Rejected 'relative_root' { Assert-G09LocalDirectory '.' }
Assert-Rejected 'unc_root' { Assert-G09LocalDirectory '\\localhost\C$\Temp' }
Assert-Rejected 'device_root' { Assert-G09LocalDirectory '\\?\C:\Temp' }
Assert-Rejected 'parent_escape' { Assert-G09LocalDirectory (Join-Path $LocalRoot '..') }
Assert-Rejected 'alternate_stream' { Assert-G09LocalDirectory ($LocalRoot + ':stream') }
Assert-Rejected 'default_remains_runner_only' { New-G09PreparationContext $false '' '' 'build' }
Assert-Rejected 'local_requires_explicit_root' { New-G09PreparationContext $true '' 'test' 'build' }
Assert-Rejected 'run_id_escape' { New-G09PreparationContext $true $LocalRoot '..\escape' 'build' }

$runId = 'entry-test-' + [Guid]::NewGuid().ToString('N').Substring(0, 12)
$context = New-G09PreparationContext $true $LocalRoot $runId 'build'
$root = $context.Root
$receipt = [ordered]@{
    schema = 1; status = 'started'; run_id = $runId; source_identity = $context.SourceIdentity
    test_script_sha256 = $testScriptHash; context_script_sha256 = $contextScriptHash; sources_unchanged = $false
    native_execution_started = $false; dac_loaded = $false; cleanup_ready = $false; tests = @()
}
try {
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $root 'local-entry.tests.ps1')
    Copy-Item -LiteralPath $contextScript -Destination (Join-Path $root 'preparation-context.ps1')
    if ($context.Mode -cne 'local' -or $context.SourceIdentity.commit -cnotmatch '^[a-f0-9]{40}$') {
        throw '本地上下文未保留真实源码身份'
    }
    $results.Add('private_local_context')
    $beforeAcl = (Get-Acl -LiteralPath $root).Sddl
    Assert-Rejected 'existing_run_not_overwritten' { New-G09PreparationContext $true $LocalRoot $runId 'build' }
    if ((Get-Acl -LiteralPath $root).Sddl -cne $beforeAcl) { throw '拒绝既有目录时修改了 ACL' }
    $results.Add('existing_acl_unchanged')
    $target = New-Item -ItemType Directory -Path (Join-Path $root 'target')
    $link = Join-Path $root 'junction'
    New-Item -ItemType Junction -Path $link -Target $target.FullName | Out-Null
    try {
        Assert-Rejected 'reparse_root' { Assert-G09LocalDirectory $link }
        New-Item -ItemType Directory -Path (Join-Path $target.FullName 'child') | Out-Null
        Assert-Rejected 'reparse_ancestor' { Assert-G09LocalDirectory (Join-Path $link 'child') }
    } finally {
        # 只移除已核本轮路径的链接本身；不递归、不沿链接访问目标。
        if ([IO.Path]::GetDirectoryName($link) -cne $root -or
            -not ((Get-Item -LiteralPath $link -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw '本轮测试链接身份不匹配，保留待核'
        }
        [IO.Directory]::Delete($link)
    }
    $output = @(Write-G09PreparationOutput $context 'INFINISHELL_CLR_BUILD_ROOT' $root)
    if ($output.Count -ne 1 -or $output[0] -cne "INFINISHELL_CLR_BUILD_ROOT=$root") {
        throw '本地输出不可读取'
    }
    $results.Add('readable_local_output')
    $receipt.sources_unchanged = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $testScriptHash -and
        (Get-FileHash -LiteralPath (Join-Path $root 'local-entry.tests.ps1') -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $testScriptHash -and
        (Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $contextScriptHash -and
        (Get-FileHash -LiteralPath (Join-Path $root 'preparation-context.ps1') -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $contextScriptHash
    if (-not $receipt.sources_unchanged) { throw '本地入口测试源码或归档在运行期间发生变化' }
    $receipt.status = 'passed'
} catch {
    $receipt.status = 'failed'
    $receipt.error_kind = $_.Exception.GetType().FullName
    $receipt.error_hresult = $_.Exception.HResult
    throw
} finally {
    $receipt.tests = @($results.ToArray())
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 5))
    $file = [IO.File]::Open((Join-Path $root 'local-entry-tests.safe.json'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush() } finally { $file.Dispose() }
    Write-Output "G09_LOCAL_ENTRY_TEST_ROOT=$root"
}
Write-Output "通过 $($results.Count) 项本地入口边界测试；证据保留，cleanup_ready=false"
