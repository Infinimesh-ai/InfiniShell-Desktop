# Codex 原生 TUI 经 SSH/tmux 的无模型通知验证

2026-09-16，macOS arm64，Codex 0.147.0、tmux 3.7c。此次由真实原生 TUI 触发 SessionStart/UserPromptSubmit，未修改的受控通知脚本通过远端 PTY、tmux 和 SSH 客户端传回 OSC。三种模式均已得到对应的正向或负向结果；这比旧 [SSH/tmux 脚本回放](SSH_TMUX_VERIFICATION.md)多验证了原生触发，但仍不是 InfiniShell GUI SSH 完整验收。

## 实际结果

| 模式 | SSH 客户端收到完整产品 OSC | 原生 TUI 阻断显示 | SSH / Codex 退出码 | 模型 HTTP 请求 |
|---|---|---|---|---|
| SSH 直接 PTY | 2：SessionStart、PromptSubmit | 本轮唯一 stopReason + `UserPromptSubmit hook (stopped)` | 0 / 0 | 0 |
| SSH + tmux，透传 on | 2：SessionStart、PromptSubmit | 同上 | 0 / 0 | 0 |
| SSH + tmux，透传 off | 0；两项原生 hook 仍实际执行 | 同上 | 0 / 0 | 0 |

透传 off 的通过含义是确认阻断，不能描述为通知到达。三种模式的参考脚本都返回 0，stdout/stderr 为空；实际通知只从控制终端发送。原生 TUI 都正常关闭，没有因超时强杀；已记录的测试启动器、Codex 和 hook PID 在结束后均不存在，独立 sshd 已退出，独立 tmux 服务已关闭。

完整脱敏证据见 [原生 SSH/tmux 记录](fixtures/codex-0.147-native-ssh-tmux-macos.json)。每个模式保存不同的原生 session/turn、hook 原始字段、插件根目录、SID/PGID、控制终端及 SSH stdout 摘录。SSH 原始字节长度依次为 13818、37071、38276；fixture 保留原始 SHA256/长度，但文字明确是脱敏版本，未保留含私有路径的 Base64。原始报告留在源树外 `/tmp/infinishell-codex-native-ssh-fourth.json`。

## 证据链与边界

1. 每种模式有独立 HOME、CODEX_HOME、空工作目录、原生插件注册表和本地 marketplace。只使用固定摘要的参考插件完整树，实际执行 `plugin marketplace add` 与 `plugin add`；宿主没有设置 PLUGIN_ROOT，记录确认它来自原生安装缓存。
2. 准备阶段仅用独立 stdio app-server 执行 initialize、hooks/list 和 config/value/write，审核并信任本次两个精确定义，不创建线程或提交模型输入。它不属于通知测量链，也不创建控制 PTY。原生目录信任由 SSH 中的 TUI 显示精确空目录后选择；没有修改普通用户的配置或信任。
3. 测量阶段是真正的 `codex --no-alt-screen <固定提示>`。每种模式只有一个固定 argv 初始提示。远端启动器直接继承 sshd 的 PTY；tmux 模式则继承实际 pane 的 PTY，保留 tmux 注入的 TMUX/TMUX_PANE。脚本没有调用 openpty、setsid 或另建终端作为通知接收端。
4. 原生 hook 的诊断包装器调用未修改的参考脚本，另把原始输入和退出信息写入私有报告。UserPromptSubmit 只接受固定提示，再输出 `continue: false` 与该模式唯一 stopReason。终端真正在 SSH stdout 显示该 stopReason 和 stopped；因此没有把包装器生成 JSON 当作原生已接收证明。
5. 通知验收只读取 `ssh -tt` 的 stdout 原始字节。直连与 on 的 PromptSubmit 同时匹配原生 session、turn 和固定提示；off 虽没有 OSC，仍通过唯一 stopReason、原生输入中的 turn 与 SSH 上的同 session 继续提示关联此次阻断。没有用旁路报告、tmux capture-pane 或本地另开的 PTY 冒充链路字节。
6. 所有原生进程使用无凭据的本机拒绝 provider；HTTP 服务对任何请求返回错误并计数，出现任何请求即失败。实际计数为零。标准终端查询应答、私有目录信任、保持已配置模型和退出按键均经 SSH 输入，只是控制操作，不是额外模型提示。

