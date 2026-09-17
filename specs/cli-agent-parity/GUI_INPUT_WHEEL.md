# 本地 CLI 输入区的双语滚动与裁剪验证

2026-09-17，macOS 中间 GUI 构建的输入区滚轮与裁剪检查通过。英文、简体中文各使用 24 行中英混排草稿，输入框自行滚动时外层正文和操作按钮保持原位，末行可完整显示；输入框移至正文上下边界时没有文字或光标越界。本轮检查的是既有断线 Claude 任务的布局，没有启动新固定策略原生会话或提交模型请求。Claude 新策略的完整双语 GUI 流程、三方完整生命周期和最终同提交平台验收仍未完成。

## 构建与证据范围

本轮基于 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f` 加 38 文件的冻结输入，工作树不是干净提交。输入清单、门禁和 GUI 构建报告均以原始字节保存在 [证据目录](validation/gui-input-wheel-1/)。已核对清单与构建报告、GUI 安全投影中的内容一致。该冻结输入的本地 `cargo check -p warp`、国际化 11 项及相关模块 1072 项通过，三个原始门禁日志的摘要也已复核；这些结果不代替最终提交的跨平台验收。

| 文件或阶段 | SHA-256／结果 |
| --- | --- |
| 原始冻结输入清单 | `5f6f6cf2f5014ec59dc0439282ffeb241bebb97c7fde317c3f4b7f1810337d86` |
| 构建报告记录的源 worker | `ade1e8aa69c26c900b72a8697e71f2942f07813084b209cd45f973e2f8ce1c10` |
| 隔离 Wheel1 app 的重新签名 worker，归档时重新计算 | `11f153d40ec19ea7c7b06365bf1f57d09ccdedd2761d8daac7e59546465353fb` |
| 隔离 app 的 `codesign --verify --deep --strict` | 通过，退出码 0 |

源 worker 所在共享 `target` 路径已经被后续构建覆盖。因此源 `ade1…` 摘要依据本轮构建报告及隔离复制时的核验记录；归档时重新核验的是保留的 Wheel1 隔离副本，未把共享路径当前字节冒充旧快照。

私有 app manifest 只保留 `app`、`bundle_id`、`source_snapshot`、`source_binary_sha256` 和 `resigned_binary_sha256` 五个安全字段。环境变量、API 私有文件路径内容、运行日志和其他私有字段没有复制进归档。

## 实际观察

截图均为原始 JPEG，分辨率为 1229 × 768，未转码、遮挡或裁剪。编号 03、11、14 是中间状态或命名不精确的截图，本轮未采用。

| 语言／场景 | 原始截图 | 观察 |
| --- | --- | --- |
| 中文：输入框滚轮前后 | [01](validation/gui-input-wheel-1/01-zh-before-wheel.jpg)、[02](validation/gui-input-wheel-1/02-zh-after-inner-wheel.jpg) | 第一条完整可见行由 1 变为 8；操作按钮中心 y=627 保持不变，外层正文不随输入框同时移动 |
| 中文：滚动至末尾 | [04](validation/gui-input-wheel-1/04-zh-last-line.jpg) | 第 24 行中英文内容完整显示；下方操作按钮仍在原位 |
| 中文：清空草稿 | [05](validation/gui-input-wheel-1/05-zh-cleared.jpg) | Cmd+A、Backspace 后恢复中文占位提示，24 行测试草稿未发送 |
| 英文：任务头与选择状态 | [06](validation/gui-input-wheel-1/06-en-task-header.jpg)、[07](validation/gui-input-wheel-1/07-en-selected-task-header.jpg) | 正确展示既有 Claude 断线任务及 CLI 身份；选择该任务不启动历史会话 |
| 英文：输入框滚轮前后 | [08](validation/gui-input-wheel-1/08-en-before-wheel.jpg)、[09](validation/gui-input-wheel-1/09-en-after-inner-wheel.jpg) | 第一条完整可见行由 1 变为 8；操作按钮中心 y=603 保持不变，外层正文不随输入框同时移动 |
| 英文：滚动至末尾 | [10](validation/gui-input-wheel-1/10-en-last-line.jpg) | 第 24 行中英文内容完整显示，末尾光标在输入框范围内 |
| 英文：部分越过上边界 | [12](validation/gui-input-wheel-1/12-en-partially-above.jpg) | 上方输入内容在正文边界裁剪，标题区域不被草稿覆盖 |
| 英文：完整移至上方 | [13](validation/gui-input-wheel-1/13-en-fully-above.jpg) | 输入框内容与光标消失，正文历史结果正常滚动 |
| 英文：完整移至下方 | [15](validation/gui-input-wheel-1/15-en-fully-below.jpg) | 输入框在正文下方，没有草稿或光标在正文之外显示 |
| 英文：清空草稿 | [16](validation/gui-input-wheel-1/16-en-cleared.jpg) | Cmd+A、Backspace 后恢复英文占位提示，24 行测试草稿未发送 |
| 中文：部分越过上边界 | [17](validation/gui-input-wheel-1/17-zh-partial.jpg) | 输入框仅在正文范围显示，中文占位提示在边界内裁剪 |
| 中文：完整移至上方 | [18](validation/gui-input-wheel-1/18-zh-fully-above.jpg) | 输入框内容与光标消失，标题区域保持完整 |
| 中文：完整移至下方 | [19](validation/gui-input-wheel-1/19-zh-fully-below.jpg) | 输入框在正文下方，没有占位提示或光标越过正文边界 |

这些观察验证了输入框处理滚轮后外层容器不会再次滚动，以及输入、占位提示与光标遵守父层正文的裁剪范围。中英文控件各自允许文案高度差异，并分别保留其按钮位置和完整末行；没有要求两种语言使用同一组绝对坐标。

## 任务记录与隐私复核

[本轮只读任务投影](validation/gui-input-wheel-1/task-state.json)采于 `2026-09-17T11:33:18Z`，通过 SQLite `mode=ro` 和 `PRAGMA query_only=ON` 读取。与[上轮只读基线](validation/gui-input-wheel-1/baseline-task-state.json)逐字段比较后：任务和原生会话 ID 均未变，当前仍为第 13 代、`disconnected`，消息仍为 14 条 `acknowledged`，增量为 0。查看第 9 代已完成结果只是显示已有历史，并未重新执行任务。

本轮仅选择任务、输入未发送草稿、滚动并清空草稿，没有启动或继续操作；HTTP 请求数量未独立测量，因此不把数据库不变单独当作网络请求计数证明。归档保留了测试任务／会话 ID 和无敏感内容的测试文本，没有原生登录凭据、API 认证值或原始私有配置。

选定的 16 张图片已逐张人工查看，并在本机使用 Swift Vision `VNRecognizeTextRequest` 的英文／简体中文准确识别模式进行静态 OCR。对逐行文本与去空白拼接文本执行凭据形态扫描，全部命中数为 0；只保存方法、图片数量、识别段数与命中数，未归档 OCR 全文。全部归档 JSON 也执行了凭据形态、私有 API 环境文件路径与端点扫描，命中数为 0。来源映射和逐文件摘要见 [archive-origins.json](validation/gui-input-wheel-1/archive-origins.json)与[manifest.json](validation/gui-input-wheel-1/manifest.json)。

## 未计为通过的项目

- Claude 新固定权限策略的新建、真实模型交互、审批、父子任务与恢复 GUI 流程。
- Codex、Grok 完整 GUI 生命周期及三方真实结果回收。
- Linux、Windows、SSH、tmux 的本轮实际滚轮和裁剪行为。
- 含全部实际修改的同一干净提交与最终完整平台门禁。

本轮是既有功能的中英文布局复核，没有新增或改变文案，无需本地化变更。P0–P5 与 Goal 仍保持实施中。
