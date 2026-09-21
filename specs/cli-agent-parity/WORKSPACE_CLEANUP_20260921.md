# CLI 对齐工作区整理记录（2026-09-21）

用户要求核对未提交改动并完成整理。本轮已将根目录同步到已推送的 CLI 分支，清除重复展示的历史差异，并把新交接文档集中到根目录。没有继续开发 CLI 功能，也没有将 Goal 改回完成。

## 2026-09-21 后续阶段的内置盘迁移

续接开发期间发现 `/private/tmp` 与用户临时目录仍保留多批本目标的旧 worker、GUI、IPC、升级夹具和 CLI 缓存。用户明确要求清除内置盘占用并改用外置盘后，约 10 GiB 已迁至 `/Volumes/ORICO/InfiniShell-Desktop-local-offload-20260921`；内置盘可用空间从约 1.8 GiB 提升至约 14 GiB。Cargo target 与新的私有临时根均使用 ORICO。

迁移只处理已确认属于本目标、已经停用且可恢复的路径；代码、当前收据和认证目录未删除或迁移。跨卷复制时一个已停止进程的 Unix socket 无法保存，它是不可复用的运行时端点，不属于持久证据。Grok 最后一次 launchd 验收曾临时保留一个内置 supervisor，结束后已删除；Claude 测试 profile 已迁到同一外置归档。

## 改动究竟是什么

整理前根目录 HEAD 为 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`，而远端 CLI 分支已到 `e6e9d619318e87d0bb889df344c8570f97977e90`。Git 按目录折叠未跟踪项时容易看不出规模；使用 `--untracked-files=all` 实际得到 1,092 个文件路径。

| 类别 | 文件数 | 整理结果 |
| --- | ---: | --- |
| 与已推送文件逐字节一致 | 1,021 | 收拢到已提交内容，不重复提交 |
| 仅 Git 换行归一化不同 | 4 | 按 Git 属性核对后，与已推送 blob 一致 |
| 已被后续提交替代的旧版本 | 18 | 原 blob 或归一化 blob 可在 CLI 分支历史找到；保存后采用最新提交 |
| 已合入 main 的网页搜索修复 | 2 | 保存原状，移除 CLI 工作区的重复残留，不重新生成修复提交 |
| Dock 插件构建产物 | 22 | 完整备份，不纳入源码提交 |
| gui-7e065085 历史 GUI 证据 | 23 | 原字节归档，不混入最新功能验收 |
| 根目录新交接导航 | 2 | 与 validation-56 的完整交接文档合并整理 |
| 合计 | 1,092 | 全部先备份校验，再整理 |

另对 validation-56 的 6 份当前交接文档单独保存。两处共 1,098 个文件均经归档回读和 SHA-256 核对；原索引、暂存补丁、未暂存补丁和原 Git 状态也独立保存。

## 网页搜索两文件已经交付

原根目录 `web_runtime.rs`、`websearch_tests.rs` 允许网页搜索的数字参数使用字符串形式，并补充回归测试。它们逐字节等同已有提交 `0018ed080d2f6cfd1e6d55c80ab8e129088a643a`「修复网页搜索数字字符串参数兼容问题」。

本次只读审计确认该提交已在远端 main（核对时为 `a6e61a1517c15c9250fb0443006d5c764842aab5`）历史内；对应独立分支和工作树也保留该提交，工作树干净。因此这不是另一项尚未完成的源码任务。CLI 分支尚未合入该修复；本轮没有为了整理而合并 main、重复提交或 cherry-pick，后续整合主线时按正常流程处理。

## 可恢复保存

归档目录：

[整理前完整备份](/Users/zhishi/Tools/github/InfiniShell-Desktop-backups/20260921-015307-uncommitted-cleanup/README.md)

| 保存对象 | 标识 |
| --- | --- |
| 根目录原状态 stash | `dac66b0d727c8b0b6161aa2fc4323146a0833f97` |
| validation-56 原文档 stash | `7082d5ad723d190ef32a7097c5730cbf860ac52f` |
| 根目录归档 | `root-working-files.tar.gz`，SHA-256 `6355667d325e3f2a27a0f0bae9240de7e810e8c92a52a5f5b59a36c011c90b07` |
| validation-56 归档 | `validation56-working-files.tar.gz`，SHA-256 `0dacc174aebd7337cbfa3925ac5de1d4d5f7043cd13896ce81dddb26c611c67d` |

目录中的 `inventory.json` 逐项记录类别、旧 HEAD／远端／本地 blob 与状态；`backup-manifest.json` 记录每个原文件的 SHA-256、大小、模式和归档摘要；`saved-stashes.json` 固定两个 stash 的对象 ID。所有旧 stash 保留，未 drop 或 pop。独立归档目录权限为 0700，初始备份文件为 0600。

需要旧文件时，先从归档或固定 stash 提取到单独目录再比较；不要把完整旧 stash 直接 pop 到当前工作区，否则会重新引入已交付旧代码及大量重复证据。

## 整理后的唯一工作入口

- 根目录：`/Users/zhishi/Tools/github/InfiniShell-Desktop`。
- 分支：`codex/cli-agent-parity`。
- HEAD：`e6e9d619318e87d0bb889df344c8570f97977e90`，采用 fast-forward，未重写历史。
- 原 validation-56 工作树：仍为同一提交的干净 detached HEAD；其中未提交交接已移至根目录。
- 其他历史工作树与分支没有删除或改写。
- 根目录只保留本次 7 份交接／状态修正／整理文档，集中暂存便于后续审阅；没有未提交的产品源码变化。
- [最新交接](HANDOFF_20260921_REOPEN.md)与[新 Goal 指令](GOAL_20260921_REOPEN.md)已改为从根目录继续。此前聊天给出的 validation-56 路径已被本轮更新。

这 7 份文档仍未形成新的交付提交，也未推送。已暂存是为了让它们成为清晰的一组交接改动，不代表 CLI 的剩余功能验收完成；新 Goal 必须保留并纳入后续提交。

## 本轮检查范围

- 整理前所有原文件的归档回读、摘要、Git 状态和索引保存。
- 源码归属与历史 blob 核对；网页搜索修复的主线归属另作只读审计。
- 整理后根目录产品代码与 e6e9d6193 比较，检查没有非文档差异。
- 暂存文档 `git diff --cached --check`，本地 Markdown 链接和结尾空白检查。
- 根目录分支／HEAD、远端跟踪关系、validation-56 干净状态及固定 stash 可达性检查。

无需本地化变更。本次未执行 Cargo、模型请求、真实 CLI、GUI 或跨平台 CI；只是恢复到已提交代码并整理文档，不为用户启动额外的全量编译，也不将旧测试绿灯冒称本轮新功能通过。
