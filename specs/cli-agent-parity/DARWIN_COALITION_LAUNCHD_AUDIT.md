# macOS coalition / launchd 专属进程域只读核验

核验日期：2026-09-16。范围是为真实 Codex CLI 异常退出后的跨 PGID 工具残留寻找无需新增权限的清理与退出证明；本轮没有修改 Rust、签名、LaunchAgents 或系统配置，没有执行 `launchctl` 的创建、移除、信号或重启操作，也没有重复模型请求。

## 结论

**本轮未发现满足当前普通应用权限、公开接口及任务级隔离要求的可实施 coalition / launchd 替代方案。** 内核 coalition 的成员关系确实比 Unix 进程组强：普通后代可以跨 `fork`、`exec`、`posix_spawn` 保留成员身份，修改 PGID/SID 不会重新创建 task。但创建专属 coalition、指定新进程的 coalition、接收最终空域通知属于 launchd 的特权控制面。普通 `launchctl` 服务管理没有公开承诺“停止本任务及未来派生的全部成员，再返回不可复活的空域回执”。

因此不能将以下任一情况改写为 `cleanup_confirmed=true`：读到 coalition ID、查到服务已停止、`launchctl bootout` 成功、同 PGID 清理成功，或连续几次成员快照为空。当前异常退出保守拒绝恢复仍然必要；真实工具完整清理失败仍未完成，已有 [Darwin 进程清理记录](DARWIN_PROCESS_CONTAINMENT.md) 的负向证据不变。

## 本机与固定上游证据

- 本机：macOS `27.0` / build `26A428`，Darwin `27.0.0`、`xnu-13432.1.9~1/RELEASE_ARM64_T6000`，arm64。SDK 为 Xcode 中的 MacOSX SDK `27.0`。
- 官方 XNU：`xnu-12377.1.9`，固定提交 `f6217f891ac0bb64f3d375211650a4c1ff8ca1ea`。本轮 GitHub `commits/main` 查询也返回该提交。它不是本机 beta 内核的完整源码；不能用其实现细节声称覆盖本机所有未公开变动。
- 官方 launchd：`launchd-842.1.4`，固定提交 `d448a1c8f70a61202f8705f94337f686b87c30c4`。它是旧实现，仅作为源码佐证；本机行为契约优先使用随系统提供的 `launchctl(1)` / `launchd.plist(5)`。GitHub 导入日期不是功能首次发布的系统版本。
- 本轮只读下载保存在私有临时目录，仓库仅新增本记录。没有通过私有系统调用实际创建 coalition；权限结果来自固定内核守卫和官方接口用途说明，而不是伪造本机 `EPERM` 实测。

