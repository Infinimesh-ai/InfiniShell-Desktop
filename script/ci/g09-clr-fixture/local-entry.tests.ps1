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

    # 用独立小仓库复现旧 checkout 的 clean + CRLF；不改真实仓库或伪造 Actions 环境。
    $testRepository = Join-Path $root 'source-repository'
    New-Item -ItemType Directory -Path $testRepository | Out-Null
    $emptyHooks = New-Item -ItemType Directory -Path (Join-Path $root 'empty-hooks')
    function Invoke-TestGit([string[]]$Arguments) {
        $output = @(& git -C $testRepository @Arguments)
        if ($LASTEXITCODE -ne 0) { throw '源码身份测试的独立 Git 操作失败' }
        return $output
    }
    $null = Invoke-TestGit @('init', '--quiet')
    $null = Invoke-TestGit @('config', 'core.autocrlf', 'false')
    $null = Invoke-TestGit @('config', 'core.safecrlf', 'false')
    $repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
    foreach ($relative in (Get-G09SourcePaths)) {
        $destination = Join-Path $testRepository $relative
        New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($destination)) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $repository $relative) -Destination $destination
        if ($relative.EndsWith('.ps1', [StringComparison]::Ordinal)) {
            $contents = [IO.File]::ReadAllText($destination, [Text.Encoding]::UTF8).Replace("`r`n", "`n").Replace("`n", "`r`n")
            [IO.File]::WriteAllText($destination, $contents, [Text.UTF8Encoding]::new($true))
        }
    }
    $attributes = Join-Path $testRepository '.gitattributes'
    [IO.File]::WriteAllText($attributes, "*.ps1 text eol=crlf`n", [Text.UTF8Encoding]::new($false))
    $null = Invoke-TestGit @('add', '--all')
    $commitArguments = @('-c', 'user.name=G09Fixture', '-c', 'user.email=g09-fixture@example.invalid',
        '-c', 'commit.gpgsign=false', '-c', ('core.hooksPath=' + $emptyHooks.FullName), 'commit', '--quiet', '-m')
    $null = Invoke-TestGit ($commitArguments + '固定测试输入')
    [IO.File]::WriteAllText($attributes, "*.ps1 text eol=lf`n", [Text.UTF8Encoding]::new($false))
    $null = Invoke-TestGit @('add', '--', '.gitattributes')
    $null = Invoke-TestGit ($commitArguments + '只调整行尾属性')
    $crlfIdentity = Get-G09SourceIdentity $testRepository
    $receipt.source_byte_cases = [ordered]@{ clean_crlf = $crlfIdentity }
    if ($crlfIdentity.git_status_dirty -or -not $crlfIdentity.dirty -or
        $crlfIdentity.files_exact_to_head -or $crlfIdentity.byte_mismatch_count -ne 3 -or
        @($crlfIdentity.files).Count -ne 13) {
        throw '没有复现过滤后 clean 与实际 CRLF 字节差异'
    }
    $results.Add('clean_crlf_records_byte_mismatch')
    Assert-Rejected 'runner_rejects_clean_crlf' { Assert-G09SourceIdentity $false $crlfIdentity $crlfIdentity.commit }
    Assert-G09SourceIdentity $true $crlfIdentity ''
    $results.Add('local_preserves_explicit_byte_dirty')
    foreach ($relative in (Get-G09SourcePaths)) {
        if ($relative.EndsWith('.ps1', [StringComparison]::Ordinal)) {
            Copy-Item -LiteralPath (Join-Path $repository $relative) -Destination (Join-Path $testRepository $relative)
        }
    }
    $lfIdentity = Get-G09SourceIdentity $testRepository
    $receipt.source_byte_cases.lf = $lfIdentity
    if (-not $lfIdentity.files_exact_to_head -or
        $lfIdentity.byte_mismatch_count -ne 0) { throw '实际 LF 原件未与提交字节一致' }
    Assert-G09SourceIdentity $false $lfIdentity $lfIdentity.commit
    Assert-G09SourceUnchanged $lfIdentity (Get-G09SourceIdentity $testRepository)
    $results.Add('runner_accepts_exact_lf')
    Assert-Rejected 'runner_rejects_wrong_commit' { Assert-G09SourceIdentity $false $lfIdentity ('0' * 40) }
    [IO.File]::AppendAllText((Join-Path $testRepository 'script/ci/g09-clr-reader/reader.cpp'),
        "`n// 仅独立测试仓库中的准备期变化。`n", [Text.UTF8Encoding]::new($false))
    $changedIdentity = Get-G09SourceIdentity $testRepository
    $receipt.source_byte_cases.changed_during_preparation = $changedIdentity
    Assert-Rejected 'preparation_rejects_changed_source' { Assert-G09SourceUnchanged $lfIdentity $changedIdentity }

    # 真实 Local 入口在编译前失败也须保留来源；runner 拒绝由上面的纯断言覆盖。
    [IO.File]::AppendAllText((Join-Path $testRepository 'script/ci/g09-clr-reader/vendor/clrdata.h'),
        "`n// 仅独立测试仓库中的错误 vendor。`n", [Text.UTF8Encoding]::new($false))
    $failedRunId = 'rejected-source'
    $savedErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & (Join-Path $PSHOME 'powershell.exe') -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $testRepository 'script/ci/g09-clr-fixture/build-reader.ps1') `
            -Local -LocalRoot $root -LocalRunId $failedRunId *> (Join-Path $root 'rejected-source.log')
        $failedExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $savedErrorAction }
    $failedBuildRoot = Join-Path $root ('g09-clr-build-local-' + $failedRunId)
    $failedReceiptPath = Join-Path $failedBuildRoot 'build.safe.json'
    $failedReceipt = [IO.File]::ReadAllText($failedReceiptPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
    if ($failedExit -eq 0 -or $failedReceipt.status -cne 'failed' -or
        $failedReceipt.stage -cne 'official_source_binding' -or -not $failedReceipt.source_identity_accepted -or
        -not $failedReceipt.source_identity.dirty -or $failedReceipt.source_identity.files_exact_to_head -or
        $failedReceipt.sources_unchanged -or $failedReceipt.native_execution_started -or $failedReceipt.dac_loaded -or
        $null -ne $failedReceipt.executable -or (Test-Path -LiteralPath (Join-Path $failedBuildRoot 'reader.exe')) -or
        $failedReceipt.PSObject.Properties.Name -contains 'compiler_exit') {
        throw '编译前失败没有保留准确来源和失败状态'
    }
    $receipt.precompile_failure = [ordered]@{
        exit_code = $failedExit; stage = $failedReceipt.stage
        receipt_sha256 = (Get-FileHash -LiteralPath $failedReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        source_identity = $failedReceipt.source_identity; sources_unchanged = $failedReceipt.sources_unchanged
    }
    $results.Add('precompile_failure_preserves_dirty_receipt')

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
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 8))
    $file = [IO.File]::Open((Join-Path $root 'local-entry-tests.safe.json'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush() } finally { $file.Dispose() }
    Write-Output "G09_LOCAL_ENTRY_TEST_ROOT=$root"
}
Write-Output "通过 $($results.Count) 项本地入口边界测试；证据保留，cleanup_ready=false"
