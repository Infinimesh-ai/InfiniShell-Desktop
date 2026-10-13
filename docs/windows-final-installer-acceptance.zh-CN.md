# Windows 最终安装器验收

当前状态：**等待最终安装器，用户尚未执行**。本轮候选为 `v0.2026.10.13.16.19.oss_00`，源码 `36abcbb5b027c17a29fc4fb4a5f91b6ebbc2d680`，[发行构建 38368533631](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38368533631)仍在运行。最终 `InfiniShellSetup.exe` 的 SHA-256 和字节数尚未绑定；助手核对并填齐后，再将最终包、校验值及采集材料交付用户执行。

按用户安排，用户在自己的 Windows 开发电脑完成安装、真实 GUI 启动、正常退出和官方卸载，助手审核回执。无需另报机器名或账号，也不要求切换桌面。已有分支验收继续按原范围保留，不重开；它不能替代本次最终安装器的回执。

## 操作顺序

1. 收到最终包和发行方 SHA-256 后，在同一交互登录会话、同一普通 PowerShell 中采集 `baseline`。若已有会被覆盖的安装或集成，或旧 `warp\InfiniShell` 数据目录存在，先发回基线核对，不直接覆盖或卸载既有环境。
2. 按完整说明进行当前用户安装，记录安装目录、PATH 和快捷方式选项；完成后采集 `installed` 与安装完成截图。
3. 从本轮快捷方式启动应用，在空白终端输出候选版本标记，保存可见窗口截图并采集 `launched`；正常退出后采集 `closed`。不需要登录 CLI Agent。
4. 使用 Windows“已安装的应用”或本次安装目录的官方卸载器卸载，保存完成截图并采集 `uninstalled`。不以手工删文件、注册表或 PATH 代替卸载。
5. 交回身份绑定、五阶段 JSON、三张截图和填写后的结果表。助手先核对最终包身份和阶段完整性，再分别判定安装、启动、退出、卸载、安装集成残留及用户数据保留；读取错误和未知值不算通过。

保留用户配置、历史和凭据，不提交其内容。URI/AUMID 登记若残留，应单独据实记录，不能归为用户数据或预称“零残留”。完整的命令、既有数据边界和异常处理见下列本轮材料。

## 本轮材料

以下原件统一位于闪迪归档的 `native-validation/windows-manual/`，不复制到仓库：

- [完整用户操作说明](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/README.zh-CN.md)
- [PowerShell 证据采集器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/collect-windows-evidence.ps1)
- [候选身份绑定（等待最终 SHA-256／大小）](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/candidate-binding.json)
- [结果填写模板](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/RESULT.zh-CN.md)
- [助手证据审查清单](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/EVIDENCE-REVIEW.zh-CN.md)与[只读绑定预检器](/Volumes/SanDisk/InfiniShell-Releases/v0.2026.10.13.16.19.oss_00/native-validation/windows-manual/review-evidence-binding.py)

脚本准备和静态自检不等于 Windows 实测通过；若最终包散列变化，另建验收轮次并保留旧记录。本入口只覆盖当前用户安装生命周期，不自动关闭 CLI Agent 延期项或 Linux 桌面验收。Linux 桌面继续延期，Linux 自动检查保留。本次仅校正文档状态，**无需本地化变更**。
