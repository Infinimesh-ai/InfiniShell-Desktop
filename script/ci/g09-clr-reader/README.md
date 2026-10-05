# 固定 Framework CLR 只读异常 reader

这是 G09 的独立诊断程序，不是产品调试后端，也不证明 PowerShell 或更新事务通过。仅支持 Windows x64 的 Framework 4.x 现场。必须先由固定 Framework 夹具验证本机 DAC 的实际接口行为，再考虑接入原失败停点。

## 构建与来源

在已经初始化的 MSVC x64 环境中编译 `reader.cpp`，使用 Windows SDK、C++17 和静态 CRT；不依赖 NETFXSDK 中可能不存在的 `xclrdata.h`，不安装任何组件：

```text
cl /nologo /std:c++17 /utf-8 /W4 /WX /O2 /MT /EHsc reader.cpp /Fo:<私有构建目录>\reader.obj /Fe:<私有构建目录>\reader.exe /link bcrypt.lib
```

`vendor/` 是 dotnet/runtime 固定提交的官方生成 ABI 头和 MIT 许可证，原始 URL、字节数和 SHA-256 见 `sources.safe.json`。不得格式化或修改这些原件。使用 .NET 8 的 ABI 声明不表示加载 .NET 8 DAC：运行时仅加载本机 `GetWindowsDirectoryW()` 下 `Microsoft.NET\Framework64\v4.0.30319\mscordacwks.dll`，不接受其它目录或搜索路径。

控制器必须先绑定实际 CLR `LOAD_DLL` 原文件句柄、映像基址、PE 时间戳/SizeOfImage/SHA，并确认同目录 DAC 与该 CLR 的数字文件版本完全相同。两文件保持拒写租约。reader 再核目标 CLR 内存 PE、DAC 全文件 SHA、逐级无重解析点父目录、加载后的 HMODULE 实际文件路径及 FileID。版本相同只是前提，不能代替实际 DAC 建实例、字段与栈读取成功。

当前没有 Windows 编译或真实 DAC 成功声明；macOS 的格式检查不能覆盖 Windows SDK 类型、COM ABI 或 Framework 4.8 行为。

## 单次控制协议

`wire.h` 定义 168 字节、小端、无填充的请求。尾部仅为指定长度、无 NUL 的 UTF-16LE DAC 路径，再以 stdin EOF 结束。没有命令行参数。读完整请求、尾部和 EOF 并再次核期限前，不查询目标或加载 DAC。

控制器使用原 reader 子进程句柄将它加入精确 `KILL_ON_JOB_CLOSE` Job 后，才将原停点进程/线程句柄复制到 reader：进程仅需 `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`，线程仅需 `THREAD_QUERY_INFORMATION | THREAD_GET_CONTEXT`。不按 PID/TID 重开。请求中的 PID/TID、双方创建 FILETIME、CLR 基址来自同一个仍被原调试器持有的 pending event。reader 复核这些对象身份；事件序号和 nonce 由控制器与请求/输出逐一对账，reader 本身不能独立证明调用方仍持有该调试事件。

原调试器必须在 reader 已退出并确认其精确 Job 为空后才 Continue，超时也必须先终止/等待本轮 reader，再继续原事件。任何 reader 超时、绑定失败、部分结果或无输出，都不能作为候选成功。同步 DAC/磁盘读取没有用户态硬抢占保证，唯一硬边界是控制器已有绝对期限及独立子进程的精确回收；不延长候选期限。

`operation=1` 读取一个 CLR first-chance 停点。上限为 4 MiB 目标虚拟内存、8192 次目标内存读取、4 层异常、32 次解栈推进，每帧最多 8 个 IL 偏移。失败的内存读取也扣预算。每次目标内存/线程上下文读取及主要循环检查同机 `GetTickCount64` 绝对期限。没有预算不足后自动重试或增额。

`operation=2` 仅为控制器回收夹具：除了 magic/version/op/nonce/deadline，所有字段必须为零，不能带路径/目标句柄。完整请求后阻塞，由控制器原期限终止并确认 Job 空。它不加载 DAC、不接触目标，不是实际异常读取的替代。

stdout 必须由控制器绑定为本轮全新普通文件；不能使用可能写满且无人消费的管道。reader 最多输出一条 32768 字节 JSON。stderr 不写路径、异常消息或内存内容。控制器保存原始文件，再有界解析；不得仅凭退出码计通过。

## 只读接口和输出解释

仅提供 ICLRDataTarget 的身份/固定 CLR 基址、ReadVirtual 和原线程 GetThreadContext。所有目标写入、SetContext/TLS 写入都返回拒绝，不调用 Attach、Invoke、Suspend、Continue 或目标函数。只允许固定标准 CONTEXT 标志，不提供 XSTATE/任意其它线程。

使用 `GetCurrentExceptionState` 和 `GetManagedObject`，不回退 `GetLastExceptionState`，不把 `GetPrevious` 冒充 InnerException，也不把 `IsSameState2` 的弱匹配当事件身份。字段通过真实元数据定位声明类型，只读取 `System.Exception._HResult`、`_innerException` 与 `System.ComponentModel.Win32Exception.nativeErrorCode`。不会读取 Message/Data/StackTrace/局部变量；输出类型名称限制在代码中的固定白名单，其它类型只报 `unknown_type` 并保留 MVID/TypeDef token。

顶层 `status="observed"` 要求当前异常链与栈读取完整、至少一个托管帧且无 `CLRDATA_EXCEPTION_PARTIAL`。任何上限截断、字段错误、未知映射或 API 失败均保留为 `partial`/`unavailable`，不冒充完整异常链。`inner_status` 仅为 `object`、`null`、`unavailable`、`truncated`；`null` 仅来自实际读取到的零引用。

所有 HRESULT 字段都是无符号 32 位 JSON 数字；`native_error_code` 是有符号 32 位或 null。`event_hresult` 是原事件数值对照，不与字段值自动等同。帧仅输出模块 MVID、MethodDef token、IL 偏移列表及映射状态；不输出原始地址/路径。

退出码 0 表示已经绑定目标和 DAC 并产出读取结果，仍可能是 `partial` 或 DAC API 不可用；2 为非法/截断请求（`stage="request"`，未查询目标/未加载 DAC）；3 为绑定或 reader 自身失败；4 为输出失败。最终夹具必须逐项核三条固定异常链、Win32 原码 1234 和固定 C# MVID/MethodDef 的真实帧，不接受仅退出 0。

没有用户可见文案变化，无需本地化变更。

## 官方合同

- [CLRDataCreateInstance](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/clrdatacreateinstance-function)
- [ICLRDataTarget](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/iclrdatatarget-interface)
- [IXCLRDataTask](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/ixclrdatatask-interface)
- [IXCLRDataStackWalk::Next](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/ixclrdatastackwalk-next-method)
- [固定官方字段枚举实现](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/inspect.cpp)
- [固定官方当前异常实现](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/task.cpp)

后两项是所固定 ABI 版本的公开参考实现，不是本机 Framework 私有实现的验证替身。
