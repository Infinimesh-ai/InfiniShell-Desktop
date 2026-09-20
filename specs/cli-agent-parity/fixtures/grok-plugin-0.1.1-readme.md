# InfiniShell Grok 状态插件

原生 Grok 插件，版本 `0.1.1`，受测 Grok `1.0.30`。无需远端 marketplace。插件仅把已知生命周期事件转换为 InfiniShell OSC 777 v1 通知，不改变权限策略。

## 安装、更新与禁用

需要 Node.js 18 或更新版本，并且 Grok hook 子进程能从 `PATH` 找到 `node`。InfiniShell 安装器应先验证运行时可用；缺失时保留普通终端体验。Windows 不要求 Python。插件文件由桌面安装器保存在持久目录，不能把临时解压目录当更新源。

```text
grok plugin validate <插件绝对目录>
grok plugin install --trust <插件绝对目录>
grok plugin disable infinishell-grok
grok plugin enable infinishell-grok
grok plugin uninstall infinishell-grok
```

本地来源由 Grok 复制到 `GROK_HOME/installed-plugins/`。受测版本的 `plugin update` 对本地复制来源返回成功，但不会更新实际文件，重复安装同一来源也会失败。桌面升级先备份本插件的四个文件，再卸载单个已识别插件并安装明确的新版本；若安装失败，使用备份重新安装旧版本并验证。存在同名重复注册或插件已被禁用时不自动执行。没有创建不存在的远端发布地址。

`grok plugin list --json` 的 `status=installed` 不代表启用。受测版本将原生启用/禁用列表写入 `GROK_HOME/config.toml` 的 `[plugins].enabled` / `disabled`。版本与安装位置保存在 `installed-plugins/registry.json`。管理器应区分未安装、禁用、不兼容和运行时缺失。

安装、更新或启用后，在运行中的 Grok 输入 `/hooks`，按 `r` 重新加载。真实 GUI 验证发现，仅退出并重新打开前台 Grok 会话可能仍保留旧 hooks 缓存；不能把注册成功当作已加载。重新加载后应能看到本插件的九项 hooks，应用收到当前通知后才能确认通知链路可用。

## 状态与兼容边界

- 必须同时存在 `WARP_CLI_AGENT_PROTOCOL_VERSION`、`GROK_HOOK_EVENT`、`GROK_SESSION_ID`，且事件类型/会话与载荷一致。普通 Claude 不会通过 Grok 身份检查。
- 兼容 Grok camelCase 和 Claude snake_case 字段；冲突载荷丢弃。事件 ID 基于规范化载荷计算，不伪造上游单调序号。
- 同一会话的少量状态放在 Grok 提供的 `GROK_PLUGIN_DATA`，只保存去重标识、时间和状态，不保存提示词或工具输入。独立 hook 进程通过锁和原子替换防止状态竞争。
- `PermissionDenied` 只是通知，不进入等待审批。单个工具失败不等于任务失败。
- `StopFailure` 保持失败状态；`SessionEnd` 和 `Stop(reason=shutdown)` 不能变成成功。只有显式 `end_turn`、有最终文本、没有继续 hook 标志且当前回合未失败时才发送当前回合的 Stop 候选响应；应用仍不能仅凭该 hook 判断任务完成。其余普通 Stop 降级为通知。
- 取消、活跃任务重连和审批回传由托管协议承担，本插件不会依据退出码或文本猜测这些状态。
- JSON 文本转义终端控制字符；tmux 使用 DCS passthrough。Unix 写 `/dev/tty`，Windows 写 `CONOUT$`；无控制终端时安静降级，不污染 ACP/hook stdout。
- Windows/真实 tmux/SSH 尚需在同一产品修改提交上实际验收。纯字节测试只验证转义逻辑，不代替真实平台交互。

## 验证

在仓库根运行 `node --test script/cli-agent-parity/grok_plugin_tests.cjs`。真实 Grok 协议与 hook 来源见 `specs/cli-agent-parity/PROTOCOL_EVIDENCE.md` 和 `fixtures/`。
