# source38 本地门禁与原生续验

候选基于 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`，冻结 178 项输入，其中 95 项相对基线修改；清单 SHA-256 为 `f5ae6eecf351f999189006a4e78045b8df335a3e4c837a6129a581f37ca8dfa0`。这仍是未提交候选，不能作为最终同提交三平台通过证明。用户现有 web 修改不在该验证树中。

本轮包含 Grok 固定文件工具策略、原生技能目录的测试观察、ACP 扩展诊断线上名称修正、拒绝审批夹具按原生取消语义校验，以及真实 GUI 验证后的 `/plugins` 重载提示。测试专用目录观察不开放产品 selectedSkills；固定文件策略尚需真实写入／编辑审批及恢复验证。

## 本地门禁与实际 GUI

`cargo check -p warp` 通过，106.148 秒；`cargo test -p warp --lib i18n::tests` 11 项通过，241.961 秒；受影响 nextest 1725 项通过、5334 项跳过，23.920 秒。main 构建通过，274.554 秒，严格签名退出 0，完整中英文 FTL 已嵌入，测试库身份前后不变。门禁进程采用 source37 全部筛选加五项重复覆盖的显式模块选择；报告保留实际 argv，不把启动后的驱动整理冒充受测脚本摘要。

测试库 SHA-256 为 `e7fed03b414cd27c87082b12da0109e41e29c09db789757d73bcae173d46c9ca`；main／监督 worker 为 `45e1cecdebdb3a186ba75e5b33645e434462260ee657a0b1b60a73310488610f`。

Python 技能运行器 16、固定策略运行器 17、父子运行器 17、无输入接口运行器 70 项通过；已安装通知 17 项中 15 通过，首次运行因 PATH 缺少 fish/tmux 跳过 2 项。随后显式绑定真实 tmux 3.7c，目标 pane 路由一项独立通过；fish 仍未验证。此处 tmux 是本机真实通知传输测试，不是 SSH 或三方在线整链。

[实际 GUI 安全报告](validation/gui-official-38-policy/verification.safe.json)绑定六张原图：英文／简体中文分别选择继承、固定只读、固定文件策略，目标按钮无截断，说明正确换行；继承时禁用子任务入口，固定策略启用。使用无认证的隔离重签副本，零模型输入，应用已退出。三个本地任务表的 3 个任务、20 个代次和 20 条消息与复制的历史逐行相同。只证明这些控件布局；不覆盖输入法、评审组合或全部界面。Grok 通用帮助的旧只读描述需随后同步。

## 原生结果

| 运行 | 原始结果 | 已证明与限制 |
| --- | --- | --- |
| fixed3-host | PASS，2 输入 | 生产 prepare 启动，首次读取允许且完整最终结果通过；保存策略及原生 ID 后冷启动 load，未自动重发，读取拒绝得到原生 PermissionRejected／Cancelled、完整空结果、被拒工具无读取输出；两阶段正常清理及认证副本删除。不是文件系统沙箱、完整应用重启或父子链证明。 |
| skill-visible3-catalog | 驱动退出 2／运行器 1，保留 FAIL | Rust 实际退出 0。原 runner 复用租约读取器最多 2 事件，新增目录后实际 3 事件被拒。独立只读复核证明唯一原生输入、零工具／审批、最终技能正文随机标记匹配；四次目录均唯一匹配 canonical 路径，前两次在输入前。它使用测试 agent profile 和合成项目信任，不证明默认生产入口或 selectedSkills。 |
| child3-host | FAIL | 仅一个父任务、一次原生输入 ACK，思考后第三次目录更新由 session_start 的 7 命令变成 skills_reload 的 24 命令；生产 verify_catalog 以 grok_creation_catalog_changed 严格拒绝。零子任务、工具和审批，未取得原生终态；实际被拒的 tools 数组未保留，不能猜测后放宽白名单。错误中的 task_input_sent=false 与 ACK 矛盾，后续修正证据。 |
| policy10 | FAIL，0 输入 | initialize、cached_token authenticate、new、_x.ai/session/info 均成功；info 746 字节。随后 empty_native_history_unconfirmed，未发 state／mcp／debug，不能将 14 请求预算记为实际请求。原证据不足以区分 sessionId、cwd、turns 或 turnIndex 条件失败，后续仅补安全形状诊断。清理收据成立、原生退出码 null 不补为 0。 |

[fixed3 收据](validation/macos-official-38-fixed3-host-events.ndjson)、[skill 原失败](validation/macos-official-38-skill-visible3-catalog-events.metadata.json)与[独立复核](validation/macos-official-38-skill-visible3-reanalysis.safe.json)、[child3 安全诊断](validation/macos-official-38-child3-host-diagnostic.json)、[policy10 调用](validation/macos-official-38-policy10-invocation.json)均已归档。独立复核没有重新调用模型，也没有改写原始 FAIL。

## 后续边界

第 39 轮候选将合入独立三事件技能读取器、info 安全形状诊断、父子目录变化安全摘要、selectedSkills 接线与 Files 六输入夹具；它们均不回填本轮通过数。当前固定策略仅限定配置和工具集，保留系统策略，不宣称操作系统沙箱。

[输入清单](validation/macos-official-38-inputs.json)、[Rust 门禁](validation/macos-official-38-gates.json)、[主构建](validation/macos-official-38-bundle.json)、[Python 回归](validation/macos-official-38-offline.json)、[真实 tmux 补验](validation/macos-official-38-tmux-hook.json)及[公开证据索引](validation/macos-official-38-archive.json)记录各自边界。固定文件在线链、Grok 子任务、完整双语输入组合、最终同提交三平台与完整工作区、SSH／tmux 在线整链仍待完成。Goal 保持进行中。
