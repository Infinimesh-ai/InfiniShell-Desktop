# source31 附件历史的双语布局与复制实测

本轮只验附件历史显示与复制。对象为实际父提交 `0059ef1bf8438c5af7545c3101b880ec2d0b10e9` 的 source31 未提交候选：111 个输入、恰好 7 项修改，`same_commit_verified=false`。不包含后续 Grok 插件0.1.1迁移、缓存认证探针握手及生产维护响应兼容修复。

[本地门禁](OFFICIAL_31_LOCAL_GATES.md)已通过 check、i18n11、Rust1430、Python443；新增SDK7和UI3逐项各一次PASS。随后 [main构建与签名](validation/macos-official-31-bundle.json)实际退出0，321.646秒，二进制866049280字节，SHA-256 `5d0320978e1f8df4f88e8ee20fd78c3f925c31ec42aa0ac67d36f9f61db3bfa2`。完整英文和简体中文FTL字节均已核对嵌入，严格签名检查退出0，原测试二进制前后保持一致。

独立副本使用 `dev.infinishell.InfiniShellParityOfficial31`、新HOME与数据profile；重签名后SHA为 `501495704bfc4bfe45668cdec2092b36a2cccaa2022d14b1c9749e333359fb52`。没有复制CLI认证、API环境或原生历史，也没有改动原source26应用。显示数据来自实际436提交的 [真实Claude PNG任务](OFFICIAL_26_CLAUDE_PNG_GUI_VERIFICATION.md)：完整应用SQLite数据库逐字节复制，消息没有改写或注入。原任务第3代的Completed是旧实测结果，不计为本候选新的CLI执行完成。

第一次准备因保留的非空SHM而拒绝，尚未启动新应用或复制数据。后续确认全部旧进程不存在、WAL为零；使用immutable只读连接读取完整主数据库，保留原SHM及主库／WAL的字节哈希。非空SHM是既有索引文件，不把它当作未提交事务。本轮前后原三个文件的哈希均保持一致。

根代理通过Computer Use实际检查1229×768窗口中的英文与简体中文：

- [英文附件消息](validation/gui-preview-official-31/01-english-attachment-history.jpg)：中文首行、英文多行原文正常换行，图片显示为“Image attachment”。
- [中文附件消息](validation/gui-preview-official-31/06-chinese-attachment-history.jpg)：原文保持一致，图片显示为“图片附件”。标签无截断，复制与恢复控件尺寸正常。
- 两种语言均没有把序列化动作JSON或内部附件缓存路径展示为消息正文。
- [真实复制后粘贴](validation/gui-preview-official-31/07-actual-copy-paste-draft.jpg)：点击历史的“复制消息”，使用原生粘贴快捷键粘贴到本地草稿，得到完整可读文字和中文图片描述。没有发送指令，随后清空本轮草稿。

语言在应用设置中选择，正常退出后使用同一独立副本重启，简体中文设置实际生效。没有点击继续历史、启动任务或发送；CLI认证文件不存在。[英文查看后](validation/gui-preview-official-31/02-after-english-view.json)、[首次退出后](validation/gui-preview-official-31/04-after-normal-app-exit.json)、[中文及复制后](validation/gui-preview-official-31/08-after-chinese-view-and-copy.json)、[最终退出后](validation/gui-preview-official-31/09-after-final-normal-app-exit.json)均核对任务、代次、消息三个表的完整行数量及哈希与复制前完全一致。新原生输入为0，没有自动重放或新结果回收。

[清理记录](validation/gui-preview-official-31/10-process-cleanup.json)核对两次应用PID46032／46379及此前快照的全部所属子进程均不存在；正常退出操作已执行，进程退出码未观察，不补记为0。[根代理视觉记录](validation/gui-preview-official-31/11-visual-review.json)明确为实际查看截图，未做OCR。

本轮不覆盖真实技能历史标签布局、其他图片格式、普通PTY、父子任务、最终实际修改提交或其他平台。技能标签及输入顺序只在新增模块回归中通过；图片历史显示不能算作本候选重新执行完整CLI链或应用恢复原生连接。Goal及P0–P5验收仍未完成。

归档的独立范围核对见 [审计报告](OFFICIAL_31_GUI_PREVIEW_ARCHIVE_AUDIT.md)；该审计不重复执行GUI或模型。
