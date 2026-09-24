# Mac 固定版本适配（2026-09-24）

> **2026-09-25 最终收敛**：Mac 必需产品能力与真实链已完成；后续平台补验与最终源码关联亦已闭环，见[固定版本验收表](ACCEPTANCE_FIXED_VERSIONS_20260924.md)。下文记录 Mac 各实际构建批次，不把它们改写为最终提交二进制实测。

本轮按用户要求先完成 Mac 能力接入，不逐阶段运行跨平台 CI，也不继续追逐 CLI 新版本。版本固定为 Codex `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41`。工作沿用 `codex/cli-agent-parity`，基线为 `104f5b8a1e857c49c5bbb782c78ddbfd89eef628`；证据来自未提交工作树的多个构建批次，不是最终同提交或完整 Goal 验收。

## Mac 收敛时的产品范围（构建批次快照）

- 三款固定版本进入 macOS 正式托管路径，不依赖新版 test-only 候选开关。`LocalCLIManagedTasks` 功能开关保留；V4 至 V8 默认 Mac 构建无需额外编译 feature，实际 GUI 已可进入本地 CLI 任务。Mac 验证完成后，固定版本的 Linux／Windows 适配已合入工作树，尚待集中平台验证。
- Grok 会话目录缺失时执行一次与当前会话、版本绑定的拉取；目录、工具、技能、权限及超时继续严格验证。继承 CLI 设置模式支持根任务与启动时绑定的单个技能，禁止父子任务工具。固定权限与 SDK 入口已接入 GUI，固定模式不能选择技能；V5 read/files 的 2／6 输入原生链通过，V7 单技能启动与恢复的两轮原生行为经离线复核通过。原 runner 因恢复历史未按回合切分仍为 false；修正夹具已纳入 V8 编译及定向回归，没有重写原收据。跨宿主固定 profile 冷恢复和父子完整链已有 V8 实测。
- 应用重启会重新连接身份核验通过的存活宿主，包括已完成回合的宿主；保留会话、结果和输入确认状态。其他终态记录保持原样，不放宽退出确认，不自动重发输入。
- 普通终端已有三款原生通知及 SSH/tmux 证据。SSH 富输入仅在当前 block、PTY 和原生会话的可信 hook 绑定成立时放行；重连缺少插件版本时保持未知，不再误报“更新插件”。
- Grok `1.0.41` 的初始插件 hook 加载缺口通过产品安装器管理的通知桥接补齐；保留版本、信任、配置禁用和同名文件冲突门禁，原插件与桥接按事件去重。最终普通会话不需要手动执行插件 reload。通知只通知，不授予工具权限。

## 已通过的托管原生链

| 项目 | 实测结果 | 范围 |
| --- | --- | --- |
| Codex V9 正式父子协调 | 1 个子任务、6 次接受输入、3 次 SDK 调用、2 次只读审批；父权限上限、双向原生 ACK、自动结果 ACK、显式继续后 inspect 回收持久结果、两次清理确认 | 父到子经产品托管邮箱，子到父经原生 SDK；没有子文件效果、GUI 或自动唤醒证明；[V9 独立收据](validation/macos-fixed-versions-20260924/codex-parent-child-duplex-v9.safe.json) |
| Claude 正式父子协调 | 5 次审批、4 次工具、5 次输入、3 次原生执行；子文件效果、双向消息 ACK、inspect、自动结果 ACK、两次清理确认 | 固定 `claude-sonnet-4-6`；官方 CLI 自行访问已授权默认账户，运行器不提取 Keychain 凭据 |
| Grok 正式根任务 | 8 次输入；两轮文本、写入允许／拒绝及文件效果、排队、取消、同一原生会话恢复、两次清理确认 | 固定 `grok-4.7`，两代监督进程；父子链另行验收 |
| Grok V5 固定读取 | 2 输入／2 ACK；新任务允许读取、同原生会话恢复后拒绝读取，两次审批均有回执，两次退出与清理确认 | [原生收据](validation/macos-fixed-versions-20260924/grok-fixed-read-v5.safe.json)；直接适配器保留同一 profile 存储，不证明跨宿主冷恢复或操作系统沙箱 |
| Grok V5 固定文件操作 | 6 输入／6 ACK；写入允许／拒绝、编辑允许／拒绝、待审批取消、冷恢复读取，8 次审批已处理或取消，文件字节与原生历史相符 | [原生收据](validation/macos-fixed-versions-20260924/grok-files-v5.safe.json)；6 次清理确认，观测到 37 个代理 TLS 连接，限定预算 64 |
| Grok V7 单技能启动／恢复 | 2 输入／2 ACK／2 审批；两轮均有输入前技能路径目录、一次精确上下文读取、原生 Completed 和正常退出 | [原始失败与独立离线复核](validation/macos-fixed-versions-20260924/grok-selected-skill-v7.safe.json)；旧 runner 为 false，离线复核零新增输入，不改写旧失败；历史切分夹具已修并通过 V8 编译及回归。[来源与摘要索引](validation/macos-fixed-versions-20260924/grok-runtime-receipts-index.safe.json) |
| Rust 通知插件安装事务 | 早期 5/5 通过；Grok 桥接 V4、V5 分别通过 6 个生产安装步骤，V5 安装结果与源码 helper 摘要相符 | 安装测试均为零模型输入、无凭据；与 CLI 可执行文件升级分别计证 |

