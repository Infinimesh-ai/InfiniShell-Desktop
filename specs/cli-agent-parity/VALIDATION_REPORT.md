# CLI 对齐验证记录

状态：实施中，Goal 未完成。工作分支 `codex/cli-agent-parity`，基线 `6921a9925955a1955503e259cd935eaea4ac2ac0`；实现检查点 `dbee1ecae81da54a1749de88a7caffb3099d7eb7` 已推送，完整验收仍未完成。用户原有计划目录和另一任务网页搜索修改均保留。

## 当前提交验证

- 后续 Claude 同连接权限观测已在冻结源码通过 check、国际化 11 项及相关模块 987 项；[源摘要与门禁](validation/macos-claude-permission-observation-gates-1.json)保留 dirty 快照身份。错误/超时不阻断普通 Inherit 聊天，投影和原生执行规则不一致时拒绝作为派发依据；仍未开放 Claude 父任务派发，真实模型与原生 Rust 运行待验。
- [固定 Codex 完整运行包](CODEX_FIXED_RUNTIME_PREPARATION.md)已补齐。Linux x64、Windows x64/ARM64 的真实官方包均已在本机完成全树提取及复核，13 项离线回归通过；[记录](validation/macos-codex-complete-packages-1.json)不计异平台程序执行或真实工具通过。

- 第六轮 Windows 已确认两项失败：持久来源探针在缓存回退后没有完整确认输出/后台 Git 退出，清理异常曾掩盖首因；ConPTY 诊断 OSC 通过但真实原生通知匹配为 0。Grok 锁映射、真实 ACP、Claude 无凭据初始化/EOF 与 Codex 原生 hook 分别通过，Windows 本轮 Rust 跳过不计通过。Linux 全部所选门禁通过（warp 定向 1684 项），详见[第六轮专报](SIXTH_PLATFORM_RUN_7E0650855.md)。
- 新包 Codex 实际两轮中的拼音、图片、技能与评审传递已有证据，私有 Codex 测试副本仅包含主程序、漏装同版本 code-mode-host，实际工具调用报宿主不存在，因此文件读取与第三轮审批未通过，不归因于模型没有选择读文件。已从固定官方完整包恢复宿主、rg 与 zsh，并核对全部成员摘要和主程序版本，见[完整运行时输入](validation/macos-codex-fixed-complete-runtime-1.json)。尚需真实工具复验；其余生命周期正在继续。真实 Oz 测试已准备唯一 profile/安全存储服务，但系统访问提示需用户手动允许；电脑操作工具明确禁止操作 SecurityAgent，没有改 ACL 或默认钥匙串来绕过。GUI 验收仍未完成。

- 最新检查点 `7e06508554ae64cdd9321e0a69274e3d7b2d55ce` 已提交并推送；[第六轮 Actions](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35126330599)同时选择 Linux 和 Windows，完整工作区测试未开启；运行及四个 artifact 已全部回收，Linux passed、Windows failed。全部应用改动已与本地第三冻结快照逐字核对，见[来源关联](validation/macos-message-source-commit-7e0650855.json)，没有将旧测试二进制的提交元数据改写为新 SHA。干净验证树的[macOS 新包](validation/macos-build-7e0650855.json)构建与签名成功（232.035s），原生执行文件 SHA-256 `707b504dcde22686f6c9df1c9ee0563bfd773b6d4d12bc2d755116fd87ffc4f7`；独立副本的 GUI/Oz 与双语验收正在进行，构建成功不计完整交互通过。该提交随后在干净验证树通过 check（63.186s）、提供商 18 项、国际化 11 项和相关模块 965 项，见[同提交本地门禁](validation/macos-local-gates-7e0650855.json)。

- Windows 字节保留与 Oz 最终结果桥检查点 `94a412eb89a4de57977072e6c45f4a692955d970` 已推送，远端 SHA 已核对一致。[第五轮 Actions](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35122575386)仅选择 Windows，已结束且失败，Linux 按参数跳过。候选 Codex 启动器的原始字节、命令长度、PS5、原生 argv 和两组真实阻断 hook 均通过，模型请求 0；ConPTY 通知未验证。Grok 在 ACP 初始化前遇到锁读取权限错误，后续默认 Rust 门禁跳过，详见 [第五轮专报](FIFTH_PLATFORM_RUN_94A412EB.md)。新只读锁诊断和等待修复在本机通过 16 项、另 1 项 Windows 原生测试跳过；macOS 固定 Grok 无凭据协议边界也通过（8.653s），leader 仍需自持句柄强制收尾。[独立证据](validation/macos-grok-lock-reader-gates-1.json)不代替 Windows 复验。

- Oz 普通消息与提供商序列化第一冻结快照通过 check 和国际化 11 项；提供商新回归 10 通过、3 失败，消息相关 803 通过、1 失败。[首轮记录](validation/macos-oz-messages-local-gates-1.json)保留夹具错误和合法进度顺序缺口。第二快照因测试缺少 `RepairRecord` 导入而编译失败，0 项执行，见[第二轮记录](validation/macos-oz-messages-local-gates-2.json)。修正后的第三个 25 文件冻结快照通过 check（57.587s）、提供商回归 18 项、国际化 11 项和相关模块 965 项；[第三轮记录](validation/macos-oz-messages-local-gates-3.json)固定源文件摘要。范围与回执边界见 [消息交付说明](OZ_LOCAL_MESSAGE_DELIVERY.md)，真实模型、GUI 和最终同提交平台仍待验。

