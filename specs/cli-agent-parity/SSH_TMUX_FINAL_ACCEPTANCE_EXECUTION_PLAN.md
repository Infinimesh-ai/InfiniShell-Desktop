# SSH/tmux 最终验收可执行入口盘点

本次仅静态盘点与 PATH 文件存在性检查，没有网络连接、CLI 执行、认证/密钥读取、GUI 操作、Cargo、target 或 refs/gates 查询。最终验收尚未执行。后续应在包含所有实际修改的最终提交上，先完成本地门禁，再使用 cross-platform-preflight.yml 的同 ref Linux/Windows 验证；旧 dirty 快照、旧原生 byte 结果均不回填为当前提交通过。

当前 PATH 中 Python、ssh、sshd、ssh-keygen、Codex、Claude、Grok、Node、jq、Bash 和 rsync 均有文件入口；tmux 未在 PATH，系统 /usr/sbin/sshd 存在。历史工具记录已给出真实独立构建的 tmux 3.7c SHA256 `a47f7c82de1e2779eadbd089ab4daccf60cd136a2b8dd976bec3399b9755a9f9`、官方 tarball 摘要与编译选项。当前该临时产物路径未定位，不能宣称已丢失或无法验收：根任务可从既有受控产物清单定位并重核摘要，或按工具记录重新构建到独占目录。原构建关闭 utf8proc，只证明通知传输；中文布局另需支持 Unicode 的真实版本。没有运行三款 --version，PATH 存在不代表版本或授权已经满足。

~/.ssh/config 的受控解析确认 `ubuntu-infinishell-build` 与 `win-infinishell-build` 两个别名存在，显式 stanza 有 IdentityFile 指令；只输出别名及布尔，没有输出地址、私钥路径或配置全文，也未读取或 stat 密钥文件。未展开有效 SSH 配置、未连接，远端 OS、CLI 安装、插件及模型授权仍未知。优先使用这两个专用构建机候选，先取得实际平台与可用身份的安全前置证据，不能据名字或已有别名判 remote ready。

现有入口及其真实边界：

| 入口 | 可复用行为 | 不能据此证明 |
| --- | --- | --- |
| probe_ssh_tmux.py:27–255 | 一次性私有 keys/known_hosts/loopback sshd、中文空格 HOME/cwd；三 CLI --version；direct、tmux on/off 的随附 SessionStart hook 回放 | 原生 CLI 触发、现代 Claude 前台 JSON emitter、在线两轮交互、GUI、异机 |
| probe_codex_ssh_tmux.py:282–435 | 固定 Codex 0.147.0 原生 TUI，实际安装隔离插件、信任精确 hooks；每模式独立 session/turn，唯一 argv prompt 被 UserPromptSubmit 原生阻断；SSH stdout 收到通知 | 在线成功模型回合、逐键/富输入、审批和应用托管任务；这是无模型阻断探针 |
| tmux_notification_tests.py:17–123 | 真实 Unix 控制终端，TMUX 为构造值；检查 OSC/DCS/ESC、现代 Claude raw terminalSequence JSON | 真正 tmux server、SSH、原生 CLI 消费 terminalSequence；新版测试恰是 JSON 输出，未接收到终端字节 |
| cross-platform-preflight.yml:608–619、test_pwsh_ssh_worker.ps1 | 同提交 Windows 控制台 infinishell-ssh worker 构建、Base64/bootstrap/PowerShell 参数透传；原生 ssh -V fallback | Windows SSH 实际连接、远端 CLI 或 tmux 完整链 |

旧 after-fix fixture 是提交 6921a992 上 dirty 的三方 hook 回放，native_cli_hook_triggered=false。Codex final-commit fixture 属干净 dbee1ec，direct/on 收 2 OSC、off 收 0，模型 HTTP 0，真实原生阻断；完整 HookRunSummary、GUI 和成功模型生命周期仍 false。source22 后的审批与全局配置修复不能用这两份旧 byte 记录验收。PLAN 中 SSH/tmux 仍是独立未完成项目。

普通终端应复用既有远端 TerminalView/SSH wrapper，直接在已确认的远端 shell 输入 `codex`、`claude`、`grok`，保留用户 alias/function；不要将本地 executable 安装扫描路径传给远端。标题栏 add_tab_with_specific_agent（workspace/view.rs:4330）创建普通新 tab 并 Ignore 默认 Agent 模式，不是重用当前 SSH pane 的远端派发 API。旧 standalone AgentDriver 的三个 Harness 均 Unsupported（harness/mod.rs:132–142），不能用它冒充无人审批的终端验收；TerminalDriver 的新 workspace 和 current_directory 也基于本地路径（terminal.rs:81、391）。已在 SSH pane 运行的 CLI 仍通过 CLIAgentSessionsModel 与 footer/rich input 交互；需要实际检查远端通知是否让该 view 建立可信会话。