Grok V8 完整父子链已通过：2 个任务、6 次输入、4 次 SDK 调用，父 4 个运行代次、子 2 个运行代次；父权限上限、双向原生 ACK、自动结果 ACK、inspect 回收、跨宿主冷恢复和 3 份宿主退出清理确认均通过。持久 profile 存储已与进程状态分离，并核对应用根及宿主代次，恢复继续使用原生会话。[V8 收据](validation/macos-fixed-versions-20260924/grok-parent-child-v8.safe.json)。V7 曾因存储位于旧宿主 `native` 下而在冷恢复失败，原真实失败继续保留，不追改为通过：[诊断](validation/macos-fixed-versions-20260924/grok-cold-resume-storage-diagnosis-v7.safe.json)。

上述托管收据见[原生链与插件](validation/macos-fixed-versions-20260924/native-summary.safe.json)。Codex 首次等待完成态父任务自动唤醒、Claude 首次模型拒绝合成提示的失败保留；后续分别改为显式继续和普通临时项目提示，没有放宽权限、版本或结果校验。

## GUI、恢复与输入

Codex 正式 GUI 恢复链通过：同一路径重启后宿主 owner epoch 从 1 变为 2，没有重发首轮输入；接回后新问题获得正确上下文；显式断开取得退出确认后，新宿主恢复同一原生会话。共 3 次用户输入、2 个宿主运行期、3 次任务运行、2 次退出与清理确认，测试进程残留为 0。首次完成态宿主遗漏的失败及人工清理记录保留。[恢复收据](validation/macos-fixed-versions-20260924/gui-recovery.safe.json)、[重启续接截图](validation/macos-fixed-versions-20260924/chinese-recovery-second-result.png)、[冷恢复截图](validation/macos-fixed-versions-20260924/chinese-recovery-cold-result.png)。

Codex GUI 的附件、技能、评审和文件引用实际进入原生请求；只读任务经过一次明确审批后读取文件，项目文件保持不变。类型化图片通道已观察到，但该轮模型颜色判断错误，不能计作图片理解正确。共 2 次用户输入、1 次审批回复，断开与事件日志确认完整。[白名单收据](validation/macos-fixed-versions-20260924/gui-codex-composer-v2.safe.json)、[提交前界面](validation/macos-fixed-versions-20260924/codex-composer-all-context-before-v2.png)。

Claude `2.1.280` 的 V7 默认构建 GUI 组合链已通过：技能回合只执行精确测试文件的 `Read`，技能标记返回；图片回合原生接收的 PNG 字节摘要与附件一致，模型正确回答左红右蓝。应用重启后原宿主 owner epoch 从 1 变为 2，没有重投；显式断开后，新宿主以相同 native session 冷恢复并正确回忆上下文。合计 3 次新输入、3 个成功回合、2 个宿主，均取得退出与完整事件日志确认。原生 CLI 使用已授权默认 Keychain，未提取凭据。V5 附件作用域拒绝和 V7 私有 HOME 未登录的失败仍保留。[GUI 主收据](validation/macos-fixed-versions-20260924/gui-claude-composer-recovery-v7.safe.json)、[图片结果](validation/macos-fixed-versions-20260924/claude-image-after-restart-result-v7.png)、[冷恢复结果](validation/macos-fixed-versions-20260924/claude-cold-resume-result-v7.png)。