- 最新资源域与固定协议检查点 `6635f98690a6cbd3c117f9d313cb19de83016418` 已推送并核对远端相同 SHA。干净 macOS 验证树通过 cargo check（1m45）、国际化 11 项（4.60s）、相关模块 650 项（10.015s）、command 5 项及脚本 75 项；见[本地门禁](validation/macos-local-gates-6635f9869.json)和[脚本门禁](validation/macos-python-gates-6635f9869.json)。[新包](validation/macos-build-6635f9869.json)已完成构建、资源处理与签名（复跑 85.663s、退出 0）；首轮观察句柄失效且没有脚本退出确认的记录独立保留，未打开 GUI。[第四轮 Actions](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35117735097)核对相同 SHA，已结束：Linux 本轮全部通过，Windows 失败；Windows 已确认 Grok 路径夹具与候选启动器字节边界失败，前三处旧夹具修复已通过各自实跑，不能因此计为 Windows 平台通过。

- 插件事务与验证环境检查点 `328d5ed35227f67884013f8c403e2a692470151d` 已提交并推送；第三轮 [Actions 35110822335](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35110822335) 已核对相同 SHA。Linux 本轮已结束并通过：cargo check、warp 定向 1655 项、共享 CLI/技能、双语 TUI、4 项监督及两项原生 Codex 边界均成功；GUI integration/full workspace 按配置跳过。Windows 的新 Python 准备成功；测试环境键大小写、PowerShell 5.1 临时 argv 验证脚本和多行 Bash 夹具失败，后续 Rust 步骤跳过。三处夹具修复已在本机通过，尚未提交及原生复验；见 [Windows 专报](WINDOWS_VERIFICATION_FIXTURES.md) 和 [第三轮平台证据](THIRD_PLATFORM_RUN_328D5ED35.md)。本轮不计整体平台通过。

- 后续检查点 `c55385a69a4cd50364a6464dfbec1c965db2ac92` 已提交并推送，干净独立工作树使用显式专属 target 再次通过 cargo check（1m25）、国际化 11 项（4.53s）及相关模块 617 项；见 [同提交本地快照](validation/macos-local-gates-c55385a69.json)。
- 相同 SHA 的 [Actions 35106400715](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35106400715) 已完成但未通过：Linux Python 安装成功，传输夹具两项失败；Windows 下载/解压成功，Python 安装步骤失败。两平台 Rust 与原生 CLI 尚未执行；[失败快照](validation/cross-platform-c55385a69-35106400715.json)保留，不作为平台成功。Windows 作业私有 NuGet Python 准备脚本已通过本地静态验证，Linux 传输夹具已按解释器自身共享库目录修复、本机 9 项回归通过；两者已纳入 328d5ed352 并在第三轮分别越过旧失败；后续状态见上方第三轮记录。

- 干净 `.worktrees/cli-agent-parity-validation` 上的 `dbee1ecae` 已通过 `cargo check -p warp`（2m49）、Python 76 项（6.706s）和 Grok Node 11 项（235.97ms）；国际化 11 项也已通过（4.79s）；后续模块编译引用了另一工作树的旧 command 产物，退出 101、0 项执行。已创建专属 target 并清除其中的 76 个本地路径包；首次仅切换链接仍被上级 Cargo 配置覆盖而失败，随后显式指定并通过 metadata 核对实际目录，dbee1ecae 加历史选择框修复已通过 cargo check（2m28）、国际化 11 项（4.71s）、相关模块 617 项（8.722s，5803 项未在筛选内）。这是追加修复的提交前门禁，见 [提交快照](validation/macos-local-gates-dbee1ecae.json)。
- [Actions 35100708171](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35100708171) 核对为相同 SHA；Linux 的 Python 低于 3.11，Windows 无 `python` 命令，均在前置环境阶段失败。Rust/原生 CLI 未验收，证据文件未生成导致上传失败；见 [完整状态与失败分类](validation/cross-platform-dbee1ecae-35100708171.json)。workflow 已补固定 action 提交与 Python 3.13.15，静态检查通过、目标平台安装待验；保留本次失败，不重跑原样配置。
- 同提交的 [Codex 原生 SSH/tmux](fixtures/codex-0.147-native-ssh-tmux-macos-final-commit.json)（含最终 SetEnv）与 [Claude 无凭据初始化/EOF](fixtures/claude-2.1.273-no-credentials-initialize-eof-macos-same-commit.json)已分别实跑通过；不计模型、GUI 或完整生命周期。
- [bundle8 GUI](validation/macos-gui-bundle8-report.md) 已确认冷启动提示、当前真实通知受理和英文 Unknown；Unconfirmed 任务行/详情/继续禁用的双语布局通过。该包仍是 dirty 中间构建；中文 Unknown 新事件、英文历史选择框裁切修复及最终同提交 GUI 待验。首轮输入受拼音源影响，固定输出断言未通过，未追加模型重试。
- 历史选择框局部宽度修复已纳入 c55385a69 并通过本地门禁，复用原文案，无需本地化变更；双语布局由下一版包复核。Grok 同版本缓存完整性与显式修复、Claude 私有暂存安装与发布恢复及对应真实生产测试已纳入 328d5ed352，实际验证范围见下节。

## macOS 资源域接入的当前验证

