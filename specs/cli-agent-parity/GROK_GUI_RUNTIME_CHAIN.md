# Grok source11 GUI、原生运行与持久化证据链

这次 macOS GUI 验收通过了 Grok 根任务的新建、两轮交互、Write 审批允许与拒绝、运行中追加排队指令、实际正文后的取消、同连接继续、应用退出与重启、原生历史加载和显式结果回收。最终同一任务有9代历史、9条原生协议确认消息，终态为7次 Completed、2次 Cancelled。该结论仅覆盖 source11 中间脏快照、固定 Grok Build 1.0.30、继承 CLI 设置的根任务；不计 SDK、子任务、父权限上限、图片、技能、其他平台或最终同提交验收，Goal 尚未完成。

任务 ID 为 `cc127ae3-4372-42f6-a8ca-edc635b2e603`，原生会话 ID 为 `01a0afb4-7ecd-70a0-92c0-5374ecae99c2`。归档任务没有交互 GUI、调用模型、运行 Cargo/Git 或读取认证文件，也没有读取大二进制重新计算散列。对原 SQLite 的独立核对使用 `mode=ro`、`PRAGMA query_only=ON`，查询仅限此精确任务及其自输入消息；数据库、环境、原始 GUI 日志、wrapper 和凭据原文件均未归档。

## 构建与语言资源边界

基线提交为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，工作区有既存改动。source11 的67文件 manifest SHA-256 为 `31619a72300d6a38e82ade3d40e2329039cee864e5c0ca38c29ceb38037e504a`；原主应用 SHA-256 为 `8026e131aad657207690144d31e3e5c01b654039c4ed60eb1d5502dc9cb14fbb`，隔离 GUI 应用重新签名后的 SHA-256 为 `b4b42782b3dd4346be77bf1f1549cd37c38f4d97087eee9b8887ab408163a25f`。这些二进制值来自根代理准备 manifest，归档不追认为独立二进制散列复测或最终干净提交。

准备 manifest 记录英文和简体中文完整 FTL 均嵌入二进制、资源不可变；归档独立核对了两份 FTL 源字节和散列。英文为374,280字节、SHA-256 `9442a4b062945cf3f5fbc5160193f4a14ae715ae9d35b7f5d5bc14be55980c3c`，简体中文为361,234字节、SHA-256 `2291d0796862e703a2869e174d38138f014183730d380d56d6d97ce4772a0d98`。只从 manifest 显式提取这些字段及固定能力边界，见[安全元数据](validation/grok-gui-runtime-chain-source11/metadata.json)；未复制完整 manifest 的 environment。

## 真实操作与持久记录

| 执行代 | 操作 | 真实结果与独立核对 |
| --- | --- | --- |
| 1 | 英文新建输入 | Completed，`GUI_GROK_ONE_4` 精确14字节；原消息正文与固定输入一致 |
| 2 | 中文多行第二轮 | Completed，`GUI_GROK_TWO_4` 精确14字节；中文与换行原样持久化 |
| 3 | Write 允许一次 | Completed；实际 `gui-approved.txt` 为20字节 `GUI_GROK_APPROVED_4\n`，正文包含工具前言，共79字节，未压成固定末句 |
| 4 | Write 拒绝一次 | Cancelled；`gui-denied.txt` 不存在 |
| 5 | 运行中追加指令 | 主回合 Completed，结果6,417字节；追加消息原提交代保持5 |
| 6 | 执行原代5追加消息 | Completed，`GUI_GROK_QUEUED_4` 精确17字节；消息与原生回合关联到执行代6，未重写原提交代 |
| 7 | 产生正文后取消 | Cancelled，真实已产生2,011字节正文，保留部分结果 |
| 8 | 同连接取消后继续 | Completed，`GUI_GROK_AFTER_CANCEL_4` 精确23字节，仍使用第一 runtime 与同一原生会话 |
| 9 | 应用重启后加载并显式回收 | Completed；不含答案的中文多行输入回收旧排队标记，结果精确17字节 |

