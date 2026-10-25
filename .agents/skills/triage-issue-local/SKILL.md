---
name: triage-issue-local
description: 对 InfiniShell issue 做证据驱动的分类，核对版本、平台、现有功能和仓库标签，只提供分诊建议，不自动评论或修改 issue。
---

# InfiniShell issue 分诊约定

先确认任务指向的仓库与 issue，区分报告事实、复现结果和推断。输出问题类别、适用标签建议、依据和最小必要的下一步；无须安装额外上游 skill。发布评论、修改标签、关闭 issue 或发送消息须有任务明确授权，本 skill 不额外授权外部写入。

## Heuristics

- 当前仓库是 InfiniShell 社区 fork，需区分其行为与上游 Warp/Zap。公开报告可能不完整，但不得在 issue 中索取密钥、token、私有工作区/仓库名称或账户标识。
- Distinguish the user's observed InfiniShell behavior from their guesses about Rust modules, UI components, server behavior, feature flags, or product intent.
- For issue reports that mention another terminal, editor, shell, or CLI tool, identify whether the problem is InfiniShell-specific or generally reproducible outside InfiniShell before assigning InfiniShell ownership.
- When the issue includes screenshots, videos, logs, stack traces, or command output, use them as primary evidence and ask follow-up questions only for missing details that cannot be inferred from that evidence.
- Before asking any follow-up questions, check the InfiniShell documentation and the repository's existing feature set to determine whether the desired behavior the reporter is describing is already supported. If an existing feature, setting, or workflow satisfies the request, recommend it to the reporter instead of treating the issue as a bug or feature gap.
- 当前 OSS 产品不要求 Warp 账户。涉及第三方模型服务账单、上游 Warp 账户或其他服务申诉时，先识别实际服务商，再提供正确支持入口的建议；不要自动导向 Warp 支持或编造 InfiniShell 账单流程。

## Follow-up question limit

Ask **at most 2 follow-up questions** per triage response. Each question must be high-value: it should meaningfully change the label assignment, owner routing, or reproduction confidence if answered. Do not ask questions whose answers can be inferred from existing evidence, and do not bundle multiple sub-questions into a single bullet. If more than 2 unknowns exist, prioritize the two that are most likely to unblock triage.

## Label taxonomy

本仓库没有 `.github/issue-triage/config.json`。通过当前仓库的只读标签列表核对实际可用标签，例如 `gh label list --repo "$issue_repo" --limit 200`；`issue_repo` 必须来自已确认的任务仓库。以下 `area:*` 仅为分类参考，不表示标签存在，不自动创建标签。

Use area labels based on the user's reported surface:

- `area:shell-terminal` for terminal output, block rendering, shell integration, prompt rendering, command execution display, and terminal-emulation behavior.
- `area:terminal-input` for command-line input editing, cursor movement, key handling, and typed text behavior.
- `area:window-tabs-panes` for window, tab, pane, split, layout, and focus behavior.
- `area:editor-notebooks` for editors, notebooks, markdown rendering, LSP, and code display.
- `area:agent` for agent conversations, agent mode, cloud/local agent execution, prompts, and AI-specific UI.
- `area:code-review` for git diff views, review UI, review comments, and PR-focused agent flows.
- `area:mcp` for MCP server connection, tool/resource discovery, OAuth, and integration issues.
- `area:settings-keybindings` for settings UI, preferences, keyboard shortcuts, and keybinding configuration.
- 对 SSH/SFTP、项目管理、同步、Workflow、Notebook 和持久化问题，按实际受影响模块与现有标签分类；不假设本项目启用了上游 Drive。
- `area:performance:*` when the report includes CPU, memory, GPU, startup, rendering, latency, or responsiveness symptoms. Add the more specific CPU, memory, or GPU label when the evidence points to that resource.

## Information to check for before asking follow-up questions

Before asking the reporter for more information, check the issue body, comments, attachments, logs, labels, and repository context for:

- InfiniShell 版本/build、来源和通道，尤其核对是否为当前 OSS 构建；上游 Warp 的通道名不能代替本项目版本。
- OS and version, architecture, display setup, window manager or desktop environment on Linux, and whether the issue is platform-specific.
- Shell and terminal context: shell name/version, prompt framework, shell integration status, command being run, terminal mode, local vs SSH/remote/tmux, and whether the behavior reproduces in a fresh session.
- Clear reproduction steps, expected behavior, actual behavior, frequency, regression timing, and whether the user can reproduce outside InfiniShell.
- Visual evidence for UI, rendering, layout, font, cursor, focus, window, pane, tab, and accessibility issues. Prefer a screenshot or short recording when the symptom is visual.
- Logs and diagnostics for crashes, hangs, startup failures, update failures, authentication failures, MCP failures, and agent execution failures. Ask for redacted logs only when the report lacks actionable evidence.
- For AI/agent reports: whether the agent is local or cloud, the model if known, relevant conversation/session link, repository context, tool or MCP server involved, and the exact user action that triggered the failure.
- For performance reports: approximate project/session size, command output size, CPU/memory/GPU observations, profile or diagnostics if provided, and whether the issue appears after long-running sessions.
- For keyboard or input reports: keyboard layout, custom keybindings, IME usage, conflicting OS shortcuts, focused surface, and whether the same keys work in other apps.
- 对第三方认证或账单问题，只使用报告者已提供且可公开的信息；需要私密账户详情时建议联系对应服务商，不在公开 issue 索取凭据或个人标识。

## Recurring follow-up patterns

- Visual UI/rendering issue with no media: ask for a screenshot or short screen recording first.
- Environment-sensitive terminal issue: ask for InfiniShell version/channel, OS/version, shell, and whether it reproduces in a fresh local session.
- SSH/tmux/remote issue: ask for local OS, remote OS, shell, whether tmux is involved, and the minimal command or workflow that reproduces it.
- Agent/MCP issue: ask for the failing workflow, local vs cloud execution, relevant session link, MCP server/tool name, and any redacted error text.
- Performance issue: ask for approximate scale, how long InfiniShell has been running, what action triggers the spike or hang, and whether logs or a profile are available.

## Owner-inference hints

`.github/STAKEHOLDERS` 保留了上游团队映射，不能据此给本项目问题指定负责人或提及上游人员。先核对当前仓库维护者和用户明确提供的分工；无法确认时只报告受影响模块，不猜测负责人。
