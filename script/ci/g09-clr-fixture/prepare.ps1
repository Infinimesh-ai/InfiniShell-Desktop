param([Parameter(Mandatory = $true)][string]$Reader)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([IntPtr]::Size -ne 8 -or $env:OS -ne 'Windows_NT' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or $PSVersionTable.PSVersion.Major -ne 5 -or
    $env:GITHUB_ACTIONS -ne 'true' -or $env:GITHUB_RUN_ID -notmatch '^\d+$' -or
    $env:GITHUB_RUN_ATTEMPT -notmatch '^\d+$' -or [string]::IsNullOrEmpty($env:GITHUB_ENV)) {
    throw 'CLR 夹具只能由 Windows x64 runner 的 Windows PowerShell 5.1 准备'
}
$base = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
if (-not [IO.Directory]::Exists($base) -or $base.StartsWith('\\')) {
    throw '缺少既有本地 runner 临时根'
}
$root = Join-Path $base ('g09-clr-fixture-' + $env:GITHUB_RUN_ID + '-' + $env:GITHUB_RUN_ATTEMPT)
if (Test-Path -LiteralPath $root) { throw 'CLR 夹具证据目录已存在' }
$directory = New-Item -ItemType Directory -Path $root
$owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetOwner($owner)
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
    $owner, [Security.AccessControl.FileSystemRights]::FullControl,
    [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
    [Security.AccessControl.PropagationFlags]::None,
    [Security.AccessControl.AccessControlType]::Allow))
Set-Acl -LiteralPath $directory.FullName -AclObject $acl
"INFINISHELL_CLR_FIXTURE_ROOT=$root" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8

# 只编译固定 C#，不启动它，也不在准备阶段加载 DAC。
$source = Join-Path $PSScriptRoot 'Fixture.cs'
$compiler = Join-Path $env:SystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$inputReader = (Resolve-Path -LiteralPath $Reader).Path
$copiedReader = Join-Path $root 'reader.exe'
$fixture = Join-Path $root 'fixture.exe'
$receipt = [ordered]@{
    schema = 1; source_commit = $env:GITHUB_SHA; status = 'preparing'
    native_execution_started = $false; cleanup_ready = $false
    scope = '固定 Framework 异常读取夹具；不代表 PowerShell 或 G09 通过'
}
try {
    foreach ($path in @($source, $compiler, $inputReader)) {
        $item = Get-Item -LiteralPath $path
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw '固定输入必须是普通文件'
        }
    }
    $readerHash = (Get-FileHash -LiteralPath $inputReader -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $inputReader -Destination $copiedReader
    if ((Get-FileHash -LiteralPath $copiedReader -Algorithm SHA256).Hash.ToLowerInvariant() -ne $readerHash) {
        throw 'reader 副本摘要不匹配'
    }
    $receipt.compiler_sha256 = (Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash.ToLowerInvariant()
    $receipt.compiler_version = (Get-Item -LiteralPath $compiler).VersionInfo.FileVersion
    $receipt.source_sha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    & $compiler /nologo /target:exe /platform:x64 /optimize- /debug- /warnaserror+ "/out:$fixture" $source *> (Join-Path $root 'compiler.log')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $fixture)) {
        throw '固定 Framework 夹具编译失败'
    }
    $receipt.fixture_sha256 = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
    # 只读取刚编译原件的元数据；不调用 Main、类型初始化器或异常方法。
    $assembly = [Reflection.Assembly]::ReflectionOnlyLoadFrom($fixture)
    $receipt.fixture_mvid = $assembly.ManifestModule.ModuleVersionId.ToString('D')
    $type = $assembly.GetType('ClrFixture', $true)
    $flags = [Reflection.BindingFlags]'Static, NonPublic'
    $receipt.fixture_method_tokens = @('ThrowNative', 'ThrowSecondNative', 'WrapNative', 'WrapOperation', 'Exercise') | ForEach-Object {
        $method = $type.GetMethod($_, $flags)
        if ($null -eq $method) { throw '固定夹具方法元数据缺失' }
        $method.MetadataToken
    }
    $receipt.reader_sha256 = $readerHash
    $receipt.status = 'prepared'
} catch {
    # 不保存异常消息，路径及环境内容不进入安全收据。
    $receipt.status = 'failed'
    $receipt.error_kind = $_.Exception.GetType().FullName
    $receipt.error_hresult = $_.Exception.HResult
    throw
} finally {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 5))
    $file = [IO.File]::Open((Join-Path $root 'preparation.safe.json'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush() } finally { $file.Dispose() }
}
