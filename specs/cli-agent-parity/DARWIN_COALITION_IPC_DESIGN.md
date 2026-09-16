# macOS coalition 执行授权与 stdio 原型

日期：2026-09-16。本记录补充 [专属域原型](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md)，只验证私有临时 job 的 IPC 组合；尚未运行新产品 worker。原始结果见 [IPC 证据](fixtures/darwin-launchd-coalition-ipc-macos27.json)。

## 已实测的最小组合

在 `/private/tmp/infinishell-coalition-ipc-bzihxrfs` 创建固定 Python helper，使用唯一 `dev.infinishell.parity.coalition.c71a6e87-8e7c-4c48-aa5c-6af372c705f4` job。该 job 不启用 KeepAlive 或触发器；环境中没有复制账号。`bootstrap` / `kickstart` 成功后，wrapper 通过私有 Unix socket 建立控制、双向 stdio 和 stderr 三个连接。

本机 SDK 的 `sys/un.h` 明确提供 `SOL_LOCAL=0`、`LOCAL_PEERPID=2`、`LOCAL_PEERTOKEN=6`，地址最大 `sun_path[104]`。所有三个连接的内核 peer token 均指向 PID 92970 / PID version 598890 / 当前 EUID，且与进程身份查询一致；resource CID 为 23359，与域外控制器不同。没有解析 `launchctl print` 来取得进程身份。

wrapper 完成 32 字节 generation/token 握手后仍不能启动子进程。控制器先验证三通道身份和专属域，再经内存 IPC 发送环境与 cwd，最后单独发送执行授权字节。子进程 PID 92971 的 resource CID 同为 23359。

| 实测项 | 结果 |
|---|---|
| 传递中文、空格、`&` 的环境值 | 完整保持 |
| 非 UTF-8 环境值 | `value-\xff` 原始字节保持 |
| 中文、空格与 `&` 工作目录 | 精确匹配私有 cwd |
| 中文与多行 stdin | 精确字节匹配 |
| 对 stdin 执行 UnixStream `SHUT_WR` | 子进程收到 EOF，随后 stdout 仍完整返回 |
| stdout / stderr 分离 | stdout 312 字节，stderr 14 字节，内容均匹配 |
| 子进程退出码 | wrapper 真实 `wait` 得到 37，经控制通道关联 generation 回传 |
| 收尾 | wrapper 真实退出事件、`bootout` 返回 0、CID 查询 `ESRCH`、job 不存在均确认 |

只发送了三个合成无凭据变量。Python/系统运行时自行增加了 `LC_CTYPE` 和 `__CF_USER_TEXT_ENCODING`；本轮没有将“关心的输入变量保持”夸大为运行时环境集合完全相同。产品需将监督者的原始 `OsString` 环境通过内存帧传递，并使用 `env_clear` 后设置快照；不得把用户环境或账号写入 plist、manifest、日志和 fixture。

本轮只测试固定 helper 的退出 37 和 stdin EOF，没有宣称 CLI SIGKILL、宿主 SIGKILL、超过 80 成员、超时或新产品 worker 已通过。已保存证据的 `production_worker_exercised=false`。

## 计划接入的最窄边界

上层 `ManagedChild` 保留真实 supervisor 的 `ChildStdin`、`ChildStdout` 和 `Child`，三款适配器无需换 IO 类型。`managed_process::run_worker` 仅在 macOS 增加专用分支：域外 supervisor 管理唯一 job；job 中的 `--execute` wrapper 留作真实 CLI 的父进程，并经控制通道报告 CLI 的原始 wait 状态。不能把 launchd job 假装成 supervisor 可 `waitpid` 的 Child，也不能先普通 fork 再声称进程已经更换 CID。

拟复用现有隐藏 `cli-agent-supervisor <manifest> --execute` 入口；将仅用于路由的 Unix socket 路径放入 job 的内部控制环境。真实 CLI 获授权前，通过内存帧拿到原环境并移除内部路由变量。若该形式可沿用现有参数，不新增公开 CLI flags 或二进制。

执行顺序必须为：

