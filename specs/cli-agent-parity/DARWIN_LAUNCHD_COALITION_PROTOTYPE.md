# macOS 专属 launchd job 与 coalition 原型

核验日期：2026-09-16。范围仅为私有临时目录中的固定原生 helper、唯一随机用户 job、只读 coalition 查询和已核实身份的进程清理。未修改产品、Rust、系统服务配置或签名，未运行 Cargo、CLI 模型或账号操作。完整原始结果与源码摘要见 [原型证据](fixtures/darwin-launchd-coalition-prototype-macos27.json)。

本轮确认了一个值得继续验证的接入方向：普通用户的专属 launchd job 可以建立独立 resource coalition，普通 `setsid` 和双重 `fork` 后代保持成员身份；清理后，该 CID 从可查询对象变为 `ESRCH`。固定公开 XNU 实现中，这对应先确认域已终止、活动引用为零，再移出查询表的回收过程。**这比空 PID 快照更强，但本轮尚未实现产品 containment，也没有重验真实 Codex 工具。**

两个负向结果同样保留：根进程 `SIGKILL` 后，另建 SID 的两后代仍写心跳；`bootout` 返回成功后，它们继续写心跳。两者都不能单独作为退出回执。

## 环境、接口和版本边界

| 项目 | 实际值或边界 |
|---|---|
| 本机 | macOS 27.0 / 26A428，arm64，Darwin 27.0.0 |
| 实际内核 | `xnu-13432.1.9~1/RELEASE_ARM64_T6000` |
| 新接口核对源码 | `xnu-12377.121.6`，`ac9718fb1af618d5ce8678d0dc6e8a58f252216f` |
| 源码与本机是否同版 | 否；实测与固定实现一致，不等于当前内核源码已逐行核对 |
| 用户域 | 当前普通用户 `gui/502`；未使用 root、额外 entitlement 或系统级域 |
| PID 身份 | `PROC_PIDUNIQIDENTIFIERINFO` 的 unique ID、PID version，加精确 helper 路径；发送信号使用 `proc_signal_with_audittoken` |
| 成员关联 | `proc_pidinfo(..., PROC_PIDCOALITIONINFO, ...)`，实际返回 40 字节，分别读取 resource / jetsam CID |
| 只读计数 | `coalition_info_resource_usage(cid, buffer, 16)`，读取结构前两个计数；计数快照不作终态 |
| 新成员查询 | `coalition_info_pid_list(uint64_t cid, pid_t *pid_list, size_t *size_inout)` |
| 公开 SDK | 本机没有 `sys/coalition.h`、`sys/coalition_private.h`、`mach/coalition.h`；该查询属于私有接口，不能据符号存在推导支持承诺 |
| 最低系统 | 未建立。受查发布源码中 `12377.1.9` 无新 wrapper，`12377.41.6` 及之后五个固定版本有；这不是 macOS 最低版本的官方声明 |

