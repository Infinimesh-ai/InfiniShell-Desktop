# macOS 隔离 GUI 实测记录

本记录来自真实 InfiniShell GUI、Codex CLI 0.147.0 和生产托管任务协调器。没有用模拟事件或直接写数据库代替界面操作。测试使用独立应用副本、三段 bundle ID、独立 `WARP_DATA_PROFILE` 与 SQLite scope；没有退出用户普通应用。CLI 配置目录隔离，只按授权复制 Codex `auth.json` 并保持 `0600`，未复制用户配置、MCP 或历史。

当前证据来自中间构建 bundle3 / bundle4 / bundle5 / bundle6，不替代最终包含全部改动的同一提交验收。bundle5 已复验新目录技能修复，并完成活动任务的正常应用退出与恢复。

## 已观察到的结果

| 场景 | 实际结果与边界 |
| --- | --- |
| GUI 启动 | bundle2 的 PersistenceWriter 单例初始化 panic 有单独记录；bundle3 与 bundle4 已真实进入窗口和任务面板。 |
| 三方身份 | 英文、简体中文界面显示 Codex 0.147.0、Claude 2.1.273、Grok 1.0.30 及隔离安装路径。Grok 托管不可用及 Claude 未完成认证验收说明可见。 |
| Claude 认证失败 / Grok 门禁 | Claude 在隔离无账号目录中完成原生握手和输入确认，随后真实返回 `Not logged in · Please run /login`；任务及历史均显示失败。Grok 填入非空提示词后启动按钮仍禁用，没有误启动托管任务。 |
| 两轮中英文输入 | Codex 同原生会话返回 `GUI_ONE`、`GUI_TWO`，输入中的中文、English、下划线和换行在 SQLite 中完整保留，均有原生接收确认。 |
| 审批允许 / 拒绝 | 在界面核对命令与工作目录后分别允许、拒绝专用 Python 夹具。允许文件包含 `PARITY_ALLOW`；拒绝文件始终不存在。CLI 真实完成分别返回 `GUI_ALLOW` / `GUI_DENIED`。 |
| 运行中追加 / 取消 | `/bin/sleep 60` 经过界面单次允许；同一运行的追加输入保存为带原生 turn ID 的 `Steer`，收到 `native_protocol` 确认。点击取消后记录为 `cancelled`，没有成功结果。下一次运行完成 `GUI_AFTER_CANCEL`。 |
| 应用重启 / 继续 | 真实退出隔离 bundle3 后启动 bundle4，保持同 bundle ID、profile 和数据库。只选择历史任务时没有新运行；点击继续才从第 5 次进入第 6 次，原生会话 ID 保持不变，返回 `GUI_RESUMED，GUI_ONE，GUI_TWO`。 |
| 活动任务应用重启 | bundle5 在已批准的 `/bin/sleep 60` 真实运行中退出隔离应用，应用正常退出 0；旧 CLI 与 sleep PID 随后消失。监督回执为 `stdio_closed`、退出 0、`cleanup_confirmed=true`，限定于该正常退出实例。重开显示第 1 次运行 `Disconnected`，选择任务未新增代次或执行；显式继续才进入第 2 次、保持原生 ID。另发新指令后完成 `GUI_ACTIVE_RESTART_RESUMED`，原生历史只有一次 sleep 调用。此结果不覆盖异常退出或另组残留场景。 |
| 新项目原生技能 | bundle5 在从未打开终端的新目录点击 Refresh 后发现 `fresh-project-check`。选择技能和中英文字后，数据库保存 typed `Skill`、原生历史出现完整 `<skill>` 注入并完成技能规定的 `FRESH_PROJECT_SKILL`。启动目录仍为输入的 `/var/...`，技能路径使用规范化后的 `/private/var/...`。原生 ACK 后清空草稿和技能 Chip。 |
| 父子任务派发与回收 | 父任务显式启用只读权限和本地工具，真实 `run_agents` 派发子任务。子任务发送 `GUI_CHILD_PROGRESS` 并完成 `GUI_CHILD_RESULT`；父任务通过真实 `inspect_local_tasks` 回收后完成 `GUI_PARENT_COLLECTED`。进度消息与最终结果消息均有原生接收确认。工具响应仅显示发送尝试，没有伪称原生确认。 |
| 父到运行中子任务追加 | 父任务的下一次运行创建新子任务，先查询其 `running` 状态，再单独发送 `GUI_PARENT_STEERED_CHILD`。该标记不在子任务初始 prompt 中；消息获得原生确认，子任务真实最终结果采用该标记。父第 2 次运行与子第 1 次运行的关系、代次均保留。 |
| 活跃面板关闭 / 重开 | 父子运行时关闭面板，独立只读查询仍显示原父子任务运行；重开关联相同记录，没有重复派发。 |
| 历史结果查看 / 完整复制 | 当前运行推进到第 2 次时仍可查看第 1 次完整结果。实际点击“复制完整历史结果”并粘回空草稿，内容、子任务 ID、中文与换行一致；校验后清除，未再次发送。 |
| 图片和文件上下文 | 简体中文界面通过真实原生文件选择器选择 JPEG 和 README，图片 Chip 与完整文件路径同时保留。提交有原生确认；CLI 实际识别沙滩棕榈树，并通过原生工具读取 README。已保存原生输入及工具输出，省略图片 base64。 |
| 附件失败保留 | 测试准备的 `.png` 文件实际为 JPEG；声明格式不符被严格拒绝，数据库仍为 0 个任务，文本和附件均保留。替换为格式匹配的 `.jpg` 后正常完成。未放宽验证器。 |
| 真实评审导入 | 在实际 `calculator.py` 差异第 2 行创建中英评论，面板导入已有评审 builder 生成的路径、行号和完整评论，源侧仍保留 1 条评论。CLI 确认接收并正确返回文件、行号和问题；测试文件未被改写。 |
| Claude 静态图片提示 | bundle6 在原中文隔离 profile 中复验新建空草稿及已原生确认后的空草稿，两者仅显示“尚未验证 Claude Code 的托管图片输入。”，不再误称草稿已保留。1229×768 中提示、消息回执和输入控件未见截断；数据库任务、运行和消息计数前后不变，未发起新模型请求。实际转换拒绝仍引用独立错误键 `cli-agent-input-images-unverified`，保留“草稿已保留”；本轮未重新触发拒绝流程。 |
| 普通 PTY Codex | 完全独立 HOME、CODEX_HOME 和 SQLite 的第三个应用实例中，Codex 0.147.0 缺插件时显示启用通知入口。富输入两轮中文、English、多行及下划线原样到达真实 CLI，返回 `PTY_ONE`、`PTY_TWO`；富输入 `/exit` 成功退出原生会话。 |
| 普通 PTY Claude / Grok | 各自品牌和通知工具栏正确出现；Claude 2.1.273 停在首次登录方式选择，Grok 1.0.30 停在浏览器设备认证等待。没有越过未认证边界提交模型输入；Grok 设备码在文本证据中脱敏。 |
| 普通 PTY 插件安装 | GUI 安装器在隔离 CODEX_HOME 写入插件，受控树 10 文件与 revision 3 全部匹配，未写 `trusted_hash`。随后工具栏正确显示原生授权说明。但真实重启 Codex 后四个修补文件被恢复为上游版本，尚未通过可靠安装验收；没有继续信任已变化的 hooks。 |
| 中文输入法组合 | 使用现有拼音输入源逐键输入 `n/i/h/a/o`，观察带下划线的 `ni hao`，按空格提交为“你好”。未切换用户输入源。该项与 Unicode 粘贴分别记录；候选列表选择尚未验证。 |
| 双语布局 | 1229×768 窗口中检查英文和简体中文的初始面板、权限、输入、附件、消息回执、结果、历史和复制入口。新增控件文字可读，无水平截断；长路径、消息与结果换行，面板按既有滚动行为展开。 |