9条消息均为 self `user_input`，持久状态为 Acknowledged、回执为 NativeProtocol。原提交代顺序为 `1,2,3,4,5,5,7,8,9`。独立审计将每代实际 `grok_current_input` 的 message ID、原提交代、runtime ID、原生 turn ID 与原消息及终态证据逐条对应，确认原生 session 一致、终态 output 与完整持久 result 一致，并重新计算正文长度及散列；结构化审计仅保存协议 ID、计数和摘要。原生 stdout/stderr 帧没有被本归档重新读取，因此9条原生确认的独立依据是生产路径的 SQLite 原生回执记录及关联账本，不是本归档独立捕获的9份原始协议帧。

批准文件的 SHA-256 为 `1d59e6eeeca169ccbf0b4524b5143b6ba66f67553322bdbf2b9844e98fc5acbc`。第6代与第9代结果 SHA-256 均为 `ff5c93d97859e84ee713a791c222c590b7f152339caf247a09d19ad6f15c0230`，第8代为 `0928fae86b13b9f65190d1a25f6cbf2e54ab83bc35c2367d6ea83b3749b36c13`。全部9代结果和输入摘要保存在[独立审计](validation/grok-gui-runtime-chain-source11/audit.json)，最终公开投影与原数据库重新投影逐字段一致。

运行中追加采用 Grok 原生后续回合排队语义，不表示当前回合即时改写。原提交代5到执行代6的身份保留，以及取消后继续，分别见[排队原生确认](validation/grok-gui-runtime-chain-source11/sqlite-queue-native-ack.json)、[排队结果](validation/grok-gui-runtime-chain-source11/sqlite-queue-result.json)、[实际取消](validation/grok-gui-runtime-chain-source11/sqlite-actual-cancel.json)与[取消后结果](validation/grok-gui-runtime-chain-source11/sqlite-after-cancel-result.json)。

## 应用退出、重新启动与无自动重投

第一应用 PID 为73939，随后通过 Quit 退出；第二次隔离应用 PID 为76823。第一 runtime 为 `f1958234-3bbc-4621-96ba-6de31f6b3e74`，重启继续产生的新 runtime 为 `7157ebef-0681-48c3-a3bc-cb3d94619306`，原生 session 保持不变。归档时两 PID 均已不存在。

[退出前快照](validation/grok-gui-runtime-chain-source11/sqlite-stopped-before-restart.json)与[重启后选择任务前快照](validation/grok-gui-runtime-chain-source11/sqlite-restarted-before-select.json)均保留8条消息。第9代原生历史加载的[握手快照](validation/grok-gui-runtime-chain-source11/sqlite-resume-handshake-no-input.json)和[显式输入前快照](validation/grok-gui-runtime-chain-source11/sqlite-resume-ready-before-explicit-input.json)仍只有8条消息，旧8代历史逐字段保留；新代 Queued、没有终态或结果。只有随后显式发送不含旧标记答案的中文多行输入，才增加第9条消息并回收旧标记，见[最终结果](validation/grok-gui-runtime-chain-source11/sqlite-history-memory-result.json)。这证明本次加载没有自动重投旧输入，并将加载历史与执行新的恢复输入分开。

两次原生退出均为 `stdio_closed`、exit code 0，cleanup confirmed。归档独立读取此两个已知 runtime 的原始 exit 与 macOS cleanup 收据，只公开固定字段与 SHA-256，核对它们与[第一进程脱敏收据](validation/grok-gui-runtime-chain-source11/first-native-cleanup-receipts.json)、[第二进程脱敏收据](validation/grok-gui-runtime-chain-source11/second-native-cleanup-receipts.json)中的来源散列完全一致；两次均有 `job_removed=true`、`resource_cid_destroyed=true`、native wait status 0，execution failed false。

