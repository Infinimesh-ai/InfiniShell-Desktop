# 三款 CLI 支持与能力对齐

本目录维护当前规格、能力边界、剩余工作和验证结论。**原批准合并范围中的 G01–G10、V03 共 11 项已关闭并合入主线；全平台实机验收尚未全部完成。** 当前结论以本页及 `CURRENT_STATUS.json` 的当前入口为准；历史记录保留原构建、平台、失败和未验边界。

## 当前主线状态（2026-10-19）

**本轮受测源码 `8b46bcc3…` 已快进合入并推送 main，PR #26 已合并；第八 Windows CI 成功，readonly、PATH／reissue、通知及 Rust／i18n 定向门禁通过。七次失败保留。Mac 更新器英中旧版拒绝、失败／重试及英文恢复已在冻结运行包限定范围通过并清理；中文失败 AX 公告已在同包前台手动检查中补验通过，正式新包未构建、未公开发布。** 精确身份、计数及边界见[发布修复整合续记](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/upstream-essence/2026-10-19-release-integration.zh-CN.md)，不在本页复制逐次进度。

- 原 Windows R9 和 Ubuntu DEB 用户演练已经收尾；R9 `c18725c5…` 已替换 Draft Setup，回下载大小／SHA 一致，其余 11 件资产未变。R9 原应用仍 `5068dae9c…`，不含本轮上游／updater，`NotSigned` 和 Draft 边界保留。
- 更新器共用链路及英中失败提示已实现；本地 61／61 nextest、显式 i18n 12／12、warp／warp_tui check 通过。Linux 定向 1942 次 Rust 执行和第八 Windows 1902 次执行通过，均非全库；Windows app 内 i18n 11 通过。本轮 Mac 更新状态双语布局已在 2560×1600 实测；最终三端新包与实际 SSH 未验。
- 旧 CLI V01／V02／V05 仍延期，G09 关闭，原 SSH／输入分类／中文搜索非阻塞项不变。TUI 冒烟未发模型请求，不等于真实 SSH 或 GUI 验证。

证据入口：[发布修复整合续记](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/upstream-essence/2026-10-19-release-integration.zh-CN.md)（源码／包身份、七轮失败简表、清理复查、待验及下次正式候选影响补验）；[机器可读当前记录](/Volumes/SanDisk/github/InfiniShell-Desktop/specs/cli-agent-parity/CURRENT_STATUS.json)的 `main_acceptance.release_integration_20261019`；[R9 回下载](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/r9-roundtrip.json)；[原多端验收梳理](/Volumes/SanDisk/github/InfiniShell-Desktop/docs/release-acceptance-review-20261019.zh-CN.md)。`r-384c44cd`／`r-b2fe8bf4` 已依独立复查 cleaned，原 cleanup=false 收据保留；本轮两次 GUI 与三夹具均 exit 0／cleaned，设置已逐字节恢复，无用户待操作，见[GUI 最终续验](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/resumed-validation/resumption-final.json)。后续[中文失败 AX 补验](/Volumes/SanDisk/InfiniShell-Archives/upstream-essence/2026-10-19/release-integration/macos-update-gui/zh-failure-a11y/result.json)捕获中文失败公告值，新增 GUI 与夹具均正常退出／cleaned，设置再次逐字节恢复；未采集 VoiceOver 语音。原 `current_release_candidate` 及历史对象继续绑定原字节；不要求现在重跑旧包六阶段，新候选按影响补验，下面 10 月 15 日内容仅为当时快照。

## 2026-10-15 状态快照（历史保留）

**统一候选最终 Mac 签名包六项 GUI 冒烟、同包重启恢复，以及完整 12 件资产核验均通过。Windows 与 Ubuntu 24.04 x86_64 用户六阶段回执及助手审核仍是发版门禁，保持 Draft。** 当前产品源码固定为 `5068dae9c75b79628865a0872684113647c204d8`，主线整合及工件核验控制器为 `203d08e5053413d5d6a14971208de4e125eb2f1c`。独立 GUI 改动 `b4e59a8cda491ba1e6430175b924b07101dbea54` 未包含在本候选，不把控制器或当前主线代码当作候选产品源码。

