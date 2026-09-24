# CLI 对齐 Goal 复核与续接（2026-09-21）

> **2026-09-25 当前验收状态**：固定 Codex `0.156.1`、Claude `2.1.280`、Grok `1.0.41`，Mac 必要能力与实链已完成；Linux 三款正式原子更新、全量回归及 [真实 GUI 中文／图片](validation/macos-fixed-versions-20260924/ci-36023402924-linux-gui.safe.json)已通过。Windows 全量回归、Claude／Grok 正式升级、Codex hook／ConPTY 及真实 GUI 已通过；剩余 Codex 原子升级和 Grok 0.1.3→0.1.4 插件迁移。Goal 保持 active。各要求、源码域和收据以[固定版本验收表](ACCEPTANCE_FIXED_VERSIONS_20260924.md)为准，后文阶段记录保留原结论。

## 2026-09-21 复核快照（历史）

> **当时复核结论：原 Goal 尚未满足全部验收，此前的 complete 结论撤回。**
> 本文件是新的续接入口，优先于 2026-09-19 交接和 `FINAL_20260921_CLI_PARITY.md` 中的“全部完成”声明。已经通过的构建、回归与真实运行证据继续有效，但只覆盖其实际源码、CLI 版本、模式与平台。本次只修正文档，不启动新的开发 Goal，不修改产品代码。

## 1. 先进入正确工作区（整理后已统一到根目录）

| 项目 | 本次复核时状态 |
| --- | --- |
| 仓库根目录 | `/Users/zhishi/Tools/github/InfiniShell-Desktop` |
| 后续开发位置 | `/Users/zhishi/Tools/github/InfiniShell-Desktop` |
| 最新归档提交 | `e6e9d619318e87d0bb889df344c8570f97977e90` |
| 最新受测代码提交 | `38a611773b8ee53860f9ab731b2476b9a40d1819` |
| 远端分支 | `origin/codex/cli-agent-parity`，本次核对指向 e6e9d6193 |
| 后续工作树状态 | 根目录当前在 `codex/cli-agent-parity`，交接和整理文档尚未提交，必须保留 |
| 根目录状态 | 已从 af1040dc8 快进到 e6e9d6193，产品源码与已推送提交一致，历史重复改动已完整备份并收拢 |
| validation-56 | 仍为 e6e9d6193 的干净 detached HEAD，仅作历史参考，后续以根目录为唯一工作入口 |

新会话先重查以上状态，保留根目录本次文档改动，继续当前分支或从当前 HEAD 创建 `codex/` 续接分支。不要从 main 重新开始，也不要再把旧根目录差异恢复到最新源码。此前给出的 validation-56 工作路径已由本节更新。

原先单独保留的内容已核实归属并安全归档：

- `web_runtime.rs` 和 `websearch_tests.rs` 的原改动逐字节等同提交 `0018ed080d2f6cfd1e6d55c80ab8e129088a643a`，已合入本次查到的远端 main；不再是未交付任务。CLI 分支尚未合入此提交，不把它重新混成 CLI 的未提交代码。
- `specs/cli-agent-parity/validation/gui-7e065085/` 的 23 个历史文件已按原字节保存在整理备份与根目录原状态 stash 中，不作为当前功能已通过的证据。
- 原暂存、未暂存、未跟踪文件、Git 索引与二进制补丁均有独立备份，清单和恢复说明见 [工作区整理记录](WORKSPACE_CLEANUP_20260921.md)。不要把完整旧 stash 直接 pop 到最新工作区。

用户随后授权梳理并完成工作区整理，根目录的 1,092 个变更路径已逐项分类并保存，6 份最新交接文档已从 validation-56 集中到根目录，另新增整理记录。此举只整理已有代码和文档，不代表 R1–R8 的功能缺口已修复；未修改旧原始验收收据。

上一任务记录已清理大型可重建 Cargo target；新会话须先核对磁盘挂载、可用空间、缓存与构建目录，不假定旧测试程序仍存在，也不在本轮文档交接时重建它们。

## 2. 已完成且应保留的成果

- e6e9d6193 已提交并推送。它相对 38a611773 的 801 个变化路径均位于 `specs/cli-agent-parity/`；没有后续产品代码差异。
- [跨平台 run 35513649252](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35513649252) 的实际 head SHA 为 38a611773。本次通过 GitHub API 核对 Linux x64、Windows x64 均 completed/success，且实际执行了完整桌面工作区测试步骤。
- [原验收归档](FINAL_20260921_CLI_PARITY.md)记录：macOS 完整桌面集合 10548 通过、83 skipped；Linux 10557 通过、88 skipped；Windows 10318 通过、94 skipped，其中一项布局测试重试通过并标为 flaky。macOS 的 `cargo check -p warp`、i18n 11 项也有通过记录。本次未重跑这些命令；macOS 首次完整执行失败且缺明细的记录继续保留。
- GUI integration 的编译、TUI 独立测试与桌面工作区执行是不同层次；不能将编译或 skipped 计成真实 GUI 运行。
- Codex、Claude、Grok 的识别、安装、插件、运行适配、状态、持久化、权限保护、自动升级和渠道切换已积累大量实现与阶段证据，不应重新从零实现。
- [升级验证报告](OFFICIAL_42_AUTOUPDATE_VERIFICATION.md)包含三款真实升级及渠道往返、Claude 2.1.278 固定策略父子链和取消的限定范围证据。它们不自动证明所有最新版、平台和完整产品链均通过。

