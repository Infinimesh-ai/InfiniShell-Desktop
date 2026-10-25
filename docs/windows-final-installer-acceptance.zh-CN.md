# Windows 安装器验收流程

本页是可复用流程，不指定旧候选或待执行任务。当前版本及实际已验/未验范围见[发布入口](releases/README.zh-CN.md)。新一轮开始前绑定真实安装包、源码、SHA-256、架构、运行身份与操作范围；同版本号不能代替同字节身份。

## 准备

- 确认来源、CI run/attempt、安装器版本、AMD64 架构、内嵌资源和实际 Authenticode 状态。`NotSigned` 明确记录为未签名。
- 静态核验入口为 [verify_release_installer.ps1](../script/windows/verify_release_installer.ps1)，由[跨平台 preflight](../.github/workflows/cross-platform-preflight.yml) 的 `release-installer:<runid>` 模式运行；它不执行安装或 GUI。
- 使用普通用户现有 Explorer 桌面，安装器、快捷方式、PowerShell 采集和 URI 激活保持同一执行宿主，见[宿主一致性](windows-acceptance-host-context.zh-CN.md)。
- 先核对已有安装、URI/AUMID、用户/系统 PATH、快捷方式和用户数据目录。归属不明时保留现场；不覆盖唯一用户数据，不为验收改变未知登记。

## 六阶段

| 阶段 | 操作与必要检查 |
| --- | --- |
| `baseline` | 记录安装前身份、集成项与 PATH 原始存在性、类型和字节 |
| `installed` | 完成当前用户安装，核对版本、来源、安装路径、卸载入口及本实例登记 |
| `launched` | 从本轮快捷方式启动可见 GUI，确认 shell 可交互，记录进程和窗口 |
| `uri_activated` | 通过系统 Shell 打开 `infinishell://action/new_window?path=...`，确认新窗口与唯一请求目录一致 |
| `closed` | 正常退出全部本轮窗口，核对相关进程及监督资源；不以强杀代替正常退出 |
| `uninstalled` | 使用本轮官方卸载器，核对 URI/AUMID、PATH、快捷方式、卸载项及文件归属清理，保留用户数据 |

原始阶段快照与后续审核结论分开保存。读取错误或未知值不能记为通过；注册表键存在、ShellExecute 成功或安装器退出 0 都不能替代实际 GUI/URI 观察。

## PATH 与范围

PATH 需要比较原始字节、注册表类型和存在状态，不只比较展开字符串。系统 PATH 应保持不变；用户编辑或其他实例接管时，只处理可确认归属的内容，不能为通过而强行恢复整个环境变量。

OneDrive 快捷方式需要实际可读的目标与 AUMID 证据；采集器不能因此放宽证据目录的重解析点保护。管理员全用户安装、并发安装、任意覆盖升级和旧卸载器混用均须独立验证，不能由一次当前用户安装推出。

包字节变化后重新绑定并补验受影响路径。安装器重发另见[重发约定](windows-installer-reissue.zh-CN.md)。本机证据按[证据约定](evidence/README.zh-CN.md)保存，临时目录清理遵守[安全门禁](local-test-storage.zh-CN.md)。