328d5ed352 加冻结源码的独立验证树已通过 cargo check（62.881s）、国际化 11 项（4.55s）、运行时与信箱 173 项（2.998s）及 command 5 项（0.23s，另 3 项内部夹具忽略）。[门禁快照](validation/macos-coalition-local-gates-2.json)记录输入文件与测试二进制摘要；新 worker 已完整构建/签名（390.563s）。4 项专属 C 监督用例通过；通用 libtest 组首轮 4 项启动/等待失败完整保留，随后同字节本机副本重验 4 项通过（5.61s）。初次 dyld 加载停滞原因未确定，没有修改产品、签名或超时来取得重验结果。见[首轮记录](validation/macos-coalition-supervisor-live-1.json)及[重验与诊断](validation/macos-coalition-supervisor-live-2.json)。真实 Codex 缺失会话与空闲崩溃两项通过，身份与完整资源域证明已核对，见[边界记录](validation/macos-codex-coalition-boundaries-1.json)。

真实 Codex 运行固定工具时，强杀宿主（17.017s）与强杀 Codex（16.078s）两项均通过：分别观测到 6/6、5/5 个必需退出事件，同一系统启动内已知资源域查询返回 ESRCH，生产清理证明有效，回执后没有继续心跳。两项各使用一次模型请求、一次精确命令审批。这修复了此前原生工具自建进程组后残留的路径；探针使用生产 supervisor、自定义 Python 宿主和真实 Codex，不替代 Rust 适配器、GUI 或其他 CLI 的完整生命周期。见[两项实证与输入摘要](validation/macos-codex-coalition-tool-gates-1.json)。当前仍为冻结源码中间构建，最终同提交验证待完成。

本实现将真实 native wait 状态、任务结果和独占资源域销毁证明分别处理；stdin EOF 的两路输出尾部、bootout 失败仍清理已知域及 bootstrap 结果未知时的撤销路径均有回归入口。协议与未覆盖边界见[接入契约](DARWIN_COALITION_PRODUCT_CONTRACT.md)。没有新增主界面消息，复用已有本地化错误入口；技术诊断按 `app/i18n/PROGRESS.md` 的边界保留，无需本地化变更。最终双语布局仍单独验收。

## Oz 父任务结果桥与普通消息接入

新增 3 组生产 `ResultReady` 订阅及私有 SQLite 回归通过，桥模块 7 项（0.76s）、相关模块 653 项（9.071s）、国际化 11 项及 cargo check 通过。当前父结果仅一次入历史、重复和旧父代/用户轮被拒绝；关闭历史持久化后保持 Sent 未确认。首次测试编译的 Timer 类型错误及第二轮修正分别保留，见[冻结快照](validation/macos-oz-result-bridge-local-gates-2.json)。

进一步审计发现，BYOP 请求构造器会忽略历史和当前 `MessagesReceivedFromAgents`。所以既有 `application_history` 回执仅证明本地历史已提交，尚不能证明下一次请求已包含正文。现已接入请求序列化、既有 Responses 上下文指纹，以及 Oz 与托管 CLI 的普通消息授权/投递；合法工具调用期间到达的子任务进度会在相应工具结果之后进入请求，真实用户输入的顺序约束保持严格。第三个冻结快照已通过 18 项提供商请求回归、965 项相关测试、11 项国际化和 check，见[消息交付说明](OZ_LOCAL_MESSAGE_DELIVERY.md)。这些本地测试不能证明真实模型已读取或执行消息，CLI 父任务的原生双向消息证据保持独立。

## Grok 固定平台输入与无凭据 ACP

Linux/Windows 的 Grok 1.0.30 完整官方原生文件已固定大小和观测摘要，下载器及 10 项测试通过；本机没有执行异平台文件。新 ACP 探针复用显式私有 leader 与 stdio 两个自持进程，macOS 固定版本实测 initialize 成功、新建受认证阻断、缺失 load/resume 被明确拒绝。stdio EOF 自行退出；leader 等待 5 秒未退出，最终强制回收，不能计自然退出或运行中取消通过。[真实结果](validation/macos-grok-fixed-acp-1.json)及[输入记录](validation/macos-grok-fixed-acp-1-inputs.json)保留这些边界。

下载器、探针、离线回归和产物上传已接入 Linux/Windows workflow，actionlint 通过，尚未在新提交派发。没有认证或模型输入，未测量系统全部 HTTP 流量；没有据此开放 Grok 托管任务。协议限制见[专报](GROK_FIXED_ACP_BOUNDARIES.md)。

## 插件事务追加验证

冻结的 Grok 缓存完整性修复与 Claude 暂存发布代码已在隔离树通过 `cargo check -p warp`（1m47）、国际化 11 项和插件相关 163 项（2.021s）；见 [本地快照](validation/macos-plugin-transactions-local-gates.json)。新增实现复用现有插件提示，无需本地化变更。

Grok 1.0.30 的真实 Rust 生产入口已通过安装、同版本损坏修复、禁用后拒绝更新、文件事务失败回滚和再次更新恢复，1 项测试、27.17s，未提交模型输入。故障注入段仅为生产文件事务，不计完整 manager 强杀；见 [五阶段记录](validation/macos-grok-production-installer-1.json)。Claude 2.1.273 的真实升级、暂存修补失败和安装父进程强杀后重试也分别通过 1 项测试。升级直接执行完整 manager，修补失败验证生产文件事务及旧版本保持，强杀仅覆盖暂存 marketplace clone 阶段；旧 HTTP 请求阻塞不作为旧原生进程仍存活的证据。详见 [Claude 事务报告](CLAUDE_PLUGIN_UPGRADE_TRANSACTION.md)。此组是 c55385a69 加冻结代码的中间快照，最终同提交平台验收仍未完成。

## 已执行的中间验证