上述测试计数不等于功能完成百分比。已验证的能力保持；后续代码改变后，对受影响部分重新验证并绑定新的提交。

## 3. 已确认的功能缺口

本节来自 e6e9d6193 对应产品代码与安全收据，不能用旧文档的“历史状态”提示一笔抹去。

### R1：Grok 1.0.34 仍只有受限的 P0 托管能力

- [grok.rs](../../app/src/ai/cli_agent_runtime/grok.rs)定义 `VERIFIED_VERSION = 1.0.30`、`P0_VERIFIED_VERSION = 1.0.34`。
- `bind_cli_version` 在新版请求固定策略、grok_profile、本地工具或 selected_skills 时直接拒绝。
- `extended_lifecycle_verified` 只认可 1.0.30；新版运行中 Submit 被拒绝，`queuedSubmit`、`localTools`、`childTasks` 均为 false。
- [grok_tests.rs](../../app/src/ai/cli_agent_runtime/grok_tests.rs)的 `latest_version_does_not_enable_fixed_profiles_or_local_tool_leases`、`latest_synthetic_handshake_claims_only_the_native_p0_verified_capabilities` 明确测试这些限制。
- 同文件的 `latest_version_cancels_a_correlated_but_unverified_write_approval` 要求新版未验证写操作审批直接取消；“有 approval=true”不能扩大为各种操作审批已对齐。
- [source50 GUI 收据](validation/macos-official-50-grok-1034-p0-gui.safe.json)同样记录子任务控件关闭、localTools/queuedSubmit/childTasks=false，且未从 GUI 独立复验拒绝、取消和恢复。

**完成条件：** 先核实当时最新正式 Grok 的真实接口，修复适配和必要夹具，再证明运行中追加、固定策略与允许／拒绝、技能、本地工具和父子任务。不能只修改版本白名单、删除保护或把 false 改成 true；也不能以降回 1.0.30 取代“开发及消费者跟进最新正式版”的目标。

### R2：Grok 父子任务仍缺成功整链证据

[较后一次 child5 收据](validation/macos-official-40c-child5-host-events.metadata.json)记录：

- `parent_child_passed=false`、`native_ack_both_directions=false`。
- `automatic_result_ack_verified=false`、`final_result_via_inspect_verified=false`。
- `test_exit_code=101`、`full_cli_parity_acceptance_passed=false`。

[source39 child4 诊断](validation/macos-official-39-child4-diagnostic.safe.json)记录 child_task_count=0，原因是 `grok_creation_catalog_changed`。本次检索未找到足以取代这些失败的后续成功父子收据；这不表示已经独立证明所有后续尝试都失败，新会话应先核对是否有遗漏的真实证据。

**完成条件：** 真实父任务派发子任务，证明权限关系、父→子追加与原生接收确认、子→父进度／最终结果及确认、持久化、取消、重启恢复和重复投递不重复执行。租约注册或 inspect 单次通过不足以替代整链。

## 4. 仍需补齐或重新核对的验收

| 编号 | 当前可确认范围 | 新会话需要完成的工作 |
| --- | --- | --- |
| R3 SSH／tmux | [source60 收据](validation/macos-official-60-ssh-tmux-replay.safe.json)为真实回环 OpenSSH、tmux 和通知字节传输；`notification_origin=bundled_hook_replay`、`native_cli_hook_triggered=false`、`product_ssh_ui_verified=false`、`different_os_host_verified=false` | 保留传输通过；补真实 CLI 触发通知及产品 SSH 路径接收，验证远端版本／插件／路径、断连与 tmux 行为。分别记录传输、真实 CLI、产品 UI 的结论；按平台风险选择场景，不机械扩展所有组合 |
| R4 最新版完整生命周期 | 历史 GUI/原生链与新版本 P0、升级、局部任务链分属不同版本及源码快照 | 为三款当前受测正式版逐项建立验收对应表；普通 PTY 与托管任务分别完成原 PLAN 的生命周期，缺证项继续验收，不能拼接旧版通过为新版通过 |
| R5 消费者可用入口 | `app/src/features.rs` 中 LocalCLIManagedTasks 仍受 `local_cli_managed_tasks` 编译特性控制；曾专门构建启用该特性的 GUI | 核对实际交付构建和功能开关，确认消费者可到达托管任务及升级入口；未开放属于待交付或明确限制，不能凭开发构建宣称消费者可用 |
| R6 双语与输入 | i18n 和部分双语 GUI 有证据；计划仍列有真实 IME／候选选择及其他平台组合待验 | 查找能覆盖当前实现的证据，对缺口补实际输入法、混排、多行、附件／上下文、技能和评审传递；区分真实键入与 Unicode 粘贴 |
| R7 自动升级与渠道 | 已有三款原生往返及配置保全证据；不同安装来源、平台和后台行为的证据边界不同 | 审核官方当前渠道、手动切换、持久化、空闲升级、活跃任务保护、失败回滚和重启恢复；未知来源明确降级，不将未开放来源计为验证通过 |
| R8 文档与证据对应 | 原最终报告曾把全部门槛写为已满足，而实际代码、收据存在上述缺口 | 维护一张“要求→代码→CLI版本→提交→模式／平台→收据→结果”的当前矩阵；旧失败原样保留。最终报告只引用实际足以支持结论的证据 |

