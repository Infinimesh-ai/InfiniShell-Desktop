# 只读取当前系统 PowerShell / Framework 的元数据；不创建或调用候选对象。
# 无路径或目标参数；测试只提取函数定义，不执行本文末尾的收集入口。
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-ContractLimits {
    @{ file_bytes = 67108864L; method_bytes = 65536; total_il_bytes = 262144
       methods = 128; inventory = 256; instructions = 16384; receipt_bytes = 16777216 }
}

function Get-ContractHash([byte[]] $Bytes) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($Bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose() }
}

function Get-ContractStreamHash([IO.Stream] $Stream) {
    $Stream.Position = 0
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($Stream)).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose() }
}

function Get-ContractFailure($Exception) {
    # 不读取 Message / StackTrace / Data，避免失败记录带入路径或运行时内容。
    @{ kind = 'collection_error'; exception_type = $Exception.GetType().FullName; hresult = $Exception.HResult }
}

function Get-ContractOpcodes {
    $result = @{}
    foreach ($field in [Reflection.Emit.OpCodes].GetFields([Reflection.BindingFlags]'Public,Static')) {
        # 仅读 CLR 的 opcode 常量，不读取被审类型的静态字段值。
        $opcode = $field.GetValue($null)
        $key = [int]$opcode.Value -band 65535
        if ($result.ContainsKey($key)) { throw [IO.InvalidDataException]::new('duplicate_opcode') }
        $result[$key] = $opcode
    }
    return $result
}

function Get-ContractMember($Member) {
    $declaring = $null
    if ($null -ne $Member.DeclaringType) { $declaring = $Member.DeclaringType.FullName }
    @{ name = $Member.Name; declaring_type = $declaring; signature = $Member.ToString()
       member_type = $Member.MemberType.ToString(); token = $Member.MetadataToken
       module_mvid = $Member.Module.ModuleVersionId.ToString()
       assembly = $Member.Module.Assembly.FullName }
}

function Resolve-ContractObservedMember([Reflection.Module] $Module, [string] $ExpectedMvid, [int] $Token) {
    # token 只能在原收据的模块内解释，系统组件换版时不能套用旧编号。
    if ($Module.ModuleVersionId.ToString() -cne $ExpectedMvid) { throw [IO.InvalidDataException]::new('observed_module_mismatch') }
    if (($Token -band -16777216) -notin @(33554432, 100663296)) { throw [IO.InvalidDataException]::new('observed_token_kind') }
    return $Module.ResolveMember($Token)
}

