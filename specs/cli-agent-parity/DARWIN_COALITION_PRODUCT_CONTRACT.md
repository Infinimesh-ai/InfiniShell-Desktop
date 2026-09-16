# macOS 资源域托管的产品接入契约

日期：2026-09-16。本文描述产品接入契约与分项验证；首轮真实新 worker 的八项夹具为四项通过、四项失败，不能视为整体完成。原始失败与成功同时保存在 [首轮真实监督记录](validation/macos-coalition-supervisor-live-1.json)。此前私有原型事实分别保存在 [专属域原型](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md) 和 [IPC 原型](DARWIN_COALITION_IPC_DESIGN.md)；旧进程组残留证据保持不变。

## 执行和清理边界

`ManagedChild` 仍持有真实域外 supervisor 的子进程和管道。macOS supervisor 使用本代次唯一的 `dev.infinishell.cli-agent.<generation>` 用户 job；普通 Command、Linux 子树与 Windows Job 的策略不变。不向 `~/Library/LaunchAgents` 写文件，不修改既有 job，不依赖 shell 展开或 `launchctl print` 文本。

执行分为三层：域外 supervisor；launchd job 内的持久 wrapper；wrapper 派生、等待现有 TCP 授权的 exec worker。最后一层经 `exec` 成为真实 CLI，wrapper 保留实际父子 `wait` 关系。`cli-agent-supervisor <manifest> --execute` 入口复用原参数；内部环境值的 `unix:` 前缀只选择 wrapper 路径，没有新增公开 CLI 开关。

1. 通过原有应用控制握手后，创建 0700 私有短路径 socket 目录、0600 socket 和 generation 目录内的 plist。job 的环境只有内部 IPC 路由，没有用户账号或环境快照。
2. 控制、双向 stdio、stderr 三个连接使用内核 `LOCAL_PEERTOKEN` 验证同一 EUID、PID 和 PID version；再核对稳定 unique ID 与 resource CID。
3. wrapper 必须属于不同于 supervisor 的非零 CID，且此时只有一个活动成员。首次 claim 完整落盘后，才能通过有界内存帧发送原始环境以及 wrapper 授权。
4. wrapper 用 `command` crate 派生等待授权的 exec worker。supervisor 核对其当前 PID / unique ID / CID，保存原生身份，再发送最终 CLI 执行字节。环境使用原始 `OsString` 字节传递，不写入 plist、claim、日志或 fixture；cwd 沿用原 manifest。
5. stdin EOF 使用 Unix socket 的写半关闭，stdout 读半部保持可用。wrapper 回传与 generation / 初始原生身份绑定的真实 wait 状态；不会用 wrapper 的自身退出码代替 CLI 退出码。域清理结束后，无论最先收到 NativeExit 还是 StdioClosed，都在同一个五秒期限内确认 stdout / stderr 两条转发结果，并显式 flush；转发失败独立返回错误，已经真实完成的清理证明仍保留。
6. 收尾先尝试 `bootout` 本次唯一服务，再使用 `proc_listallpids` 发现候选、逐 PID 读取稳定身份。移除命令失败也必须独立尝试已领取 CID 的清理，并汇总两个错误；域销毁不能把未确认移除的 job 变成成功。只有属于已确认 CID 的当前身份可以收到 audit-token 信号。进程在 `exec` 时 PID version 会改变，因此允许在 PID / unique ID / CID 全部相同时刷新信号版本；PID 复用拒绝。
7. 成员列表、无心跳和 `bootout` 成功都不是完成证明。只有首次有效的 CID 在同一 boot 下由资源查询返回精确 `ESRCH`，才记为该内核域已销毁；其他错误和超时拒绝确认。没有使用私有 PID-list 的 80 条内部截断。

动态接口通过显式 `/usr/lib/libproc.dylib` 和 `/usr/lib/libSystem.B.dylib` 一次性加载，避免测试宿主未链接某个库时错误依赖 `RTLD_DEFAULT`。ABI 长度、身份、专属域、用户域或符号不满足时，真实 CLI 仍未获得执行授权。尚未建立最低 macOS 发布版本或私有 API 的官方支持承诺；这仍属于平台验收边界。

注册前仅对本次随机 label 执行只读 `launchctl print`，要求精确不存在；现存或无法核对的同名 job 不会被删除。`bootstrap` 在调用前标记结果待确认，若进程超时或系统调用失败，则仍尝试撤销本次 label，保留原始与清理错误。明确非零拒绝注册的情况不认领 job。结果未知的启动分支没有首次 CID 证明，不生成成功回执；本轮尚未真实注入“launchd 已受理、launchctl 尚未回报”的窗口，不把静态修复算实测。

