# 第十轮跨平台验证

同一提交 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 的 [GitHub Actions 第十轮](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35190750961)已成功：Linux x64、Windows x64 所选门禁均通过，`full_workspace_tests=false`。该提交的 macOS 干净验证树也通过 check、国际化及相关模块测试，见 [macOS 门禁](validation/macos-b3d8-clean-gates.json)。

原始平台产物、步骤结果与文件摘要归档于 [tenth-b3d8](validation/tenth-b3d8/manifest.json)。Windows 原始 CRLF 和终端字节保持不变，目录局部 `.gitattributes` 明确禁止文本规范化，避免破坏摘要证据。

## 新增的真实结论

- 正式 Codex rev4 的五项 hooks 在 Windows 原生 0.147.0 中注册成功；两组包含空格、中文和 shell 特殊字符的路径都通过。原生授权前后的哈希一致，临时授权配置已回滚，未调用模型。
- 正式 Windows hooks 的 `SessionStart`、`UserPromptSubmit` 通知确实到达 ConPTY 字节流，不再仅以 hook 返回或独立控制台标记证明传输。其余事件的运行实效不由本探针代替。
- Linux 专用 jq 用例的范围修复已通过；本轮流程中的固定 CLI、原生初始化、插件事务、进程监督及恢复失败门禁均按各自范围保留证据。

## 未扩大结论

本轮没有运行全工作区门禁，未证明 Windows 普通 Codex TUI、三款 CLI 的官方在线完整生命周期、GUI 双语布局、SSH/tmux 全流程或应用重启后全部恢复分支。构建 b3d8 也不包含后续未提交的 Claude 固定权限策略、Grok 最终结果核验和新布局修复；这些改动需在包含实际代码的提交上重新验证。
