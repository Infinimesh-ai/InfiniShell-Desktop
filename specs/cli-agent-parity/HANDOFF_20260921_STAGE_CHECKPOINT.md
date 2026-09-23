# CLI Agent Parity 阶段检查点交接（更新至 2026-09-23）

## 1. 阶段结论

- 总 Goal 仍为 **active / 未完成**。本轮仅冻结当前实现、真实验证结果和失败收据，不得据此恢复此前“全部完成”的结论。
- 2026-09-23 历史失败：[receipt115](validation/cross-platform-preflight-115-645e8b4f.safe.json) 已结算 `645e8b4f…` 的 run 35827672098：Linux 28 成功／1 失败／7 跳过，Grok 原生通知 hook 夹具发生 5 秒超时和一次通知缺失；Windows 44 成功／1 失败／3 跳过，两个严格 Job 原子调试测试每项重试三次，均在已挂起进程等待首事件时超时。六份 artifact／31 个文件在仓库外核验，无符号链接、JSON／NDJSON 解析错误或常见凭据／邮箱模式命中。后续结果见 receipt120，`ManualOnly` 不变。
- 固定官方 Grok `1.0.41` 的 [receipt116](validation/macos-working-tree-116-grok-1041-zero-input-p0.safe.json) 在 macOS arm64 隔离环境通过一次原生 ACP 零输入 initialize／authenticate／session/new 和 EOF 退出；首次试验因 CLI 自身 marketplace 初始化改变私有配置而严格审计失败，第二次仅按已审核的 marketplace 例外通过。没有模型输入、业务工具、产品托管链或跨平台结论，正式门禁继续关闭。安装版通知 worker 的本机复制产物首次协议启动耗时约 6.9 秒，超过 hook 的 500 毫秒协议预算；显式冷启动预检后的同一测试组为 15 通过／2 环境跳过。这只证明已就绪路径，冷启动产品投递仍待单独闭环。
- [receipt117](validation/macos-clean-commit-117-grok-1041-zero-input-p0.safe.json) 将相同官方 `1.0.41` 零输入 ACP 探针在干净提交 `84a174f9ecf142c286fc97d3f17664d232dce1c9` 上重跑通过：0 模型输入、0 业务工具、原生 EOF 退出 0、认证副本与隧道清理。仍只有 P0 候选范围，不把不同提交的结果拼作总验收。
- [receipt118](validation/macos-working-tree-118-grok-1041-selected-skill-catalog.safe.json) 保留新版默认 leader 单技能零输入探针因设置广播旧形状被拒的三次失败；[receipt119](validation/macos-working-tree-119-grok-1041-settings-schema.safe.json) 只用脱敏字段名／类型定位到 `1.0.41` 比旧 23 字段多 `subagent_model_inheritance_enabled: boolean`。据此加入 test-only 精确版本、大小／SHA 和 24 字段候选，正式产品版本门禁仍拒绝 `1.0.41`。[receipt121](validation/macos-working-tree-121-grok-1041-selected-skill-catalog.safe.json) 的 raw ACP 第二次零输入目录达到 30 条、唯一所选技能路径匹配及原生 EOF 清理；首次公告通知拒绝仍保留。Rust 候选定向 7／7、旧版回归 13／13、隔离 `cargo check -p warp` 通过；raw ACP 成功不等于 Rust 产品适配器已实测。
- [receipt122](validation/macos-working-tree-122-grok-1041-selected-skill-turn.safe.json) 保留唯一真实输入的失败：第一次被零输入请求构造器拒绝，0 模型输入；第二次向原生 stdin 写入并 flush 1 次，随后通知会话归属校验失败，原生 prompt ACK 未观察，审批 0、历史未读、技能效果未确认。进程组、认证副本、隧道和私有根均清理；已发输入不重试，产品门禁不变。只读复核发现原始隔离日志已删除，收据未保留失败通知的方法或 session 字段类别；探针对无会话的连接级模型广播有误拒可能，但不能据此判定本次真实帧。后续只允许在失败分支记录脱敏方法及 session 字段类别，先做离线形状核对，不放宽会话事件或重试该输入。
- [receipt120](validation/cross-platform-preflight-120-84a174f9.safe.json) 结算精确提交 `84a174f9…` 的 [run 35837274454](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35837274454)：Linux 34 步通过／2 步跳过，固定 Grok `1.0.40` 安装版 hook 为 15 通过／2 环境跳过；Windows 44 步通过／1 步失败／3 步跳过。严格 Job 的系统命令原生退出测试通过（0.326 秒），非系统 DLL 拒绝测试三次均到外层 30 秒超时；日志仅确认根进程主线程已恢复并进入根映像初始事件阶段，未证明超时发生在事件等待、映像摘要还是准备阶段。严格 Job 包装器报告清理完成，但没有独立活动进程数；DLL 拒绝及原子产品升级未通过。6 个 artifact／37 文件在仓库外完成结构、JSON／NDJSON 与常见敏感模式审计；GUI、full workspace 和同提交 macOS 仍缺失。
- [receipt124](validation/macos-working-tree-124-grok-1041-rust-adapter-zero-input.safe.json) 在 `932167716…` 基础的未清洁工作树上，以固定官方 Grok `1.0.41` 和本轮编译的真实 Rust test-only adapter 完成默认 leader 零输入握手、30 条目录中的唯一选定本地技能路径匹配及原生退出清理；输入、审批、工具均为 0。首次误将隔离 wrapper 当官方二进制校验而在启动前拒绝的负证据保留，第二次用独立固定二进制身份与 wrapper 绑定后通过。正式产品门禁仍拒绝 `1.0.41`，完整技能模型回合、同提交及其他平台未验。
- [receipt123](validation/cross-platform-preflight-123-93216771.safe.json) 结算精确提交 `932167716…` 的 Windows 专项 [run 35847734427](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35847734427)：Windows 46 成功／0 失败／2 跳过，Linux 按输入跳过。严格 Job 的系统进程原生退出测试 0.352 秒、非系统 DLL 拒绝测试 32.788 秒，均首试通过；后者断言精确拒绝错误、未执行恶意 DLL 标记、严格 Job 清理和回执匹配。成功测试内部各阶段毫秒与独立 ActiveProcesses 数值未公开，不能据总耗时确定旧 30 秒失败的具体阻塞点。5 个 artifact／24 文件在仓库外审计无结构、JSON／NDJSON 或常见敏感模式问题。三款真实 CLI 原子升级、同提交 Linux／macOS、GUI 与 full workspace 均未由本 run 验收。
- [receipt125](validation/macos-clean-commit-125-cddafab7-zero-input-and-cancel-failed.safe.json) 绑定干净 `cddafab7…`：Grok `1.0.41` 实际 Rust adapter 零输入默认 leader 目录再次通过，30 条中唯一选定本地技能路径匹配、清理确认，正式版本门禁及完整技能回合仍关闭；Codex `0.155.1` 运行中工具取消实链失败，只有 SessionReady 和提交阶段写入安全投影，原生输入接受与失败事件类别未知，工具开始／取消均未记录，夹具最终残留审计为 0 且未用兜底强杀。不能把残留为 0 外推为产品取消通过。无模型 supervisor 夹具从外置盘执行缺根心跳，复制同摘要测试二进制到内置 `/private/tmp` 后 2／2 通过；精确 macOS 授权机制未证实。
- [receipt126](validation/macos-clean-commit-126-4ebd031c-codex-tool-start-failed.safe.json) 绑定干净 `4ebd031c…`：内置盘同源测试程序的无模型 supervisor 预检 2／2；Codex `0.155.1` 实链收到原生提交 ACK、回合开始和精确固定命令审批请求，已允许该请求，随后在夹具认定工具运行前以 Failed 结束。未发送取消，外层无兜底强杀且残留审计为 0，产品取消验收仍失败。只读复核发现夹具只按未包装命令字符串识别工具开始，而旧成功记录的原生 `item/started` 使用 `/bin/zsh -lc` 包装；本次原生失败原因及工具是否实际启动仍未知，不能倒推为已通过。
- [receipt127](validation/macos-clean-commit-127-99c04310-codex-tree-identity-failed.safe.json) 绑定干净 `99c04310…`：Codex `0.155.1` 的原生固定命令进度确认为精确 `/bin/zsh -lc` 包装，提交 ACK 和固定审批允许均通过；随后夹具在父子 Python 进程身份核对处失败，未发送取消。无模型签名 supervisor 预检 2／2，外层 0 残留且未兜底强杀。该失败不能充当产品取消回执。
- receipt126 后的测试修正复用审批的精确 shell 包装解析来识别原生执行开始，只记录匹配类别、同命令完成事件、失败类别和标记文件存在性，不写原生命令或错误文本。Windows 的清理单测已把虚构 PID 的 `taskkill` 调用完全模拟；旧 `cddafab7…` CI 的 Windows Python 步骤失败仍须在 job 结束后看原日志，不从静态代码推断唯一根因。
- receipt127 后的无模型核对发现本机框架版 Python 的 `sys.executable` 与实际 `Python.app` 进程映像不同；测试夹具现仅在同一 `Python.framework` 版本目录接受后者，并另查父子进程的固定脚本参数。内置盘的真实 Python 父子进程测试 1／1 通过且自然退出；该候选仍须在新干净提交上重做 Codex 取消实链。
- Codex 后续测试诊断候选只增加安全里程碑类别，并允许工作区内固定命令无需审批时继续观察真实工具；仍要求若出现审批则精确匹配并回传。`cddafab7…` 的失败不可由代码推断补写原因，后续须在新干净 SHA、内置盘测试二进制和同源签名 supervisor 上另行实测。
- 工作分支：`codex/cli-agent-parity`。
- 已结算的跨平台代码检查点：`c25221a22b03c28ca8c8538bee9229ca1bb0ec87`（`修复 Windows Codex 安装器固定版本`），receipt108 的 Linux／Windows 聚焦预检绑定该精确提交。更新的聚焦矩阵 [run 35815324806](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35815324806) 绑定 `2c4880f15ead4fb929ee8eeef7a463a250a47ee7`，Linux 成功、Windows 因严格 Job 的内层 ignored 测试名称失配而失败，见 [receipt112](validation/cross-platform-preflight-112-2c4880f-windows-debug-fixture.safe.json)；[run 35821695492](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35821695492) 绑定 `4b12091e…`，Linux 成功、Windows 在 Grok Unix-only 测试编译处首败，见 [receipt114](validation/cross-platform-preflight-114-4b12091-grok-unix-test-gate.safe.json)，严格原子诊断仍未执行。后续提交不自动继承这些结果。Grok 单技能修复已推送为 `eebf08b96cff09bbe7fbfd13841aee93a42234f6`，receipt111 的真实实链仍发生在提交前工作树，未冒充该 SHA 的同提交矩阵。receipt107 的 Claude 父子链仍绑定 `45ba0d11…`，receipt106 的 Grok root 候选链仍绑定 `ac0fa70e…`。
- 基线归档仍为 `e6e9d619318e87d0bb889df344c8570f97977e90`；此前受测代码提交为 `38a611773b8ee53860f9ab731b2476b9a40d1819`。
- 后续必须继续使用同一工作树和当前分支，不得从 `main` 重来，不得 `reset/clean` 根目录，也不得直接弹出整理前的完整 stash。

