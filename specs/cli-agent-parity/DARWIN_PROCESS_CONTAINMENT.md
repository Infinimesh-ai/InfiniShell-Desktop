# Darwin 跨进程组子树证明的边界

2026-09-16，本机 macOS 27.0（26A428）。调查源于真实 Codex SIGKILL 后工具残留，见 `CODEX_SUPERVISED_TOOL_TREE_EVIDENCE.md`。本轮没有修改 Rust、签名权限、系统服务或用户配置。以下结论区分本机 API 实测、固定源码与仍不可用的机制，不把进程扫描当作完整清理证明。

## kqueue 不能继承追踪整棵树

本机 SDK `sys/event.h` 明确注明 NOTE_TRACK、NOTE_TRACKERR 和 NOTE_CHILD 从 macOS 10.5 起不再支持。固定 XNU 提交 `f6217f891ac0bb64f3d375211650a4c1ff8ca1ea` 的 `filt_procattach` 对这些位直接返回 ENOTSUP；NOTE_FORK 事件不向用户 kevent 传递子 PID。[固定头文件](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/event.h)、[固定实现](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_event.c)。

`probe_darwin_process_apis.py` 在允许夹具 fork 之前注册监听。本机注册 NOTE_TRACK 返回 errno 45；单独 NOTE_FORK/NOTE_EXIT 可以收到，ident 都是根 PID，data 均为 0，没有子 PID。记录位于 `fixtures/darwin-process-api-evidence.json`。这是接口预期的负向验证成功，不是子树追踪能力成功。

libproc 的父子关系/进程列表读取是当前快照。即使收到 NOTE_FORK 后立刻读取，也无法保证捕获已经退出的中间父进程及其后代关系；反复读取或一段时间无变化都不能补回内核未交付的历史。因此不能把这种算法标为新的强 containment。

## 审计令牌可以避免误杀复用 PID，但不补全发现过程

本机 `proc_signal_with_audittoken` 可用。固定内核按 PID version 查找目标、检查调用者权限，再持有原进程引用发信号；它不等于先读取 PID 再使用普通 kill 的有竞争窗口路径。[固定信号实现](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/proc_info.c)、[固定进程身份实现](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_proc.c)。

无模型测试只对自己创建的 sleep 调用该接口：错误 PID version 返回 ESRCH，原进程保持运行；正确 version 的 SIGKILL 返回成功，原进程退出 -9。测试以 libproc 的固定 `PROC_PIDUNIQIDENTIFIERINFO` 结构取得版本，记录结构大小 56 字节。这里验证的是版本不匹配保护，没有制造真实系统 PID 复用。

此机制可以作为后续清理“已证明属于本次任务且身份仍匹配”的残留工具的基础；若该进程在 exec 后版本变化、接口不可用或操作失败，必须保留不确定状态。它既不提供递归发现，也不能证明没有遗漏的孙进程。当前应用最低部署版本仍为 10.14，不能用仅在本机存在的符号无条件链接所有支持系统；后续实现需要明确版本/动态符号门槛并保留失败状态。

## 新版 Endpoint Security 有子树接口，但当前产物没有权限

本机 27.0 SDK 新增 `es_new_descendants_client`，声明范围是调用者及递归后代，包含未来 fork/exec。它不要求 root 或 TCC，但仍要求 `com.apple.developer.endpoint-security.client` entitlement，且最低 macOS 27.0；旧的 `es_new_client` 有不同的 root/TCC 条件，不能混淆。[Apple Endpoint Security 文档入口](https://developer.apple.com/documentation/endpointsecurity)。

通过临时 C 程序只调用该新接口、不订阅任何系统事件，本机返回 `ES_NEW_CLIENT_RESULT_ERR_NOT_ENTITLED`（3），见 `fixtures/darwin-endpointsecurity-descendants-evidence.json`。没有添加 entitlement、重签用户程序、申请系统权限或改产品签名。该机制不能作为当前普通用户安装和旧 macOS 的现成修复。即便将来获得权限，事件丢失、退出确认与回执代次关联仍需单独设计和验证。

对应复跑源码为 `script/cli-agent-parity/probe_darwin_descendants_client.c`，使用 macOS 27 SDK 的 `xcrun clang -fblocks <源码> -lEndpointSecurity -o <临时可执行文件>` 构建。输出值是原生 `es_new_client_result_t`，程序 exit 0 只表示完成查询，不代表获准创建客户端。

## 当前最小可交付防线与后续范围

1. 对仅有 unix_process_group 证明的信号退出或非零退出，应用不能授予恢复许可；必须同时拒绝历史错误 `cleanup_confirmed=true` 回执。新诊断回执可以保留，但应写 `cleanup_confirmed=false`。bundle4 的真实 Codex SIGKILL 重验已观察到关联回执为 false，`unsafe_recovery_prevented=true`；工具仍在写心跳，`cleanup_failed=true`、整体 `passed=false`，见 `CODEX_SUPERVISED_TOOL_TREE_EVIDENCE.md`。Rust 旧回执拒绝与 GUI 恢复入口不由这个字段 probe 代测。
2. 普通宿主 EOF 促成 Codex 正常退出的成功证据仍保留。它不证明异常根退出安全，也不能用正常根 code 0 推断任意未知 CLI 的所有外部工作都已停止。
3. 若后续增加基于审计令牌的已知工具清理，它只是安全减小残留范围；没有完整发现证明时仍不能放开恢复。不能新增“静默若干毫秒即成功”的判断。
4. 当前无额外权限的 Darwin 接口尚未形成可证明完整跨 PGID/SID 子树的实现。原生 CLI 崩溃的自动清理与随后安全继续仍未完成；不得把保守阻断或本机 API 负向测试计作 P4/P5 全部完成。

代码冻结期间，本调查只交付探测脚本、真实结果和方案。固定源码文件摘要保存在 `fixtures/darwin-process-source.json`；该提交用于可复核的机制说明，没有声称它正好是本机 beta 内核的完整源码。
