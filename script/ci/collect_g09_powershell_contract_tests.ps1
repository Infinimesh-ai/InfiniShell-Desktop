# 只编译并反射固定 C# 夹具，不调用夹具方法，不运行 Windows 收集入口。
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($args.Count -ne 0) { throw 'arguments_forbidden' }
$source = Join-Path $PSScriptRoot 'collect_g09_powershell_contract.ps1'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw 'collector_parse_failed' }
foreach ($name in @('Get-ContractLimits', 'Get-ContractHash', 'Get-ContractStreamHash', 'Get-ContractFailure', 'Get-ContractOpcodes', 'Get-ContractMember', 'Read-ContractIL', 'Get-ContractMethod', 'Save-ContractReceipt', 'Assert-ContractX64Image')) {
    $nodes = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name }, $true))
    if ($nodes.Count -ne 1) { throw 'function_extraction_ambiguous' }
    . ([scriptblock]::Create($nodes[0].Extent.Text))
}
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
namespace G09ContractFixture {
    public static class Sample {
        public static int Branch(int value) { if (value < 0) return -value; return value + 7; }
        public static int Switch(int value) {
            switch (value) {
                case 0: return 17; case 1: return 23; case 2: return 41;
                case 3: return 53; case 4: return 67; case 5: return 79;
                default: return -1;
            }
        }
        public static int Handler(int value) {
            int result = 0;
            try { result = 100 / value; }
            catch (DivideByZeroException) { result = -7; }
            finally { result += 1; }
            return result;
        }
        public static int Generic<T>(T value) {
            List<T> items = new List<T>(); items.Add(value); return items.Count;
        }
        public static long Constants(int value) {
            string label = "fixture_only_not_collected";
            return label.Length + 1234567890123L + value;
        }
        [DllImport("not_loaded_or_invoked")]
        public static extern int NoBody();
    }
}
'@

function Assert-ContractTest([bool] $Value, [string] $Name) { if (-not $Value) { throw $Name } }
function Assert-ContractReject([scriptblock] $Action, [string] $Name) {
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    Assert-ContractTest $rejected $Name
}
$passed = [Collections.Generic.List[string]]::new()
$limits = Get-ContractLimits
$opcodes = Get-ContractOpcodes
$type = [G09ContractFixture.Sample]
$budget = @{ methods = 0; il_bytes = 0 }
$branchMethod = $type.GetMethod('Branch')
$branch = Get-ContractMethod $branchMethod $opcodes $limits $budget
Assert-ContractTest ($branch.status -eq 'decoded' -and @($branch.instructions | Where-Object { $_.branch_targets.Count -ne 0 }).Count -gt 0) 'real_branch_targets'
Assert-ContractTest ($branch.instructions.Count -gt 3 -and @($branch.instructions | Where-Object { $_.offset -isnot [int] }).Count -eq 0) 'instruction_array_must_be_flat'
Assert-ContractTest ((Get-ContractHash ([Convert]::FromBase64String($branch.il_base64))) -ceq $branch.il_sha256) 'raw_il_hash'
$passed.Add('real_branch_offsets_and_raw_il_hash')

$switch = Get-ContractMethod ($type.GetMethod('Switch')) $opcodes $limits $budget
$tables = @($switch.instructions | Where-Object { $_.opcode -ceq 'switch' })
Assert-ContractTest ($tables.Count -eq 1 -and $tables[0].branch_targets.Count -eq 6) 'real_switch_table'
$passed.Add('real_switch_relative_targets')

$handler = Get-ContractMethod ($type.GetMethod('Handler')) $opcodes $limits $budget
Assert-ContractTest (@($handler.exception_regions | Where-Object { $_.kind -ceq 'Finally' }).Count -eq 1) 'real_finally_region'
Assert-ContractTest (@($handler.exception_regions | Where-Object { $_.catch_type -ceq 'System.DivideByZeroException' }).Count -eq 1) 'real_catch_region'
Assert-ContractTest ($handler.locals.Count -gt 0 -and $handler.max_stack -gt 0) 'real_local_signature'
$passed.Add('real_exception_regions_and_locals')

$generic = Get-ContractMethod ($type.GetMethod('Generic')) $opcodes $limits $budget
Assert-ContractTest ($generic.status -eq 'decoded') 'generic_context_resolution'
Assert-ContractTest (@($generic.instructions | Where-Object { $null -ne $_.member -and $_.member.name -ceq 'Add' }).Count -eq 1) 'generic_member_identity'
$passed.Add('generic_metadata_token_resolution')

$constantMethod = $type.GetMethod('Constants')
$constants = Get-ContractMethod $constantMethod $opcodes $limits $budget
Assert-ContractTest (@($constants.instructions | Where-Object { $_.operand_kind -ceq 'InlineI8' -and $_.operand -ceq '1234567890123' }).Count -eq 1) 'int64_operand_preserved'
Assert-ContractTest (@($constants.instructions | Where-Object { $_.resolution -ceq 'string_content_not_collected' }).Count -eq 1) 'literal_token_without_content'
Assert-ContractTest (($constants | ConvertTo-Json -Depth 32 -Compress) -notmatch 'fixture_only_not_collected') 'string_literal_not_disclosed'
$passed.Add('operands_preserved_without_literal_contents')

$noBody = Get-ContractMethod ($type.GetMethod('NoBody')) $opcodes $limits $budget
Assert-ContractTest ($noBody.status -ceq 'no_body') 'no_il_explicitly_marked'
$passed.Add('external_method_no_body_is_explicit')

