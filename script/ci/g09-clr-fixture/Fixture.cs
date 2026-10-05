using System;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Threading;

// 固定异常只验证原调试停点的读取能力，不执行网络、文件或 CLI 操作。
internal static class ClrFixture
{
    private static int result = 1;

    private static int Main()
    {
        // 特意使用工作线程，防止读取器把创建时的主线程冒充当前异常线程。
        Thread worker = new Thread(Exercise);
        worker.Start();
        worker.Join();
        return result;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void ThrowNative()
    {
        throw new Win32Exception(1234, "fixed fixture");
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void WrapNative()
    {
        try { ThrowNative(); }
        catch (Win32Exception exception)
        {
            throw new InvalidOperationException("fixed fixture", exception);
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
        try { WrapOperation(); }
        catch (ApplicationException exception)
        {
            InvalidOperationException middle = exception.InnerException as InvalidOperationException;
            Win32Exception inner = middle == null ? null : middle.InnerException as Win32Exception;
            result = inner != null && inner.NativeErrorCode == 1234 ? 0 : 2;
        }
    }
}