本地应用托管 Coordinator 使用本地 executable/cwd/state 与进程监督，不是 SSH transport。SSH 普通终端全链、本地托管父子任务全链分别验证；不能把远端前台 CLI 映射成一个尚未声明的 App 托管任务。若需要远端托管同等消息/权限/结果接口，须另补真实远端 adapter，本轮盘点不声称已有。Grok SDK/父权限 ceiling 未通过时保持功能差异与关闭门禁。

后续命令模板只使用实际存在的参数，所有路径/摘要在同提交前置验证后填写；运行器输出须选新名字，外层使用 O_EXCL 的 once 标记，保持旧失败，禁止盲目重投。现有两个探针没有通用三方在线 GUI 自动化，也没有 output 的 xb 保证，不能直接当最终总验收器。

```sh
python3 -B script/cli-agent-parity/tmux_notification_tests.py
python3 -B script/cli-agent-parity/probe_ssh_tmux.py \
  --tmux "$parity_tmux" --sshd /usr/sbin/sshd \
  --codex "$parity_codex" --claude "$parity_claude" --grok "$parity_grok" \
  --codex-plugin "$parity_codex_plugin" --claude-plugin "$parity_claude_plugin" \
  --output "$parity_evidence_dir/ssh-tmux-replay-new.json"
python3 -B script/cli-agent-parity/probe_codex_ssh_tmux.py \
  --codex "$parity_codex" --expected-codex-sha256 "$parity_codex_sha" \
  --tmux "$parity_tmux" --expected-tmux-sha256 "$parity_tmux_sha" \
  --sshd /usr/sbin/sshd --output "$parity_evidence_dir/codex-native-ssh-new.json"
```

Windows 在同提交 workflow 使用现有命令 `cargo build -p warp --bin infinishell-ssh --features "release_bundle,gui,autoupdate,nld_classifier_v3,nld_heuristic_v2,rust_ssh_worker"`，再 `.\script\windows\test_pwsh_ssh_worker.ps1 -WorkerPath "$env:CARGO_TARGET_DIR\debug\infinishell-ssh.exe"`；它们仍是前置层。Linux 开发远端已有 `script/deploy_remote_server --host ubuntu-infinishell-build --profile dev-remote` 的 musl 编译/rsync入口，需要先核实际平台、上传路径和最终提交产物。ssh_transport.rs:433–447 的开发交叉编译前置不足会回退 release 下载；最终验收必须拒绝把旧 release daemon 当当前 worker，绑定真实 remote 产物 SHA/构建来源和 resources。

最小实际在线验收：在同提交 GUI 开启独立 SSH 会话，受控远端 HOME/cwd 与新 tmux socket/session，逐款分别运行 direct、可见 pane 的 allow-passthrough on，并单测 off 的通知降级。设置只作用本次 socket/window，不改用户全局 tmux；现代 Claude 必须是原生交互前台，Grok 必须触发自己的原生 hooks，不能从 SessionStart 回放合成成功。每款用唯一合成验收标记完成新建、两轮中英文/多行与附件/文件/技能/评审输入、允许/拒绝审批、追加、取消、精确 nativeS 继续、SSH 断连/重连、应用重启与结果回收。逐节点保存真实输入数量、view/session/task/nativeS、消息 ID/回执、进程/终态证据和安全截图；stdout 原字节只留受控私有域，公开 type/bytes/SHA，凭据值不投影。

断连时先判断旧 remote CLI/tmux pane 是否仍活跃：活跃只 attach 旧 socket/session 并重关联，不重复启动、不重发上轮 prompt；确认旧进程结束后才使用经本版本验证的精确 nativeS resume 命令，不用 --last 猜会话。新进程/任务 generation 必须变更，旧 hook/approval/message callback 不得绑定新回合；同 ID 重投只校验回执/缓存，不重复副作用。取消只清理本次 lease/PID/PGID/tmux socket，记录真实退出/EOF/receipt；单纯 ssh 退出不能算 CLI 已取消，tmux 仍活跃不能显示成功。多层 tmux、不可见 pane、Windows 默认 shell/ConPTY、同会话原生取消和恢复失败保留独立结果，不用 on 字节测试代替。

本轮只新增文档，未改变用户文案，无需本地化变更；没有计任何新 SSH/tmux、远端授权或 Goal 验收 PASS。
