# 第九轮平台验证

提交 `e4738e1ea96ea727c15d51e2c31dbf34152814b6` 的 [Actions 运行](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35186353142)已结束。Windows 所选检查通过，Linux 失败，整体不计跨平台通过；`full_workspace_tests=false`。

Windows 通过 `cargo check`、warp 定向 1693 项、共享 AI 契约 71 项、CLI 参数 128 项、双语 TUI 9 项、进程归属 2 项、真实监督清理 4 项以及 rust-genai 81 项。原生插件缓存更新恢复、候选五项 hook 注册、两组真实通知经 ConPTY 传输、Claude 无凭据初始化/EOF/权限观测及 Grok 无凭据 ACP 边界均通过。Codex 缺失历史、CMD shim 缺失历史与空闲崩溃三项生产 Rust 适配器测试通过，未替换为新会话。

Windows 缺失会话的退出码 1 只在已确认 `windows_job` 清理且原因是 `StopRequested`/`StdioClosed` 时接受，不能推断原生正常退出；记录仍保留退出来源边界。ConPTY 证据的 scope 是候选配方传输，不是普通 Codex TUI、产品通知 UI 或模型生命周期。rev4 正式资源在后续修改中，未参加本轮。

Linux 在 Python 文件事务步骤失败：Windows 专用 `jq --binary` 原生用例误在 Linux 执行。该组 22 项中 21 项通过、1 项错误；后续 Rust 和原生 CLI 检查全部跳过，artifact 无文件错误是前置失败的后果。修复提交 `a245c639d` 仅把这一原生用例限定到 Windows，保留其他平台离线契约；修复后的 Linux 实跑仍待后续同提交验证。

三个 artifact 的原始文件、摘要及 GitHub job/step 状态保存在 [证据目录](validation/windows-ninth-e4738e1ea/manifest.json)。[Linux 失败记录](validation/linux-ninth-e4738e1ea.json)继续保留。所有无凭据结果均不计真实模型成功，跳过的完整工作区与 GUI 集成门禁不计通过。