function Read-ContractIL([byte[]] $Bytes, [Reflection.MethodBase] $Method, $Opcodes, $Limits) {
    if ($Bytes.Length -gt $Limits.method_bytes) { throw [IO.InvalidDataException]::new('method_il_limit') }
    $rows = [Collections.Generic.List[object]]::new()
    $offset = 0
    while ($offset -lt $Bytes.Length) {
        if ($rows.Count -ge $Limits.instructions) { throw [IO.InvalidDataException]::new('instruction_limit') }
        $start = $offset
        $code = [int]$Bytes[$offset]; $offset++
        if ($code -eq 254) {
            if ($offset -ge $Bytes.Length) { throw [IO.InvalidDataException]::new('truncated_opcode') }
            $code = 65024 -bor [int]$Bytes[$offset]; $offset++
        }
        if (-not $Opcodes.ContainsKey($code)) { throw [IO.InvalidDataException]::new('unknown_opcode') }
        $opcode = $Opcodes[$code]
        $kind = $opcode.OperandType.ToString()
        $width = switch ($kind) {
            'InlineNone' { 0 }
            { $_ -in @('ShortInlineBrTarget', 'ShortInlineI', 'ShortInlineVar') } { 1 }
            'InlineVar' { 2 }
            { $_ -in @('InlineBrTarget', 'InlineField', 'InlineI', 'InlineMethod', 'InlineSig', 'InlineString', 'InlineSwitch', 'InlineTok', 'InlineType', 'ShortInlineR') } { 4 }
            { $_ -in @('InlineI8', 'InlineR') } { 8 }
            default { throw [IO.InvalidDataException]::new('unsupported_operand') }
        }
        if ($width -gt $Bytes.Length - $offset) { throw [IO.InvalidDataException]::new('truncated_operand') }
        $operandStart = $offset
        $operand = $null; $targets = @(); $token = $null; $resolved = $null; $resolution = 'not_applicable'; $failure = $null
        switch ($kind) {
            'InlineSwitch' {
                $count = [BitConverter]::ToInt32($Bytes, $offset)
                $offset += 4
                if ($count -lt 0 -or $count -gt (($Bytes.Length - $offset) / 4)) { throw [IO.InvalidDataException]::new('truncated_switch') }
                $end = $offset + 4 * $count
                $targets = @(for ($index = 0; $index -lt $count; $index++) {
                    [long]$end + [BitConverter]::ToInt32($Bytes, $offset + 4 * $index)
                })
                $operand = $count
                $offset = $end
            }
            'ShortInlineBrTarget' {
                $delta = [int]$Bytes[$offset]
                if ($delta -ge 128) { $delta -= 256 }
                $offset++
                $operand = $delta; $targets = @([long]$offset + $delta)
            }
            'InlineBrTarget' {
                $operand = [BitConverter]::ToInt32($Bytes, $offset)
                $offset += 4; $targets = @([long]$offset + $operand)
            }
            'ShortInlineI' {
                $operand = [int]$Bytes[$offset]
                if ($operand -ge 128) { $operand -= 256 }
                $offset++
            }
            'ShortInlineVar' { $operand = [int]$Bytes[$offset]; $offset++ }
            'InlineVar' { $operand = [BitConverter]::ToUInt16($Bytes, $offset); $offset += 2 }
            'InlineI' { $operand = [BitConverter]::ToInt32($Bytes, $offset); $offset += 4 }
            'InlineI8' { $operand = [BitConverter]::ToInt64($Bytes, $offset).ToString(); $offset += 8 }
            'ShortInlineR' { $operand = [BitConverter]::ToSingle($Bytes, $offset).ToString('R', [Globalization.CultureInfo]::InvariantCulture); $offset += 4 }
            'InlineR' { $operand = [BitConverter]::ToDouble($Bytes, $offset).ToString('R', [Globalization.CultureInfo]::InvariantCulture); $offset += 8 }
            'InlineNone' {}
            default {
                $token = [BitConverter]::ToInt32($Bytes, $offset); $offset += 4
                $operand = $token
                if ($kind -eq 'InlineString') {
                    # 字符串内容不是本次分支合同的取证目标；保留原 token 及原 IL。
                    $resolution = 'string_content_not_collected'
                } else {
                    try {
                        if ($kind -eq 'InlineSig') {
                            $signature = $Method.Module.ResolveSignature($token)
                            if ($signature.Length -gt 4096) { throw [IO.InvalidDataException]::new('signature_limit') }
                            $resolved = @{ signature_base64 = [Convert]::ToBase64String($signature) }
                        } else {
                            $typeArguments = [Type[]]@()
                            if ($null -ne $Method.DeclaringType) { $typeArguments = $Method.DeclaringType.GetGenericArguments() }
                            $methodArguments = [Type[]]@()
                            if ($Method -is [Reflection.MethodInfo]) { $methodArguments = $Method.GetGenericArguments() }
                            $resolved = Get-ContractMember ($Method.Module.ResolveMember($token, $typeArguments, $methodArguments))
                        }
                        $resolution = 'resolved'
                    } catch {
                        $resolution = 'unresolved'; $failure = Get-ContractFailure $_.Exception
                    }
                }
            }
        }
        $raw = [byte[]]@()
        if ($offset -gt $operandStart) { $raw = $Bytes[$operandStart..($offset - 1)] }
        $rows.Add(@{ offset = $start; size = $offset - $start; opcode = $opcode.Name
            operand_kind = $kind; operand = $operand; operand_base64 = [Convert]::ToBase64String($raw)
            branch_targets = $targets; metadata_token = $token; resolution = $resolution
            member = $resolved; failure = $failure })
    }
    $boundaries = @{}
    foreach ($row in $rows) { $boundaries[[long]$row.offset] = $true }
    foreach ($row in $rows) {
        foreach ($target in $row.branch_targets) {
            if (-not $boundaries.ContainsKey([long]$target)) { throw [IO.InvalidDataException]::new('branch_target_not_instruction') }
        }
    }
    return $rows.ToArray()
}