## 2. 本轮冻结的当前事实

### 2.1 Claude Code 官方在线账户

- 默认在线账户认证已验证为可用；安全收据只记录登录状态、账户来源和套餐类型，不记录身份、邮箱、组织或令牌。
- 适配器与协调器增加了显式 `--use-authorized-default-account` 路径：该模式不注入 `CLAUDE_CONFIG_DIR`，并与隔离配置模式互斥。
- 当前固定权限配置已收紧到 `restricted + manual + host permission prompts + Read/Edit`，拒绝用户、项目、本地设置源，并对管理策略异常 fail closed。
- receipt102 的计划模式拒绝与外置 debug supervisor `EAGAIN` 启动失败继续保留，未被后续成功改写。当前验收明确使用系统 `/private/tmp`、签名 release supervisor 和隔离 `WARP_DATA_PROFILE`，不再把外置临时目录的 launchd／Unix socket 问题混入产品结论。
- 干净提交 `45ba0d11…` 的真实生产 runtime-host 父子链已通过：官方 `claude-sonnet-4-6` 完成子任务创建、5 次逐次审批、4 次原生本地工具、5 个输入／3 个执行、2 次原生输入合并、父子双向原生 ACK、自动子结果投递与最终 inspect。三条只在隔离验收标记启用时产生的原生结果关联分别覆盖 2／2／1 个输入；普通会话事件序列不变。
- 父子 runtime-host 均产生 v2 密封退出账本，`last/ack` 分别为 `23/22`、`36/35`，原生进程退出、adapter 成功／终止和 event journal 完成都已核对；runner 退出 0，私有与公开双重审计及敏感值扫描均通过。
- 本收据仍不证明真实 GUI 父子操作、IME／双语布局、SSH／tmux、Linux／Windows、完整异常生命周期或全量 P0–P5。
- 当前安全收据：[receipt107](validation/macos-working-tree-107-claude-21278-parent-child-clean-commit.safe.json)；历史失败继续见 [receipt102](validation/macos-working-tree-102-claude-authorized-parent-child-current.safe.json)。