| 检查 | 结果与边界 |
| --- | --- |
| 基线/中间 `cargo check -p warp` | check20 已通过（2m26），包含 Stop 降级、授权冷缓存、受控路径校验及 Unconfirmed 状态/迁移/界面；见 [build23 快照](validation/macos-local-gates-build23.json)，尚非最终提交 |
| `cargo test -p warp --lib i18n::tests` | build23 11 passed（4.67s），包含结果待确认说明及本轮双语修改；尚非最终提交，GUI 布局另行验证 |
| Grok 随附插件 Node 测试 | 11 passed，包括真实 402 事件、双来源去重、旧事件、控制字符及 tmux 编码 |
| Grok 原生插件安装/升级/回滚 | 隔离配置中成功；确认原生 local plugin update 返回成功但可能不更新文件，管理器使用可验证事务与回滚 |
| Grok hooks 的控制 PTY | macOS PTY 捕获 OSC；不等于在 InfiniShell 中完整交互通过 |
| Codex 原生协议 | 新建、两轮、审批允许/拒绝、取消、同 ID 历史继续已有真实事件；实际 Rust 适配器与 macOS bundle4/5 GUI 已通过中间快照，其他平台及最终提交仍待验 |
| Claude 原生控制与错误 | initialize、queued/replay 输入确认、interrupt 控制确认、未登录错误及缺失 resume；成功模型生命周期未通过 |
| Claude 固定文件与无凭据原生控制 | 2.1.273 在 macOS 隔离环境通过无账号初始化及空闲 stdin EOF 后自行退出，退出码 0、无强制清理、无模型输入；固定 Linux/Windows 原生文件完整下载的大小、摘要及发布清单签名核验通过。获取和控制探针已在第三轮 Linux 原生通过，Windows 因前置夹具失败而跳过；不替代生产 Rust 适配器、PTY 或模型生命周期。见 [专门报告](CLAUDE_NO_CREDENTIALS_VERIFICATION.md) |
| Grok ACP | initialize、cached_token、新建、空会话跨进程 load/resume 已验证；两轮模型请求为 HTTP 402，不能计成功 |
| 无凭据原生缺失会话恢复 | build15 libtest + bundle5 真实监督 worker 通过精确随机会话 ID 的原生拒绝，只有同代 Disconnected、无原生替代会话、无模型输入；同时核验原生退出码 0 和清理回执。见 `validation/macos-codex-missing-session-supervised-1.ndjson`，不计为有历史恢复通过 |
| 无凭据原生空闲崩溃 | build16 libtest + bundle6 监督 worker 在真实 SessionReady 后精准终止 Codex，恰好同代 Ready/Disconnected，无模型命令及成功事件，恢复 gate 拒绝异常回执；macOS cleanup=false，不计完整清理。见 `CODEX_IDLE_CRASH_VERIFICATION.md` 与 `validation/macos-codex-idle-crash-supervised-1.ndjson` |
| 实际 Rust 适配器生命周期 | 第四次 `validation/macos-codex-adapter-live-4.ndjson` 全部通过：两轮、审批允许/拒绝、运行中追加并验证模型实际应用、原生取消、同会话进程重启与结果。前次测试器拒绝原生 shell 包装命令的失败证据保留；现在只接受精确匹配的可执行程序、参数、工作目录和预置测试脚本。监督版已在 `validation/macos-codex-supervised-lifecycle-1.ndjson` 重验通过；尚非 GUI/应用重启或最终提交验收 |
| GUI 图片与文件上下文 | bundle4 中文界面已从原生文件选择器附加真实 JPEG 和 README，收到原生接收后才清除草稿/附件，模型实际看图并读取文件后完成；扩展名误写 PNG 的 JPEG 被格式校验拒绝，未创建任务且草稿/附件完整保留。两次结果均作为正负证据保留 |
| 原生 Codex 插件负向生命周期 | macOS 0.147.0 完成缺失→安装→禁用→损坏来源重装失败→修复来源→恢复禁用→移除；失败时配置/缓存保持，显式恢复后配置逐字相同。原生重装会重新启用插件；不计为新版本升级、产品安装器或其他平台通过。见 `CODEX_PLUGIN_NATIVE_LIFECYCLE.md` |
| Claude 原生插件缓存审计 | 无模型隔离实测 2.1.273 / 插件 2.2.0 的重启、同版本更新和重复安装保留修补，禁用后 0 hooks 且保持 false；受控的非发布版本 2.2.1-fixture 更新会切换至未修补的新缓存，产品必须拒绝未知版本。60 秒未观察到后台自动更新，不计调度通过；见 `CLAUDE_PLUGIN_CACHE_AUDIT.md`，不替代 Rust GUI 或模型验收 |
| Codex 普通 PTY 插件重启回退 | bundle5 GUI 安装后 revision 3 的 10 个受控文件全部匹配，但真实重启原生 CLI 后四个修补文件还原为上游；工具栏退回更新提示。固定 0.147.0 的后台 marketplace 自动更新会强制重装缓存，固定 Git ref 仍触发，原方式未通过且当时暂停授权。后续持久来源及限定配置键迁入已实现；build16 两项配置事务测试因 TOML 格式正常化被误判而失败，修复后 build17 492 项全部通过。bundle7 GUI 更新后来源和缓存经 CLI/应用重启保持，五项 hook 已由原生界面逐项授权，重启不重复要求信任；GUI 富通知到达仍未确认。旧失败 [记录](validation/macos-gui-pty-codex-patch-reverted.json)和新 [bundle7 报告](validation/macos-gui-bundle7-plugin-report.md)分别保留 |
| 普通 PTY Stop 终态可信性 | 固定 Codex 0.147.0 源码及官方测试确认 Stop hook 可要求同一 turn 继续而无新 UserPromptSubmit；旧 Stop→Success 会过早锁定。Claude/Grok 契约同样不提供 handlers 聚合后的最终确认。三款接收端已改为保存响应并显示 Unknown，当前有效 ToolComplete 可恢复运行，审批/明确失败仍有效；保留旧事件保护、其他 CLI 语义及托管原生 Completed。build19 旧取消断言失败与 build20 相关测试撤下记录保留；修复后的相关回归在 build23 通过，后续 bundle8 已确认英文 Unknown 通知；中文新事件与最终同提交 GUI 待验。此前普通 PTY 绿态不计真实完成。见 [Stop 契约审计](STOP_HOOK_COMPLETION_AUDIT.md) |
| 已绑定 PTY 的 Unconfirmed 持久状态 | 兼容 PTY 子 pane 的未知结果保持当前原生会话与 generation 占用，不再误记 Disconnected，不生成最终结果；后续有效事件仍可更新，明确断线或应用启动恢复才断开，不重放消息。真实 SQLite/Diesel 的状态、会话唯一性、旧消息不重投与迁移回归在 build23 通过；独立 GUI 数据库副本的原任务/历史/消息三表均为空，其 SQL/索引夹具不能冒称有用户任务的升级或新 GUI 启动验收。托管原生 Completed 独立保留，见 [状态与迁移报告](UNCONFIRMED_TASK_STATE.md) |
| Codex 授权冷缓存与跨平台清单路径 | 原生授权显示已不依赖进程内运行时探测热缓存；按正常路径组件构造受控树清单键，拒绝根/前缀/父级/非 UTF-8 组件，并保留 Unix 文件名中的字面反斜杠。build23 本地回归通过，不改原生信任、不计 Windows 实跑；bundle8 冷启动已确认正确提示，当前原生通知受理后提示隐藏；Windows 与最终同提交 GUI 待验 |
| Codex 原生 hook 完整传输 | 独立 macOS 控制 PTY、固定 0.147.0、私有原生安装与精确测试授权下，未改 SessionStart/UserPromptSubmit 参考脚本输出完整通知；四项原生 HookRunSummary 为 completed，一项阻断为 stopped，实际捕获四条 OSC 共 870 字节。初始化/建线程后的等待未触发 hook；随后固定输入被原生 hook 阻断，模型 HTTP 请求为 0。原生注入 PLUGIN_ROOT，四个 hook 均可打开控制终端；不能将 GUI 问题推定为 runner 剥离 TTY。没有经过 GUI/SSH/tmux，也未验证成功 Stop，见 [专门报告](CODEX_HOOK_TRANSPORT_VERIFICATION.md) |
| Codex 原生 TUI 经 SSH/tmux 的 hook 传输 | 独立私有 HOME、固定 0.147.0 的真实原生 TUI 通过回环 SSH stdout，在 direct/on 各收到两条 OSC；off 两个 hook 已执行但零条 OSC，作为阻断负向。三模式各一次固定 argv 输入均被原生 UserPromptSubmit 阻断，session/turn 与原生界面对应，模型 HTTP 为 0、持有进程正常退出。没有完整 HookRunSummary，不计逐键输入、GUI 或成功模型生命周期；此行为在含最终 SetEnv 的干净 dbee1ecae 提交复跑通过，见 [专报与脱敏夹具](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md) |
| Codex 真实图片 | `validation/macos-codex-image-input.ndjson`：临时随机纯色图片使用中性文件名与不包含答案的提示，模型经实际 localImage 输入回答正确颜色并原生 Completed；监督版 `validation/macos-codex-supervised-image-1.ndjson` 再次通过（1 passed）。尚非 GUI 附件入口或其他平台验收 |
| Codex 动态工具保存恢复 | `validation/macos-codex-local-tools-restore-production.ndjson`：生产 connect 入口完成真实查询工具调用、关闭适配器进程、同原生 ID 恢复、再次调用查询工具和最终结果，监督版 `validation/macos-codex-supervised-tools-restore-1.ndjson` 再次通过（1 passed）。已据此解除 0.147.0 恢复工具门禁；不代表 GUI 或应用重启验收 |
| 相关模块多轮快照 | build11 `cli_agent` 466 passed、0 failed、10 ignored；build10 编译期间插件 revision 2→3 更新形成的资源不一致已通过冻结重建消除。权限、历史分页、消息 View 新回归通过；footer24、存储/信箱40、编辑器图片4、本地启动36、评审导入1均通过。ignored未运行不计通过，过滤有重叠不加总 |
| build16 相关模块门禁 | `cli_agent OR local_cli_mailbox` 共 489 项，487 passed、2 failed（6.524s）。失败均为新 Codex 来源配置事务，失败证据保留；修复后 build17 492 项全部通过（6.699s），新增缺失/空表、非法类型与内联表回归。真实跨进程邮箱故障注入已通过（build16 0.556s），验证 Sent 后强杀/同库重开不重投、旧 ACK 拒绝、新 ACK 接受及结果领取唯一，不冒充三方原生 ACK |
| build19 / build20 历史门禁 | build19 i18n 11 passed；CLI/邮箱相关 495 项中 494 passed、1 failed：`stop_event_disarms_pending_cancel` 旧 helper 把 Unknown 等同于取消状态。失败 [记录](validation/macos-local-gates-build19.json)保留，未保存该测试二进制摘要，不以其他构建代替。build20 只有 i18n 11 项通过（6.27s）；相关 nextest 在等待共享目标锁、编译前撤下，执行 0 项、退出码 130，[记录](validation/macos-local-gates-build20.json)不计模块通过 |
| build21 非 UTF-8 夹具失败 | i18n 11 项通过（4.74s）；相关 502 项中 501 passed、1 failed（6.798s）。`codex_hook_trust::tests::non_utf8_filename_cannot_enter_the_controlled_tree` 在 macOS 文件系统重命名时被 EILSEQ 拒绝，尚未进入产品校验。夹具现限定 Linux，目标平台仍待验，原 [失败记录](validation/macos-local-gates-build21.json)保留 |
| build22 / build23 最新本地门禁 | build22 i18n 11 项通过（4.52s），未保存二进制摘要；后续界面说明变化统一进入 build23。build23 相关 617/617 passed（8.590s）、i18n 11 passed（4.67s）、check20 通过（2m26）；617 项为明确筛选范围，5811 项跳过不计通过。见 [build22](validation/macos-local-gates-build22.json)、[build23](validation/macos-local-gates-build23.json)，均为 dirty 中间工作树 |
| 新项目技能发现 | build12 `cli_agent` 468 passed、1 failed、13 ignored；独立重跑同样失败。真实 GUI 也未显示新项目技能。已定位索引内容的真实路径与输入目录别名不一致，新增 canonical 索引和技能查询绑定，并保留原启动目录；build14 普通目录与符号链接真实索引回归 1 passed（0.20s），build15 完整 `cli_agent` 470 passed、0 failed、13 ignored，含真实 dispatcher 旧 listener 替换回归；bundle5 GUI 已实际刷新/选择技能并提交原生输入，见 `validation/macos-gui-bundle5-native-skill.ndjson` |
| TUI 编译与消息布局 | 首次完整构建暴露 6 条 CLI 文案未进入 TUI 独立编译期资源域，已新增中英文 TUI 消息键并更新调用；build4 已成功（3m14），真实 tmux 中可进入 TUI 主界面；第一轮消息行测试编译失败（缺 TuiBufferExt、颜色 Option 比较）已修，第二轮 9 项全部通过（0.264s），包含英文/简体中文 36/80 列真实 TuiBuffer 渲染与未确认消息的非成功符号/颜色断言。启动证据仅覆盖真实程序启动，不冒充运行中消息工具调用验收，见 `validation/macos-tui-startup-build4.json` |
| 进程管理底层 | `cargo test -p command --lib managed::tests::` 1 passed，3 个内部夹具默认忽略；bundle4 的 4 项真实监督测试通过（10.97s），新增退出码 37 验收。macOS 异常退出明确验证回执 false 及恢复拒绝，Linux/Windows 测试仍要求全树/Job 确认但尚未执行；此处通过的是已声明平台边界和负向保护测试，不能替代 macOS 真实工具完整清理 |
| 桌面打包 | `./script/run --dont-open --features local_cli_managed_tasks` bundle6 成功签名（2m31），源执行文件 SHA256 `eeb2486cc28e50f499ddf279fcabade4c3efea5783af54f98b090dd8b313557b`；中文静态图片能力提示已在无附件/ACK 后空草稿的真实界面复验，任务/消息计数未变，未触发模型。bundle3/4/5 的生命周期证据独立保留；bundle2 的早期初始化 panic 已修，失败栈保存在 `validation/macos-gui-bundle2-startup-panic.txt`。插件持久来源修复尚不在 bundle6 内 |
| bundle7 持久来源与六张双语布局 | 源执行文件 SHA256 `56e3ce6d304c7c2bb040222c8ca7ddd5ba7c978820c49765d2a757fbb8b48b68`；通过持久来源与五项原生授权的上述路径。1229×768 窗口、约 614 像素说明分屏中，授权/安装/更新的中英文共六张页面标题、正文、命令、复制控件及尾部提示均无截断或重叠。安装/更新页面为隔离布局入口，未执行这些页面的安装命令；不等于六条功能流程通过。GUI 富通知仍未确认，包内旧 Stop 成功语义不计可信完成。见 [报告与截图](validation/macos-gui-bundle7-plugin-report.md) |
| bundle8 构建与 GUI | 打包签名成功，退出码 0、构建 3m07，源执行文件 SHA256 `6f61a5d378dfd8e0cab94030ec3ef3d46e08ff3ec9029f2378229a2d7b3510e0`；见 [构建快照](validation/macos-gui-bundle8-build.json)。包含授权冷缓存、Stop/Unconfirmed 及后续说明修复，独立物理副本已确认当前通知、英文 Unknown 与 Unconfirmed 双语任务详情；固定输出断言因输入偏差未通过，英文历史选择框裁切修复及中文新事件待验。不计最终同 SHA 验收 |
| 配套插件与验证器脚本复验 | tmux 修补后通知事务 13、原生插件兼容 13、Unix PTY 4 项通过；此前 Python 全量 64 项通过（5.027s），gate5 扩展为 76 项通过（5.114s），修正 Claude 准备测试对 Windows SYSTEMROOT 的断言后 gate6 全量 76 项再次通过（5.142s）。Grok 插件 11 项通过；本机脚本测试不替代 Windows 原生执行或三方 CLI 场景 |
| Codex 原生 hook 路径 | `CODEX_PLUGIN_PATH_EVIDENCE.json` 三类特殊目录 SessionStart 通过；先按原生契约显式审核隔离fixture，未改用户trust。默认shell_snapshot的独立路径限制单列，完整产品通知仍待验 |
| 其他模块中间快照 | 本地存储/信箱 33、本地启动 31、编排 46、技能快捷菜单 15、操作执行 140、本地工具转换 4 项通过。过滤器覆盖可能重叠，不加总为独立测试数量；无匹配过滤器不计通过 |
| 共享任务与技能源契约 | `cargo nextest run -p ai --lib` 定向动作/结果/技能筛选 71 passed，220项不在本次筛选内 |
| CLI 参数完整回归 | `cargo test -p warp_cli --lib` 128 passed，包含隐藏监督 worker 两种入口及带空格中文路径参数 |
| 工作树检查 | 持续执行定向 rustfmt 与 `git diff --check`；不替代行为测试 |

