# Windows 最终安装器验收

当前结论：**Windows URI 与 Linux DEB 卸载源码修复已验证；统一 `16.45` 候选正在构建，最终安装器和用户六阶段复验仍待完成。** 产品源码为 `5068dae9c75b79628865a0872684113647c204d8`，主线整合及只读核验控制器为 `203d08e5053413d5d6a14971208de4e125eb2f1c`；独立 GUI 改动 `b4e59a8cda491ba1e6430175b924b07101dbea54` 不属于本候选。

Windows 实现字节与已验证的 `5acb349c7032c0fc07ba4fbbfaba93d87fa6407b` 一致，见[Git 对照](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/github/windows-source-equivalence.json)。[Windows 门禁 38512735757](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38512735757)的 80 项 Inno 断言、6 项隔离注册表回归、616 项应用测试（含该 6 项）、492 项 crate、528 项 Zap、11 项 completer 保留原 5ac 范围，不称为 5068 新运行。5068 的[Linux 原生门禁 38534655990](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38534655990)和本机 cargo check／i18n 12/12 已通过；源码通过不代替最终安装器。

候选为 `v0.2026.10.14.16.45.oss_00`，annotated tag 对象 `27d40029759b541dc8903a0deb9ec0948e9670fa`；[正式 run 38537451605](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38537451605)正在构建。Setup SHA-256、大小、实际 Authenticode、静态报告和便携 ZIP 均待实际产出后绑定，不能借用旧包摘要或 `NotSigned`。新归档入口为 [candidate.json](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.16.45.oss_00/candidate.json)，尚未交付或启用新包执行材料。

`13.00` 因 DEB 卸载遗漏包名替换被统一修复候选替代，[run 38514368665](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38514368665)已主动取消，原件保留，不记为 Windows 编译失败。旧 `01.46` 的[用户证据审核](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-user-review-20261014/README.zh-CN.md)确认安装、GUI 启动、正常退出和官方卸载分项通过，但 URI 仍指旧失效目标而整体拒收。旧 macOS 和 12 资产结果仅属于原字节。

用户确认等待统一修复候选，在 Windows 开发电脑和 [Ubuntu 24.04 x86_64](ubuntu-final-installer-acceptance.zh-CN.md) 各验一次；新 Ubuntu DEB 生命周期不再延期，旧 CLI 认证矩阵继续延期。两份新包与回执、macOS 最终包及 12 资产核验未完成前不宣称发版验收完成。

## 新候选六阶段安排（待最终包交付）

正式 run 全部成功、最终 Setup 静态核验及身份绑定通过后，交付含同字节安装器和采集材料的便携 ZIP；用户使用现有 PowerShell 5.1 或 7，无需 Git 或 Python。执行和审查顺序为：

1. `baseline`：记录本轮基线，先核对既有安装与 URI/AUMID，不覆盖或清理归属未知的环境。
2. `installed`：完成当前用户安装，核对安装器首次登记及本实例路径。
3. `launched`：从本轮快捷方式启动可见 GUI，采集本候选进程和窗口证据。
4. `uri_activated`：通过交付的激活脚本实际打开 `infinishell://action/new_window?path=...`；记录唯一请求、对应 launched 摘要、返回时间、新窗口和目录标记，不能只读注册表替代真实唤起。
5. `closed`：正常关闭本轮所有产品窗口和进程，采集退出证据。
6. `uninstalled`：使用官方卸载器，核对本实例 URI/AUMID、PATH、快捷方式和卸载项，保留用户数据与被其他实例接管的关联；助手审核全部原始回执后判定。

新材料的准备记录见[修复归档](/Volumes/SanDisk/InfiniShell-Archives/windows-uri-fix/2026-10-14/handoff-preparation/README.zh-CN.md)，仍是待绑定模板，不能直接执行。用户无需另报机器名或账号，不要求切换桌面；已有分支验收不重开，也不替代最终安装器六阶段回执。

## 旧 01.46 候选交付时快照（保留历史）

交付时状态：**最终安装器静态核验已通过，便携验收包可交付；当时用户五阶段尚未执行**。以下包身份和操作材料仅供追溯，不用于新候选执行。候选为 `v0.2026.10.14.01.46.oss_00`，源码 `5360f5d6cf7e3b25263431027993a3a26dca9cb5`，annotated tag 对象 `45219f419c628bf3f5dbace5fd10013f6cf2c0c2`。本地门禁及[同 SHA 双平台门禁 38441190656](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38441190656)首次运行全部通过，主线和标签已推送；[正式发行构建 38444122158](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38444122158) attempt 1 已成功完成，Windows desktop producer 为 `115576837239`，artifact 为 `11860520494`。

正式原件已完成来源、provider 摘要、ZIP 完整性与唯一安装器核对；既有 Windows 节点实际读取 Authenticode 为 **`NotSigned`**，读取错误、签名证书与时间戳证书均为空。这是明确的 OSS 未签名状态，不代表有效数字签名。只读解包确认主程序为 AMD64，内嵌 CI 原 manifest 字节及六件资源摘要／大小全部匹配本 run 原件；没有安装或启动 GUI。

| 交付文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `InfiniShellSetup.exe` | 580408345 | `56276e369cf3bb85cec82165619a73e544cbf9f48932861e705f57a3f8feb5f6` |
| `InfiniShell-Windows-Acceptance-v0.2026.10.14.01.46.oss_00.zip` | 580483206 | `cef94e3ee7140f6d62af0f953bb5d827cba214f5dc11573f1d2f57e514b52073` |

