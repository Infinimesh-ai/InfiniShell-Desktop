# 原生终端通知插件的回合关联审计

审计日期：2026-09-16，macOS arm64。只读取得上游源码和本机 CLI 帮助，未改用户配置、未安装或更新正常用户插件、未发送模型请求。后续在独立临时 CLI_HOME 中通过官方 marketplace 执行原生安装与修补验证；Cargo 由主代理统一执行。

## 已核实的来源和字段

| 对象 | 固定版本 / 提交 | 原生字段与实际通知行为 |
| --- | --- | --- |
| Claude Warp 插件 | `2.1.0`，`bb6c1cf2f5cd7eb609678a2b175e7f4504ca61c2` | builder 不透传回合关联键；与 2.2.0 的三个原始脚本相同，但 hooks 清单缺 StopFailure |
| Claude Warp 插件 | `2.2.0`，`8c28e936ae51cbb23a1a5657fca2bfd30cf06f12` | builder 只取 session/cwd/project；Stop 等待 0.3 秒后扫描转录最后的用户与助手消息；新版加入 StopFailure |
| Codex Warp 插件 | `0.4.0`，`31ce59d9011cfb1d78f265649a228dac5de58d76` | builder 丢弃 `turn_id`；Stop 使用当前 `last_assistant_message`，但没有回合关联 |
| Codex CLI | `0.147.0`，官方 `rust-v0.147.0` schema | UserPromptSubmit、Stop、PermissionRequest、PostToolUse 明确包含原生 `turn_id` |
| Claude Code | 本机 `2.1.273`；官方文档规定 `prompt_id` 要求 ≥ `2.1.196` | 公共 hook 输入中的 `prompt_id` 标识当前用户提示词；不是可任意替换为 `turn_id` 的字段；第一条输入之前可缺失 |