监督前生命周期与图片证据对应 libtest SHA-256 `3b275295d4de15f4b45f13c0eadab5398e6fb95e4baba065cc96618a1a9930c3`。监督版三项成功证据对应 build11 libtest `4f099c906a96f6a3f2e9a43dc2756c4e63326b95f2bbe39c43ee41f455be954d` 与 bundle3 worker `17a2dd3d147072681a52acc7cadc00c1a4880552ff81ede6e8c6c0d5832a41b2`。元数据均明确标记 dirty 工作树，且随后还在修复异常退出回执与技能索引，不能替代最终同提交跨平台证据。本地历史门禁快照见 `validation/macos-local-gates-build11.json`，build15/check18/TUI tests2 与脚本检查记录见 `validation/macos-local-gates-build15.json`；build16 的通过和失败记录见 `validation/macos-local-gates-build16.json`，修复后 build17 的 492 项回归见 `validation/macos-local-gates-build17.json`，此前双语/check19 见 `validation/macos-local-gates-build18.json`；Stop 降级后的 build19 成败见 [记录](validation/macos-local-gates-build19.json)。后续 build20 撤下、build21 夹具失败和 build22/23 通过分别见对应快照。build23 测试二进制 SHA-256 为 `9c41a9b34d98108fe031a878bf0714b5b9f5dd2670ae41d26223bb2f3507ee73`。本轮测试及 bundle8 构建时工作树还含另一任务的网页搜索数字字符串修改；这两文件已排除本目标暂存并保留工作区，最终提交须在独立工作树重新验证。bundle7/8 与原生探针也只证明各自的 dirty 中间快照，bundle8 GUI 的通过项和未通过项见当前提交验证；最终同 SHA GUI 尚未验证。