## 持久证明格式

所有文件均位于当前 SQLite scope 的 `state_dir/cli-agent-processes/<generation>/`。新资源域文件先写私有临时文件并刷盘，再以不覆盖既有文件的方式发布；完整或部分旧文件都不能被替换。

| 文件 | 关键字段与用途 |
|---|---|
| `manifest.json` | 原有 version / launch_allowed / generation / token / executable / arguments / cwd / parent_control；格式不变 |
| `macos-job.plist` | 唯一 label、固定 wrapper 入口和私有 IPC 路由；无用户环境快照 |
| `macos-coalition.json` | version=1、generation、manifest_sha256、label、boot_session、wrapper 身份；保存首次有效 CID |
| `macos-native.json` | generation、claim_sha256、identity；保存授权前 exec worker 的身份 |
| `macos-cleanup.json` | version=1、generation、claim_sha256、可选 native_sha256、job_removed、resource_cid_destroyed、可选 native_wait_status、execution_failed |
| `exit.json` | 原有结构，新 containment=`macos_resource_coalition`；准确 CLI exit_code 或未知，cleanup_confirmed 与原生结果分开 |

两个身份对象的字段均为 `pid: i32`、`pid_version: u32`、`unique_id: u64`、`resource_cid: u64`。生产只读查询入口为 `command::managed::macos_process_identity(pid)`；它在 CID 查询前后两次读取 unique ID / PID version，拒绝不稳定身份。真实边界测试应复用此函数，不能再依靠 CLI 向上回溯到应用 supervisor 的 PPID 链，因为 launchd wrapper 有独立父链。

恢复校验绑定 manifest 摘要、generation、首次 claim、native claim、退出状态和完整清理证明。旧 `unix_process_group` 回执不会升级成新类型。异常退出码可以有真实的完整清理证明；反过来，原生退出 0 也不能替代域销毁证明。

活跃清理对象必须匹配当前 boot，旧 boot 不得查询或发送信号。已经持久化的完整清理证明则只校验它与同代首次 claim 的关系，不查询旧 CID，因此不会仅因用户后来重启系统而失效。旧 boot 加缺失证明或 `cleanup_confirmed=false` 始终不能推断成功。

如果执行链失败但已经有首次 claim，且随后真实确认 CID 销毁，可以保存 `execution_failed=true`、`native_wait_status=null` 的清理证明。此时 worker 仍返回错误，原生结果保持未知；回执只证明可以安全考虑后续恢复，任务状态仍由 runtime/store 决定，绝不生成模型成功结果。

## 已准备的验证入口

定向 rustfmt、`git diff --check` 及统一编译已完成；固定 C 夹具先前的单独预检不能替代下方新 worker 分项结果。以下为保留的可复核运行入口。

非 ignored 回归覆盖共享 CID 拒绝、旧 boot/零 CID 拒绝、历史完成证明跨 boot、缺证明、错 CID、部分清理、真实 wait 与 exit code 不一致、PID 复用、环境非法字节、IPC 超长和既有证明不可覆盖。筛选路径：`managed::macos::tests`（command crate）及 `managed_process::macos::tests`（warp lib）。

真实测试需要 `INFINISHELL_CLI_SUPERVISOR_EXECUTABLE` 指向本次源码构建的主程序或 TUI worker，不能指向 libtest 自身或旧 bundle。对新 libtest 二进制分别运行：

```sh
env INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=/absolute/new/infinishell /absolute/new/warp-libtest 'managed_process::live_tests::supervised_' --ignored --nocapture --test-threads=1
env INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=/absolute/new/infinishell /absolute/new/warp-libtest 'managed_process::macos::live_tests::supervised_' --ignored --nocapture --test-threads=1
```

现有四个真实通用用例继续覆盖原生 exit 37、正常 finish、宿主 SIGKILL、脱离进程组后代与回执丢失，macOS 预期改为专属 CID 的完整证明。新增四例：

