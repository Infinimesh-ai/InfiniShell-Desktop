# macOS bundle8 冷启动授权、真实 Unknown 和 Unconfirmed 布局复核

日期：2026-09-16。此包属于 dirty 中间构建，包含另外任务的 `web_runtime.rs` / `websearch_tests.rs` 工作区修改，不属于最终同提交验收。构建来源见 [bundle8 构建记录](macos-gui-bundle8-build.json)。

## 构建与现场隔离

源执行文件 SHA256 为 `6f61a5d378dfd8e0cab94030ec3ef3d46e08ff3ec9029f2378229a2d7b3510e0`。普通 PTY 验证副本使用独立物理路径与 bundle ID `dev.infinishell.InfiniShellParityPTY8`，重新签名后的执行文件 SHA256 为 `50da358baf099842e78a7b9dfbceb254f13155f975ac54db88b57cf79feb01bf`。

先正常结束旧测试 Codex，再正常关闭 bundle7 测试应用（退出码 0）。bundle8 复用原完全私有 HOME、Codex 配置和受控原生插件来源，使用新的私有 app profile；没有访问或修改用户普通应用。没有重新安装插件、修改原生信任、调整通知设置或扩大 feature 范围。

[通知路径证据](macos-gui-bundle8-notification-lifecycle.json)保存冷启动前/本轮后的配置、信任、缓存 10 文件与持久来源 37 文件摘要。所有摘要相同，五项原生信任保持原值。

## 当前真实通知与 Unknown

1. 独立进程冷启动后，真实 Codex 0.147.0 直接进入交互，没有新的 Hook 授权操作。工具栏正确显示 `Verify notifications`，不再将已配置的冷缓存误报为待授权。
2. 发送唯一一次模型请求。本轮 CUA `typeText` 在当前拼音输入源下未按预定字符串输入：预定为 `只回复 GUI_BUNDLE8_NOTIFY_ONE，不调用任何工具。`，原生实际收到 `GUIBUNDLE8NOTIFYONE，。`。模型询问标记含义，故固定输出断言不通过；没有追加第二次模型请求来补齐该断言。
3. 本轮原生 session 为 `01a0aa54-55ed-7882-a1ef-50642e4c3161`，turn 为 `01a0aa54-971a-7a51-8ded-017eea64b37f`；原生记录恰有一个开始/完成回合，没有模型工具调用。
4. 当前回合开始后，`Verify notifications` 隐藏。该产品路径需要当前会话已收到富通知且插件版本达标；结合全新会话、唯一原生回合与后续通知，能够确认当前富通知被应用接受，不能据此声称五种 Hook 全部通过。
5. 原生输出结束后，通过正式 `Cmd-Shift-U` 打开应用内部通知列表，唯一当前通知为 `Codex needs attention`，正文完整显示 `Status unknown. Check the terminal to confirm the task outcome.`。此次普通 PTY Stop 明确呈现未知结果，没有将原生 Hook Stop 当成成功。

截图：[冷启动待验证](macos-gui-bundle8-cold-verify-notifications-en.png)、[当前通知后提示隐藏](macos-gui-bundle8-after-current-notification-en.png)、[真实 Unknown 通知](macos-gui-bundle8-unknown-notification-en.png)。英文正文在约 400 像素宽的通知浮层中完整换行，无裁切。

原有桌面通知设置为 `dismissed`，因此没有桌面通知，但应用内部通知列表仍提供了真实结果状态。普通手动 Codex 未绑定托管任务，本报告不要求其写入 `local_cli_tasks`，也不把缺少该表记录当成丢失。

本包没有验证中文 Unknown 通知正文：通知正文在事件发生时本地化为字符串，不能通过切换界面语言把原英文事件变为中文事件；本轮限定一次模型请求。该项应在后续最终提交的中文实际回合补验。

## Unconfirmed 双语布局与禁用控件

另外建立两个完全私有、无账号的 EN / ZH 应用实例，均从保留的 bundle8 物理副本复制，不再依赖后来可能被覆盖的共享构建输出。启动前数据库中任务为 0；应用和任务面板启动完成后，分别在对应的独立测试库插入一条明确命名的合成 `LAYOUT8-UNCONFIRMED-EN` / `LAYOUT8-UNCONFIRMED-ZH` 记录及相应运行历史，通过产品“刷新”读取。该夹具仅验证布局和禁用行为，不是原生生命周期或持久状态生产证据。

两种语言均实际检查：

- 任务行显示 `Outcome unconfirmed` / `结果待确认`。
- 详情显示相同状态和完整的“CLI 尚未确认任务结果，请查看现有终端”对应说明。
- 原生 ID 为非空的 `synthetic-layout-no-process-*`，避免将缺少 ID 导致的禁用误判为状态门禁。
- “Continue history” / “继续历史会话”呈灰色；实际点击后界面、任务代次和记录内容保持不变。
- 两个测试库最终仍各有 1 个任务、1 次运行、0 条消息；记录哈希不变，应用进程树下没有 Codex/Claude/Grok 进程。
- 两份布局应用均正常退出，退出码 0。

完整记录及截图摘要见 [Unconfirmed 布局证据](macos-gui-bundle8-unconfirmed-layout.json)。

| 检查 | 英文 | 简体中文 |
|---|---|---|
| 任务行 | [截图](macos-gui-bundle8-unconfirmed-row-en.png) | [截图](macos-gui-bundle8-unconfirmed-row-zh-CN.png) |
| 详情说明与继续禁用 | [截图](macos-gui-bundle8-unconfirmed-details-disabled-en.png) | [截图](macos-gui-bundle8-unconfirmed-details-disabled-zh-CN.png) |

指定的任务行、详情说明和按钮布局通过。额外发现英文历史结果下拉框的选中标签受默认宽度限制，`Run 1 · Outcome unconfirmed` 尾部有裁切；详情处状态全文可见，中文该标签完整。此项已单独反馈，不计为“所有布局均无裁切”。

## 验收边界

已通过：冷启动已配置授权的正确展示、当前会话富通知受理、真实英文 Unknown 状态，以及合成 Unconfirmed 的双语行/详情/禁用控制。

未通过或未验证：预定固定输出（输入注入偏差）、中文真实 Unknown 通知、五种 Hook 各自完整运行、英文历史选择框完整标签、最终同提交包的 GUI 验收。没有以本报告代替跨平台或完整 CLI 生命周期验收。
