# Source39 本地门禁与原生验收

本轮以 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607` 为父提交，冻结 188 个文件，其中 109 个与父提交不同。当前是未提交候选，不能计最终同提交三平台通过。冻结清单 SHA256 为 `3eb14394fc030d484e2066f3e2bfe8d9ac70e0ffdb512f1cffd08ebcb740e188`；用户已有 web_runtime.rs、websearch_tests.rs 与 gui-7e065085 目录未纳入。

本轮实现 Grok 托管 Inherit 模式单个所选技能的精确原生路径绑定；技能可见性、当前路径和内容在提交与实际发送前复核，固定权限策略不开放技能。新增固定文件策略六输入夹具、默认生产入口技能与中文文件／评审文本组合夹具；补充目录和空会话 info 的安全诊断，保留原有拒绝条件。运行器清理异常不再跳过私有认证删除，Windows 离线清理回归与两平台 CI 入口同步。

## 已完成的本地门禁

| 门禁 | 实际结果 |
| --- | --- |
| `cargo check -p warp` | 退出 0，63.195 秒 |
| `cargo test -p warp --lib i18n::tests` | 11 通过，168.288 秒 |
| 受影响模块 nextest | 1746 通过，5336 未选／忽略，24.167 秒；不是全工作区 |
| 六组 Python 离线回归 | 159 通过，无跳过：技能 22、只读策略 20、父子 19、文件策略 14、所选技能 10、预检 74 |
| main 构建、严格签名与完整双语资源 | 退出 0，205.182 秒；严格签名 0；完整 en／zh-CN FTL 已嵌入；testlib 未变 |

证据：[冻结输入](validation/macos-official-39-inputs.json)、[Cargo 门禁](validation/macos-official-39-gates.json)、[Python 回归](validation/macos-official-39-offline.json)。新增／调整用户文案已同步英文与简体中文；本轮实际双语布局待验，不能由 i18n 通过推断。

## 真实接口与 UI

四项本轮原生验收均已单次执行，绑定同一冻结源码、testlib 与 main；失败保留，没有重投。

| 验收 | 实际结果及边界 |
| --- | --- |
| 默认 leader 所选技能组合 | **通过**：1 输入、1 原生 ACK、1 次精确文件读取审批；发送前原生目录提供唯一匹配路径，技能／中文文件／评审组合经生产输入构造与默认 agent profile 传递；三个秘密对照结果与完整历史匹配，进程及认证清理通过。不是 GUI 验收。 |
| Files1 | **失败，2 个实际输入**：write_allow 通过；同原生 ID 冷继续后 write_deny 确为 PermissionRejected／Cancelled，文件未创建，但夹具错误要求取消输出为空。独立只读复核确认 39 字节仅来自本回合审批前文本，没有工具后的文本／执行结果。修复候选已合根工作区，尚未编译，不把原运行追改成通过。剩余四输入未发送。 |
| Child4 | **失败，1 个父输入、0 子任务**：父输入被原生确认后目录变化触发权限拒绝。新诊断确认原有 read_file/search_tool/use_tool 均在，共 6 项、另外 3 个字符串，非重复或畸形；尚在核实这三项是否来自当前已注册 MCP 工具，未放宽拒绝。 |
| policy11 | **失败，0 模型输入**：New／info 成功且会话和 cwd 完全匹配，但实际 turns=1、turnIndex=0 与夹具 turns=0 假定不同。后续 state／MCP／debug／冷恢复未执行，不能推断这些接口已支持。私有工作目录和认证副本均删除，连接已关闭。 |

证据分别为 validation 下 macos-official-39 各命名的事件、metadata、network 与 invocation；[Files1 独立安全诊断](validation/macos-official-39-files1-diagnostic.safe.json)、[Child4 安全诊断](validation/macos-official-39-child4-diagnostic.safe.json)保留原失败。Child4 仅复用 source38 只读前置成功，不将旧证据算作本轮父子通过。

[普通终端 GUI 独立报告](OFFICIAL_39_GROK_GUI_VERIFICATION.md)记录 13 次发送尝试、12 个原生回合及 23 张截图。身份／插件、中文多行文件、真实技能菜单、允许／拒绝、取消、继续、应用重启与同 ID 恢复、真实评审入口及结果回收已分别核对；但审批等待时追加富输入的正文未成为模型消息，尾回车却批准了原生当前选中的单次写入。该原始失败不改写，整体未通过，后续发送守卫与明确降级正在实现。全局七项配置不变，进程与私有认证副本已清理。

默认技能验收保留生产 agent profile，但包装器会对私有合成项目增加显式 trust；`test_agent_profile_override=false` 不表示原生 argv 未变，也不表示产品自动信任用户项目。固定策略约束可用工具，不构成操作系统文件系统沙箱。

source38 失败与独立复核仍见 [原报告](OFFICIAL_38_LOCAL_AND_NATIVE_VERIFICATION.md)。所有原始认证、原生历史正文和未脱敏输出均留在私有路径，不纳入交付。

## 尚未满足

最终实际修改提交的 macOS／Linux／Windows 及完整工作区门禁、三方全部普通终端与托管任务验收、SSH／tmux 完整链、故障组合和最终双语布局仍未完成。并行审计另发现 Windows Codex／Claude 正式通知插件安装仍被 Unix 门控关闭；Codex 已有原生 ConPTY 候选证据，生产接线、Windows 运行器和独立 CI 步骤已合根工作区候选但未重新编译，不能把候选传输通过当成安装器已支持。Goal 保持进行中。
