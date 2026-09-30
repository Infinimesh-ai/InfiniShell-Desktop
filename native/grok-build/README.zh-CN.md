# Grok 普通 TUI 原生输入桥

这是 G01 的原生能力补丁，尚未完成 InfiniShell 普通终端接入和真实功能验收。G01 保持开放，PR #22 保持草稿。已有官方发行版的验收不能用于这个独立构建。

## 固定来源与构建

- 官方源码：<https://github.com/xai-org/grok-build>
- 基线：`07e35a3dfeed2f200d319ef6c893b5ea286d9a51`，版本字段 `1.0.41`。
- `SOURCE_REV`：`84745de98b3d3996729aefcefd518890ffb73930`，不同于已验官方发行版 `4220f3b224a6`。
- 补丁身份及门禁范围见 `source.json`；许可证和修改声明见 `LICENSE`、`NOTICE`。

在该精确基线的独立干净工作树内，核验补丁 SHA-256 后运行以下命令。不要覆盖系统安装或默认 Grok 配置；当前二进制仅用于后续独立验收。

```sh
git apply --check /absolute/path/to/InfiniShell-Desktop/native/grok-build/terminal-bridge.patch
git apply /absolute/path/to/InfiniShell-Desktop/native/grok-build/terminal-bridge.patch
GROK_VERSION=1.0.41+infinishell.terminal-bridge.1 cargo check --locked -p xai-grok-pager-bin
GROK_VERSION=1.0.41+infinishell.terminal-bridge.1 cargo test --locked -p xai-grok-pager -p xai-grok-shell -p xai-grok-tools -p xai-grok-shell-terminal --lib terminal_bridge -- --test-threads=1
GROK_VERSION=1.0.41+infinishell.terminal-bridge.1 cargo build --locked -p xai-grok-pager-bin
```

Rust 工具链由上游 `rust-toolchain.toml` 固定为 `1.94.0`；`protoc` 沿用上游查找方式。用户本机执行时必须遵守仓库 `docs/local-test-storage.zh-CN.md`：每轮短 `TMPDIR`、身份记录、原件归档与退出后清理；不修改 `HOME` 或 `CODEX_HOME`。

## 接入合同

仅普通 TUI 进程显式设置 `GROK_TERMINAL_BRIDGE_DIR` 时开启 Unix socket。该根必须预先创建，属于当前用户、权限 `0700`，真实绝对路径及祖先不可由组或其他用户写入。每次 TUI 启动产生独立 `t-<UUID>` 目录，包含私有 `control.sock` 和 `manifest.json`；清单控制令牌不进入日志或验收归档。退出清理 socket/清单，保留仅含身份、摘要与状态的 `receipts.jsonl`。

调用方必须将 socket 对端的内核进程身份、UID、实际二进制及当前终端 PTY 对应核验；清单里的 PID、进程名或存活检查不能独立证明身份。InfiniShell 的生产调用方尚未接入，不能仅凭清单放开普通入口。

帧为四字节大端 JSON 长度及 UTF-8 JSON；请求外层为 `{"token":"<private>","request":{...}}`。响应使用相同帧格式。`request.operation` 支持：

- `state`：传 `instance_id`，返回可信可用状态及短期一次性租约。
- `submit_if_idle`：传当前租约的 `lease_id/session_id/binding_epoch/input_epoch`、`instance_id`、相同且规范的非空 UUID `message_id/prompt_id` 以及字面 `text`。
- `query_receipt`：传 `instance_id/message_id`，仅查询原事务。处于未确认状态时可向原生 actor 补查持久收据，绝不重发输入。

空闲检查与领取在 TUI 同一事件循环完成；原生 actor 再检查会话运行、队列、审批及真实后台任务。普通原生权限和 Hook 继续生效。先将一次领取账本刷盘，再发专用原子接口；只有用户消息写入原生历史且持久化屏障成功才记 `native_acknowledged`。普通回合完成、Hook 拒绝、本地派发和连接断开不构成此确认。已领取的相同 ID 只读原收据，未知结果不得自动生成新 ID 重投。

定制构建使用独立 leader 名称和精确客户端登记标记，拒绝旧客户端混入；旧 leader 不支持专用能力时不发输入，重连撤销旧租约。构建时必须显式设置上列 `GROK_VERSION`，使版本输出表明这不是官方发行二进制。Windows 尚未实现对应原生传输，其他平台实机验收按用户指令后置，不记为通过。

独立运行验收必须使用私有 `GROK_HOME`、`--no-auto-update` 和该私有配置中的 `[cli].auto_update = false`。上游未实现 `GROK_DISABLE_AUTOUPDATER` 环境变量，不能用它代替上述关闭方式；不修改用户默认配置。

## 验收边界

本补丁尚不能关闭 G01：需要独立二进制身份、InfiniShell 生产接入、真实普通 PTY 中文／多行／长文本、原生审批保护、旧会话／重连／重复点击及双语界面验收。离线回归与编译结果分别记录，不冒充这些功能证据。上游完整 `--tests` 检查目前还会因未公开的 `docs/internal/25-enterprise.md`、`22-environment-variables.md` 而失败；本补丁不伪造这些内部文档，定向门禁明确选择库测试。上游库测试另缺一个 `base64::Engine` 导入，补丁仅补该必要导入以运行现有测试。
