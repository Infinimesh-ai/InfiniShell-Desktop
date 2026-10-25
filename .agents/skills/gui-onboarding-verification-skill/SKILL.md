---
name: gui-onboarding-verification-skill
description: 验证指定 InfiniShell GUI 候选的首次安装与引导流程，记录版本、平台和截图证据。适用于本项目桌面引导验收，不适用于上游 Warp 登录流程或 TUI。
---

# InfiniShell 桌面引导验收

以用户指定的 InfiniShell 包、版本和平台为受测对象。先核对候选来源、tag/源码 SHA、包 SHA-256 与应用身份；未指定版本时先从当前发布记录确认目标，不能自行替换成上游 Warp stable。

## 环境与范围

- GUI 入口位于 `app/`；TUI 使用 `tui-verify-change`，不能用桌面截图代替 TUI 验收。
- 本机执行前读取根 `AGENTS.md` 和适用的 `.local/README.md`；保留已有用户配置，使用已授权的隔离测试环境。
- 需要远程自动检查时，使用本仓库 `cross-platform-cloud-verification` skill 和 `.github/workflows/cross-platform-preflight.yml`。CI 构建、无显示环境或静态检查不能替代真实桌面引导验收。
- 当前项目的 OSS 流程不要求 Warp 账户。不要安装上游 Warp、调用 Oz、获取内部测试 token，或引入登录/云端流程作为本项目的验收前置条件。
- 若缺少目标平台的交互桌面，记录未覆盖范围并提供具体操作清单；其他平台的结果只能作为补充证据。

## 执行与记录

1. 记录安装前状态、候选身份与实际平台。只执行任务已授权的安装、启动和卸载操作；不得覆盖唯一的用户数据。
2. 按实际首次启动界面走到可用终端，在每个可见引导步骤以及最终终端状态保留截图和操作说明。
3. 验证当前候选真实提供的引导分支。若任务涉及中英文引导，分别检查两种语言的文案、截断、换行与控件尺寸。
4. 按问题或已观察到的分支补充最小必要测试，避免为了上游已移除的账户流程创建额外任务。
5. 验收安装生命周期时，分别记录 `baseline → installed → launched → uri_activated → closed → uninstalled`。未执行的阶段写明未执行；不能由安装器退出码推断 GUI、URI 或卸载通过。

报告绑定到同一候选，列出实际步骤、证据索引、发现和未覆盖项。截图和日志按本机证据规则保存，脱敏后才可作为共享材料。本 skill 不授权推送、发布 Release、上传私有原始证据或向他人发送消息。
