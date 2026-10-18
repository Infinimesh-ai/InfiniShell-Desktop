[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$CompilerPath,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw '此测试需要 Windows 和现有 Inno Setup 编译器' }
$CompilerPath = [IO.Path]::GetFullPath($CompilerPath)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) { throw '编译器不存在；此脚本不会下载或安装编译器' }
if (Test-Path -LiteralPath $OutputDirectory) { throw '拒绝覆盖已存在的证据目录' }
$legacyHash = '630fb260603f0279b97d1e206bbb91688f5178aa64806832166b29366a01307a'
if ((Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'environment-legacy-r4.iss') -Algorithm SHA256).Hash.ToLowerInvariant() -cne $legacyHash) {
    throw '冻结 R4 原始 helper 摘要不符'
}
$sourceDirectory = Join-Path $OutputDirectory 'source'
New-Item -ItemType Directory -Path $sourceDirectory | Out-Null
foreach ($name in @('environment.iss','environment-legacy-r4.iss','environment-action-tests.iss','test_windows_environment_reissue.ps1')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $sourceDirectory $name)
}
$testId = [Guid]::NewGuid().ToString('N')
$keyPath = "Software\InfiniShellEnvironmentTests\$testId"
$registry = [Microsoft.Win32.Registry]::CurrentUser
$utf8Bom = New-Object Text.UTF8Encoding($true)
$utf8Plain = New-Object Text.UTF8Encoding($false)
$paths = @{A='C:\安装 测试\InfiniShell\bin';B='D:\TUI Tools\bin';C='E:\第三个 产品\bin'}
$checks = New-Object 'System.Collections.Generic.List[object]'
$records = New-Object 'System.Collections.Generic.List[object]'
$positiveCases = New-Object 'System.Collections.Generic.List[object]'
$negativeControls = New-Object 'System.Collections.Generic.List[object]'

