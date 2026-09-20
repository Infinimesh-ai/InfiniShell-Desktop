# CLI 对齐 Goal 阶段性交接（2026-09-19）

用户要求在当前阶段收尾，由新会话继续。**目标未完成；不能标记为完成或发布已验收声明。** 先读仓库 `AGENTS.md`、本文件、`PLAN.md`、`CLI_AUTOUPDATE.md`，再核对真实工作区。此前全部 P0–P5 和自动升级范围继续有效。

## 工作区与保存边界

- 仓库：`/Users/zhishi/Tools/github/InfiniShell-Desktop`。
- 当前分支：`codex/cli-agent-parity`；HEAD：`af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`。
- 本阶段没有新提交、推送或派发 CI。根工作区含大量此前授权改动、已有暂存文件和当前未验证改动，不能执行全局 reset、clean 或覆盖。
- 用户独立改动 `app/src/ai/agent_providers/tools/web_runtime.rs`、`websearch_tests.rs` 没有纳入本 Goal 的冻结输入；保留原状。`validation/gui-7e065085/` 同样保留。
- `.worktrees/cli-agent-parity-validation-34` 是当前冻结 **source45**，不是根工作区最新状态。245 个文件，193 个与父提交不同，清单 `/tmp/infinishell-cli-parity-official-45-inputs.json`，SHA-256 `8f19bb52ac6b019618dd3fa79be5c05108d8be9cec9b0b54483c6334b955e2f5`。
- 根工作区后续统称 **source46 候选**，尚未冻结，尚未运行当前候选的 Cargo 门禁。不要把 source45 的绿灯套给它。
- 共用构建缓存：`/Volumes/ORICO/CargoTarget/InfiniShell-Desktop-cli-agent-parity-dbee1ecae`。本轮真实 CLI 进程已结束；新会话先确认外盘仍挂载，再准备新快照和构建。保留 source45 的清单与旧证据，覆盖冻结树前先归档。

## 持续有效的产品决定

已有安装默认跟随原渠道；新安装默认 Codex `latest`、Claude `latest`、Grok `stable`。消费者可逐 CLI 开关自动升级，手动选择 Codex `Latest / Alpha`、Claude `Latest / Stable`、Grok `Stable / Alpha`。不能自动从正式渠道跳到 Alpha。Claude latest 是官方默认，未声称有真实用户份额统计。

显式渠道只在成功更新事务后同步原生渠道；启动和普通探测不得偷偷改全局配置。同版本但渠道不一致，也走配置事务并尊重忙碌状态和自动升级开关，不运行安装器。失败回滚、配置竞争和未确认升级不得重放。版本检测不等于托管协议能力已验证。

普通终端与托管任务有不同能力合同。普通 PTY 的 Stop hook 不能证明最终成功，继续显示结果未知；只有可靠原生终态和结果回收才能标成功。活跃任务重新关联与启动新进程继续旧会话必须区分。

## 已取得的真实结果

详细收据见 [自动升级专报](OFFICIAL_42_AUTOUPDATE_VERIFICATION.md)、[最新版接口核验](LATEST_PROTOCOL_INTERFACE_20260919.md) 和 `validation/`。下面均限定受测快照和平台，不是最终交付验收。

| 范围 | 当前结果 |
| --- | --- |
| source45 本地 | `cargo check -p warp`、i18n 11 项、受影响 Rust 1979 项、Python 321 项、共享动作结果 5 项、TUI 编译和桌面构建通过；严格签名和完整双语资源嵌入通过 |
| Codex 0.155.1 基础适配器 | source45 两轮交互、允许／拒绝及文件效果、追加、取消、重复消息 ID、进程退出后同原生会话继续和结果回收通过；未覆盖 GUI 重启、父子或其他平台 |
| Claude 2.1.278 基础适配器 | source44 的 8 输入基础链通过；source45 父子和固定文件链在任务输入前被旧摘要门禁拒绝，失败保留 |
| Codex 渠道 | 0.155.1 → 0.156.0-alpha.7 产品通过；正向外层因官方 `codex → bin/codex` 别名失败，后续完整 42 文件／10 目录审计确认精确别名；另案 Alpha → Latest 产品与外层通过，未改写原失败 |
| Grok 渠道 | Stable 1.0.34 → Alpha 1.0.38 → Stable 1.0.34 均通过；同版本渠道同步也通过，未启动安装器 |
| Claude 渠道 | 2.1.278 → Stable 2.1.267 安装成功，但额外改写 `.claude/.claude.json`，整体验收失败；候选修复尚未通过 Cargo 或真实更新 |
| 原生后台更新 | Codex 约 303 秒后确实启动定时安装脚本，静默结果未知；Grok Stable 后台缓存和其后独立手动检查分项通过，总结果因市场初始化字段变化失败；Alpha 只有新缓存及后验无进程，原夹具缺完整退出回执 |
| source45 GUI | 英中说明、来源未知／未安装状态、Grok 渠道菜单、关闭开关和语言重启保存通过，4 张截图；成功安装、可升级、验证中布局仍未验 |
| Grok 1.0.34 原始 ACP | 第十二例四阶段通过：允许读取、拒绝读取、真实输出后取消、正常退出后同原生会话加载并回忆结果；4 输入、19 RPC、2 进程、2 正常 EOF。生产适配器未据此开放新能力 |