实际简体中文输入法已验证物理拼音按键、marked text、数字选词、空格选词、中文／ASCII 混输和多行保留。该输入法用例未提交模型，系统候选浮窗没有单独截图；它与普通粘贴测试分别记录。[IME 收据](validation/macos-fixed-versions-20260924/ime.safe.json)、[混输截图](validation/macos-fixed-versions-20260924/ime-mixed-multiline.png)。

Grok V8 默认 GUI 的固定读取策略实链已通过：精确测试文件的一次 `AllowOnce` 审批后完成首轮；应用重启后原宿主 owner epoch 从 1 变为 2，输入数保持不变；第二轮正确回忆上下文；断开并确认原进程退出后，新宿主以相同 native session 恢复，零输入等待后第三轮正确回忆标记。共 3 次输入、3 个成功回合、2 份完整宿主退出收据，私有认证副本已删除、原认证元数据未变。[GUI 收据](validation/macos-fixed-versions-20260924/gui-grok-fixed-read-recovery-v8.safe.json)、[只读审批](validation/macos-fixed-versions-20260924/grok-fixed-read-approval-v8.png)、[冷恢复结果](validation/macos-fixed-versions-20260924/grok-cold-resume-result-v8.png)。

默认构建的入口、Grok 继承／固定权限按钮和技能选择状态已检查中英文布局，未见标签截断。该布局证据不替代固定权限真实执行。升级开关与所选通道已在实际 GUI 重启后恢复；该用例没有安装 CLI，也不证明通道切换升级事务。[双语策略收据](validation/macos-fixed-versions-20260924/gui-default-bilingual-policy-v4.safe.json)、[默认英文入口](validation/macos-fixed-versions-20260924/default-cli-entry-en-v4.png)、[偏好恢复收据](validation/macos-fixed-versions-20260924/gui-update-preferences-restart-v5.safe.json)、[恢复截图](validation/macos-fixed-versions-20260924/update-preferences-restored-top-v5.png)。

V10 修复了同一宿主完成多轮后重启的代次核验：必须能从初始 manifest 对应到连续、配置不变的历史，不能只放宽代次大小。真实 GUI 以固定只读 Grok 完成两轮后正常退出应用，重启后 owner epoch 为 `[1,2]`、输入仍为 2；第三轮正确返回 `CEDAR83 RESTART_OK`，显式断开取得完整退出确认。总 3 次输入、0 工具，认证副本已删且源 stat 未变。[多轮重启收据](validation/macos-fixed-versions-20260924/gui-grok-two-turns-restart-v10.safe.json)。另一个继承模式任务首轮已完成，但随后 `skills_reload` 目录元数据导致连接中断；该失败独立保留。V11 已修复可独立缺省的目录时钟字段，真实继承模式首轮完成后，原生 `skills_reload` 从 32 项增至 33 项；输入数仍为 1、宿主事件日志未新增回合，连接保持正常，显式断开得到完整退出确认。[V11 回归收据](validation/macos-fixed-versions-20260924/gui-grok-idle-catalog-v11.safe.json)。

## 普通终端、SSH 与 tmux 原生通知

以下正例都由真实 CLI 触发 hook，并与产品接收的原生会话／回合配对。SSH 使用本机隔离 sshd 的公钥登录和真实 PTY，属于 Mac 到同机 SSH，不是其他操作系统远端验收。表内 `1/2/2` 分别为 SessionStart／PromptSubmit／Stop 数量。

| 固定版本 | 本地普通 PTY | SSH 直连 | tmux 透传开启、断连再接回 | 透传关闭 |
| --- | --- | --- | --- | --- |
| Codex `0.156.1` | V2 中文多行两轮，`1/2/2`；hook 绑定后粘贴成功 | 两轮成功；首个旧模型 400 另保留，因此总计 `1/3/2` | V2 同会话、同 pane 两轮，`1/2/2`，仅一次 SessionStart | 原零输入未触发 hook；补 1 次模型输入后产品接收 0，但原生发送未独立观察，负例证据仍不完整 |
| Claude `2.1.280` | V2 中文多行两轮，`1/2/2` | 两轮，`1/2/2` | V2 同会话、同 pane 两轮，`1/2/2` | 零模型输入；内层实际 SessionStart 1，产品接收 0 |
| Grok `1.0.41` | V4 两轮，`1/2/2` | V4 两轮，`1/2/2` | V5 同会话、同 pane 两轮，`1/2/2`，原生与产品事件相符 | V5 零模型输入；内层 SessionStart 1，产品接收 0 |