function Get-ContractMethod([Reflection.MethodBase] $Method, $Opcodes, $Limits, $Budget) {
    $Budget.methods++
    if ($Budget.methods -gt $Limits.methods) { throw [IO.InvalidDataException]::new('method_count_limit') }
    $result = Get-ContractMember $Method
    $result.status = 'no_body'; $result.failure = $null
    $body = $Method.GetMethodBody()
    if ($null -eq $body) { return $result }
    $bytes = $body.GetILAsByteArray()
    # 失败方法同样消耗总预算，不允许通过略过坏方法重新获得读取额度。
    $Budget.il_bytes += $bytes.Length
    if ($bytes.Length -gt $Limits.method_bytes -or $Budget.il_bytes -gt $Limits.total_il_bytes) { throw [IO.InvalidDataException]::new('total_il_limit') }
    $result.il_bytes = $bytes.Length
    $result.il_sha256 = Get-ContractHash $bytes
    $result.il_base64 = [Convert]::ToBase64String($bytes)
    $result.max_stack = $body.MaxStackSize; $result.init_locals = $body.InitLocals
    $result.local_signature_token = $body.LocalSignatureMetadataToken
    $result.locals = @($body.LocalVariables | ForEach-Object {
        @{ index = $_.LocalIndex; type = $_.LocalType.ToString(); pinned = $_.IsPinned }
    })
    $result.instructions = @(Read-ContractIL $bytes $Method $Opcodes $Limits)
    $boundaries = @{}; $boundaries[[long]$bytes.Length] = $true
    foreach ($instruction in $result.instructions) { $boundaries[[long]$instruction.offset] = $true }
    $result.exception_regions = @($body.ExceptionHandlingClauses | ForEach-Object {
        $clause = $_
        foreach ($boundary in @($clause.TryOffset, ($clause.TryOffset + $clause.TryLength), $clause.HandlerOffset, ($clause.HandlerOffset + $clause.HandlerLength))) {
            if (-not $boundaries.ContainsKey([long]$boundary)) { throw [IO.InvalidDataException]::new('exception_region_bounds') }
        }
        $catch = $null; $filter = $null
        if ($clause.Flags -eq [Reflection.ExceptionHandlingClauseOptions]::Clause) { $catch = $clause.CatchType.FullName }
        if ($clause.Flags -eq [Reflection.ExceptionHandlingClauseOptions]::Filter) {
            $filter = $clause.FilterOffset
            if (-not $boundaries.ContainsKey([long]$filter) -or $filter -ge $bytes.Length) { throw [IO.InvalidDataException]::new('exception_filter_bounds') }
        }
        @{ kind = $clause.Flags.ToString(); try_offset = $clause.TryOffset; try_length = $clause.TryLength
           handler_offset = $clause.HandlerOffset; handler_length = $clause.HandlerLength; catch_type = $catch; filter_offset = $filter }
    })
    $result.status = 'decoded'
    if (@($result.instructions | Where-Object { $_.resolution -eq 'unresolved' }).Count -ne 0) { $result.status = 'unresolved_metadata' }
    return $result
}

