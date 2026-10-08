# 仅加载原脚本的函数定义，验证内存/本轮临时夹具；不执行 Windows 收集入口。
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot 'collect_g09_windows_images.ps1'
$tokens = $null; $errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw ($errors | Out-String) }
foreach ($name in @('Get-FixedCandidates', 'Get-StreamSha256', 'Copy-MatchingStream', 'Get-SafeFailure')) {
    $nodes = @($ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name }, $true))
    if ($nodes.Count -ne 1) { throw 'function_extraction_ambiguous' }
    . ([scriptblock]::Create($nodes[0].Extent.Text))
}
# 只编译固定 P/Invoke 声明，不在本机调用任何 Windows API。
$native = @($ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.StringConstantExpressionAst] -and $node.Value.StartsWith('using System;') }, $true))
if ($native.Count -ne 1) { throw 'native_declaration_ambiguous' }
Add-Type -TypeDefinition $native[0].Value

function Assert-True([bool] $Value, [string] $Reason) { if (-not $Value) { throw $Reason } }
$passed = 0
$candidates = @(Get-FixedCandidates)
Assert-True ($candidates.Count -eq 4) 'fixed_candidate_count'
Assert-True (($candidates.name -join ',') -ceq 'cmd.exe,ntdll.dll,kernel32.dll,kernelbase.dll') 'fixed_candidate_names'
Assert-True (@($candidates | Where-Object { $_.sha256 -cnotmatch '^[a-f0-9]{64}$' }).Count -eq 0) 'fixed_hash_format'
$passed++

$root = Join-Path ([System.IO.Path]::GetTempPath()) ('g09-image-contract-test-' + [Guid]::NewGuid().ToString('N'))
[void][System.IO.Directory]::CreateDirectory($root)
$bytes = [System.Text.Encoding]::UTF8.GetBytes('固定静态映像夹具；不执行。')
$source = [System.IO.MemoryStream]::new($bytes)
try {
    $expected = Get-StreamSha256 $source
    $rejectedPath = Join-Path $root 'wrong-hash.pe'
    $rejected = $false
    try { [void](Copy-MatchingStream $source ('0' * 64) $rejectedPath) } catch { $rejected = $true }
    Assert-True ($rejected -and -not [System.IO.File]::Exists($rejectedPath)) 'hash_mismatch_must_not_create_output'
    $passed++

    $copyPath = Join-Path $root 'matching.pe'
    $copy = Copy-MatchingStream $source $expected $copyPath
    Assert-True ($copy.before -ceq $expected -and $copy.after -ceq $expected -and $copy.copied -ceq $expected -and $copy.size -eq $bytes.Length) 'full_source_and_copy_hashes'
    Assert-True ([Convert]::ToHexString([System.IO.File]::ReadAllBytes($copyPath)) -ceq [Convert]::ToHexString($bytes)) 'copy_bytes_differ'
    $passed++

    $rejected = $false
    try { [void](Copy-MatchingStream $source $expected $copyPath) } catch { $rejected = $true }
    Assert-True $rejected 'existing_output_must_not_be_overwritten'
    Assert-True ([Convert]::ToHexString([System.IO.File]::ReadAllBytes($copyPath)) -ceq [Convert]::ToHexString($bytes)) 'existing_output_changed'
    $passed++

    $failure = Get-SafeFailure ([System.ComponentModel.Win32Exception]::new(5, '不能输出这个任意路径'))
    Assert-True ($failure.win32 -eq 5 -and ($failure | ConvertTo-Json -Compress) -notmatch '任意路径') 'safe_win32_failure'
    $passed++
} finally {
    $source.Dispose()
    # 仅删除本次新建的固定夹具；真实收集脚本不执行删除。
    [System.IO.Directory]::Delete($root, $true)
}
@{ passed = $passed; parser = 'passed'; native_declarations = 'compiled_only_not_invoked'; windows_collection_executed = $false } | ConvertTo-Json -Compress