- `supervised_macos_stdin_eof_preserves_output_environment_and_exit_37`：真实 stdio 半关闭、中文多行、原始 PATH、stdout 与 stderr 各 2 MiB 加未换行尾部和准确原生 37。只在下层监督测试捕获 stderr，不改变生产 `ManagedChild` 接口或其默认丢弃诊断 stderr 的行为。
- `supervised_macos_cli_sigkill_cleans_more_than_eighty_detached_descendants`：固定 90 个后代混合 setsid / 双重 fork，根 CLI 身份绑定 SIGKILL，91 个受控进程真实 NOTE_EXIT、域外对照继续心跳，最终 CID 销毁。
- `supervised_macos_live_domain_zero_timeout_never_writes_success`：真实尚未授权的独占 job，零清理期限必须失败且没有成功证明，随后仅清理本次 job。
- `supervised_macos_job_removal_failure_still_stops_its_claimed_wrapper`：注入移除操作错误，使用真实成员查询和身份绑定信号停止未授权 wrapper，观察真实 NOTE_EXIT，仍保持 job 未移除和没有成功证明；最后正常移除本次服务。移除错误是明确的故障注入，不是一次真实 launchctl 拒绝的报告。

新的 C 夹具只在私有临时目录构建，所有持久进程自带 90 秒上限，测试宿主拥有的外部子进程均有 kill/wait guard。不能把心跳稳定代替域销毁；无模型夹具不会替代真实 Codex 工具、Claude 或 Grok 的完整生命周期验收。

## 首轮新 worker 实测：四项通过，四项失败

执行目录固定为 `.worktrees/cli-agent-parity-validation`，两组均使用 `--ignored --nocapture --test-threads=1`，没有 Cargo、模型请求或账号复制。源码基准为 `328d5ed35227f67884013f8c403e2a692470151d` 加冻结输入清单；清单 SHA-256 为 `3a17f35ef5d87108a3de79c0b815804bbf862dbdba8d301e82b91fcfa873c1f9`，18 个文件在运行前后完全一致。这是精确冻结快照验证，不能冒称尚未形成的同提交跨平台验收。

| 二进制 | SHA-256 |
|---|---|
| 新签名 worker | `7d71b20ada94a9da20294d52f51e03044d848159b8ffcc05fa7eb68baeaf06f0` |
| 新 libtest | `c30658e5f94dee4b99b49905f97fe8a2118fed9d98901262c803e2d6de8dda39` |

两者均位于独立 `/Volumes/ORICO/CargoTarget/InfiniShell-Desktop-cli-agent-parity-dbee1ecae/debug/` 目标目录，绝对路径与构建元数据摘要在 JSON 中。执行前后二进制摘要保持一致。

| 实际用例 | 结果与证据边界 |
|---|---|
| 通用脱离进程组后代 | 失败：15 秒内监督宿主与根心跳未同时就绪，未进入后代清理验收 |
| 通用正常 finish / 回执缺失 | 失败：同一就绪前置条件超时 |
| 通用宿主 SIGKILL | 失败：同一就绪前置条件超时，未执行该例宿主故障注入 |
| 通用原生 exit 37 | 失败：15 秒内替身宿主未结束，不能宣称退出码已验 |
| macOS 根 SIGKILL / 90 个后代 | 通过：固定 C 根身份绑定 SIGKILL；91 个受控进程 NOTE_EXIT、域外对照继续运行、CID 最终销毁与完整回执均由真实测试断言 |
| macOS 移除失败仍清理 wrapper | 通过：明确注入移除错误，真实信号及 NOTE_EXIT；job 未移除状态和无成功证明保持，随后正常收尾 |
| macOS 零清理期限 | 通过：真实活 CID 返回超时，未写成功证明，随后正常收尾 |
| macOS EOF / 两路大尾部 / exit 37 | 通过：两路各 2 MiB 加中文无末尾换行尾部完整匹配，真实 wait=37，完整清理证明通过 |

通用组原生运行耗时 60.25 秒，0 passed / 4 failed；现场为 `/private/tmp/infinishell-coalition-live-ix5h890u`。每代都捕获了 manifest、首次 coalition claim 与 native claim，但没有捕获完整 cleanup / exit 文件。运行中读取到的宿主 stderr 只有 i18n 初始化，不能据此推断没有原生错误。通用用例使用自动删除的 tempdir，panic 后目录消失；记录器只在自己的私有 TMPDIR 内机会性保存 JSON 文件，保留此收集限制，不把未捕获文件直接当作从未产生。失败原因目前尚未确定，没有修改或放宽测试门槛。

macOS 专属组耗时 4.66 秒，4 passed / 0 failed；保留目录为 `/private/tmp/infinishell-coalition-live-rpz805ig`。两个正向用例完整保存 manifest 摘要、claim、native claim、cleanup proof 和 receipt，摘要链均匹配；manifest 中的临时握手 token 已移除，只保留原始文件摘要。两个负向用例没有成功证明。C 源码、可执行文件、stderr 大尾部的摘要以及夹具自己报告的 PID 文件列表也保留在记录中；PID 文件不是退出证明。