function Assert-ContractDirectory([string] $Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if ($full -notmatch '^[A-Za-z]:\\' -or $full.IndexOf(':', 2) -ge 0) { throw [IO.InvalidDataException]::new('local_path_required') }
    $cursor = [IO.DirectoryInfo]::new($full)
    while ($null -ne $cursor) {
        if (-not $cursor.Exists -or ($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw [IO.InvalidDataException]::new('directory_identity_rejected') }
        $cursor = $cursor.Parent
    }
    return $full
}

function Open-ContractFile([string] $Path, $Limits) {
    $full = [IO.Path]::GetFullPath($Path)
    [void](Assert-ContractDirectory ([IO.Path]::GetDirectoryName($full)))
    if (([IO.File]::GetAttributes($full) -band ([IO.FileAttributes]::ReparsePoint -bor [IO.FileAttributes]::Directory)) -ne 0) { throw [IO.InvalidDataException]::new('file_kind_rejected') }
    # 原文件保持拒写/拒删除至反射完成及第二次摘要结束；不复制或执行该文件。
    $stream = [IO.FileStream]::new($full, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($stream.Length -le 0 -or $stream.Length -gt $Limits.file_bytes) { throw [IO.InvalidDataException]::new('file_size_limit') }
        return @{ stream = $stream; record = @{ location = $full; bytes = $stream.Length
            sha256_before = (Get-ContractStreamHash $stream); sha256_after = $null
            file_version = [Diagnostics.FileVersionInfo]::GetVersionInfo($full).FileVersion } }
    } catch { $stream.Dispose(); throw }
}

function Save-ContractReceipt([string] $Path, $Report, $Limits) {
    $text = $Report | ConvertTo-Json -Depth 32 -Compress
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text + "`n")
    if ($bytes.Length -gt $Limits.receipt_bytes) { throw [IO.InvalidDataException]::new('receipt_size_limit') }
    $file = [IO.FileStream]::new($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush($true) }
    finally { $file.Dispose() }
}

function Assert-ContractX64Image([IO.Stream] $Stream) {
    $reader = [IO.BinaryReader]::new($Stream, [Text.Encoding]::UTF8, $true)
    try {
        $Stream.Position = 0
        if ($Stream.Length -lt 64 -or $reader.ReadUInt16() -ne 23117) { throw [IO.InvalidDataException]::new('host_dos_header') }
        $Stream.Position = 60
        $pe = $reader.ReadInt32()
        if ($pe -lt 64 -or $pe -gt $Stream.Length - 26) { throw [IO.InvalidDataException]::new('host_pe_bounds') }
        $Stream.Position = $pe
        if ($reader.ReadUInt32() -ne 17744 -or $reader.ReadUInt16() -ne 34404) { throw [IO.InvalidDataException]::new('host_x64_required') }
        $Stream.Position = $pe + 24
        if ($reader.ReadUInt16() -ne 523) { throw [IO.InvalidDataException]::new('host_pe32plus_required') }
    } finally { $reader.Dispose() }
}

