# Ubuntu 最终 DEB 桌面验收

当前结论：**用户已确认在 Ubuntu 24.04 x86_64 原生桌面执行验收，并等待统一修复后的新候选，再各做一次 Windows 和 Ubuntu 复验。** 统一候选 `v0.2026.10.14.16.45.oss_00` 正式构建已成功并产出 Linux artifact；归档下载正在恢复，实际 DEB 静态审核、验收 ZIP 和任务指令尚未交付；本页是稳定状态入口，不是可以直接执行的安装材料。

Linux 桌面已从本轮延期调整为“用户在现有 Ubuntu 桌面执行、助手审核证据”。Linux 自动检查继续保留。旧 Linux 分支的归档、关闭和延期记录仍作为历史保存；本次不恢复旧开发分支，不重开已关闭的 G09，也不把在线模型、认证 CLI Agent 的跨平台矩阵重新纳入这次包生命周期验收。

## 候选范围与当前阻塞

旧候选 `v0.2026.10.14.13.00.oss_00` 固定源码 `5acb349c7032c0fc07ba4fbbfaba93d87fa6407b`，其 Linux DEB 打包路径遗漏了 postrm 的 `@@PACKAGE_NAME@@` 替换，卸载可能保留 `/usr/bin/infinishell` 失效链接。该候选不作为本轮 Windows/Ubuntu 统一用户复验的交付版本；[原正式构建 38514368665](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38514368665)及[候选归档](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.13.00.oss_00/candidate.json)只用于追溯，不能通过手动删链接或改旧包掩盖缺陷。

新产品候选以 `5acb349c7` 已验证的 Windows URI 修复为基础，仅加入本轮必要的 Linux 打包/卸载修复，固定源码为 `5068dae9c75b79628865a0872684113647c204d8`；annotated tag 为 `v0.2026.10.14.16.45.oss_00`，tag 对象为 `27d40029759b541dc8903a0deb9ec0948e9670fa`。[正式构建 38537451605](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38537451605) attempt 1 全部成功；身份以[新候选归档](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/candidate.json)为准。Linux 源码门禁通过，依据见[源码验证记录](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/linux-deb-cleanup-fix/source-gates.json)；最终包的静态核验和真实桌面六阶段仍待完成。

修复已整合至主线及验收控制器 `203d08e5053413d5d6a14971208de4e125eb2f1c`，这不是候选产品 SHA。主线后续 `b4e59a8cd`（CLI 启动器打开富输入等 GUI 改动）不进入这轮产品候选。新包大小、摘要和生产工件身份仍须从实际产物核验，不能借用旧资产。Windows 原生门禁结果属于 `5acb349c7`，新候选保持相同 Windows 实现字节，本轮最终安装器已另经原生静态审核，但 Windows 用户六阶段仍待验；旧 macOS/12 资产通过记录也只属于旧候选。

| 交付项 | 当前状态与关闭条件 |
|---|---|
| 新候选产品 SHA、tag、tag 对象、正式 run | 已按上述身份冻结，正式构建成功；保留旧 tag 和旧证据 |
| 官方 Ubuntu amd64 DEB | artifact `11903140677` 已产出；下载恢复中，尚待确认包元数据、程序、desktop、六份 SSH 资源、安装/卸载脚本 |
| 用户验收 ZIP 与 SHA-256 | 待正式 DEB 静态审核通过；与 Windows 使用同一候选源码/tag |
| 用户任务指令 | 模板已准备；待填真实附件名、版本和 ZIP 摘要后交付 |
| Ubuntu 现场验收 | 尚未执行；六阶段证据和必要截图待用户回传、助手复核 |
| 发版结论 | 保持未放行；静态包核验或采集脚本通过不代替真实桌面验收 |

首次下载在 900 秒达到传输超时，约 1.13 GB 部分原件及错误日志已保留；正在执行唯一一次 Linux 专用 7200 秒恢复，来源、provider 摘要、ZIP 及内容核验未放宽。见[下载恢复记录](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/linux-download-recovery/state.json)。未完成下载不算正式 DEB 静态审核通过，也不是产品构建失败。

