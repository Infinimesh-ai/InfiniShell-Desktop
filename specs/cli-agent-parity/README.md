# 三款 CLI 支持与能力对齐

本目录只维护当前规格、能力边界、剩余工作和验证结论。**当前为阶段交付，原 Goal 尚未完成。** 目录精简、历史测试通过或代码合并都不关闭未满足的验收。

## 当前入口

| 目的 | 文档 |
| --- | --- |
| 目标、P0–P5、实施与验收要求 | [PLAN.md](PLAN.md) |
| 完成状态与结论更正 | [CURRENT_STATUS.json](CURRENT_STATUS.json) |
| 剩余功能、原生差异与补验条件 | [KNOWN_GAPS.md](KNOWN_GAPS.md) |
| 三方能力与消费者可用范围 | [CAPABILITY_MATRIX.md](CAPABILITY_MATRIX.md)、[RELEASE_SUPPORT.md](RELEASE_SUPPORT.md) |
| 自动升级、渠道与事务约定 | [CLI_AUTOUPDATE.md](CLI_AUTOUPDATE.md) |
| 已通过、失败、未验及源码／平台对应 | [VALIDATION_REPORT.md](VALIDATION_REPORT.md) |

`CODEX_PLUGIN_CACHE_REFRESH.md`、`PLUGIN_COMPATIBILITY.md`、`SSH_TMUX_VERIFICATION.md` 是配套插件说明正在引用的简短兼容入口，正文已收敛到上述文档。

## 维护约定

2026-09-25 按用户要求，目录维护方式调整为保留规格与结论，不再提交完整运行收据、截图、转储或逐轮过程专报。该决定替代此前“所有历史材料继续留在当前目录”的约定。

- 同类进展更新现有入口，避免新建重复的“最终总结”。独立功能按仓库 [Spec 约定](../../CONTRIBUTING.md#opening-a-spec-pr)使用简短设计和关联 PR。
- 验证结论记录日期、源码提交、CLI 版本、平台、模式、检查范围、通过／失败／跳过及待补项，必要时引用 CI 运行或持久产物摘要。只保留结论不等于放宽真实验证要求。
- 新的原始日志、截图和探针输出保存在仓库外；`validation/` 已忽略，仅兼容仍向该位置输出的本地旧脚本。无需强制加入 Git。
- [fixtures/](fixtures/) 保留固定路径和字节。它包含代码编译、协议回归、插件兼容迁移及版本准备依赖，不属于可以按过程文件清理的验收产物。本次不迁移或删除夹具。
- 本次仅整理文档与历史材料，不改变产品能力，不重跑源码未变的在线验收，**无需本地化变更**。

## 历史追溯

精简前完整目录固定于提交 `e3ef39689cd8b686dfe040b90217ed1067b4d268`，包括当时的通过、失败、截图、原始收据和摘要索引。历史内容从当前树移除，Git 历史未改写；需要时可通过[固定提交目录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity)或以下方式读取单个文件：

```sh
git show e3ef39689cd8b686dfe040b90217ed1067b4d268:specs/cli-agent-parity/<原相对路径>
```

整理前已在维护机器的仓库外保留完整压缩备份及逐文件 SHA-256 清单；备份位于 `~/Documents/InfiniShell-Archives/cli-agent-parity/`，以完整提交号命名。跨机器追溯以固定 Git 提交为准，不依赖该本机目录。

历史整体完成判断已由 `CURRENT_STATUS.json` 更正，原始证据链接仅用于追溯，不能取代当前缺项状态。后续更新结论需绑定实际实现和验证；不能将新工作区的未提交改动自动记为通过。