当前候选是 `v0.2026.10.14.16.45.oss_00`，annotated tag 对象 `27d40029759b541dc8903a0deb9ec0948e9670fa`；[正式构建 38537451605](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38537451605) attempt 1 已成功完成，当前 Draft 有完整 12 件最终资产。用户已确认等待同一修复候选，在 Windows 开发电脑与 **Ubuntu 24.04 x86_64** 各验一次，助手审核回执。新 Setup 经 [38703172103 原生只读核验](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38703172103)确认 AMD64、内嵌六资源一致，实际 Authenticode 为 `NotSigned`；[Windows 便携包收据](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/windows-manual/InfiniShell-Windows-Handoff-v0.2026.10.14.16.45.oss_00.json)已生成。Ubuntu 下载恢复成功，真实 DEB 的元数据、ELF、desktop、六资源和卸载脚本静态审核通过；两平台安装包的[Draft 回下载对照](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/windows-ubuntu-draft-roundtrip/result.json)通过；本轮又完成全部 12 件最终资产回下载。Mac 首次链接失败保留，本机 response-file 调用修正后签名 DMG 已生成，Apple 公证 Accepted、装订及[最终 DMG 静态核验](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/macos-final/dmg-verification/result.json)通过，ARM64 版本执行及本轮六项实际 GUI 冒烟通过，完整 12 资产已核验；两端用户实际桌面回执及助手审核仍待完成；旧 CLI 认证矩阵仍延期，G09 不重开。新产物仅允许写入 Draft，公开版本仍为 `v0.2026.10.07.01.09.oss_00`。

本轮 [Mac GUI 结果](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/macos-gui/result.json)覆盖 A→B→B→A 实际请求路由、Responses strict schema、普通/备用浮窗拖动滚动及重启恢复、Retina 2x 连字开/关输入拖选、英文/中文真实 HTTP 401 和同一 Plan 的重启/双语保存提示。14 个请求中 13 个完成，首个 CLI B 被续轮替代而断开；8 个带工具请求 schema 全部满足判据，其中 7 个完成，未将断开请求算作完整完成。浮窗重启恢复 **481 × 271.5999755859375** 逻辑像素。DMG 与执行文件摘要绑定同包，退出后严格签名复核通过；临时设置增量已恢复，精确挂载已非强制卸载。[25 张原始截图及摘要索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/macos-gui/evidence-index.json)保留误点等原始尝试。模型是本机固定夹具，不外推商业模型能力或成功云同步。 [独立复核](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/macos-gui/independent-audit/result.json)确认六项证据成立，25 张图逐张审阅、81 件原件摘要匹配；截图实际为 JPEG，保留接口原字节和原 `.png` 文件名，格式差异不影响验收。

[完整 12 件资产清单及索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/full12-roundtrip/evidence-index.json)已封存：共 **3,374,447,497 字节**，本地、GitHub provider、完整回下载 SHA-256 和长度一致。只向 Draft 新增最终 DMG、替换两份签名后的 Mac SSH，其余九件 ID 和摘要不变，已交付 Windows/Ubuntu 包无需重取。Linux 四格式均完成静态核验；Arch 主程序与 DEB 全字节一致，RPM 注释剥离及 AppImage 动态链接变换有有限差异证明，不宣称 Linux 原生运行通过。

非阻塞改进 `MAC-SETTINGS-SEARCH-ZH-01`：中文“连字”搜索无结果，英文 `ligature` 可找到并操作该设置；保留[截图](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/macos-gui/r-5cmojj28/25-chinese-ligature-search-empty.png)，后续完善中文搜索。

两套交付使用同一源码和标签： [Windows ZIP](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/windows-manual/InfiniShell-Windows-Acceptance-v0.2026.10.14.16.45.oss_00.zip)／[Windows 任务](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/windows-manual/InfiniShell-Windows-Task-v0.2026.10.14.16.45.oss_00.zh-CN.md)；[Ubuntu 24.04 x86_64 ZIP](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/ubuntu-manual/InfiniShell-Ubuntu24-Acceptance-v0.2026.10.14.16.45.oss_00.zip)／[Ubuntu 任务](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/ubuntu-manual/InfiniShell-Ubuntu24-Task-v0.2026.10.14.16.45.oss_00.zh-CN.md)。各自 ZIP 内已包含安装器、采集器和完整说明，用户按任务六阶段执行后回传证据，助手再审核。见[交付复核与证据索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/handoff-delivery-20261015/review.json)。

