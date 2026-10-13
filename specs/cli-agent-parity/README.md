# 三款 CLI 支持与能力对齐

本目录维护当前规格、能力边界、剩余工作和验证结论。**原批准合并范围中的 G01–G10、V03 共 11 项已关闭并合入主线；全平台实机验收尚未全部完成。** 当前结论以本页及 `CURRENT_STATUS.json` 的当前入口为准；历史记录保留原构建、平台、失败和未验边界。

## 当前主线状态（2026-10-13）

状态校正基于 `main` 提交 `b4bfeacc3b29a33886a423002d34f5d0cbf5ee48`；随后在 `45a8911075277d127e959510631c3d4b01b44903` 执行[初轮 macOS GUI 冒烟](#macos-近期-gui-冒烟2026-10-13)，保留原失败结论。同日完成[两项缺陷修复和四项 GUI 复验](#macos-gui-修复复验2026-10-13)，macOS 实测全部通过。修复已提交并推送 `codex/macos-gui-smoke-fixes`，提交为 `1ca59d352972cac0de0e562a9fcebd8c8066668f`；节点上线后启动了同提交 Linux／Windows 自动检查，但两节点在应用测试阶段同时失联，检查未完成；恢复并通过后再完成主线收尾。公开安装包仍是 `v0.2026.10.07.01.09.oss_00`；源码修复通过不等于变更已随安装包发布。

| 范围 | 当前结论与依据 |
| --- | --- |
| G01–G10、V03，共 11 项 | 在原批准范围关闭。[PR #22](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/22) 于 2026-10-08 合并，G09 已正式签收；其余项目保留原平台与构建依据，不外推为全平台实机通过 |
| G09 最终门禁 | 冻结源码 `a6e62df6975935c2fbe24f30cf44fa93f1a108b0` 的 [Windows 正常矩阵](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37707705470)与[同源 Linux／Windows 门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37714092430)完成；合并提交为 `3e962df37074684401d8ae83f31cc476c17f1249`。旧 S1–S4 失败与未知首因保留，不重新作为当前开放项 |
| Windows 验收修复整合 | [PR #25](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/25)已合并，[同提交源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38002117690)通过；没有重新完成 V01／V02／V05，也不推翻 PR #22 对 G09 的关闭 |
| Settings 双语标签与 Windows SSH 输出 | [PR #24](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/24)已合并，实际语言切换、重启恢复及两跳 SSH 输出已有通过记录，不重开旧发行说明中的显示问题 |
| 10 月 12 日续行快照与 TUI 补全回归 | [本地及双平台门禁](../../docs/upstream-essence/2026-10-12.md)通过；本轮及之前收尾文档未改变受测产品源码，不为文档重复调度这些门禁 |

`KNOWN_GAPS.md`、`VALIDATION_REPORT.md`、能力矩阵和支持说明的早期“G09 开放／PR 草稿／14 项未闭合”按原快照阅读。最终 G09 签收以 PR #22 及其收据为准，延期不计通过。

### 延期项与独立补验

| 项目 | 当前证据与后续安排 |
| --- | --- |
| Linux V01／V02／V05 | 完整模型生命周期、物理中文输入法和真实审批双语布局延期，不计通过。旧分支已归档，沿用[下方安排](#linux-桌面验收延期与归档2026-10-13)，保留 Linux 自动检查，不要求现在切换桌面 |
| Windows V01 | Claude R04 完整记录仍绑定原 Build09；Codex 0.156.1、Grok 1.0.41 的 Build11 已有应用内启动、单轮回复及断开正例。遵守用户 10 月 8 日缩减并结束验收的安排，完整生命周期延期；宣称完整 Windows 生命周期已验前再补 |
| Windows V02 | 用户已人工确认输入法无问题，原合同要求的连续物理输入录像未补。正式证据不计完整通过，补证低优先，不单为录像重做人工操作 |
| Windows V05 | 静态双语视口与模拟审批已有证据；真实 Grok 审批待决、允许、拒绝的完整布局延期，下次集中验审批或相关界面改动时补齐 |
| Codex V04 | 图片进入原生请求已有证据，原颜色回答错误保留；独立原生模型正例不替代应用内复验。作为下一轮优先小项，补一条可判定的应用内读图链；不认定为已确认投递缺陷，也不扩成全平台全格式矩阵 |

Windows 依据见[应用内复测和验收边界](windows-integration-20261008/README.zh-CN.md#2026-10-08-应用内复测补充)，V04 的原件范围见[图片投递与理解](KNOWN_GAPS.md#v04--图片投递与模型理解分别验收)。这些延期项不阻塞继续开发；以后改变支持范围时，仍须满足对应完整条件。

G11 的追加语义差异、G12 的普通 Codex Escape 不保证工具退出，继续作为必须如实展示的原生边界；G13 在新增支持 CLI 版本时验证协议、插件、权限与恢复兼容。其他远端系统、架构及 SSH／tmux 组合保留未覆盖边界，有对应变更或新增支持承诺时补验。

### 持续门禁与下次发版必须完成的验收

相关代码变更继续执行 `cargo check`、必要的 i18n／定向回归，以及受影响的 Linux／Windows 自动检查。现有跨平台工作流入口为 `ci/**` 推送或 `workflow_dispatch`，`run_linux` 默认开启；不是每次推送 `main` 都自动执行。文档更新不触发已通过的模型、GUI 或双平台验收重跑。

以下三项属于下一发行候选门禁；macOS 修复后的 debug 包四项 GUI 冒烟全部通过，原失败历史保留，其余仍待对应最终产物验收：

| 必做项 | 执行时机与通过条件 |
| --- | --- |
| 最终发行产物校验 | 每次发版绑定同一源码与正式标签；核对既定 12 件资产、内嵌 SSH 资源和下载摘要，完成 macOS Developer ID 签名、公证及装订检查。继续保留 Intel macOS SSH 扩展，不恢复 Intel 桌面 DMG |
| Windows 安装生命周期 | 下次 Windows 发版前，在现有隔离／干净账号使用最终正式安装器完成安装、启动、官方卸载并记录残留检查。只有安装器构建或 SSH 扩展 `--version` 成功不算通过；以实际原件说明卸载结果，不预称零残留 |
| 最近 GUI 改动的候选包冒烟 | 本机修复包四项通过：浮窗宽高拖动／重启恢复、Retina 图片覆盖区域输入／拖动选字、英中同步错误、Plan 保存 tooltip。最终候选仍需核对包身份及必要冒烟；修复的 Linux／Windows 自动检查因 runner 失联中断，待恢复后重试，不要求补做已延期的 Linux 桌面矩阵 |

发行边界见[当前公开版的验证说明](https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/tag/v0.2026.10.07.01.09.oss_00)及[发布工作流](../../.github/workflows/infinishell_release.yml)；最近 GUI 改动的自动覆盖与人工未验范围见 [Zap 吸收报告](../../docs/upstream-essence/2026-10-07-zap.md)。这三项未完成前，不宣称下一发行候选已完成验收。

### macOS 近期 GUI 冒烟（2026-10-13）

以下为初轮历史结论；当前修复结果见[后续复验](#macos-gui-修复复验2026-10-13)，不改写原失败证据。

在 Apple M1 Pro／macOS 27.0（26A428）Retina 桌面实际操作主线 `45a8911075277d127e959510631c3d4b01b44903` 的独立 debug 测试包。构建通过；二进制 SHA-256 为 `0de734e87f38f80edfe6e9b7e775f881278d76a24cb32f96a1157b035a8a4897`。采用本机测试 features 和 Apple Development 签名，不是最终 Developer ID 公证发行包。**四项结果为两项通过、一项部分通过、一项失败，GUI 发版门禁未通过。**

| 项目 | 实际结果与后续 |
| --- | --- |
| 浮窗拖拽与重启 | **失败，产品阻塞 MAC-GUI-B01**：拖拽能写入宽高，540 × 218.4 逻辑像素重启后保持一致，新浮窗恢复高度；但 alt-screen 父布局仍以窗口宽度的 30% 减边距为上限，保存宽度增至 983.6 也无法展开。[对应约束](../../app/src/terminal/alt_screen/alt_screen_element.rs)在普通缓冲区对照中不存在。修复后须真实复验两条路径的宽高、换行、滚动和重启 |
| Retina 图片输入／选择 | **部分通过**：连字关／开均有图片覆盖区域点击、ABC123 输入和双击选词复制回送正例。最初计划的拖动选字未取得明确可见选区，图片外对照也未稳定确认，保留 MAC-GUI-V01 补验，不判为已证实图片缺陷，不改写初始判据 |
| 英文／简体中文同步错误 | **通过**：合成无效 token 触发真实 HTTP 401，两种语言的错误及多行服务响应完整可读；无数据上传／下载，未保存无效凭据 |
| 英文／简体中文 Plan tooltip | **通过**：本地固定模型回复触发真实文档工具，保存提示两种语言均完整可读，正常退出重启后 Plan 恢复。模拟回复只用于触发 GUI，不证明真实模型能力 |

原始截图、输入字节、SQLite 尺寸收据、构建日志及测试包归档于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-main-45a891107/`；入口为 `REPORT.zh-CN.md`、`EVIDENCE_INDEX.md`、`result.json` 和 `SHA256SUMS`。初次测试包标识错误与选择夹具修正记录保留，未计为产品缺陷。测试进程已退出，四个本轮短 TMPDIR 已按身份核对后清理。

下次发版先修复 B01 并完成 GUI 复验、补清 V01；12 件最终发行资产及 macOS 签名公证装订、Windows 最终安装器生命周期仍为必验。Linux 桌面继续延期且自动检查保留。本轮仅验收与记录，**无需本地化变更**；没有将历史失败、未覆盖平台或延期项改记为通过。

### macOS GUI 修复复验（2026-10-13）

**四项全部通过（4 通过／0 部分／0 失败），本轮 macOS 已知产品阻塞清零。** 在 `45a891107` 基线上修复 alt-screen 浮窗父布局的旧 30% 宽度上限（MAC-GUI-B01）；补验同时确认快速拖动可早于下一帧，旧选择状态导致丢失 Update／End，现于按下／松开即时更新（MAC-GUI-B02，对应原 MAC-GUI-V01 补验）。两项修复均在同一最终包验证。

| 项目 | 最终实际结果 |
| --- | --- |
| 浮窗 | alt-screen 和普通缓冲区真实宽高拖动、换行及内部滚动通过；保存 640 × 140.3 逻辑像素，正常退出后新建浮窗实际恢复，SQLite 前后收据一致 |
| Retina 图片交互 | 连字关／开均完成覆盖区域点击、ABC123 输入及真实拖动选字；两条路径复制并回送的原始字节均为 `SELECT_ME_RETINA_06` |
| 英中同步错误 | 同一最终包真实 HTTP 401，双语错误及多行响应完整可读；无上传／下载或成功凭据保存 |
| 英中 Plan 提示 | 真实文档工具创建 Plan，英文 tooltip 完整；切中文并正常重启后 Plan 恢复，中文 tooltip 完整 |

`cargo check --locked -p warp`、debug 构建通过；定向 nextest **50/50**（含新增 7 项和 i18n 12 项）通过，另行 `cargo test ... i18n::tests` **12/12** 通过。格式、diff、actionlint（声明实际自托管标签）及独立差异审查通过。无需本地化变更：修复未新增或改变用户文案，双语布局已复验。

修复已提交／推送为 `1ca59d352972cac0de0e562a9fcebd8c8066668f`，当前在独立验证分支；生产源码及回归测试与 macOS 归档一致，同时以基线 SHA 和归档 `source-final-code.diff` 绑定，补丁 SHA-256 为 `853704f52d86edf57b6f02b2f5bbe81dfe50c04b2c9f047e06427d25075cd400`；最终签名后二进制 SHA-256 为 `d60d27ad841f952bed109822f0305efb0c39881ec635cf3129636e45f2c15183`。使用 `warpui/test-util,rust-embed/debug-embed` 和 Apple Development 本机签名，仍非正式公证发行包。本地固定模型只用于触发真实 GUI／文档工具，不证明模型能力。

完整报告、索引、结果和 SHA-256 清单位于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-width-fix/`；原失败、仅宽度修复的中间包、操作定位过程均保留。GUI／provider 及后代退出，全部本轮短 TMPDIR 已按身份清理，归档包和隔离 profile 保留。

**收尾安排历史**：用户先确认本轮完成 macOS、远端自动检查待节点上线；随后要求检查节点并继续。两个节点曾恢复在线且空闲；[同提交定向运行 38352476222](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38352476222)已完成编译和基础回归，但两 runner 在应用回归阶段失联，GitHub 标记失败，应用测试结论未知且尚未上传日志产物。这是基础设施中断，不能记为产品通过或产品断言失败。两节点恢复后同提交重试一次；原中断记录归档于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-cross-platform-fix/`。macOS 已完成的实测结果保留。 后续只读诊断确认两台 SSH 仍可达、runner 进程仍在，但使用同组网关 DNS 的两台均无法解析 GitHub；单次指定解析 IP 仍连接重置。待恢复 DNS／外网通路，未修改节点网络或服务配置。

**尚未完成的门禁**：本修复的 Linux／Windows 定向自动检查（运行 38352476222 因两节点失联中断，待恢复后重试）；最终候选身份及必要冒烟、12 件正式资产与 macOS 签名公证装订、Windows 最终安装器生命周期。新回归已接入现有自动检查过滤器。Linux 桌面继续延期，不计通过，不要求切换桌面。

## 当前入口

| 目的 | 文档 |
| --- | --- |
| 目标、P0–P5、实施与验收要求 | [PLAN.md](PLAN.md) |
| 当前关闭、延期与发版必验清单 | [本页当前状态](#当前主线状态2026-10-13)、[CURRENT_STATUS.json](CURRENT_STATUS.json) 的 `main_acceptance` |
| 原关闭条件、原生差异与历史缺口 | [KNOWN_GAPS.md](KNOWN_GAPS.md)，按顶部当前入口区分历史快照 |
| 三方能力与消费者可用范围 | [CAPABILITY_MATRIX.md](CAPABILITY_MATRIX.md)、[RELEASE_SUPPORT.md](RELEASE_SUPPORT.md) |
| 自动升级、渠道与事务约定 | [CLI_AUTOUPDATE.md](CLI_AUTOUPDATE.md) |
| 已通过、失败、未验及源码／平台对应 | [VALIDATION_REPORT.md](VALIDATION_REPORT.md) |
| Linux 桌面验收延期与归档入口 | [本页归档决定](#linux-桌面验收延期与归档2026-10-13) |

`CODEX_PLUGIN_CACHE_REFRESH.md`、`PLUGIN_COMPATIBILITY.md`、`SSH_TMUX_VERIFICATION.md` 是配套插件说明正在引用的简短兼容入口，正文已收敛到上述文档。

## Linux 桌面验收延期与归档（2026-10-13）

按用户决定，归档并删除 `codex/acceptance-linux-v01-v02-v05` 分支。该分支独有的 3 个提交只增加四份验收 JSON，没有独有产品代码；原受测源码为 `28d0922519b02b0690626edc148638c0150cda1c`，记录提交为 `1a5910afec898cc00522c9147ce665c539ab3968`。

- 归档标签 `archive/acceptance-linux-v01-v02-v05-2026-10-13` 保留原提交历史；[四份原始记录与证据索引](https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/1a5910afec898cc00522c9147ce665c539ab3968/specs/cli-agent-parity/acceptance/linux-28d092251-20261005)按原字节保留，不改写历史状态。
- 闪迪备份位于 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/linux-acceptance-archived-2026-10-13/`，包含四份 JSON、Git bundle、三提交补丁、恢复说明及 SHA-256 清单。bundle 依赖上述受测源码提交，该提交已包含在主线历史中。
- 索引内 42 项原始证据仍指向 Linux 私有目录 `/home/chiiz/.local/share/infinishell-acceptance/linux-28d092251-20261005`。本次只归档记录与索引，没有复制或重新核验远端原件，也不清理配置、构建产物或原始日志。
- Linux V01（真实模型生命周期）、V02（真实中文输入法）、V05（双语界面与真实审批）延期，保持未验、不计通过；Windows 的验收安排不因本决定改变。后续从届时主线固定新的受测源码和二进制，优先评估用户日常桌面，不把切换 Xfce/X11 作为默认前置要求。
- 保留现有 [Linux 自动检查工作流](../../.github/workflows/cross-platform-preflight.yml)，继续使用 `ci/**` 推送和 `workflow_dispatch` 入口，`run_linux` 默认开启；不修改工作流、测试或验收判据。自动检查通过不替代延期的桌面实机验收。

本次仅归档与更新验收安排，**无需本地化变更**。

## 维护约定

2026-09-25 按用户要求，目录维护方式调整为保留规格与结论，不再提交完整运行收据、截图、转储或逐轮过程专报。该决定替代此前“所有历史材料继续留在当前目录”的约定。

- 同类进展更新现有入口，避免新建重复的“最终总结”。独立功能按仓库 [Spec 约定](../../CONTRIBUTING.md#opening-a-spec-pr)使用简短设计和关联 PR。
- 验证结论记录日期、源码提交、CLI 版本、平台、模式、检查范围、通过／失败／跳过及待补项，必要时引用 CI 运行或持久产物摘要。只保留结论不等于放宽真实验证要求。
- 新的原始日志、截图和探针输出保存在仓库外；`validation/` 已忽略，仅兼容仍向该位置输出的本地旧脚本。无需强制加入 Git。
- [fixtures/](fixtures/) 保留固定路径和字节。它包含代码编译、协议回归、插件兼容迁移及版本准备依赖，不属于可以按过程文件清理的验收产物。本次不迁移或删除夹具。
- 本次仅整理文档与历史材料，不改变产品能力，不重跑源码未变的在线验收，**无需本地化变更**。

## 历史追溯

精简前完整目录固定于提交 `e3ef39689cd8b686dfe040b90217ed1067b4d268`，包括当时的通过、失败、截图、原始收据和摘要索引。历史内容从当前树移除，Git 历史未改写；需要时可通过[固定提交目录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity)或以下方式读取单个文件：

```sh
git show e3ef39689cd8b686dfe040b90217ed1067b4d268:specs/cli-agent-parity/<原相对路径>
```

整理前已在维护机器的仓库外保留完整压缩备份及逐文件 SHA-256 清单；备份位于 `~/Documents/InfiniShell-Archives/cli-agent-parity/`，以完整提交号命名。跨机器追溯以固定 Git 提交为准，不依赖该本机目录。

当前机器可读结论位于 `CURRENT_STATUS.json` 的根级状态、`current_goal_scope`、`required_goal_merge_gate` 与 `main_acceptance`。原入口值保存在 `historical_entry_before_20261013`，原 Goal 大对象及合并门禁分别原样保留为 `historical_goal_scope_before_merge` 和 `historical_required_goal_merge_gate`；其余逐轮快照继续按各自来源阅读。原始证据链接仅用于追溯，不能取代当前状态；后续更新结论仍需绑定实际实现和验证。
