$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

try {
    # 只读取本次通知的独立管道，不占用原生 hook 的 JSON 标准输入/输出。
    $inputStream = [Console]::OpenStandardInput()
    $buffer = New-Object byte[] 8192
    $bytes = New-Object IO.MemoryStream
    try {
        while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            if ($bytes.Length + $count -gt 1048576) { throw 'Notification exceeds the byte limit' }
            $bytes.Write($buffer, 0, $count)
        }
        $utf8 = New-Object Text.UTF8Encoding($false, $true)
        $message = $utf8.GetString($bytes.ToArray())
    } finally {
        $bytes.Dispose()
    }
    if ($message.Length -eq 0) { throw 'Notification is empty' }
    # 写 Unicode 控制台 API，避免依赖或改动用户的控制台代码页及模式。
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class InfiniShellNotifyConsole {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr security,
                                    uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleMode(IntPtr handle, out uint mode);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    static extern bool WriteConsoleW(IntPtr handle, string text, uint length, out uint written, IntPtr reserved);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);
    public static void Write(string text) {
        IntPtr handle = CreateFileW("CONOUT$", 0xC0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        if (handle == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error());
        try {
            uint mode;
            if (!GetConsoleMode(handle, out mode)) throw new Win32Exception(Marshal.GetLastWin32Error());
            if ((mode & 5) != 5) throw new InvalidOperationException("Console VT processing is unavailable");
            uint written;
            if (!WriteConsoleW(handle, text, (uint)text.Length, out written, IntPtr.Zero))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            if (written != text.Length) throw new InvalidOperationException("Notification write was incomplete");
        } finally {
            CloseHandle(handle);
        }
    }
}
'@
    [InfiniShellNotifyConsole]::Write($message)
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
