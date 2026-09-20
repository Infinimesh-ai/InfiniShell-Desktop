# source37 插件修复与原生验收

基于 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607` 的独立候选，177 项输入、94 项相对基线修改，摘要 `415908767190975f76b0a81ced2ab7a7ad2cca5bde610a748c7f474128e7faf8`。不是最终提交或跨平台完成证明。

## 已完成

- `cargo check -p warp`：93.040 秒，退出 0。
- `cargo test -p warp --lib i18n::tests`：11 项通过，235.321 秒。
- 受影响 nextest：1698 项通过、5334 项跳过，23.783 秒。包含新原生夹具的编译及离线边界，不表示 ignored 原生测试已执行。
- Python：技能运行器 13、固定策略运行器 14、父子运行器 16、已安装通知测试 16 项通过；fish 缺失，1 项跳过。Node 通知转换 14 项通过。

[输入](validation/macos-official-37-inputs.json)、[Rust 门禁](validation/macos-official-37-gates.json)、[运行器回归](validation/macos-official-37-offline.json)均已归档。

## Grok 通知 0.1.2

修复 source35 的真实 `ENXIO`：Unix PTY 启动与 shell 引导记录当前终端路径；没有控制终端的 hook 在 tmux 中解析当前 pane，在其他环境使用当前 shell 路径。无可靠目标时降级，绝不向 hook stdout 写终端通知。Windows 继续使用独立控制台通道，不能将 Unix 回归算作 Windows 通过。

真实脱离控制终端测试证明 0.1.1 无输出、0.1.2 到达目标 PTY；真实 tmux 两 pane 测试证明通知只到目标 pane、两个陈旧外层 PTY 均无输出。写入失败不消耗去重标识，后续重投仍可发送。这些使用真实 PTY 和 Node，未发送模型输入；下方 GUI 实验补证应用消费，实际 SSH 及 Windows 仍须另验。

## 真实生产安装器

固定 Grok 1.0.30 与 Node 25.9.0、全新无凭据 HOME 中执行生产安装器，六阶段全部通过：安装、已知 0.1.0 来源升级、同版本损坏修复、禁用状态拒绝更新、受控文件事务失败回滚、回滚后再次更新。安装与升级后的实际通知模块版本均为 **0.1.2**。

[安全报告](validation/macos-official-37-installer.json)的旧稳定步骤名 `production_upgrade_known_010_to_011` 沿用既有字段，但 `current_plugin_version` 及实际导出版本明确为 0.1.2，不将名称误读为受测目标版本。此次原生升级起点是 0.1.0，0.1.1 实际升级另见下方 GUI 证据。故障是生产文件事务注入，未冒充进程强杀恢复。已安装模块的导出检查不等于 hook main 或 OSC 被应用消费。

## Grok 真实 GUI 升级与通知消费

独立应用从保留 source35 历史和实际 0.1.1 插件的复制件启动，点击正式更新入口，源和缓存四文件均升级为 source37 的 0.1.2。原 source35 及复制的旧插件源未变。仅在 Hooks 页按 `r` 未刷新插件注册表；在 Plugins 页按 `r` 后，实际显示 0.1.2 与九项 hooks。此发现已用于后续候选的中英文提示修正，未改动本次冻结源码。

同一原生会话完成两次正式模型输入，第二轮正确复述首轮标记；两轮后均直接观察到应用通知收件箱的 **Grok Build needs attention / Status unknown**。这证明 0.1.2 的真实 hook 通知已被应用消费，同时普通终端 Stop 没有被误判为成功。[安全报告及七张截图](validation/gui-official-37-grok-upgrade/verification.safe.json)保留输入计数、二进制／插件／历史摘要、截图对应关系。应用和 CLI 正常退出，认证副本已清理，原认证 stat 未变；没有归档历史正文或认证内容。此次不扩大为 SSH、Windows 或托管生命周期通过。

## 主程序与真实运行

主程序打包退出 0，285.305 秒；严格签名、完整双语 FTL 嵌入及测试二进制保持不变全部确认。[打包报告](validation/macos-official-37-bundle.json)记录 worker SHA-256 `83b5e2af482cdc7494a2437fbbf98c0982bd0f33bb68774dd5ac9438311c0df7`。

`fixed1` 真实运行失败，13.698 秒、Rust 101：没有 Ready、没有输入提交或模型输入。当次失败发生在生产启动准备后的 supervisor 边界，不得计固定策略或恢复通过。`skill-visible1` 真实运行失败，15.331 秒：实际提交并接收 1 条输入、出现 1 次审批，但精确技能读取与最终哨兵均未确认。两次原有失败报告保留，认证副本与隧道已清理；技能运行的正常传输关闭已经确认。

后续确认 `fixed1` 的外层测试沙箱阻止 macOS launchd 监督进程启动。`fixed2-host` 使用同一 source37 生产产物，加已审阅 source38 Python 夹具修复，不再声明 OS 沙箱或全部原生连接受代理预算约束。实际运行 22.397 秒、两次输入：首次读取允许、最终结果及原生工具结束全部通过；同原生会话冷加载的身份、profile 和无重放均通过，第二轮真实拒绝得到原生 `PermissionRejected`／`Cancelled` 和完整空结果历史。夹具误要求 Completed 而提前失败，原始 [失败公报](validation/macos-official-37-fixed2-host-events.ndjson)及 [调用记录](validation/macos-official-37-fixed2-host-invocation.json)保持不变。只修夹具并重新执行后才能计整链通过，父子任务尚未启动。

## Claude 普通 PTY GUI

固定 Claude Code **2.1.273** 已完成普通 PTY 链路，共 **13 条正式模型输入**，原生历史有 23 条用户记录，二者不混为输入计数。独立 HOME、配置目录、合成项目与应用配置中，经真实首次界面启动，并通过正式应用插件入口安装 **2.2.0**。[安全报告](validation/gui-official-37-claude-pty/verification.safe.json)记录原生会话、结果摘要及 22 张截图；截图大小与 SHA-256 已逐一复核，原始历史和配置正文未归档。

- 中英文两轮、中文多行富输入、含中文和空格的文件上下文均返回预期结果；应用技能菜单选择可见技能后真实执行，隐藏技能在英文和简体中文菜单中均未出现。
- 原生 Write 审批允许一次后，文件与预期字节一致；拒绝后文件保持原值。运行中追加指令有输出仍在流式返回时的队列截图和最终结果；较早的追加结果单独只证明执行，不代替运行中排队证据。
- 在待处理 Write 审批处按 Esc 结束本次请求，原生历史记录用户中断，文件未变。随后显式继续原生会话，保持同一会话 ID 并返回先前记忆；关闭旧应用进程、启动新应用后，显式继续前历史摘要保持不变，继续后再次返回新结果。
- 6 个明确用户全局 Claude 配置路径在 6 次启动与审计快照中均保持不变。英文和简体中文的应用工具栏、富输入与技能菜单未观察到截断；原生 Claude 内容保留其自身语言。

该结果仅计普通 PTY 链路。中文文本提交不证明真实 IME 组合输入或快速 Enter；原生审批处取消不证明托管任务的 `RuntimeEvent::Cancelled`；显式启动进程继续历史不等于自动重新关联仍活跃任务。插件安装成功也不证明 OSC 已被应用消费。代码评审上下文 GUI、Claude 插件故障注入及修复、托管父子任务恢复、SSH/tmux、其他平台和最终同提交验收均不在本轮通过范围。

## Grok 可见技能原生展开

`skill-visible2-trust` 使用 source37 生产主程序与测试二进制，叠加 **source38 仅 Python 合成项目信任夹具**，11.957 秒退出 0。实际提交并接收 **1 条输入**；秘密哨兵仅位于技能正文，没有进入提交输入，最终原生历史与回复摘要匹配哨兵。观察到 **0 次工具事件、0 次显式技能文件读取、0 次审批**，因此证明此次原生技能展开和结果回收，不能称为一次 `read_file` 审批验收。

`--trust` 仅作用于合成项目与独立 HOME，审批允许清单未变；技能、项目与配置模板快照保持不变。原生只在独立配置中初始化 `marketplace` 表，权限段保持不变，并非整个独立配置字节未变。传输关闭、隧道停止、认证副本删除和原认证文件 stat 未变均已确认。网络记录为未解密 TLS 的 7 次连接尝试与 4,155,730 字节，不能据此计数模型 HTTP 调用。

本轮没有观察到 `available_commands` 目录，原生命令与路径绑定仍未验证；隐藏技能对照也未执行。`product_selected_skills_verified=false`，不能计为应用 `selectedSkills` 入口完成；混合夹具不构成 source38 产品或最终同提交通过。原有 `skill-visible1` 失败证据继续保留。

归档前已检查公报字段、凭据形态和交叉摘要，仅保存 [事件](validation/macos-official-37-skill-visible2-trust-events.ndjson)、[元数据](validation/macos-official-37-skill-visible2-trust-events.metadata.json)、[网络摘要](validation/macos-official-37-skill-visible2-trust-events.network.json)、[目录观察](validation/macos-official-37-skill-visible2-trust-catalog.safe.json)与[调用记录](validation/macos-official-37-skill-visible2-trust-invocation.json)。不归档原始历史、提示正文、秘密哨兵或认证内容。

## 正在执行与边界

精确原生技能、固定读取策略及父子双向消息夹具已编译，后续运行按真实失败定位后推进。source38 的可选扩展通知调查修复及固定文件写入策略不在此冻结快照；Grok 只读子任务验证也不能代替可写编码子任务。最终同提交三平台、完整工作区、SSH/tmux 整链与其余验收继续推进，Goal 保持进行中。
