# source26 Grok 官方 GUI 部分验收

本轮继续使用实际提交 `436cc739234061c092cedd106432bbbfa3c2d645` 的隔离重签应用副本，二进制 SHA-256 为 `2c0d7c02c7dd6cf21ee6088b0aa04dd8b4ea66287d6fb4be4dc3f4132a0a05ad`。Claude 已完成的基本链见 [独立报告](OFFICIAL_26_GUI_PARTIAL_VERIFICATION.md)。本报告只记录 Grok 已真实执行的步骤，不计 SDK、子任务、所有平台或 Goal 完成。

## 准备与认证范围

应用、HOME 和数据库继续使用独立验收数据域。Grok 另设当前用户独占的短路径临时 HOME 与项目，固定原生二进制为 `1.0.30`，SHA-256 为 `d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb`。仅 opaque 复制用户此前已完成官方授权的单个 `auth.json`，不解析、显示或归档认证内容。应用不继承 Claude API 认证值。

复用受控官方 TLS 隧道与 macOS 沙箱，只放行 `auth.x.ai`、`cli-chat-proxy.grok.com`；最多 32 条 TLS 连接、32 MiB，TLS 不解密，不能换算为模型 HTTP 调用或费用上限。独立配置选择 `grok-4.6-build`，所有工具要求原生审批，Claude／Codex 兼容 hooks 与 MCP 在此基本链中关闭；兼容 hooks 的单独验收不能由本报告替代。沙箱 canary 已真实证明指定代理与 Unix socket 可连接，其他网络及认证来源读取被拒绝。

第一次准备使用长 GUI 临时路径，Unix socket canary 在启动 CLI／任务前遇到 `AF_UNIX path too long`；代理和认证副本均已清理，0 条官方 TLS 连接，不计产品或模型失败。第二次改用短路径独立原生夹具后准备成功，原失败清理记录仍保留。公共元数据与截图见 [gui-grok-436cc739](validation/gui-grok-436cc739/)。

## 同原生会话两轮

实际任务 ID 为 `e48d0020-7629-4929-a75c-5c960ce2140c`，原生会话 ID 为 `01a0b0a6-e037-7cc1-9fc8-9573baffe8a6`。首轮输入包含中文、英文和三行文本，实际返回固定标记 `GUI_GROK_ONE_436CC739`。第二轮不提供上一轮标记答案，原生在同一会话中正确返回第一轮标记与 `GUI_GROK_TWO_436CC739`。

两条消息都有原生协议接收确认，两轮均持久化实际 `Completed` 与完整结果。首轮结果 21 字节，第二轮结果 43 字节；界面显示与只读 SQLite 核对一致。首轮 [证明](validation/gui-grok-436cc739/05-grok-first-result-sql-proof.json) SHA-256 为 `d14c1ead8a21fd4400a6a15776689e0930009d13c410e69bbb9d6c193d73c259`；第二轮 [证明](validation/gui-grok-436cc739/09-grok-second-result-sql-proof.json) SHA-256 为 `4779f29c1f9c68bab238f16fade68901fa45904a0d8d54fed7e0aa631a80171a`。截图 `03`—`10` 对应输入、接收与实际结果；粘贴接口超时后已逐次核对真实文本，没有因工具超时重投输入，不计中文输入法验证。

## 审批与后续待验

第 3 轮实际进入 `waiting_for_user`，原生 `session/request_permission` 指向隔离项目中的 `gui-grok-approved.txt`。审批前目标文件仍为 26 字节、SHA-256 `7857aa507dea8e88e0382e7eed1a7c215bc35f2acc27a0e0535c82a8b1e12a93`。截图保留原生允许／拒绝选项与目标路径；未从界面推断没有展示的工具输入字段。实际点击 `Allow once` 后，文件才改为预期的26字节、SHA-256 `de2ce846b5c3e93ea6ddce4a7c187b3a3c9399a8d699b861b83e4710f3cb3851`，与预期文本及单个换行逐字节一致；第3轮保存完整74字节结果，包含原生中文前言与完成标记，未压缩为固定末句。[允许后的证明](validation/gui-grok-436cc739/18-grok-after-allow-sql-proof.json) SHA-256 为 `da0184c8f1716d31846c836839ae8394db8b6b0a35f80d94817bcc6f4661e2ce`，截图 `19` 显示实际结果。

