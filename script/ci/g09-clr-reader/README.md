# 固定 Framework CLR 只读异常 reader

这是 G09 的独立诊断程序，不是产品调试后端，也不证明 PowerShell 或更新事务通过。仅支持 Windows x64 的 Framework 4.x 现场。固定 Framework 四异常夹具已在 `1d85bdb36` 验证本机 DAC 的实际接口行为；接入原失败停点仍须保留相同绑定、期限与回收合同。

## 构建与来源

在已经初始化的 MSVC x64 环境中编译 `reader.cpp`，使用 Windows SDK、C++17 和静态 CRT；不依赖 NETFXSDK 中可能不存在的 `xclrdata.h`，不安装任何组件：

```text
cl /nologo /std:c++17 /utf-8 /W4 /WX /O2 /MT /EHsc reader.cpp /Fo:<私有构建目录>\reader.obj /Fe:<私有构建目录>\reader.exe /link bcrypt.lib
```

`vendor/` 是 dotnet/runtime 固定提交的官方生成 ABI 头和 MIT 许可证，原始 URL、字节数和 SHA-256 见 `sources.safe.json`。不得格式化或修改这些原件。`sos_layout.h` 仅从同提交 `dacprivate.h` 复制五种公开 DAC 输出结构，使用 Windows x64 的 `sizeof`/`offsetof` 断言；不解释 CLR 私有 FieldDesc 位域。只查询主 `ISOSDacInterface`，不加载 SOS DLL，不使用 ISOS2。使用 .NET 8 的 ABI 声明不表示加载 .NET 8 DAC：运行时仅加载本机 `GetWindowsDirectoryW()` 下 `Microsoft.NET\Framework64\v4.0.30319\mscordacwks.dll`，不接受其它目录或搜索路径。

控制器必须先绑定实际 CLR `LOAD_DLL` 原文件句柄、映像基址、PE 时间戳/SizeOfImage/SHA，并确认同目录 DAC 与该 CLR 的数字文件版本完全相同。两文件保持拒写租约。reader 再核目标 CLR 内存 PE、DAC 全文件 SHA、逐级无重解析点父目录、加载后的 HMODULE 实际文件路径及 FileID。版本相同只是前提，不能代替实际 DAC 建实例、字段与栈读取成功。

`112adcc74` 已完成 Windows 编译和真实夹具执行，但原 `GetCurrentExceptionState` 在三个 first-chance 停点分别无对象、读到前一次 Win32Exception、读到前一次 InvalidOperationException，未满足当前异常合同。`9578948db` 的 last-thrown 候选已区分相同 HRESULT 的原码 1234/5678，但 Win32 字段读取耗尽旧额度，包装链的 inner 类型又退化为声明类型 System.Exception，栈映射也未完整。本次修改仍须新的固定夹具验证；macOS 的格式检查不能覆盖 Windows SDK 类型、COM ABI 或 Framework 4.8 行为。

`a8766acff` 已完整读到前两条 Win32 字段和不同原码，约 4.924 MB 读取未耗预算，但固定 Throw 方法的 IL 映射仍返回 E_NOINTERFACE，且第二个 reader 退出后的即时 Job 空检查失败，中断后两条包装异常。因此没有完整夹具通过结论；现有原件未记录原 RIP 与映射范围，不能断言失败已由方法端点解释。

`1d85bdb36` 的 Windows 运行 `37277365566` 已通过全部四异常、同原工作线程及完整 inner 链核验。实际 throw 帧 IL 为16/16/36/24，第三项 direct，其余从严格校验的末项开放结束标记获得，保留原 E_NOINTERFACE。55原事件已 Continue，fixture自然退出0，所有reader收割且精确Job空；前轮失败保持。共享Rust控制器必须由固定夹具与首PS候选复用，不能用两套实现互相代验。

## 单次控制协议

`wire.h` 定义 168 字节、小端、无填充的请求。尾部仅为指定长度、无 NUL 的 UTF-16LE DAC 路径，再以 stdin EOF 结束。没有命令行参数。读完整请求、尾部和 EOF 并再次核期限前，不查询目标或加载 DAC。

控制器使用原 reader 子进程句柄将它加入精确 `KILL_ON_JOB_CLOSE` Job 后，才将原停点进程/线程句柄复制到 reader：进程仅需 `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`，线程仅需 `THREAD_QUERY_INFORMATION | THREAD_GET_CONTEXT`。不按 PID/TID 重开。请求中的 PID/TID、双方创建 FILETIME、CLR 基址来自同一个仍被原调试器持有的 pending event。reader 复核这些对象身份；事件序号和 nonce 由控制器与请求/输出逐一对账，reader 本身不能独立证明调用方仍持有该调试事件。

