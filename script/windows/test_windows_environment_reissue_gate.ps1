[CmdletBinding()]
param(
    [string] $SourcePath = (Join-Path $PSScriptRoot 'test_windows_environment_reissue.ps1')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# 从生产控制脚本提取实际最终门禁，仅提供输入夹具，不复制一份判断实现。
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($SourcePath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw '重发控制脚本存在解析错误' }
$cleanup = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.AssignmentStatementAst] -and
        $node.Left.Extent.Text -ceq '$result.cleanup_passed'
}, $true))
$failed = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.AssignmentStatementAst] -and
        $node.Left.Extent.Text -ceq '$result.status' -and $node.Right.Extent.Text -ceq "'failed'"
}, $true))
$passed = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.IfStatementAst] -and
        $node.Extent.Text.Contains('$result.status=''passed''')
}, $true))
if ($cleanup.Count -ne 1 -or $failed.Count -ne 1 -or $passed.Count -ne 1) {
    throw '无法唯一定位重发最终门禁，拒绝用替代逻辑验证'
}
$gate = [scriptblock]::Create((@($cleanup[0].Extent.Text, $failed[0].Extent.Text, $passed[0].Extent.Text) -join "`n"))

foreach ($actualCleanup in @($true, $false)) {
    # 两个夹具都模拟全部产品断言成功，只改变真实测试键清理的读回结果。
    $positiveCases = @(1..40 | ForEach-Object { [pscustomobject]@{ ownership_root_removed = $true } })
    $negativeControls = @(1, 2)
    $result = @{
        execution_error = $null
        failed_assertions = @()
        execution_context_hkcu_environment_path_unchanged = $true
        positive_test_registry_cleanup_passed = $actualCleanup
        status = 'incomplete'
    }
    & $gate
    $expected = 'failed'
    if ($actualCleanup) { $expected = 'passed' }
    if ($result.cleanup_passed -ne $actualCleanup -or $result.status -cne $expected) {
        throw "重发门禁未遵守真实测试键清理结果：cleanup=$actualCleanup，status=$($result.status)"
    }
}
[pscustomobject]@{ status = 'passed'; cases = 2; assertions = 4 }