## 六阶段与操作边界

目标限定为 Ubuntu 24.04 x86_64 的官方 DEB：包名 `infinishell`，主程序 `/opt/infinishell/infinishell`，安装脚本创建 `/usr/bin/infinishell` 链接，desktop 为 `dev.infinishell.InfiniShell.desktop`，协议为 `x-scheme-handler/infinishell`。本次通过不会扩大成其他 Linux 发行版、ARM64 或 RPM/AppImage/Arch 桌面均已通过。

| 阶段 | 用户操作与必要证据 |
|---|---|
| `baseline` | 普通用户本地桌面、Ubuntu/架构、原安装状态、URI 默认处理项和用户目录存在性；发现既有安装或冲突时保留现场待审核 |
| `installed` | 用户安装交付的官方 DEB，首次打开前记录包版本/架构、已装主程序和六资源摘要、链接与 desktop |
| `launched` | 从桌面应用列表启动真实 GUI，核对进程、可交互 shell、英中文案、真实中文输入法及基本标签页/窗口/设置观察 |
| `uri_activated` | 经系统 `xdg-open` 实际打开 `infinishell://action/new_window?path=...`，新窗口 `pwd` 匹配本轮唯一目录标记并截图 |
| `closed` | 正常退出全部窗口，采集无残留候选进程；不能用强杀代替正常退出 |
| `uninstalled` | 用户仅移除本轮包，采集包文件、程序链接、desktop、资源及进程清理结果；用户数据目录允许保留 |

用户操作采用正式包内的独立任务说明和采集器。采集器不自动安装/卸载、不强写 MIME 默认关联、不读取配置或数据库内容；只记录用户目录存在性，不能据此宣称配置内容没有变化。已有配置可保留；已有安装、冲突命令或旧关联需要先审核，不擅自升级或卸载用户原版本。卸载使用 `apt remove`，不使用 purge/autoremove，也不为了“通过”删除残留文件。

Linux desktop/MIME 机制按现场事实判断：新安装的默认处理项可以为空，真实 URI 唤起仍需观察；若出现系统选择应用提示，记录本次打开行为，不强改默认项。`xdg-open` 返回成功、进程存在或 MIME 查询结果都不能代替“新窗口和正确工作目录”的截图。英文、简体中文、物理输入法未执行时必须明确记录缺证据，不能用粘贴文字代替输入法实测；不要求登录模型服务。

## 证据索引与后续更新

以下均为发行方准备和历史资料，不能原样充当正式用户验收包：

- [Ubuntu 材料准备结果与摘要索引](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/ubuntu-handoff-preparation/handoff-preparation-result.json)：采集/复核/打包工具及 58 项合成检查；真实 Ubuntu GUI 未执行。
- [Ubuntu 静态包审核入口](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/ubuntu-package-preparation/README.zh-CN.md)：正式 artifact、DEB、内嵌资源与卸载脚本核验规则。
- [最终材料绑定与生成步骤](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/ubuntu-handoff-preparation/RELEASE-PREPARATION.zh-CN.md)：只接受新正式候选及成功静态审核，生成 ZIP、独立任务和回读校验 receipt。
- [用户回执审核规则](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/ubuntu-handoff-preparation/EVIDENCE-REVIEW.zh-CN.md)：六阶段身份/摘要/时序链、实际 URI、人工截图和残留判断。
- [Windows 对应验收入口](windows-final-installer-acceptance.zh-CN.md)：独立平台证据，同一最终候选。

新候选身份已固定。实际包核验完成后，本页再补正式 DEB 摘要、最终验收 ZIP/任务链接及证据归档位置；当前这些交付项仍待完成。用户回传后再补实际审核结论，保留失败样本、原始日志和每轮证据；不将“工具准备完成”“源码修复通过”“用户已执行”合并写成“发版通过”。本次文档状态说明**无需本地化变更**。