第4轮同样先进入真实等待审批，点击 `Deny once` 后目标文件保持原31字节和SHA-256 `36d67c532bbb91adbcc014e32275ba5d21bc0c5243ba70cac5bb6527c43848c3`，没有改写或重试。Grok原生以 `Cancelled` 结束该被拒绝的回合，保留58字节已产生的中文前言；没有强行转换为成功或伪造拒绝后的模型答复。[拒绝后的证明](validation/gui-grok-436cc739/26-grok-after-deny-sql-proof.json) SHA-256 为 `f57e50724e694b4c780dfe5c82df1ad8c80a6d591cc61f3555fce219179bd80b`，截图 `23`—`27` 与其对应。

第5轮再次要求实际 Write，原生等待审批时，取消目标文件仍为33字节、SHA-256 `da1066b3e92c90bd1812729c96725e77e45d839f16d6a1b1d2bc66205c1db227`。截图 `31` 保留该轮原生工具路径及 `rawInput`；未点击允许。随后通过运行中的 `Queue instruction` 提交下一条指令；提交后消息数由5增至6，但尚未获得原生确认，草稿继续保留。实际点击 `Cancel turn` 后，第5轮原生取消，第6轮自动执行后续指令并完成；目标文件保持原值。追加消息 ID `4fa7702e-37fe-4baf-98c5-b726b33b9a32` 的提交代为5、实际执行代为6，其原生回合 ID `64715aef-c39f-4d53-9e71-cf3eaa0907a6` 与结束事件匹配，原生会话 ID 不变。[取消及追加结果证明](validation/gui-grok-436cc739/36-grok-cancel-and-appended-result.json) SHA-256 为 `3de9b7077a21d3095b7a07d6cf4e6ef2effbaa1b3bc9b3d5dafe176302fe2c73`；保存24字节完整结果，包含追加标记及原生输出的末尾换行，未把入队当作确认或完成。

第6轮完成后实际断开 CLI，`Continue history` 仅启动同一历史会话的连接，第7代停留 queued，消息数仍为6，没有重放旧输入；[启动前后证明](validation/gui-grok-436cc739/40-grok-history-connection-before-new-message.json) SHA-256 为 `b6e93d4fd77d0ab73361cbcca756d0637a5ce00f7e04ca5b680344f2f520964c`。新草稿的前几次鼠标点击没有增加消息数；编辑器聚焦后再次操作，才实际收到第7条消息 `807c0e0f-fafa-4c57-bd32-40d69df57b85` 的原生确认，草稿清空。未认定前几次点击已进入发送动作，未重放已收到确认的消息。[只读代码复核](GROK_HISTORY_SEND_REVIEW.md) 未找到确定的生产缺陷，点击无效原因保持未知。

SDK9及无输入policy1的真实失败保持原结论；候选source27的policy2、SDK10同样失败。新[协议差异复核](GROK_27_PROTOCOL_DIFFERENCE_REVIEW.md) 将SDK未知响应ID的摘要精确匹配到固定二进制的 `skills-reload` 协议字面量，但尚未证明请求所有权或安全接纳规则；[官方候选契约复核](GROK_SKILLS_RELOAD_CONTRACT_REVIEW.md)已核实内部维护方法及包装结果的源码依据，未证明它对应固定受测native。25字节通知的原始UTF-8摘要已匹配 `_x.ai/mcp/servers_updated`，形成[仅空目录的候选修复](GROK_POLICY_GLOBAL_CATALOG_FIX.md)，尚未真实policy3验收。根任务成功不证明业务 SDK 来源、完整父权限上限或子任务能力。

## 应用重启与网络夹具边界

第7代收到原生确认及开始事件后，观察约4分钟仍没有正文或终态。随后通过应用正常退出，重开相同隔离副本与数据域；第7代恢复为 `Disconnected`，7条消息及已有结果保留，不自动重投。[重启后证明](validation/gui-grok-436cc739/53-grok-restored-task-disconnected-without-replay.json) SHA-256 为 `8aa37fd312190b796b098fb925343084aae4819e09dfa29142f762d9d10e092d`。重开前的进程检查发现旧应用进程为僵尸态，其他记录的后代进程已消失；未据此声明所有旧PID不存在或原生退出码为0。两次准备守卫提前拒绝后才实际启动新应用，原始守卫记录没有改写为成功。

明确点击 `Continue history` 后，第8代queued时仍为7条消息、原生会话ID不变。随后仅发送一条新指令，消息ID `6b7c34d5-5d09-4158-a8f5-3d2506786339`、原生回合ID `87b07999-e63e-45c7-b147-39758806880c`，均取得原生确认。第8代最初尚无终态；当时原网络夹具已达到32次TLS转发上限／1800秒期限，拒绝记录共72次预算拒绝、214次其他authority拒绝；[原网络汇总](validation/gui-grok-436cc739/61-grok-network-scope2-final.json)保持原始字节与SHA。该上限属于验收夹具，未推断为Grok登录失效。

