# Claude 下一轮队列与 Codex rev4 检查点

日期：2026-09-17。基线为 `a245c639d307ababbc9b26e604e71318b8ee9522`；本地验证使用明确冻结的修改文件，不能计为最终同提交跨平台验收。

## Claude 可用行为

运行中的 Claude 任务现在可以排队一条独立下一轮指令。原生接收确认保留提交时的 generation；仅收到实际开始事件才创建下一轮 generation。前一轮结果先保存，取消当前轮不会把排队指令误记为已取消。队列占用时禁止再次发送；原生未确认下一轮开始时不会伪造成功。应用重启保留消息与确认，不自动重投。

固定 Claude Code 2.1.273 通过用户授权的 API，在私有 HOME、项目和应用配置中完成真实 GUI 验证。沿用原生会话 `4ce7cdad-d9a4-48fa-9e8c-eee8fc7761da`：

| 场景 | 实际结果 |
| --- | --- |
| 第 9 轮运行中排队中英双语输入 | 原生确认仍关联提交代 9；第 9 轮保存完整结果后，第 10 轮独立完成并返回 `GUI_QUEUED_SECOND_4K8M` |
| 队列已占用再输入草稿 | 发送按钮禁用；草稿清空后未提交，消息总数未增加 |
| 第 11 轮运行中排队后取消 | 第 11 轮收到原生 Cancelled，第 12 轮独立完成并返回 `GUI_AFTER_QUEUED_CANCEL_9C2P` |
| 第 13 轮运行中排队后正常退出应用 | 退出前第 14 条消息已原生确认，尚无下一轮开始；重启后第 13 轮为 Disconnected、结果为空、pending ID 保留，仍为 14 条消息，没有自动重投或伪造第 14 轮成功 |
| 双语布局 | 1229×768 下，中文/英文排队按钮、提示换行及取消按钮可见，无截断；历史记录长滚动时发现编辑器占位符越出外层裁剪的问题，单独修复中，不计整个弹窗布局通过 |

证据见 [GUI 目录](validation/gui-claude-queue-1/manifest.json)。应用来自 [11 文件 GUI 构建](validation/macos-queue-grok-gui-build.json)，源码为 [冻结输入](validation/macos-queue-grok-inputs-1.json)。此次重启后没有自动继续历史会话；已有完成任务的原 ID 恢复证据另见 [先前 GUI 验收](CLAUDE_GUI_E473_A245.md)。待执行消息的最终恢复/去重全链仍需单列验收。

## Codex rev4 与 Grok 内部实现

Codex 随附插件 rev4 纳入 Windows 编码 PowerShell 命令和 `CONOUT$` 通知输出，原生 jq 使用二进制输出保留 LF/CRLF 原始字节。保留精确 rev3 来源以执行事务迁移；修改过的旧来源或混合缓存拒绝迁移，失败恢复旧指针、缓存和原配置。Windows 原生探针改为验证正式随附的五个 hook，先验证未信任状态再在私有配置中授权；另外的阻断 hook 只用于零模型运行探针。Windows 自动安装仍关闭，原生授权摘要仍待该平台实际采集。

Grok ACP 内部适配新增精确单次审批、取消、队列及多文本流片段关联。真实 no-leader 夹具来自 Grok Build 1.0.30 使用自定义 Claude 后端；不代表官方 Grok 模型额度或生产监督路径通过。产品托管门禁仍关闭。发现终态 RPC 可能先于未知后续文本流的收束边界；已有正常顺序夹具不能证明任意乱序下结果完整，正在核对原生回放水位接口。

## 本地门禁与证据有效性

[44 文件冻结输入](validation/macos-rev4-queue-inputs.json)通过 [重新编译后的门禁](validation/macos-rev4-queue-gates.json)：`cargo check -p warp`、`cargo test -p warp --lib i18n::tests`（11 项）、相关模块 nextest（1033 项）。新增三项 rev3 迁移测试实际执行；测试二进制 SHA256 为 `5e42354b8c04ae205a0cd51ff5c224e4e95035717fc4dcc80039cd8429a989c8`。

第一次复制冻结文件保留旧 mtime，Cargo 重用了此前测试二进制，故 [第一次记录](validation/macos-rev4-queue-invalid-stale-gates.json)已明确标为无效。重新更新所有输入 mtime 后编译并核对测试新增项与二进制摘要，源码内容摘要保持一致。无效轮次不计通过。

[七组 Python 离线门禁](validation/macos-rev4-python-gates.json)共 111 项，其中 105 项通过、6 项因 Windows 原生条件跳过。对应 Windows 实际行为不能用这些跳过项代替。历史第九轮平台结果见 [e473 报告](NINTH_PLATFORM_RUN_E4738E1EA.md)。最终同提交平台检查、完整工作区、SSH/tmux 与三方完整产品生命周期仍未完成；Goal 保持进行中。
