# 实际436的Claude PNG桌面与恢复验收

本轮使用实际提交 `436cc739234061c092cedd106432bbbfa3c2d645` 的隔离重签macOS应用，副本SHA-256 `2c0d7c02c7dd6cf21ee6088b0aa04dd8b4ea66287d6fb4be4dc3f4132a0a05ad`、Claude `2.1.273`、用户已授权API模型 `claude-sonnet-4-6`。凭据仅通过既有受保护文件加载到独立应用环境，没有在截图、证明或新增凭据文件中记录。应用、HOME、项目、SQLite和CLI配置仍为source26独立验收域；不操作日常应用或用户全局设置。

此次覆盖真实GUI附件、持久化、显式历史继续、正常应用重启、不自动重放及新文本输入结果回收。它与[source23生产typed PNG原生验收](OFFICIAL_23_NATIVE_LIVE.md)分别保留范围，不代表最新0059提交、其他格式、父子任务或其他平台已通过。

## 合成图片与真实桌面操作

生成96×96的合成PNG，左半红色、右半蓝色，248字节，SHA-256 `6e6f0c4608c8d97790950c80e966369767f0d6c7f043b65a16e8ec206000695d`。[原始夹具](validation/gui-claude-png-436cc739/fixture-colors.png)不含用户图片。通过Computer Use的 `@oai/sky` 操作本地任务窗口，选择Claude、原隔离项目及Inherit权限，未启用子任务或父子消息。

点击附件按钮后使用真实macOS文件选择器。最初GoTo字段的粘贴、常规Return与部分点击没有生效，未选择默认显示的用户文件。改用本次合成PNG的短路径普通文件，设置可编辑字段，并用 `KP_Enter` 确认后才完成选择；这些工具操作差异不计产品失败、中文输入法或附件完成证据。短路径符号链接试探未被选中；最终选择的是普通PNG文件。截图 `01`实际显示附件chip，`02`保留中英文三行草稿。粘贴工具返回超时后已核对完整草稿，没有重投。

## 三轮真实结果与持久化

任务ID `f11f573b-504a-4423-bd04-94942633f19d`，原生会话ID `273ef03b-aada-4921-8d78-9a387b61aa0c`。第一轮指令要求从图片识别左右颜色，未提供具体颜色答案；应用保存的输入为 `Submit.input=[Text,LocalImage]`。结果真实为 `GUI_CLAUDE_PNG_436CC739`、`left=red`、`right=blue`，43字节，SHA-256 `59bdfd4e0e1411d3696a1f2e85a10551c471de8421acefbac2e72ccc60b91837`。原生结束回合与消息UUID `0ba11103-3b55-4ff1-89b5-6e94247706c8`匹配。[只读SQLite证明](validation/gui-claude-png-436cc739/05-png-first-sqlite-result.json)SHA-256 `aa4628d9a52f3631cd7014a0c972eb92eef1d27e47d6258433ae23fd20c855c4`；截图 `04`实际显示三行结果。

附件已按内容SHA存入当前数据域的 `local-cli-attachments`，文件名SHA、248字节和原始PNG完全一致。公开证明仅记录媒体类型、大小、SHA、路径域验证布尔值，不输出输入全文、base64或私有数据库。它证明应用持久化引用及实际结果；完整原生wire图片投影另由source23验收支撑。

第一轮完成后明确断开CLI，再点击 `Continue history`，第2代queued时仍为1条消息、同一原生会话，附件引用存在。[历史连接不重放证明](validation/gui-claude-png-436cc739/08-png-history-ready-no-replay.json)SHA-256 `6c98e730ee3f6a66d34ceec86d6b9315544680fddc355b10422044789f1e3ba7`。明确提交一条新的纯文本记忆指令，不附加图片；第二条消息UUID `2e90dda4-dc5d-4d84-8e1f-716f3c81a709`，保存输入只有Text。实际 `Completed` 50字节，含 `GUI_CLAUDE_PNG_RESUME_436CC739`及正确左右颜色，结果SHA-256 `858daf5095b0a13d5141a6431b40fbb6b092e26c8b1c3d9f084d17c2c1687227`。[结果证明](validation/gui-claude-png-436cc739/12-png-history-colors-result.json)SHA-256 `ee423c2fca5098ff94dbb23a2bc265b95e2439dbf6dd9841bc200be876f433a7`，截图 `13`显示实际结果。

随后通过应用快捷键正常退出，重开前确认应用PID35536及记录的全部后代PID均不存在，没有观察退出码，不编造原生退出0。新应用PID38245使用相同副本、HOME、项目和数据域。实际列表恢复原任务；消息数仍为2、两代结果及图片文件保持。[重启恢复证明](validation/gui-claude-png-436cc739/18-restart-attachment-and-no-replay.json)SHA-256 `29d97dbb4b08d03b55a1fb51b6793313f7b1c5eb41fa694606c913cf583d3fea`，截图 `16`—`19`记录恢复的任务、消息与历史结果。

重启后明确点击 `Continue history`，第3代queued时仍为2条消息，旧图片输入没有自动投递；[连接证明](validation/gui-claude-png-436cc739/21-restart-history-no-input-replay.json)SHA-256 `ae803c737787ff5f25425ce9740672a31cc7087d9d133f57bdd591c3d77ba70e`。然后仅发送一条新的Text输入，UUID `d7a0597f-ce6a-4752-9ec1-3a4187be3c6d`，实际回收51字节 `Completed` 结果，含 `GUI_CLAUDE_PNG_RESTART_436CC739`、`left=red`、`right=blue`，SHA-256 `27ba55336dd01df0a534bbb6eb6118f23927394133ec11e6c5ca06412c58f569`。[最终结果证明](validation/gui-claude-png-436cc739/25-restart-image-memory-result.json)SHA-256 `5c59fc71239506f060a6da582d7ce6ccde7e90a2e3ede9b27eb0fa8cd06b6747`，截图 `26`实际显示三行。

最终正常断开并退出应用，PID38245已不存在；[退出后的只读证明](validation/gui-claude-png-436cc739/28-final-result-after-normal-app-exit.json)SHA-256 `eeecaab33cacfda9df03ec2a8d3bb6ac92ed540f8a4fcb53c8535d6ba3bb5c7c`确认3代Completed、3条原生协议ACK及同一图片文件仍在。没有把退出请求、queued或ACK自身计作原生完成。

## 范围与未通过项

已完成上述实际436、Claude PNG的桌面根任务专项验收；不覆盖技能与PNG混合、其他图片格式、活跃进程重新关联、父子GUI、普通PTY、真实输入法、其他平台或完整异常组合。全Goal仍在实施中。本轮没有产品源码或文案变更，无需本地化变更；本次图片专项界面为英文，不替代完整双语布局门槛。公共证据还须完成独立归档安全审计后才能暂存。
