# 固定 Framework CLR 只读异常 reader

这是 G09 的独立诊断程序，不是产品调试后端，也不证明 PowerShell 或更新事务通过。仅支持 Windows x64 的 Framework 4.x 现场。必须先由固定 Framework 夹具验证本机 DAC 的实际接口行为，再考虑接入原失败停点。

## 构建与来源

在已经初始化的 MSVC x64 环境中编译 `reader.cpp`，使用 Windows SDK、C++17 和静态 CRT；不依赖 NETFXSDK 中可能不存在的 `xclrdata.h`，不安装任何组件：

```text
cl /nologo /std:c++17 /utf-8 /W4 /WX /O2 /MT /EHsc reader.cpp /Fo:<私有构建目录>\reader.obj /Fe:<私有构建目录>\reader.exe /link bcrypt.lib
```

`vendor/` 是 dotnet/runtime 固定提交的官方生成 ABI 头和 MIT 许可证，原始 URL、字节数和 SHA-256 见 `sources.safe.json`。不得格式化或修改这些原件。`sos_layout.h` 仅从同提交 `dacprivate.h` 复制五种公开 DAC 输出结构，使用 Windows x64 的 `sizeof`/`offsetof` 断言；不解释 CLR 私有 FieldDesc 位域。只查询主 `ISOSDacInterface`，不加载 SOS DLL，不使用 ISOS2。使用 .NET 8 的 ABI 声明不表示加载 .NET 8 DAC：运行时仅加载本机 `GetWindowsDirectoryW()` 下 `Microsoft.NET\Framework64\v4.0.30319\mscordacwks.dll`，不接受其它目录或搜索路径。

控制器必须先绑定实际 CLR `LOAD_DLL` 原文件句柄、映像基址、PE 时间戳/SizeOfImage/SHA，并确认同目录 DAC 与该 CLR 的数字文件版本完全相同。两文件保持拒写租约。reader 再核目标 CLR 内存 PE、DAC 全文件 SHA、逐级无重解析点父目录、加载后的 HMODULE 实际文件路径及 FileID。版本相同只是前提，不能代替实际 DAC 建实例、字段与栈读取成功。

`112adcc74` 已完成 Windows 编译和真实夹具执行，但原 `GetCurrentExceptionState` 在三个 first-chance 停点分别无对象、读到前一次 Win32Exception、读到前一次 InvalidOperationException，未满足当前异常合同。`9578948db` 的 last-thrown 候选已区分相同 HRESULT 的原码 1234/5678，但 Win32 字段读取耗尽旧额度，包装链的 inner 类型又退化为声明类型 System.Exception，栈映射也未完整。本次修改仍须新的固定夹具验证；macOS 的格式检查不能覆盖 Windows SDK 类型、COM ABI 或 Framework 4.8 行为。

`a8766acff` 已完整读到前两条 Win32 字段和不同原码，约 4.924 MB 读取未耗预算，但固定 Throw 方法的 IL 映射仍返回 E_NOINTERFACE，且第二个 reader 退出后的即时 Job 空检查失败，中断后两条包装异常。因此没有完整夹具通过结论；现有原件未记录原 RIP 与映射范围，不能断言失败已由方法端点解释。

## 单次控制协议

`wire.h` 定义 168 字节、小端、无填充的请求。尾部仅为指定长度、无 NUL 的 UTF-16LE DAC 路径，再以 stdin EOF 结束。没有命令行参数。读完整请求、尾部和 EOF 并再次核期限前，不查询目标或加载 DAC。

控制器使用原 reader 子进程句柄将它加入精确 `KILL_ON_JOB_CLOSE` Job 后，才将原停点进程/线程句柄复制到 reader：进程仅需 `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`，线程仅需 `THREAD_QUERY_INFORMATION | THREAD_GET_CONTEXT`。不按 PID/TID 重开。请求中的 PID/TID、双方创建 FILETIME、CLR 基址来自同一个仍被原调试器持有的 pending event。reader 复核这些对象身份；事件序号和 nonce 由控制器与请求/输出逐一对账，reader 本身不能独立证明调用方仍持有该调试事件。

原调试器必须在 reader 已退出并确认其精确 Job 为空后才 Continue，超时也必须先终止/等待本轮 reader，再继续原事件。任何 reader 超时、绑定失败、部分结果或无输出，都不能作为候选成功。同步 DAC/磁盘读取没有用户态硬抢占保证，唯一硬边界是控制器已有绝对期限及独立子进程的精确回收；不延长候选期限。

`operation=1` 读取一个 CLR first-chance 停点。上限为 16 MiB 目标虚拟内存、8192 次目标内存读取、4 层异常、32 次解栈推进，每帧最多 8 个 IL 偏移。失败的内存读取也扣预算。每次目标内存/线程上下文读取及主要循环检查同机 `GetTickCount64` 绝对期限。没有预算不足后自动重试或增额。