Codex 首轮 tmux 的 Return 曾表现为换行，Ctrl+J 后只产生一次 PromptSubmit；重连后 Return 正常。原因未独立确认，不宣称修复了该疑点。Grok 这组模型输入为 ASCII，不计中文输入通过；原 V4 tmux 缺少 socket 许可、随后代理预算耗尽的失败仍保留，最终 tmux 通过来自 V5。Grok V4 本地／直连与 V5 tmux 的公共通知域摘要已核对，helper 本身分别绑定各自快照，不宣称同一最终提交。

安全回执：[Codex 直连](validation/macos-fixed-versions-20260924/codex-ssh-direct-hooks.safe.json)、[本地](validation/macos-fixed-versions-20260924/codex-local-hooks-v2.safe.json)、[tmux](validation/macos-fixed-versions-20260924/codex-tmux-complete-v2.safe.json)、[关闭透传补测](validation/macos-fixed-versions-20260924/codex-tmux-off-one-input-v2.safe.json)；[Claude 直连](validation/macos-fixed-versions-20260924/claude-ssh-direct-hooks.safe.json)、[本地](validation/macos-fixed-versions-20260924/claude-local-hooks-v2.safe.json)、[tmux](validation/macos-fixed-versions-20260924/claude-tmux-complete-v2.safe.json)；[Grok 分批范围索引](validation/macos-fixed-versions-20260924/grok-notification-scope-index.safe.json)、[V5 生产安装绑定](validation/macos-fixed-versions-20260924/grok-v5-production-helper.safe.json)、[禁用来源与零输入负例](validation/macos-fixed-versions-20260924/grok-hook-disable-layers-final.safe.json)。[Grok 本地截图](validation/macos-fixed-versions-20260924/grok-local-two-turns-v4.png)、[V5 重接截图](validation/macos-fixed-versions-20260924/grok-tmux-reattach-two-turns-v5.png)。

测试 sshd、隧道、私有 SSH 密钥、认证副本和已识别的原生进程均已清理，原认证源 stat 未变，日志保留。普通 PTY 清理不冒充托管监督器的完整进程树退出收据。[前批清理](validation/macos-fixed-versions-20260924/native-ssh-cleanup-before-v4.safe.json)、[Grok V5 清理](validation/macos-fixed-versions-20260924/grok-v5-ssh-cleanup.safe.json)。

V8 普通 PTY 生命周期补验共 16 次输入（Codex 6、Claude 5、Grok 5），三款允许／拒绝、后续继续及应用重启后同原生会话恢复均有证据，重启不会自动提交 prompt。Claude／Grok 取消使测试进程退出；Codex 原生 Escape 仅中断对话，测试 shell 继续至自然结束，不能计作工具取消清理通过。普通 PTY 按原计划保留原生 TUI、显示未知状态并提供显式关闭终端；这不等于托管进程树退出证明。[普通 PTY 归档索引](validation/macos-fixed-versions-20260924/ordinary-pty/archive-index.safe.json) 也明确纠正汇总收据中重读已替换 GUI 路径造成的摘要时点错误，实际三次启动均绑定 V8。

## Mac 原生 CLI 原子升级

V4 正式门禁下，Codex `0.155.1 → 0.156.1`、Claude `2.1.278 → 2.1.280`、Grok `1.0.40 → 1.0.41` 三项均通过，无目标候选开关、无凭据、无模型输入。每项实际产品执行 1 次、检查 2 次，13 个配置文件的内容与权限保持，旧二进制及目标引用未变，升级后版本正确，事务日志移除、worker 进程组停止。[正式升级收据](validation/macos-fixed-versions-20260924/formal-updates-v4.safe.json)。

范围限定为已识别的官方原生安装，不包含 npm／Homebrew、不包含其他平台或实时最新版。执行器、监督器按升级相关源码绑定，不宣称最终整个仓库同提交。忙碌保护已有产品管理器与启动预约回归；该正式升级样本没有在运行中的真实模型会话期间发起升级。历史候选矩阵的 15 个通过和 7 个失败保留，与本轮正式 3/3 分开，不能合并成同一批成功。