[应用刚退出记录](validation/grok-gui-runtime-chain-source11/app-first-exit.json)的 exit receipt 尚未出现，是此前瞬间状态；[随后清理记录](validation/grok-gui-runtime-chain-source11/app-first-cleanup.json)已确认真实收据。记录中 `native_job_manifest_removed=false` 与 `coalition_receipt_removed=false` 表示取证文件留存，不能解释为活动 job 未清理。实际活动资源的删除由上述 macOS cleanup 收据另行证明。

[最终网络记录](validation/grok-gui-runtime-chain-source11/network-final.json)记录 tunnels stopped 与 auth copy removed 均为 true，30次官方 TLS 连接、8,928,303字节及205次非官方 origin 拒绝，未观察到预算耗尽。归档独立确认代理端口关闭、私有 opaque auth 副本已不存在。TLS 未解密，连接数不等于 HTTP 模型调用数，也不证明费用上限。

## 双语截图与隐私核对

36张截图经本机 Swift Vision accurate OCR，关闭语言修正，使用 `en-US` 与 `zh-Hans`；逐行及去空白拼接文本扫描7类凭据或私有端点模式，全部零命中。未保存 OCR 全文；见[扫描结果](validation/grok-gui-runtime-chain-source11/privacy-scan.json)和[扫描脚本](validation/grok-gui-runtime-chain-source11/privacy-scan.swift)。模式扫描不能替代所有秘密的穷尽识别；另独立目视检查了10张关键画面，未见认证内容。

截图源名虽然曾被描述为 PNG，但魔数和 JFIF 标记证明36张均为 JPEG。归档保留 `.jpg`、原始字节、尺寸和散列，不转码、不裁剪。英文[第1张](validation/grok-gui-runtime-chain-source11/figure-01-en-picker.jpg)与简体中文[第27张](validation/grok-gui-runtime-chain-source11/figure-27-zh-picker-restart.jpg)中 Grok 能力提示均为两行，未见该提示截断或与控件重叠；长面板使用原有滚动区域，此检查不推定其他页面或窗口尺寸的布局通过。

[第2张未提交草稿](validation/grok-gui-runtime-chain-source11/figure-02-en-new-draft.jpg)中 `type_text` 自动化输入丢失下划线和冒号；[第3张](validation/grok-gui-runtime-chain-source11/figure-03-en-draft-corrected.jpg)通过 paste 修正后才启动。第一条实际持久输入与正确原文一致，因此记录为输入工具偏差，不计产品输入失败。[中文多行第二轮](validation/grok-gui-runtime-chain-source11/figure-06-en-chinese-multiline-draft.jpg)、[明确 Write 参数](validation/grok-gui-runtime-chain-source11/figure-11-en-approval-exact.jpg)、[拒绝参数](validation/grok-gui-runtime-chain-source11/figure-15-en-deny-exact.jpg)、[取消前真实正文](validation/grok-gui-runtime-chain-source11/figure-22-en-cancel-live-output.jpg)及[不含答案的恢复草稿](validation/grok-gui-runtime-chain-source11/figure-31-zh-recovery-draft.jpg)均已目视核对。

[第34张](validation/grok-gui-runtime-chain-source11/figure-34-zh-saved-full-result.jpg)同时显示第9代当前结果及保存结果，均为完整 `GUI_GROK_QUEUED_4`，并显示同一原生 session。归档共有36张逐字截图、16份根代理显式脱敏 JSON 的逐字副本、安全元数据、OCR记录和独立审计；截图可见内容仅来自隔离验收的固定或数值测试输入、协议 ID 与测试结果，原始数据库正文及 GUI 日志不作为独立文本产物复制。

本次成功不覆盖 source12/source13 的新增夹具或审查收紧，不追认为此前失败已查明或修复，也不改变 SDK、本地子任务及父权限上限尚未通过的边界。最终包含实际修改的同一提交跨平台门禁仍需另行完成。