### 2.2 Grok Build 1.0.40

- 当前实测正式版为 `1.0.40 (eb1a2256660d)`；macOS arm64 二进制 SHA-256 为 `3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2`。
- receipt103 的固定全序握手失败保留；后续实现已按相关原生事件改为有界偏序，接受动态且有界的官方模型目录，并只为“当前版本、正在恢复、session 精确匹配、`isReplay=true`、已审核方法”放行 `session/load` 响应前历史回放，其他错误 session、非 replay 与未知方法继续 fail closed。
- 干净提交 `ac0fa70e…` 的真实生产 supervisor／ACP root 候选链已通过：官方 `grok-4.7` 完成同一原生会话两轮、允许写入、拒绝无文件效果、运行中排队输入的原生 ACK 与下一轮结果、原生取消终态、完整历史核对，以及新进程恢复原会话和排队标记。两代 supervisor 均有 `cleanup_confirmed=true`，认证副本、内部 state、隧道与 staged 进程残留均已清理。
- 能力仍由 test-only candidate gate 隔离，`public_product_gate_open=false`；same-turn steering、技能、本地工具、子任务、父权限上限、App 重启／GUI、产品网络隔离和其他平台没有由本收据证明。
- 当前安全收据：[receipt106](validation/macos-working-tree-106-grok-1040-root-lifecycle-clean-commit.safe.json)；历史失败继续见 [receipt103](validation/macos-working-tree-103-grok-1040-root-lifecycle-current.safe.json)。
- 新的 [receipt111](validation/macos-working-tree-111-grok-1040-selected-skill-combination.safe.json) 在当前工作树上完成默认 leader＋单个显式选定技能的 macOS 隔离实链：原生 ACP 目录由基线 29 条增至 30 条，第 9 位是规范路径与名称匹配的唯一技能；零输入握手通过后，真实 1 次输入／接收、1 次精确只读审批、1 次上下文读取、3 次原生工具事件、唯一 slash 的最终历史与随机技能标记均核对通过。原生进程、隧道和认证副本清理确认。原有静态 29 条目录验证器是本轮首个可修复失败点，现只对单技能的精确额外条目放行；产品正式门禁仍关闭，GUI 与同提交跨平台未验收。
- [receipt113](validation/macos-clean-commit-113-grok-1040-selected-skill.safe.json) 已在干净提交 `4b12091e788fb79f7bbf12c1fbf67d98c52aabe3` 的 macOS arm64 重新完成固定 Grok `1.0.40` 的 0 输入目录握手及 1 次真实选定技能组合：30 条目录含唯一绑定技能，1 次输入／接收、1 次精确只读审批、1 次上下文读取、3 条原生工具事件、最终历史与随机标记匹配，密封退出和隔离清理确认。用户重新登录后默认 `grok` 已指向 `1.0.41`，该新版未被本收据验收；外置盘 supervisor 的预检 EAGAIN 保留为环境失败。原登录文件未变，临时认证副本已删。正式产品门禁、GUI、当前默认新版和同提交跨平台全范围仍未验收。