source45 测试二进制：`debug/deps/warp-3ad688c9d0ab1400`，SHA-256 `48c0c48fcf38fcdab111dc09362e422d60c581bb605acf12cbdcc226fd622913`；主程序相对上述缓存路径为 `debug/bundle/osx/InfiniShell.app/Contents/MacOS/infinishell`，摘要 `834c6921edcc74b0b94b5cbbd732e3dc00a925b5dede5f2db33a52ab1e7e37f9`。后续重新构建后这些路径可能被覆盖，必须以摘要核对。

第十二例原始 ACP 清单摘要 `37aee0beeaee9ec8ff1d27b5449a41c73478be20a449cd3c14c6a50ac39da5eb`，结果见 [安全收据](validation/grok-1034-native-p0-native12.safe.json)。它是独立原生探针，不是 source45 生产适配器或整体 Goal 通过。第 1–11 例失败全部保留；第 9 例前三阶段通过，第 10 例回合超时，第 11 例旧联合结果断言失败，不能反推具体原因。

## 根工作区 source46 候选变动

1. Claude 固定权限预检扩展精确官方 2.1.278 平台摘要，仍核对真实控制接口；新增 8 个 Rust 用例尚未运行。2.1.278 零模型只读五控制预检另案通过。
2. Claude 更新改用监督代次拥有的私有配置目录，原配置只在已成功的渠道 CAS 事务中发布。保留系统策略；远端缓存中的版本上下限和禁更新约束取限制交集，动态 `policyHelper(s)` 无法保真时降级。新增生产／恢复测试尚未 Cargo 验证，也未重新跑 Claude 渠道案例。
3. Grok 通知 worker：复用宿主 CLI worker 入口和 fs4 系统锁；只发有界结构化帧，完整 OSC／tmux 帧不超过 4096 字节。锁不按 PID／时间删除；退出由系统释放。最终字段、Windows ACL／Console 和真实 PTY 回归请核对代码与代理交接，不能声称已验证。
4. Grok 插件 0.1.3 候选：10 hooks，补 StopCancelled；Node 做纯映射并调用 `WARP_CLI_AGENT_NOTIFY_EXECUTABLE`，不再自己写 TTY 或持久化会话锁。过滤子代理事件，保留原生 prompt_id；未支持 helper 时明确降级。旧 0.1.0／0.1.1／0.1.2 精确配方与新测试尚需统一门禁。旧安装后探针仍假定 0.1.2，待更新。
5. 应用 `EventCursor` 将 Grok 纳入 prompt_id 关联；旧取消不能覆盖新回合。普通 Notification／未知事件不解除取消等待。新增回归未跑 Cargo。
6. 本地 Unix／Windows 只宣告本次宿主 worker，容器清除宿主路径，WSL 不透传。SSH 每跳先清除父路径，再从已部署的远端同平台布局绑定；插件仍需握手。远端未安装 worker 时不能冒称可用。修改 bash／zsh／fish／PowerShell bootstrap；bash 与 zsh 语法检查通过，本机 fish 命令未找到，PowerShell 和跨平台尚未验证。
7. Grok 原始 P0 探针补合法非终态通知／内部 reload／冷加载历史别名，严格保留会话、回合、工具与审批合同；文件标记允许有界排版文字，工具完成独立校验。47 项离线用例通过，随后第十二例真实四阶段通过。

本次新增 fs4 的 app 依赖尚未统一刷新并核对 `Cargo.lock`；下一轮冻结前必须处理。当前没有 source46 编译通过的证据。

所有代理已停止修改，最终增量和文件摘要已存入仓库：