原调试器必须在 reader 已退出并确认其精确 Job 为空后才 Continue，超时也必须先终止/等待本轮 reader，再继续原事件。任何 reader 超时、绑定失败、部分结果或无输出，都不能作为候选成功。同步 DAC/磁盘读取没有用户态硬抢占保证，唯一硬边界是控制器已有绝对期限及独立子进程的精确回收；不延长候选期限。

`operation=1` 读取一个 CLR first-chance 停点。目标虚拟内存额度只接受夹具固定16 MiB或PowerShell固定24 MiB；8192次目标内存读取、4层异常、32次解栈推进，每帧最多8个IL偏移不变。失败的内存读取也扣预算。每次目标内存/线程上下文读取及主要循环检查同机 `GetTickCount64` 绝对期限。没有预算不足后自动重试或增额。

该诊断容量由元数据接口合同确定，不改变产品或候选期限。官方 `GetFieldDescData` 的 MetaSig 和字段名称查询均可能经 `GetMDImport` 整块读取模块元数据；不是读取三个字段就只需三个标量。既存系统模块记录中 System.dll 为 3,541,424 字节，mscorlib.dll 为 5,445,664 字节，两 MVID 与 `9578948db` 观测一致。以两完整文件容量加原 4 MiB 其它读取空间和夹具 1 MiB 文件上界规划，合计 14,229,968 字节，采用固定 16 MiB；这不是实测 metadata 大小或下一轮必过证明，也不替代当轮模块身份核验。原 4 MiB 的失败原件保留，PowerShell 的额外模块未在本夹具中验收。

PowerShell容量按已有同环境合同原件中的System.Management.Automation.dll 6,642,688字节及powershell.exe 454,656字节，在上述16 MiB上加两份完整文件上界，共23,874,560字节，固定为24 MiB。来源合同SHA `7c534817905aa10d038d2fa03a9cf04e22a5d71abd93fa099c9a254cc0661e54`；这是接入前容量规划，不是实际metadata大小、所有模块足够或PS成功的证据。夹具仍请求16 MiB，实际PS耗量与任何partial必须原样保存。

首PS授权只来自已验证私有验收清单，读取器路径和SHA写入原generation的独占sidecar，并绑定原启动清单摘要；候选环境不携带这些配置。控制器保存原CREATE_THREAD句柄，EXIT_THREAD成功Continue后才释放；停止、超时或reader错误时保留活动reader所有权，先终止原候选，再在既有清理期限内回收精确reader Job。等待切片不超过100ms，不能重建候选期限或把Drop视为回收成功。

`operation=2` 仅为控制器回收夹具：除了 magic/version/op/nonce/deadline，所有字段必须为零，不能带路径/目标句柄。完整请求后阻塞，由控制器原期限终止并确认 Job 空。它不加载 DAC、不接触目标，不是实际异常读取的替代。

`operation=3` 只在原 root 的自有 `SHGetFileInfoW` 返回单步停点读取 CLR 栈，要求 `exception_code=0x80000004`、`first_chance=1`、`exception_hresult=0`。它不是 CLR 异常：不调用异常对象或链读取，输出 `exception_source="none"`、空 `chain`、`object_chain_complete=false`、`tracker_complete=false`、`exception_state_flags=0`；`exception_api_hresult=E_PENDING` 表示没有调用异常接口。`status=observed` 仅表示栈完整且未耗尽额度或期限；部分帧、无帧和读取失败均保留其原状态。它不读取 Message、Data、StackTrace、locals 或参数值。

Rust 调用方通过独立 `ClrNativeReturnStop`、`ClrReader::start_native_return` 和 `bind_native_return_and_send` 请求 operation 3，使用 `native-return-reader-{sequence}` 独占输出名；原 `start`/`bind_and_send` 仍固定 operation 1，不能交叉绑定。两种操作均沿用原句柄及创建时间、序号、nonce、CLR/DAC 文件、16/24 MiB 额度、Job 和退出回收校验，回复 operation 必须匹配原请求。168 字节布局与 version 1 不变；自有返回点的地址、线程、断点恢复和 pending 所有权仍由原调试控制器证明，reader 不凭单步异常代码推断这些授权。

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

## 原生分类返回的运行时帧

