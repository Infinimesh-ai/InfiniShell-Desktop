# SSH 与 tmux 独立验证

2026-09-16，macOS arm64。本次运行真实 OpenSSH 回环连接和独立 tmux 服务，验证远端 CLI 上下文与随附 hook 的通知传输。没有提供模型凭据、发送模型请求、改用户 SSH/CLI 配置、修改系统服务或全局 tmux 设置。源码仍包含未提交修改，fixture 记录运行时 HEAD 和 dirty 状态；这不是最终同提交跨平台门禁。

## 实际环境与隔离

系统 OpenSSH 10.3p1 / LibreSSL 3.3.6 支持普通用户启动高端口 loopback sshd。脚本创建一次性 host/client key、严格 known_hosts、独立 authorized_keys、ForceCommand 与 HOME，结束后清理。只允许固定探测名称，不将 SSH 原始命令作为 shell 源码执行；临时配置关闭密码/PAM/转发。它从未修改 `~/.ssh/authorized_keys` 或系统 sshd。

本机没有 tmux。使用官方发布源码在临时目录编译 tmux 3.7c 与静态 libevent 2.1.13，下载包 SHA-256 与官方 release asset digest 一致，见 `fixtures/ssh-tmux-tooling.json`。[tmux 发布](https://github.com/tmux/tmux/releases/tag/3.7c)、[libevent 发布](https://github.com/libevent/libevent/releases/tag/release-2.1.13-stable)。构建明确关闭 utf8proc 与 jemalloc，仅用来验证通知字节传输；它不提供 Unicode/emoji 布局验收证据。

真正经 SSH 执行的 `--version` 返回 Codex 0.147.0、Claude 2.1.273、Grok 1.0.30。三者路径均解析到隔离远端 HOME 的 bin，工作目录为另一个含空格和中文的目录。远端 Codex/Claude/Grok 插件注册表均不存在，而独立的本地控制标记存在；探测明确确认本地“已安装”不能证明远端已安装。CLI 文件来自本机真实二进制的链接，因此这是同一 macOS 上的回环 SSH，不是其他操作系统或物理主机验证。

上游插件完整树先在隔离目录经固定 SHA-256 验证并应用随附替换件，再由 SSH ForceCommand 根据真实 `hooks.json` 执行 SessionStart 脚本。输入是明确标注的构造载荷；没有由原生 CLI 触发 hook，也没有启动 InfiniShell GUI。`native_cli_hook_triggered=false`、`product_ssh_ui_verified=false` 等字段防止将脚本回放算成产品验收。

## 修复前后结果

每格为实际到达 SSH 客户端的 OSC 777 数量；全部九次运行均 SSH exit 0，且独立 hook runner 完成标记存在。

| 路径 | Codex 修复前 → 后 | Claude 兼容 TTY 修复前 → 后 | Grok 前 → 后 |
| --- | --- | --- | --- |
| SSH 直接 PTY | 1 → 1 | 1 → 1 | 1 → 1 |
| SSH + tmux，透传关闭 | 0 → 0 | 0 → 0 | 0 → 0 |
| SSH + tmux，透传开启 | 0 → 1 | 0 → 1 | 1 → 1 |

证据为 `fixtures/ssh-tmux-macos-before-fix.json` 与 `fixtures/ssh-tmux-macos-after-fix.json`。保留脱敏 JSON、原始字节流的 SHA-256/长度、CLI 版本与每次完成标记；没有保存凭据。after 文件还固定两个 `warp-notify.sh` 的摘要与 patch revision 3。关闭透传的结果是确认阻断，不能算通知成功。

tmux 需要显式允许透传，默认值是 off；on 只允许可见 pane，all 另含不可见 pane。探测只在自己新建的独立服务器配置 on/off，未修改用户全局设置。用户要在目标 tmux 窗口选择允许通知，可执行 `tmux set-option -w allow-passthrough on`；此操作不会由插件安装器自动执行。配置语义来自 [tmux 3.7c 固定选项实现](https://github.com/tmux/tmux/blob/3.7c/options-table.c)。

Claude 有两个真实不同的传输契约。旧版或版本未知的兼容分支向 `/dev/tty` 写字节，本次 SSH 回放验证的是这个分支。Claude ≥ 2.1.141 的上游 emitter 使用 `terminalSequence` JSON，由 CLI 自己输出；该字段仅接受限定 OSC/BEL，DCS 不在允许范围。新版 JSON 必须保持原始 OSC；它只在交互界面可见时输出，`-p` 与 Agent SDK 忽略该字段。[Claude 原生通知文档](https://code.claude.com/docs/en/hooks#emit-terminal-notifications)。因此本次补丁只给兼容 TTY 加封装；现代原生 Claude 的 SSH/tmux 输出仍未通过实测。

## 回归与复跑

以下三组本机实际通过，均使用真实 Bash/jq；前两组属于离线载荷或文件事务，最后一组使用真实 Unix 控制终端，但不启动模型。

```sh
python3 script/cli-agent-parity/plugin_compatibility_tests.py   # 13 项
python3 script/cli-agent-parity/notification_patch_tests.py     # 13 项
python3 script/cli-agent-parity/tmux_notification_tests.py      # 4 项，Unix PTY
```

PTY 回归覆盖 direct OSC、DCS 封套与所有 ESC 双写、含空格/中文/单引号/命令替换字符路径、现代 JSON 保留 raw OSC、未知版本无 TTY 的 JSON 回退。`claude-2.2.0-emit-terminal-sequence.sh` fixture 是固定上游原文，其 SHA-256 与 `PATCH_METADATA.json` 的完整树一致，MIT 许可沿用随附 Claude 目录。

完整 SSH 回放使用以下命令，CLI 与插件路径必须是受测版本的实际文件；临时 sshd 不可用或验证失败时命令返回非零，不能跳过后声称通过。

```sh
python3 script/cli-agent-parity/probe_ssh_tmux.py \
  --tmux /absolute/path/to/tmux \
  --codex /absolute/path/to/codex \
  --claude /absolute/path/to/claude \
  --grok /absolute/path/to/grok \
  --codex-plugin /absolute/path/to/codex-warp/plugins/warp \
  --claude-plugin /absolute/path/to/claude-code-warp/plugins/warp \
  --output /absolute/path/to/ssh-tmux-report.json
```

两个 `upstream.patch` 在固定原始上游树分别通过 `git apply --check`。notify 纳入 Rust/Python 同一文件事务，新增失败恢复与自定义文件拒绝回归；Rust 定向测试等待主代理统一回收。补丁第 3 版完整 Codex 插件再次原生 `hooks/list`，五项仍 untrusted，用户信任配置没有任何写入。

## 未通过的验收

- 完整 InfiniShell SSH 会话中的插件安装、工具栏状态、富输入、审批、取消、恢复及结果回收。
- 现代 Claude 原生 `terminalSequence` 在交互 UI 前台的 SSH/tmux 输出。
- Linux/Windows、ConPTY、远端 Windows 默认 shell 与同提交平台门禁。
- 经过 SSH 执行导出安装脚本的原生安装链；本次只是目标环境的版本探测与通知传输。
- 不可见 tmux pane、多层嵌套 tmux、screen、断线重连的产品级通知接收。

这些缺口不能由 raw byte fixture、CLI `--version` 或本地插件状态代替。此次修改没有新增 GUI/TUI 文案，无需本地化变更。
