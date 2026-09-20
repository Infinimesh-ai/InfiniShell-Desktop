# 三款 CLI 自动升级与渠道验证

## 当前结论

source43 的三款真实原生升级和安装替换拒绝已通过；source45 的渠道事务已通过 check、本地化 11 项、受影响 Rust 1979 项、Python 321 项、共享动作结果 5 项、TUI 编译及桌面构建。Grok 真实 Stable／Alpha 往返与同版本渠道同步通过；Codex 正反向产品更新均通过，独立正向包装器误拒官方别名的原失败与后续补充审计分别保留。source50 同源测试程序与签名监督程序已补验 Claude Latest／Stable 双向原生更新及同版本渠道同步；source56 又用官方 2.1.278 补齐固定策略恢复、父子派发、运行批量取消和待编辑审批取消，并用启用托管任务 feature 的签名 GUI 核对版本检出、固定审批入口和双语布局。source45 的元数据污染失败、source50 首次冷探测失败及 source56 两个失败边界均保留。source56 仍无 GUI 任务运行／重启和跨平台复验，最新版完整三方任务链与最终同提交三平台门槛仍未满足。以下保留各快照各自的证据边界。

## source42d 历史候选

source42d 的 `cargo check -p warp` 已通过（58.057 秒），本地化 11 项通过（140.649 秒），受影响 Rust 1903 项全部通过（28.847 秒，5318 项未选）。共享动作结果 5 项通过（20.612 秒），TUI 编译通过（185.707 秒）。桌面构建通过（296.387 秒），严格签名和完整中英文资源检查通过，测试程序身份未改变。真实产品更新链尚未通过；这不是最终同提交三平台通过。

