# Windows 最终安装器验收

当前状态：**正式发行构建已启动，等待最终安装器；用户尚未执行**。新候选为 `v0.2026.10.13.22.25.oss_00`，源码 `f429fe2c0426846f41f79fef15a791c5f7cbb011`，annotated tag 对象 `20260485d332089610007c6c1df1d84462093ef2`。同 SHA 的[双平台自动门禁 38411587441](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38411587441)首次运行成功后已推送 tag，[正式发行构建 38414535397](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38414535397) attempt 1 已启动；构建完成和包身份仍待核验。

新候选的 `InfiniShellSetup.exe` SHA-256／字节数和 CI 原 manifest SHA-256 均保持空值。助手将在正式新包产出后核对来源、SHA-256、字节数、实际 Authenticode 状态及 CI 原始六资源清单，再交付新包和采集材料。当前采集器仍拒绝执行，不应移除该限制或开始最终人工验收；不得预填新 SHA 或复用旧包回执。

按用户安排，用户在自己的 Windows 开发电脑完成安装、真实 GUI 启动、正常退出和官方卸载，助手审核回执。无需另报机器名或账号，也不要求切换桌面。已有分支验收继续按原范围保留，不重开；它不能替代本次最终安装器的回执。

## 新包交付后的操作顺序

1. 收到最终包和发行方 SHA-256 后，在同一交互登录会话、同一普通 PowerShell 中采集 `baseline`。若已有会被覆盖的安装或集成，或旧 `warp\InfiniShell` 数据目录存在，先发回基线核对，不直接覆盖或卸载既有环境。
2. 按完整说明进行当前用户安装，记录安装目录、PATH 和快捷方式选项；完成后采集 `installed` 与安装完成截图。
3. 从本轮快捷方式启动应用，在空白终端输出候选版本标记，保存可见窗口截图并采集 `launched`；正常退出后采集 `closed`。不需要登录 CLI Agent。
4. 使用 Windows“已安装的应用”或本次安装目录的官方卸载器卸载，保存完成截图并采集 `uninstalled`。不以手工删文件、注册表或 PATH 代替卸载。
5. 交回身份绑定、五阶段 JSON、三张截图和填写后的结果表。助手先核对最终包身份和阶段完整性，再分别判定安装、启动、退出、卸载、安装集成残留及用户数据保留；读取错误和未知值不算通过。

保留用户配置、历史和凭据，不提交其内容。URI/AUMID 登记若残留，应单独据实记录，不能归为用户数据或预称“零残留”。完整命令、既有数据边界和异常处理见下列新候选准备材料；执行须等待正式交付。

## 新候选准备材料

以下材料位于新候选的独立归档，旧结果没有复制为本轮结果。此时仅可审阅，不能执行验收：

- [完整用户操作说明](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/README.zh-CN.md)
- [PowerShell 证据采集器（尚未启用）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/collect-windows-evidence.ps1)
- [新候选身份绑定（run／安装器／清单待填）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/candidate-binding.json)
- [结果填写模板](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/RESULT.zh-CN.md)
- [助手证据审查清单](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/EVIDENCE-REVIEW.zh-CN.md)与[只读绑定预检器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.22.25.oss_00/native-validation/windows-manual/review-evidence-binding.py)

## 旧候选历史材料

旧候选 `v0.2026.10.13.16.19.oss_00`（源码 `36abcbb5b027c17a29fc4fb4a5f91b6ebbc2d680`）因两处已确认的 macOS/BYOP GUI 缺陷拒收，[发行构建 38368533631](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368533631)由主控主动取消，Windows 桌面 artifact 未产出；这不代表 Windows 编译失败。[关闭记录与原始日志索引](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-desktop/REPORT.zh-CN.md)和以下旧材料保留，不能作为新候选的执行材料：

- [完整用户操作说明](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/README.zh-CN.md)
- [PowerShell 证据采集器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/collect-windows-evidence.ps1)
- [旧候选身份绑定（已禁用，安装器 SHA-256／大小未填）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/candidate-binding.json)
- [结果填写模板](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/RESULT.zh-CN.md)
- [助手证据审查清单](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/EVIDENCE-REVIEW.zh-CN.md)与[只读绑定预检器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/review-evidence-binding.py)

脚本准备和静态自检不等于 Windows 实测通过；若最终包散列变化，另建验收轮次并保留旧记录。本入口只覆盖当前用户安装生命周期，不自动关闭 CLI Agent 延期项或 Linux 桌面验收。Linux 桌面继续延期，Linux 自动检查保留。本次仅校正文档状态，**无需本地化变更**。
