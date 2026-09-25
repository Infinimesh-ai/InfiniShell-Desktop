# 三款 CLI 支持与能力对齐

本目录保存 Codex CLI、Claude Code、Grok Build 在 InfiniShell 中的产品约定、实现计划、能力边界及验证证据。

**当前为阶段交付，原 Goal 完整验收尚未满足。** 当前完成结论以 [CURRENT_STATUS.json](CURRENT_STATUS.json) 为准，剩余能力与验收项以 [KNOWN_GAPS.md](KNOWN_GAPS.md) 为准。文件名中的 `FINAL`、旧报告中的成功总结，以及目录整理本身，都不代表整个 Goal 已完成或已经合并、发布。

## 从哪里开始

| 阅读目的 | 入口 |
| --- | --- |
| 判断当前完成状态和历史结论更正 | [当前状态](CURRENT_STATUS.json) |
| 查看缺项、影响、替代方式和关闭条件 | [统一缺项清单](KNOWN_GAPS.md) |
| 了解三方实际能力和消费者支持范围 | [能力矩阵](CAPABILITY_MATRIX.md)、[支持说明](RELEASE_SUPPORT.md) |
| 核对阶段合并范围与验收边界 | [阶段合并准备](MERGE_READINESS_20260925.md) |
| 继续原目标与 P0–P5 工作 | [实施计划](PLAN.md)，结合当前缺项清单阅读其中的历史记录 |
| 检查固定版本和源码对应的验证 | [固定版本验收](ACCEPTANCE_FIXED_VERSIONS_20260924.md)、[相关提交验证](FINAL_SAME_COMMIT_VERIFICATION_20260925.md) |
| 查看自动升级及渠道规则 | [CLI 自动升级](CLI_AUTOUPDATE.md) |
| 追溯早期验证、交接与决策 | [验证报告](VALIDATION_REPORT.md)、[阶段交接](HANDOFF_20260921_STAGE_CHECKPOINT.md)，再沿链接查原始专报 |

阅读证据时同时核对源码提交、CLI 版本、平台、运行模式、权限策略及测试范围。某个用例通过不扩展到未覆盖的组合。当前状态更正只撤回历史整体完成推断，不把真实通过改为失败，也不把原始失败改为成功。

## 文件分工

| 文件或目录 | 用途与维护方式 |
| --- | --- |
| 上表中的当前入口 | 继续维护，汇总当前状态、支持范围、剩余工作及对应证据；避免再创建同用途的多份“最终总结”。 |
| 专题方案与协议文档 | 按当前入口中的引用阅读，如 [本地工具](LOCAL_TOOLS.md)、[插件兼容](PLUGIN_COMPATIBILITY.md)、[协议证据](PROTOCOL_EVIDENCE.md)。它们仍可承载有效设计，不能仅凭文件较早就全部标为过时。 |
| 顶层阶段专报 | `OFFICIAL_*`、`CI_*`、`SOURCE*`、`*_PLATFORM_RUN_*` 及旧交接、诊断等记录各自当时的操作与结论；保留原路径，通过当前入口追溯，不作为独立的当前完成判据。 |
| [validation/](validation/) | 按运行／阶段保存的原始验证收据、截图和来源索引；保留失败、跳过和未覆盖项，不覆盖旧运行。 |
| [fixtures/](fixtures/) | 协议回归、版本准备及兼容迁移所用的固定输入；它们是代码和测试依赖，不能按过程文件删除。 |

例如，[Claude 回归测试](../../app/src/ai/cli_agent_runtime/claude_tests.rs)、[Grok 插件兼容实现](../../app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs)及[版本准备测试](../../script/cli-agent-parity/prepare_grok_cli_tests.py)直接引用 `fixtures/`。历史证据还包含相对路径和文件摘要，例如[验收归档索引](validation/final-same-commit-20260925/ARCHIVE_INDEX.safe.json)；搬迁和内容改写都需要处理这些依赖。

## 本次目录整理决策

2026-09-25 在提交 `75eca85f27aaeb0d130e87060b048c25d26e3437` 盘点 Git 跟踪文件：本目录有 2,387 个文件，其中顶层 168 个、`validation/` 2,117 个、`fixtures/` 102 个。其余 284 个规格目录中，268 个只有一至两个文件。这是新增本 README 前的快照，说明本专题同时承担了大规模验收存档；其他目录较小不等于它们都经过了统一归档。

仓库 [CONTRIBUTING.md](../../CONTRIBUTING.md#opening-a-spec-pr) 推荐每个规格以简短的 `product.md`、`tech.md` 描述行为和实现，并用 PR 承载讨论；没有规定完成后必须删除、压缩或搬入 `archive/`。已有专题采用原地维护状态和限制的方式，例如 [warp-control-cli](../warp-control-cli/README.md)、[递归 SSH](../recursive-ssh-extension/PRODUCT.md)和 [X11 后台操作](../x11-background-computer-use/TECH.md)。

据此，本专题采用以下处理：

1. 用本 README 作为目录入口，将当前结论与历史材料分开导航。此前的状态纠偏已经落地，本次补齐目录组织说明。
2. 已有设计、阶段报告、失败记录、截图、收据和夹具原地保留。本次不执行物理归档、删除、压缩或 Git 历史改写，也不声称文件数量或仓库体积已经减少。
3. 不为形式一致再复制一套 `PRODUCT.md`／`TECH.md`；现有计划、矩阵和支持说明继续承担这些职责。
4. 暂不批量搬迁历史报告。将来若确有阅读或存储问题，再单独评估引用迁移与证据完整性；单纯 `git mv` 不会减少 Git 历史体积。
5. 本决策只适用于本专题，不构成对全部 `specs/` 的强制归档政策。

## 后续如何保持简洁

- 每次先从 `KNOWN_GAPS.md` 选定范围，明确固定版本、平台、模式与关闭条件；沿用已有入口更新当前结果，避免向顶层持续堆放重复总结。
- 新的独立功能或版本迁移采用仓库的短规格与关联 PR 方式组织。常规讨论、试错和中间进度放在 PR／任务记录中；有长期价值的决策再收敛到对应专题文档。
- 新验证材料在 `validation/` 下按独立运行分组，只提交脱敏后、足以支撑验收的必要证据与来源摘要；不同运行不覆盖，失败与成功分别记录。测试需要的固定输入放在 `fixtures/`，已有路径保持稳定。
- 阶段结束时更新当前状态、缺项、能力矩阵、支持说明及验证入口；关闭一项需有对应证据，不能靠“已归档”或“已合并”关闭剩余验收。
- 本次仅新增文档导航，**无需本地化变更**；未改变产品行为或原始验收结果。