### 2.3 Windows 原子升级与真实 CLI

- 产品仍保持 `ManualOnly`，在产生副作用前 fail closed；候选 PE/调试器代码保留，但未接入生产自动替换路径。
- Windows 上三款当前 CLI 均完成真实二进制身份、签名和 `--version` 验证：Codex `0.155.1`、Claude Code `2.1.278`、Grok Build `1.0.40`。
- PE 结构与程序/祖先租约检查通过，但持有 cwd 句柄时叶子目录仍可改名；调试器进程树超过 30 秒不退出；三次 updater 参数运行均超时，且终止调试器后 Claude/Grok 根进程曾存活，已显式清理。
- [receipt105](validation/macos-working-tree-105-codex-01551-windows-asset-correction.safe.json) 已纠正资产结论：receipt104 错把 `0.155.1` 官方包与 legacy `0.147.0` 清单比较；版本选择后的仓内 `0.155.1` size/SHA 与 GitHub API digest 一致，无需修改产品摘要。cwd、debugger、退出收据和清理缺口不受此纠正影响。
- receipt112 的 Windows 严格 Job 诊断因内层 `--exact` 名错误筛出 0 项，是历史夹具失败；receipt120 已在新提交实际运行两项 ignored 测试并取得系统命令原生退出通过，但非系统 DLL 拒绝仍超时。receipt104 的真实阻断不被夹具修复或单个系统命令测试覆盖。
- 当前安全收据：`validation/windows-working-tree-104-atomic-real-cli-failclosed.safe.json`。
- `84a174f9…` 的非系统 DLL 拒绝诊断曾在外层 30 秒超时；`932167716…` 给该大型测试二进制用例配备 120 秒有界外层期限和专用 nextest 设置后，两项严格 Job 测试均在 Windows 实机首试通过。仍不能从成功测试被隐藏的阶段日志反推旧超时的具体阻塞点。
- 后续未提交候选已给严格 Job 增加挂起派生和恢复失败时移交子进程的专用入口；这些错误路径只发出终止请求，由外层 Job 有界确认整树退出，且创建进程事件没有可终止句柄时绝不 Continue。隔离本机 `cargo check -p warp`、Windows 目标 `command` check 和 i18n 11 项通过；这批新错误路径尚未在远端 Windows 实机复验，`ManualOnly` 不变。

