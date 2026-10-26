# 上游精华巡检

本目录记录 InfiniShell 对 `warpdotdev/warp` 的选择性评审与引入决策，关注终端、TUI、Agent、稳定性、性能、安全及跨平台变化。当前发行状态统一见[发布入口](../releases/README.zh-CN.md)。

## 巡检基线

最近一期为 [2026-10-26 评审](2026-10-26.md)，已审上游断点为 `1e6c3ae465d6a925a9b97ae03877aaadd159b8f1`。下一轮从该断点之后检查，并核对当前 main 是否已有等价实现。日期报告用于解释引入决策，不作为当前发布、候选或操作待办。

[2026-10-19](2026-10-19.md) 的五组实现已在本轮起点 main 中核实；[2026-10-12](2026-10-12.md) 的续行快照与 TUI 补全隔离，以及 [2026-10-05](2026-10-05.md) 的选择性吸收也已有评审记录。不要仅因上游 patch-id 不同就重复推荐。Zap 专项评审不改变 Warp 上游扫描断点。

## 决策分类

| 状态 | 判据 |
| --- | --- |
| 已吸收 | 代码进入实际编译/运行路径，有明确本地提交及验证范围 |
| 部分吸收 | 只移植仍适用的路径，不恢复已裁剪的产品功能 |
| 本地等价 | 通过补丁或语义核对确认本地已有相同效果 |
| 继续观察 | 写明收益、冲突风险及重新评估条件 |
| 不适用 | 依赖停用的私有后端、商业化或已删除路径 |
| 不吸收 | 与项目定位冲突，或收益不足以覆盖维护成本 |

## 每轮流程

1. 读取根 `AGENTS.md`，核对工作树、main、origin/main 和上次评审断点。
2. 获取上游更新并记录共同祖先、双向提交数；优先检查新增提交。
3. 对候选同时阅读补丁与当前本地实现，用 `git cherry`、patch-id 和语义检索排除重复。
4. 记录上游 commit/PR、价值、适用路径、风险、移植方式和必要验证；不以文件仍存在判断功能已启用。
5. 巡检默认只读。需要落地时，在用户授权范围从 main 创建独立 `codex/upstream-essence-YYYY-MM-DD` 分支。
6. 先做聚焦回归，再执行 `cargo check`；涉及 UI、本地化、终端或跨平台行为时补适用验证，明确未覆盖项。
7. 更新本页评审断点；推翻旧决策时在新报告中说明原因，不把旧验证结果移用于新源码。

```sh
git fetch upstream master
git rev-parse main origin/main upstream/master
git merge-base main upstream/master
git rev-list --left-right --count main...upstream/master
git cherry -v main upstream/master
```