Windows x64 本地固定夹具在 `0x4550` 返回停点实际读到 Framework 的 `DomainBoundILStubClass.IL_STUB_PInvoke` 首帧，其 MethodDef 为 nil（`0x06000000`），原 IL 映射为 `E_FAIL`；下一个固定调用者具有实际 MethodDef 与 IL 160。旧失败原件保持 `partial`，不追改结论。

仅 `operation=3` 的首帧允许识别为 `frame_kind="runtime_pinvoke_stub"`：同一 MethodInstance 的 `GetName(0)` 必须完整返回，固定 512 WCHAR 缓冲无截断或嵌入 NUL；名称必须精确匹配 Framework 的 `DomainBoundILStubClass.IL_STUB_PInvoke(` 或 `DomainNeutralILStubClass.IL_STUB_PInvoke(`。同时要求 nil MethodDef、managed simple type、未知 detailed type、完整原上下文、原 `E_FAIL` 和零 IL 条目。输出仅有固定名称类别、完整性和 HRESULT，不输出原名称或签名。

这个帧保留在栈中，`mapping_hresult` 和 `il_status` 仍为原 `E_FAIL`，不赋予伪造 IL。其它帧仍须为有效 metadata MethodDef 且映射完整；至少一个真实元数据帧和正常解栈结束才允许完整状态。未知 nil 帧、其它 stub、非首帧、名称读取失败和任何其它映射错误均保持 `partial`。控制器另行核对实际调用者 MVID/MethodDef/有效 IL，因此运行时桩不能代替调用者。

## 官方合同