该诊断容量由元数据接口合同确定，不改变产品或候选期限。官方 `GetFieldDescData` 的 MetaSig 和字段名称查询均可能经 `GetMDImport` 整块读取模块元数据；不是读取三个字段就只需三个标量。既存系统模块记录中 System.dll 为 3,541,424 字节，mscorlib.dll 为 5,445,664 字节，两 MVID 与 `9578948db` 观测一致。以两完整文件容量加原 4 MiB 其它读取空间和夹具 1 MiB 文件上界规划，合计 14,229,968 字节，采用固定 16 MiB；这不是实测 metadata 大小或下一轮必过证明，也不替代当轮模块身份核验。原 4 MiB 的失败原件保留，PowerShell 的额外模块未在本夹具中验收。

`operation=2` 仅为控制器回收夹具：除了 magic/version/op/nonce/deadline，所有字段必须为零，不能带路径/目标句柄。完整请求后阻塞，由控制器原期限终止并确认 Job 空。它不加载 DAC、不接触目标，不是实际异常读取的替代。

stdout 必须由控制器绑定为本轮全新普通文件；不能使用可能写满且无人消费的管道。reader 最多输出一条 32768 字节 JSON。stderr 不写路径、异常消息或内存内容。控制器保存原始文件，再有界解析；不得仅凭退出码计通过。

## 只读接口和输出解释

仅提供 ICLRDataTarget 的身份/固定 CLR 基址、ReadVirtual 和原线程 GetThreadContext。所有目标写入、SetContext/TLS 写入都返回拒绝，不调用 Attach、Invoke、Suspend、Continue 或目标函数。只允许固定标准 CONTEXT 标志，不提供 XSTATE/任意其它线程。

使用 `GetLastExceptionState` 和 `GetManagedObject`，明确输出 `exception_source="last_thrown_object_candidate"`，不回退旧 tracker，不把 `GetPrevious` 冒充 InnerException，也不把 `IsSameState2` 的弱匹配当事件身份。固定官方参考实现先将待抛对象写入线程的 last-thrown handle，再调用 RaiseException；first-chance 通知早于 CLR 异常处理器更新 tracker。但这不是本机 Framework 实际新鲜性的证明：控制器必须保持原停点，并用实际线程、事件顺序、同 HRESULT 的不同 Win32 原码、完整包装链和固定 MethodDef/IL 逐项验证，不能仅凭事件 HRESULT 相同就把候选称为当前对象。

每层对象都由 `GetObjectData` 取得实际 MethodTable，再通过 `GetMethodTableData`、模块及 TypeDef 元数据绑定 MVID/类型；不再从引用字段的 `GetAssociatedValue` 推断动态类型。祖先链必须真实到达全局 Exception MethodTable；查找可选 Win32Exception 声明类型时，仅在全局 Object MethodTable 和 System.Object 名称同时吻合后确认不存在。

`GetMethodTableFieldData`/`GetFieldDescData` 给出真实声明 MT、模块、FieldDef 和偏移，再以同模块 `GetFieldByToken2` 核对唯一字段名，只直接读取 `System.Exception._HResult`、`_innerException` 与 `System.ComponentModel.Win32Exception.nativeErrorCode`。字段必须是预期标量或对象引用类型、非静态、非线程/上下文静态，地址须在已核对象范围内。inner 非零时重新绑定其实际 MT；循环、缺字段、身份或接口错误都失败。不会调用读取 Message/StackTrace 引用槽的 `GetObjectExceptionData`，不会读取 Message/Data/StackTrace/局部变量；输出类型名称限制在代码中的固定白名单，其它类型只报 `unknown_type` 并保留 MVID/TypeDef token。

原 `CLRDATA_EXCEPTION_PARTIAL` 完整保留在 `exception_state_flags`，`tracker_complete` 始终为 false。该标志表示 last-thrown 状态没有完整异常 tracker，不能清掉标志来声明成功。`object_chain_complete` 独立表示本轮对象字段及 InnerException 链已完整读到实际 null 尾端；它不声明 tracker 完整或候选必然是当前对象。

顶层 `status="observed"` 要求对象链与栈读取完整、至少一个托管帧、预算未耗尽且期限未到。任何上限截断、字段错误、未知映射或未解决的 API 失败均保留为 `partial`/`unavailable`，不冒充完整异常链。`inner_status` 仅为 `object`、`null`、`unavailable`、`truncated`；`null` 仅来自实际读取到的零引用。旧原件的字段 E_INVALIDARG、读取预算耗尽和 IL 映射失败不能仅由更换对象来源解释，也没有放宽其失败条件。

