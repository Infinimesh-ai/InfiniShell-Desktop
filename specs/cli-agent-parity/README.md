# 三款 CLI 支持与能力对齐

本目录维护 Codex CLI、Claude Code 和 Grok Build 的支持范围、能力差异与验证约定。当前发行版本及验收边界统一见[发布入口](../../docs/releases/README.zh-CN.md)，机器摘要见 [CURRENT_STATUS.json](CURRENT_STATUS.json)。

## 当前主线状态

原批准合并范围 G01–G10、V03 已关闭并进入主线。完整的跨平台模型生命周期、输入法及真实审批布局仍有延期项；功能合并不等于全部平台、CLI 版本和组合均已实测。未验范围见[当前限制](KNOWN_GAPS.md)。

| 需要了解的内容 | 入口 |
| --- | --- |
| 三款 CLI 的协议、输入、技能与审批差异 | [能力矩阵](CAPABILITY_MATRIX.md) |
| 安装、通知插件、更新渠道与失败回退 | [支持说明](RELEASE_SUPPORT.md) |
| 当前未验范围与持续兼容边界 | [当前限制](KNOWN_GAPS.md) |
| 验证层级、门禁和证据要求 | [验证约定](VALIDATION_REPORT.md) |
| CLI 自动升级来源与事务合同 | [自动升级](CLI_AUTOUPDATE.md) |
| 通知插件与 Codex 持久来源 | [插件兼容](PLUGIN_COMPATIBILITY.md)、[Codex 来源](CODEX_PLUGIN_CACHE_REFRESH.md) |
| 远程 CLI 与 tmux | [SSH/tmux 验证范围](SSH_TMUX_VERIFICATION.md) |
| Windows/Ubuntu 安装生命周期 | [Windows 流程](../../docs/windows-final-installer-acceptance.zh-CN.md)、[Ubuntu 流程](../../docs/ubuntu-final-installer-acceptance.zh-CN.md) |

普通终端、托管任务和通知插件分别判断就绪。安装成功不表示登录成功，ACK 不表示模型完成，进程退出不表示全部后代和资源已清理。未知版本或来源保留安全回退，不自动开放未经验证的能力。

固定协议夹具位于 `fixtures/`，是回归输入；不要把夹具通过、静态检查或模拟事件当成真实模型与桌面验收。
