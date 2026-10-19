[CmdletBinding()]
param(
    [string] $SourcePath = (Join-Path $PSScriptRoot 'test_windows_environment.ps1')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# 只编译实际夹具的互操作声明并反射签名，不调用 Windows API 或访问注册表。
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($SourcePath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw '原始字节夹具存在解析错误' }
$definitions = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and
        $node.GetCommandName() -ceq 'Add-Type' -and
        $node.Extent.Text.Contains('public static class InfiniShellRawPathRegistry')
}, $true))
if ($definitions.Count -ne 1 -or
    $definitions[0].CommandElements[-1] -isnot [Management.Automation.Language.StringConstantExpressionAst]) {
    throw '无法唯一定位实际互操作声明，拒绝以替代签名验证'
}
$definition = $definitions[0].CommandElements[-1].Value
$handleType = [Microsoft.Win32.RegistryKey].GetProperty('Handle').PropertyType
if ($handleType -ne [Microsoft.Win32.SafeHandles.SafeRegistryHandle]) { throw '不支持此运行时的注册表句柄类型' }

foreach ($oldPointerSignature in @($false, $true)) {
    $className = 'InfiniShellRawPathRegistryGate' + [Guid]::NewGuid().ToString('N')
    $code = $definition.Replace('InfiniShellRawPathRegistry', $className)
    if ($oldPointerSignature) {
        # 负对照复现本次 PS 5.1 的裸指针签名错配，必须被同一个类型判断拒绝。
        $code = $code.Replace('SafeRegistryHandle key', 'IntPtr key')
    }
    $compiledType = Add-Type -TypeDefinition $code -PassThru
    foreach ($method in @('SetValue', 'QueryValue')) {
        $compatible = $compiledType.GetMethod($method).GetParameters()[0].ParameterType -eq $handleType
        if ($compatible -eq $oldPointerSignature) {
            throw "原始字节夹具句柄签名不符：$method，旧指针负对照=$oldPointerSignature"
        }
    }
}
[pscustomobject]@{ status = 'passed'; cases = 2; assertions = 4 }
