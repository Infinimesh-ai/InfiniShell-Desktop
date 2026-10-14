# Windows 最终安装器验收

当前结论：**旧 `01.46` 候选的用户证据已复核，安装、GUI 启动、正常退出和官方卸载分项通过，但 URI 登记仍指向旧失效目标，阻塞发版。** 详见[审核报告](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.14.01.46.oss_00/native-validation/windows-user-review-20261014/README.zh-CN.md)。本轮修复安装登记、启动时恢复及按所有权卸载清理；新包和复验指令待验证完成后替换入口。保留用户数据，Linux 桌面继续延期。

## 旧 01.46 候选交付时快照（保留历史）

当前状态：**最终安装器静态核验已通过，便携验收包可交付；用户五阶段尚未执行**。候选为 `v0.2026.10.14.01.46.oss_00`，源码 `5360f5d6cf7e3b25263431027993a3a26dca9cb5`，annotated tag 对象 `45219f419c628bf3f5dbace5fd10013f6cf2c0c2`。本地门禁及[同 SHA 双平台门禁 38441190656](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38441190656)首次运行全部通过，主线和标签已推送；[正式发行构建 38444122158](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38444122158) attempt 1 已成功完成，Windows desktop producer 为 `115576837239`，artifact 为 `11860520494`。

正式原件已完成来源、provider 摘要、ZIP 完整性与唯一安装器核对；既有 Windows 节点实际读取 Authenticode 为 **`NotSigned`**，读取错误、签名证书与时间戳证书均为空。这是明确的 OSS 未签名状态，不代表有效数字签名。只读解包确认主程序为 AMD64，内嵌 CI 原 manifest 字节及六件资源摘要／大小全部匹配本 run 原件；没有安装或启动 GUI。

| 交付文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `InfiniShellSetup.exe` | 580408345 | `56276e369cf3bb85cec82165619a73e544cbf9f48932861e705f57a3f8feb5f6` |
| `InfiniShell-Windows-Acceptance-v0.2026.10.14.01.46.oss_00.zip` | 580483206 | `cef94e3ee7140f6d62af0f953bb5d827cba214f5dc11573f1d2f57e514b52073` |

CI 原 manifest SHA-256 为 `2abbe09e3a4b1557702fcd8003b19e06c45d4ac7ee9094ca01920013ded8c824`。候选绑定已填写实际包身份，采集器已启用；启用前材料和两项控制器问题的原始记录均保留。便携 ZIP 恰有 10 个成员，包含同字节 Setup、采集器、身份绑定、CI 原清单、中文说明及结果／审查材料，已完成 CRC 和逐成员摘要回验。用户完整解压即可使用现有 PowerShell 5.1 或 7，无需 Python、Git 或 Mac 路径；辅助 ZIP 不作为第 13 件 GitHub 发布资产。静态通过不代替用户生命周期回执，也不表示已经公开发布。

按用户安排，用户在自己的 Windows 开发电脑完成安装、真实 GUI 启动、正常退出和官方卸载，助手审核回执。无需另报机器名或账号，也不要求切换桌面。已有分支验收继续按原范围保留，不重开；它不能替代本次最终安装器的回执。

## 用户五阶段操作顺序

1. 收到最终包和发行方 SHA-256 后，在同一交互登录会话、同一普通 PowerShell 中采集 `baseline`。若已有会被覆盖的安装或集成，或旧 `warp\InfiniShell` 数据目录存在，先发回基线核对，不直接覆盖或卸载既有环境。
2. 按完整说明进行当前用户安装，记录安装目录、PATH 和快捷方式选项；完成后采集 `installed` 与安装完成截图。
3. 从本轮快捷方式启动应用，在空白终端输出候选版本标记，保存可见窗口截图并采集 `launched`；正常退出后采集 `closed`。不需要登录 CLI Agent。
4. 使用 Windows“已安装的应用”或本次安装目录的官方卸载器卸载，保存完成截图并采集 `uninstalled`。不以手工删文件、注册表或 PATH 代替卸载。
5. 交回身份绑定、五阶段 JSON、三张截图和填写后的结果表。助手先核对最终包身份和阶段完整性，再分别判定安装、启动、退出、卸载、安装集成残留及用户数据保留；读取错误和未知值不算通过。

保留用户配置、历史和凭据，不提交其内容。URI/AUMID 登记若残留，应单独据实记录，不能归为用户数据或预称“零残留”。完整命令、既有数据边界和异常处理见下列本轮交付材料；收到完整 ZIP 并核对身份后执行。

## 本轮交付材料

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

脚本准备和静态自检不等于 Windows 实测通过；若最终包散列变化，另建验收轮次并保留旧记录。本入口只覆盖当前用户安装生命周期，不自动关闭 CLI Agent 延期项或 Linux 桌面验收。Linux 桌面继续延期，Linux 自动检查保留。本次仅校正文档状态，**无需本地化变更**。
