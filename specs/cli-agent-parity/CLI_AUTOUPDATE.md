# 三方 CLI 自动升级与最新版对齐

## 范围与当前状态

2026-09-23 13:37 UTC 最新复核见 [receipt132](validation/version-discovery-132-20260923-latest-drift.safe.json)：Codex 官方稳定版已发布 `0.156.1`，Claude 官方 latest 为 `2.1.280`，Grok stable 隔离查询仍为 `1.0.41`。下文 `0.155.1`／`2.1.278` 版本与渠道表保留为 2026-09-21 历史快照；新版 Codex 仅通过零输入协议预检，新版 Claude 尚无产品实链，最终自动升级与 P0–P5 不得沿用旧版结论。

2026-09-19 用户明确要求在当前 P0–P5 Goal 内增加 Codex CLI、Claude Code、Grok Build 自动升级，消费者与开发环境均跟进最新版。历史 source43–56 的真实升级、渠道与 Claude 链按各自快照保留；当前工作树进一步实现结构化 LaunchBinding、binding digest、事务 sidecar，以及 macOS 签名私有快照、Linux sealed memfd＋`execveat`、Windows replacement lease／挂起启动。macOS 快照已修复扩展属性逐字节镜像，当前同源产品用例真实完成 Claude `2.1.267`→`2.1.278` 与 Grok `1.0.34`→`1.0.40`；两者均只调用一次 execute、没有模型输入或凭据、配置及旧／参考二进制保持。此前不安全祖先拒绝、xattr 失败和 Grok inspect 网络失败继续保留。Codex 当前产品目标、Linux 实际执行、Windows 真实依赖闭包、三款渠道切换及事务恢复仍未全部通过，因此消费者自动升级总体未完成。当前边界见 [receipt100](validation/macos-working-tree-100-auth-autoupdate-live.safe.json) 与 [receipt99](validation/macos-working-tree-99-atomic-update-current.safe.json)，Runtime Host 与来源门禁见 [receipt98](validation/macos-working-tree-98-current-runtime-binding-gates.safe.json)，历史候选边界见[原验证记录](OFFICIAL_42_AUTOUPDATE_VERIFICATION.md)。

2026-09-21 Windows 实机补验进一步证明当前挂起调试候选不能发布：cwd 句柄不能阻止叶目录改名，调试事件链不能可靠退出，Claude／Grok 在 debugger 被终止后曾保留根进程，三款真实 updater 参数均没有原生退出收据。因此 Windows 产品入口与三款来源继续在事务副作用前 `ManualOnly`，详见 [receipt104](validation/windows-working-tree-104-atomic-real-cli-failclosed.safe.json)。[receipt105](validation/macos-working-tree-105-codex-01551-windows-asset-correction.safe.json) 后续确认 receipt104 的 Codex `0.155.1` 资产漂移是误与 legacy `0.147.0` 清单比较；版本选择后的仓内清单与当前 GitHub API digest 一致，无需改产品摘要。这一纠正不改变 cwd 路径绑定、debug tree 终止、Job 清理和原生退出收据的实际缺口；这些条件完成前仍不能开放。

2026-09-23 补核：[receipt109](validation/linux-atomic-execveat-109-c25221a22.safe.json) 从已成功的 run 35746148046 Linux job 原始日志确认，密封 memfd `execveat` 的实际执行及失败不回退 pathname 均已通过。以下旧阶段“Linux 需要执行实际 `execveat` 用例”只适用于当时快照；现在仍缺的是 Codex／Claude／Grok 的 Linux 产品升级事务、退出回执和恢复。

原固定 Codex 0.147.0、Claude 2.1.273、Grok 1.0.30 的验收文件保持原样。自动更新不能覆盖已冻结二进制或把历史通过追认为新版通过。最终验收要记录当时最新正式发行版本、平台、二进制摘要、实际源码提交和插件版本。

## 更新约定