## 未计为通过或仍待复验

- bundle4 的新项目技能失败已由 bundle5 的真实正向流程复验；失败截图仍保留。最终提交和后续构建仍须遵守统一验证门禁。
- Claude 与 Grok 的已登录完整模型生命周期、权限允许/拒绝、取消和恢复未在本 GUI 流程完成，不能由 Codex 的结果替代。
- bundle6 再次尝试在空草稿逐键输入 `s/h/i` 并按 Down：应用截图可见带下划线的 `shi`，未能看到候选列表；CUA 读取系统拼音候选窗返回 SCIM_Extension.appex 无法打开。未选择不可见候选、未改输入源，取消后重开面板确证空草稿，任务/运行/消息计数不变。因此候选列表选择仍未验证，不能以组合输入替代。真实语音采集、普通 PTY Claude / Grok 的认证后完整富输入亦未验证。
- Codex 原生重启后插件修补回退已保存前后哈希与真实界面。五项 hooks 授权和收到可信富通知的闭环因此暂停，不能计为通过。
- 此处为 macOS 本地托管任务，不能替代 Linux、Windows、SSH 或 tmux 验证。
- 首次审批完成后曾出现发送点击未提交、关闭重开面板后提交成功。后续以最新屏幕定位重复测试均正常，尚未稳定复现或确认产品根因，不能写成已修复问题。
- 图片任务的模型将自动加载技能中的 `PARITY_SKILL` 误称为 README 标记。README 的真实内容与原生工具读取证据另行保留；此错误语句不能当作文件事实或图片传输失败。
- 简体中文代码评审旧界面的 `Uncommitted changes`、`Discard all` 等既有英文，以及原评审 prompt 的英文模板不是新增面板文案；本记录没有将其算作全仓本地化完成。

