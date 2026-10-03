# 只读固定候选的原句柄；SHA 匹配前不把 DLL 文件名认定为旧快照模块。
# 不接收路径参数，不执行映像，不修改文件权限，不搜索替代映像。
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-FixedCandidates {
    @(
        @{ name = 'cmd.exe'; snapshot_role = 'root_cmd'; sha256 = '64afc6db3aad1289533662e2d79e27dd55c7dcdb8cd918b08e145ad82ad5acb4' }
        @{ name = 'ntdll.dll'; snapshot_role = 'root_module_1'; sha256 = 'cfb1a39a15036ee71daa52e428523c9e7bf83d85bb1b2f0a268d05c3b5392bdd' }
        @{ name = 'kernel32.dll'; snapshot_role = 'root_module_2'; sha256 = 'f338ef32eeed13f72ab31dd402a7c3d2452235814ffe2779fda8c76680fa0461' }
        @{ name = 'kernelbase.dll'; snapshot_role = 'root_module_3'; sha256 = 'e75b795bb6fc69711ed684832e0a73c4eeb1ab177bfc62d7415a26ada167ae1d' }
    )
}

function Get-StreamSha256([System.IO.Stream] $Stream) {
    $Stream.Position = 0
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { return [Convert]::ToHexString($sha.ComputeHash($Stream)).ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Copy-MatchingStream([System.IO.Stream] $Source, [string] $ExpectedHash, [string] $Destination) {
    # 完整读取摘要通过后才创建输出；同一源句柄不重新按路径打开。
    $before = Get-StreamSha256 $Source
    if ($before -cne $ExpectedHash) { throw [System.IO.InvalidDataException]::new('source_sha256_mismatch') }
    $output = [System.IO.FileStream]::new($Destination, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
    try {
        $Source.Position = 0
        $Source.CopyTo($output, 65536)
        $output.Flush($true)
        $copied = Get-StreamSha256 $output
        $after = Get-StreamSha256 $Source
        if ($copied -cne $ExpectedHash -or $after -cne $ExpectedHash -or $output.Length -ne $Source.Length) {
            throw [System.IO.InvalidDataException]::new('copy_sha256_mismatch')
        }
        return @{ before = $before; after = $after; copied = $copied; size = $output.Length }
    } finally { $output.Dispose() }
}

function Get-SafeFailure($Exception) {
    # 仅记录类型和原始错误码，不写入任意异常路径或环境内容。
    while ($Exception.InnerException) { $Exception = $Exception.InnerException }
    $code = $null
    if ($Exception -is [System.ComponentModel.Win32Exception]) { $code = $Exception.NativeErrorCode }
    return @{ type = $Exception.GetType().FullName; hresult = $Exception.HResult; win32 = $code }
}

function Get-Identity($Handle) {
    $i = [G09ImageContract.Native]::Information($Handle)
    return [ordered]@{
        volume_serial = $i.VolumeSerialNumber
        file_index = (([uint64]$i.FileIndexHigh -shl 32) -bor $i.FileIndexLow).ToString()
        size = (([uint64]$i.FileSizeHigh -shl 32) -bor $i.FileSizeLow)
        creation_time = (([uint64]$i.CreationTimeHigh -shl 32) -bor $i.CreationTimeLow).ToString()
        last_write_time = (([uint64]$i.LastWriteTimeHigh -shl 32) -bor $i.LastWriteTimeLow).ToString()
        attributes = $i.FileAttributes
        links = $i.NumberOfLinks
    }
}

function Open-DirectoryChain([string] $Path, $Locks) {
    # 根到叶逐一持句柄；拒绝重解析点且不共享删除，防止祖先改名/替换。
    $full = [System.IO.Path]::GetFullPath($Path)
    if ($full -notmatch '^[A-Za-z]:\\' -or $full.IndexOf(':', 2) -ge 0) { throw 'local_drive_path_required' }
    $parts = [System.Collections.Generic.List[string]]::new()
    $cursor = $full.TrimEnd('\')
    while ($cursor.TrimEnd('\').Length -gt 2) {
        $parts.Insert(0, $cursor)
        $cursor = [System.IO.Path]::GetDirectoryName($cursor)
        if ($null -eq $cursor) { break }
    }
    $parts.Insert(0, [System.IO.Path]::GetPathRoot($full))
    foreach ($part in $parts) {
        $handle = [G09ImageContract.Native]::Open($part, $true)
        $Locks.Add($handle)
        [G09ImageContract.Native]::ValidatePath($handle, $part, $true)
    }
}

function Invoke-Collection {
    if (-not $IsWindows -or -not [Environment]::Is64BitProcess) { throw 'windows_x64_process_required' }
    if ($args.Count -ne 0) { throw 'arguments_forbidden' }
    if ($env:GITHUB_RUN_ID -notmatch '^[0-9]+$' -or $env:GITHUB_RUN_ATTEMPT -notmatch '^[0-9]+$' -or $env:GITHUB_SHA -notmatch '^[0-9a-f]{40}$') { throw 'workflow_identity_required' }
    # P/Invoke 仅补充 .NET 未提供的原句柄身份/重解析点检查；不派生进程。
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
namespace G09ImageContract {
    [StructLayout(LayoutKind.Sequential)]
    public struct FileInformation {
        public uint FileAttributes, CreationTimeLow, CreationTimeHigh, LastAccessTimeLow, LastAccessTimeHigh;
        public uint LastWriteTimeLow, LastWriteTimeHigh, VolumeSerialNumber, FileSizeHigh, FileSizeLow;
        public uint NumberOfLinks, FileIndexHigh, FileIndexLow;
    }
    public static class Native {
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
        static extern SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError=true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool GetFileInformationByHandle(SafeFileHandle file, out FileInformation info);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
        static extern uint GetFinalPathNameByHandleW(SafeFileHandle file, StringBuilder path, uint length, uint flags);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
        static extern uint GetSystemDirectoryW(StringBuilder path, uint length);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool CreateDirectoryW(string path, IntPtr security);
        public static SafeFileHandle Open(string path, bool directory) {
            // OPEN_EXISTING + OPEN_REPARSE_POINT；源文件仅共享读，目录仅共享读写。
            var h = CreateFileW(path, directory ? 0x80u : 0x80000000u, directory ? 3u : 1u,
                IntPtr.Zero, 3u, 0x00200000u | (directory ? 0x02000000u : 0u), IntPtr.Zero);
            if (h.IsInvalid) { int error = Marshal.GetLastWin32Error(); h.Dispose(); throw new Win32Exception(error); }
            return h;
        }
        public static FileInformation Information(SafeFileHandle file) {
            FileInformation i;
            if (!GetFileInformationByHandle(file, out i)) throw new Win32Exception(Marshal.GetLastWin32Error());
            return i;
        }
        public static void ValidatePath(SafeFileHandle file, string expected, bool directory) {
            var info = Information(file);
            if ((info.FileAttributes & 0x400u) != 0 || ((info.FileAttributes & 0x10u) != 0) != directory)
                throw new InvalidOperationException("reparse_or_kind_mismatch");
            var path = new StringBuilder(32768);
            uint size = GetFinalPathNameByHandleW(file, path, (uint)path.Capacity, 0);
            if (size == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (size >= path.Capacity || !String.Equals(path.ToString().TrimEnd('\\'), (@"\\?\" + expected).TrimEnd('\\'), StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("canonical_path_mismatch");
        }
        public static string SystemDirectory() {
            var path = new StringBuilder(32768);
            uint size = GetSystemDirectoryW(path, (uint)path.Capacity);
            if (size == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (size >= path.Capacity) throw new InvalidOperationException("system_directory_length");
            return path.ToString();
        }
        public static void CreateNewDirectory(string path) {
            if (!CreateDirectoryW(path, IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }
}
'@
    $outputLocks = [System.Collections.Generic.List[Microsoft.Win32.SafeHandles.SafeFileHandle]]::new()
    $sourceLocks = [System.Collections.Generic.List[Microsoft.Win32.SafeHandles.SafeFileHandle]]::new()
    $report = [ordered]@{
        schema = 'g09-fixed-image-contract-v1'; source_commit = $env:GITHUB_SHA
        run_id = $env:GITHUB_RUN_ID; run_attempt = $env:GITHUB_RUN_ATTEMPT
        reference_commit = 'b3c8cc8392050a98339885c4aeff78504e21fa79'
        reference_generation = 'e7c9aa44-b1f2-46b3-81a6-519abca693fa'
        reference_stderr_sha256 = 'c4ba589282f1be80fcd212a3b476d6cac5d4a51e6cd5faaf2582c5e51545daef'
        scope = 'static_bytes_only_no_execution_no_g09_closure'
        passed = $false; source_handles_released = $false; stage = 'output_directory'
        failure = $null; source_ancestors = @(); files = @()
    }
    try {
        Open-DirectoryChain $env:RUNNER_TEMP $outputLocks
        $outputDirectory = Join-Path $env:RUNNER_TEMP "g09-image-contract-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
        [G09ImageContract.Native]::CreateNewDirectory($outputDirectory)
        $outputHandle = [G09ImageContract.Native]::Open($outputDirectory, $true)
        $outputLocks.Add($outputHandle)
        [G09ImageContract.Native]::ValidatePath($outputHandle, $outputDirectory, $true)
        try {
            $report.stage = 'source_directory'
            $systemDirectory = [G09ImageContract.Native]::SystemDirectory()
            if ([System.IO.Path]::GetFileName($systemDirectory) -ine 'System32') { throw 'system32_required' }
            Open-DirectoryChain $systemDirectory $sourceLocks
            $report.source_ancestors = @($sourceLocks | ForEach-Object { @{ before = (Get-Identity $_); after = $null } })
            foreach ($candidate in Get-FixedCandidates) {
                $item = [ordered]@{
                    candidate_relative_path = "System32/$($candidate.name)"; snapshot_role = $candidate.snapshot_role
                    expected_sha256 = $candidate.sha256; matched_reference = $false; stage = 'open'
                    before = $null; after = $null; sha256_before = $null; sha256_after = $null
                    artifact_relative_path = $null; copied_sha256 = $null; copied_size = $null
                    passed = $false; failure = $null
                }
                $report.files += $item
                $handle = $null; $stream = $null
                try {
                    $path = Join-Path $systemDirectory $candidate.name
                    $handle = [G09ImageContract.Native]::Open($path, $false)
                    [G09ImageContract.Native]::ValidatePath($handle, $path, $false)
                    $item.before = Get-Identity $handle
                    if ($item.before.size -lt 1 -or $item.before.size -gt 32MB) { throw 'file_size_bound' }
                    $stream = [System.IO.FileStream]::new($handle, [System.IO.FileAccess]::Read, 65536, $false)
                    $item.stage = 'source_hash'
                    $item.sha256_before = Get-StreamSha256 $stream
                    $item.sha256_after = Get-StreamSha256 $stream
                    $item.after = Get-Identity $handle
                    if (($item.before | ConvertTo-Json -Compress) -cne ($item.after | ConvertTo-Json -Compress)) { throw 'source_identity_changed' }
                    if ($item.sha256_before -cne $candidate.sha256 -or $item.sha256_after -cne $candidate.sha256) { throw 'source_sha256_mismatch' }
                    $item.matched_reference = $true
                    $item.stage = 'copy'
                    $item.artifact_relative_path = "$($candidate.name).pe"
                    $copy = Copy-MatchingStream $stream $candidate.sha256 (Join-Path $outputDirectory $item.artifact_relative_path)
                    $item.sha256_after = $copy.after; $item.copied_sha256 = $copy.copied; $item.copied_size = $copy.size
                    $item.after = Get-Identity $handle
                    [G09ImageContract.Native]::ValidatePath($handle, $path, $false)
                    if (($item.before | ConvertTo-Json -Compress) -cne ($item.after | ConvertTo-Json -Compress)) { throw 'source_identity_changed' }
                    $item.stage = 'complete'; $item.passed = $true
                } catch { $item.failure = Get-SafeFailure $_.Exception }
                finally {
                    if ($stream) { $stream.Dispose() }
                    if ($handle) { $handle.Dispose() }
                }
            }
            for ($i = 0; $i -lt $sourceLocks.Count; $i++) {
                $report.source_ancestors[$i].after = Get-Identity $sourceLocks[$i]
                $before = $report.source_ancestors[$i].before; $after = $report.source_ancestors[$i].after
                # 目录时间可被系统活动改变；只比较原对象、类型及无重解析属性。
                foreach ($key in @('volume_serial', 'file_index', 'creation_time', 'attributes')) {
                    if ($before[$key] -ne $after[$key]) { throw 'ancestor_identity_changed' }
                }
            }
            $report.passed = @($report.files | Where-Object { -not $_.passed }).Count -eq 0
            $report.stage = 'complete'
        } catch { $report.failure = Get-SafeFailure $_.Exception }
        finally {
            foreach ($handle in $sourceLocks) { $handle.Dispose() }
            $report.source_handles_released = $true
        }
        $json = [System.Text.UTF8Encoding]::new($false).GetBytes(($report | ConvertTo-Json -Depth 12) + "`n")
        $index = [System.IO.FileStream]::new((Join-Path $outputDirectory 'index.safe.json'), [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        try { $index.Write($json, 0, $json.Length); $index.Flush($true) } finally { $index.Dispose() }
        if (-not $report.passed) { throw 'fixed_image_contract_failed_see_index' }
    } finally {
        foreach ($handle in $sourceLocks) { $handle.Dispose() }
        foreach ($handle in $outputLocks) { $handle.Dispose() }
    }
}

if ($args.Count -ne 0) { throw 'arguments_forbidden' }
Invoke-Collection