CI 原 manifest SHA-256 为 `2abbe09e3a4b1557702fcd8003b19e06c45d4ac7ee9094ca01920013ded8c824`。候选绑定已填写实际包身份，采集器已启用；启用前材料和两项控制器问题的原始记录均保留。便携 ZIP 恰有 10 个成员，包含同字节 Setup、采集器、身份绑定、CI 原清单、中文说明及结果／审查材料，已完成 CRC 和逐成员摘要回验。用户完整解压即可使用现有 PowerShell 5.1 或 7，无需 Python、Git 或 Mac 路径；辅助 ZIP 不作为第 13 件 GitHub 发布资产。静态通过不代替用户生命周期回执，也不表示已经公开发布。

按用户安排，用户在自己的 Windows 开发电脑完成安装、真实 GUI 启动、正常退出和官方卸载，助手审核回执。无需另报机器名或账号，也不要求切换桌面。已有分支验收继续按原范围保留，不重开；它不能替代本次最终安装器的回执。

### 旧包用户五阶段操作顺序

1. 收到最终包和发行方 SHA-256 后，在同一交互登录会话、同一普通 PowerShell 中采集 `baseline`。若已有会被覆盖的安装或集成，或旧 `warp\InfiniShell` 数据目录存在，先发回基线核对，不直接覆盖或卸载既有环境。
2. 按完整说明进行当前用户安装，记录安装目录、PATH 和快捷方式选项；完成后采集 `installed` 与安装完成截图。
3. 从本轮快捷方式启动应用，在空白终端输出候选版本标记，保存可见窗口截图并采集 `launched`；正常退出后采集 `closed`。不需要登录 CLI Agent。
4. 使用 Windows“已安装的应用”或本次安装目录的官方卸载器卸载，保存完成截图并采集 `uninstalled`。不以手工删文件、注册表或 PATH 代替卸载。
5. 交回身份绑定、五阶段 JSON、三张截图和填写后的结果表。助手先核对最终包身份和阶段完整性，再分别判定安装、启动、退出、卸载、安装集成残留及用户数据保留；读取错误和未知值不算通过。

保留用户配置、历史和凭据，不提交其内容。URI/AUMID 登记若残留，应单独据实记录，不能归为用户数据或预称“零残留”。完整命令、既有数据边界和异常处理见下列本轮交付材料；收到完整 ZIP 并核对身份后执行。

### 旧包交付材料

以下材料位于本候选的独立归档，旧结果没有复制为本轮结果。用户执行五阶段后，助手再审核实际回执：

- [完整便携验收 ZIP（含同字节安装器）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/handoff/InfiniShell-Windows-Acceptance-v0.2026.10.14.01.46.oss_00.zip)与[组装回验收据](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/handoff/assembly-result.json)
- [完整用户操作说明](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/README.zh-CN.md)
- [PowerShell 证据采集器（已启用）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/collect-windows-evidence.ps1)
- [候选身份绑定（实际安装器／CI 原清单已核验）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/candidate-binding.json)
- [结果填写模板](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/RESULT.zh-CN.md)
- [助手证据审查清单](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/EVIDENCE-REVIEW.zh-CN.md)与[只读绑定预检器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-manual/review-evidence-binding.py)
- [本轮安装器只读核验报告](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-desktop/REPORT.zh-CN.md)与[证据索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-desktop/evidence-index.json)

## 旧候选历史材料

上一候选 `v0.2026.10.13.22.25.oss_00`（源码 `f429fe2c0426846f41f79fef15a791c5f7cbb011`）因后补共享初始化入口回归失败拒收，[发行构建 38414535397](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38414535397)由主控主动取消。Windows 两种架构 SSH 编译成功，但桌面安装器未产出，也未执行本候选 Windows 原生或生命周期验收；这不代表 Windows 编译失败。[Windows 关闭记录](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-desktop/CLOSEOUT.zh-CN.md)和[弃用材料](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/README.zh-CN.md)保留，不得用于新候选验收。

旧候选 `v0.2026.10.13.16.19.oss_00`（源码 `36abcbb5b027c17a29fc4fb4a5f91b6ebbc2d680`）因两处已确认的 macOS/BYOP GUI 缺陷拒收，[发行构建 38368533631](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368533631)由主控主动取消，Windows 桌面 artifact 未产出；这不代表 Windows 编译失败。[关闭记录与原始日志索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-desktop/REPORT.zh-CN.md)和以下旧材料保留，不能作为新候选的执行材料：

- [完整用户操作说明](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/README.zh-CN.md)
- [PowerShell 证据采集器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/collect-windows-evidence.ps1)
- [旧候选身份绑定（已禁用，安装器 SHA-256／大小未填）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/candidate-binding.json)
- [结果填写模板](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/RESULT.zh-CN.md)
- [助手证据审查清单](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/EVIDENCE-REVIEW.zh-CN.md)与[只读绑定预检器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/review-evidence-binding.py)

脚本准备和静态自检不等于 Windows 实测通过；若最终包散列变化，另建验收轮次并保留旧记录。本入口只覆盖当前用户 Windows 安装生命周期；Ubuntu DEB 生命周期由独立文档管理，两者都不自动关闭旧 CLI Agent 延期矩阵。Linux 自动检查保留。本次仅校正文档状态，**无需本地化变更**。
