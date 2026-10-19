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
    if (-not $oldPointerSignature -and -not $compiledType.GetMethod('QueryValue').GetParameters()[4].IsOut) {
        throw '原生输出数组必须声明 Out，不能依赖 blittable pin 优化的偶然写回'
    }
}

# 执行真实读取函数的缓冲区与编码路径；假响应只验证 PowerShell 契约，不代替 Windows API。
$readFunctions = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Get-RawPath'
}, $true))
if ($readFunctions.Count -ne 1) { throw '无法唯一定位实际原始字节读取函数' }
$writeFunctions = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Set-RawPath'
}, $true))
if ($writeFunctions.Count -ne 1) { throw '无法唯一定位实际原始字节写入函数' }
$memoryClass = 'InfiniShellRawPathMemoryGate' + [Guid]::NewGuid().ToString('N')
$memoryCode = @'
using System;
using System.Runtime.InteropServices;
public static class RawPathMemoryGate {
    public static byte[] Bytes;
    public static int QueryStatus;
    public static byte[] WrittenBuffer;
    public static uint WrittenSize;
    public static int SetValue(object key, string name, uint reserved,
        uint type, byte[] data, uint size) {
        WrittenBuffer = (byte[])data.Clone();
        WrittenSize = size;
        return 0;
    }
    public static int QueryValue(object key, string name, IntPtr reserved,
        out uint type, [Out] byte[] data, ref uint size) {
        type = 1;
        if (QueryStatus != 0) { return QueryStatus; }
        if (data == null) { size = (uint)Bytes.Length; return 0; }
        if (size < Bytes.Length || data.Length < Bytes.Length) { return 234; }
        Buffer.BlockCopy(Bytes, 0, data, 0, Bytes.Length);
        size = (uint)Bytes.Length;
        return 0;
    }
}
'@
$memoryType = Add-Type -TypeDefinition ($memoryCode.Replace('RawPathMemoryGate', $memoryClass)) -PassThru
. ([scriptblock]::Create($readFunctions[0].Extent.Text.Replace('[InfiniShellRawPathRegistry]', "[$memoryClass]")))
. ([scriptblock]::Create($writeFunctions[0].Extent.Text.Replace('[InfiniShellRawPathRegistry]', "[$memoryClass]")))
$key = [pscustomobject]@{ Handle = $null }
$key | Add-Member -MemberType ScriptMethod -Name Dispose -Value { }
$registry = [pscustomobject]@{ TestKey = $key }
$registry | Add-Member -MemberType ScriptMethod -Name OpenSubKey -Value { param($name) $this.TestKey }
$registry | Add-Member -MemberType ScriptMethod -Name CreateSubKey -Value { param($name) $this.TestKey }
$keyPath = 'offline-fixture'
foreach ($fixture in @(
    @{ bytes = [byte[]]@() },
    @{ bytes = [Text.Encoding]::Unicode.GetBytes('C:\raw') },
    @{ bytes = [byte[]]@(67, 0, 58) }
)) {
    $memoryType.GetField('Bytes').SetValue($null, $fixture.bytes)
    $actual = Get-RawPath 'readback'
    if ($actual.kind -ne 1 -or $actual.bytes -cne [Convert]::ToBase64String($fixture.bytes)) {
        throw '真实原始字节读取函数丢失假响应的字节或类型'
    }
    # 捕获真实函数传给写 API 的缓冲区和计数；哨兵不得进入实际持久化字节数。
    Set-RawPath 'write' $fixture.bytes 1
    $written = $memoryType.GetField('WrittenBuffer').GetValue($null)
    $size = $memoryType.GetField('WrittenSize').GetValue($null)
    if ($size -ne $fixture.bytes.Length -or $written.Length -ne $size + 2 -or
        [Convert]::ToBase64String($written, 0, $size) -cne [Convert]::ToBase64String($fixture.bytes) -or
        $written[$size] -eq 0 -or $written[$size + 1] -eq 0) {
        throw '真实原始字节写入函数未保留精确计数和非零尾哨兵'
    }
}
# 缺失值只在诊断模式下返回明确不存在；权限失败不能混同不存在。
$memoryType.GetField('QueryStatus').SetValue($null, 2)
$rejected = $false
try { Get-RawPath 'missing' | Out-Null } catch { $rejected = $_.Exception.Message.Contains('Win32=2') }
if (-not $rejected) { throw '默认原始夹具读取必须拒绝不存在的 PATH' }
$missing = Get-RawPath 'missing' -AllowMissing
if ($missing.exists -ne $false -or $missing.kind -ne 0 -or $null -ne $missing.bytes) {
    throw '诊断读取未明确记录不存在的 PATH'
}
$memoryType.GetField('QueryStatus').SetValue($null, 5)
$rejected = $false
try { Get-RawPath 'denied' -AllowMissing | Out-Null } catch { $rejected = $_.Exception.Message.Contains('Win32=5') }
if (-not $rejected) { throw '诊断读取不得将权限失败当成不存在' }
[pscustomobject]@{ status = 'passed'; cases = 11; assertions = 14 }