两组共八个本次唯一 label 的最终只读 `launchctl print` 均返回 113。该事实仅表明 job 定义已不存在，不把它单独当作完整子树退出证明。JSON 顶层仍为 `passed=false`，并显式保留 `real_codex_exercised=false`。

## 同字节副本重验：通用四项通过，初始装载停滞仍未解释

独立结果见 [第二轮记录](validation/macos-coalition-supervisor-live-2.json)。没有修改产品、测试、超时、签名或系统权限；原 `live-1` 文件及其四项失败完整保留。将原 libtest 物理复制到私有本机路径 `/private/tmp/infinishell-coalition-libtest-v2vlq51l/warp-3ad688c9d0ab1400`，复制前后及测试后均核对相同 SHA-256 `c30658e5f94dee4b99b49905f97fe8a2118fed9d98901262c803e2d6de8dda39`。worker 仍使用上表原路径与摘要。

单例对照均限定 `fixture_native_exit`，不请求模型、不读取账号，正常退出的预期始终是原生 37：

| 对照 | 实际观察 |
|---|---|
| 原路径经完整生产监督链 | 3 秒内没有 stdout / stderr；对已核对身份的原生 PID 采样 2 秒，可见 dyld 映射文件时的 `open` 栈；关闭私有通道后完整清理，原生 37 未发生 |
| 本机同字节副本经完整链，首次 | 同样没有输出；采样仅得到 `_dyld_start`，尚无映像列表；清理完成，但不是正常退出 37 |
| 原路径与本机副本直接 Popen | 分别 0.070 / 0.128 秒准确退出 37，并产生 libtest / i18n 输出 |
| 同签名 exec worker，不经过 launchd | pipes 与 Unix sockets 两种 stdio 分别 0.165 / 0.178 秒准确退出 37 |
| 唯一私有 launchd job 直接启动 libtest | 正常输出，job 状态记录退出 37，随后撤销本次 label |
| 唯一私有 job 启动同签名 exec worker，再 exec libtest | 正常输出与退出 37，随后撤销本次 label |
| 同本机副本再次经完整生产链 | 约 0.45 秒完成；原生 wait=9472、exit_code=37、CID 销毁与完整摘要链一致 |

一次原生 job 对照最初把尚处于 `xpcproxy` 的过渡映像视为失败；该记录与清理保留。探针随后改为仅在实际 libtest 映像出现、身份稳定后认领，未向未知 PID 发信号。直接 launchd 对照会得到 launchd 自身的继承和默认环境，不能把它说成与产品 `env_clear` 后的环境完全相同。归档只保留必要的 job 状态行，完整环境值不写入仓库。

两次失败采样都不完整，不能确定系统根因或内核等待机制。后续增加的 `ps state/wchan` 只在超时分支执行，但那一次快速成功，因此没有取得旧失败的同期进程状态。`otool -L` 列出的依赖均在 `/System/Library` 或 `/usr/lib`；本机副本也曾失败，所以不能归因外置卷，更不能把 TCC 或缓存预热写成已确认根因。

最后用同一本机副本，在验证树以原筛选和 `--ignored --nocapture --test-threads=1` 运行通用四项：**4 passed / 0 failed / 0 ignored，5.61 秒**。正常 finish、宿主 SIGKILL、脱离组后代、准确 exit 37 与回执缺失保护均通过原始断言，四组完整 proof / receipt 的摘要链已归档。源码 18 文件及两个二进制摘要前后相同，四个本次 label 最终只读查询均为 113。该文件的 `passed=true` 仅指本机副本这四项验收；`initial_loader_stall_resolved=false` 保留首轮问题，未把成功重验称为修复。

## 尚未完成的边界

通用 libtest 替身组已有完整通过重验，但初始装载停滞的原因仍未确定；真实 Codex 工具两个 SIGKILL 分支、较老 macOS、同提交跨平台验证也不由本记录宣称通过。独立 supervisor 自身被 SIGKILL 后没有外部实体能补写可信证明，缺回执继续阻止恢复；本轮未增加“应用重启后只清理旧域”的新产品入口。外部 XPC 服务代执行或特权更换 resource coalition 不属于普通 fork 后代的隔离承诺。

本接入没有新增界面消息；错误继续经已有本地化启动/连接失败路径展示，无需新增本地化键。运行结果与恢复策略不得借此将未确认任务改为成功。
