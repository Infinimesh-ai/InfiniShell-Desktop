# Codex rev4 Windows 只读原生授权判断

第十轮 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 的 Windows x64 正式原生证据足以支持**读取本地信任配置并显示 Required / Configured / Unknown**。本次适配不把 Configured 当作 Active，不写 `trusted_hash`、不替用户启用 hook，也不开放 Windows 自动安装或完整生命周期门禁。

## 正式证据与边界

原始 artifact：`codex-windows-native-hooks-b3d8b0f11657b0ed1ba34035dda8d363eb52747f/codex-windows-native-hooks.json`，SHA256 `803db5ecf876ea79aae8fb4f1d27ae206fabba746c482ae3d64174a49943bb5e`。固定原生 CLI 为 `codex-cli 0.147.0` / `codex-x86_64-pc-windows-msvc.exe`，SHA256 `935a1911ed2556e4ffcec995f4886ac2ac425863ba26fed264df62e30272ad9d`。

本次只抽取两个路径用例各自的 `formal_registration`：它们均为 formal、passed，恰好五项，无脚本插桩、无清单改写、无测试第六项 hook，不创建 thread/turn；读取未信任列表后，在私有配置中授权并确认 trusted，最后成功回滚。两个不同路径用例的五项 key/currentHash 完全一致。整个报告 `credentials_provided=false`，`model_http_requests=[]`。

后续触发验证使用独立阻断 hook 的记录没有混入契约。ConPTY 是另一个原生证据维度；普通 Codex TUI、真实模型、完整运行/取消/恢复生命周期、Windows ARM64 均不由本契约证明。

随附资源 `SOURCE_METADATA.json` SHA256 为 `dc5754dbf7cc3a69ab5261216073bfb711c923082bf8f2b6a6bd5b391c49fa96`，正式 hooks 文件 SHA256 为 `0dd32ef391296e43284e3a45e8c712271f0ccb1e61e27c7fb38b14f1db451d96`。已逐项核对原生报告的完整十文件树与随附源树一致，并将每项原生 `command` 与正式 `commandWindows` 做完整字节比较；没有从 Unix 哈希推导 Windows 哈希。

| 正式事件 | Windows 原生 currentHash |
| --- | --- |
| permissionRequest | `sha256:44070d73fcbf068004c31131a2627104bf6c327a0a45cb2f3ea7a66c39eb1308` |
| postToolUse | `sha256:1b249d42206a018be2cab3f53d27b739b67c4997c8583c2ad883ff937f46509e` |
| sessionStart | `sha256:84c9084a052e6efc4f8104ff953afdf1f8bde6613e87638ce5cf1f3fc6ab230d` |
| userPromptSubmit | `sha256:c5e2c592551335a5dc0de1aa80273f4d3e6195da3209b3f313c3b5b4425c1c20` |
| stop | `sha256:7aca65d37ccae169a00489c40f6b130d5611c7774ca6fed130b3a5b6d19aa3cd` |

[脱敏原生夹具](fixtures/codex-0.147.0-native-hook-trust-rev4-windows.json)保留两个用例、资源树及授权前后对应哈希；临时机器路径与原生跟踪全文未纳入。较长 PowerShell command 保存 SHA256，正式源码字节由 hooks 资源及测试共同核对。

## 实现

- 新增独立 `NATIVE_HOOK_TRUST_WINDOWS.json`；Windows 使用正式 Windows 五项契约，其余平台保留既有 Unix 契约。
- Windows x64 删除无条件 Unknown 分支，继续复用既有完整插件树核验、只读配置解析与短期缓存。Windows 其他架构仍由产品入口返回 Unknown。
- 任一 hook 未授权、被禁用、哈希过期或用了另一平台哈希，返回 Required；缺文件、自定义资源或无法解析，返回 Unknown。只有当前平台五项全部匹配才返回 Configured。
- 安装/更新策略、PATCH_METADATA、SOURCE_METADATA、脚本、当前会话通知确认、Active 判定均未修改。历史资源 README 的“Windows 原生摘要尚未采集”描述由本记录补充；不得据本记录扩大其余生产门禁。

## 验证与待办

新增回归覆盖正式原生夹具与随附资源对应、平台契约选择、逐项混入另一平台哈希后拒绝，以及 Windows x64 的真实 manager 入口只读显示 Configured 且自动安装仍关闭。既有缺失/禁用/损坏/缓存失效/只读测试改由当前平台契约生成配置，Windows 不再依赖 Unix 值。

本次已完成离线原始 artifact / 两用例 / 全资源树 / 命令字节核对、JSON 验证、rustfmt、diff 检查。按分工未运行 Cargo、CI、真实 CLI 或模型，新增 Rust 回归尚未执行；须在最终包含这些修改的同一提交运行受影响测试、`cargo check -p warp`、`cargo test -p warp --lib i18n::tests` 与相关平台门禁。第十轮证明资源和原生接口，不能替代新 Rust 适配的同提交验收。

本地化审计：复用现有“授权通知 / 核对通知”和英文对应文案；双语提示均已明确本地配置匹配不等于当前会话已生效，无需本地化变更。本次没有新增布局，未把此前 macOS GUI 截图计为 Windows 布局验证。