新接口的准确签名来自 [固定私有头文件](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/sys/coalition_private.h#L51-L60)，其 [wrapper](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/libsyscall/wrappers/coalition.c#L78-L82) 直接调用按 CID 查询的 syscall。本轮在取得签名后才调用，没有猜测 ABI。创建、请求终止、回收 coalition 仍由 launchd 执行；原型没有调用需要 privileged coalition 的管理 syscall。

## 首轮：旧 sysctl 明确失败后停止

首轮目录为 `/private/tmp/infinishell-coalition-launchd-prototype-4jqvb4zy`，唯一 label 为 `dev.infinishell.parity.coalition.6061b2c6-a6f3-4abd-b1a9-05d1dbe3c512`。

`bootstrap` 与 `kickstart` 均返回 0，根 helper 的 resource CID 为 22514，域外对照为 7944。然而 `kern.coalition_pid_list` 返回 `ENOENT`；后续只读 `sysctlnametomib` 同样返回 `ENOENT`，确认失败的是接口名称解析，不能解释为空成员列表。固定旧实现将该 sysctl 放在 `DEVELOPMENT || DEBUG` 分支；普通 RELEASE 内核不可据此使用。[旧实现条件](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/sys_coalition.c#L599-L601)

因此首轮没有写入 `go`，没有派生后代或执行根 `SIGKILL`。原型只 `bootout` 自己的 job，确认根与对照的退出事件，并保留原始 `failure`。这轮不计成功；它补正了 [前次只读审计](DARWIN_COALITION_LAUNCHD_AUDIT.md) 对旧 sysctl 实际可用范围的缺失。

## 第二轮：跨 SID 后代与域销毁

第二轮目录为 `/private/tmp/infinishell-coalition-launchd-prototype-v2-x4wx636w`，唯一 label 为 `dev.infinishell.parity.coalition.8eca8546-a2dd-413b-9d98-f492d872ab61`。

plist 仅设置固定 helper 路径、私有工作目录、`KeepAlive=false`、`AbandonProcessGroup=false`、`ExitTimeOut=2` 与私有输出路径；无定时、按需触发器或自动加载目录写入。显式 `kickstart` 一次后，根 helper 先等待私有 `go` 文件。只有确认它拥有独立 CID 且当时成员恰为根进程，才允许固定派生动作。

| 角色 | PID | PGID | SID | PID version | resource CID |
|---|---:|---:|---:|---:|---:|
| 域外对照 | 89534 | 89523 | 89523 | 592187 | 7944 |
| job 根进程 | 89539 | 89539 | 1 | 592199 | 22612 |
| `setsid` 后代 | 89540 | 89540 | 89540 | 592200 | 22612 |
| 双重 fork 中间进程 | 89541 | 89541 | 89541 | 592201 | 22612 |
| 双重 fork 最终后代 | 89542 | 89541 | 89541 | 592202 | 22612 |

中间进程正常退出并由根 `waitpid` 回收。其余四个持久 helper 均先注册 `kqueue NOTE_EXIT`，再次核对身份，才进入后续步骤。每次信号仅发给记录中的 helper，重新比对 unique ID、PID version 和路径，再由内核按 audit token 的 PID version 绑定信号；没有对枚举结果中的未知成员发信号。

| 阶段 | resource 查询 started / exited | 成员 PID | 后代是否继续写入 |
|---|---|---|---|
| 根 SIGKILL 前 | 5 / 2 | 89539、89540、89542 | 是 |
| 根 SIGKILL 后 | 5 / 3 | 89540、89542 | 是 |
| `bootout` 返回 0 后 | 5 / 3 | 89540、89542 | 是 |
| 仅清理本次已核实身份的两个后代后 | `-1 / ESRCH` | 新 PID 查询同为 `-1 / ESRCH` | 已观测两者退出事件 |

计数按内核 task 生命周期统计，不能将 started=5 解释为当时有五个活 PID。心跳窗口仅证明后代仍在运行，不能以窗口内无写入替代退出证明。

`bootout` 后再次 `kickstart` 同一个已移除 label 返回 113，明确为该服务不存在。该事实证明当前 job 定义已移除；不会被当作后代已死的证明。域外对照在根崩溃和 bootout 后均存活、持续心跳，直到最终清理阶段才对它单独发送身份绑定的 SIGTERM 并 `wait` 回收。

两个 job 的最终只读 `launchctl print` 均返回 113，精确指向本次 label 不存在。所有注册过退出监听的 helper 均收到真实退出事件，没有遗留本次 job 或持久 helper。

## `ESRCH` 与空列表的差别

以下是对固定 `ac9718fb` 实现的推导，适用前提为：同一系统启动周期中，已验证的非零专属 CID、准确 ABI、当前读取权限与查询路径，以及完整保留错误码。

1. `coalition_info` 先用 CID 查找对象；对象不存在才返回 `ESRCH`。该固定实现此处没有权限过滤；`resource_usage` 的内部失败另映射为 `EINVAL`、`ENOMEM` 或 `EIO`。[查找和错误路径](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/kern/sys_coalition.c#L253-L272)、[按 CID 查询](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/kern/sys_coalition.c#L397-L429)
2. 创建时保留调用者和全局列表两个引用，并递增分配 ID。查询表中的对象不会仅因调用者释放而先归零；ID 在该启动周期单调分配，不作普通回收复用。[创建和引用](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/osfmk/kern/coalition.c#L1329-L1344)
3. 源文件中唯一从 CID hash 移除对象的位置在 `coalition_reap_internal`：持域锁检查 `terminated`，确认 `active_count == 0`，置 `reaped`，才移除 hash 并释放列表引用。[完整回收条件](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/osfmk/kern/coalition.c#L2215-L2262)
4. 新成员关联和外部激活都在锁内拒绝 `terminated` 或 `reaped`。因此回收后不是“暂时没有成员、稍后还能加入”的状态。[激活守卫](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/osfmk/kern/coalition.c#L1466-L1485)、[关联守卫](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/osfmk/kern/coalition.c#L1954-L1982)

由此，**已知有效 CID 的后续精确 `ESRCH` 可以作为该固定内核模型下的已回收证明候选**。本轮实际读取的两个 API 都在清理前成功返回存活后代，清理后才返回 `ESRCH`，与该路径一致。不能把任意错误、未曾成功读到的 CID、换机/重启后的旧 CID 或一次空 PID 列表提升为相同结论。将来落盘回执至少必须关联系统启动身份、任务代次、原生会话和首次受验证的 CID；“同 label 服务不存在”还不足以恢复历史任务。

`bootout` 也没有封住存活后代的继续 fork；只有最终回收的内核条件才排除了该 CID 后续加入。原型没有直接接收 launchd 的 privileged 空域通知，使用的是查询已销毁对象这一只读观测路径。

## 枚举上限与未覆盖部分

旧 sysctl 的 100 PID 上限不能移用于新 API。新私有头定义上限 512，但固定 `coalition_get_pid_list` 内部只有 `MAX_SORTED_PIDS=80`，resource 路径也经过这个数组。[外层上限](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/sys/coalition_private.h#L56-L60)、[内部数组与 resource 分支](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/osfmk/kern/coalition.c#L2625-L2746)

本轮调用方容量为 100；私有固定夹具最多三个活成员，同时核对了所有角色身份和 task 计数。对较大的既有共享域，只读预检实际返回 80 个成员且不含查询者，进一步说明不能把“少于调用方容量”当作未截断。本轮没有对该共享域做任何信号或管理操作。

后续清理可以将枚举当作发现候选的途径，逐项核实身份并分批处理，但成功条件仍必须是独立 CID 的真实回收；不能因为列表重复、不再变化或某批为空而成功。达到期限仍未取得可靠回收证明时应失败关闭。

以下均未完成，不能计入 P4/P5 产品验收：

- 真实 Codex / Claude / Grok 进入该专属 job，并保持现有 stdio 协议与准确退出码。
- 独立 supervisor EOF、宿主崩溃、启动失败、重新关联和原子回执接线。
- 并发快速 fork、超过 80 个成员、身份失效/查询失败和权限变化的负向实测。
- 较老 macOS、当前实际内核同版源码保证、私有接口可发布性与最低系统支持。
- 外部 XPC 服务代执行或主动使用特权接口更换 resource coalition 的工作；本轮覆盖的是普通派生后代。

当前生产版的异常 Unix 根退出回执仍需保持保守拒绝。这里提供了可继续验证的专属域与回收观测方法，没有改变 [真实 Codex 残留/恢复阻止记录](DARWIN_PROCESS_CONTAINMENT.md)。本轮没有用户可见功能变更，无需本地化变更。

## 保存与复核

原型 C 源码和 Python 控制器只留在上面的两个私有临时目录，未加入产品或通用脚本。证据 JSON 保留两轮原始报告，不覆盖首轮 failure 或原始 `full_containment_proven=false`；外层十项观测检查全部通过，并独立记录 `production_containment_verified=false`。源码、helper、控制器、plist 和原始报告均有 SHA-256，固定上游文件也逐项记录 URL 和摘要。

第二轮固定 helper 源码 SHA-256 为 `a5e3d5dc6727153d887c9b81e4edc60c952c978f3518c3bd5525b03bbff7ccde`。复核已保存 JSON 可重做事实断言；再次执行控制器会注册一个新 job，不能把读取证据和重新运行原型混为一谈。