仅当 `GetILOffsetsByAddress` 返回 E_NOINTERFACE 时，才允许读取同一个 MethodInstance 的完整 `GetILAddressMap`（最多 256 项）、代表入口和一个原生代码范围。官方旧版 DAC 仅为 EPILOG 处理末项 `nativeEndOffset=0`；现代实现也处理末条有效 IL。兼容路径要求入口等于范围起点、原 RIP 严格落在 `[start,end)`、末项为非 sentinel 的 IL 且原始 end 等于入口、所有先前条目连续且有界，并且只有末项命中。它不读取或宣称证明 IL opcode 为 throw；实际抛出 MethodDef 与有效 IL 的对应仍由夹具验证。范围外（包括恰好等于 end）、多范围、超限、不完整或歧义均保留 partial。枚举器始终释放，所有读取仍使用原 16 MiB、8192 次和绝对期限；没有 IP 减一、猜测长度、最近项或默认第二项回退。

所有 HRESULT 字段都是无符号 32 位 JSON 数字；`native_error_code` 是有符号 32 位或 null。`event_hresult` 是原事件数值对照，不与字段值自动等同。帧仅输出模块 MVID、MethodDef token、IL 偏移列表及映射状态；不输出原始地址/路径。`context_hresult` 保留 GetContext 的原返回值，`mapping_hresult` 始终保留 GetILOffsetsByAddress 的原返回值（未调用时为 E_PENDING）。`mapping_source="terminal_end_marker"` 仅表示上述精确兼容路径成功，`il_status` 表示含边界检查的最终映射结果，原 API 的失败不会被覆盖。尝试兼容时的 `terminal_map` 保留固定 stage、兼容及枚举释放 HRESULT、条目/命中数量、已绑定范围长度、原 RIP 相对偏移和末项序号/相对起止；未取得的数值为 null。包括 `ip_offset==extent_length` 在内的失败证据不转换成成功。

退出码 0 表示已经绑定目标和 DAC 并产出读取结果，仍可能是 `partial` 或 DAC API 不可用；2 为非法/截断请求（`stage="request"`，未查询目标/未加载 DAC）；3 为绑定或 reader 自身失败；4 为输出失败。最终夹具必须按原事件顺序核四条固定异常链：Win32 原码 1234、在其 catch 内抛出的同 HRESULT 原码 5678、包装后者的 InvalidOperationException、再包装它的 ApplicationException；每条还须核固定 C# MVID/实际抛出 MethodDef 的真实 IL 帧。前一次同 HRESULT 对象、乱序、丢失 PARTIAL、字段/栈不完整均不得通过，不接受仅退出 0。

没有用户可见文案变化，无需本地化变更。

## 官方合同

- [CLRDataCreateInstance](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/clrdatacreateinstance-function)
- [ICLRDataTarget](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/iclrdatatarget-interface)
- [IXCLRDataTask](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/ixclrdatatask-interface)
- [IXCLRDataStackWalk::Next](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/ixclrdatastackwalk-next-method)
- [GetILAddressMap 接口](https://learn.microsoft.com/en-us/dotnet/framework/unmanaged-api/debugging/ixclrdatamethodinstance-getiladdressmap-method)
- [旧版 DAC 末项映射边界](https://github.com/dotnet/coreclr/blob/v2.0.0/src/debug/daccess/task.cpp#L3891-L3912)
- [现代 DAC 末条有效 IL 结束标记](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/task.cpp#L3834-L3904)
- [同一 MethodDesc 的原生范围](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/daccess.cpp#L5916-L5957)
- [Windows first-chance 与异常处理器顺序](https://learn.microsoft.com/en-us/windows/win32/debug/debugger-exception-handling)
- [固定官方字段枚举实现](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/inspect.cpp)
- [固定官方 SOS 主接口](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/inc/sospriv.idl)
- [固定官方 DAC 输出结构](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/inc/dacprivate.h)
- [实际 MT、FieldDesc 与对象大小接口](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/request.cpp#L1812-L1887)
- [DAC 整块元数据导入](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/daccess.cpp#L6679-L6778)
- [官方 ClrMD 普通实例字段地址规则](https://github.com/microsoft/clrmd/blob/4681623e8ad436a3c140bc51eac52f8532a48a37/src/Microsoft.Diagnostics.Runtime/ClrInstanceField.cs)
- [固定官方 last-thrown 状态与 PARTIAL 实现](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/task.cpp#L539-L557)
- [固定官方 RaiseException 前更新 last-thrown handle](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/vm/excep.cpp#L2736-L2795)

固定提交源码是所固定 ABI 版本的公开参考实现，不是本机 Framework 私有实现的验证替身。
