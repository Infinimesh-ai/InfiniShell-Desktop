# 当前发布与验收范围

[English](README.md) · [文档索引](../README.zh-CN.md)

截至 2026-10-25，正式 Latest 为 [v0.2026.10.21.15.36.oss_00](https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/tag/v0.2026.10.21.15.36.oss_00)，于 2026-10-22 06:05:02（北京时间）公开。产品源码固定为 `255849d04ffe6a8645f7ec31f3cd6fd58004c4be`；后续文档整理不移动产品标签，也不代表新增产品验收。

| 用途 | 资产 |
| --- | --- |
| 桌面安装 | macOS Apple Silicon DMG、Windows x64 Setup、Linux x86_64 的 AppImage/DEB/RPM/Arch，共 6 件 |
| SSH 扩展 | macOS/Linux/Windows 各 x86_64 与 aarch64，共 6 件 |

Intel macOS 只提供 SSH 扩展，没有桌面 DMG。Windows Setup 实际为 `NotSigned`，不得描述为已签名。

## 验收边界

- 原批准的 CLI 合并范围 G01–G10、V03 共 11 项已关闭并合入主线；V01/V02/V05 的延期和限定范围保留，不表示全平台实机全部通过。
- 同一产品源码的完整预检：Linux 12,212 通过、123 跳过；Windows 11,932 通过、160 跳过。跳过不计通过。
- macOS 最终 DMG 的签名、公证、装订及限定英中 GUI 已验证；旧正式版发现本版、实际升级及持久化保留已有后续记录。受控 provider 场景不外推真实模型能力；升级不证明旧 shell/PTY/后台进程连续存活。
- 本轮 Windows 安装/PATH/非 BMP/卸载/GUI 与 Ubuntu GUI 按用户决定未补验，保持 `user_skipped/not_tested`。本版发现更未来版本的 macOS GUI 延期未验。审批仍挂起时输入 C 保持 `not_executed_not_waived`。
- Linux/Windows 最终 SSH 各 10 场景按原边界接纳。Windows 原 20 秒收尾失败保留；后续自然退出或端点消失不证明全部后代、句柄和临时目录已清理。
- 12 件资产的 Draft 回下载与公开后匿名完整下载核验已通过。既有 `cleanup_ready=false`、失败和未验记录不因文档整理而改变。

## 记录入口

- [机器可读当前摘要](../../specs/cli-agent-parity/CURRENT_STATUS.json)
- [CLI 能力与范围](../../specs/cli-agent-parity/README.md)
- [公共文档与私有证据约定](../evidence/README.zh-CN.md)

更新本页时同时更新英文版及机器摘要。旧 G09、Draft 和一次性平台验收流水不再进入公共 Git，原件保留本地备份；本页的未验和清理边界仍然有效。