[5068 Linux 原生门禁 38534655990](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38534655990) attempt 1 全部通过：真实 `dpkg-deb` 构建与提取的 **252 个维护脚本场景**、双 musl 编译检查、socket **9**、crate **503**、迁移／同步 **525**、补全 **22**、应用 **625**，其中 Linux i18n **11/11** 逐名通过；通知测试 **109 通过／2 项 Windows 专属跳过**。本机 `cargo check`、`cargo check -p warp` 和 i18n **12/12** 通过。原件及 ZIP 摘要复核见[Linux 审核记录](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/linux-deb-cleanup-fix/remote/38534655990/review.json)，不计为 Ubuntu 桌面或正式候选包通过。

Windows 实现沿用已验证的 `5acb349c7032c0fc07ba4fbbfaba93d87fa6407b` 字节：[Windows 原生门禁 38512735757](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38512735757)的 Inno **80 项断言**、隔离注册表 **6 项回归**、应用 **616 项**（含该 6 项）、crate **492 项**、Zap **528 项**、completer **11 项**按原源码保留。[Git 同源对照](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/windows-source-equivalence.json)确认 5ac 到 5068 的 Windows 实现未变；这不是 5068 的新 Windows 原生运行，也不替代新 Setup 核验。

`13.00` 候选因 DEB 卸载脚本遗漏包名替换、可能遗留程序链接而被统一修复候选替代；[正式 run 38514368665](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38514368665)已主动取消，原件见[取消记录](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.13.00.oss_00/github/superseded-cancellation/run-final.json)，不记作 Windows 编译失败。其完整旧 current 对象与此前 `01.46` URI 拒收均保存在 `CURRENT_STATUS.json` 的历史入口；旧包通过分项不移用于新标签。

此前 `main` 已快进合入并推送 `36abcbb5b027c17a29fc4fb4a5f91b6ebbc2d680`，修复 Linux musl socket 长度字段的 ABI 类型差异，并补入双 musl 编译门禁。本机 `cargo check`、两 musl 目标 `command --all-targets` 检查及 i18n **12/12** 通过；[同提交 Linux 源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368196945)通过，Rust **1661 项**、Python **109 项通过／2 项 Windows 专属跳过**。其中 socket 身份测试在 glibc 宿主实际 **9/9** 通过，两 musl 目标仅计编译通过。