历史进程监督失败及后续修复：独立控制连接断开后清理受管理进程并写退出回执，恢复在启动新进程前核对原运行代次；活跃任务仅重新关联已有协调器。Linux 使用子树回收，Windows 使用严格 Job。macOS 真实 Codex 自建工具进程组；宿主 SIGKILL 后 CLI 经 EOF 正常退出能清理工具，但直接 SIGKILL CLI 后工具继续运行，现有进程组监督仍错误写出清理确认。负向证据见 `fixtures/codex-0.147-supervised-cli-crash-macos.json`；这是正常原生工具路径的产品缺陷，不能归类为任意外部守护进程，也不能计入完整清理通过。bundle4 已实际将该回执写为 false，旧版异常 true 回执也由应用 gate 拒绝；恢复拒绝不等于已清理工具。新对照见 `fixtures/codex-0.147-supervised-cli-crash-macos-recovery-gated.json`，Darwin 系统能力与授权限制见 `DARWIN_PROCESS_CONTAINMENT.md`。前次 [coalition / launchd 只读审计](DARWIN_COALITION_LAUNCHD_AUDIT.md)之后，新增 [受控用户 job 原型](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md)已实测专属 CID 覆盖 setsid/双重 fork 后代，身份绑定清理后两个查询返回 ESRCH。根 SIGKILL 和 bootout 后仍有后代存活的负向同样保留。旧 sysctl 在 RELEASE 不可用；新私有查询有内部 80 成员上限，公开源码与本机内核版本不同。这些是当时的原型限制。后续产品已接入独占资源域，使用公开候选枚举后逐一核对身份，绕开 80 成员查询上限；真实 Codex 宿主/CLI 两种崩溃均已通过完整清理，见本报告开头的当前验证，旧失败证据保持不变。

