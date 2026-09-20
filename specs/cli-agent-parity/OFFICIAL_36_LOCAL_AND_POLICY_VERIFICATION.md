# source36 本地门禁与权限接口调查

候选基于 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`，冻结 161 项输入、其中 73 项相对基线修改。输入摘要 `d2b492b032fbda8374ec6eccb111fccc4ba27969044e15d07616df097923343c`。验证树与用户工作区分离；两项用户 Web 工具改动未纳入。不是同提交最终验收。

本批包含 Grok 固定创建策略与父策略检查、认证副本清理及路径别名修复、可选诊断接口降级、Codex Windows 缓存恢复观察预算、非 Git 当前目录技能发现，以及插件重新加载的双语提示。固定创建策略只确认启动输入，不能声称文件系统沙箱或上游权限实现已获证明。

| 本地门禁 | 结果 | 用时 |
| --- | --- | --- |
| `cargo check -p warp` | 退出 0 | 98.209 秒 |
| `cargo test -p warp --lib i18n::tests` | 11 项通过 | 251.342 秒 |
| 受影响 warp nextest | 1665 项通过、5353 项跳过 | 30.993 秒 |
| 主程序构建 | 退出 0 | 251.406 秒 |
| 严格签名、完整英文及简体中文资源嵌入 | 全部通过 | — |

四项非 Git 技能发现回归均包含在上述 1665 项中。Python 权限预检 63 项通过；Codex 缓存探针 18 项通过、5 项 Windows 条件测试在 macOS 跳过。后续新增真实技能、固定策略、父子夹具不属于本快照，不能沿用这些结果。

首次主程序构建随会话中断停止，没有终态结果；确认进程不存在后保留中断日志，再对同一份冻结输入构建得到表中结果。构建后测试二进制字节未改变。[输入](validation/macos-official-36-inputs.json)、[门禁](validation/macos-official-36-gates.json)、[构建](validation/macos-official-36-bundle.json)均已归档。

## policy9：仍失败，未发送模型输入

真实 Grok 1.0.30 初始化、cached-token 认证、New 成功。`x.ai/session/info`、`x.ai/session/state`、`x.ai/mcp/list`、`x.ai/debug/agent` 均返回 `-32601`，四个可选诊断错误已正确记录并继续，不再在第一个缺失接口处终止。后续通知排空及原生清理契约仍失败，Rust 退出 101、运行器退出 1；未知目录继续记为 unavailable，不伪造空目录。

运行器确认临时认证、网络转发和私有工作目录已清理；这不替代未通过的原生监督退出判据。权限加载、有效模式、工具目录、父权限上限均保持 unknown，不能计固定策略实测通过。[调用摘要](validation/macos-official-36-policy9-invocation.json)和[安全诊断](validation/macos-official-36-policy9-events.metadata.json)保留失败。

## 未完成的验收

[实际 GUI](validation/gui-official-36/verification.safe.json)已验证非 Git 项目的英文与简体中文技能菜单、隐藏不可调用项、通过应用选中后原生返回正文标记，以及打开菜单时动态新增/删除技能。固定策略两种语言按钮均正常显示并使子任务开关可选，但下方误用继承权限说明，语义修复仍待下一候选；本次不将布局通过冒充文案语义正确。仅发送一个正式模型输入，验收应用/原生进程均退出、认证副本删除。生产固定策略在线链、父子双向消息与回收仍待完成。普通 Grok 通知在 source35 发现无控制终端导致 `ENXIO`，修复不在此快照。同提交三平台、完整工作区及 SSH/tmux 整链尚未完成，Goal 保持进行中。