此前 `1ca59d352` 的[两项缺陷修复和四项 macOS debug GUI 复验](#macos-gui-修复复验2026-10-13)及同提交 Linux／Windows 自动检查通过，原 GUI 失败和首轮网络中断记录保留。正式候选 `v0.2026.10.13.16.19.oss_00` 已完成六份 SSH 资源校验、macOS Developer ID 签名、公证和装订，但实际 GUI 发现 CLI 模型路由与 Responses 工具 schema 两处阻塞，整体拒收；[旧发行构建 38368533631](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368533631)余下打包已主动取消，不记为 Linux／Windows 编译失败。

修复提交 `f429fe2c0426846f41f79fef15a791c5f7cbb011` 已快进合入并推送 `main`；本地 `cargo check`、定向回归 **39/39**（含新增 8 项）、i18n **12/12** 通过；[同提交双平台门禁 38411587441](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38411587441)首次运行全部通过：Linux 应用 **624/624**、Windows **609/609**，新增 8 项及通用 i18n 11 项两端均逐名 PASS。该候选 `v0.2026.10.13.22.25.oss_00` 后续因共享初始化入口的循环模型借用回归拒收，[发行构建 38414535397](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38414535397)已主动取消。真实入口测试与调用栈复现了 singleton lookup panic；当前未找到生产调用者，不据此宣称实际共享 GUI 崩溃。Intel runner 的独立 DNS 失败与其他任务的主动取消分别保留，实际基础设施重试为 0。

此前修复提交 `5360f5d6cf7e3b25263431027993a3a26dca9cb5` 已合入并推送主线：在历史模型更新前构造请求快照，保持 Base／CLI 选择不变；本地 `cargo check`、**40/40** 定向回归和 **12/12** i18n 通过。[同提交双平台门禁 38441190656](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38441190656)首次运行通过：Linux 应用 **625/625**、Windows **610/610**，累计新增 9 项及通用 i18n 11 项两端均逐名 PASS。旧候选 `v0.2026.10.14.01.46.oss_00` 的[正式发行构建 38444122158](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38444122158) attempt 1 全部成功；该轮六份 SSH 资源通过静态及限定原生验证，最终 macOS DMG 通过签名、公证装订及 19 项检查，六项 GUI 实际操作和独立复核通过。Linux 四包原生检查、Windows 安装器静态核验及 12 件资产 Draft 全量下载回验均通过。后来收到的 Windows 用户回执确认 URI 登记失败，因此该候选整体拒收；这些通过分项不移用于新候选。

| 范围 | 当前结论与依据 |
| --- | --- |
| G01–G10、V03，共 11 项 | 在原批准范围关闭。[PR #22](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/22) 于 2026-10-08 合并，G09 已正式签收；其余项目保留原平台与构建依据，不外推为全平台实机通过 |
| G09 最终门禁 | 冻结源码 `a6e62df6975935c2fbe24f30cf44fa93f1a108b0` 的 [Windows 正常矩阵](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37707705470)与[同源 Linux／Windows 门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37714092430)完成；合并提交为 `3e962df37074684401d8ae83f31cc476c17f1249`。旧 S1–S4 失败与未知首因保留，不重新作为当前开放项 |
| Windows 验收修复整合 | [PR #25](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/25)已合并，[同提交源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38002117690)通过；没有重新完成 V01／V02／V05，也不推翻 PR #22 对 G09 的关闭 |
| Settings 双语标签与 Windows SSH 输出 | [PR #24](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/24)已合并，实际语言切换、重启恢复及两跳 SSH 输出已有通过记录，不重开旧发行说明中的显示问题 |
| 10 月 12 日续行快照与 TUI 补全回归 | [本地及双平台门禁](../../docs/upstream-essence/2026-10-12.md)通过；本轮及之前收尾文档未改变受测产品源码，不为文档重复调度这些门禁 |

`KNOWN_GAPS.md`、`VALIDATION_REPORT.md`、能力矩阵和支持说明的早期“G09 开放／PR 草稿／14 项未闭合”按原快照阅读。最终 G09 签收以 PR #22 及其收据为准，延期不计通过。

### Windows URI 发版阻塞与修复（2026-10-14）

用户已交回 `01.46` 候选的完整 Windows 证据，助手[复核报告](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-user-review-20261014/README.zh-CN.md)确认安装、可见 GUI、正常退出和官方卸载分项通过。**URI 命令五阶段仍指向旧失效目标，`WIN-RELEASE-URI-01` 使旧候选拒收**；旧 URI/AUMID 基线已有，不能算作新安装遗留。

本轮已修复安装时登记、启动同步恢复及读回、URI/AUMID 归属标记与按所有权卸载清理，并通过上述原生门禁；[源码与门禁收据](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/source-gates-5acb349c7.json)和[Windows 原生审核](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/remote/38512735757/review.json)均绑定 `5acb349c7`。早期 `4c02c6755` 的 Inno 声明编译失败按原 run `38512138949` 保留，不作为新源码结果。现场旧命令摘要不变的首因仍未确定，不将 `KEY_READ` 或独立确认的终止 NUL 缺陷冒称为现场唯一根因。

新候选 Setup 和 Ubuntu DEB 已完成静态校验、便携 ZIP 绑定及两资产 Draft 回下载对照，现在由用户按各自交付任务执行 `baseline → installed → launched → uri_activated → closed → uninstalled` 六阶段，助手审核实际 URI 唤起和本实例登记清理。源码修复通过尚不能关闭发行验收阻塞。Ubuntu 24.04 x86_64 将按[独立 DEB 验收入口](../../docs/ubuntu-final-installer-acceptance.zh-CN.md)执行同候选六阶段；旧 CLI 认证矩阵继续延期，自动检查保留。无需本地化变更。

### 延期项与独立补验

| 项目 | 当前证据与后续安排 |
| --- | --- |
| Linux V01／V02／V05 | 旧 CLI 认证矩阵的完整模型生命周期、物理输入法及真实审批双语布局继续延期，不计通过。归档决定保留；本轮新增 Ubuntu DEB 安装生命周期验收，不自动关闭这些旧矩阵项 |
| Windows V01 | Claude R04 完整记录仍绑定原 Build09；Codex 0.156.1、Grok 1.0.41 的 Build11 已有应用内启动、单轮回复及断开正例。遵守用户 10 月 8 日缩减并结束验收的安排，完整生命周期延期；宣称完整 Windows 生命周期已验前再补 |
| Windows V02 | 用户已人工确认输入法无问题，原合同要求的连续物理输入录像未补。正式证据不计完整通过，补证低优先，不单为录像重做人工操作 |
| Windows V05 | 静态双语视口与模拟审批已有证据；真实 Grok 审批待决、允许、拒绝的完整布局延期，下次集中验审批或相关界面改动时补齐 |
| Codex V04 | 图片进入原生请求已有证据，原颜色回答错误保留；独立原生模型正例不替代应用内复验。作为下一轮优先小项，补一条可判定的应用内读图链；不认定为已确认投递缺陷，也不扩成全平台全格式矩阵 |

Windows 依据见[应用内复测和验收边界](windows-integration-20261008/README.zh-CN.md#2026-10-08-应用内复测补充)，V04 的原件范围见[图片投递与理解](KNOWN_GAPS.md#v04--图片投递与模型理解分别验收)。这些延期项不阻塞继续开发；以后改变支持范围时，仍须满足对应完整条件。

G11 的追加语义差异、G12 的普通 Codex Escape 不保证工具退出，继续作为必须如实展示的原生边界；G13 在新增支持 CLI 版本时验证协议、插件、权限与恢复兼容。其他远端系统、架构及 SSH／tmux 组合保留未覆盖边界，有对应变更或新增支持承诺时补验。

### 持续门禁与下次发版必须完成的验收

相关代码变更继续执行 `cargo check`、必要的 i18n／定向回归，以及受影响的 Linux／Windows 自动检查。现有跨平台工作流入口为 `ci/**` 推送或 `workflow_dispatch`，`run_linux` 默认开启；不是每次推送 `main` 都自动执行。文档更新不触发已通过的模型、GUI 或双平台验收重跑。

以下四项均按新 `16.45` 候选绑定实际产物和证据；旧 `01.46` 的通过分项、URI 拒收及 `13.00` 的替代／取消结果保留：

| 必做项 | 执行时机与通过条件 |
| --- | --- |
| 最终发行产物校验 | **本候选完整 12 资产门禁通过**。绑定同一源码与标签，核对既定 12 件资产、内嵌 SSH 资源及全量 Draft 下载摘要，完成 macOS Developer ID 签名、公证及装订。保留 Intel macOS SSH 扩展，不恢复 Intel 桌面 DMG |
| Windows 安装生命周期 | **新 Setup 静态审核及便携包组装完成，用户六阶段待验**。用户在现有 Windows 开发电脑执行安装、可见 GUI、实际 URI 唤起、正常退出、官方卸载与残留记录，助手审核。两套包已就绪，任务已绑定实际安装器及 ZIP 摘要；不重开分支验收，不另索要机器名、账号或要求切换桌面。见[操作入口](../../docs/windows-final-installer-acceptance.zh-CN.md) |
| Ubuntu DEB 安装生命周期 | **实际 DEB 静态审核、便携包和 Draft 回下载通过；用户六阶段待完成**。用户在 Ubuntu 24.04 x86_64 执行安装、可见 GUI、真实 URI 新窗口与目录标记、正常退出及官方卸载；助手核对本包程序链接清理、用户数据保留及全部回执。见[操作入口](../../docs/ubuntu-final-installer-acceptance.zh-CN.md)。旧 V01／V02／V05 不因此关闭 |
| 最近 GUI 改动的候选包冒烟 | **本候选签名 DMG 六项实际 GUI 和同包重启恢复通过**，见[新包结果](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/macos-gui/result.json)。旧包结果仅按[历史记录](#正式候选-macos-验收2026-10-14)保留，不重开旧 Linux CLI 认证矩阵 |

发行边界见[当前公开版的验证说明](https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/tag/v0.2026.10.07.01.09.oss_00)及[发布工作流](../../.github/workflows/infinishell_release.yml)；最近 GUI 改动的自动覆盖与人工未验范围见 [Zap 吸收报告](../../docs/upstream-essence/2026-10-07-zap.md)。Windows／Ubuntu 用户回执及助手审核关闭前，不宣称下一发行候选已完成验收；新产物仅允许写入 Draft，尚未公开发布。

本次状态更新前的完整入口保存在[收尾前历史快照](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/macos-full12-closeout-20261015/before-index.json)；本轮主线 `cargo check --locked` 已通过（`r-w61eogvo`，保留 5 条既有警告），结果见[验证收据](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/macos-full12-closeout-20261015/validation.json)。

新轮归档为 `/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/`，Windows 修复证据为 `/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/`。旧 `01.46` 全部证据仍在原目录；`22.25` 保留自动检查、预编译、DNS 失败和主动取消证据，共享初始化红／绿回归保存在 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-14-shared-init-fix/`。`16.19` 保留签名公证、六份 SSH 校验、22 张 GUI 原始截图及拒收结果；原 BYOP 修复门禁仍在 `/Volumes/SanDisk/InfiniShell-Archives/macos-gui-smoke/2026-10-13-byop-routing-fix/`。**无需本地化变更**；旧候选的双语实测只按原包计证。本轮 Ubuntu DEB 生命周期待用户验收，旧 CLI 认证矩阵延期，自动检查保留。

### 正式候选 macOS 验收（2026-10-14）

以下是旧 `01.46` 候选的通过记录，保留原证据范围；`13.00` 已被替代，`16.45` 的最新结论见本页顶部。

`v0.2026.10.14.01.46.oss_00` 的最终 DMG **19/19 签名、公证装订和静态检查通过；GUI 六项实际操作全部通过，独立审核通过**。DMG SHA-256 为 `c45d7b4eea75a2fc068ad818b816cd1ae1d59a43480bed3c5ec21e4f33440cad`，执行文件 SHA-256 为 `eb2ac5cc44736b4375a87b612a34112dca4bf8ee02afacd9d136576ed9e0bfc1`；Apple 公证 `7c979869-41d0-4f15-a6c5-636aa7981cc5` Accepted。

- 普通 Agent、CLI 接管、CLI 续轮、返回普通任务的实际请求按 **A→B→B→A** 路由；9 次带工具 Responses 请求均满足 strict/schema 判据，17 次总请求正常完成。
- 普通和备用屏幕的浮窗宽高、换行及内部滚动通过；同一签名包退出重启后实际恢复 **668 × 141.2999** 逻辑像素。窗口数据库行 ID 会在快照保存时重排，按稳定文档标识和实际窗口对应核对。
- Retina 2x 下连字开／关都完成图片覆盖区域点击、ABC123 输入、拖动选字及精确复制回送 `SELECT_ME_RETINA_06`。
- 英文／简体中文实际 HTTP 401 错误完整可读；真实 `create_documents` 创建的同一 Plan 重启恢复，两种语言的本地保存提示完整。

证据入口为本候选归档的 `macos-gui/result.json`、`macos-gui/evidence-index.json`、`macos-gui/r-own9vyp9/` 与 `macos-final/validation/result.json`；独立审查 `macos-gui/independent-audit/final-audit.json`（SHA-256 `0ba18d2b7add5a40c241ac6253d2cb937d5289d82acb4b3d30f6472c4815a1e8`）复核 90/90 原件摘要及 30 张截图通过。本轮使用本机固定模型触发真实 UI／HTTP／工具路径，不外推真实模型能力或成功云同步。临时 provider、模型、语言、连字和尺寸已恢复；settings.toml 完整摘要与私有基线相同，未全量覆盖数据库。GUI、后代和 provider 已退出，DMG 正常卸载；临时目录严格保留，不放宽清理规则。

另记录非阻塞改进：中文“连字”设置搜索没有结果，英文 `ligature` 可找到；此项单列后续本地化搜索优化。本轮未新增产品代码，**无需本地化变更**。

### 最终发行资产与 Windows 交付（2026-10-14）

以下保留旧 `01.46` 的交付时快照；后续 Windows URI 用户回执已使该候选拒收，不作为后续 `13.00` 或当前 `16.45` 的资产或生命周期结论。

正式 run `38444122158` attempt 1 全部成功，候选仍固定在源码 `5360f5d6c`。Draft `412209685` 精确包含 **12 件资产**：ARM64 macOS DMG、Windows x64 安装器、四份 Linux 包及六份 SSH 扩展。仅 DMG 和两份签名后的 Mac SSH 资源经本机补入；其余九件保持 CI 原字节。全量认证下载后，本地原件、GitHub 摘要和下载件逐项一致，标签、资产身份及公开 Latest 均未改变；没有公开发布。

Linux AppImage／DEB／RPM／Arch 均完成只读解包、元数据与 ELF 检查、内嵌六资源核对，以及本包二进制实际版本检查；没有安装系统包或执行 Linux GUI。Windows 安装器来源、大小、摘要、AMD64 主程序和内嵌原始六资源均通过，实际 Authenticode 为 **`NotSigned`**，按 OSS 未签名边界记录，不称为有效签名。

[Windows 独立操作文档](../../docs/windows-final-installer-acceptance.zh-CN.md)保留当时的包身份和材料；后续五阶段回执的安装、启动、退出、官方卸载分项通过，但 URI 仍指向旧失效目标。当前入口已切换到新候选的六阶段安排，不重开已关闭的 Windows 分支验收，也不将旧包静态结果充当新包安装卸载通过。

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
| 当前关闭、延期与发版必验清单 | [本页当前状态](/Volumes/SanDisk/github/InfiniShell-Desktop/specs/cli-agent-parity/README.md#当前主线状态2026-10-19)、[CURRENT_STATUS.json](/Volumes/SanDisk/github/InfiniShell-Desktop/specs/cli-agent-parity/CURRENT_STATUS.json) 的 `main_acceptance.release_integration_20261019` |
| 原关闭条件、原生差异与历史缺口 | [KNOWN_GAPS.md](KNOWN_GAPS.md)，按顶部当前入口区分历史快照 |
| 三方能力与消费者可用范围 | [CAPABILITY_MATRIX.md](CAPABILITY_MATRIX.md)、[RELEASE_SUPPORT.md](RELEASE_SUPPORT.md) |
| 自动升级、渠道与事务约定 | [CLI_AUTOUPDATE.md](CLI_AUTOUPDATE.md) |
| 已通过、失败、未验及源码／平台对应 | [VALIDATION_REPORT.md](VALIDATION_REPORT.md) |
| Linux 旧 CLI 认证矩阵延期与归档 | [本页历史归档决定](#linux-桌面验收延期与归档2026-10-13)，不包含本轮新增 DEB 生命周期验收 |
| Ubuntu 24.04 x86_64 最终 DEB 用户操作与助手审核 | [独立验收入口](../../docs/ubuntu-final-installer-acceptance.zh-CN.md)，等待最终包身份绑定后交付执行 |
| Windows 最终安装器用户操作与助手审核 | [独立验收入口](../../docs/windows-final-installer-acceptance.zh-CN.md)，等待最终包身份绑定后交付执行 |

`CODEX_PLUGIN_CACHE_REFRESH.md`、`PLUGIN_COMPATIBILITY.md`、`SSH_TMUX_VERIFICATION.md` 是配套插件说明正在引用的简短兼容入口，正文已收敛到上述文档。

## Linux 桌面验收延期与归档（2026-10-13）

以下保留 10 月 13 日的旧矩阵归档决定。10 月 14 日用户另行确认新候选 Ubuntu 24.04 x86_64 DEB 生命周期验收；该新增范围不再延期，旧 V01／V02／V05 认证矩阵继续按原决定保留。

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

当前机器可读结论位于 `CURRENT_STATUS.json` 的根级状态、`current_goal_scope`、`required_goal_merge_gate` 与 `main_acceptance.release_integration_20261019`；原 `current_release_candidate` 及验收对象保留原字节范围。原入口值保存在 `historical_entry_before_20261013`，原 Goal 大对象及合并门禁分别原样保留为 `historical_goal_scope_before_merge` 和 `historical_required_goal_merge_gate`；其余逐轮快照继续按各自来源阅读。本轮替换的旧当前摘要完整保存在 `historical_current_summaries_before_gui_closeout_20261013`。原始证据链接仅用于追溯，不能取代当前状态；后续更新结论仍需绑定实际实现和验证。