- 新安装默认选择官方正式发行：Codex `latest`、Claude `latest`、Grok `stable`。官方默认不代表有用户占比统计，不宣称已确认最常用渠道。
- 已有安装默认“跟随当前安装”，独立保存三款 CLI 的用户偏好。允许手动切换：Codex `latest`／`alpha`，Claude `latest`／`stable`，Grok `stable`／`alpha`。默认不会将正式版切到预发布；已有预发布安装不会被默默转回正式版。
- Claude 的 `stable` 通常延迟接收更新，不能与 `latest` 混淆；Codex/Grok 的 `alpha` 为预发布。目标版本按所选渠道解析，显式切换可能安装较低版本，不能以版本号更大为唯一成功条件。
- Codex 当前原生 `update` 只选正式版，跟进 `alpha` 必须由应用解析可信渠道元数据并执行匹配安装来源的目标版本命令。Claude 临时 `--settings` 只控制本次更新，不能独自保证之后仍用所选渠道。显式选择渠道后的成功更新事务只同步 Claude 的 `autoUpdatesChannel` 或 Grok 的 `cli.channel`，失败恢复原值；跟随安装时不修改其渠道。启动 CLI 不写全局配置。
- 同版本也核对原生渠道：配置仍指向其他渠道时生成只同步配置的事务，等待会话关闭，并遵守应用自动更新开关；该事务不运行安装器、不伪造原生退出回执。
- Codex 的原生更新标记只有与当前正式发行目录精确匹配时才表示已开启；升级到正式版时保留该开启语义，原关闭或无效状态不被意外激活。切到 Alpha 后保持官方安装器移除标记的结果，随后切回正式版不会默默重开原生 daemon 更新。InfiniShell 自己的自动更新偏好不受影响。
- Homebrew 等来源只能使用其实际支持的发行包；无法安全切换渠道时明确显示限制及官方安装入口，不隐式卸载或改用第二套安装。
- 应用提供自动检查、更新状态、开关与立即检查入口。缺失 CLI 显示安装入口，不能把缺失记为已更新。
- 更新前识别实际可执行路径及安装来源；官方原生安装、npm、Homebrew、WinGet 和自定义入口不能混用更新命令。无写权限、企业策略或无法可靠识别来源时，明确说明原因并给出可执行的下一步。
- 保留活跃普通终端及托管任务使用的运行版本。应用管理的自动升级在对应 CLI 空闲时切换；不得中断审批、运行中追加指令、取消或结果回收，不为升级降低任务权限。
- CLI 更新与通知／编排插件更新分别核验。更新后重新读取版本、插件完整性与真实协议能力；检测到新版本不等于托管任务或审批已经兼容。
- 使用官方发行来源和摘要／签名校验；更新失败时保留可用版本和任务历史，记录失败原因并允许重试。用户全局信任、登录与审批配置不属于更新迁移范围。
- 更新任务本身需要防重复、超时与失败恢复；应用重启不能重复运行未确认的升级事务，也不能把仅下载完成显示成更新成功。
- SSH／tmux 使用远端实际版本和安装来源，不能拿本机升级状态代替远端结果。

## 已核验官方接口

