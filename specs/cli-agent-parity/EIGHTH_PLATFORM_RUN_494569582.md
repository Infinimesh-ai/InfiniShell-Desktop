# 第八轮跨平台验证：494569582

[Actions 35182914808](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35182914808)已结束，实际提交为 `49456958279cc193392b2d09118e680fc0b14ea4`。Linux 所选检查通过，Windows 未通过。完整工作区测试和 GUI 集成编译本轮未开启，不能计为发布验收完成。

| 平台 | 实际结果 |
| --- | --- |
| macOS | 干净同提交 check、i18n 11 项、相关模块 987 项通过；见[本地记录](validation/macos-local-gates-494569582.json) |
| Linux | 所选门禁全部通过：warp 1706、ai 71、CLI 128、TUI 9、command 1、监督进程真实清理 4、rust-genai 81；见[完整步骤与日志摘要](validation/linux-eighth-494569582.json) |
| Windows | check、warp 1678、ai 71、CLI 128、TUI 9、command 2、监督进程真实清理 4、rust-genai 81 通过；三项原生验收失败，见[完整记录](validation/windows-eighth-494569582.json) |

上一轮的 Windows Claude 文件事务夹具、权限探针路径和嵌套 Job 派生回归已通过本轮真实门禁，不能因此将以下失败忽略。

## 仍失败的 Windows 原生检查

1. **Codex 持久插件来源清理。** 所有 app-server Job 活跃进程归零及输出 EOF 证明成立；删除私有 Git `tmp_pack` 时出现 WinError 5。本轮未记录文件属性，不能直接断定为只读。后续修复增加精确属性/身份诊断；仅在已有完整进程收尾证明、私有目录身份不变且目标确为单链接只读普通文件时，允许清除只读位并重试一次，其他错误仍失败。
2. **ConPTY 通知。** 真正原生 SessionStart 和 UserPromptSubmit 的 OSC 已到达 PTY，但原 10 秒窗口内未收到全部 hook 与回合的完成事件，因此整轮未通过。还发现原生输入 LF 在通知 query 中变成 CRLF；原判定没有比较 query。后续临时候选使用 `jq --binary` 保留 LF/CRLF，增加严格逐字比对和完整关联终态等待，仍需 Windows 原生复验。生产通知配方未开放。
3. **缺失会话恢复。** 原生精确返回 `no rollout found for thread id ...`，应用未新建替代会话，并已确认监督清理。测试随后要求退出码为 0，实际为 1，因此在写入证据前失败。原报告没有保留完整回执，不能据此区分该次 1 是原生自然退出还是 Windows Job 停止收尾。后续须先保存观测，再按具体停止原因核对；本轮的 idle-crash 和 cmd 包装恢复未继续执行，均不计通过。

## 后续冻结输入

[19 文件冻结输入](validation/macos-api-cancel-1-inputs.json)在本提交基础上增加 Claude 真实取消聚合、Grok 原生公布的 API 认证及 Windows 探针修复，已通过 [check、11 项 i18n 和 1001 项相关测试](validation/macos-api-cancel-1-gates.json)。这是 dirty 中间快照，不覆盖稍后新增的退出码夹具修复，也不能代替下一提交的平台验收。

所有原始失败均保留。三款 CLI 的完整模型、产品 GUI、最终双语布局及 SSH/tmux 验收继续推进，Goal 保持 active。