R1、R2 是明确缺口；R3–R8 含已经通过的部分以及本轮尚未确认的覆盖，不能全部写成产品失败，也不能默认已完成。上游缺少真实能力时应保留可靠降级、记录事实并继续独立工作；没有用户明确调整验收范围，原必需项不能因此计为完成。

## 5. 下一轮建议顺序与并行边界

1. 阅读 AGENTS、本文件、PLAN、CLI_AUTOUPDATE 和所需技能，核对根目录工作区与凭据可用性，保留交接改动并按需要创建 codex/ 续接分支；先建立当前矩阵。不要因接手就重跑全量构建或要求用户重新授权。
2. 以 Grok 最新正式版的真实接口与最小生产链为优先，完成 R1；诊断每次失败的具体阶段，保留失败收据。固定版本用于可复验，不以未知最新版替换正在运行的任务。
3. 在协议与权限合同可靠后完成 R2。按原计划保留各 CLI 差异，避免为表面对齐而绕过审批或放宽未知来源。
4. 可并行分工：Grok 协议／权限适配、父子协调器及持久化、SSH／tmux 与平台验收。先约定不重叠写入域；公共类型、入口和能力矩阵由主代理顺序整合。信息收集可先并行。
5. 补 R3–R7 中实际缺少的产品证据。消费者自动升级及渠道选择仍属于 Goal，不另行排除。
6. 定向回归通过后再跑本地强制门禁。冻结包含实际代码变更的提交，按 cross-platform-cloud-verification 技能验证同一提交的相关平台；最终阶段执行仓库适用完整门禁。避免无新变化反复运行同一全量套件。
7. 检查最终矩阵、更新报告并提交推送。所有原完成条件满足后才能将新 Goal 标记 complete；仅遇到真实外部阻塞时准确说明并继续独立工作。

## 6. 新 Goal 保留的完整范围与门槛

[PLAN](PLAN.md) 的 P0–P5 与[自动升级约定](CLI_AUTOUPDATE.md)全部继续有效，不能把本次列出的缺口当成剩余范围的穷尽清单。至少包括三方识别／安装／版本／身份、工具栏与富输入、附件／文件／技能／评审、插件及恢复、可信状态和事件顺序、权限与配置保全、本地父子任务、双向 ACK／进度／结果、持久化与恢复、自动升级及可切换渠道、跨平台和 SSH／tmux。

- 三款 CLI 分别完成：新建 → 两轮交互 → 审批允许／拒绝 → 追加指令 → 取消 → 继续 → 应用重启 → 恢复 → 结果回收；普通终端与托管模式分别留证。
- 完成父子权限、双向消息、原生接收确认、进度和结果回收，并验证重启后的持久关系及消息恢复。
- 覆盖插件缺失／不兼容、CLI 崩溃、重复／乱序／旧事件、重投、恢复失败、升级失败及事务恢复。
- 完成英文和简体中文同步、实际布局与输入检查。
- 通过 `cargo check -p warp`、`cargo test -p warp --lib i18n::tests`、受影响测试与仓库适用完整门禁；明确 GUI integration、TUI 和 skipped 的边界。
- 对包含实际修改的同一提交完成 macOS、Linux、Windows 相关验证；SSH／tmux 单独保留真实范围证据。旧 SHA 的绿灯不能替代新修改。
- 交付代码、配套插件、夹具、发布所需文件、能力矩阵与验证报告。上游限制及未通过项不得计完成。

## 7. 本次交接自身的验证范围

本次新增交接、Goal 指令和整理记录，修正 PLAN、能力矩阵、验证报告和原最终报告的状态入口。整理前后的保存与比对见工作区整理记录；根目录产品源码恢复为已推送提交，未新增产品逻辑或改写原始收据，无需本地化变更。执行文档链接与空白检查，不重新运行 Cargo、真实 CLI 或跨平台 CI，不把文档修正视为功能修复。本次没有新的产品提交或推送，新会话须保留交接文档并纳入后续合理提交。

可直接复制的完整指令见 [新 Goal 指令](GOAL_20260921_REOPEN.md)。