## 完整验收矩阵

“待验/阻塞”均未通过。协议探测与应用内验收分别记录，不互相替代。

| 验收路径 | Codex | Claude | Grok |
| --- | --- | --- | --- |
| 新建、两轮与结果 | macOS GUI 已通过中间包 | 缺测试登录 | 额度耗尽 |
| 允许/拒绝审批 | macOS GUI 已验证允许文件存在、拒绝文件不存在 | 缺测试登录 | 真实权限协议未通过 |
| 运行中追加并验证模型应用 | macOS GUI 原生 Steer 接收与父→运行中子任务实际采用追加内容通过 | 缺测试登录 | 额度耗尽 |
| 取消真实当前回合 | macOS GUI 取消运行中的真实 sleep 回合，记录 cancelled；下一轮可完成 | 缺测试登录 | 运行中取消未通过 |
| 继续、应用重启、历史恢复 | macOS bundle4/5 GUI 已验证完成历史及活动任务正常退出后的同原生 ID 继续，未重复执行；资源域实现的真实 Codex 崩溃清理独立通过，最终同提交整链待验 | 缺测试登录 | 仅空历史已有实证 |
| 父子消息 ACK 与结果回收 | macOS GUI 生产协调器派发、子→父进度、父→运行中子追加、双向原生接收确认、结果回执与父查询回收通过；对应隔离 bundle4，中间证据见 `validation/macos-gui-parent-child-and-resume.json` | 无模型登录，真实父任务派发的权限上限尚不能证明，未通过 | 尚未开放执行 |
| 插件缺失/禁用/不兼容/失败恢复 | 原生隔离注册表生命周期部分通过；build16 两项事务失败已修，build17 回归通过；bundle7 GUI 持久来源及原生授权经重启保持，bundle8 已确认当前 GUI 富通知，完整负向组合与最终同提交 GUI 仍未通过 | 固定版本同版本保持及禁用、生产升级与暂存失败/父进程强杀后重试已有实证；完整故障组合与最终同提交平台验收仍待完成 | 真实生产安装、同版本修复、禁用拒绝、文件事务回滚及恢复已通过；其他平台和完整故障组合待验 |
| 崩溃、重复/旧事件、消息重投 | build15 旧 listener 的真实 dispatcher 回归通过；旧进程组清理失败保留，资源域实现的真实运行工具崩溃与空闲崩溃清理通过；跨进程 Sent/结果重投故障注入通过；最终同提交待验 | 共享状态回归通过，不替代原生模型进程崩溃验收 | 共享状态回归通过，不替代 ACP 执行验收 |

