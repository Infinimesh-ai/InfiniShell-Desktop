# 第五轮 Windows 验证：94a412eb

[运行 35122575386](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35122575386) 验证精确提交 `94a412eb89a4de57977072e6c45f4a692955d970`，参数为 Windows=true、Linux=false、full_workspace_tests=false。Windows job `104883726502` 在 2026-09-16 16:32:48–16:40:50 UTC 执行，最终 **failure**；Linux 按参数 skipped。完整步骤、产物摘要及脱敏原生结果见 [JSON 快照](validation/fifth-platform-run-94a412eb.json)。[第四轮报告](FOURTH_PLATFORM_RUN_6635F9869.md) 保留其 Linux 成功和 Windows 两项失败，不能合并不同提交为最终三平台通过。

## Codex 候选 launcher 的原生通过项

固定 Codex 0.147.0 Windows x64 文件为 298668336 字节，SHA-256 `935a1911ed2556e4ffcec995f4886ac2ac425863ba26fed264df62e30272ad9d`。候选 PowerShell 明文 LF/UTF-8 SHA-256 为 `327603d72b007e9ea68a3902cee3f8b4cda4ebd70c6729826f58de79651d8952`。

- Windows PowerShell 5.1 语法、原生 argv 往返、严格原始 stdin/stdout 字节断言通过。
- 完整 `cmd.exe` 8191 字符边界实际执行通过；8192 字符在执行前拒绝。五条生成命令长度为 7214–7250 字符。
- 中文/空格路径和含单引号、`$()`、反引号、`&`、`%PATH%`、`!name!`、`^`、括号的路径两个原生 case 均通过，未生成注入哨兵文件。
- 两个私有插件均经真实原生安装与 `hooks/list` 校验：五项原插件定义加一项独立测试阻断定义，只有本次精确审阅的临时定义被授权。`SessionStart`、`UserPromptSubmit` 原脚本各执行一次并完成，独立 `UserPromptSubmit` 阻断返回 `stopped`。不是全部五种事件都已执行。
- 原生 session/turn ID 与脚本捕获的输入对应，中文多行 prompt 保持完整，`PLUGIN_ROOT` 指向实际已安装副本。原脚本内容、来源输入和执行前配置保持不变。
- 两个 case 的本机拒绝 provider 实测请求数均为 0，整体 `model_http_requests=[]`。这是固定受阻输入，未运行模型任务。

本轮 `native_conpty_notifications_verified=false`、`full_lifecycle_verified=false`、`model_generation_verified=false` 保持不变。原生 hook 完成与正确输入不能证明 `/dev/tty` 经 Windows ConPTY 把 OSC 交给应用；生产自动安装 gate 仍未开放。第四轮没有记录实际差异字节，不能倒推它已直接测得 BOM；本轮只能证明继承 stdin 的候选实现满足严格断言。

## Grok 首个直接失败

固定 Windows Grok 1.0.30 的下载、150036808 字节、SHA-256 `ca24ea63272ba7881261f4a52498d1f5bd884b01da25845990422a10dd315266`、PE32+ x86-64 和原生 `--version` 均通过。离线套件包含上轮路径分隔修正的 10 项测试也已通过。

ACP 探针在自持 leader PID 30820 启动后抛出 `PermissionError: [Errno 13] Permission denied`。证据 `cases=[]`、没有 stdio PID 或协议事件；尚未发送 `initialize`，所以不能认为 ACP 响应错误。自持 leader 被终止并回收，记录退出码 1，私有目录移除成功；这是强制收尾，不是自然退出通过。

原 artifact 未保存具体文件操作栈。结合执行位置和固定源码，直接候选原因是探针用普通读读取活跃 `.lock`：Grok 的 `fs2` 全范围排他锁在 Windows 拒绝其他句柄普通读取。依据、窄修及新真实跨进程回归见 [ACP 专报补充](GROK_FIXED_ACP_BOUNDARIES.md#第五轮-windows-排他锁读取失败与窄修)。此修正不属于本轮提交，不能标为 Windows 已通过。产品 `grok.rs` 没有复用这个 PID 文件读取，所以不把探针失败归为同一个产品缺陷。

Grok 后续默认步骤被跳过，包括 Claude 初始化、原生 Codex 生命周期、Rust check/tests/监督验证。`always` Git Bash payload 13 项及上述 Codex Windows hook 探针独立通过，证据上传成功。首因是 Grok 探针，不是上传或后续缺失检查。

本报告仅回收真实结果；未派发新运行、未运行 Cargo、未改变产品、开关、workflow 或用户配置。无需本地化变更。
