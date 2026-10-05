# 移植已验证的 codex_windows_notify.ps1 控制台写入，不触碰标准输出或控制台模式。
function Write-InfiniShellNotification([string] $Message) {
    $utf8 = New-Object Text.UTF8Encoding($false, $true)
    if ($Message.Length -eq 0 -or $utf8.GetByteCount($Message) -gt 1048576) {
        throw 'notification_size_invalid'
    }
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
    [InfiniShellNotifyConsole]::Write($Message)
}
