# source26 Claude PNG GUI 公共归档独立审计

本审计复核已关闭的 [Claude PNG 桌面与恢复验收报告](OFFICIAL_26_CLAUDE_PNG_GUI_VERIFICATION.md)及其公共证据，不独立重演 GUI。实际提交 `436cc739234061c092cedd106432bbbfa3c2d645`、隔离应用副本身份和进程生命周期来自原公共准备记录及根报告，没有重新读取二进制、冻结树、认证、配置或数据库。

原目录共 31 个文件：13 个 JSON、17 个 JPEG、1 个 PNG。逐文件真实字节数与 SHA-256、解析、图片解码和六类固定凭据形状扫描见 [安全审计 JSON](validation/gui-claude-png-436cc739/archive-audit.json)。JSON 严格解析通过，拒绝重复键和非有限常量；所有 JPEG 均为实际 JPEG、1229×768，校验与完整解码通过。JSON 原序列化文本、独立解码字符串、JPEG／PNG 原字节、根报告及新增两档案的固定形状扫描均为 0 命中。没有对截图执行 OCR 或独立视觉语义检查；原字节扫描不证明像素内没有凭据。

公开 PNG 夹具实际为 248 字节、SHA-256 `6e6f0c4608c8d97790950c80e966369767f0d6c7f043b65a16e8ec206000695d`，96×96、RGB。独立解码检查全部 9216 个像素：左半 4608 个像素均为红色 `(255,0,0)`，右半 4608 个均为蓝色 `(0,0,255)`；不存在透明像素或其他颜色。这是对合成夹具的实际像素核对，不是截图 OCR 或模型重演。

最终 `28` 记录保留 3 个 Completed 执行代、3 条 acknowledged 消息，receipt_kind 均为 native_protocol；全部原生会话为 `273ef03b-aada-4921-8d78-9a387b61aa0c`。三代 current_input.turn_id、消息 UUID、终态 turn_id 逐项相同，submission_generation 与实际代一致；结束会话、结果字节数及摘要均与对应代一致，correct_image_colors 均为 true。结果分别为 43／50／51 字节，SHA-256 依次为 `59bdfd4e0e1411d3696a1f2e85a10551c471de8421acefbac2e72ccc60b91837`、`858daf5095b0a13d5141a6431b40fbb6b092e26c8b1c3d9f084d17c2c1687227`、`27ba55336dd01df0a534bbb6eb6118f23927394133ec11e6c5ca06412c58f569`。颜色布尔及输出摘要来自公共结果投影；本代理没有重读私有模型输出或重新生成答复。

第一条消息 input_variants 为 `[Text, LocalImage]`，后两条均为 `[Text]`。所有公开任务快照仅记录同一个附件：image/png、248 字节、摘要与原夹具一致，filename_hash_matches 和 under_current_attachment_store 均为 true。应用附件存储的实际文件字节和路径未由本代理读取；核对范围是原公开投影与夹具内容一致，不把私有文件独立验证计入本审计。

第 2／3 代连接就绪的 `08`／`21` 记录仅 queued，分别仍为 1／2 条消息，没有 current_input 或终态；旧图片输入没有因就绪记录而增加投递。`11`／`24` 的运行中接收确认对应后续新 Text 输入及最终同一 current_input。正常应用重启前的 `12` 与恢复后的 `18`，以及最后完成的 `25` 与退出后的 `28`，其 task、generations、messages、attachments 逐对象一致。已完成旧代及附件引用均保持，queued／ACK 自身没有被算作完成。

根报告全部 12 个 64 位摘要引用均匹配本目录实际文件 SHA 或明确公开摘要；8 个夹具／JSON 文件引用逐文件对应核实，本地链接均存在。source23 报告仅检查链接路径存在，没有重读其证据或扩大验收范围。本轮没有发现错误链接、错误 SHA 或上述任务身份冲突。

重启准备记录注明旧进程均不存在、退出码未观察；最后应用 PID38245 已不存在及 producer 已关闭属于根代理现场来源，本代理没有再次检查进程，也不据此声明原生 exit0。`29` 记录明确仅移除两个本次 picker alias、保留项目合成夹具、没有修改原生认证／配置／任务历史；本审计只核对该公共清理记录，未访问清理路径。

本轮只新增本报告和安全审计 JSON；原 00–29、PNG 夹具及根报告字节前后不变。没有改旧 Claude `65` 档案、Grok 目录、产品源码或文案，没有执行 CLI、Cargo、GUI、网络、模型、Git 修改或私有数据库查询。无需本地化变更。此审计不替代图片 wire 重新验证、其他格式、活跃进程重新关联、技能与图片混合、父子 GUI、普通 PTY、真实输入法、中英文全链布局、其他平台、SSH/tmux 或 Goal 的剩余验收。
