[CmdletBinding()]
param(
    [string] $CompilerPath,
    [string] $OutputDirectory,
    [switch] $SkipNativeMatrix
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw '此测试需要 Windows 和 Inno Setup 6' }
if (-not $CompilerPath) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { $CompilerPath = $command.Source }
}
if (-not $CompilerPath -or -not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) {
    throw '请通过 -CompilerPath 指定现有 ISCC.exe；不会下载或安装编译器'
}
$CompilerPath = [IO.Path]::GetFullPath($CompilerPath)
$testId = [Guid]::NewGuid().ToString('N')
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) "infinishell-environment-$testId"
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) {
    if (@(Get-ChildItem -LiteralPath $OutputDirectory -Force).Count) { throw '证据目录必须为空' }
} else { New-Item -ItemType Directory -Path $OutputDirectory | Out-Null }

# 每轮保留实际编译的源码，失败后也能独立复查。
$sourceDirectory = Join-Path $OutputDirectory 'source'
New-Item -ItemType Directory -Path $sourceDirectory | Out-Null
foreach ($sourceName in @('environment.iss', 'environment-tests.iss', 'test_windows_environment.ps1', 'environment-legacy-r4.iss', 'environment-action-tests.iss')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $sourceName) -Destination (Join-Path $sourceDirectory $sourceName)
}