### 2.4 同提交 Linux／Windows 聚焦预检

- [run 35746148046](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35746148046) 在精确提交 `c25221a22…` 上结算成功：Linux x64 为 34 个成功步骤、2 个按 `full_workspace_tests=false` 跳过；Windows x64 为 45 个成功步骤、2 个同样跳过。两个 job 的 check、IPC frame-limit 恢复、生命周期／取消／Responses、双语 TUI、进程所有权、supervisor、宿主崩溃清理、无凭据原生恢复和 rust-genai 均通过。
- Windows 同一 job 同时验证 Codex `0.155.1` 当前运行时与专用于生产通知安装器合同的官方 `0.147.0` 包；生产 Rust 安装器 metadata 为 `accepted=true`、退出 0、无超时、无凭据、0 个模型命令、migration 通过、EOF 与私有根清理确认。它没有验证原生 hook 实际执行、完整受监督进程树清理或 App 重启／UI，不能回填为三款自动原子升级通过。
- 6 个 artifact 共 37 个文件已下载到仓库外临时目录并逐份核对；JSON／NDJSON 解析失败为 0，邮箱与常见凭据形态扫描均为 0，只把大小、SHA／manifest 摘要和允许字段写入 [receipt108](validation/cross-platform-preflight-108-c25221a22.safe.json)。
- 该 run 是聚焦跨平台边界证据，不含同提交 macOS、GUI integration 或 full workspace；因此总 Goal 继续 active。
- 复核该 Linux job 原始日志后，[receipt109](validation/linux-atomic-execveat-109-c25221a22.safe.json) 确认密封 memfd 的真实 `execveat` 参数／环境／cwd 夹具及失败后禁止 pathname 回退均在 Linux x64 实机通过；此前“Linux `execveat` 完全未运行”的阶段表述已过时。三款 CLI 的 Linux 产品升级事务、退出回执与恢复仍未运行。
- [receipt112](validation/cross-platform-preflight-112-2c4880f-windows-debug-fixture.safe.json) 对 run 35815324806 的两 job 做了结算：Linux 34 成功／2 跳过；Windows 44 成功／1 失败／3 跳过，首败是严格 Job 的内层 `--exact` 测试名错误。6 个 artifact 共 37 个文件已在仓库外下载，JSON／NDJSON 可解析、无符号链接，常见凭据与邮箱模式扫描为 0；这仍是失败的聚焦矩阵，不能替代更新源码的跨平台验收。
- [receipt114](validation/cross-platform-preflight-114-4b12091-grok-unix-test-gate.safe.json) 对 [run 35821695492](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35821695492) 的精确 `4b12091e…` 做了结算：Linux 34 成功／2 跳过；Windows 39 成功／1 失败／8 跳过。首败在 `cargo test --no-run -p warp --lib`：Grok 单技能测试未标 Unix-only，却引用 `#[cfg(all(test, unix))]` 的探针／字段，E0433 与 E0609 使 Windows lifecycle 测试尚未运行；严格原子 image-debug 与生产通知安装器步骤均被跳过，不能声称已复验 receipt112 的夹具修复。5 个 artifact／25 个文件在仓库外下载并核验格式、无符号链接和常见敏感模式；两台 runner 结算后均在线空闲。当前只给该测试补 `#[cfg(unix)]`，待本地门禁与新 SHA 的最小矩阵复验。
- receipt115 的 run 35827672098 已完成结算；`84a174f9…` 的 run 35837274454 为 Linux 通过、Windows 原子 DLL 拒绝诊断失败，详见 receipt120；`932167716…` 的 Windows 专项 run 35847734427 已使严格 Job 两项测试首试通过，详见 receipt123。下一步在新的干净 SHA 上复验挂起派生／恢复错误路径和受影响平台，不把这些机制测试当三款真实 CLI 产品升级。
- 2026-09-23 的 [receipt110](validation/macos-working-tree-110-grok-1040-selected-skill-gate-failed.safe.json) 保留 Grok `1.0.40` 默认入口单技能候选的负证据：用户重新登录后原生会话创建成功，私有原生日志在同一进程／会话两次公告目标技能名，但 ACP 接收路径缺少输入前的 `available_commands_update`，30 秒后按请求超时失败；公告日志不等于原生路径目录交付。0 次产品输入，正式技能门禁未开放。私有认证副本已移除；仅测试夹具接受固定 marketplace 初始化或单字段 purge。
- receipt111 的输入前观察器移到静态目录形状验证前，重新实测发现默认 leader 实际交付 30 条 ACP 命令（含唯一技能），旧 29 条硬编码会拒绝。receipt110 当时“未观察到”仍保留为历史失败，但不能再解释为官方 CLI 从不送达目录；修复后的默认入口组合独立通过。直连 `--no-leader` 则送达不同的 10 条目录并被当前适配器拒绝，仅作零输入对照，不推广为产品入口。

