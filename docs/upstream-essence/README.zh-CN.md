# 上游精华巡检

本目录记录 InfiniShell 对 `warpdotdev/warp` 的周期巡检结果、引入决策和本地落地提交。目标不是追平上游，而是持续筛选适合本项目定位的终端、TUI、Agent/多代理、稳定性、性能、安全与跨平台改动。

最新记录：[2026-10-12 巡检与吸收](./2026-10-12.md)。本轮审阅新增 43 个上游提交，用户授权在独立分支实施续行快照修复和 TUI 补全预热测试隔离；本地 20 项定向测试、12 项 i18n、GUI/TUI 默认构建及 6 项原生 shell PTY 检查通过。分支已推送，[Linux/Windows 专项预检](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/38198269728) 均通过，两平台应用/TUI 回归分别为 595 / 580 项，4 项新增续行回归和修订后的补全预热用例均明确执行并通过。尚未合入 `main`。下一轮从已审 upstream `25e82210a59c8e535a14c3e22c39ec1d3361a7f7` 之后继续，并先核对这两项的分支与主线状态，避免重复移植。全库格式与 lint 的既有问题及验证边界见本期报告。

[2026-10-05](./2026-10-05.md) 的七组修复及 Codex rev7、09-22 与 09-28 的十组历史修复仍已在主线；10 月 7 日 Zap 专项不改变 Warp 上游扫描断点。历史实施、验证范围和必要的用户重新授权说明见各期报告。

## 决策状态

| 状态 | 含义 |
|---|---|
| 已吸收 | 代码已进入活跃编译路径，且有本地提交与验证记录。 |
| 部分吸收 | 只移植仍存在的本地路径；被产品裁剪删除的上游子路径不恢复。 |
| 本地等价 | 不引入上游补丁；用 `git cherry`、patch-id 和代码检索确认本地已有等价实现。 |
| 继续观察 | 当前收益不足以覆盖冲突或回归风险，并写明重新评估的触发条件。 |
| 不适用 | 依赖 InfiniShell 已停挂的 Warp 私有后端、商业化或已删除产品路径。 |
| 不吸收 | 与当前定位冲突，或只有品牌、发布流程、纯依赖升级等低价值变化。 |

“已吸收”不以文件仍留在仓库为准：只有活跃模块、实际编译路径或明确保留的跨平台资源发生有效变化才算完成。向已停挂模块套补丁不计为吸收。

## 每周期执行顺序

1. 读取根目录 `AGENTS.md`，确认本地工程约束。
2. `git fetch upstream master`，记录本地 `main`、`origin/main`、`upstream/master`、共同祖先和双向提交数。
3. 以上一次记录的 upstream SHA 为起点，优先审新增提交；同时检查本地主线变化是否让旧候选重新适用。
4. 对候选至少检查补丁和本地相关实现，并执行 `git cherry`、patch-id 或语义检索，避免重复引入。
5. 按“建议吸收 / 继续观察 / 不适用或不吸收”归档。候选必须记录上游 commit/PR、价值、路径、冲突风险、吸收方式和最小验证命令。
6. 巡检自动任务保持只读；需要落地时，从 `main` 创建独立 `codex/upstream-essence-YYYY-MM-DD` 分支，按逻辑拆分本地提交。
7. 先运行聚焦测试，再执行 `cargo check`；涉及 TUI、跨平台或真实终端行为时，追加对应实机或云端验证，并如实记录未完成项。

## 基线命令

```sh
git fetch upstream master
git rev-parse main origin/main upstream/master
git merge-base main upstream/master
git rev-list --left-right --count main...upstream/master
git cherry -v main upstream/master
```

周期报告提交后，将本文件的“最新记录”链接更新到新报告。旧报告保持不可变；若旧决策被推翻，在新周期报告中引用旧候选并说明原因。