$registry = [Microsoft.Win32.Registry]::CurrentUser
$keyPath = "Software\InfiniShellEnvironmentTests\$testId"
$target = 'C:\安装 测试\InfiniShell\bin'
$plain = 'C:\Windows;'
$expand = '%SystemRoot%\System32;'
$fixtures = [ordered]@{}
function Add-Fixture {
    param([string] $Name, [AllowNull()] $Value, [int] $Kind, [AllowNull()] $Expected, [int] $ExpectedKind)
    $fixtures[$Name] = [pscustomobject]@{ value = $Value; kind = $Kind; expected = $Expected; expected_kind = $ExpectedKind }
}
Add-Fixture 'missing' $null 0 $null 0
Add-Fixture 'empty-sz' '' 1 '' 1
Add-Fixture 'empty-expand' '' 2 '' 2
Add-Fixture 'plain' 'C:\Windows' 1 'C:\Windows' 1
Add-Fixture 'separators' ';C:\Windows;;;D:\中文 路径;' 1 ';C:\Windows;;;D:\中文 路径;' 1
Add-Fixture 'expand' '%SystemRoot%\System32;D:\中文;%NOT_DEFINED_InfiniShell%;' 2 '%SystemRoot%\System32;D:\中文;%NOT_DEFINED_InfiniShell%;' 2
Add-Fixture 'delimiter-only' ';;' 1 ';;' 1
Add-Fixture 'preexisting' "C:\Windows;$target;D:\Tools" 1 "C:\Windows;$target;D:\Tools" 1
$equivalent = 'C:\Windows; "' + $target.ToLowerInvariant() + '\" ;D:\Tools'
Add-Fixture 'preexisting-equivalent' $equivalent 2 $equivalent 2
Add-Fixture 'preexisting-duplicates' "$target;C:\Windows;$target" 1 "$target;C:\Windows;$target" 1
Add-Fixture 'preexisting-expand' '%INFINISHELL_PATH_TEST_TARGET%;C:\Windows' 2 '%INFINISHELL_PATH_TEST_TARGET%;C:\Windows' 2
Add-Fixture 'unexpanded-sz' '%INFINISHELL_PATH_TEST_TARGET%;C:\Windows' 1 '%INFINISHELL_PATH_TEST_TARGET%;C:\Windows' 1
Add-Fixture 'legacy-no-owner' "$plain;$target" 1 "$plain;$target" 1
Add-Fixture 'upgrade' $plain 1 $plain 1
Add-Fixture 'upgrade-edited' $plain 1 'D:\Added;;C:\Windows;' 1
Add-Fixture 'edited-append' $plain 1 'C:\Windows;;D:\Added;;' 1
Add-Fixture 'edited-prefix' $plain 1 'D:\Added;;C:\Windows;' 1
Add-Fixture 'edited-replace' $plain 1 'C:\Changed;D:\Added' 1
Add-Fixture 'edited-reorder' $plain 1 'D:\Added;C:\Windows' 1
Add-Fixture 'edited-type-expand' $plain 1 $plain 2
Add-Fixture 'edited-type-sz' $expand 2 $expand 1
Add-Fixture 'edited-delete' $plain 1 $null 0
Add-Fixture 'edited-empty' $plain 1 '' 1
Add-Fixture 'edited-duplicate' $plain 1 "$plain;$target;$target" 1
Add-Fixture 'edited-quoted' $plain 1 ('C:\Windows;"' + $target + '"') 1
Add-Fixture 'edited-trailing-slash' $plain 1 ("C:\Windows;$target" + '\') 1
Add-Fixture 'edited-longer' $plain 1 "C:\Windows;$target-other" 1
Add-Fixture 'missing-user-added' $null 0 'D:\Added' 2
Add-Fixture 'two-products' $plain 1 $plain 1
Add-Fixture 'tui-migration' $plain 1 $plain 1
Add-Fixture 'wrong-type' 7 4 7 4
foreach ($name in @('read-denied', 'write-denied', 'record-denied')) { Add-Fixture $name $plain 1 $plain 1 }
Add-Fixture 'write-denied-missing' $null 0 $null 0
Add-Fixture 'remove-denied' "$plain;$target" 1 "$plain;$target" 1
Add-Fixture 'ownership-remove-denied' "$plain;$target" 1 "$plain;$target" 1

$orders = @('AB', 'BA', 'ABC', 'ACB', 'BAC', 'BCA', 'CAB', 'CBA')
$origins = @(
    @{ value = $null; kind = 0 }, @{ value = ''; kind = 1 }, @{ value = ''; kind = 2 },
    @{ value = ';C:\Windows;;;D:\中文 路径;'; kind = 1 },
    @{ value = '%SystemRoot%\System32;;%NOT_DEFINED_InfiniShell%;'; kind = 2 }
)
for ($kindIndex = 0; $kindIndex -lt $origins.Count; $kindIndex++) {
    foreach ($order in $orders) {
        $origin = $origins[$kindIndex]
        Add-Fixture "matrix-$kindIndex-$order" $origin.value $origin.kind $origin.value $origin.kind
    }
}
foreach ($name in @('cross-upgrade', 'r4-single', 'r4-chain', 'pending-add-before', 'pending-add-after',
    'pending-remove-before', 'pending-remove-after', 'cleanup-denied-retry', 'state-denied-retry',
    'shared-write-denied', 'pending-write-denied')) { Add-Fixture $name $null 0 $null 0 }
Add-Fixture 'cross-edit-0' $null 0 '' 1
Add-Fixture 'cross-edit-1' $null 0 '' 2
Add-Fixture 'cross-edit-2' $null 0 $null 0
Add-Fixture 'cross-edit-3' $null 0 '' 1
Add-Fixture 'cross-edit-4' $null 0 'D:\User' 2
Add-Fixture 'cross-edit-5' $null 0 '' 1
Add-Fixture 'r4-incomplete' $null 0 '' 2
Add-Fixture 'old-helper-interleaving' $null 0 $null 0
Add-Fixture 'pending-user-edit' $null 0 '' 1
Add-Fixture 'invalid-state' $null 0 $null 0
Add-Fixture 'invalid-pending' $null 0 $null 0

foreach ($name in @('pending-owner-prepare-0', 'pending-owner-prepare-1', 'stale-empty-group', 'owner-delete-denied', 'group-delete-denied')) { Add-Fixture $name $null 0 $null 0 }

foreach ($name in @('owner-enum-denied', 'projection-create-denied', 'projection-second-denied', 'projection-half-write', 'projection-cleanup-denied', 'native-r5-state-0')) {
    Add-Fixture $name $null 0 $null 0
}
foreach ($name in @('native-r5-state-1', 'native-r5-state-2', 'native-anchor-ambiguous-0', 'native-anchor-ambiguous-1')) { Add-Fixture $name $null 0 '' 2 }
Add-Fixture 'projection-third-path' $null 0 "$target;D:\TUI Tools\bin;D:\User" 2
Add-Fixture 'projection-old-remove' $null 0 'D:\TUI Tools\bin' 2
Add-Fixture 'projection-old-add' $null 0 "$target;D:\TUI Tools\bin;E:\第三个 产品\bin" 2
Add-Fixture 'invalid-root-value' $null 0 $null 0
foreach ($i in 0..5) { Add-Fixture "invalid-owner-$i" $null 0 $target 2 }

function Get-Actual {
    param([string] $SubKey)
    $key = $registry.OpenSubKey($SubKey)
    try {
        if (-not $key -or $key.GetValueNames() -notcontains 'Path') {
            return [pscustomobject]@{ value = $null; kind = 0 }
        }
        [pscustomobject]@{
            value = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            kind = [int]$key.GetValueKind('Path')
        }
    } finally { if ($key) { $key.Dispose() } }
}
function Set-Denial {
    param([string] $SubKey, [Security.AccessControl.RegistryRights] $Rights)
    $key = $registry.OpenSubKey($SubKey,
        [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,
        ([Security.AccessControl.RegistryRights]::ReadKey -bor [Security.AccessControl.RegistryRights]::ChangePermissions))
    try {
        $acl = $key.GetAccessControl()
        $original = $acl.GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]::Access)
        $rule = [Security.AccessControl.RegistryAccessRule]::new(
            [Security.Principal.WindowsIdentity]::GetCurrent().User, $Rights,
            [Security.AccessControl.AccessControlType]::Deny)
        $acl.AddAccessRule($rule)
        $key.SetAccessControl($acl)
        [pscustomobject]@{ path = $SubKey; original_acl = $original }
    } finally { $key.Dispose() }
}
function Restore-Denial {
    param($Denied)
    $key = $registry.OpenSubKey($Denied.path,
        [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,
        [Security.AccessControl.RegistryRights]::ChangePermissions)
    try {
        $acl = New-Object Security.AccessControl.RegistrySecurity
        $acl.SetSecurityDescriptorSddlForm($Denied.original_acl, [Security.AccessControl.AccessControlSections]::Access)
        $key.SetAccessControl($acl)
    } finally { $key.Dispose() }
}
function Invoke-HiddenProcess {
    param([string] $FilePath, [string[]] $Arguments, [string] $LogName)
    $quoted = foreach ($argument in $Arguments) {
        if ($argument -match '["\r\n]') { throw '进程参数含不允许的引号或换行' }
        '"' + ($argument -replace '(\\+)$', '$1$1') + '"'
    }
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $FilePath
    $info.Arguments = $quoted -join ' '
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $stdout = [IO.File]::Open((Join-Path $OutputDirectory "$LogName.stdout.log"), [IO.FileMode]::CreateNew)
    $stderr = [IO.File]::Open((Join-Path $OutputDirectory "$LogName.stderr.log"), [IO.FileMode]::CreateNew)
    $process = New-Object Diagnostics.Process
    try {
        $process.StartInfo = $info
        if (-not $process.Start()) { throw '测试进程未启动' }
        $outTask = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $errTask = $process.StandardError.BaseStream.CopyToAsync($stderr)
        $process.WaitForExit()
        $null = $outTask.GetAwaiter().GetResult()
        $null = $errTask.GetAwaiter().GetResult()
        $process.ExitCode
    } finally { $process.Dispose(); $stdout.Dispose(); $stderr.Dispose() }
}

$script:nativeActionIndex = 0
$nativePaths = @('C:\安装 测试\InfiniShell\bin', 'D:\TUI Tools\bin', 'E:\第三个 产品\bin')
function New-NativeCase {
    param([string] $Name, $Initial, [int] $Kind, $Expected, [int] $ExpectedKind)
    Add-Fixture $Name $Initial $Kind $Expected $ExpectedKind
    $key = $registry.CreateSubKey("$keyPath\$Name")
    try { if ($Kind) { $key.SetValue('Path', $Initial, [Microsoft.Win32.RegistryValueKind]$Kind) } }
    finally { $key.Dispose() }
}
function Read-Ownership {
    param([string] $Name)
    $group = $registry.OpenSubKey("$keyPath\$Name\InfiniShellPathOwnership")
    if (-not $group) { return $null }
    try {
        $state = [ordered]@{ values = [ordered]@{}; owners = [ordered]@{} }
        foreach ($valueName in $group.GetValueNames()) { $state.values[$valueName] = $group.GetValue($valueName) }
        foreach ($ownerName in $group.GetSubKeyNames()) {
            $owned = $group.OpenSubKey($ownerName)
            try {
                $record = [ordered]@{}
                foreach ($valueName in $owned.GetValueNames()) { $record[$valueName] = $owned.GetValue($valueName) }
                $state.owners[$ownerName] = $record
            } finally { $owned.Dispose() }
        }
        return $state
    } finally { $group.Dispose() }
}
function Invoke-NativeAction {
    param([string] $Name, [string] $Flavor, [string] $Action, [int] $Product, [bool] $ExpectedSuccess = $true)
    $script:nativeActionIndex++
    $receipt = 'action-{0:d5}' -f $script:nativeActionIndex
    $letter = @('A', 'B', 'C')[$Product]
    $code = Invoke-HiddenProcess (Join-Path $OutputDirectory "environment-action-$Flavor.exe") @(
        '/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/case=$Name", "/action=$Action",
        "/product=$letter", "/receipt=$receipt", "/LOG=$(Join-Path $OutputDirectory "$receipt.inno.log")"
    ) $receipt
    $expectedReceipt = "${testId}:$([int]$ExpectedSuccess)"
    if ($code -ne 1 -or [IO.File]::ReadAllText((Join-Path $OutputDirectory "$receipt.txt")) -cne $expectedReceipt) {
        throw "真实 $Flavor helper 动作结果不符：$Name $Action $letter"
    }
    # 每个旧进程返回后立刻读取，绝不再调新版收尾来掩盖旧版残留。
    $snapshot = [ordered]@{ case = $Name; helper = $Flavor; action = $Action; product = $letter;
        receipt = $receipt; expected_success = $ExpectedSuccess; path = (Get-Actual "$keyPath\$Name"); ownership = (Read-Ownership $Name) }
    ($snapshot | ConvertTo-Json -Compress -Depth 12) | Add-Content -LiteralPath (Join-Path $OutputDirectory 'native-actions.jsonl') -Encoding UTF8
    $result.native_actions = $script:nativeActionIndex
}
function Assert-NativeState {
    param([string] $Name, $Value, [int] $Kind, [int] $Owners)
    $actual = Get-Actual "$keyPath\$Name"
    if ($actual.kind -ne $Kind -or [string]$actual.value -cne [string]$Value) { throw "真实旧新版逐步 PATH 核验失败：$Name" }
    $ownership = Read-Ownership $Name
    if ($Owners -eq 0) {
        if ($ownership) { throw "真实最后卸载后仍有归属根：$Name" }
    } else {
        if (-not $ownership -or $ownership.owners.Count -ne $Owners -or $ownership.values.Count) {
            throw "真实 helper 移交元数据不符：$Name"
        }
        foreach ($owner in $ownership.owners.Values) {
            if ($owner.Count -ne 4 -or $owner.Committed -ne 1) { throw "真实 helper 归属未提交：$Name" }
        }
    }
    $result.native_step_assertions += 2
}
function Compile-NativeHelpers {
    foreach ($flavor in @('new', 'old')) {
        $helper = 'environment.iss'
        if ($flavor -eq 'old') { $helper = 'environment-legacy-r4.iss' }
        $code = Invoke-HiddenProcess $CompilerPath @(
            (Join-Path $sourceDirectory 'environment-action-tests.iss'), "/DHelperFile=$helper",
            "/DHelperFlavor=$flavor", "/DTestId=$testId", "/DEvidenceDir=$OutputDirectory"
        ) "compiler-action-$flavor"
        if ($code -ne 0) { throw '独立旧新版 helper 编译失败' }
    }
}
function Invoke-LegacyMatrix {
    foreach ($kindIndex in 0..4) {
        $origin = $origins[$kindIndex]
        foreach ($pattern in @('ON', 'NO', 'OON', 'ONO', 'NOO', 'ONN', 'NON', 'NNO')) {
            $count = $pattern.Length
            $removals = @('AB', 'BA')
            if ($count -eq 3) { $removals = @('ABC', 'ACB', 'BAC', 'BCA', 'CAB', 'CBA') }
            foreach ($removal in $removals) {
                $name = "native-$kindIndex-$pattern-$removal"
                New-NativeCase $name $origin.value $origin.kind $origin.value $origin.kind
                $active = @($false, $false, $false)
                $expected = [string]$origin.value
                $installedKind = $origin.kind
                if ($installedKind -eq 0) { $installedKind = 2 }
                for ($product = 0; $product -lt $count; $product++) {
                    $flavor = 'new'; if ($pattern[$product] -eq 'O') { $flavor = 'old' }
                    Invoke-NativeAction $name $flavor 'add' $product
                    if ($expected) { $expected += ';' }
                    $expected += $nativePaths[$product]
                    $active[$product] = $true
                    Assert-NativeState $name $expected $installedKind ($product + 1)
                }
                for ($step = 0; $step -lt $count; $step++) {
                    $product = 'ABC'.IndexOf($removal[$step])
                    $flavor = 'new'; if ($pattern[$product] -eq 'O') { $flavor = 'old' }
                    Invoke-NativeAction $name $flavor 'remove' $product
                    $active[$product] = $false
                    $expected = [string]$origin.value
                    for ($i = 0; $i -lt $count; $i++) {
                        if ($active[$i]) {
                            if ($expected) { $expected += ';' }
                            $expected += $nativePaths[$i]
                        }
                    }
                    $expectedKind = $installedKind
                    if ($step -eq $count - 1) { $expectedKind = $origin.kind }
                    Assert-NativeState $name $expected $expectedKind ($count - $step - 1)
                }
                $result.native_matrix_cases++
            }
        }
    }
}

function Remove-TestToken {
    param([string] $Value, [string] $Token)
    $tokens = $Value.Split(';')
    $matches = @($tokens | Where-Object { $_ -ceq $Token })
    if ($matches.Count -ne 1) { throw '测试预期只能删除唯一原文 token' }
    return [string]::Join(';', [string[]]@($tokens | Where-Object { $_ -cne $Token }))
}
function Invoke-NativeEdges {
    $a = $nativePaths[0]; $b = $nativePaths[1]
    $edits = @(
        @{ name='between'; value="P;$a;U;$b"; expected='P;U'; keep=@($true,$true) },
        @{ name='separators'; value="P;;$a;$b;"; expected='P;;'; keep=@($true,$true) },
        @{ name='internal-empty'; value="$a;;$b"; expected=''; keep=@($true,$true) },
        @{ name='empty'; value=''; expected=''; keep=@($false,$false) },
        @{ name='duplicate'; value="P;$a;$a;$b"; expected="P;$a;$a"; keep=@($false,$true) },
        @{ name='quoted'; value=('P;"'+$a+'";'+$b); expected=('P;"'+$a+'"'); keep=@($false,$true) },
        @{ name='longer'; value="P;$a-other;$b"; expected="P;$a-other"; keep=@($false,$true) },
        @{ name='original'; value='P'; expected='P'; keep=@($false,$false) }
    )
    foreach ($kind in @(1, 2)) {
        foreach ($edit in $edits) {
            foreach ($order in @('AB','BA')) {
                $name = "native-edit-$kind-$($edit.name)-$order"
                New-NativeCase $name 'P' $kind $edit.expected $kind
                Invoke-NativeAction $name 'old' 'add' 0
                Invoke-NativeAction $name 'new' 'add' 1
                $key=$registry.OpenSubKey("$keyPath\$name",$true)
                try { $key.SetValue('Path',$edit.value,[Microsoft.Win32.RegistryValueKind]$kind) }
                finally { $key.Dispose() }
                # 目标C未拥有PATH，仍必须安全移交其他记录；不新增测试项目。
                Invoke-NativeAction $name 'new' 'remove' 2
                $remaining = [int]$edit.keep[0] + [int]$edit.keep[1]
                $expected = $edit.value
                Assert-NativeState $name $expected $kind $remaining
                foreach ($letter in $order.ToCharArray()) {
                    $product = 'AB'.IndexOf($letter)
                    Invoke-NativeAction $name 'old' 'remove' $product
                    if ($edit.keep[$product]) { $expected = Remove-TestToken $expected $nativePaths[$product]; $remaining-- }
                    Assert-NativeState $name $expected $kind $remaining
                }
                if ($expected -cne $edit.expected) { throw '用户编辑残余预期不一致' }
                $result.native_edge_cases++
            }
        }
    }
    foreach ($kind in @(0,1)) {
        $name = "native-edit-type-$kind"
        $expectedKind = $kind
        New-NativeCase $name $null 0 '' $expectedKind
        Invoke-NativeAction $name 'old' 'add' 0
        Invoke-NativeAction $name 'new' 'add' 1
        $key=$registry.OpenSubKey("$keyPath\$name",$true)
        try {
            if ($kind -eq 0) { $key.DeleteValue('Path') }
            else { $key.SetValue('Path',"$a;$b",[Microsoft.Win32.RegistryValueKind]::String) }
        } finally {$key.Dispose()}
        Invoke-NativeAction $name 'new' 'remove' 2
        Invoke-NativeAction $name 'old' 'remove' 0
        Invoke-NativeAction $name 'old' 'remove' 1
        Assert-NativeState $name '' $expectedKind 0
        $result.native_edge_cases++
    }
    foreach ($i in 0..1) {
        $name = "native-anchor-ambiguous-$i"
        Invoke-NativeAction $name 'new' 'add' 0
        Invoke-NativeAction $name 'old' 'remove' $i
        Invoke-NativeAction $name 'old' 'remove' (1-$i)
        Assert-NativeState $name '' 2 0
        $result.native_edge_cases++
    }
    $name='native-upgrade-legacy-append'
    New-NativeCase $name $null 0 $null 0
    Invoke-NativeAction $name 'new' 'add' 1
    Invoke-NativeAction $name 'old' 'add' 0
    Invoke-NativeAction $name 'old' 'add' 2
    Invoke-NativeAction $name 'old' 'remove' 0
    Invoke-NativeAction $name 'new' 'add' 1
    Invoke-NativeAction $name 'old' 'remove' 1
    Invoke-NativeAction $name 'old' 'remove' 2
    Assert-NativeState $name $null 0 0
    $result.native_edge_cases++
    $name='native-missing-owner-handoff'
    New-NativeCase $name $null 0 $null 0
    Invoke-NativeAction $name 'old' 'add' 0
    Invoke-NativeAction $name 'old' 'add' 1
    Invoke-NativeAction $name 'new' 'remove' 2
    Invoke-NativeAction $name 'old' 'remove' 0
    Invoke-NativeAction $name 'old' 'remove' 1
    Assert-NativeState $name $null 0 0
    $result.native_edge_cases++
    # 边界用例明确记录旧代码的局限，不将这些旧行为标为已修复。
    $name='native-limit-future-old-install'
    New-NativeCase $name $null 0 '' 2
    Invoke-NativeAction $name 'old' 'add' 0
    Invoke-NativeAction $name 'new' 'add' 1
    Invoke-NativeAction $name 'new' 'remove' 0
    Invoke-NativeAction $name 'old' 'add' 2
    Invoke-NativeAction $name 'old' 'remove' 1
    Invoke-NativeAction $name 'old' 'remove' 2
    Assert-NativeState $name '' 2 0
    $result.known_limits += [pscustomobject]@{case=$name; observed='empty REG_EXPAND_SZ'; reason='new owner created after last new helper handoff'}
    $name='native-limit-missing-history'
    New-NativeCase $name $null 0 '' 2
    Invoke-NativeAction $name 'old' 'add' 0
    Invoke-NativeAction $name 'old' 'add' 1
    Invoke-NativeAction $name 'old' 'remove' 0
    Invoke-NativeAction $name 'new' 'add' 1
    Invoke-NativeAction $name 'old' 'remove' 1
    Assert-NativeState $name '' 2 0
    $result.known_limits += [pscustomobject]@{case=$name; observed='empty REG_EXPAND_SZ'; reason='original absence history already removed before new helper attached'}
}

$deniedKeys = @()
$realPathBefore = Get-Actual 'Environment'
$result = [ordered]@{
    schema_version = 1; test_id = $testId; status = 'incomplete'; registry_root = "HKCU\$keyPath"
    compiler = $CompilerPath; helper_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'environment.iss') -Algorithm SHA256).Hash.ToLowerInvariant()
    started_at = [DateTime]::UtcNow.ToString('o'); compiler_exit_code = $null; harness_exit_code = $null
    native_matrix_skipped = [bool]$SkipNativeMatrix; assertions = 0; native_actions = 0; native_step_assertions = 0; native_matrix_cases = 0; native_edge_cases = 0; known_limits = @(); verified_cases = @(); real_user_path_unchanged = $null; cleanup_passed = $false; error = $null
}
try {
    $existing = $registry.OpenSubKey($keyPath)
    if ($existing) { $existing.Dispose(); throw '随机测试键已存在，拒绝覆盖' }
    $root = $registry.CreateSubKey($keyPath)
    $root.SetValue('TestId', $testId, [Microsoft.Win32.RegistryValueKind]::String)
    $root.Dispose()
    foreach ($name in $fixtures.Keys) {
        $fixture = $fixtures[$name]
        $key = $registry.CreateSubKey("$keyPath\$name")
        if ($fixture.kind) { $key.SetValue('Path', $fixture.value, [Microsoft.Win32.RegistryValueKind]$fixture.kind) }
        $key.Dispose()
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $pathId = -join ($sha.ComputeHash([Text.Encoding]::Unicode.GetBytes($target.ToUpperInvariant())) | ForEach-Object { $_.ToString('x2') }) }
    finally { $sha.Dispose() }
    foreach ($name in @('remove-denied', 'ownership-remove-denied')) {
        $owned = $registry.CreateSubKey("$keyPath\$name\InfiniShellPathOwnership\$pathId")
        $owned.SetValue('Committed', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
        $owned.SetValue('OriginalType', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
        $owned.SetValue('OriginalValue', $plain, [Microsoft.Win32.RegistryValueKind]::String)
        $owned.SetValue('AddedPath', $target, [Microsoft.Win32.RegistryValueKind]::String)
        $owned.Dispose()
    }
    $deniedKeys += Set-Denial "$keyPath\read-denied" ([Security.AccessControl.RegistryRights]::QueryValues)
    $deniedKeys += Set-Denial "$keyPath\write-denied" ([Security.AccessControl.RegistryRights]::SetValue)
    $deniedKeys += Set-Denial "$keyPath\write-denied-missing" ([Security.AccessControl.RegistryRights]::SetValue)
    $deniedKeys += Set-Denial "$keyPath\record-denied" ([Security.AccessControl.RegistryRights]::CreateSubKey)
    $deniedKeys += Set-Denial "$keyPath\remove-denied" ([Security.AccessControl.RegistryRights]::SetValue)
    $deniedKeys += Set-Denial "$keyPath\ownership-remove-denied\InfiniShellPathOwnership\$pathId" ([Security.AccessControl.RegistryRights]::SetValue)

    $result.compiler_exit_code = Invoke-HiddenProcess $CompilerPath @(
        (Join-Path $sourceDirectory 'environment-tests.iss'), "/DTestId=$testId", "/DEvidenceDir=$OutputDirectory"
    ) 'compiler'
    if ($result.compiler_exit_code -ne 0) { throw 'ISCC 编译失败，保留 compiler 日志' }
    Compile-NativeHelpers
    # 只供本测试进程及其子进程展开夹具，不持久修改任何环境变量。
    [Environment]::SetEnvironmentVariable('INFINISHELL_PATH_TEST_TARGET', $target, 'Process')
    $result.harness_exit_code = Invoke-HiddenProcess (Join-Path $OutputDirectory 'environment-test.exe') @(
        '/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=$(Join-Path $OutputDirectory 'inno.log')"
    ) 'harness'
    if ($result.harness_exit_code -ne 1) { throw 'InitializeSetup=False 的退出码应为 1' }
    if ([IO.File]::ReadAllText((Join-Path $OutputDirectory 'completed.txt')) -cne $testId) { throw '缺少完整成功回执' }
    Invoke-NativeAction 'projection-old-remove' 'old' 'remove' 0
    Invoke-NativeAction 'projection-old-add' 'old' 'add' 2
    $deniedKeys += Set-Denial "$keyPath\owner-enum-denied\InfiniShellPathOwnership" ([Security.AccessControl.RegistryRights]::EnumerateSubKeys)
    # 在真实隔离键上拒绝写入，覆盖 PATH 已修改后的清理失败和共享状态失败。
    foreach ($name in @('projection-create-denied', 'projection-cleanup-denied')) {
        $deniedKeys += Set-Denial "$keyPath\$name\InfiniShellPathOwnership" ([Security.AccessControl.RegistryRights]::SetValue)
    }
    $deniedKeys += Set-Denial "$keyPath\projection-second-denied\InfiniShellPathOwnership\$pathId" ([Security.AccessControl.RegistryRights]::SetValue)
    $deniedKeys += Set-Denial "$keyPath\owner-delete-denied\InfiniShellPathOwnership\$pathId" ([Security.AccessControl.RegistryRights]::Delete)
    $deniedKeys += Set-Denial "$keyPath\group-delete-denied\InfiniShellPathOwnership" ([Security.AccessControl.RegistryRights]::Delete)
    $deniedKeys += Set-Denial "$keyPath\cleanup-denied-retry\InfiniShellPathOwnership\$pathId" ([Security.AccessControl.RegistryRights]::SetValue)
    foreach ($name in @('state-denied-retry', 'shared-write-denied', 'pending-write-denied')) {
        $deniedKeys += Set-Denial "$keyPath\$name\InfiniShellPathOwnership" ([Security.AccessControl.RegistryRights]::SetValue)
    }
    $result.denied_harness_exit_code = Invoke-HiddenProcess (Join-Path $OutputDirectory 'environment-test.exe') @(
        '/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/phase=denied',
        "/LOG=$(Join-Path $OutputDirectory 'inno-denied.log')"
    ) 'harness-denied'
    if ($result.denied_harness_exit_code -ne 1 -or
        [IO.File]::ReadAllText((Join-Path $OutputDirectory 'completed-denied.txt')) -cne $testId) { throw '拒绝写入阶段未完成' }
    foreach ($denied in $deniedKeys) { Restore-Denial $denied }
    $deniedKeys = @()
    $result.retry_harness_exit_code = Invoke-HiddenProcess (Join-Path $OutputDirectory 'environment-test.exe') @(
        '/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/phase=retry',
        "/LOG=$(Join-Path $OutputDirectory 'inno-retry.log')"
    ) 'harness-retry'
    if ($result.retry_harness_exit_code -ne 1 -or
        [IO.File]::ReadAllText((Join-Path $OutputDirectory 'completed-retry.txt')) -cne $testId) { throw '恢复重试阶段未完成' }
    $assertions = @(Get-Content -LiteralPath (Join-Path $OutputDirectory 'assertions.log'))
    if (-not $assertions.Count -or @($assertions | Where-Object { $_ -notmatch '^PASS: ' }).Count -or
        $assertions[-1] -cne 'PASS: retry completion receipt') { throw '断言证据不完整' }
    $result.assertions = $assertions.Count
    foreach ($name in @('owner-enum-denied', 'projection-create-denied', 'projection-second-denied', 'projection-half-write', 'projection-cleanup-denied')) {
        Invoke-NativeAction $name 'old' 'remove' 0
        if ($name -notin @('owner-enum-denied', 'projection-create-denied')) { Invoke-NativeAction $name 'old' 'remove' 1 }
        Assert-NativeState $name $null 0 0
    }
    foreach ($i in 0..2) {
        $name = "native-r5-state-$i"
        Invoke-NativeAction $name 'new' 'add' 0
        Invoke-NativeAction $name 'old' 'remove' 0
        Invoke-NativeAction $name 'old' 'remove' 1
        $kind = 2; if ($i -eq 0) { $kind = 0 }
        Assert-NativeState $name '' $kind 0
    }
    Invoke-NativeEdges
    if (-not $SkipNativeMatrix) { Invoke-LegacyMatrix }

    foreach ($denied in $deniedKeys) { Restore-Denial $denied }
    $deniedKeys = @()
    foreach ($name in $fixtures.Keys) {
        $actual = Get-Actual "$keyPath\$name"
        $expected = $fixtures[$name]
        if ($actual.kind -ne $expected.expected_kind -or [string]$actual.value -cne [string]$expected.expected) {
            throw "独立注册表核验失败：$name"
        }
        $ownership = $registry.OpenSubKey("$keyPath\$name\InfiniShellPathOwnership")
        try {
            if ($name -in @('remove-denied', 'ownership-remove-denied')) {
                if (-not $ownership -or $ownership.SubKeyCount -ne 1) { throw '卸载写入被拒绝时须保留有效所有权' }
                $owned = $ownership.OpenSubKey($pathId)
                try { if ($owned.GetValue('Committed') -ne 1) { throw '卸载被拒绝时应保持原提交状态' } }
                finally { $owned.Dispose() }
            } elseif ($name -like 'invalid-owner-*' -or $name -in @('projection-third-path', 'projection-old-remove', 'projection-old-add')) {
                if (-not $ownership) { throw '被拒绝或中断第三状态的元数据应保留供诊断' }
            } elseif ($name -in @('invalid-state', 'invalid-pending', 'invalid-root-value')) {
                if (-not $ownership -or $ownership.ValueCount -ne 1) { throw '非法元数据应原样保留供诊断' }
            } elseif ($ownership -and ($ownership.SubKeyCount -gt 0 -or $ownership.ValueCount -gt 0)) {
                throw "残留所有权记录：$name"
            }
        } finally { if ($ownership) { $ownership.Dispose() } }
        $result.verified_cases += [pscustomobject]@{ name = $name; kind = $actual.kind; value = $actual.value }
    }
    $realPathAfter = Get-Actual 'Environment'
    $result.real_user_path_unchanged = $realPathAfter.kind -eq $realPathBefore.kind -and
        [string]$realPathAfter.value -ceq [string]$realPathBefore.value
    if (-not $result.real_user_path_unchanged) { throw '真实用户 PATH 在测试期间发生变化' }
    $result.status = 'passed'
} catch {
    $result.status = 'failed'
    $result.error = $_.Exception.ToString()
} finally {
    try {
        $realPathAfter = Get-Actual 'Environment'
        $result.real_user_path_unchanged = $realPathAfter.kind -eq $realPathBefore.kind -and
            [string]$realPathAfter.value -ceq [string]$realPathBefore.value
        if (-not $result.real_user_path_unchanged) { throw '真实用户 PATH 在测试期间发生变化' }
        if (Test-Path -LiteralPath (Join-Path $OutputDirectory 'assertions.log')) {
            $result.assertions = @(Get-Content -LiteralPath (Join-Path $OutputDirectory 'assertions.log')).Count
        }
        foreach ($denied in $deniedKeys) { Restore-Denial $denied }
        # 失败时保留测试键；成功时仅删除带本轮随机身份的测试根。
        if ($result.status -eq 'passed') {
            if (-not $keyPath.StartsWith('Software\InfiniShellEnvironmentTests\', [StringComparison]::Ordinal) -or
                $keyPath -cne "Software\InfiniShellEnvironmentTests\$testId") { throw '测试根越界' }
            $root = $registry.OpenSubKey($keyPath)
            try { if (-not $root -or $root.GetValue('TestId') -cne $testId) { throw '测试根身份不符' } }
            finally { if ($root) { $root.Dispose() } }
            $registry.DeleteSubKeyTree($keyPath, $false)
            $remaining = $registry.OpenSubKey($keyPath)
            if ($remaining) { $remaining.Dispose(); throw '测试键清理失败' }
            $result.cleanup_passed = $true
        }
    } catch {
        $result.status = 'failed'
        $result.error = "$($result.error) 清理失败：$($_.Exception.Message)"
    }
    $result.finished_at = [DateTime]::UtcNow.ToString('o')
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Encoding UTF8
}
Write-Host "Windows PATH 隔离回归：$($result.status)，证据：$OutputDirectory"
if ($result.status -ne 'passed') { exit 1 }
