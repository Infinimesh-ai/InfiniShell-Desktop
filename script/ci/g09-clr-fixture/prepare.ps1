param([Parameter(Mandatory = $true)][string]$Reader, [switch]$Local, [string]$LocalRoot, [string]$LocalRunId)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$contextScript = Join-Path $PSScriptRoot 'preparation-context.ps1'
$contextScriptHash = (Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant()
. $contextScript
if ((Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $contextScriptHash) {
    throw '受控准备入口在加载期间发生变化'
}
$context = New-G09PreparationContext -Local $Local -LocalRoot $LocalRoot -LocalRunId $LocalRunId -Kind 'fixture'
$root = $context.Root
Write-G09PreparationOutput $context 'INFINISHELL_CLR_FIXTURE_ROOT' $root

# 只编译固定 C#，不启动它，也不在准备阶段加载 DAC。
$source = Join-Path $PSScriptRoot 'Fixture.cs'
$compiler = Join-Path $env:SystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$copiedReader = Join-Path $root 'reader.exe'
$copiedSource = Join-Path $root 'Fixture.cs'
$fixture = Join-Path $root 'fixture.exe'
$fixtureNode = Join-Path $root 'fixture-node.exe'
$sourceRecheckFailed = $false
$receipt = [ordered]@{
    schema = 1; source_commit = $context.SourceIdentity.commit; status = 'preparing'
    preparation_mode = $context.Mode; source_identity = $context.SourceIdentity
    source_identity_accepted = $false; source_identity_unchanged = $false; sources_unchanged = $false
    context_script_sha256 = $contextScriptHash; context_script_unchanged = $false
    run_id = $context.RunId; run_attempt = $context.RunAttempt
    native_execution_started = $false; dac_loaded = $false; cleanup_ready = $false
    scope = '固定 Framework 异常及本地映像分类读取夹具；不代表 PowerShell 或 G09 通过'
}
try {
    Assert-G09SourceIdentity $Local $context.SourceIdentity $env:GITHUB_SHA
    $receipt.source_identity_accepted = $true
    $inputReader = (Resolve-Path -LiteralPath $Reader).Path
    foreach ($path in @($source, $compiler, $inputReader)) {
        $item = Get-Item -LiteralPath $path
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw '固定输入必须是普通文件'
        }
    }
    $readerHash = (Get-FileHash -LiteralPath $inputReader -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Local) {
        # 本地只接受本轮受控构建的 reader；不能借本地开关导入任意可执行文件。
        $buildRoot = Join-Path $LocalRoot ('g09-clr-build-local-' + $LocalRunId)
        $buildRoot = Assert-G09LocalDirectory $buildRoot
        if (-not [string]::Equals($inputReader, (Join-Path $buildRoot 'reader.exe'), [StringComparison]::OrdinalIgnoreCase)) {
            throw '本地 reader 必须来自同一轮受控构建目录'
        }
        $buildReceiptPath = Join-Path $buildRoot 'build.safe.json'
        $buildReceiptFile = Get-Item -LiteralPath $buildReceiptPath
        if ($buildReceiptFile.PSIsContainer -or ($buildReceiptFile.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $buildReceiptFile.Length -le 0 -or $buildReceiptFile.Length -gt 1048576) {
            throw 'reader 构建收据不是固定边界内的普通文件'
        }
        $buildReceipt = [IO.File]::ReadAllText($buildReceiptPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
        if ($buildReceipt.schema -ne 1 -or $buildReceipt.status -cne 'compiled' -or
            $buildReceipt.preparation_mode -cne 'local' -or $buildReceipt.run_id -cne $LocalRunId -or
            $buildReceipt.source_commit -cne $context.SourceIdentity.commit -or -not $buildReceipt.sources_unchanged -or
            -not $buildReceipt.context_script_unchanged -or
            $buildReceipt.native_execution_started -or $buildReceipt.dac_loaded -or $buildReceipt.cleanup_ready -or
            $buildReceipt.executable.sha256 -cne $readerHash -or @($buildReceipt.inputs).Count -ne 9) {
            throw 'reader 构建收据与本轮源码或可执行文件不匹配'
        }
        Assert-G09SourceUnchanged $buildReceipt.source_identity $context.SourceIdentity
        foreach ($name in @('reader.cpp', 'wire.h', 'sos_layout.h', 'README.md', 'sources.safe.json',
                            'vendor/clrdata.h', 'vendor/xclrdata.h', 'vendor/sospriv.h', 'vendor/LICENSE.TXT')) {
            $entries = @($buildReceipt.inputs | Where-Object { $_.path -ceq $name })
            if ($entries.Count -ne 1 -or -not $entries[0].unchanged) {
                throw 'reader 构建输入不属于固定来源'
            }
            $currentSource = Join-Path (Join-Path $PSScriptRoot '..\g09-clr-reader') $name
            if ((Get-FileHash -LiteralPath $currentSource -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entries[0].sha256_before) {
                throw 'reader 构建后源码已变化'
            }
        }
        $receipt.reader_build_receipt_sha256 = (Get-FileHash -LiteralPath $buildReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if (-not $Local) {
        $buildReceiptPath = Join-Path ([IO.Path]::GetDirectoryName($inputReader)) 'build.safe.json'
        $buildReceiptFile = Get-Item -LiteralPath $buildReceiptPath
        if ($buildReceiptFile.PSIsContainer -or ($buildReceiptFile.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $buildReceiptFile.Length -le 0 -or $buildReceiptFile.Length -gt 1048576) {
            throw 'reader 元数据合同收据不是有界普通文件'
        }
        $buildReceipt = [IO.File]::ReadAllText($buildReceiptPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
        $receipt.reader_build_receipt_sha256 = (Get-FileHash -LiteralPath $buildReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($buildReceipt.status -cne 'compiled' -or $buildReceipt.executable.sha256 -cne $readerHash -or
        $buildReceipt.process_start_info_contract.getter_invoked -or
        $buildReceipt.process_start_info_contract.target_value_observed -or
        $buildReceipt.process_start_info_contract.field_name -cne 'useShellExecute' -or
        $buildReceipt.process_start_info_contract.field_type -cne 'System.Boolean') {
        throw '原 reader 的 ProcessStartInfo 元数据合同不匹配'
    }
    $receipt.process_start_info_contract = $buildReceipt.process_start_info_contract
    Copy-Item -LiteralPath $inputReader -Destination $copiedReader
    if ((Get-FileHash -LiteralPath $copiedReader -Algorithm SHA256).Hash.ToLowerInvariant() -ne $readerHash) {
        throw 'reader 副本摘要不匹配'
    }
    $receipt.compiler_sha256 = (Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash.ToLowerInvariant()
    $receipt.compiler_version = (Get-Item -LiteralPath $compiler).VersionInfo.FileVersion
    $receipt.source_sha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $source -Destination $copiedSource
    if ((Get-FileHash -LiteralPath $copiedSource -Algorithm SHA256).Hash.ToLowerInvariant() -cne $receipt.source_sha256) {
        throw '固定夹具源码副本摘要不匹配'
    }
    $receipt.preparation_script_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $root 'prepare.ps1')
    Copy-Item -LiteralPath $contextScript -Destination (Join-Path $root 'preparation-context.ps1')
    if ((Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $contextScriptHash -or
        (Get-FileHash -LiteralPath (Join-Path $root 'preparation-context.ps1') -Algorithm SHA256).Hash.ToLowerInvariant() -cne $contextScriptHash) {
        throw '受控准备入口的源码或归档摘要不匹配'
    }
    if ($Local -and $buildReceipt.context_script_sha256 -cne $receipt.context_script_sha256) {
        throw 'reader 构建后受控准备入口已变化'
    }
    & $compiler /nologo /target:exe /platform:x64 /optimize- /debug- /warnaserror+ "/out:$fixture" $copiedSource *> (Join-Path $root 'compiler.log')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $fixture)) {
        throw '固定 Framework 夹具编译失败'
    }
    $receipt.fixture_sha256 = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $fixture -Destination $fixtureNode
    $receipt.fixture_node_sha256 = (Get-FileHash -LiteralPath $fixtureNode -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($receipt.fixture_node_sha256 -ne $receipt.fixture_sha256) {
        throw '固定无害子模式副本摘要不匹配'
    }
    # 只读取刚编译原件的元数据；不调用 Main、类型初始化器或异常方法。
    $null = [Reflection.Assembly]::ReflectionOnlyLoadFrom([Diagnostics.ProcessStartInfo].Assembly.Location)
    $assembly = [Reflection.Assembly]::ReflectionOnlyLoadFrom($fixture)
    $receipt.fixture_mvid = $assembly.ManifestModule.ModuleVersionId.ToString('D')
    $type = $assembly.GetType('ClrFixture', $true)
    $flags = [Reflection.BindingFlags]'Static, NonPublic'
    $shellMethod = $type.GetMethod('ClassifyBoundNode', $flags)
    if ($null -eq $shellMethod) { throw '固定映像分类方法元数据缺失' }
    $receipt.fixture_shell_method_token = $shellMethod.MetadataToken
    $methodFlags = [int]$shellMethod.GetMethodImplementationFlags()
    if (($methodFlags -band 72) -ne 72) { throw '固定方法缺少 NoInlining 与 NoOptimization 标志' }
    $receipt.fixture_shell_method_implementation_flags = $methodFlags
    # 仅解码刚编译的方法体，按唯一边界调用确定实际局部索引；不搜索字节子串或执行方法。
    $boundary = $type.GetMethod('ContinuationBoundary', $flags)
    if ($null -eq $boundary) { throw '固定续点边界方法缺失' }
    $body = $shellMethod.GetMethodBody()
    $il = $body.GetILAsByteArray()
    $opcodes = @{}
    foreach ($opcodeField in [Reflection.Emit.OpCodes].GetFields([Reflection.BindingFlags]'Public, Static')) {
        $opcode = $opcodeField.GetValue($null)
        $opcodes[[int]$opcode.Value -band 65535] = $opcode
    }
    $instructions = @()
    for ($offset = 0; $offset -lt $il.Length;) {
        $startOffset = $offset
        $code = [int]$il[$offset++]
        if ($code -eq 254) {
            if ($offset -ge $il.Length) { throw '固定 IL 操作码被截断' }
            $code = 0xfe00 -bor [int]$il[$offset++]
        }
        if (-not $opcodes.ContainsKey($code)) { throw '固定 IL 操作码未知' }
        $opcode = $opcodes[$code]
        $size = switch ($opcode.OperandType.ToString()) {
            'InlineNone' { 0 }
            { $_ -in @('ShortInlineBrTarget', 'ShortInlineI', 'ShortInlineVar') } { 1 }
            'InlineVar' { 2 }
            { $_ -in @('InlineBrTarget', 'InlineField', 'InlineI', 'InlineMethod', 'InlineSig', 'InlineString', 'InlineTok', 'InlineType', 'ShortInlineR') } { 4 }
            { $_ -in @('InlineI8', 'InlineR') } { 8 }
            default { throw '固定 IL 含未批准的操作数形式' }
        }
        if ($offset + $size -gt $il.Length) { throw '固定 IL 操作数被截断' }
        $operand = if ($size -eq 1) { [int]$il[$offset] } elseif ($size -eq 2) {
            [BitConverter]::ToUInt16($il, $offset)
        } elseif ($size -eq 4) { [BitConverter]::ToInt32($il, $offset) } else { $null }
        $instructions += [pscustomobject]@{ offset = $startOffset; opcode = $opcode.Name; operand = $operand }
        $offset += $size
    }
    $boundaryCalls = @(for ($i = 2; $i -lt $instructions.Count; $i++) {
        if ($instructions[$i].opcode -ceq 'call' -and $instructions[$i].operand -eq $boundary.MetadataToken) { $i }
    })
    if ($boundaryCalls.Count -ne 1) { throw '固定续点调用不唯一' }
    $callIndex = $boundaryCalls[0]
    $booleanLoad = $instructions[$callIndex - 2]
    $startInfoLoad = $instructions[$callIndex - 1]
    $booleanIndex = if ($booleanLoad.opcode -match '^ldloc\.([0-3])$') { [int]$Matches[1] }
        elseif ($booleanLoad.opcode -in @('ldloc', 'ldloc.s')) { [int]$booleanLoad.operand }
        else { throw '固定布尔参数不是按值读取的局部变量' }
    $startInfoIndex = if ($startInfoLoad.opcode -match '^ldloc\.([0-3])$') { [int]$Matches[1] }
        elseif ($startInfoLoad.opcode -in @('ldloc', 'ldloc.s')) { [int]$startInfoLoad.operand }
        else { throw '固定 ProcessStartInfo 不是局部变量' }
    if ($booleanIndex -ge $body.LocalVariables.Count -or $startInfoIndex -ge $body.LocalVariables.Count -or
        $body.LocalVariables[$booleanIndex].LocalType.FullName -cne 'System.Boolean' -or
        $body.LocalVariables[$startInfoIndex].LocalType.FullName -cne 'System.Diagnostics.ProcessStartInfo') {
        throw '固定续点局部变量类型不符'
    }
    $decisionIndex = $callIndex + 1
    while ($decisionIndex -lt $instructions.Count -and $instructions[$decisionIndex].opcode -ceq 'nop') { $decisionIndex++ }
    if ($decisionIndex -ge $instructions.Count) { throw '固定边界调用后缺少布尔读取' }
    $decisionLoad = $instructions[$decisionIndex]
    $decisionLocal = if ($decisionLoad.opcode -match '^ldloc\.([0-3])$') { [int]$Matches[1] }
        elseif ($decisionLoad.opcode -in @('ldloc', 'ldloc.s')) { [int]$decisionLoad.operand }
        else { throw '固定边界调用后不是布尔局部读取' }
    if ($decisionLocal -ne $booleanIndex) { throw '固定调用前后布尔局部不同' }
    $receipt.fixture_continuation = [ordered]@{
        method_token = $shellMethod.MetadataToken; boundary_method_token = $boundary.MetadataToken
        approved_il_offsets = @($decisionLoad.offset)
        boundary_bool_load_offset = $booleanLoad.offset; boundary_call_offset = $instructions[$callIndex].offset
        bool_local_index = $booleanIndex; start_info_local_index = $startInfoIndex
        il_base64 = [Convert]::ToBase64String($il)
        expected_boolean = $true; expected_use_shell_execute = $false
    }
    $receipt.fixture_method_tokens = @('ThrowNative', 'ThrowSecondNative', 'WrapNative', 'WrapOperation', 'Exercise') | ForEach-Object {
        $method = $type.GetMethod($_, $flags)
        if ($null -eq $method) { throw '固定夹具方法元数据缺失' }
        $method.MetadataToken
    }
    $receipt.reader_sha256 = $readerHash
    $receipt.context_script_sha256_after = (Get-FileHash -LiteralPath $contextScript -Algorithm SHA256).Hash.ToLowerInvariant()
    $receipt.archived_context_script_sha256 = (Get-FileHash -LiteralPath (Join-Path $root 'preparation-context.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()
    $receipt.context_script_unchanged = $receipt.context_script_sha256_after -ceq $contextScriptHash -and
        $receipt.archived_context_script_sha256 -ceq $contextScriptHash
    $receipt.sources_unchanged = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $receipt.source_sha256 -and
        (Get-FileHash -LiteralPath $copiedSource -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $receipt.source_sha256 -and
        (Get-FileHash -LiteralPath $inputReader -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $readerHash -and
        (Get-FileHash -LiteralPath $copiedReader -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $readerHash -and
        $receipt.context_script_unchanged
    if (-not $receipt.sources_unchanged) { throw '固定准备输入在编译期间发生变化' }
    $receipt.status = 'prepared'
} catch {
    # 不保存异常消息，路径及环境内容不进入安全收据。
    $receipt.status = 'failed'
    $receipt.error_kind = $_.Exception.GetType().FullName
    $receipt.error_hresult = $_.Exception.HResult
    throw
} finally {
    try {
        $receipt.source_identity_after = Get-G09SourceIdentity
        Assert-G09SourceUnchanged $context.SourceIdentity $receipt.source_identity_after
        Assert-G09SourceIdentity $Local $receipt.source_identity_after $env:GITHUB_SHA
        $receipt.source_identity_unchanged = $true
    } catch {
        $sourceRecheckFailed = $true
        $receipt.status = 'failed'
        $receipt.source_recheck_error_kind = $_.Exception.GetType().FullName
        $receipt.source_recheck_error_hresult = $_.Exception.HResult
    }
    $receipt.sources_unchanged = $receipt.sources_unchanged -and $receipt.source_identity_accepted -and
        $receipt.source_identity_unchanged
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 8))
    $file = [IO.File]::Open((Join-Path $root 'preparation.safe.json'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush() } finally { $file.Dispose() }
}
if ($sourceRecheckFailed) { throw '固定准备来源的提交或原始字节发生变化' }
