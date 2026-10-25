---
name: changelog-draft
description: 为 InfiniShell 指定源码或发布区间生成可审核的更新日志草稿，核对 PR、直接提交、用户可见变化与贡献者归属，只输出 Markdown 和 JSON 审计工件。
---

# InfiniShell 更新日志草稿

## 输入与边界

确认目标仓库、base/head tag 或 SHA、发布通道和输出目录。当前项目默认仓库为 `Infinimesh-ai/InfiniShell-Desktop`、通道为 `oss`；先核对当前 Git remote，用户指定的 fork 或区间优先。不得默认查询 `warpdotdev/warp`。

输出目录由用户指定，或使用适用机器配置中允许的本地工件目录；不把原始 PR 数据、私有审计材料或临时草稿写进受跟踪的公共文档目录。只生成草稿，不修改 `channel_versions.json`、生产配置或现有发布，不创建分支、推送、提 PR、发消息。

## 收集当前仓库的证据

1. 用本地 Git 核对 tag 解析结果和 `base..head` 提交区间，记录完整 SHA。用户未指定 base 时，核对同通道上一公开版本；hotfix 与首次发布不能机械套用上游周发版规则。
2. 对区间内提交查询当前仓库关联的 PR。可使用连接器，或只读 GitHub CLI，例如：

   ```sh
   git log --format='%H %s' "$base_ref..$head_ref"
   gh api "repos/$release_repo/commits/$commit_sha/pulls"
   gh pr view "$pr_number" --repo "$release_repo" --json number,url,title,body,author,labels,files,mergedAt
   ```

   `release_repo`、`base_ref`、`head_ref`、`commit_sha` 和 `pr_number` 必须来自已核实的输入或上一条结果。不同仓库的相同 PR 编号不是同一条记录。
3. 去重后保留 PR 真实 URL、作者、文件列表、正文和区间内 commit SHA。没有关联 PR 的直接提交也必须审阅，保留 commit SHA，不能漏掉或编造 PR 编号。
4. 从正文抽取实际 `CHANGELOG-*` 标记，忽略 HTML 注释内的模板示例。`CHANGELOG-NONE` 表示明确不纳入。上游 `CHANGELOG-OZ` 标记在本项目需复核能力是否存在，不能将上游云功能写成本项目已支持。

旧 `scripts/fetch_prs.py` 保留上游内部仓库映射，并会清空非上游仓库的 PR URL，因此不用于本项目的数据收集。`examples/` 中的上游历史例子仅示意格式；不得复用其仓库、版本或功能结论。

## 分类与归属

- 参照相邻 `classify-changelog-pr/SKILL.md` 判断用户可见性；以目标 SHA 的实际源码为准。当前 OSS 功能可见性需要联合检查 `crates/warp_features/src/lib.rs`、`app/src/features.rs`、通道入口以及 TUI 初始化，不能仅根据上游 Stable/Preview 列表判断。
- 按新功能、改进、修复分类；TUI 影响单独判断，共享变化可同时展示在桌面和 TUI 部分。未确认的能力进入 `needs_review`，不能为了补全草稿写成已发布功能。
- 仓库所有者确认是组织且需要区分外部贡献者时，可调用现有 `scripts/classify_contributors.py --org "$release_org" --authors ...`。权限或网络失败归为未知，不能推断为外部贡献者。
- 需要查询 issue reporter 时，使用同一已核实仓库调用 `scripts/fetch_issue_reporters.py --repo "$release_repo" --issues ...`；只有适用时才传 `--org "$release_org"`。跨仓库引用需单独核对，不能把上游编号套到本项目。
- 归属模式尊重用户选择；所有作者链接使用读取到的 GitHub 身份。私有数据不进入公开 Markdown。

## 输出

在输出目录写入：

- `changelog-draft.md`：面向用户的简洁更新说明，条目附真实 PR 或 commit 链接。使用仓库既定发布语言；若任务要求双语，两版功能范围与证据必须一致。
- `changelog-draft.json`：记录仓库、通道、base/head 完整 SHA、生成时间，以及 `entries`、`skipped`、`needs_review`、贡献者/报告者和来源证据。每个 PR、每个没有 PR 的直接提交都应有可追踪去向。

条目可沿用既有结构：`pr_number`（直接提交为 `null`）、`commit_sha`、`url`、`category`、`text`、`source`、`author`、`is_external`、`source_repo`、`confidence`、`rationale`、`feature_flag`。不要把旧脚本字段名 `internal_pr` 当成仓库私密性的判断依据。

仅在用户需要兼容格式时，使用 `scripts/convert_to_release_json.py --input <draft-json> --output <release-json>` 生成附加工件。当前 `infinishell_release.yml` 使用自己的发布说明生成路径，不会因为生成该 JSON 就更新 Release、Slack 或应用内更新说明。转换器只为带 PR 编号的条目附链接，直接提交链接仍以 Markdown 和审计 JSON 为准。

## 核验

核对区间内提交与审计去向，确认没有漏掉直接提交；每个 PR 以仓库加编号去重；复核标记、真实链接、通道可见性和贡献者归属。明确区分草稿、已合入源码与已经公开的版本，不自动发布或上传产物。