V10 将官方 CLI 更新与托管功能版本门禁解耦：未知托管版本仍关闭其托管能力，但不阻止合法官方渠道更新。真实 Claude `2.1.280 → 2.1.267`、Latest → Stable 已走一次 execute／两次 inspect，目标官方摘要与严格签名核对通过，渠道持久化，仅允许的渠道字段变化，其他配置保持；未准备通知插件、无认证或模型输入，原生退出与 journal 清理通过。[降级收据](validation/macos-fixed-versions-20260924/claude-stable-downgrade-v10.safe.json)。未确认退出宿主及恢复扫描的 Busy 保护也已补齐回归。

## 验证状态与交付边界

- V7 默认构建的 `cargo check -p warp`、libtest、i18n 11/11、监督器构建均通过，构建期间源码摘要未变；实际 GUI bundle 另有签名后摘要。[V7 构建](validation/macos-fixed-versions-20260924/goal-v7-build.safe.json)、[GUI 构建](validation/macos-fixed-versions-20260924/build-default-goal-v7.safe.json)。
- V7 集中定向测试 **1625 通过、0 失败、54 忽略**，私有临时目录原子升级测试另 15 通过。V5 的 `current_handshake_allows_only_the_exact_empty_mcp_refresh` 失败已修复；原 1621／1／54 收据仍保留，不能追改为通过。V4 的 1612／0／54 等旧批次分别保留，不累加重叠测试。[V7](validation/macos-fixed-versions-20260924/goal-v7-focused.safe.json)、[V4](validation/macos-fixed-versions-20260924/goal-v4-focused.safe.json)、[V5 原失败](validation/macos-fixed-versions-20260924/goal-v5-focused.safe.json)。
- 早期恢复范围 272/272、组合定向 541/541、Python 138/138 和插件事务 5/5 仍是各自范围的历史通过，不是最终快照的重跑。[早期验证](validation/macos-fixed-versions-20260924/local-gates.safe.json)。
- 源码清单分别保留 [V2](validation/macos-fixed-versions-20260924/goal-v2-sources.safe.json)、[V4](validation/macos-fixed-versions-20260924/goal-v4-sources.safe.json)、[V5](validation/macos-fixed-versions-20260924/goal-v5-sources.safe.json)。V7 构建收据绑定的源码清单摘要为 `d064b7d096ea3cab9d560af44b448e24f91302b139d7dda03735c57213e9fb47`；V5 清单摘要 `abc9be30755cd23e21e99f6befd3e7fdf295545a7933ffcfb4f186db3985a655` 和早期 [39 文件清单](validation/macos-fixed-versions-20260924/source.safe.json) 仅作各自批次的历史来源。
- 用户可见权限与升级文案已同步英文／简体中文，实际双语布局已检查。宿主恢复、可信 hook 输入绑定、未知版本提示条件及通知桥接没有新增用户可见文案，无需本地化变更。

本次归档只包含必要安全 JSON 和已检查截图，原始认证、完整转录及私有大日志不进入仓库。[归档来源与 SHA 索引](validation/macos-fixed-versions-20260924/delivery-continuation-index.safe.json) 明确区分原件复制与白名单摘要；[V7 续记索引](validation/macos-fixed-versions-20260924/mac-v7-continuation-index.safe.json) 归档 11 个构建、GUI 与失败来源文件。

V10 默认 `cargo check`、libtest、i18n 11 项与监督器构建通过；集中定向 **1635 通过、0 失败、54 忽略**，Mac 原子测试另 15 通过，构建期间源码未变。[V10 构建索引](validation/macos-fixed-versions-20260924/mac-v10-build-index.safe.json)。这些通过不覆盖随后合入的平台与通知插件修改；最终成组验证待完成。

V11 冻结源码已通过默认 `cargo check -p warp`、libtest、i18n 11 项及 supervisor 构建；定向 **1645 通过、0 失败、54 忽略**，Mac 原子另 **15 通过**。包含最终 Linux／Windows 固定版本门禁、Windows 原子事务、通知桥接 0.1.4、独立新版 Codex Windows 探针与系统剪贴板 GUI 用例。构建期间源码摘要未变；Linux／Windows 专属路径仍须实机验证。[V11 构建及 GUI 索引](validation/macos-fixed-versions-20260924/mac-v11-local-index.safe.json)。