# 从真实 switch 的原字节截断，不能把部分表误当作完整反汇编。
$switchBytes = [Convert]::FromBase64String($switch.il_base64)
$cut = $tables[0].offset + $tables[0].size - 1
$truncated = [byte[]]$switchBytes[0..($cut - 1)]
Assert-ContractReject { Read-ContractIL $truncated ($type.GetMethod('Switch')) $opcodes $limits } 'truncated_switch_rejected'
Assert-ContractReject { Read-ContractIL ([byte[]]@(254)) $branchMethod $opcodes $limits } 'truncated_two_byte_opcode_rejected'
Assert-ContractReject { Read-ContractIL ([byte[]]@(32, 1, 2)) $branchMethod $opcodes $limits } 'truncated_int32_operand_rejected'
$passed.Add('truncated_switch_opcode_and_operand_rejected')

# 分支跳入操作数字节或方法末尾，均不应成为有效目标。
Assert-ContractReject { Read-ContractIL ([byte[]]@(43, 255, 42)) $branchMethod $opcodes $limits } 'branch_into_operand_rejected'
Assert-ContractReject { Read-ContractIL ([byte[]]@(43, 1, 42)) $branchMethod $opcodes $limits } 'branch_past_body_rejected'
$passed.Add('non_instruction_branch_targets_rejected')

$badToken = [Convert]::FromBase64String($constants.il_base64)
$call = @($constants.instructions | Where-Object { $_.operand_kind -ceq 'InlineMethod' })[0]
[BitConverter]::GetBytes([int]2147483647).CopyTo($badToken, $call.offset + $call.size - 4)
$badRows = @(Read-ContractIL $badToken $constantMethod $opcodes $limits)
Assert-ContractTest (@($badRows | Where-Object { $_.resolution -ceq 'unresolved' -and $null -ne $_.failure }).Count -eq 1) 'unresolved_token_not_silently_omitted'
$passed.Add('unresolved_metadata_keeps_token_and_failure')

$small = Get-ContractLimits; $small.method_bytes = 1
Assert-ContractReject { Get-ContractMethod $branchMethod $opcodes $small @{ methods = 0; il_bytes = 0 } } 'method_size_budget_rejected'
$small = Get-ContractLimits; $small.total_il_bytes = $branch.il_bytes
$used = @{ methods = 0; il_bytes = 0 }
[void](Get-ContractMethod $branchMethod $opcodes $small $used)
Assert-ContractReject { Get-ContractMethod $branchMethod $opcodes $small $used } 'aggregate_il_budget_rejected'
Assert-ContractTest ($used.il_bytes -eq 2 * $branch.il_bytes) 'failed_attempt_keeps_budget'
$small = Get-ContractLimits; $small.methods = 0
Assert-ContractReject { Get-ContractMethod $branchMethod $opcodes $small @{ methods = 0; il_bytes = 0 } } 'method_count_budget_rejected'
$small = Get-ContractLimits; $small.instructions = 1
Assert-ContractReject { Read-ContractIL ([Convert]::FromBase64String($branch.il_base64)) $branchMethod $opcodes $small } 'instruction_budget_rejected'
$passed.Add('method_instruction_and_aggregate_budgets_enforced')

$root = Join-Path ([IO.Path]::GetTempPath()) ('g09-il-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
try {
    $path = Join-Path $root 'contract.safe.json'
    $report = @{ method = $branch; target_invoked = $false }
    Save-ContractReceipt $path $report $limits
    $before = [IO.File]::ReadAllBytes($path)
    Assert-ContractReject { Save-ContractReceipt $path @{ replacement = $true } $limits } 'receipt_overwrite_rejected'
    Assert-ContractTest ((Get-ContractHash ([IO.File]::ReadAllBytes($path))) -ceq (Get-ContractHash $before)) 'receipt_preserved'
    $small = Get-ContractLimits; $small.receipt_bytes = 1
    $oversized = Join-Path $root 'oversized.json'
    Assert-ContractReject { Save-ContractReceipt $oversized $report $small } 'receipt_budget_rejected'
    Assert-ContractTest (-not [IO.File]::Exists($oversized)) 'oversized_receipt_not_created'
    $passed.Add('receipt_create_new_and_size_bound')
} finally {
    # 仅删除本轮新建且未包含外部链接的固定夹具目录。
    [IO.Directory]::Delete($root, $true)
}
$safe = Get-ContractFailure ([IO.IOException]::new('private_path_must_not_be_output'))
Assert-ContractTest (($safe | ConvertTo-Json -Compress) -notmatch 'private_path') 'failure_redacted'
$passed.Add('failure_contains_no_exception_message')
$peBytes = [byte[]]::new(128)
[BitConverter]::GetBytes([uint16]23117).CopyTo($peBytes, 0)
[BitConverter]::GetBytes([int]64).CopyTo($peBytes, 60)
[BitConverter]::GetBytes([uint32]17744).CopyTo($peBytes, 64)
[BitConverter]::GetBytes([uint16]34404).CopyTo($peBytes, 68)
[BitConverter]::GetBytes([uint16]523).CopyTo($peBytes, 88)
$pe = [IO.MemoryStream]::new($peBytes)
try {
    Assert-ContractX64Image $pe
    $peBytes[68] = 76; $peBytes[69] = 1
    Assert-ContractReject { Assert-ContractX64Image $pe } 'non_x64_host_rejected'
} finally { $pe.Dispose() }
$passed.Add('host_pe_machine_must_be_x64')
@{ passed = $passed.Count; tests = $passed.ToArray(); parser = 'passed'
   fixture_methods_invoked = $false; windows_collection_executed = $false } | ConvertTo-Json -Compress
