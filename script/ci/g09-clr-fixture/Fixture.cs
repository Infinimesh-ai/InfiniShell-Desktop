using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading;

// 固定异常和本地映像分类只验证原调试停点，不执行网络或真实 CLI 操作。
internal static class ClrFixture
{
    private static int result = 1;

    private static int Main(string[] arguments)
    {
        // 子模式在创建工作线程前返回，不能递归启动另一份夹具。
        if (arguments.Length == 1 && arguments[0] == "--bound-node-child")
            return Path.GetFileName(Assembly.GetExecutingAssembly().Location) == "fixture-node.exe" ? 0 : 3;
        if (arguments.Length != 0) return 3;
        // 特意使用工作线程，防止读取器把创建时的主线程冒充当前异常线程。
        Thread worker = new Thread(Exercise);
        worker.Start();
        worker.Join();
        return result;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern UIntPtr SHGetFileInfoW(string path, uint attributes, IntPtr information,
                                               uint informationSize, uint flags);

    // 固定能力测试显式保留此方法的局部变量；真实 PowerShell 不使用该编译设置。
    [MethodImpl(MethodImplOptions.NoInlining | MethodImplOptions.NoOptimization)]
    private static void ClassifyBoundNode(bool beforeStart)
    {
        string path = Path.Combine(Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location),
                                   "fixture-node.exe");
        if (path.StartsWith(@"\\?\", StringComparison.Ordinal)) path = path.Substring(4);
        if (beforeStart)
        {
            // 首次启动前调用独立取返回；读取器回收并恢复观察后，第二次验证预算外跳过和 RF。
            UIntPtr before = SHGetFileInfoW(path, 0, IntPtr.Zero, 0, 0x2000);
            SHGetFileInfoW(path, 0, IntPtr.Zero, 0, 0x2000);
            ProcessStartInfo start = new ProcessStartInfo(path, "--bound-node-child");
            start.UseShellExecute = false;
            start.CreateNoWindow = true;
            bool initialReady = true;
            InitialBoundary(initialReady, start);
            if (!initialReady) throw new InvalidOperationException("fixed initial state failed");
            using (Process child = Process.Start(start))
            {
                // 准备阶段核对唯一调用及其返回后的布尔读取，保证续点在赋值和边界调用之后。
                bool shouldWait = child != null;
                ContinuationBoundary(shouldWait, start);
                if (!shouldWait || !child.WaitForExit(5000) || child.ExitCode != 0)
                    throw new InvalidOperationException("fixed child failed");
            }
            Console.WriteLine(before.ToUInt64().ToString(CultureInfo.InvariantCulture));
        }
        else
        {
            // 首个固定异常的读取器回收后，仍须取得真实启动后分类返回。
            UIntPtr value = SHGetFileInfoW(path, 0, IntPtr.Zero, 0, 0x2000);
            Console.WriteLine(value.ToUInt64().ToString(CultureInfo.InvariantCulture));
        }
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void InitialBoundary(bool initialReady, ProcessStartInfo start)
    {
        if (!initialReady || start.UseShellExecute)
            throw new InvalidOperationException("fixed initial boundary failed");
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void ContinuationBoundary(bool shouldWait, ProcessStartInfo start)
    {
        if (!shouldWait || start.UseShellExecute)
            throw new InvalidOperationException("fixed continuation failed");
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void ThrowNative()
    {
        throw new Win32Exception(1234, "fixed fixture");
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void ThrowSecondNative()
    {
        // 与第一次相同 HRESULT；原生错误码和方法身份必须指向这次新抛出的对象。
        throw new Win32Exception(5678, "fixed fixture");
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void WrapNative()
    {
        try { ThrowNative(); }
        catch (Win32Exception)
        {
            ClassifyBoundNode(false);
            try { ThrowSecondNative(); }
            catch (Win32Exception exception)
            {
                throw new InvalidOperationException("fixed fixture", exception);
            }
        }
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void WrapOperation()
    {
        try { WrapNative(); }
        catch (InvalidOperationException exception)
        {
            throw new ApplicationException("fixed fixture", exception);
        }
    }

    private static void Exercise()
    {
        ClassifyBoundNode(true);
        try { WrapOperation(); }
        catch (ApplicationException exception)
        {
            InvalidOperationException middle = exception.InnerException as InvalidOperationException;
            Win32Exception inner = middle == null ? null : middle.InnerException as Win32Exception;
            result = inner != null && inner.NativeErrorCode == 5678 ? 0 : 2;
        }
    }
}