对应一手来源：[Claude 插件 2.1.0](https://github.com/warpdotdev/claude-code-warp/tree/bb6c1cf2f5cd7eb609678a2b175e7f4504ca61c2/plugins/warp)、[Claude 插件 2.2.0](https://github.com/warpdotdev/claude-code-warp/tree/8c28e936ae51cbb23a1a5657fca2bfd30cf06f12/plugins/warp)、[Codex 插件 0.4.0](https://github.com/warpdotdev/codex-warp/tree/31ce59d9011cfb1d78f265649a228dac5de58d76/plugins/warp)、[Codex 0.147.0 hook schema](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/hooks/src/schema.rs)、[Claude hooks 公共输入](https://code.claude.com/docs/en/hooks#common-input-fields)。

两款通知插件均没有原生 `event_id` 或 `sequence`。Codex 仓库中 orchestration 的 `sequence` 是 Oz 父子消息队列序号，与本地 hook 通知顺序无关，不能借用。`PLUGIN_COMPATIBILITY_EVIDENCE.json` 保存了真实上游 Bash builder 对构造输入的执行结果：即使输入含原生关联键，原实现也将其丢弃。该证据明确标记为脚本回放，不冒充 CLI 实际回合捕获。

## 已确认的错误路径

同一 native session 内发生 `PromptSubmit(A)` → `PromptSubmit(B)` → 迟到的 `Stop(A)` 时，现有通知只有 session ID，接收端无法判断 Stop 属于 A。Claude 旧脚本还可能在 0.3 秒等待后读取到 B 的提示词，从而让过时 Stop 看起来像 B 的完成。

只有 session ID、重复内容哈希或插件开始执行时的本地递增序号都不能补回原生因果关系：旧回调若在 B 之后才开始执行，本地序号反而会把它标为最新事件。依赖 `transcript_path` 的“最后一条用户消息”同样不可靠。Claude 官方文档明确说明转录异步写入，并提供当前 hook 的 `last_assistant_message` 作为直接来源。[Stop 输入说明](https://code.claude.com/docs/en/hooks#stop-input)

## 随附的最小修补件

产物位于 `app/assets/bundled/cli-agent-plugins/claude/` 和 `codex/`。每个目录包含替换文件、`upstream.patch`、MIT `LICENSE`、`PATCH_METADATA.json` 和说明。它们是现有插件的兼容修补，不冒充新发布的上游插件版本。Claude/Codex 本地安装器已接入受控修补，只有用户触发安装或更新才应用。

- Claude：五个文件，透传 `prompt_id`；Stop 改用当前载荷中的助手最终文本，删除等待与转录末尾扫描；Grok 实际运行期环境出现时退出全部 Claude 通知路径，包含旧协议回退。
- Codex：四个文件，仅透传受测 schema 明确的 `turn_id`；补上 `stop_hook_active` 检查。
- 两者：缺少对应原生关联键的 stop/stop_failure 改为普通 notification，并携带 `terminal_unverified=true` 供接收端明确降级；没有最终文本不声称成功；JSON 中 C0/C1 均转义。不生成事件 ID 或序号。
- 通知脚本使用原有 Bash、jq，补齐兼容 TTY 的 tmux DCS 传输，不改变权限策略、工具调用或上游插件版本号。Codex 安装器另将固定 Git marketplace 迁为随附完整本地来源，具体边界如下。

接收端须分别保存 `turn_id` / `prompt_id`，在处理终态和回合内事件之前校验当前原生关联。对已进入新回合却缺少可关联字段的旧版本事件应降级。只有发布替换文件而不改接收端，不能修复过时 Stop 覆盖新回合的问题。

本地安装器在写入前 probe 精确 CLI 版本（Claude 2.1.273 / Codex 0.147.0）与 Bash、jq；子进程通过 `command::async` 结构化 argv 派生，显式传递同一个配置目录与 PATH，避免 login shell 重定向配置路径。Claude 最低插件版本为 **2.2.0**，2.1.0 仅允许作为已审计的升级来源，不能缺少 StopFailure 仍被认为就绪。

`PATCH_METADATA.json` 记录完整上游文件树摘要。自动安装/更新前拒绝自定义 marketplace、重复安装记录、多个 Codex 缓存版本、链接或被修改/增加的文件，保持禁用状态。Claude 已在受测版本时直接应用修补，不重复安装原生插件；未知上游新版本不会自动修补，原生更新成功不等于本集成兼容。

Codex 0.147.0 原生重启会刷新固定 Git 来源并强制覆盖同版本缓存，真实 GUI 已复现修补丢失。因此 Codex 默认安装改为随附完整 36 文件来源，使用 `SOURCE_METADATA.json` 验证原始与部署摘要，在私有暂存 HOME 由原生 CLI 注册和安装，再把目标 marketplace、插件启用字段及受控缓存迁入真实 HOME。插件 ID、无关配置、禁用的 orchestration 及原有 hook 信任保持；不再以反复修补缓存作为可靠安装方案。未知本地来源仍被拒绝，仅完整匹配本应用固定目录与清单的来源获准。Rust 事务 build16 的 TOML 格式误判已修，build17 回归通过；新 GUI 重启和其他平台仍待验。详见 [Codex 缓存刷新与持久来源](CODEX_PLUGIN_CACHE_REFRESH.md)。

Claude 脚本替换先保存受控文件内容与权限，使用同目录临时文件、同步写入和原子重命名；任何写入或后验失败只恢复本次已替换文件，检测到并发用户编辑则不覆盖并报告恢复不完整。进程被强制退出可能留下部分已修补文件；下次操作接受原始/已修补的已知摘要组合并完成修补，不会把部分状态当成已就绪。Codex 则保留完整旧缓存与阶段资料，失败时仅回退仍匹配本次写入的限定配置字段和缓存；无法确认时保留恢复资料并报错。两者均不回滚无关设置。Codex 配置提交是重读比较加原子文件替换，仍有跨进程并发窗口，不能称为原子 CAS。

本地 `needs_update()` 使用一秒短期缓存；到期先比较文件元数据，变化时才重新计算受控脚本摘要，安装/更新调用立即使缓存失效，避免 UI 每帧读取完整插件。原插件版本号相同但修补丢失仍需要更新。

## 验证与剩余边界

- 修补第 3 版通过各目标版本（Claude 2.2.0 / Codex 0.4.0）固定上游副本的 `git apply --check`；Claude 2.1.0 仅用于升级预检，不应用新 hooks 清单。
- `python3 script/cli-agent-parity/plugin_compatibility_tests.py`：13 项离线测试通过，覆盖原生键差异、相同文本的不同回合、无键终态降级、旧 Stop 与新转录、Stop 继续运行、后台工作、Grok 兼容加载、C0/C1 及产物摘要。
- `python3 script/cli-agent-parity/notification_patch_tests.py`：13 项文件事务与离线交付测试通过，覆盖原子失败回滚、并发修改保护、禁用配置、符号链接、自定义文件和独立导出。
- `notification_patch_tests.rs`：14 项定向 Rust 测试已提交主代理统一执行；本节不把未回收的 Cargo 结果视为通过。
- `PLUGIN_INSTALLATION_EVIDENCE.json`：两款 CLI 通过官方 GitHub marketplace 原生安装到临时目录；独立导出的脚本在该目录应用/重复应用/后验通过，配置摘要保持不变；主动加入自定义文件后预检拒绝且未写文件，恢复夹具后校验通过。该证据针对真实 CLI 安装 + Python 文件事务，不冒充 Rust manager 实际运行、模型或原生 hook 生命周期。
- 原生 CLI hook 输出的完整生命周期、Rust manager 实际进程链、接收端旧回调回归、Windows/Git Bash、完整 SSH/UI 和双语 GUI 仍需主验收流程验证；回环 SSH/tmux 脚本传输已完成独立正负验证，见 `SSH_TMUX_VERIFICATION.md`。
- Stop 本身不能证明其他并行 Stop hooks 没有阻止结束；本修补不把 hook 顺序问题宣称为经过验证的模型或任务完成协议。严格的托管完成仍应来自原生结构化回合终态。

本轮增加了兼容性、修改冲突、失败与手动修补说明四个用户界面消息键；英文与简体中文由主代理同步维护并统一执行 i18n 和布局门禁。

## SSH、容器与 Windows 的交付边界

现有 footer 将 SSH 和容器会话路由至手动模式。`LocalCommandExecutor` 不具备远端文件传输能力，本地安装器不能把本机路径发送给远端或用本地摘要认定 SSH 插件已修补。随源码提供 `script/cli-agent-parity/apply_notification_patch.py`，可导出含两套修补、许可、清单和独立安装脚本的目录：

```sh
python3 script/cli-agent-parity/apply_notification_patch.py --export /tmp/infinishell-notification-patch
```

将此目录完整复制到目标主机，保持目录结构，并校对 `SHA256SUMS.json`。Codex 导出包包含完整持久来源与安装 helper，可直接安装或更新；Claude 需先用原生 CLI 安装上表所列受测插件，再执行修补。两者使用目标机器的 Python 3.11+，例如：

```sh
python3 /path/to/infinishell-notification-patch/apply_notification_patch.py --agent codex --cli-home "$HOME/.codex"
python3 /path/to/infinishell-notification-patch/apply_notification_patch.py --agent claude --cli-home "$HOME/.claude" --check
```

`--cli` 可传目标机器 CLI 的绝对路径；`--cli-home`、CLI argv 与文件 API 全部结构化处理，不把路径拼接进 shell。`--check` 只核对且未修补时返回非零。两个命令并不替代目标机器上的真实 hook、审批或 tmux 验收；导出后的脚本已在 macOS 独立目录执行，导出安装命令本身尚未通过 SSH 执行；另有回环 SSH/tmux 受控插件通知回放通过，不能混为原生远端安装或产品验证。

Windows 自动集成继续关闭。离线脚本提供显式 `--files-only`：仍检查真实 CLI 精确版本、禁用配置和全部摘要，原子替换时先关闭文件句柄，使用 Windows 可用的文件 API；报告明确 `files_only=true`、`bash_jq_probed=false`、`native_notifications_verified=false`。示例在 PowerShell 中使用目标 Windows 的 Python 3.11+ 与 CLI：

```powershell
python C:\patch\apply_notification_patch.py --agent codex --cli-home "$env:USERPROFILE\.codex" --files-only
```

这个选项只交付文件，不证明 Git Bash、jq、路径转换、TTY、OSC 或通知已可运行；文件事务本身的 Windows 实跑也仍需同一修改提交的 Windows runner 验证。WSL 应在 WSL 内使用其自己的 CLI_HOME 与 POSIX 脚本，不能将 Windows 本机缓存当成 WSL 缓存。上述限制属于未完成的跨平台验收，不能计为 P5 完成。

## 路径参数与第 2 版清单修补

上游两款 `hooks/hooks.json` 的脚本路径均未引用，空格会拆分命令。第 2 版将该清单作为第 4 / 3 个受控文件，与脚本共同预检、原子替换与失败回滚。清单沿用各目标版本的固定原始 SHA-256；自定义 hooks 仍在写入任何脚本前被拒绝，未扩大匹配范围。修补后的摘要也参与 `needs_update()` 与后验检查。

Claude 采用 `command: "bash"` 与 `args: ["${CLAUDE_PLUGIN_ROOT}/scripts/…"]`：原生 exec form 对每个参数替换路径，没有第二次 shell 解析。该机制已经用本机 2.1.273、隔离配置及 `--init-only` 实测三类目录（空格与中文、单引号、双引号，均含命令替换字符），只执行 SessionStart 观测脚本，不请求模型。`probe_notification_paths.py` 和 `PLUGIN_PATH_EVIDENCE.json` 保存可重放步骤与非敏感证据；它不代表完整通知生命周期。[Claude exec form 官方契约](https://code.claude.com/docs/en/hooks#exec-form-and-shell-form)

Codex 保留其 shell-form 契约，命令是 `bash "$PLUGIN_ROOT/scripts/…"`，变量值由 shell 在参数边界内展开，不把目录内容拼接为待解释代码；固定 0.147.0 runner 将命令和 handler 环境分别传给进程。Bash/jq 测试从真实随附 hooks 清单执行 Stop，验证中英文路径、单/双引号与命令替换字符不改变 JSON，也没有生成注入标记。**Codex 原生 SessionStart 路径现已隔离实测通过**，见下方补充；脚本回放仍不能代替完整通知生命周期。[Codex plugin 环境契约](https://learn.chatgpt.com/docs/hooks#plugin-hooks)、[固定版本 runner](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/hooks/src/engine/command_runner.rs)

`PLUGIN_PATH_TRANSACTION_EVIDENCE.json` 对完整固定上游文件树在含空格、中文和特殊字符目录下执行真实 Python 文件事务：新清单写入失败恢复全部已改脚本，重复应用及完整树摘要验证通过，自定义清单拒绝且无写入。Rust 中增加相同故障边界回归，待主代理统一执行。新增测试夹具脚本显式写 LF 与 UTF-8，避免原生 Windows Python 默认 CRLF 破坏 Bash shebang。

Windows 文件事务仍需同 SHA runner 实测。尤其 Codex 0.147.0 的默认 Windows hook shell 为 `cmd.exe /C`，不能把 Git Bash 回放通过视为该原生入口兼容；自动安装门禁保持关闭。此次修改没有新增用户可见文案，无需本地化变更。

## Codex 原生路径与信任状态补充

`probe_codex_notification_paths.py` 和 `CODEX_PLUGIN_PATH_EVIDENCE.json` 使用原生 marketplace/add 安装独立静态观测插件，复用随附 SessionStart 命令定义。三类 CODEX_HOME 路径（空格与中文、单引号/命令替换字符、双引号/命令替换字符）均正确传入 PLUGIN_ROOT 并执行原生 SessionStart，未生成注入标记。SessionStart 在首次 turn 时执行，只有 thread/start 不足以触发；测试不提供凭据，首次输入会尝试模型连接但不验证模型生成。此结果不代表真实 InfiniShell PTY 通知、审批或完整插件生命周期。

安装后原生 `hooks/list` 返回 `untrusted`，所以安装成功不能等同于通知可用。测试仅核对并授权其自己创建的静态脚本，随后重新读取原生 `trusted` 状态；产品应引导用户在 Codex `/hooks` 中审核当前定义，不自动写入用户 `trusted_hash`。[固定版本配置契约](https://raw.githubusercontent.com/openai/codex/rust-v0.147.0/codex-rs/core/config.schema.json)

独立边界：Codex 0.147.0 默认 shell_snapshot 的路径处理在特殊 CODEX_HOME 下出现 shell 解析问题，受控测试曾生成预设的 INJECTED 标记。原始脱敏记录见 `validation/macos-codex-shell-snapshot-path-limitation.json`；它发生于 Codex shell 快照阶段，不能归为通知 hook 失败。为了单独验证通知路径，上述正向测试仅在自己的 CLI 子进程中关闭 shell_snapshot；没有修改用户配置。默认快照启用时的特殊路径兼容仍属于外部未解决限制。

## Codex 原生 Hook 信任与安装结果

Codex CLI 0.147.0 中，插件安装成功并不表示通知 Hook 已经获准运行。`probe_codex_hook_trust.py` 将完整受控 codex-warp 0.4.0 + patch revision 3 放入隔离的原生 marketplace，执行实际 `hooks/list`，五项均返回 `trustStatus: untrusted`。没有创建 thread/turn，没有模型请求或凭据；原生配置在查询前后的字节完全相同。原始结果保存在 `fixtures/codex-0.147.0-native-hook-trust.json`，随附 `NATIVE_HOOK_TRUST.json` 固定这五项定义的原生摘要。

上游 key 由插件 ID、相对 hooks 路径、事件、组序号和 handler 序号组成。`currentHash` 是正规化 Hook 定义的摘要，在 `${...}` 路径替换之前计算，不能拿 `hooks.json` 文件摘要替代。原生配置的 `hooks.state.<key>.trusted_hash` 必须与当前原生摘要完全相等；旧摘要是 modified，缺失是 untrusted，`enabled=false` 仍阻止执行。[固定声明 key 实现](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/hooks/src/declarations.rs)、[固定正规化与信任判定](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/hooks/src/engine/discovery.rs)、[固定配置 schema](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/core/config.schema.json)

生产安装器只安装和校验文件；从不代写用户的 `trusted_hash` 或 hook 启用状态。安装/更新提示以及独立授权入口要求用户在 Codex 内打开 `/hooks`，查看当前 `warp@codex-warp` 通知定义后自行决定启用与信任。该命令在说明中不可作为普通 shell 命令自动执行。替换 Hook 定义后，旧信任可能失效，需要重新检查。

只读检测返回四种状态：不适用、需要检查、本机已配置、未知。本机已配置要求整个受控插件树匹配、五项原生摘要匹配且 CLI 曾通过本进程的精确版本预检，预检后的实际路径/文件戳没有变化；它不表示当前 CLI 会话已经采用该配置。其他配置层、运行中的旧会话或应用重启后未重新验证版本，都不能被虚构为已激活。真正生效仍取决于当前会话发出的有效通知。

界面查询使用短期缓存；到期先比较配置、整个插件树及 CLI 的元数据，只有变化才重新读取和计算摘要。安装/更新显式使对应 home 的授权缓存失效。Rust 回归覆盖未信任、旧摘要、禁用、缺失、无版本证明、无自动 shell 执行和缓存失效；实际测试结果以统一门禁日志为准。

## 第 3 版：tmux 的兼容 TTY 传输

真实 macOS 回环 SSH 中，原始 `warp-notify.sh` 在直接 PTY 可以到达客户端，但经过 `allow-passthrough=on` 的 tmux 后 Codex 与 Claude 兼容 TTY 路径均没有 OSC 777；Grok 的既有 DCS 封装可以通过。修补第 3 版给上述两个 TTY 分支加入 DCS 封套并双写所有内部 ESC，实际重跑后直连 3/3、开启透传 3/3；关闭透传时三方全部阻断，未计为通知成功。只改固定受控脚本，不修改全局 tmux 设置。

Claude 新版的 `terminalSequence` 输出字段不能含 DCS，因此只在兼容 TTY 分支封装；新版 JSON 与未知版本没有控制终端的 JSON 回退继续返回原始 OSC。四项真实 Unix PTY 回归验证直接字节、tmux 转义、现代 JSON 与无 TTY 回退；现有 13 项兼容测试也核对现代 JSON 不包含 DCS。新的 13 项文件事务测试验证 notify 替换失败恢复此前文件、用户自定义 notify 在任何写入前被拒绝。Rust 测试与 `include_str!` 同步扩充，由主代理统一编译。

Codex 对第 3 版完整树重新执行原生 `hooks/list`，五项定义仍为 untrusted，配置字节未变；`hooks.json` 本版没有变化，所以原生定义摘要与第 2 版相同。历史响应保留在 `fixtures/codex-0.147.0-native-hook-trust-patch2.json`。通知脚本内容摘要由本应用完整树校验覆盖，不能用相同原生定义摘要推断自定义文件安全或已激活。

完整范围、来源、复跑命令与未通过门槛见 [SSH/tmux 验证记录](SSH_TMUX_VERIFICATION.md)。本次没有新增用户界面文案，无需本地化变更。