function Invoke-PowerShellContractCollection {
    if ($args.Count -ne 0) { throw [IO.InvalidDataException]::new('arguments_forbidden') }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or -not [Environment]::Is64BitProcess -or
        $PSVersionTable.PSEdition -cne 'Desktop' -or $PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) {
        throw [IO.InvalidDataException]::new('windows_powershell_51_x64_required')
    }
    if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or $env:RUNNER_ARCH -cne 'X64' -or
        $env:GITHUB_REPOSITORY -cne 'Infinimesh-ai/InfiniShell-Desktop' -or $env:GITHUB_WORKFLOW -cne 'Cross-platform preflight' -or
        $env:GITHUB_EVENT_NAME -cne 'workflow_dispatch' -or $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$' -or
        $env:GITHUB_RUN_ID -cnotmatch '^[1-9][0-9]{0,19}$' -or $env:GITHUB_RUN_ATTEMPT -cnotmatch '^[1-9][0-9]{0,5}$' -or
        $env:GITHUB_REF -cnotmatch '^refs/heads/[A-Za-z0-9._/-]+$') { throw [IO.InvalidDataException]::new('workflow_identity_required') }
    $limits = Get-ContractLimits
    $report = @{ schema = 'g09-powershell-contract-v1'; source_commit = $env:GITHUB_SHA; run_id = $env:GITHUB_RUN_ID
        run_attempt = $env:GITHUB_RUN_ATTEMPT; ref = $env:GITHUB_REF; workflow = $env:GITHUB_WORKFLOW; event = $env:GITHUB_EVENT_NAME
        powershell_version = $PSVersionTable.PSVersion.ToString(); clr_version = [Environment]::Version.ToString()
        scope = 'static_loaded_module_il_not_restricted_execution_or_g09_closure'; target_invoked = $false
        passed = $false; status = 'failed'; stage = 'output_directory'; failure = $null; files = @(); assemblies = @(); ncp_inventory = @(); methods = @()
        unresolved = @(); observed_types = @(); limits = $limits; handles_released = $false }
    $output = $null; $receipt = $null
    $held = [Collections.Generic.List[object]]::new()
    try {
        $runner = Assert-ContractDirectory $env:RUNNER_TEMP
        $output = Join-Path $runner "g09-powershell-contract-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
        if ([IO.Directory]::Exists($output) -or [IO.File]::Exists($output)) { throw [IO.IOException]::new('output_exists') }
        [void][IO.Directory]::CreateDirectory($output)
        [void](Assert-ContractDirectory $output)
        $receipt = Join-Path $output 'contract.safe.json'
        $report.stage = 'loaded_files'
        $current = [Diagnostics.Process]::GetCurrentProcess()
        try { $executable = $current.MainModule.FileName } finally { $current.Dispose() }
        $expected = Join-Path ([Environment]::SystemDirectory) 'WindowsPowerShell\v1.0\powershell.exe'
        if (-not [string]::Equals($executable, $expected, [StringComparison]::OrdinalIgnoreCase)) { throw [IO.InvalidDataException]::new('system_powershell_required') }
        $hostFile = Open-ContractFile $executable $limits
        $hostFile.record.role = 'powershell_executable'; $held.Add($hostFile); $report.files += $hostFile.record
        Assert-ContractX64Image $hostFile.stream
        # 实际加载对象决定位置，不从文件名猜测 GAC 或 native image 路径。
        $ncp = [System.Management.Automation.PowerShell].Assembly.GetType('System.Management.Automation.NativeCommandProcessor', $true)
        $types = @($ncp, [Diagnostics.Process], [Console])
        $modules = @{}
        foreach ($type in $types) {
            $assembly = $type.Assembly
            $id = $assembly.ManifestModule.ModuleVersionId.ToString()
            if ($modules.ContainsKey($id)) { continue }
            if ($assembly.IsDynamic -or $assembly.ReflectionOnly -or [string]::IsNullOrEmpty($assembly.Location)) { throw [IO.InvalidDataException]::new('loaded_assembly_required') }
            $file = Open-ContractFile $assembly.Location $limits
            $file.record.role = 'managed_assembly'; $held.Add($file); $report.files += $file.record
            $entry = @{ name = $assembly.FullName; location = $assembly.Location; mvid = $id
                image_runtime_version = $assembly.ImageRuntimeVersion; file = $file.record }
            $report.assemblies += $entry; $modules[$id] = $true
        }
        $report.stage = 'method_inventory'
        $flags = [Reflection.BindingFlags]'Public,NonPublic,Instance,Static,DeclaredOnly'
        $declared = @($ncp.GetMethods($flags)) + @($ncp.GetConstructors($flags))
        if ($declared.Count -gt $limits.inventory) { throw [IO.InvalidDataException]::new('inventory_limit') }
        $report.ncp_inventory = @($declared | Sort-Object MetadataToken | ForEach-Object { Get-ContractMember $_ })
        $queue = [Collections.Generic.Queue[Reflection.MethodBase]]::new()
        $selected = @{}
        # NCP 体积有界；完整声明方法避免猜测私有版本中的重命名/拆分执行入口。
        foreach ($method in $declared) { $queue.Enqueue($method) }
        $processNames = @('Start', 'StartWithCreateProcess', 'StartWithShellExecuteEx', 'WaitForExit', 'GetProcessHandle', 'CreatePipe', 'CreatePipeWithSecurityAttributes', 'SetProcessHandle', 'SetProcessId', 'ReleaseProcessHandle', 'EnsureWatchingForExit')
        $consoleNames = @('get_InputEncoding', 'set_InputEncoding', 'get_OutputEncoding', 'set_OutputEncoding', 'get_In', 'get_Out', 'get_Error', 'InitializeStdOutError', 'InitializeStdIn', 'GetStandardFile', 'OpenStandardInput', 'OpenStandardOutput', 'OpenStandardError')
        foreach ($pair in @(@{ type = [Diagnostics.Process]; names = $processNames }, @{ type = [Console]; names = $consoleNames })) {
            foreach ($method in $pair.type.GetMethods($flags)) {
                if ($method.Name -cin $pair.names) { $queue.Enqueue($method) }
            }
        }
        # 来自 a74 原失败进程的晚期异常帧；只解码元数据，不重跑候选或调用这些方法。
        $report.observed_source = @{ run_id = '37299690314'; generation = 'f961a206-028e-4f4c-a1c0-7ab642db1176'
            artifact_sha256 = '65c109949aea234133dd0dd9eb0379f7fec9cebb35810b54d7c6d1fd784c1b17'
            frame_events = @(175, 176, 178, 179, 187, 189, 191, 193, 195, 197, 199, 201, 203) }
        $observed = @(
            @{ module = $ncp.Module; mvid = '0a210000-3870-4dec-b53e-175f62acb623'
               types = @(33555000, 33555147, 33555165, 33555173, 33555450)
               methods = @(100666493, 100666540, 100666542, 100666970, 100666972, 100667326, 100667348, 100667353,
                   100668184, 100668501, 100672923, 100672926, 100672927, 100672928, 100672951, 100673698,
                   100673702, 100676943, 100677064, 100677096, 100677435, 100677592, 100680528, 100680530,
                   100680555, 100685212, 100685213, 100686754, 100686773, 100686810, 100686815, 100686829,
                   100686872, 100691784, 100695809) }
            @{ module = [Console].Module; mvid = '3020f90f-960f-4330-86d5-45c4e07afdfd'
               types = @(33554772, 33554839, 33554927); methods = @(100678670, 100678671, 100678672, 100678945) }
            @{ module = [Diagnostics.Process].Module; mvid = 'c9e847ed-2baf-4284-8d70-833560c74034'
               types = @(33555900); methods = @() }
        )
        foreach ($entry in $observed) {
            foreach ($token in $entry.types) {
                $report.observed_types += Get-ContractMember (Resolve-ContractObservedMember $entry.module $entry.mvid $token)
            }
            foreach ($token in $entry.methods) {
                $queue.Enqueue((Resolve-ContractObservedMember $entry.module $entry.mvid $token))
            }
        }
        $budget = @{ methods = 0; il_bytes = 0; method_json_bytes = 0 }
        $report.budget = $budget
        $opcodes = Get-ContractOpcodes
        $report.stage = 'method_bodies'
        while ($queue.Count -ne 0) {
            $method = $queue.Dequeue()
            $key = $method.Module.ModuleVersionId.ToString() + ':' + $method.MetadataToken
            if ($selected.ContainsKey($key)) { continue }
            $selected[$key] = $true
            $data = Get-ContractMethod $method $opcodes $limits $budget
            # 给来源及清单保留空间；超限时仍可保存此前完整方法，不能只剩写出失败。
            $budget.method_json_bytes += [Text.Encoding]::UTF8.GetByteCount(($data | ConvertTo-Json -Depth 32 -Compress))
            if ($budget.method_json_bytes -gt $limits.receipt_bytes - 1048576) { throw [IO.InvalidDataException]::new('method_receipt_budget') }
            $report.methods += $data
            if ($data.status -ne 'decoded') { $report.unresolved += @{ method = $key; status = $data.status } }
            if (-not $data.ContainsKey('instructions')) { continue }
            foreach ($instruction in $data.instructions) {
                if ($instruction.operand_kind -ne 'InlineMethod' -or $instruction.resolution -ne 'resolved') { continue }
                $member = $instruction.member
                # 仅沿同模块的固定相关类型补直接 helper，不递归整个 Framework。
                $follow = ($method.DeclaringType -eq $ncp -and $member.module_mvid -eq $ncp.Module.ModuleVersionId.ToString() -and
                    $member.name -cin @('IsWindowsApplication', 'IsConsoleApplication', 'FindExecutable', 'AllocateHiddenConsole')) -or
                    ($method.DeclaringType -eq [Console] -and $member.declaring_type -ceq 'System.Console')
                if ($follow) { $queue.Enqueue($method.Module.ResolveMethod($instruction.metadata_token)) }
            }
        }
        # 必要入口缺失不能当作无分支；完整清单仍保留供离线判断当前布局。
        $requiredMethods = @(
            @{ type = $ncp.FullName; name = 'Complete' }, @{ type = $ncp.FullName; name = 'GetProcessStartInfo' }, @{ type = $ncp.FullName; name = 'CalculateIORedirection' }
            @{ type = 'System.Diagnostics.Process'; name = 'Start' }, @{ type = 'System.Diagnostics.Process'; name = 'StartWithCreateProcess' }
            @{ type = 'System.Diagnostics.Process'; name = 'StartWithShellExecuteEx' }, @{ type = 'System.Diagnostics.Process'; name = 'WaitForExit' }
            @{ type = 'System.Diagnostics.Process'; name = 'GetProcessHandle' }, @{ type = 'System.Console'; name = 'get_InputEncoding' }
            @{ type = 'System.Console'; name = 'set_InputEncoding' }, @{ type = 'System.Console'; name = 'get_OutputEncoding' }, @{ type = 'System.Console'; name = 'set_OutputEncoding' }
        )
        foreach ($required in $requiredMethods) {
            if (@($report.methods | Where-Object { $_.declaring_type -ceq $required.type -and $_.name -ceq $required.name -and $_.status -eq 'decoded' }).Count -eq 0) {
                $report.unresolved += @{ type = $required.type; name = $required.name; status = 'required_body_missing_or_incomplete' }
            }
        }
        foreach ($name in @('IsWindowsApplication', 'IsConsoleApplication')) {
            if (@($report.methods | Where-Object { $_.name -ceq $name -and $_.status -eq 'decoded' }).Count -eq 0) {
                $report.unresolved += @{ name = $name; status = 'classification_body_missing_or_incomplete' }
            }
        }
        $report.stage = 'completed'
        $report.passed = $report.unresolved.Count -eq 0
        $report.status = $(if ($report.passed) { 'completed' } else { 'partial' })
    } catch {
        $report.failure = Get-ContractFailure $_.Exception
        if ($report.stage -eq 'method_bodies') { $report.status = 'partial' }
    } finally {
        # 解码失败也保留每个已打开原文件的第二次摘要；原错误优先，不覆盖前述失败。
        foreach ($file in $held) {
            try {
                $file.record.sha256_after = Get-ContractStreamHash $file.stream
                if ($file.stream.Length -ne $file.record.bytes -or $file.record.sha256_before -cne $file.record.sha256_after) { throw [IO.InvalidDataException]::new('source_file_changed') }
            } catch {
                $file.record.failure = Get-ContractFailure $_.Exception
                $report.passed = $false; $report.status = 'failed'
            } finally { $file.stream.Dispose() }
        }
        $report.handles_released = $true
    }
    if ($null -ne $receipt) {
        try { Save-ContractReceipt $receipt $report $limits }
        catch {
            # 主收据不能覆盖；写失败只向 CI 日志输出固定字段，保留已有文件原样。
            [Console]::Error.WriteLine((@{ schema = 'g09-powershell-contract-write-failure-v1'; failure = (Get-ContractFailure $_.Exception) } | ConvertTo-Json -Compress))
            return 1
        }
    } else {
        [Console]::Error.WriteLine((@{ schema = 'g09-powershell-contract-prepare-failure-v1'; stage = $report.stage; failure = $report.failure } | ConvertTo-Json -Compress))
        return 1
    }
    if ($report.passed) { return 0 }
    return 1
}

try {
    if ($args.Count -ne 0) { throw [IO.InvalidDataException]::new('arguments_forbidden') }
    exit (Invoke-PowerShellContractCollection)
} catch {
    # 平台/控制面尚未验证时，不能向不可信路径落盘。
    @{ schema = 'g09-powershell-contract-entry-failure-v1'; failure = (Get-ContractFailure $_.Exception) } | ConvertTo-Json -Compress
    exit 1
}