本次没有取得完整 HookRunSummary；`native_hook_summary_obtained=false`。原生 TUI 的 stopped 是本轮阻断的直接可见证据，不能延伸成成功模型回合或 Stop hook 终态证明。普通 PTY Stop 的可信降级仍见 [Stop 审计](STOP_HOOK_COMPLETION_AUDIT.md)。

## 探针失败与修正记录

前三次没有通过，证据保存在 [初始化失败记录](fixtures/codex-0.147-native-ssh-tmux-macos-preflight-failures.json)，没有删除或合并成成功记录：

- 第一轮停在原生目录信任页。屏幕用光标位移代替部分空格，最初的纯文本匹配没有识别，35 秒超时；未取得 hook 传输证据。
- 第二轮尝试将完整目录移至系统 `/tmp`，原生 SSH 公钥认证拒绝。恢复既有私有密钥目录，只有 tmux socket 使用短目录；未到达原生 TUI。
- 第三轮已明确选择信任空目录，随后停在原生 GPT-5.4 模型迁移提示。探针现只对该已观察页面选择“Use existing model”，保留绑定本机拒绝 provider 的原模型；没有接受替换或改变审批策略。

三轮的模型 HTTP 计数均为零，已记录的测试 PID 均不再存活。第四轮才取得表中的原生传输结果。之后新增的阻断状态、唯一 stopReason 和会话断言已对第四轮原始字节离线复验，没有再次提交提示。收尾还把既有 SSH 探针的私有 HOME `SetEnv` 防护补入新配置生成器，使 sshd 调用 ForceCommand 前也使用私有 HOME；该机械补充通过独立 `sshd -t` 配置解析，未重跑第四轮原生输入。fixture 的脚本摘要记录导出时版本，不冒称每项后续断言都在第四轮启动前存在。

## 复跑与范围

入口：[probe_codex_ssh_tmux.py](../../script/cli-agent-parity/probe_codex_ssh_tmux.py)。依赖 Python 3.11+、普通用户可启动的 OpenSSH sshd、Bash、jq、固定 Codex 和 tmux 文件。tmux 的固定获取、构建及摘要依据沿用 [工具记录](fixtures/ssh-tmux-tooling.json)，没有全局安装。

```sh
python3 -B script/cli-agent-parity/probe_codex_ssh_tmux.py \
  --codex /opt/homebrew/bin/codex \
  --expected-codex-sha256 19c4f144c5226a9f17c58e6f0fa854843b0f77a6eb420f40e2745a12f10f5d37 \
  --tmux /absolute/path/to/verified/tmux \
  --expected-tmux-sha256 a47f7c82de1e2779eadbd089ab4daccf60cd136a2b8dd976bec3399b9755a9f9 \
  --output /tmp/codex-native-ssh-tmux.json
```

缺少工具、未进入原生 hook、信任或模型选择出现未知页面、缺少原生阻断显示、字节不符、HTTP 请求非零、退出失败均不计通过。本次新探针和两个生成脚本的语法检查、三份真实字节回放断言通过；另对既有记录定向构造旧 turn、其他 session、参考脚本失败、强制退出、伪插件目录、缺少原生阻断确认、SSH 字节缺失和 off 意外透传八项反例，全部被拒绝。这些是记录校验器测试，不是再次原生执行。未运行 Cargo 或远端 workflow。

固定官方提交 `be6e8eac029b183056b7e4402879f15d2c85f61b` 的 [TUI 初始提示参数](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/tui/src/cli.rs#L13)、[无认证 provider 登录分支](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/tui/src/lib.rs#L1872)及 [模型循环前的 hook 阻断](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/core/src/session/turn.rs#L233)与此次真实行为一致。

仍未验证：不同物理主机/操作系统、Windows/ConPTY、现代 Claude 原生 JSON 传输、Grok 原生触发、InfiniShell GUI SSH 的工具栏/富输入/审批/取消/恢复/结果回收、多层 tmux、不可见 pane、断线重连，以及成功模型生命周期。本次仍是 dirty 工作树中间证据，不替代最终同提交跨平台验收。无产品文案变化，无需本地化变更。