function Save-Json {
    param([string]$Name,$Value)
    $json = ($Value | ConvertTo-Json -Depth 30) -replace "`r`n", "`n"
    [IO.File]::WriteAllText((Join-Path $OutputDirectory $Name),$json+"`n",$utf8Bom)
}
function Get-Hash {
    param([string]$File)
    (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Get-PathState {
    param([string]$SubKey)
    $key = $registry.OpenSubKey($SubKey)
    try {
        if (-not $key -or $key.GetValueNames() -notcontains 'Path') { return [pscustomobject]@{kind=0;value=$null} }
        [pscustomobject]@{kind=[int]$key.GetValueKind('Path');value=$key.GetValue('Path',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)}
    } finally { if ($key) { $key.Dispose() } }
}
function Get-Tree {
    param([string]$SubKey)
    $key = $registry.OpenSubKey($SubKey)
    try {
        if (-not $key) { return [pscustomobject]@{exists=$false;values=@();children=@()} }
        $values = @(foreach ($name in @($key.GetValueNames() | Sort-Object)) {
            [pscustomobject]@{name=$name;kind=[int]$key.GetValueKind($name);value=$key.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)}
        })
        $children = @(foreach ($name in @($key.GetSubKeyNames() | Sort-Object)) {
            [pscustomobject]@{name=$name;node=(Get-Tree "$SubKey\$name")}
        })
        [pscustomobject]@{exists=$true;values=$values;children=$children}
    } finally { if ($key) { $key.Dispose() } }
}
function Get-State {
    param([string]$CaseName)
    [pscustomobject]@{path=(Get-PathState "$keyPath\$CaseName");ownership=(Get-Tree "$keyPath\$CaseName\InfiniShellPathOwnership")}
}
function Same-Path {
    param($Left,$Right)
    $Left.kind -eq $Right.kind -and [string]$Left.value -ceq [string]$Right.value
}
function Get-PathDigest {
    param($State)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($sha.ComputeHash($utf8Plain.GetBytes([string]$State.value))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
    [pscustomobject]@{exists=($State.kind -ne 0);kind=$State.kind;raw_value_sha256=$hash}
}
function Check {
    param([string]$CaseName,[int]$Step,[string]$Condition,[bool]$Passed)
    $checks.Add([pscustomobject]@{case=$CaseName;step=$Step;condition=$Condition;passed=$Passed})
}
function Expected-Path {
    param($Origin,[string[]]$Active)
    $value = [string]$Origin.value
    foreach ($product in $Active) {
        if ($value -ne '') { $value += ';' }
        $value += $paths[$product]
    }
    $kind = $Origin.kind
    if ($Active.Count -gt 0 -and $kind -eq 0) { $kind = 2 }
    if ($kind -eq 0) { $value = $null }
    [pscustomobject]@{kind=$kind;value=$value}
}
function Invoke-HiddenProcess {
    param([string]$FilePath,[string[]]$Arguments,[string]$LogName)
    $quoted = foreach ($argument in $Arguments) {
        if ($argument -match '["\r\n]') { throw '进程参数含不允许的引号或换行' }
        '"' + ($argument -replace '(\\+)$','$1$1') + '"'
    }
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName=$FilePath; $info.Arguments=$quoted -join ' '
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    $stdout=[IO.File]::Open((Join-Path $OutputDirectory "$LogName.stdout.log"),[IO.FileMode]::CreateNew)
    $stderr=[IO.File]::Open((Join-Path $OutputDirectory "$LogName.stderr.log"),[IO.FileMode]::CreateNew)
    $process=New-Object Diagnostics.Process
    try {
        $process.StartInfo=$info; $started=[DateTime]::UtcNow.ToString('o')
        if (-not $process.Start()) { throw '测试进程未启动' }
        $processId=$process.Id
        $outTask=$process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $errTask=$process.StandardError.BaseStream.CopyToAsync($stderr)
        $process.WaitForExit()
        $null=$outTask.GetAwaiter().GetResult(); $null=$errTask.GetAwaiter().GetResult()
        [pscustomobject]@{executable=(Split-Path $FilePath -Leaf);pid=$processId;started_utc=$started;finished_utc=[DateTime]::UtcNow.ToString('o');arguments=$Arguments;exit_code=$process.ExitCode}
    } finally { $process.Dispose(); $stdout.Dispose(); $stderr.Dispose() }
}
function Run-Case {
    param([string]$Name,$Origin,[string[]]$Operations,[bool]$Negative)
    $firstCheck=$checks.Count
    $key=$registry.CreateSubKey("$keyPath\$Name")
    try { if ($Origin.kind -ne 0) { $key.SetValue('Path',$Origin.value,[Microsoft.Win32.RegistryValueKind]$Origin.kind) } }
    finally { $key.Dispose() }
    $initial=Get-State $Name
    Save-Json "$Name-initial.json" $initial
    $active=@(); $step=0
    foreach ($operation in $Operations) {
        $step++; $parts=$operation.Split(','); $flavor=$parts[0]; $action=$parts[1]; $product=$parts[2]
        $receipt="$Name-$step"
        $invocation=Invoke-HiddenProcess (Join-Path $OutputDirectory "environment-action-$flavor.exe") @('/SP-','/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/case=$Name","/action=$action","/product=$product","/receipt=$receipt","/LOG=$(Join-Path $OutputDirectory "$receipt.log")") $receipt
        $receiptText=[IO.File]::ReadAllText((Join-Path $OutputDirectory "$receipt.txt"))
        # InitializeSetup 只执行 helper 后返回 False；退出码 1 需与身份回执一起判定。
        $nativePassed=($invocation.exit_code -eq 1 -and $receiptText -ceq "${testId}:1")
        Check $Name $step 'native_action_receipt' $nativePassed
        if (-not $nativePassed) { throw "原生动作失败：$receipt" }
        if ($action -eq 'add' -and $active -notcontains $product) { $active += $product }
        if ($action -eq 'remove') { $active=@($active | Where-Object {$_ -cne $product}) }
        $expected=Expected-Path $Origin $active
        # 每步从 .NET 独立读回，末步后不再运行任何 helper。
        $actual=Get-State $Name
        $exact=Same-Path $actual.path $expected
        if (-not $Negative -or $step -lt $Operations.Count) { Check $Name $step 'path_kind_and_raw_value' $exact }
        foreach ($candidate in @('A','B','C')) {
            $count=@(([string]$actual.path.value).Split(';') | Where-Object {$_ -ceq $paths[$candidate]}).Count
            $expectedCount=0; if ($active -contains $candidate) { $expectedCount=1 }
            Check $Name $step "product_${candidate}_presence" ($count -eq $expectedCount)
        }
        $ownerPaths=@(); $validOwners=$true
        foreach ($child in $actual.ownership.children) {
            $fields=$child.node.values
            $names=@($fields | ForEach-Object {$_.name} | Sort-Object)
            if (($names -join '|') -cne 'AddedPath|Committed|OriginalType|OriginalValue' -or $child.node.children.Count -ne 0) { $validOwners=$false; continue }
            $field=@{}; foreach ($entry in $fields) { $field[$entry.name]=$entry }
            if ($field.Committed.kind -ne 4 -or $field.Committed.value -ne 1 -or $field.OriginalType.kind -ne 4 -or $field.OriginalType.value -notin @(0,1,2) -or $field.OriginalValue.kind -ne 1 -or $field.AddedPath.kind -ne 1) { $validOwners=$false }
            $ownerPaths += $field.AddedPath.value
        }
        $expectedOwnerPaths=@($active | ForEach-Object {$paths[$_]} | Sort-Object)
        Check $Name $step 'owner_fields_and_product_set' ($validOwners -and (($ownerPaths | Sort-Object) -join '|') -ceq ($expectedOwnerPaths -join '|'))
        Check $Name $step 'no_shared_journal_after_action' ($actual.ownership.values.Count -eq 0)
        if ($active.Count -eq 0) { Check $Name $step 'ownership_root_removed' (-not $actual.ownership.exists) }
        $record=[pscustomobject]@{case=$Name;negative_control=$Negative;step=$step;helper=$flavor;action=$action;product=$product;active_products=$active;expected_path=$expected;path_exact_match=$exact;invocation=$invocation;receipt=$receiptText;actual=$actual}
        $records.Add($record)
        [IO.File]::AppendAllText((Join-Path $OutputDirectory 'native-actions.jsonl'),($record | ConvertTo-Json -Depth 30 -Compress)+"`n",$utf8Plain)
    }
    $restored=Same-Path $actual.path $initial.path
    $caseStatus='failed'; if ($restored) { $caseStatus='passed' }
    $caseResult=[pscustomobject]@{name=$Name;initial=$initial;actual_final=$actual;actual_restoration_status=$caseStatus;path_exactly_restored=$restored;ownership_root_removed=(-not $actual.ownership.exists);operations=$Operations;native_actions=$step;assertions=($checks.Count-$firstCheck)}
    if ($Negative) {
        # 对照的实际断言保持失败，单独验证已知旧版反例仍可重现，不能冒称旧 EXE 被修复。
        $expectedMismatch=($Origin.kind -eq 0)
        Check $Name $step 'negative_control_observed_expected_mismatch' (($restored -eq (-not $expectedMismatch)) -and $actual.path.kind -eq 2 -and $actual.path.value -ceq '')
        $negativeControls.Add([pscustomobject]@{name=$Name;role='unmodified_R4_future_installer';expected_restoration_status=$(if($expectedMismatch){'failed'}else{'passed'});actual_restoration_status=$caseStatus;expected_actual_mismatch=$expectedMismatch;mismatch_observed=(-not $restored);case_result=$caseResult})
    } else {
        Check $Name $step 'initial_state_exactly_restored' $restored
        $positiveCases.Add($caseResult)
    }
    Save-Json "$Name-result.json" $caseResult
}

$before=Get-PathState 'Environment'
$result=[ordered]@{schema_version=1;status='incomplete';test_id=$testId;registry_root="HKCU\$keyPath";started_utc=[DateTime]::UtcNow.ToString('o');powershell_version=$PSVersionTable.PSVersion.ToString();account_sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;driver_pid=$PID;session_id=[Diagnostics.Process]::GetCurrentProcess().SessionId;compile=@();execution_error=$null;execution_context_hkcu_environment_path_before=(Get-PathDigest $before);desktop_environment_path_verified=$false;localization_audit='无需本地化变更：只新增测试脚本，无用户界面文案变化。'}
try {
    $existing=$registry.OpenSubKey($keyPath)
    if ($existing) { $existing.Dispose(); throw '随机隔离测试键已存在' }
    $key=$registry.CreateSubKey($keyPath)
    try { $key.SetValue('TestId',$testId,[Microsoft.Win32.RegistryValueKind]::String) } finally { $key.Dispose() }
    foreach ($flavor in @('new','old')) {
        $helper='environment.iss'; if ($flavor -eq 'old') { $helper='environment-legacy-r4.iss' }
        $run=Invoke-HiddenProcess $CompilerPath @((Join-Path $sourceDirectory 'environment-action-tests.iss'),"/DHelperFile=$helper","/DHelperFlavor=$flavor","/DTestId=$testId","/DEvidenceDir=$OutputDirectory") "compile-$flavor"
        $result.compile += $run
        if ($run.exit_code -ne 0) { throw "独立动作器编译失败：$flavor" }
    }
    $origins=@(
        [pscustomobject]@{name='absent';kind=0;value=$null},
        [pscustomobject]@{name='empty-sz';kind=1;value=''},
        [pscustomobject]@{name='empty-expand';kind=2;value=''},
        [pscustomobject]@{name='raw-nonempty';kind=1;value=';C:\Windows;;;D:\中文 路径;'},
        [pscustomobject]@{name='expanded-nonempty';kind=2;value='%SystemRoot%\System32;;%NOT_DEFINED_InfiniShell%;'}
    )
    $prefix=@('old,add,A','new,add,B','new,remove,A')
    foreach ($origin in $origins) {
        foreach ($order in @('BC','CB')) {
            $operations=$prefix + @('new,add,C')
            foreach ($product in $order.ToCharArray()) {
                $flavor='new'; if ($product -ceq 'B') { $flavor='old' }
                $operations += "$flavor,remove,$product"
            }
            Run-Case "future-$($origin.name)-$order" $origin $operations $false
        }
        foreach ($order in @('ABC','ACB','BAC','BCA','CAB','CBA')) {
            # C 同目录升级及未来再次新增 A，旧 B 可处于三个卸载位置。
            $operations=$prefix + @('new,add,C','new,add,C','new,add,A')
            foreach ($product in $order.ToCharArray()) {
                $flavor='new'; if ($product -ceq 'B') { $flavor='old' }
                $operations += "$flavor,remove,$product"
            }
            Run-Case "repeated-$($origin.name)-$order" $origin $operations $false
        }
    }
    foreach ($origin in @($origins[0],$origins[2])) {
        Run-Case "negative-$($origin.name)" $origin ($prefix + @('old,add,C','old,remove,B','old,remove,C')) $true
    }
    $result.positive_test_registry_cleanup_passed=$true
    foreach ($case in $positiveCases) {
        if ($case.path_exactly_restored -and $case.ownership_root_removed) {
            $registry.DeleteSubKeyTree("$keyPath\$($case.name)",$false)
            $remaining=$registry.OpenSubKey("$keyPath\$($case.name)")
            if ($remaining) { $remaining.Dispose(); $result.positive_test_registry_cleanup_passed=$false }
        } else { $result.positive_test_registry_cleanup_passed=$false }
    }
    $result.negative_registry_evidence_preserved=$true
} catch { $result.execution_error=$_.Exception.ToString() }
finally {
    $after=Get-PathState 'Environment'
    $result.execution_context_hkcu_environment_path_after=Get-PathDigest $after
    $result.execution_context_hkcu_environment_path_unchanged=Same-Path $before $after
    $result.verified_cases=@($positiveCases.ToArray())
    $result.negativecontrols=@($negativeControls.ToArray())
    $result.positive_cases=$positiveCases.Count
    $result.negative_control_cases=$negativeControls.Count
    $result.native_actions=$records.Count
    $result.independent_step_snapshots=$records.Count
    $result.assertions=$checks.Count
    $result.failed_assertions=@($checks | Where-Object {-not $_.passed})
    $result.negative_actual_failed=@($negativeControls | Where-Object {$_.actual_restoration_status -eq 'failed'}).Count
    $result.negative_actual_passed=@($negativeControls | Where-Object {$_.actual_restoration_status -eq 'passed'}).Count
    $result.cleanup_passed=($positiveCases.Count -eq 40 -and @($positiveCases | Where-Object {-not $_.ownership_root_removed}).Count -eq 0)
    $result.guarantee_boundary='通过范围仅为未来新增产品使用重发安装/卸载 helper；未修改的 R4 未来安装器仍保留失败对照。串行执行，无后台协调器；最后动作后仅 .NET 读回。'
    $result.finished_utc=[DateTime]::UtcNow.ToString('o')
    $result.status='failed'
    if (-not $result.execution_error -and $result.failed_assertions.Count -eq 0 -and $result.execution_context_hkcu_environment_path_unchanged -and $positiveCases.Count -eq 40 -and $negativeControls.Count -eq 2 -and $result.cleanup_passed) { $result.status='passed' }
    $bound=@()
    foreach ($file in @((Get-ChildItem -LiteralPath $sourceDirectory -File | ForEach-Object {$_.FullName}),$CompilerPath,(Join-Path $OutputDirectory 'environment-action-new.exe'),(Join-Path $OutputDirectory 'environment-action-old.exe'))) {
        foreach ($item in $file) {
            if (Test-Path -LiteralPath $item -PathType Leaf) {
                $path=$item; if ($item.StartsWith($OutputDirectory+'\',[StringComparison]::OrdinalIgnoreCase)) { $path=$item.Substring($OutputDirectory.Length+1).Replace('\','/') }
                $bound += [pscustomobject]@{path=$path;sha256=(Get-Hash $item);size=(Get-Item -LiteralPath $item).Length}
            }
        }
    }
    Save-Json 'source-binding.json' ([pscustomobject]@{test_id=$testId;files=$bound;frozen_R4_sha256=$legacyHash})
    Save-Json 'assertions.json' $checks.ToArray()
    $result.source_binding_file='source-binding.json'
    $result.source_binding_sha256=Get-Hash (Join-Path $OutputDirectory 'source-binding.json')
    Save-Json 'result.json' $result
    $manifest=@(Get-ChildItem -LiteralPath $OutputDirectory -File -Recurse | Sort-Object FullName | ForEach-Object {
        [pscustomobject]@{path=$_.FullName.Substring($OutputDirectory.Length+1).Replace('\','/');sha256=(Get-Hash $_.FullName);size=$_.Length}
    })
    Save-Json 'evidence-manifest.json' ([pscustomobject]@{test_id=$testId;files=$manifest;note='清单不含自身；保留原旧版新增产品导致的失败对照。'})
}
[pscustomobject]@{status=$result.status;positive_cases=$result.positive_cases;negative_controls=$result.negative_control_cases;native_actions=$result.native_actions;assertions=$result.assertions;failed_assertions=$result.failed_assertions.Count;negative_actual_failed=$result.negative_actual_failed;execution_error=$result.execution_error;result_file=(Join-Path $OutputDirectory 'result.json')} | ConvertTo-Json -Depth 4
if ($result.status -ne 'passed') { exit 1 }