1. 沿用现有独占 generation 目录、manifest/token、父应用控制握手；预检必需符号、系统启动身份和当前用户域。
2. 创建 0700 私有短路径 socket 目录，避免较长 SQLite scope 路径超过 Unix 地址长度。持久目录仍使用当前任务 scope，socket 不承载持久状态。
3. 创建只属于此 generation 的 job，wrapper 连接后核对内核 peer token、资源 CID、控制器在域外，以及执行前唯一成员；未通过不得发送 CLI 执行授权。
4. 将首次核验的 CID、系统启动身份、唯一 label、generation、manifest 摘要原子记录，再发送内存环境和最终授权。先启动后补记不满足恢复要求。
5. wrapper 通过现有 `command` crate 真正派生 CLI，保留父子 wait 关系。spawn 失败、CLI code、signal、wrapper 连接丢失分别传递；不能将 wrapper 自身的 0 当 CLI 成功。
6. stdout 通过 socket 转发；stdin 结束使用半关闭，避免 wrapper/监督者的额外 fd 克隆掩盖 EOF。宿主控制 EOF 由域外 supervisor 进入有界清理。
7. 移除本次 job，再按已核验的专属 CID 发现、逐项核对并清理成员，最终只接受此前有效 CID 的精确销毁证据。只读快照和等待时间不能生成成功回执。
8. 新 containment 名称与独立证明文件关联系统启动身份、CID、generation、manifest。原有 `unix_process_group` 回执仍按原规则验证，不能自动升级。旧 boot、错 CID、缺证明、未知错误或超时均拒绝恢复。

## 枚举是发现途径，销毁是成功条件

为避免把新私有 PID-list 的内部 80 成员截断误判为全部成员，可使用 SDK 已公开声明的 `proc_listallpids`，逐个读取候选 PID 的 unique ID / PID version / resource CID。只有同一稳定身份确属已核验的专属 CID，才能形成可发送信号的目标；复用 PID、身份改变、查询失败都不能直接当作已清理。

该方法仍是快照，不能独立证明完成。成员继续 fork 或单批遗漏时重复发现；最终由已知 CID 的回收证明收敛。即使某批读不到剩余成员，也必须保持未确认，直到精确 `ESRCH` 或超时。与新 PID-list 相比，这减少固定截断依赖，但其真实大规模分批行为仍须使用本次受控夹具证明。

`ManagedTree` 当前绑定可 wait 的真实 Child，macOS coalition 所有权应使用独立类型或独立实现模块；不要改变普通 Command、Linux 子树或 Windows Job 的行为。状态和成员查询中不输出无关系统 PID、环境或进程路径。

## 必须补齐的受控验收

| 分支 | 精确门槛 |
|---|---|
| 授权前失败 | 错 token / 同域 / 版本或接口缺失 / spawn 失败，均没有真实 CLI 执行标记；清理结果独立记录 |
| 正常与 EOF | 子进程 exit 37 被准确保留；中文多行输入、stdin EOF 后完整 stdout 保持；新 worker 真跑 |
| CLI SIGKILL | wrapper 回传原生 signal；另 SID / 双 fork 后代全部停止；CID 销毁后才可确认清理 |
| 宿主 SIGKILL | 域外 supervisor 通过应用控制 EOF 清理；结果不依赖应用 Drop 正常执行 |
| 超过 80 个成员 | 多批发现并身份绑定清理；域外对照一直存活；最终 CID 销毁，不能以某批空或不足 80 成功 |
| 清理超时或未知错误 | 不写成功回执、不允许恢复；保留残留/拒绝恢复的区别 |
| 恢复 | 同启动身份、首次 CID、代次和 manifest 都匹配；旧 boot、CID 替换、旧组回执不能通过新 gate |

若独立 supervisor 自身被强杀，job 内 wrapper 无法独自在自己的 CID 中等待“自身所在域已经销毁”。缺失外部监督者和完整回执必须阻止恢复；不能用 wrapper 的退出或任意超时冒充全域证明。后续是否需要由重启后的应用凭持久 claim 发起仅清理、不执行的恢复过程，应明确作为单独路径实现和验证。

本记录与原型没有产品界面变化，无需本地化变更；接入后的所有代码与新增实际验收仍由统一构建完成。