## 3. 已通过的定向门禁与版本边界

- `c25221a22…` 本地隔离 target 的 `cargo check -p warp --features local_cli_managed_tasks` 通过；Codex source runner Python `16/16`、准备器 Python `48/48`、YAML 解析、actionlint（仅允许仓库自定义 runner label）和 diff check 通过。
- receipt107 的 Claude 原生结果证据 `2/2`、Claude Rust `117/117`、协调器 Rust `63/63`、Claude 外层审计 Python `66/66`、i18n `11/11`、feature supervisor build、无模型探针 `2/2` 与真实父子链仍绑定 `45ba0d11…`。
- `cargo fmt --all -- --check`、`git diff --check` 通过。
- 本轮最终修改没有新增或变动用户可见文案，无需本地化资源变更。
- receipt106 的 Grok `139/139` 与 Python `37/37` 仍按 `ac0fa70e…` 快照保留；receipt102／104–105 的历史 Claude、Windows 和升级阶段门禁也只属于各自快照。
- Grok 当前版目录绑定修复的离线 `current_` 回归 `12/12` 通过，覆盖目录先于／后于 `session/new` 响应及输入前观察器；这不证明官方 `1.0.40` 已实际送达 ACP 目录，也不证明选定技能实链成功。
- receipt111 后续门禁：Grok 定向 Rust `236/236`、选定技能 runner Python `12/12`、SDK origin runner Python `108/108`、默认及 `local_cli_managed_tasks` 两种 `cargo check -p warp`、i18n `11/11` 已通过；本轮无新增或变动用户可见文案，无需本地化资源变更。真实单技能链仍绑定 dirty tree，不等于冻结提交或全量 P0–P5 验收。
- receipt114 后续的单行 cfg 候选在独立 target 下通过 `cargo check -p warp` 和对应 Grok Rust 单测 `1/1`；`cargo fmt --all -- --check`、`git diff --check` 与安全收据 JSON 解析通过。仓库自带 `./script/format --check` 报出多处未改文件的既有导入顺序差异；定向 `cargo clippy -p warp --lib --tests -- -D warnings` 先在未改的 `crates/command` 报 8 项既有 lint，未触及本次一行测试修复，不顺手批量改动。此候选仍须 Windows 远端编译确认。
- `c25221a22…` 的 Linux／Windows 聚焦矩阵已通过，但同提交 macOS、full workspace、SSH/tmux 完整产品接收、真实 GUI IME／双语布局和全范围 P0–P5 生命周期仍未完成，因此不能标记完成。

