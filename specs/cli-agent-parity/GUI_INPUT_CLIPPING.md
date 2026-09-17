# 本地 CLI 任务输入区裁剪验证

2026-09-17 在 macOS Apple Silicon 的真实 InfiniShell GUI 中复现并验证输入区裁剪修复。英文、简体中文界面均覆盖输入区滚到外层视口上方、部分可见和下方的位置；英文界面另验证中英混排多行草稿及内部选择。**输入区内的滚轮仍同时带动外层滚动，不能将本记录计为滚轮独立消费通过。**

## 构建与证据边界

修复后使用 `InfiniShellParityClip1.app`，源构建为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 加冻结的 **9 个文件快照**。这不是当前根工作区含 Claude 固定权限配置等改动的完整版本，也不是包含修复的干净提交。精确路径和逐文件 SHA256 见 [source-inputs.json](validation/gui-input-clip-1/source-inputs.json)。

构建及本地门禁分别记录在 [bundle.json](validation/gui-input-clip-1/bundle.json) 和 [gates.json](validation/gui-input-clip-1/gates.json)：`cargo check -p warp`、`cargo test -p warp --lib i18n::tests`、记录中的定向 nextest 过滤集均退出 0，GUI 构建退出 0。归档时核对了三份元数据的一致性及四份原始日志的 SHA256；没有重新运行这些检查，也未复制原始构建日志。

源构建 worker SHA256 为 `7395968450618673fa0e58dd7f838ba345f8b665c7199e212e5f1955906b2336`；隔离副本二次签名后为 `2e8f3fa68eae56358207f3b213792c08c197581d3d1cfa8b8360ef758aaa6a94`。隔离 GUI manifest 明确记录了两者，归档时重新散列副本验证一致。[runtime-binaries.json](validation/gui-input-clip-1/runtime-binaries.json) 同时保存修复前 Queue1 的对应摘要，未复制环境变量或将旧 manifest 的 commit 标签当作本次源码身份。

## 问题与修复

原输入区使用嵌套 `ClippedScrollable`，滚出外层任务视口后仍可能绘制占位文字。修复复用 `NewScrollable::vertical(SingleAxisConfig::Clipped { ... })` 和现有滚动状态，使输入内容的绘制受外层视口与输入区边界共同约束。未增加用户文案，本次无需本地化变更；已有中英文文案均纳入布局检查。

修复前截图中，英文占位文字出现在整图约 **y=45**，高于弹窗顶边约 y=72；这些坐标来自归档的 1229×768 图像。

## 真实 GUI 结果

| 证据 | 操作与观察 | 结论范围 |
| --- | --- | --- |
| [00 修复前英文](validation/gui-input-clip-1/00-before-fix-en.jpg) | Queue1：输入区滚到上方后，占位文字越过弹窗顶边。 | 原问题已复现。 |
| [01 修复后英文：上方](validation/gui-input-clip-1/01-after-fix-above-en.jpg) | Clip1：输入区完全位于内容视口上方。 | 占位文字未在弹窗外继续绘制。 |
| [02 修复后英文：部分](validation/gui-input-clip-1/02-after-fix-partial-en.jpg) | 输入区仅部分进入内容视口。 | 仅可见交集内保留文字，内容上边界裁剪生效。 |
| [03 修复后英文：下方](validation/gui-input-clip-1/03-after-fix-below-en.jpg) | 输入区移到内容视口下方，界面显示消息及附件区域。 | 输入文字没有溢出底边。 |
| [04 多行草稿](validation/gui-input-clip-1/04-multiline-end-en.jpg) | 输入 24 行 `Unsent draft line N: 中文换行与 English scrolling`，截图可见前部约 1–11 行及输入区滚动条。 | 中英文混排显示、换行和有限高度区域成立；文件名中的 `end` 不表示该图展示末行。 |
| [05 多行滚动与选择](validation/gui-input-clip-1/05-multiline-scroll-select-en.jpg) | 内部视口移动到第 14–24 行，并选中文字。 | 末行可达；文本与选择高亮限制在输入区内，未覆盖下方按钮。 |
| [06 修复后中文：上方](validation/gui-input-clip-1/06-after-fix-above-zh-CN.jpg) | 中文界面，输入区完全位于内容视口上方。 | 无输入文字越过弹窗顶部。 |
| [07 修复后中文：部分](validation/gui-input-clip-1/07-after-fix-partial-zh-CN.jpg) | 中文界面，输入区部分可见。 | 占位文字随内容视口裁剪，按钮文字保持可读。 |
| [08 修复后中文：下方](validation/gui-input-clip-1/08-after-fix-below-zh-CN.jpg) | 中文界面，输入区位于内容视口下方。 | 无输入文字溢出底部；较长中文消息信息正常换行。 |

截图中的任务输出仍是原有英文模型内容，不属于界面翻译缺失。部分可见状态的文字被视口截断是本次预期裁剪；本记录没有将其计为标签布局缺陷。中文界面没有另做 24 行草稿完整操作，因此不扩大多行验证的语言范围。

## 任务与未发送草稿

全程使用同一任务 `f13f4d58-7cf8-4690-9399-41de3bf11b87`。界面选择查看第 9 次历史结果；当前运行仍为第 13 次。归档时通过 SQLite `mode=ro` 与 `PRAGMA query_only=ON` 复核：[task-state.json](validation/gui-input-clip-1/task-state.json)。

- 当前第 13 次为 `disconnected`；第 9 次历史为 `completed`。
- 消息总数仍为 **14**，均为 `acknowledged`；接收确认不能作为任务完成依据。
- 24 行草稿没有发送。操作者确认测试后使用 Cmd+A、Backspace 清空；数据库计数复核没有新增任务消息。

本轮只验证布局和输入区域行为，没有启动新模型请求，也不把历史成功、滚轮动作或接收确认作为当前回合完成证据。

## 归档与未验证项

[manifest.json](validation/gui-input-clip-1/manifest.json) 列出每个归档文件的 SHA256。九张截图保留原始字节，无裁剪、转码或内容修改；源文件虽使用 `.png` 文件名，实际编码均为 JPEG，归档改用 `.jpg`，并保留源文件名映射。所有截图均逐张查看，并通过 Apple Vision 中英文 OCR 后的凭据模式扫描；JSON 也完成递归字段与值检查，无命中，未保存 OCR 全文或读取凭据文件。检查方法见 [privacy-check.json](validation/gui-input-clip-1/privacy-check.json)。

仍须独立完成：输入区滚轮是否应阻止外层同时滚动的处理与复测；含实际修改的同一提交的相关平台验证；当前完整工作区及 Claude 固定权限配置的 GUI 验收。此次 macOS 快照不证明 Linux、Windows、SSH/tmux 或三款 CLI 完整生命周期通过。