## 证据索引

- `macos-gui-bundle6-ime-candidate-attempt.json` 与 `macos-gui-bundle6-ime-test-draft-cleared.png`：候选列表未能读取的真实尝试、空草稿及无新请求校验。

- `macos-gui-bundle6-zh-static-hint.json` 与 `macos-gui-bundle6-zh-claude-{empty-input,ack-empty-input}-hint.png`：中文静态能力提示、新旧草稿状态、执行包哈希及无新任务的计数核对。

- `macos-gui-pty-codex-two-turns.ndjson`：完全 HOME 隔离的普通 Codex PTY 两轮原生输入输出。
- `macos-gui-pty-codex-plugin-tree.json` / `macos-gui-pty-codex-patch-reverted.json`：GUI 安装后与 CLI 重启后受控文件哈希。
- `macos-gui-pty-codex-patch-reverted-after-restart.png`：原生 hook 详情与更新提示。

- `macos-gui-bundle5-skill-active-restart.json`：技能及活动重启各阶段任务、代次、消息与真实退出回执。
- `macos-gui-bundle5-native-skill.ndjson`：原生技能内容注入与最终结果。
- `macos-gui-active-restart-native.ndjson`：唯一 sleep 调用、退出时中断及显式继续后的最终结果。
- `macos-gui-bundle5-{fresh-project-skill-discovered,native-skill-draft,native-skill-result}.png`：技能真实 GUI。
- `macos-gui-active-restart-{approval,disconnected,resumed}.png`：活动重启真实 GUI。

- `macos-gui-bundle4-final-records.json`：两种语言的任务、各次运行和消息持久记录，以及夹具副作用校验。
- `macos-gui-restart-before-resume.json`：应用重启后、明确继续之前的任务代次。
- `macos-gui-steer-cancel-and-child.json`：真实 Steer、取消、父子运行中消息与结果回收。
- `macos-gui-zh-image-file-native.ndjson`：图片原生输入及文件读取，不含图片 base64。
- `macos-gui-*.png`：真实窗口截图；按文件名区分布局、审批、输入法、历史复制、图片失败、评审与父子消息。

普通 PTY 隔离副本执行二进制 SHA256：`35237d40a719fded8648c2f5f02a117373fe34a018173f02cacb7f1278a6bb56`（bundle5 源 worker 为 `d259017ab63795612e70593aeeae10f1bca62df98bb86f05f8252142968ddc56`）。