V11 更新说明及固定版本平台说明已同步英文和简体中文，并在同一签名 GUI 构建中实际检查，两种语言均无截断或重叠；中文通过正常退出后重启载入，该布局检查零模型输入。[双语收据](validation/macos-fixed-versions-20260924/gui-bilingual-final-v11.safe.json)。通知、协议、恢复及原子更新内部逻辑无需额外本地化变更。

三款托管 GUI、普通 PTY 恢复、Grok V8 父子冷恢复、Codex V9 双向 ACK、V10 多轮重启及真实渠道降级均已有各自源码与版本证据。V11 已关闭 Grok 继承模式回合后目录刷新缺口，通知 0.1.4 的首轮真实审批发现发送器白名单缺少 `permission_request`，原审批完成但通知未到产品，失败与安装迁移 6 步通过分别[归档](validation/macos-fixed-versions-20260924/ordinary-pty/grok-permission-v11/index.safe.json)；已补最小修复并开始 V12 定向验证。Windows 产品原子更新已接入但真实平台事务尚未执行。V11 完整桌面测试为 10858 通过、3 前置超时、109 跳过；3 项均在大型夹具双 SHA 阶段超时，尚未进入产品宿主。保持同 workspace 二进制和原 60 秒预算串行重测后 3/3 通过，原失败保留，[复验](validation/macos-fixed-versions-20260924/workspace-host-serial-v11.safe.json)不冒称一次全绿。集中多平台验证待派发。tmux 关闭透传按公共解析器风险范围复用 Claude／Grok 的原生发送正例，Codex 原生发送未独立观察这一事实仍保留。完整 Goal 保持进行中。

## Grok 会话级审批提醒

V12 修复通知 worker 的事件白名单后，真实 `permission_request` 已进入产品，但 Grok `1.0.41` 原生 `Notification(permission_prompt)` 同时缺少 `promptId` 和 `prompt_id`，无法证明当前回合处于阻塞。原生 `UserPromptSubmit` 有合法回合标识，二者的原生会话标识一致。[V12 失败与清理索引](validation/macos-fixed-versions-20260924/ordinary-pty/grok-permission-v12/index.safe.json)保留唯一输入、单次允许、文件效果和退出成功，以及产品提醒未出现的失败。

后续适配将这种事件单独作为会话级“需要操作”提醒：要求当前已知、非空且相同的原生会话，已收到原生输入且未等待下一轮，并继续按事件标识去重；不补造回合标识、不更新任务结果或上下文、不解除取消等待。提醒复用现有中英文通知文案和原生审批入口，工具完成或后续输入后清除。V13 的真实界面验证已通过，不能将 V12 失败改记为成功。无需新增本地化消息键。

V13 的 `cargo check -p warp`、默认 libtest 与监督器构建、i18n 11 项和受影响回归 **378 通过、0 失败、6 忽略**均已完成；构建期间 125 个源文件摘要不变。签名 GUI 摘要为 `32e8e8477eaa58c5e3db9377c8b9078e504737bab07e683affa895fd2eb0786f`。没有重新跑与这 9 个通知域文件无关的完整工作区；沿用 V11 完整结果和三项同二进制串行复验。[V13 本地门禁索引](validation/macos-fixed-versions-20260924/mac-v13-local-index.safe.json)。

V13 实际普通 PTY 一次提示、一次 `Yes, proceed` 后，产品通知中心的 `Waiting for input` 被 `Status unknown` 替换；精确文件字节和最终标记正确，原生 `/quit` 退出 0，GUI、已识别原生进程、隧道及认证副本均已清理，源认证 stat 未变。原生审批通知仍无 prompt ID，不宣称回合 Blocked 或 Success；系统通知未启用，本轮只证明产品通知中心。[V13 审批完整索引](validation/macos-fixed-versions-20260924/ordinary-pty/grok-permission-v13/index.safe.json)、[等待审批](validation/macos-fixed-versions-20260924/ordinary-pty/grok-permission-v13/attention-approval-v13.png)、[审批后提醒替换](validation/macos-fixed-versions-20260924/ordinary-pty/grok-permission-v13/attention-replaced-unknown-v13.png)。Mac 必要实链现已收敛，后续只剩统一跨平台验收与最终报告。