## 4. 本地磁盘与外置磁盘

- 已将约 10 GiB 可迁移的任务缓存、旧 worker、GUI/IPC/自动升级临时目录、Claude 隔离配置和 Grok 旧临时目录迁往：`/Volumes/ORICO/InfiniShell-Desktop-local-offload-20260921`。
- 当前 `.envrc` 外置临时目录：`/Volumes/ACASIS/InfiniShell-Desktop-tmp`；默认 Cargo target：`/Volumes/ACASIS/CargoTarget/InfiniShell-Desktop`。本轮定向构建使用 `/Volumes/ACASIS/CargoTarget/InfiniShell-Desktop-cli-agent-parity-e846c137d`。
- launchd／Unix socket 真实探针显式覆盖 `TMPDIR`／`TEMP`／`TMP=/private/tmp`；默认外置临时目录会导致同一二进制启动失败，不能用该环境失败否定产品链。
- 当前内置磁盘约 105 GiB 可用、ACASIS 约 1.6 TiB 可用；没有移动认证材料，也没有广泛清理用户数据。
- 详细记录见 `WORKSPACE_CLEANUP_20260921.md`。

## 5. 新会话的严格入口

1. `git fetch origin`，确认当前分支为 `codex/cli-agent-parity`、工作树干净，且本交接提交与远端 SHA 一致。
2. 依次阅读 `AGENTS.md`、`HANDOFF_20260921_REOPEN.md`、本文、`PLAN.md`、`CAPABILITY_MATRIX.md`、`VALIDATION_REPORT.md`；收据先看本文引用的 106–114，按失败链再追溯 102–105，避免把历史快照当当前状态。
3. 先做定向闭环，不要因接手而立即重复全量构建。
4. Grok：receipt106 已完成 `ac0fa70e…` 的固定 `1.0.40` 干净提交 root 候选链；receipt110 的原先负收据保留。receipt111 定位目录实际送达而旧 29 条验证器拒绝唯一技能增量；receipt113 又在干净 `4b12091e…` 上通过默认 leader 的单技能真实输入、唯一路径、精确只读审批、历史及最终标记。直连的不同目录仍失败。继续补本地工具、子任务／父权限上限、App 重启／GUI、产品网络隔离和当前默认 `1.0.41` 的独立版本评估；不得因 macOS 隔离候选通过就开放正式功能。
5. Claude：receipt107 已完成 `45ba0d11…` 的 release supervisor 真实父子全链，不要重复消费相同模型链；下一步补真实 GUI 父子操作／重启、SSH／tmux、完整异常生命周期和同提交跨平台证据。运行 launchd 夹具时继续显式使用系统 `/private/tmp`，不得退回计划模式或放宽工具权限换取通过。
6. Windows：补齐 cwd 身份绑定、调试进程树退出和完整 Job 残留清理收据；沿版本选择后的 `0.155.1` 清单复核官方资产，不再使用 legacy `0.147.0` 条目比较。在这些条件完成前继续保持 `ManualOnly`。
7. receipt108／109 已完成 `c25221a22…` 的 Linux／Windows 聚焦预检及 Linux `execveat` 机制实测；receipt115／120 保留后续失败，receipt123 已在 `932167716…` 的 Windows 专项使严格 Job 两项测试首试通过。下一步在新 SHA 上复验挂起派生／恢复错误路径，并完成 Codex 当前版运行中工具取消实链；之后按下述分层门禁节奏处理剩余范围：Linux 三款产品原子升级事务、SSH/tmux 产品接收、GUI IME 与双语布局、异常矩阵，以及功能冻结后的同提交 macOS 与 full workspace 验证。
8. 只有“要求→实现→CLI 版本→源码提交→模式/平台→收据→结果”矩阵中所有必需项在同一当前提交上通过，才可把 Goal 标记为 complete。