最直接的官方说明来自 [XNU coalition 设计文档](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/doc/observability/coalitions.md#L6-L23)：创建、终止、回收和指定 spawn coalition 的接口只供 launchd 或 XNU 单测使用；已有进程不能更换成员身份。文档也说明 loaded service 可以在重启时重用同一 coalition，因此 coalition ID 不能替代应用的 `runtime_generation`。

## 能力逐项核对

| 机制 | 普通应用能否取得任务专属域 | 未来 fork / setsid | 整体终止及空域证明 | 本次结论 |
| --- | --- | --- | --- | --- |
| 当前应用继承的 resource / jetsam coalition | 通常包含宿主及相关进程，不能假定每个 CLI 任务独享 | 普通直接派生继承 coalition；PGID/SID 与成员关系不同 | 不能安全杀掉整个宿主共享域，也没有每任务成员句柄 | 不能用于本任务清理 |
| 每任务单独的 launchd job | launchd 可以为 job 管理 coalition；普通用户的 agent 管理与内核控制权不同 | 已继承的普通后代保持 coalition，但 job 状态不是成员退出状态 | 公开 plist 默认清理只有同 PGID；没有公开的 job 整体空域回执 | 没有足够契约可据此实施 |
| `coalition_create` + `posix_spawnattr_setcoalition_np` | 固定内核要求 privileged coalition，指定 spawn 另可用私有 entitlement | 在已成功创建并关联的域内可保留成员身份 | 创建/成员关联接口不直接提供整体杀进程 | 普通产品权限不满足 |
| `coalition_terminate` / `coalition_reap` | 属于 privileged coalition 调用面 | `terminate` 后已有成员仍可 fork | `terminate` 不杀进程；`reap` 只在已终止且无活动成员时成功 | 有内核空域语义，但当前权限和公开接口不能建立这条控制链 |
| `proc_listcoalitions` / coalition PID 或资源查询 | 读取的是既有域信息，不创建或独占域 | 查询不会封住后续派生 | 快照不等于已封域并回收 | 仅诊断，不能抬高回执可信度 |
| launchd responsibility / PID domain | 责任归属或 XPC 可见性上下文 | 归属传播不等于父子树管理句柄 | 未发现公开的整体停止、空域订阅契约 | 不能代替生命周期监督 |

### 创建和成员关联的权限

`coalition()` 在分发 CREATE、TERMINATE、REAP 前调用 `task_is_in_privileged_coalition`；未满足直接返回 `EPERM`。这项判断检查 task 的 coalition privileged 位，**不是仅判断当前 UID 是否为 root**。初始化 coalition 持有该特权，普通应用不能通过设置一个创建 flag 自行取得它。开发内核可选的 `unrestrict_coalition_syscalls` 在普通 RELEASE 分支固定关闭。[系统调用守卫](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/sys_coalition.c#L220-L250)、[特权判断](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L1599-L1621)、[开发内核开关](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L84-L91)。

`posix_spawnattr_setcoalition_np` 只是把 coalition ID 写入 spawn 属性。真正执行时，非 privileged 调用方还必须具有 `com.apple.private.coalition-spawn`；该私有 entitlement 不会自动授予 CREATE/REAP 权限。至少必须明确 resource coalition，否则内核拒绝不完整关联；普通未指定调用沿用父 task 的 coalition。[spawn 守卫及关联条件](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_exec.c#L4046-L4144)、[私有 entitlement 名](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/mach/coalition.h#L34-L38)、[task 创建时继承](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/task.c#L1930-L1960)。

这条内核继承规则能覆盖普通直接后代另建 PGID/SID 的情况；它不表示经由外部常驻服务执行的任意工作都成为同域成员。XNU 文档明确区分 resource 与 jetsam coalition，扩展服务可能分配独立 resource coalition，XPC/扩展还存在私有的放弃 jetsam coalition 配置。不能把“代表调用方记账”解释为“获得该外部服务的整体终止权”。[资源与 jetsam 范围](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/doc/observability/coalitions.md#L122-L131)。

### terminate 不是 kill，空域通知也不是公开订阅接口

`coalition_terminate` 设置 `termrequested`；当 `active_count == 0` 时才标记 `terminated` 并发送通知，没有发信号或终止 task 的步骤。`active_count` 还包含持有该 coalition 的 voucher；因此它不是简单的当前 PID 个数。[终止实现](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L2175-L2234)、[活动计数结构](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L280-L304)。

这里还有不能忽略的注释与实现差异：syscall 注释称请求终止后新的外部 spawn 开始失败，但固定版本的 `coalition_find_and_activate_by_id` 和 `coalition_adopt_task_internal` 实际只拒绝 `terminated` / `reaped`，没有检查 `termrequested`。所以连“terminate 返回后已阻止新的外部成员”也不能当作该版本实现保证。域内现存成员仍可 fork 则在 syscall 注释中明确说明。[调用注释](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/sys_coalition.c#L95-L108)、[查找并激活守卫](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L1485-L1510)、[关联守卫](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L1980-L2006)。

`coalition_reap` 的确能提供更强的事实：在内核锁内确认 `terminated` 且 `active_count == 0`，随后将域标记为 reaped 并移出可查询列表。不能用一次进程快照模拟此原子条件。但这条接口仍受前述 privileged 守卫限制。[回收条件](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L2237-L2284)。

最终空域通知发往全机 `HOST_COALITION_PORT`，不是创建时返回的单任务订阅句柄。其 setter 经过 `host_priv_t`，固定实现还要求内核/initproc，或相应 CSR 限制被解除；仅 telemetry 特定端口另有 entitlement 例外，不适用于 coalition 端口。替换全机 coalition 通知端口也会影响系统管理，绝不是应用可尝试的隔离验证。[发送位置](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/coalition.c#L566-L581)、[全机端口映射](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/mach/host_special_ports.h#L233-L237)、[设置权限](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/osfmk/kern/host.c#L1314-L1362)。

### launchd 服务域、责任域和进程清理

本机 `launchctl(1)` 将 domain 定义为服务执行策略和 Mach bootstrap 名称的范围。`pid/<pid>` 是该进程可见的 XPC 服务上下文，`gui/<uid>` / `user/<uid>` 还共享部分资源；这些域名不是任务子树隔离句柄。system domain 的修改需要 root，而当前用户的 agent 管理不能据此推导出内核 coalition 管理特权。

本机 `launchd.plist(5)` 的 `AbandonProcessGroup`（609–613 行）明确：job 结束后默认清理相同 PGID 的剩余进程。旧官方实现也确实在 job 回收时调用 `killpg2(j->p, SIGTERM)`；它没有给跨 `setsid` 子树提供保证。[固定 launchd 默认规则](https://github.com/apple-oss-distributions/launchd/blob/d448a1c8f70a61202f8705f94337f686b87c30c4/man/launchd.plist.5#L364-L368)、[旧实现](https://github.com/apple-oss-distributions/launchd/blob/d448a1c8f70a61202f8705f94337f686b87c30c4/src/core.c#L3618-L3626)。

`launchctl kill` 文档只承诺向服务实例发送信号；`bootout` 描述移除服务或域定义，没有承诺整个 coalition 已空。`print` 明确不能当作 API，`procinfo` 仅诊断且需 root，其输出也不承诺稳定。本机文档的这些限制分别位于 `launchctl.1` 的 90–103、247–276、291–300、677–684 行，不能将服务状态或命令 exit code 直接接入生产退出回执。

responsibility 是另一层归属元数据。固定旧 launchd 源码初始化责任进程，并说明该归属延续到后代；XNU 的 `proc_set_responsible_pid` 仅保存 PID 及 executable UUID，没有附带成员句柄、原子阻止派生或整体退出事件。这里只能确认它不是所读代码中的清理原语；没有把未公开现代 responsibility 协议臆测为完全相同实现。[责任归属初始化](https://github.com/apple-oss-distributions/launchd/blob/d448a1c8f70a61202f8705f94337f686b87c30c4/src/core.c#L4682-L4694)、[内核归属字段](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_proc.c#L5063-L5079)。

Apple 公开 daemon 指南要求被管理进程避免自行 daemonize，并建议不调用 `setsid`；它没有承诺替任意第三方工具追回脱离进程组的后代。为每个任务增加 LaunchAgent 不能据此消除已经实测的 Codex 工具边界。[官方 launchd 行为要求](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)。

### 普通用户临时 job：可以研究，但不能直接当作已证明的替代实现

用户 agent 提供了无需系统级 daemon 的启动方式，专用 service label 也有价值：可将真实 CLI 的直接生命周期交给比 GUI 更长寿的 launchd 管理。**此处没有断言临时用户 job 普遍不可用。** 未解决的是普通调用方如何通过受支持接口确认它的全部跨 PGID/SID 成员已退出，以及这一确认如何关联特定运行代次。

本机手册还给出容易造成重复执行或假成功的具体约束：

| 操作 / 配置 | 本机公开说明 | 对后续试验的约束 |
| --- | --- | --- |
| `launchctl submit -l <label> -- <program>` | 不需要 plist，但会在失败后保持程序存活，即重新启动（`launchctl.1` 529–539 行） | 不能直接用它监督本次需要禁止重复执行的 CLI 任务 |
| `launchctl remove <label>` | 立即返回，不等待 job 停止（551–553 行） | 返回 0 不是直接进程或子树退出确认 |
| `launchctl stop <label>` | 满足按需启动条件时可能立即重启（557–560 行） | 不能当作取消后不再运行的保证 |
| `KeepAlive=false` 且无按需/定时触发 | 能避免为该服务配置主动复活条件 | 应用于下一次有界试验，而不是默认采用 `submit` |
| `ExitTimeOut` | job 停止时从 SIGTERM 升级到 SIGKILL 的等待时间（`launchd.plist.5` 402–409 行） | 只明确服务停止超时，不扩大 `AbandonProcessGroup` 的同 PGID 清理范围 |

若下一阶段进行无模型现场探索，可使用**私有临时目录中的 plist**向当前用户自己的 `gui/<uid>` 或 `user/<uid>` 域注册唯一 job，不写入任何自动加载目录；label 包含随机 task/generation，关闭 KeepAlive、定时与按需触发，所有文件路径只指向该次夹具。需要分别测：直接 job 启动/退出；宿主退出而 job 继续受管理；CLI 根 SIGKILL 后同 PGID 子进程与另建 SID 子进程的行为；取消/移除是否意外触发新实例。该计划本轮未执行，尚未确认当前 beta 的实际用户域注册结果。

已知夹具的启动标记、独立 IPC、退出和心跳可检验上述具体行为；不能用心跳静默或 PID 枚举把任意未来子孙的完整退出证据补出来。即使该次 `setsid` 夹具恰好被 launchd 清理，也还需要找到稳定的域身份与空域确认契约，才能更改生产回执条件。这是后续方向及证明门槛，不是对所有可能 launchd 机制的普遍否定。

## SDK、版本和快照边界

本机 SDK 中 `sys/coalition.h`、`mach/coalition.h`、`spawn_private.h`、`responsibility.h` 均不存在；公共 `spawn.h` / `libproc.h` 没有这些管理声明。`libsystem_kernel.tbd` 确实导出 coalition 与 setcoalition 符号，`libxpc.tbd` 也有 `_xpc_coalition_copy_info`，但符号可链接不代表公开支持契约或调用授权。

固定上游 `spawn_private.h` 将 `posix_spawnattr_setcoalition_np` 标为 macOS 10.10 起可用。**这是私有符号的版本标记，不是完整 containment 功能的最低可支持系统版本**；无法据此把当前产品的较老 macOS 支持范围判为已验证。[私有版本声明](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/libsyscall/wrappers/spawn/spawn_private.h#L83-L89)。

当前 SDK 另导出 `_coalition_info_pid_list`，而受核对的固定 XNU 没有对应实现。没有为该新符号猜参数或调用行为；它也没有出现在本机公共头文件中。已有 `proc_listcoalitions` 使用分次计数、分配、复制；固定 `kern.coalition_pid_list` 还具有 100 PID 的本地数组上限。这些列表是诊断快照，不创建专属域，也不原子封住新派生。[列表读取](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/proc_info.c#L1714-L1766)、[固定 PID 列表上限](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/sys_coalition.c#L718-L796)。

## 实施条件与本轮止点

要把 coalition 路线推进为实际实现，至少还需要一个公开、受支持、普通应用可调用的系统代理契约，同时具备：

1. 每个 runtime generation 独享的域，关联真实 CLI 根进程前即建立，且覆盖之后正常派生的跨 PGID/SID 工具。
2. 宿主或 CLI 崩溃后仍有效的域控制权，能够停止全部成员并处理停止期间的新派生。
3. 与该代域身份绑定的稳定终止/空域确认，而非 root 进程状态、PID 名单或调试输出。
4. 与宿主现有 coalition 和其他任务完全分离，不依赖额外 Apple 私有 entitlement、修改 SIP/CSR 或接管全机通知端口。

本次读取的接口没有同时满足这些条件，故没有可直接接入产品的“创建专属 coalition”修复。上面的临时用户 job 是可进一步证伪或补证的具体方向，本轮按只读范围未执行注册或控制操作。后续若找到上述代理契约，再用固定无模型心跳夹具验证宿主 SIGKILL、CLI SIGKILL、派生竞争、空域确认和代次隔离；未取得前继续保留 `cleanup_failed` 与 `unsafe_recovery_prevented` 的分别记账。

本轮无用户可见功能变更，无需本地化变更；没有新增 Cargo 门禁或把源码审计计为真实 CLI 清理验收。

## 证据摘要

| 只读文件 | SHA-256 |
| --- | --- |
| 固定 XNU `doc/observability/coalitions.md` | `4f2b293f0fa0b2a3fafbe80eb404c5ca0f9f41d7f4c2fb93802f67d811aaadc1` |
| 固定 XNU `bsd/kern/sys_coalition.c` | `1f252b9329751286f670a8d6824117c2712b4bf20bbf82376fe12c3d39835545` |
| 固定 XNU `osfmk/kern/coalition.c` | `244a162e8d29296b766c4dae07f7c8072bf67a742d8be5f233c71dae8e247976` |
| 固定 launchd `src/core.c` | `2584122ba6a925aaaf9e00411b6dd4a64f6f9fcca7fd30470cb2be5050b2744e` |
| 本机 `/usr/share/man/man1/launchctl.1` | `cf8752dd8eb8c3370c7d03b333430aa68a665c907c475ea407cb1b6542083ae2` |
| 本机 `/usr/share/man/man5/launchd.plist.5` | `1c5f5041c1d3492988bfa6f9dc6d969dfd072d20af2703c28d9a8f6ed6aaadcb` |
| 本机 SDK `usr/lib/system/libsystem_kernel.tbd` | `563ea4296cf1fdec17b6b561aa77f00cd08a7294b3f9e798999f934629f38004` |