- [Framework 同代 DAC 的桩类别与 CLR 到原生调用名称](https://github.com/dotnet/coreclr/blob/v2.0.0/src/vm/ilstubresolver.cpp#L42-L78)
- [同一 MethodInstance 的名称读取](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/task.cpp#L3861-L3900)

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

## 有界托管续点（显式 v2）

旧 v1 的 op1/2/3 请求布局与用途不变。显式 v2 在 168 字节原头与 DAC 路径之间增加固定 200 字节方法合同；op3 可请求当前真实栈中一个 MVID/MethodDef 的有界映射，op4 是独立托管续点类型，不能把它冒充 SHGetFileInfoW 返回或 CLR 异常。

显式 v3 仅用于 op3，在原头与 DAC 路径之间传两个固定 200 字节规格。两组 IL 不重叠且属于同一 MVID/MethodDef；reader 在同一原帧与 MethodInstance 取得两点，核同 pre nonce、出生身份、RSP、EnC、extent、完整 map/代码摘要及不同 IL/地址后整对交付。公开 `managed_mapping_pair` 与私有 `private_target_pair` 必须一致，缺半对、重复地址或 unknown 均拒绝；私有原件从公开 Value 移除。v1/v2 和单票据 op4 布局不变，读取/输出预算不增加。

当前双点规格的首次点为 Complete IL `0046`、Boolean local4（soloCommand）及 PSI local5，位于第一次 Process.Start 前；local0 此时不具备后续等待决策语义。core 撤回全部自有 DR，读取并回收 initial op4 后将 DR1 换成原 pair 的后续点，正常继续原事件，不能等待尚未创建的 Node。后续点沿用下列规格与 Node 竞态处理。真实候选最多四个 reader：pre-op3、initial-op4、continuation-op4 各一次；第四槽由 initial 恢复后至 Node 创建前原 worker 首个 CLR 异常与 post 分类读取互斥使用。异常占槽后仍保留 post 原生返回，但明确其 CLR 栈未读。首个不支持的 CLR 事件也耗槽，不追逐后续异常；旧固定四异常能力验与此候选预算分开记录。

方法规格最多四个预批准 IL、一个 Boolean local 索引和一个可选 ProcessStartInfo local 索引；不搜索名称、不枚举所有方法、不触发 JIT。原 PowerShell Complete 的批准集合为 `0230/0232/0293/0294`，local0 已赋值且未再变，local5 是 ProcessStartInfo。`0314/0315` 属于 finally，不能当作等价正常续点。实际命中只解释对应停点的状态，不把未命中当作分支值。

pre 只使用实际栈帧持有的 MethodInstance，读取完整最多 256 项 IL map、唯一原生 extent、EnC version 和同帧 RSP；扫描范围内第二个匹配即 unknown。公开记录扫描帧数、原 stack HRESULT 和 scan_complete，不能把 32 帧截断前缀的唯一匹配宣称为全栈唯一。只选择规格中有一个原始合法非空范围、逆向映射也唯一的精确 IL，不选最近项。完整原始 map 的 IL、相对起止、source type 逐项以四个小端 u64 串接后计算 SHA256。旧 DAC 任意位置 EPILOG 的 `end=entry` 及末项普通 IL 的同类开放标记，仅作为从该起点到 extent 末尾的保守排除区；不会把补出的长度当作批准地址，也不会从后续 funclet 序言猜测终点。

`map_notes` 在失败时也保留固定 stage、API 报告的 map_count、完整返回时最多 256 个原始 IL 整数和最多四个批准点的匹配数、闭范围状态及实际逆向 HRESULT/数量/单个 IL；未调用或未取得为 null。边界拒绝保留实际条目索引、IL、相对起止与 extent 长度，开放排除项也记录相对边界。extent 内零长项记录索引、IL 和相对位置，保留在完整 map 摘要中，但不占有半开范围、不可作为候选。stage 区分 EnC/extent/map API、完整性、范围重叠、批准点缺失和逆向验证，不输出原生地址，不为了补诊断追加 DAC 调用。

映射票据绑定 pre 序号/nonce、原进线程创建时间、方法版本、帧 RSP、完整代码/map 摘要及地址。地址仅在私有 reader stdout 原件和 opaque Rust target 内存在；poll 核验后从公开 Value 移除 private_target，target 一次领取，Debug 只输出遮蔽文本。core 负责 DR1 归属、Node 时序、执行页/代码复核、全撤 DR、reader 回收与原上下文恢复；reader 没有目标写入或 Continue 权限。

执行页复核仅投影到票据的完整原方法 extent；VirtualQueryEx 的方法外连续页增长不构成方法映射变化。每段仍必须已提交、可执行且无 guard/noaccess，原 allocation、类型、完整保护位、连续覆盖及完整代码摘要都须相同；样本读取前即截在方法末端，分段、身份、保护或代码不符仍拒绝。原生 SHGetFileInfoW 返回配对的映射比较不变。

父进程的续点可能先于子进程 CREATE 事件递送。[Windows 调试事件契约](https://learn.microsoft.com/en-us/windows/win32/debug/debugging-events)只保证 CREATE 先于该子进程自身的执行与其他事件。此时 core 先核验自有 DR1、原帧与映射，仅给原工作线程增加一次显式暂停，调用方用 [DBG_REPLY_LATER](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-continuedebugevent)延后同一事件。原 Continue 成功后才确认延期；实际 Node CREATE 的映像及创建身份完整绑定后，核对原暂停线程上下文、平衡该次暂停，再等待同一异常重放。helper 两端仅为原始首机会单步事件接受此状态，不改变 CLR 异常的 NOT_HANDLED 语义。

首次递送、Node CREATE、重放序号分别保留，不把首次命中改写成 Node 之后；未延后时首次递送即读取事件。只有重放的完整异常字段、原进线程身份、上下文及代码映射均匹配，才消费原有的一次续点读取预算。取消时在精确目标终止请求之后回收自有暂停，并要求原线程或进程退出获证；不通过改为嵌套 Wait、猜测 Node 或额外异常采集补齐证据。

op4 要求实际首个 metadata frame、原线程 RIP、同帧 RSP、MVID/token/EnC/extent/map/代码摘要均与票据一致。局部值必须 GetNumLocations 非零、flags 和精确 GetSize/GetBytes 成功；保留 get_local、位置、类型、字节读取各层 HRESULT。没有位置的 S_OK Value 不是成功读值。Boolean 必须核验真实类型名，且只接受一个字节的 0/1；未知为 null，不转成 false。引用 Value 的 GetType 不提供声明类型实例，因此可选 PSI 不调用这项接口及类型名称接口，两项 HRESULT 保留 E_PENDING；先读取精确八字节对象引用、实际对象/MT/类型，再通过现有 FieldDesc 与同模块 FieldDef 核对 `useShellExecute` 原始 Boolean 字段。不调用 getter，不猜偏移。只有实际 System.dll 的 getter→FieldDef 小合同匹配后，外层才能把该原始字段解释为属性。

[官方 stack.cpp 局部读取实现](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/stack.cpp#L889-L956)并非 E_NOTIMPL，但 ValueFromDebugInfo 可返回没有位置的 Value；[GetSize/GetBytes](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/inspect.cpp#L306-L419)才检验是否有字节。固定原 Framework 夹具仍须验证实际值，公开实现不替代原 DAC 能力验收。所有映射、代码和字段读取继续扣原 16/24 MiB、8192 次、32 帧、32768 字节输出及原绝对期限，不增加候选或异常预算。此能力本身不关闭 G09；无需本地化变更。
