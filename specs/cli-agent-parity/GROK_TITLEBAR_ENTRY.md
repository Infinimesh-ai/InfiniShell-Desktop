# Grok 标题栏快捷入口

当前标题栏仅显示检测到安装且 per-agent titlebar 开启的 CLI。旧 `PerAgentSettings::default_for` 包含 Claude/Codex/Gemini/Antigravity，漏掉 Grok，因此未自定义设置的 Grok 默认隐藏。补入 Grok 后，安装检测通过即可默认显示，点击仍走既有普通终端启动；不开放未验证的托管执行。用户明确保存的隐藏设置继续优先。

沿用既有 Grok 图标、名称和“标题栏”设置，中英文均已有资源，无需本地化变更。新增图标的双语布局待包含修复的新包实测，不能用旧包截图代替。

冻结两文件输入同时修复 Windows jq 原生用例误在 Linux 执行的测试范围：只将原生文本/二进制输出测试限定到 Windows，不改变 Windows 参数或断言；其余离线契约在各平台运行。第九轮 Linux 的真实失败与后续 Rust 未执行情况见 `validation/linux-ninth-e4738e1ea.json`。

[门禁](validation/macos-grok-titlebar-gates.json)与[输入摘要](validation/macos-grok-titlebar-inputs.json)：check 62.036s、i18n 11 项通过、相关模块 1002 项通过；Windows hook 脚本21项通过、1项原生 Windows 用例按平台跳过。验证使用隔离工作区，不含用户原有网页搜索修改。