### 5.1 新会话的省时验证节奏

- 开发阶段先把相关功能和已知失败按平台／能力成组收敛：每组在本地跑 `cargo check`、受影响的定向 Rust／Python 测试、格式／diff 检查；有用户可见功能变化时按 AGENTS.md 跑 i18n 门禁和双语布局检查。先审 `#[cfg]`、测试过滤器实际匹配数及 workflow 条件，避免把可静态发现的夹具错误交给长时间 CI。
- 需要原生平台才能判定的边界，在上述便宜门禁通过后只选受影响平台做必要的远端验证；现有 workflow 支持 Linux／Windows 单独选择，但不要假设它有未实现的 compile-only 快车道。重大跨平台改动可做一次早期聚焦检查；不要每修一行测试夹具就自动重发 Linux＋Windows 全矩阵。失败必须保留并按首败定位，不为刷绿重复运行。
- 主要 P0–P5 功能、真实 CLI／GUI／SSH-tmux 和升级阻断收敛到候选冻结提交后，再跑一次同 SHA 的聚焦 Linux＋Windows 矩阵；最终在**同一最终源码提交**完成要求中的 macOS 实链、Linux／Windows full workspace、真实 GUI／IME／双语、SSH／tmux、父子与异常生命周期及原子升级收据。任何后续源码修复都使受影响项的旧 SHA 证据失效，必须按影响范围补验，不能拼接旧收据宣称总 Goal 完成。

## 6. 安全边界

- Claude 官方在线账户已获授权；兼容 API 凭据仅存于系统钥匙串，不得写入仓库、收据、日志或聊天。
- 用户曾在聊天中提供兼容 API 密钥；项目最终验收后应轮换该密钥。
- 历史失败必须保留；回放、模拟、编译、skipped、租约注册或单次 inspect 均不能替代真实交互通过。