随后一次补网夹具虽绑定原端口，却因线程仍指向旧已关闭server，在服务前失败；[失败与清理记录](validation/gui-grok-436cc739/63-grok-network-scope3-fixture-failure-cleanup.json)保留，没有算作原生模型失败或网络成功。修正专用夹具的线程目标后，第4网络窗口恢复同一端口，仅允许原有 `auth.x.ai`、`cli-chat-proxy.grok.com`，上限96次TLS／32MiB／900秒；没有重启应用或CLI，也没有新投递输入。此后第8代真实 `Completed`，保存72字节完整结果，依次为首次、追加和重启标记，SHA-256 `bfdc32dfa6ef1c4e44b7f96298edd0f608073f5db9513f0596c2d1a3f79521f5`。[最终只读证明](validation/gui-grok-436cc739/59-grok-restart-new-input-result.json) SHA-256 为 `68204d3068d27c78653af0b5de5ae5d5dd1a38fe6635fc5dcf1c1422044bbd5f`，结束事件与当前第8代原生回合和会话匹配，8条消息均确认。截图 `66` 实际显示三行完整标记；截图 `65` 仅显示部分结果，不用它证明完整内容。

[恢复网络契约复核](GROK_RESUME_NETWORK_CONTRACT_REVIEW.md)只读核对官方候选源码与安全公共记录。第8代完成说明两个未知authority及 `api.x.ai` 不是该回合完成所必需，保持最小网络域；没有认定所有Grok功能都无需它们，也没有给第7代无终态指定唯一原因。上述结果不覆盖第7代的中断记录。

## 独立历史继续复验

第8代完成后明确断开CLI，再次点击 `Continue history`。第9代queued时消息数仍为8，同一原生会话，没有旧输入重放；[连接就绪证明](validation/gui-grok-436cc739/69-grok-history-nine-ready-no-replay.json) SHA-256 为 `f8479ec2ddbd65b3928faf0275bd6d7bc480f5bed48d7949cfbccae45c5e608b`。随后发送新的记忆指令，消息ID `c3c7a4eb-146a-4642-b10d-ace66243c519`、原生回合ID `7404c720-0838-49cc-ac4c-aa8e06c7da72`、运行实例ID `f5c2c8bc-afe1-4109-bc48-8212a1e89b98`；[原生接收证明](validation/gui-grok-436cc739/72-grok-history-nine-native-ack.json) SHA-256 为 `da2759938e153888062004ab709af6feddf11136c3519446458d606638739bbb`，共9条消息。粘贴接口返回超时后已核对界面草稿完整，只发送一次。

第9代随后真实 `Completed`，结束事件的会话及原生回合与当前输入匹配，保存71字节完整结果，依次为首次、追加和本轮历史继续标记，SHA-256 `4be96ba053005587e0bc9b2724176060db7775692fe3a56b1dd0824eff72d6c8`。[结果证明](validation/gui-grok-436cc739/73-grok-history-nine-result.json) SHA-256 为 `924351f458c47aeaf468988d4bb7dd456dcc282a2be26125f1659c78099f287c`；截图 `74` 实际展示三行标记。正常断开并通过应用退出后，[再次只读核对](validation/gui-grok-436cc739/76-grok-result-after-normal-app-exit.json)确认该结果、9条消息及第7代Disconnected均持久保存，SHA-256 `12e53e42f618a174c79b0422c851a309529ce6d67876fb4deaea42e4b99dce8a`。应用PID31783已不存在，但不据此声明原生退出码为0。

第4网络窗口已主动结束，生产夹具进程实际exit0，15次官方TLS转发／744175字节、隧道关闭与opaque认证副本删除均确认。[最终网络记录](validation/gui-grok-436cc739/77-grok-network-scope4-final.json) SHA-256 为 `278176adf6186e3d5eaee8cbf3b46d0d32ff222a3698469916129988ae72adf6`；网络生产夹具exit0不代表Grok原生exit0。根任务基本能力已有上述实际证据，原第7代失败及SDK／权限上限、父子、普通PTY、其他平台的缺口仍保留，Goal未完成。

本轮仅验收，不修改产品源码或用户文案；无需本地化变更。此前双语组件检查保持独立范围，本轮尚未完成 Grok 全链双语检查。
