# 三款 CLI 支持与能力对齐

本目录维护当前规格、能力边界、剩余工作和验证结论。**原批准合并范围中的 G01–G10、V03 共 11 项已关闭并合入主线；全平台实机验收尚未全部完成。** 当前结论以本页及 `CURRENT_STATUS.json` 的当前入口为准；历史记录保留原构建、平台、失败和未验边界。

## 当前主线状态（2026-10-14）

此前 `main` 已快进合入并推送 `36abcbb5b027c17a29fc4fb4a5f91b6ebbc2d680`，修复 Linux musl socket 长度字段的 ABI 类型差异，并补入双 musl 编译门禁。本机 `cargo check`、两 musl 目标 `command --all-targets` 检查及 i18n **12/12** 通过；[同提交 Linux 源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368196945)通过，Rust **1661 项**、Python **109 项通过／2 项 Windows 专属跳过**。其中 socket 身份测试在 glibc 宿主实际 **9/9** 通过，两 musl 目标仅计编译通过。

此前 `1ca59d352` 的[两项缺陷修复和四项 macOS debug GUI 复验](#macos-gui-修复复验2026-10-13)及同提交 Linux／Windows 自动检查通过，原 GUI 失败和首轮网络中断记录保留。正式候选 `v0.2026.10.13.16.19.oss_00` 已完成六份 SSH 资源校验、macOS Developer ID 签名、公证和装订，但实际 GUI 发现 CLI 模型路由与 Responses 工具 schema 两处阻塞，整体拒收；[旧发行构建 38368533631](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368533631)余下打包已主动取消，不记为 Linux／Windows 编译失败。

修复提交 `f429fe2c0426846f41f79fef15a791c5f7cbb011` 已快进合入并推送 `main`；本地 `cargo check`、定向回归 **39/39**（含新增 8 项）、i18n **12/12** 通过；[同提交双平台门禁 38411587441](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38411587441)首次运行全部通过：Linux 应用 **624/624**、Windows **609/609**，新增 8 项及通用 i18n 11 项两端均逐名 PASS。该候选 `v0.2026.10.13.22.25.oss_00` 后续因共享初始化入口的循环模型借用回归拒收，[发行构建 38414535397](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38414535397)已主动取消。真实入口测试与调用栈复现了 singleton lookup panic；当前未找到生产调用者，不据此宣称实际共享 GUI 崩溃。Intel runner 的独立 DNS 失败与其他任务的主动取消分别保留，实际基础设施重试为 0。

修复提交 `5360f5d6cf7e3b25263431027993a3a26dca9cb5` 已合入并推送主线：在历史模型更新前构造请求快照，保持 Base／CLI 选择不变；本地 `cargo check`、**40/40** 定向回归和 **12/12** i18n 通过。[同提交双平台门禁 38441190656](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38441190656)首次运行通过：Linux 应用 **625/625**、Windows **610/610**，累计新增 9 项及通用 i18n 11 项两端均逐名 PASS。新候选 `v0.2026.10.14.01.46.oss_00` 标签已推送，[正式发行构建 38444122158](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38444122158) attempt 1 全部成功；本轮六份 SSH 资源已通过静态及限定原生验证，最终 macOS DMG 已通过签名、公证装订及 19 项检查，六项 GUI 实际操作全部通过，独立证据复核通过。Linux 四包原生检查、Windows 最终安装器静态核验及 12 件资产 Draft 全量下载回验已通过；Windows 便携验收包已就绪，用户五阶段回执仍待完成。公开安装包仍是 `v0.2026.10.07.01.09.oss_00`，新候选最终验收未完成。

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

以下三项按各自状态管理：本轮最终 macOS GUI 和发行资产已完成，仅 Windows 最终安装器用户回执及助手审核仍待关闭。debug 包和旧签名候选的历史通过与失败均保留：

| 必做项 | 执行时机与通过条件 |
| --- | --- |
| 最终发行产物校验 | **本候选 12 件资产及全量 Draft 下载回验通过**。每次发版绑定同一源码与正式标签；核对既定 12 件资产、内嵌 SSH 资源和下载摘要，完成 macOS Developer ID 签名、公证及装订检查。继续保留 Intel macOS SSH 扩展，不恢复 Intel 桌面 DMG |
| Windows 安装生命周期 | 按用户新安排，由用户在自己的 Windows 开发电脑完成最终安装器的安装、可见 GUI 启动、正常退出、官方卸载与残留记录，助手审核回执；本候选实际 SHA-256／大小已绑定，含安装器的便携 ZIP 和采集器已交付，五阶段尚未执行。已有分支验收不重开，也不替代本次最终安装器回执；不再索要机器名、账号或要求切换桌面。见[独立操作入口](../../docs/windows-final-installer-acceptance.zh-CN.md) |
| 最近 GUI 改动的候选包冒烟 | **本候选六项实测通过，独立证据审核通过**：四项 GUI、A→B→B→A 实际路由、Responses strict schema 均通过；同包正常重启，配置恢复。见[最终包记录](#正式候选-macos-验收2026-10-14)；不要求补做 Linux 桌面矩阵 |

发行边界见[当前公开版的验证说明](https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/tag/v0.2026.10.07.01.09.oss_00)及[发布工作流](../../.github/workflows/infinishell_release.yml)；最近 GUI 改动的自动覆盖与人工未验范围见 [Zap 吸收报告](../../docs/upstream-essence/2026-10-07-zap.md)。Windows 用户回执及助手审核关闭前，不宣称下一发行候选已完成验收；当前保持 Draft，尚未公开发布。

新轮归档为 `/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/`。上一轮 `/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/` 保留原自动检查、预编译、DNS 失败和主动取消证据；共享初始化修复及红／绿回归记录位于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-14-shared-init-fix/`。旧轮 `/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/` 保留签名公证、六份 SSH 校验、22 张 GUI 原始截图及拒收结果；入口为 `macos-gui/result.json`、`macos-gui/evidence-index.json` 和 `STATUS.zh-CN.md`。修复源码门禁归档于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-byop-routing-fix/`。**无需本地化变更**：修复改变请求路由与协议归一化，没有新增或改变用户文案；指定双语界面已在新候选复验。Linux 桌面继续延期，自动检查保留。

### 正式候选 macOS 验收（2026-10-14）

`v0.2026.10.14.01.46.oss_00` 的最终 DMG **19/19 签名、公证装订和静态检查通过；GUI 六项实际操作全部通过，独立审核通过**。DMG SHA-256 为 `c45d7b4eea75a2fc068ad818b816cd1ae1d59a43480bed3c5ec21e4f33440cad`，执行文件 SHA-256 为 `eb2ac5cc44736b4375a87b612a34112dca4bf8ee02afacd9d136576ed9e0bfc1`；Apple 公证 `7c979869-41d0-4f15-a6c5-636aa7981cc5` Accepted。

- 普通 Agent、CLI 接管、CLI 续轮、返回普通任务的实际请求按 **A→B→B→A** 路由；9 次带工具 Responses 请求均满足 strict/schema 判据，17 次总请求正常完成。
- 普通和备用屏幕的浮窗宽高、换行及内部滚动通过；同一签名包退出重启后实际恢复 **668 × 141.2999** 逻辑像素。窗口数据库行 ID 会在快照保存时重排，按稳定文档标识和实际窗口对应核对。
- Retina 2x 下连字开／关都完成图片覆盖区域点击、ABC123 输入、拖动选字及精确复制回送 `SELECT_ME_RETINA_06`。
- 英文／简体中文实际 HTTP 401 错误完整可读；真实 `create_documents` 创建的同一 Plan 重启恢复，两种语言的本地保存提示完整。

证据入口为本候选归档的 `macos-gui/result.json`、`macos-gui/evidence-index.json`、`macos-gui/r-own9vyp9/` 与 `macos-final/validation/result.json`；独立审查 `macos-gui/independent-audit/final-audit.json`（SHA-256 `0ba18d2b7add5a40c241ac6253d2cb937d5289d82acb4b3d30f6472c4815a1e8`）复核 90/90 原件摘要及 30 张截图通过。本轮使用本机固定模型触发真实 UI／HTTP／工具路径，不外推真实模型能力或成功云同步。临时 provider、模型、语言、连字和尺寸已恢复；settings.toml 完整摘要与私有基线相同，未全量覆盖数据库。GUI、后代和 provider 已退出，DMG 正常卸载；临时目录严格保留，不放宽清理规则。

另记录非阻塞改进：中文“连字”设置搜索没有结果，英文 `ligature` 可找到；此项单列后续本地化搜索优化。本轮未新增产品代码，**无需本地化变更**。

### 最终发行资产与 Windows 交付（2026-10-14）

正式 run `38444122158` attempt 1 全部成功，候选仍固定在源码 `5360f5d6c`。Draft `412209685` 精确包含 **12 件资产**：ARM64 macOS DMG、Windows x64 安装器、四份 Linux 包及六份 SSH 扩展。仅 DMG 和两份签名后的 Mac SSH 资源经本机补入；其余九件保持 CI 原字节。全量认证下载后，本地原件、GitHub 摘要和下载件逐项一致，标签、资产身份及公开 Latest 均未改变；没有公开发布。

Linux AppImage／DEB／RPM／Arch 均完成只读解包、元数据与 ELF 检查、内嵌六资源核对，以及本包二进制实际版本检查；没有安装系统包或执行 Linux GUI。Windows 安装器来源、大小、摘要、AMD64 主程序和内嵌原始六资源均通过，实际 Authenticode 为 **`NotSigned`**，按 OSS 未签名边界记录，不称为有效签名。

[Windows 独立操作文档](../../docs/windows-final-installer-acceptance.zh-CN.md)已绑定最终包，便携 ZIP 包含安装器和全部采集材料；用户在现有 Windows 开发电脑完成五阶段，助手审核回执。**这仍是本候选剩余的发版门禁**，不重开已关闭的 Windows 分支验收，也不将静态结果充当安装卸载通过。

归档入口：[`final-acceptance/result.json`](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/final-acceptance/result.json)和[证据索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/final-acceptance/evidence-index.json)；12 件回验为 `github/draft-roundtrip/result.json`（SHA-256 `428a26eddb0748d9cdbe98e76b1a597a5737b2450134acca4783c2565b02f2fd`），Linux 四包及 Windows 安装器原件分别在 `native-validation/linux-packages/` 和 `native-validation/windows-desktop/`。源头 `candidate.json` 保留构建启动时快照，当前结论以 `STATUS.zh-CN.md` 和上述汇总为准；旧候选和失败原件保留。Linux 桌面继续延期，自动检查保留；本次仅文档归档，**无需本地化变更**。

### 正式签名候选拒收与修复（2026-10-13）

旧候选 DMG 摘要为 `4ba15c20da518bdc8114994c817b327458fba7c049ce581aa40c7d9bb1a8df3e`；Apple 公证 `47f39a29-c42c-4d84-9fd2-e97ebc53f1fb` Accepted，静态签名／装订／Gatekeeper 校验 **19/19** 通过。实际 GUI 完成 Retina 连字开关下输入和拖选、英中真实 HTTP 401、英中 Plan 保存提示与重启恢复；浮窗宽高及滚动在记录变通后完成，重启实际恢复 765 × 164。

整体拒收原因及修复：CLI 子任务请求原先保留 Base 模型，现按实际单个 CLI 任务选择已解析的 CLI 模型；普通任务与多目标请求继续使用 Base。Responses strict 参数原先保留数组 `uniqueItems`，服务端拒绝 `send_message_to_agent` schema；现移除不支持的约束，运行时收件人去重检查保留。两项源码修复通过本地门禁，最终签名包尚待复验，不将旧失败改写为通过。

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

修复提交 `1ca59d352972cac0de0e562a9fcebd8c8066668f` 已快进合入 `main`；生产源码及回归测试与 macOS 归档一致，同时以基线 SHA 和归档 `source-final-code.diff` 绑定，补丁 SHA-256 为 `853704f52d86edf57b6f02b2f5bbe81dfe50c04b2c9f047e06427d25075cd400`；最终签名后二进制 SHA-256 为 `d60d27ad841f952bed109822f0305efb0c39881ec635cf3129636e45f2c15183`。使用 `warpui/test-util,rust-embed/debug-embed` 和 Apple Development 本机签名，仍非正式公证发行包。本地固定模型只用于触发真实 GUI／文档工具，不证明模型能力。

完整报告、索引、结果和 SHA-256 清单位于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-width-fix/`；原失败、仅宽度修复的中间包、操作定位过程均保留。GUI／provider 及后代退出，全部本轮短 TMPDIR 已按身份清理，归档包和隔离 profile 保留。

**收尾安排历史**：用户先确认本轮完成 macOS、远端自动检查待节点上线；随后要求检查节点并继续。两个节点曾恢复在线且空闲；[同提交定向运行 38352476222 首次尝试](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38352476222/attempts/1)已完成编译和基础回归，但两 runner 在应用回归阶段失联，GitHub 标记失败，应用测试结论未知且尚未上传日志产物。这是基础设施中断，不能记为产品通过或产品断言失败。后续重试保持同一受测提交；原中断记录归档于 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-cross-platform-fix/`。macOS 已完成的实测结果保留。 后续只读诊断确认两台 SSH 仍可达、runner 进程仍在，但使用同组网关 DNS 的两台均无法解析 GitHub；单次指定解析 IP 仍连接重置。未修改节点网络或服务配置。用户恢复网络后，两台默认 DNS 访问 GitHub 均返回 HTTP 200；同一提交的[第二次尝试](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38352476222/attempts/2)全部通过，独立证据目录为原归档下的 `retry-1/`，包含报告、证据索引、完整 Actions 日志和 SHA-256 清单。

**双平台最终核验**：两平台 `cargo check` 均通过；基础 crate／窗口与同步 crate／v2 补全／应用回归分别为 Linux **503／525／22／602** 项全通过、Windows **492／528／11／587** 项全通过。新增 7 项回归与通用 i18n 11 项在两平台各逐名 PASS；第 12 项 i18n 是 macOS 专用布局测试，只在本机计证。Python 通知测试 Linux 111 项（跳过 2）、Windows 108 项（跳过 5），均成功，跳过不计通过。逐项日志审计和独立复核通过，未启用全工作区套件。

**尚未完成的发版门禁**：最终候选身份及必要冒烟、12 件正式资产与 macOS 签名公证装订、Windows 最终安装器生命周期。新回归已接入现有自动检查过滤器。Linux 桌面继续延期，不计通过，不要求切换桌面。

## 当前入口

| 目的 | 文档 |
| --- | --- |
| 目标、P0–P5、实施与验收要求 | [PLAN.md](PLAN.md) |
| 当前关闭、延期与发版必验清单 | [本页当前状态](#当前主线状态2026-10-14)、[CURRENT_STATUS.json](CURRENT_STATUS.json) 的 `main_acceptance` |
| 原关闭条件、原生差异与历史缺口 | [KNOWN_GAPS.md](KNOWN_GAPS.md)，按顶部当前入口区分历史快照 |
| 三方能力与消费者可用范围 | [CAPABILITY_MATRIX.md](CAPABILITY_MATRIX.md)、[RELEASE_SUPPORT.md](RELEASE_SUPPORT.md) |
| 自动升级、渠道与事务约定 | [CLI_AUTOUPDATE.md](CLI_AUTOUPDATE.md) |
| 已通过、失败、未验及源码／平台对应 | [VALIDATION_REPORT.md](VALIDATION_REPORT.md) |
| Linux 桌面验收延期与归档入口 | [本页归档决定](#linux-桌面验收延期与归档2026-10-13) |
| Windows 最终安装器用户操作与助手审核 | [独立验收入口](../../docs/windows-final-installer-acceptance.zh-CN.md)，等待最终包身份绑定后交付执行 |

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

当前机器可读结论位于 `CURRENT_STATUS.json` 的根级状态、`current_goal_scope`、`required_goal_merge_gate` 与 `main_acceptance`。原入口值保存在 `historical_entry_before_20261013`，原 Goal 大对象及合并门禁分别原样保留为 `historical_goal_scope_before_merge` 和 `historical_required_goal_merge_gate`；其余逐轮快照继续按各自来源阅读。本轮替换的旧当前摘要完整保存在 `historical_current_summaries_before_gui_closeout_20261013`。原始证据链接仅用于追溯，不能取代当前状态；后续更新结论仍需绑定实际实现和验证。