| CLI | 实时发现 | 官方更新入口与区别 | 当前证据边界 |
| --- | --- | --- | --- |
| Codex | 2026-09-21 最新复核：正式版 0.155.1；npm alpha 0.156.0-alpha.12 | [官方 CLI 文档](https://developers.openai.com/codex/cli/)与当前源码区分独立安装、npm 和 Homebrew；原生 update 无渠道参数 | 正式完整包、全部 Mach-O 签名及帮助通过；历史渠道往返使用 alpha.7，当前 alpha.12 尚未运行，不能沿用旧收据；详见[证据](CLI_AUTOUPDATE_CODEX_CLAUDE_EVIDENCE.md)与 receipt98 |
| Claude | latest 2.1.278；stable 2.1.267 | [官方设置文档](https://code.claude.com/docs/en/setup)区分原生后台更新、update、npm、Homebrew 与 WinGet | 官方清单／摘要／签名及 latest↔stable 临时覆盖更新实测通过；详见[证据](CLI_AUTOUPDATE_CODEX_CLAUDE_EVIDENCE.md) |
| Grok | 2026-09-21 后续复核：stable 1.0.40 | [官方 CLI 参考](https://docs.x.ai/build/cli/reference)列出 update、--check、--version、--stable 与 --alpha | 沿既有 internal stable 从 1.0.34 实际更新到 1.0.40；更新后 JSON 再确认 `channel=stable`、`updateAvailable=false`，OAuth 仍有效。较早 alpha／stable 发现收据保留其时间点，不改写为当时已运行；详见[证据](CLI_AUTOUPDATE_GROK_EVIDENCE.md)与 receipt100 |

本机开发环境中，Codex 已在确认无活动目标进程后沿现有 Homebrew latest 来源从 `0.154.0` 升级为 `0.155.1`，新二进制版本、摘要和严格签名见 [receipt89](validation/macos-official-89-codex-01551-development-upgrade.safe.json)。Grok 后续已沿现有 internal stable 从 `1.0.34` 更新到 `1.0.40`，严格签名通过，OAuth 与模型查询仍有效；Claude 的现有 Homebrew `claude-code` stable 保持 `2.1.267`，`claude update` 明确报告该渠道已是最新，`2.1.278` 属于 `claude-code@latest`。开发环境版本检查和独立 CLI 认证不能替代 InfiniShell 产品事务验收。

## 加入 P0–P5 的验收

1. P0：保存三方最新版本的官方接口、完整运行包、摘要及原生帮助；更新开发准备器与兼容性矩阵。
2. P1／P2：安装发现和消费者入口显示版本、更新状态与自动升级设置；插件在新版 CLI 上完成缺失、升级、禁用及回滚验证；英文与简体中文同步。
3. P3／P4：升级不能破坏当前运行、审批、父子任务、消息 ACK 或结果回收；更新后新启动与冷继续使用正确版本，历史任务不能自动重放。
4. P5：同一实际提交覆盖 macOS、Linux、Windows，单独 SSH／tmux；三款最新版分别完成原完整验收链。
5. 渠道门禁：三款默认与保留现状、手动切换、目标降级、不支持的来源／渠道组合、设置持久化，以及应用重启后仍跟进所选渠道。
6. 新增故障门禁：并发检查、运行中推迟切换、网络失败、下载或摘要错误、更新进程失败、无权限、未知安装来源、版本未变化、插件不兼容、应用重启后的更新事务恢复和保留可用旧版。

## 产品实现的当前验证范围

- 来源发现已直接构造结构化 artifact 角色、参数／环境引用及 LaunchBinding，不再扫描字符串猜测目标路径。binding digest 同 generation 和 manifest 一起写入 schema 2 journal、`launch-binding.json` 与 `exit-binding.json`；旧 schema、缺失或不匹配 digest 均拒绝恢复。
- update 专用监督入口在看到不支持的 binding 时，会在创建 generation、manifest 或进程之前拒绝。当前三款 native 来源在 macOS／Linux 使用 `NativeFile`，macOS 通过签名私有 generation 快照执行并在复制后精确镜像源扩展属性，Linux 通过 sealed memfd＋`execveat` 执行；Windows native 因非空 PE 导入闭包尚未绑定而返回 `ManualOnly`。npm、Homebrew 及未知 helper 闭包仍可靠降级，WinGet 仍不可识别。macOS atomic 15／15、实际签名快照 1／1、Claude 与 Grok 产品升级各 1／1、Python verifier 27／27、i18n 11／11、启用 feature 的 `cargo check -p warp` 与主二进制 build 通过。
- 上述关闭了 macOS 的 pathname 复核后替换窗口，并证明 Claude、Grok 两个精确版本对修改的是原安装目标而不是快照自身，但仍不满足自动升级总完成门槛。Linux 需要在 Linux 主机执行实际 `execveat` 用例；Windows 需要对真实 CLI 的 PE 依赖闭包给出可验证绑定；Codex 仍需当前源码产品更新收据，三款还需渠道切换、升级失败与应用重启事务恢复。脚本、动态库、npm、helper 依赖闭包未知时继续 fail closed。

- 三款设置分别持久化自动升级开关和渠道；安装扫描未命中不会重置用户选择。设置页使用同一组件并保留各 CLI 实际渠道。
- 更新命令接入既有进程监督器。事务记录绑定同次执行代次、原安装摘要、目标版本及实际 manager program／参数／cwd；恢复先检查真实退出回执，再执行版本探测和配置核对，不重放安装命令。只有 journal 处于 `prepared` 或合法 Claude 准备阶段、持久 launch 合同有效且监督代次目录尚未建立时，才在 journal 锁内持久认领精确 `not_started` 后恢复；旧 journal、畸形 Claude 阶段及 `command_returned` 等后期缺账本均保持人工恢复而不伪造未启动。监督 manifest 之后、OS spawn 之前先持久化 `spawn-attempt`；只有显式 OS 拒绝才能写 `spawn-rejected`，不再把进程尚未观察到当作确定未启动。
- Grok 配置恢复先持久化同目录暂存身份，再原子认领文件，核对后无覆盖地恢复；并发用户保存发生冲突时保留两份文件并报告需要恢复。
- 失败记录按有效渠道、精确版本和配置目标摘要区分意图，抑制相同更新的后台重投；更换渠道不被旧意图阻止，显式“立即更新”仍可重试。启动入口与升级模型共享本地会话保留状态，确定进程结束后才释放。
- 命令被升级保护拒绝启动时，executor 返回 LaunchFailed，GUI、SDK 及本地 BYOP 记录如实显示失败；旧远端 protobuf 没有对应失败承载，该远端续轮仍未验收。
- 当前原生升级夹具限定 Unix，Windows 离线边界不能代替 Windows 原生安装验收；WinGet、Windows Codex 原生更新及 Claude npm 的企业版本策略尚未开放为已验证产品路径。
- 普通 managed process 仍可按既有合同启动任务；update 专用入口已禁止把复核后的 pathname 执行计作安全升级。脚本及 Node／Homebrew 运行时读取的依赖闭包尚未钉住。`/dev/fd` 会改变脚本入口与 `$0`，不能作为通用修复；完成门槛仍需平台专用绑定执行或可信私有快照，并做真实升级故障验证。
- 身份捕获本身使用一次 no-follow 打开，并在同一 handle 上取得平台 file ID、大小与 SHA-256，摘要前后复核 handle 元数据。LaunchBinding 与三平台原子实现均已编译到当前源码合同；macOS 已执行 Claude／Grok 两款真实产品更新，Linux／Windows 与 Codex 当前产品更新仍未完成。Windows x86_64／aarch64 MSVC 和 Linux x86_64／aarch64 musl 的临时 harness 只证明条件编译，不是实际执行证据。

自动升级及最新版三方链未全部通过前，Goal 保持进行中。

身份复核到真实执行的跨平台闭环及可靠降级规则见 [绑定执行计划](AUTOUPDATE_ATOMIC_EXECUTION_PLAN.md)。