## 平台与布局

- macOS：本机编译和协议探测已执行；1229×768 的中英文新增 GUI 控件、消息和历史布局已实测可读，真实拼音组合、图片/文件/技能/评审已有证据；bundle6 静态能力提示已复验，bundle7 授权/安装/更新六张双语说明布局通过。独立 Codex 原生 hook、原生 TUI 经回环 SSH/tmux 传输与 Claude 无凭据初始化/EOF 各自通过，不替代完整三方流程；bundle8 已确认当前 GUI 富通知与英文 Unknown，中文新事件及英文历史选择框修复仍待复核。输入法候选列表选择作为补充覆盖尚未验证，已完成的逐键组合与空格提交证据独立保留。
- Linux x64 / Windows x64：已核对仓库对应 self-hosted runner 在线。前两轮因 Python 前置环境失败而未通过；第三轮已越过环境准备，Linux 本轮配置的所有步骤通过，GUI integration/full workspace 跳过，Windows 因上述三处夹具问题失败。预检已加入固定原生 CLI 获取、插件注册表失败恢复、持久来源重启对照、缺失会话恢复和证据上传；Claude 2.1.273 固定两平台文件已在 macOS 完整下载并核验字节，获取及无凭据原生初始化/EOF 探针在第三轮 Linux 已通过，Windows 仍待修复前置夹具后复验。下载、原生 probe 与产品安装器验收分别记录。
- Windows：PowerShell 参数转义、安装/派发路径统一与夹具已有实现；受控插件树已改为按正常路径组件生成清单键，本地回归通过不等于 Windows 实跑。非 UTF-8 文件名负向夹具因 macOS 文件系统前置拒绝，已限定 Linux，不能算 Windows 文件系统验证。预检已加入真实固定 Codex EXE 经中文/空格/`&` 路径中的 `.cmd` 包装、生产 Rust 适配器及监督 worker 的无模型缺失会话用例，尚未执行；此项不代替 NPM Node shim、ConPTY、多行和图片的独立实测。
- SSH：隔离 macOS 回环连接的远端 HOME/工作区/三款真实 CLI 版本查询通过；新增固定 Codex 原生 TUI 经实际 SSH stdout 收到真实 hook 通知，见 [原生专报](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)。含最终 SetEnv 的版本已在干净 dbee1ecae 提交复跑通过；完整产品 SSH 交互仍未验。
- tmux：既有构造通知的三方直连/on 到达与默认 off 阻断见 `SSH_TMUX_VERIFICATION.md`；新增真实 Codex 原生 TUI 在直连/on 各两条通知、off 零条且真实 hook 已执行，分别绑定原生 session/turn，模型 HTTP 为零。两种证据分开记录，原生探针没有完整 HookRunSummary，也不计逐键输入、GUI SSH 或完整模型生命周期。Claude 构造通知走兼容 TTY 分支，现代原生 JSON/UI 与完整 CLI 场景仍未验。

远端验证仅通过 `.github/workflows/cross-platform-preflight.yml`。先通过本地门禁，再推送实际修改提交并核对 Actions `headSha`；不能测试未包含实现的主分支来替代。

## 额外外部限制

Codex 0.147.0 默认 shell_snapshot 在包含引号与命令替换字符的 CODEX_HOME 中会错误解释路径，独立受控记录见 `validation/macos-codex-shell-snapshot-path-limitation.json`。上述特殊路径的通知正向验收仅在隔离测试子进程关闭该特性；未修改用户配置，不能据此声称默认特殊路径兼容通过。

## 下一轮必须记录

1. 最终本地命令、测试数量和结果；失败原因及修复后重跑结果。
2. Rust 适配器真实生命周期的脱敏证据与对应代码提交。
3. 两种语言的实际界面尺寸、截断/换行及截图。
4. 各平台 Actions URL、head SHA 和实际 CLI 交互证据。
5. 尚存外部限制与所有未通过验收项；只有全部完成后更新 Goal。

## 第七轮最终结果（2026-09-17）

`37b0bc73288aab5be4d0d151796eea557be4c0fb`：Linux 所选门禁成功，Windows 失败；完整计数、失败定位及跳过项见 [第七轮报告](SEVENTH_PLATFORM_RUN_37B0BC732.md)。同提交 macOS 完整 Codex 官方包已通过真实生命周期、本地工具恢复和图片三项验证，见 [原生适配器报告](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md)；它不替代 GUI 应用重启或最终三平台验收。

Claude API 鉴权补充：固定 2.1.273 / `claude-sonnet-4-6` 原生 print 请求在 2.607 秒返回 success 与精确标记，见 [脱敏报告](validation/macos-claude-api-smoke-1.json)。未启用工具，未运行托管适配器或 GUI，不计完整生命周期通过。