- [Claude 更新隔离交接](validation/source46-draft-claude-update-handoff.json)：新增 19 项 Rust 未运行，既有 Python 22 + 10 项通过，无已知半接线；策略保全与 Windows 仍待编译／实测。
- [Grok worker 交接](validation/source46-draft-grok-worker-handoff.json)：15 个普通测试、1 个 ignored 子进程夹具和 1 个 CLI 解析测试已写未运行；新 worker 在任何平台尚未编译或运行。
- [Grok 0.1.3 插件交接](validation/source46-draft-grok-plugin-handoff.json)：Node 语法通过，原 `grok_plugin_tests.cjs` 尚未迁移，真实结果为 **5 通过、9 失败**。原 CLI 支持集合仍为 1.0.30；0.1.3 不能计为完成。
- [阶段工作区摘要](validation/stage-handoff-20260919-working-state.json)：记录当前候选文件、保留的暂存数量、源码摘要与未验证边界，只是续接清单，不是冻结或验收通过证明。

阶段末限定 `git diff --check` 通过；PowerShell 文件的既有 CRLF 规范提示保留。没有运行新 Cargo 门禁，也没有为了收尾提交不通过的候选。

## 下一会话执行顺序

1. 核对 Goal 状态和工作区、外盘、代理最终交接；不要重新执行已有原生案例或覆盖旧 receipt。先审阅 source46 所有实际差异，特别是 Claude 策略保留／恢复清理、Windows 锁和取消事件归属。
2. 补完 0.1.3 Node 离线测试、`run_installed_grok_hook.py`／`installed_grok_hook_tests.py` 的真实 worker 接线、必要 UI 双语说明。核 Cargo.lock 和所有新增 worker 枚举的穷尽分发。未完成项不得略过。
3. 按明确文件列表归档 source45、冻结新的候选；保留用户独立改动不纳入。复用 `/tmp/infinishell-cli-parity-official-45-{freeze,gates,python-gates,extra-gates,main}.py` 的校验方式，但创建新文件名／清单，不修改旧记录。新增 warp_cli 和 worker 用例须纳入受影响门禁。
4. 运行 `cargo check -p warp`、`cargo test -p warp --lib i18n::tests`、受影响 Rust／Node／Python，以及 TUI 和仓库必需门禁。构建真实主程序并绑定同一源码清单后，才做新原生验收。
5. Claude 2.1.278 固定文件／父子／批量取消／审批取消链，及修复后的 Latest ↔ Stable 和同版本渠道同步，用独立新夹具跑。旧失败不覆盖。
6. 依据 Grok 1.0.34 原始 P0 通过证据做最小生产适配，然后跑实际生产进程链。当前 `grok.rs` 仍只有 1.0.30 的完整能力，1.0.34 的审批／固定策略／本地工具／技能门禁未开放。不要一次把未知的 queued、close、SDK 子任务能力全部设真。
7. 完成 Grok 0.1.3 在真实 1.0.30／1.0.34 的验证／加载／StopCancelled 派发和实际 GUI 消费；真实 PTY 并发／锁持有者退出、tmux、SSH、Windows ConPTY 分别验证。
8. 最终按 PLAN 补齐三款完整链：新建 → 两轮 → 允许／拒绝 → 追加 → 取消 → 继续 → 应用重启 → 恢复 → 结果回收；父子消息、重投、故障恢复、安装缺失／不兼容等组合仍须落实。完成双语布局和真实 IME。
9. 本地通过后，按跨平台技能对包含实际改动的同一提交验证 macOS、Linux、Windows、完整 workspace 和 SSH／tmux。此前 CI14 对旧 af1040 的 Linux 通过、Windows 缓存探针失败，不代表本候选结果。

## 私有验收资料

真实认证已经得到用户授权，不要要求重复登录或把密钥复制进聊天／仓库。以下仅是已有私有运行入口，使用前核对存在、权限和当前认证状态；失败时准确说明：

- Claude：`/private/tmp/infinishell-claude-login-81lk9az5/api-environment.json`；不打印字段或值。
- Grok：`/private/tmp/infinishell-grok-official-login-ap0byk0e/home/.grok`；既有 OAuth 授权源。只用正式运行器按不透明字节复制到新 0700 夹具并在结束后删除副本。
- Codex：用户本机正式认证，由既有运行器复制；不遍历、不打印认证内容。
- 三款官方版本／渠道包清单：`/private/tmp/infinishell-cli-channel-preparation-6pncmu6p/preparation.json`。
- Grok worker 真实 PTY 设计依据：`/private/tmp/infinishell-grok-pty-transport-2bvfk2fr/{REPORT.md,HELPER_PLAN.md,report.safe.json}`。无锁并发通知实际损坏，不能回退旧无锁发送。

本文件和验证报告是续接入口。它们不替代下轮对当前代码、磁盘与外部服务的实际核对。