当前冻结输入为 235 个路径，父提交 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`，清单 SHA-256：`6d67d82dec66e0d301820a216e8d3ab6acb041bd2986a9b8728f9b0a307c30f9`。171 个路径与父提交不同，全部仍是未提交候选；用户独立的 web runtime/search 修改未纳入冻结树。

## source43 后续候选

source43 将 Codex 0.155.1、Claude 2.1.278、Grok 1.0.34 的版本探测与本次协议握手严格配对，补齐 Codex 完整安装包准备器，并修正上述 macOS 验收环境误拒绝。冻结 237 个路径，175 个与父提交不同；源码清单摘要为 `f9a596871fde5059b5800f2edc6f1767b6b7cb23fe8537a4ab98f9d6ffe6db96`。

check 通过（66.520 秒）、本地化 11 项通过（159.252 秒）、受影响 Rust 1931 项全部通过（29.723 秒，5318 项未选）；Python 安装包准备器 44 项与更新驱动 14 项通过。共享动作结果 5 项通过（1.483 秒），TUI 编译通过（64.164 秒）。桌面构建通过（205.471 秒），严格签名、完整双语资源和测试程序身份检查通过。Grok 1.0.34 的固定权限、审批、取消及子任务能力仍未因版本匹配而开放。

三款真实产品升级均通过，每例实际调用生产检查两次、生产执行一次，无认证及模型输入：

| CLI | 升级 | 耗时 | 收据 |
| --- | --- | --- | --- |
| Codex | 0.147.0 → 0.155.1 | 115.950 秒 | [原生升级](validation/macos-official-43-codex-update-native1.safe.json) |
| Claude | 2.1.273 → 2.1.278 | 104.697 秒 | [原生升级](validation/macos-official-43-claude-update-native1.safe.json) |
| Grok | 1.0.30 → 1.0.34 | 98.921 秒 | [原生升级](validation/macos-official-43-grok-update-native1.safe.json) |

每例升级后入口匹配官方目标摘要，旧文件保留，13 项配置路径的字节、语义及权限保持，活动升级日志已清理。测试进程正常退出且进程组为空；驱动没有另行证明任意脱离进程组的后代清理，其收据仍明确为 `detached_descendants_verified: false`。证据绑定同一源码快照的测试程序与监督程序，不是最终同一提交、跨平台、实际渠道往返或应用任务链通过。

审计另发现任务管理器在会话就绪时仍写入旧版本常量；修复会进入下一候选，不改写 source43 的范围，source43 不能计作最新版完整托管链通过。Claude/Grok 最新版准备器也在下一候选中继续实施。

## source44 后续候选

冻结 243 个路径，185 个与父提交不同，35 个与 source43 不同；清单 SHA-256 为 `f9591fb3011b8bc1c10a63f342090a1ce7f07398478bc9d2770702eaa56e4398`。会话就绪事件仅在本次版本探测和协议握手匹配后携带真实版本；协调器持久化该版本及所属运行代次。Claude 首次控制握手早于系统初始化，尚无版本时保留历史记录而不误绑定新代次。

check 通过（68.158 秒），本地化 11 项通过（210.045 秒），受影响 Rust 1945 项通过（29.232 秒，5318 项未选），Python 11 组共 271 项通过。共享动作结果 5 项通过（2.008 秒），TUI 编译通过（70.519 秒）。桌面构建通过（217.706 秒），严格签名、完整双语资源与测试程序身份检查通过。Claude 2.1.278 基础适配器原生链通过（31.862 秒）：8 条输入覆盖中英文多行两轮、Write 允许与拒绝、运行中追加、流式取消和旧进程退出后同 ID 冷恢复。项目配置及 CLI 摘要保持，测试退出 0；[原生收据](validation/macos-official-44-claude-adapter1.safe.json)。它不覆盖固定权限、父子任务、真实 App 重启或 GUI。

Codex 0.155.1 另通过零认证、零模型的原生缺失会话（5.941 秒）和初始化后空闲崩溃（2.774 秒）验收，42 文件完整官方包在执行前后核对。它们分别覆盖恢复失败不另建会话、原生断连状态及 macOS 空闲进程清理，不覆盖运行中工具树或模型任务链。[缺失会话](validation/macos-official-44-codex-missing-session1.safe.json) · [空闲崩溃](validation/macos-official-44-codex-idle-crash1.safe.json)。

Codex 0.155.1 有模型基础链首例在第二轮失败（13.286 秒、实际 2 条原生输入）：两个回合均真实完成，第二轮返回 `PARITY_TWO.`，夹具要求无句号的精确标记。尚未执行后续审批、追加、取消或恢复，不能计为完整任务链通过。[失败收据](validation/macos-official-44-codex-lifecycle1-failed.safe.json)保持原状；下一候选只修正纯标记末尾标点的过度断言，协议身份、原生终态、权限与文件效果门槛不变。

执行前审计发现 Claude 父子验收运行器将平台变量覆盖为输出文件句柄，会导致尾部版本校验失败。该例没有发起 CLI 或模型输入，修复及回归测试进入下一候选；source44 仅先执行不受影响的基础适配器链。不能将该脚本的离线测试通过视为完整父子链通过。

另查明原生后台更新与应用渠道偏好的保持问题：Grok/Claude 原生配置恢复后可能仍按旧渠道更新；Codex 恢复旧 `auto-update-version` 字节会使标记不再匹配新版目录，改变原先开启的更新语义。渠道保持修复与真实往返验证尚未通过，三款单向升级记录不覆盖这一问题。

## source45 渠道保持候选

冻结 245 个文件，193 个与父提交不同，20 个与 source44 不同；源码清单摘要 `8f19bb52ac6b019618dd3fa79be5c05108d8be9cec9b0b54483c6334b955e2f5`。新增显式渠道成功事务、同版本配置同步、Codex 原生更新标记语义保持和按更新意图区分失败记录。默认跟随安装、失败恢复原字节、并发保存保护及启动零配置写入继续保留。

check 通过（101.743 秒），本地化 11 项通过（253.856 秒），受影响 Rust 1979 项全部通过（34.239 秒、5318 项未选），Python 12 组共 321 项通过。共享动作结果 5 项通过，TUI 编译通过（107.359 秒），桌面构建通过（275.097 秒）；严格签名校验与完整双语资源嵌入通过，测试程序摘要未变。主程序摘要为 `834c6921edcc74b0b94b5cbbd732e3dc00a925b5dede5f2db33a52ab1e7e37f9`，见 [构建记录](validation/macos-official-45-bundle.json) 和 [补充门禁](validation/macos-official-45-extra-gates.json)。真实原生渠道往返仍待验证。

Claude 派生运行器显式支持 2.1.278；父子运行器的平台变量覆盖已修正。但同快照的 [父子链](validation/macos-official-45-claude-coordinator1-failed.safe.json) 与 [受限文件链](validation/macos-official-45-claude-profile1-failed.safe.json) 均失败：生产预检仍只认可 2.1.273 的平台摘要，返回 `claude_profile_executable_unverified`，未发送任务输入。父子测试同时报告缺少退出回执；运行器结束后针对私有测试路径的进程检查未发现存活原生进程，不将这个额外检查冒充完整退出回执。批量和审批派生链尚未运行，需先核对新版控制接口再扩展兼容清单。

Codex 纯标记断言允许外侧空白及至多一个结尾句号，仍拒绝其他前后文、旧标记和非完成终态。[同候选 0.155.1 原生链](validation/macos-official-45-codex-lifecycle1.safe.json)已通过（48.613 秒）：两轮交互、允许／拒绝工具及实际文件效果、同回合追加、取消、重复消息 ID、关闭进程后同原生会话继续和最终结果回收。完整 42 文件官方包、源码／测试程序／监督程序的前后摘要一致。认证由已授权的正式运行器复制到本案临时目录，源文件身份未变；没有验证桌面应用重启、父子链或跨平台。

共享 Actions 已加入两平台的升级驱动离线测试，以及 Linux 的原始 Grok P0 POSIX 离线夹具；这些步骤不运行真实模型。actionlint 仅报告既有自托管标签，当前 GitHub runner 清单确认该标签存在；忽略这一已核对标签后通过。Linux/Windows runner 目前在线，不代表已运行 source45 跨平台门禁。

## source45 原生渠道与界面补充

Grok 的 Stable 1.0.34 → Alpha 1.0.38，以及从该真实输出复制的 Alpha → Stable 均通过产品检查／执行与独立配置审计；只修改 `cli.channel`，无认证、无模型输入。另从反向输出复制的同版本渠道不匹配案例通过：版本保持 1.0.34，渠道回到 stable，没有启动安装器或新监督代次。它不是下载安装通过。[正向](validation/macos-official-45-grok-stable-alpha-wrapper.receipt.safe.json) · [反向](validation/macos-official-45-grok-alpha-stable-wrapper.receipt.safe.json) · [同版本渠道同步](validation/macos-official-45-grok-channel-only-receipt.safe.json)。

Codex 0.155.1 → 0.156.0-alpha.7 的产品更新通过；独立包装器因新目录多出 `codex → bin/codex` 链接报告 `package_symlink`。补充审计将此精确别名与固定官方安装脚本对应，并核对全部 42 文件和 10 个目录；原失败收据不改写，也不把任意链接纳为合法。原生更新标记在正式切换 Alpha 后移除。[产品收据](validation/macos-official-45-codex-latest-alpha-receipt.safe.json) · [原包装器失败](validation/macos-official-45-codex-latest-alpha-wrapper.receipt.safe.json)。

从真实 Alpha 安装树创建的 Codex 独立反向案例 0.156.0-alpha.7 → 0.155.1 通过（产品 114.762 秒，包装器 119.356 秒），完整包只接受官方精确别名，所有配置保持，原先移除的后台更新标记没有被重新开启。[反向产品收据](validation/macos-official-45-codex-alpha-latest2-receipt.safe.json) · [反向包装器](validation/macos-official-45-codex-alpha-latest2-wrapper.receipt.safe.json) · [正向别名补充审计](validation/macos-official-45-codex-official-alias-audit.safe.json)。

后台观察与显式更新分开记录：Codex 原生 daemon 在启动约 302.931 秒后实际触发 updater → 官方安装脚本，未调用手动 daemon update，进程清理已核对；静默安装器没有提供成功回执，`periodic_check_succeeded` 保持 null，仅计定时检查启动。[后台收据](validation/macos-official-45-codex-background-baseline2.safe.json)。Grok Stable 裸 TUI 先生成所选 1.0.34 的新鲜后台缓存，正常退出后才运行手动 check 校验 stable／autoUpdate=true；原总结果因原生市场初始化字段变化为失败，独立审计保留这些分项通过，不能改写总结果。[原结果](validation/macos-official-45-grok-stable-background-failed.safe.json) · [独立审计](validation/macos-official-45-grok-stable-background-audit.safe.json)。Grok Alpha 也出现 1.0.38 新缓存，但夹具等待退出异常导致无完整退出回执，只有部分证据；后验进程扫描为空不能补成正常退出。[Alpha 失败](validation/macos-official-45-grok-alpha-background-failed.safe.json)。

Claude 2.1.278 → Stable 2.1.267 实际安装和版本校验成功，但完整配置审计失败：原生 `update` 在 `.claude/.claude.json` 增加首次启动及迁移元数据，超出预期的渠道字段变动。该例总体未通过，正在将更新过程配置隔离并保留组织更新约束；不能以最终版本正确掩盖配置副作用。[失败收据](validation/macos-official-45-claude-latest-stable-receipt.safe.json)。独立零网络、零认证对照中，这两个版本单独执行 `--version` 均未改变合成设置和元数据；此对照只排除了版本命令，不证明其他启动路径无副作用。[版本探测对照](validation/claude-native-version-only-278-267.safe.json)。

Claude 2.1.278 的原生只读预检另通过五个控制请求：两次 initialize 同 PID、default 空闲状态、合成权限规则与工具收窄、空 hooks 均核对，正常 EOF／退出和进程组清理通过。未发送模型输入，尚未通过固定权限或父子链；下一候选只在精确官方平台摘要和接口核对后扩展支持。[只读预检](validation/claude-278-raw-profile-preflight.safe.json)。

source45 独立签名 GUI 完成英文和简体中文说明、未安装／来源未知状态和 Grok 渠道菜单检查，1230 × 768 下所见控件无截断或重叠。三个自动升级开关以正确大写设置键预置关闭，首次启动确认，语言切换后正常重启再次确认保存；两次启动均已退出。成功安装、可升级和验证中状态布局仍未验证，也没有从此 GUI 执行原生更新。[4 张截图及收据](validation/gui-autoupdate-45/verification.safe.json)。

## source50 Claude 渠道隔离补充

2026-09-19T22:10:56Z 再次核对官方 native latest 端点与 npm latest 均为 2.1.278，Stable 为 2.1.267；两个固定 macOS arm64 文件分别为 217695408 与 200489184 字节，摘要和严格签名均匹配官方清单。source50 的 271 文件清单摘要为 `e1c87ad68ad83486b9d0b83bbad6840b8aa8c90fd89501ac731b5ce95171b0b0`，测试程序摘要 `c06e5bfe2c44e70d8ec9c004c4c52e6d2756b73f03195e0d3cd88b8b918c67ae`，签名监督程序摘要 `da269b5e1d28e33e3c036b3f461b0f6f2b4ebe486bb22e511631aa0a9f99be45`。

三个独立私有夹具均实际调用两次生产检查和一次生产执行，无凭据、无模型输入，13 条配置路径只允许声明的 `autoUpdatesChannel` 变化；其余路径字节、权限、旧版文件及目标参考文件均保持，更新 journal 清理，测试进程组退出：

| 用例 | 结果 | 耗时 | 收据 |
| --- | --- | ---: | --- |
| 2.1.278 Latest → Stable 2.1.267 | 通过，执行原生更新 | 107.858 秒 | [正向](validation/macos-official-50-claude-latest-stable.safe.json) |
| 2.1.267 Stable → Latest 2.1.278 | 通过，执行原生更新 | 100.977 秒 | [反向](validation/macos-official-50-claude-stable-latest.safe.json) |
| 2.1.278 Stable → Latest | 通过，仅同步渠道，入口和监督代次不变 | 105.523 秒 | [同版本同步](validation/macos-official-50-claude-channel-only.safe.json) |

首个 freshly downloaded 2.1.278 用例在 `inspect_before` 返回 `ProbeFailed`，未执行更新：该 217 MB 签名文件首次启动超过未放宽的三秒生产版本探测预算；同一文件只读预热后实测 0.018 秒且合成配置逐字节不变。该失败未重跑或改写，保存在[冷探测失败](validation/macos-official-50-claude-latest-stable-cold-probe-failed.safe.json)。汇总、同源绑定和 source50→source54 差异边界见[source55 安全记录](validation/macos-official-55-claude-channel-isolation.safe.json)。source50 到 source54 的自动升级实现逐字节一致，但 SSH bootstrap 与测试／证据文件另有变化，因此本组不能冒充整个 source54／55 的精确同源签名构建；也未证明脱离进程组的后代清理或 Linux／Windows 行为。

## source56 Claude 2.1.278 任务链补充

[安全收据](validation/macos-official-56-claude-278-native.safe.json)绑定官方 Claude Code 2.1.278、source56 测试程序和 source50 签名监督程序。固定策略恢复通过 7 次 profile 核对、允许／拒绝审批、同策略恢复及 3 种越权恢复拒绝。第一轮父子链暴露权限上限仍硬编码 2.1.273，导致 2.1.278 父任务在创建子任务前被拒绝并最终超时；source56 改为复用 Claude 适配器的唯一受支持版本表，修正后完成真实子任务创建、父子同策略、双向原生 ACK、3 组结果关联、自动结果和双清理回执。

运行批量取消首例的产品测试已经完整通过，但外层运行器误把并入既有批次的输入要求为 `turn_started=true`，该例总体仍保留失败。精确约束改为运行中／并入／继续分别为 `true/false/true` 后，28 项离线测试及第二次真实运行通过；2 个输入取消、1 个继续、interrupt ACK、原生取消终态、批次结果和清理均核对。待编辑审批取消也通过：编辑未放行、文件四次保持、原回合 `aborted_tools`、下一输入同会话完成。source56 同时通过受影响 Rust 模块、Python 28 项、i18n 11 项和 `cargo check -p warp`。真实 GUI 暴露旧说明仍只写 2.1.273，英文／简体中文同键同步补入 2.1.278 后，i18n 11 项和 check 再次通过。

这组任务链不改写 source50 的渠道升级结论。source56 GUI 第一次构建在 `warp` rlib 归档阶段因磁盘空间耗尽（errno 28）失败，旧 target 二进制保持不变；该失败原样保留。清理可重建缓存后，无 feature 构建通过但不含任务管理器入口，随后启用 `local_cli_managed_tasks` 的最终构建与 deep/strict 签名通过。签名 GUI 精确检出官方 Claude 2.1.278 与固定路径，中英文版本说明和固定审批按钮均无截断或重叠；没有发送模型请求或启动任务，因此不代表 GUI 任务、应用重启、持久化、Linux／Windows 或完整工作区通过。source56 仍未提交，未获提交／推送授权，因此未派发要求精确远端提交的跨平台 workflow。

## 本轮实现

| 能力 | 当前实现与边界 |
| --- | --- |
| 默认与渠道选择 | 新安装 Codex latest、Claude latest、Grok stable；已有安装默认跟随来源，支持各自的正式／预览或延迟稳定渠道 |
| 用户设置 | 每 CLI 独立自动升级开关、渠道持久化、检查及立即更新、当前／目标版本和来源显示；中英同步 |
| 执行保护 | 应用内普通终端与托管任务阻止并发升级；已派发安装阻止新 CLI 启动；原生退出前不因回合完成而释放 |
| 失败恢复 | 监督进程退出回执、代次和安装摘要共同核对；不重放未确认安装；失败目标持久抑制自动重试 |
| 配置保全 | Grok 暂存认领与无覆盖恢复；并发新配置保留；Claude 使用临时渠道设置 |
| 启动拒绝 | 明确 LaunchFailed，及时结束 waiter，GUI/SDK/BYOP 本地恢复如实呈现；旧远端 protobuf 续轮尚未验收 |
| 故障测试 | 渠道合法性、安装变化、锁、旧事件、更新失败、配置保存竞争、各恢复阶段及保留原文件 |
| 原生驱动 | 14 项离线测试通过；测试程序和监督程序须绑定同一成功源码，私有安装无认证、无模型输入 |

## 保留的失败

- source42：check 退出 101，新增结果泛型不一致及视图借用冲突；i18n 和后续门禁未执行。
- source42a：check 退出 101，登记 waiter 前终端锁尚未释放；后续门禁未执行。
- source42b：上述生产问题修正后 check 退出 0；测试库编译发现 AppContext 和 BlockId 测试夹具类型错误，本地化及后续门禁未执行。
- source42c：check 和本地化通过；1903 项中 1901 通过，两项启动保护夹具未注册 ToastStack，退出 100。
- source42d：只为上述两个测试补齐通知模型，check、本地化及受影响测试全部通过。不能把早期失败改记为成功。

输入、失败及后续收据保存在 `validation/macos-official-42*.json`。source41a 的 1834 项历史通过仍只属于原快照。

[最新版接口的独立核验](LATEST_PROTOCOL_INTERFACE_20260919.md)已完成版本、帮助或 schema／初始化；后续版本适配仍属于下一候选，不与本轮升级源码混用。

## 真实界面与原生更新入口

独立签名窗口完成英文及简体中文的三个渠道菜单、独立开关、搜索和重启恢复。Codex/Grok 选择 Alpha、Claude 选择 Stable，三个开关关闭后重启，所选值均保留；这里仅选择偏好，没有安装 Alpha 或点击更新。检查窗口为 1230 × 768，所验升级说明及下拉菜单没有截断或重叠。[11 张截图与安全记录](validation/gui-autoupdate-42d/verification.safe.json)绑定 source42d 的源码及签名程序摘要。

更正早期归因：source42d 最初预置使用小写 `codex/claude/grok`，而实际设置枚举要求 `Codex/Claude/Grok`。界面操作后这些大写键确实保存于该私有 profile 的 `settings.toml`；不能据最初开关未关闭推断 `settings_file` 未启用或系统偏好存储。原截图和原始记录保留，归因以此更正为准。检测器仍从系统路径发现 Claude/Codex，但更新器显示来源未识别；Grok 未安装，安装指南可见。此证据不替代成功安装状态布局、真实 IME 或升级执行验收。独立窗口已退出。

界面检查还发现未安装 Grok 时通用状态写着“此安装需要手动更新”，与具体未安装提示不一致。下一候选已同步将中英文通用状态改为“暂无法自动更新此 CLI”，具体原因和安装指南保持独立；这两条新文案尚不属于 source42d/43 的已验界面。

Grok 原生更新首例在环境验证入口退出 101，尚未进入生产检查或更新：macOS 框架自动加入 `__CF_USER_TEXT_ENCODING`，被测试白名单误拒绝。无认证私有进程的环境对照已定位原因；下一候选仅允许符合当前用户默认编码的这一平台变量。原始失败保留在 [失败收据](validation/macos-official-42d-grok-update-native1-failed.safe.json)，不能计为升级通过，也不能在同一用例上覆盖重跑。

## 待验收

- 三款产品原生单向升级与安装替换拒绝已按 source43 范围通过；Claude 双向 Latest／Stable 与同版本同步已按 source50 范围通过，Grok／Codex 渠道结果各保留原快照边界。后台渠道保持、其他失败恢复及成功安装状态布局仍待完成。双语渠道偏好与开关重启恢复已按 source42d 范围通过。
- WinGet、Windows Codex 原生安装、Claude npm 企业版本策略等未证实来源明确降级；Unix 原生夹具不计 Windows 通过。
- 最新版协议、插件、完整任务链、最终同提交 macOS/Linux/Windows 完整门禁与单独 SSH/tmux。

全部门槛满足前 Goal 保持 active。
