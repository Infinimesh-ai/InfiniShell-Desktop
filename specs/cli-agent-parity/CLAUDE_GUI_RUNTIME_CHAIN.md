# Claude 真实 GUI 任务链的阶段证据

2026-09-17：根代理在 source6 私有应用副本中，通过真实 API 执行了同一 Claude 任务的六代原生执行，随后以主二进制固定的 source8 副本重启应用、显式继续历史会话并完成第七代，再完成中文第八代审批拒绝与追加指令。英文及简体中文的权限折叠界面有阶段截图，debug 语言资源动态加载的边界另列。第一代精确交互夹具失败，第五代取消失败；第六代后续指令完成不能替代取消成功。source8 恢复属于新进程继续历史会话，未验证活跃进程重新关联；完整双语功能矩阵和最终同提交跨平台验收仍未通过。

本次归档代理仅读取根代理指定的状态、创建 manifest 和 SQLite 原生回执安全投影，没有操作 GUI、模型、CLI、Cargo 或 Git，也没有读取认证/API 文件或其他私人日志。

## source6 应用与来源

私有应用为 `InfiniShellParityQueue2.app`，bundle ID 为 `dev.infinishell.InfiniShellParityE473Claude`。创建 manifest 记录的重签主二进制 SHA-256 为 `26ac0170f95c2714e054f26c497cf40016b4684be3233d3a0b647877bd7e68d6`，来源主二进制 SHA-256 为 `091fe9364cec79601d539fe436d24d4a375b7928548cfde8d9e343e31fb61d4a`。此处是 manifest 中主二进制摘要，归档代理未再次读取或运行它，也不将其称为整个应用目录摘要。

manifest 的 `source_snapshot` 固定了 source6 的 38 个文件，其基准提交为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，来源清单为 `infinishell-cli-parity-official-6-inputs.json`。manifest 自身的 `commit` 字段另记为 `a245c639d307ababbc9b26e604e71318b8ee9522`，`worktree_dirty=true`；两字段分别原样投影，不能据此认定含实际修改的干净提交。manifest 的 `models_executed=0` 是创建时静态值，不能用它否认随后根代理进行的真实 GUI API 执行。

[安全审计投影](validation/claude-gui-runtime-chain-source6.audit.json) 保存完整源快照的路径与摘要清单、稳定任务 ID、代数、原生终态、必要标记和结果摘要。任务 ID 为 `90319eee-f61f-4ecf-bcfd-beca68d58d59`，六代均关联原生会话 `a815d581-b7ac-4a42-80bd-8c7f22ed397c`。状态观察时间为 `2026-09-17T12:48:14.284095+00:00`，数据库投影明确以只读方式获得。

## 六代执行的实际结果

| 代数 | 实际结果 | 验收判断 |
| --- | --- | --- |
| 1 | 原生 `Completed`；结果为 1,019 UTF-8 字节，SHA-256 `2dae985cab4ae80290a19ac1d9d2ff80d5342b96c0d20bf0eb3efe785c0e2722`。模型拒绝夹具中的伪造策略指令；`GUI_ONE` 仅出现在解释正文中。 | `result != GUI_ONE`，夹具失败；不因原生完成或正文包含标记算通过。长正文留在私有证据，不归档。 |
| 2 | 原生 `Completed`，精确结果 `GUI_TWO`，7 UTF-8 字节，SHA-256 `48cc94d6f0952a8fd3cc5e724352f1341feb3fac60f4e9b0f752be07eac0729b`。 | 此次精确交互通过；不能补成第一代夹具通过。 |
| 3 | 根代理实际允许 `Edit`，追加消息折入活跃执行；保存的合并记录为 `Completed`。真实结果同时包含 `GUI_ALLOWED_DONE` 与 `GUI_QUEUED_3`，30 UTF-8 字节，SHA-256 `456a6f4f354171baeded318fff1c92538cc07b9aa915b892656c2d8f32e51dbf`。 | 审批允许、追加消息原生接收、实际合并执行和两个结果标记有证据；文件证明见下文。 |
| 4 | 根代理实际拒绝 `Edit`，原生 `Completed`，精确结果 `GUI_DENY_CONFIRMED`，18 UTF-8 字节，SHA-256 `337580cc941c7bd2067de5909d528b77bdd766769ab6d60da32d90f02910c241`。 | 拒绝后的正常完成与精确标记有证据，文件未被该次编辑修改。 |
| 5 | 根代理发出取消请求，实际保存为 `failed`；原生 outcome 为 `Failed`，诊断为 `[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use`。结果为空，另有 J 指令仍待执行。 | 取消失败；不能称为 `Cancelled`，不能计取消成功或同轮合并批次取消通过。 |
| 6 | J 指令随后独立执行，原生 `Completed`，结果包含 `GUI_CANCEL_BATCH_QUEUED`，85 UTF-8 字节，SHA-256 `b0f270bd4e44226dfbf842a545bd9f3e4f471f33bb0ae2350e9342b81d3f15d9`。 | 后续指令实际完成；它没有加入第五代，不能替代第五代取消证据。 |

第三代活跃执行输入 ID 为 `1ca238c3-9659-4bb2-8e18-1ffe08e7a530`，加入的消息 ID 为 `5623d8b5-10fb-45e1-a014-13854a498a5d`，`submission_generation=3`，保存的 outcome 为 `Completed`。第四至第六代配置继续保留这条第三代历史记录，不能将其再计为新的合并。

第五代活跃输入为 `1bc61f7c-d813-40ea-a93c-590911942f72`。J 的输入 ID 为 `c7c888ff-998e-4f1c-b0a0-f537db643956`，在第五代接收并保存 `submission_generation=5`；它未出现第五代 `InputJoined`，随后成为第六代真实终态的 `turn_id`。接收代数与实际后续执行代数不同，证据投影分别保留，不能从第五代 ACK 推断同轮执行。

## 原生接收与文件证明

[SQLite 原生回执安全投影](validation/claude-gui-runtime-chain-source6.native-receipts.json) 按根代理提供的原字节归档，七个不同消息 ID 均为 `state=acknowledged`、`receipt_kind=native_protocol`。七条消息对应五条首执行输入、第三代追加输入和第五代待处理 J；实际原生执行终态共六代，其中五代 `Completed`、一代 `Failed`，只有第三代有本代新增合并记录。ACK 只证明原生接收，执行完成仍须逐项检查终态。

根代理观察到第三代批准编辑并落地文件效果。指定状态投影中的终态文件为 12 字节，SHA-256 为 `acfa758900501297030a7b3a7d376dc34455046b045cb6c7582f10f87cdafcbf`，`unchanged_after_deny_and_cancel=true`。归档保存此文件证明与根代理的允许/拒绝操作观察，未读取或复制私人项目文件。此状态文件没有单独给出第三代编辑前后的两个文件副本，不能将本次归档描述为重新执行文件验证。

## 归档完整性与隐私

| 来源／产物 | SHA-256 | 保存方式 |
| --- | --- | --- |
| 私有 `gui-lifecycle-state-1.json` | `b897a1aa597b08e56449a29b0120ead76fdd008f57d8acf37ddeb729674458f0` | 仅保存摘要与安全投影；不复制长结果或原终态整树。 |
| 私有创建 `manifest.json` | `080383b7064496fd19d5c0647a0f04260923fb1f16d94a60012da4ccddca7177` | 仅按应用、二进制和源快照字段白名单投影；不复制 environment、home、配置或日志路径。 |
| 原生回执安全投影 | `0cc4b12c29505c3a168561cf6449fd28e6962502ce382aeb3e8d3241eba5e61a` | 原字节副本，七条实际 `native_protocol` 回执。 |
| 新安全审计投影 | `7eda21d153a5df1bdae52cbe23b368ae42711f71709388c85b84eefcaaf9abeb` | 明确变换，不称原字节私有证据。 |

公开产物在内存中检查提供方密钥、Bearer token、凭据赋值、私有 API 端点与 API 环境文件路径五类定向签名，均零命中。只保留方法与计数，不输出匹配全文；定向零命中不能作为通用秘密检测声明。原始私有 manifest、配置、环境和第一代长正文不归档。

## official8 中间门禁单独归档

source6 真实 GUI 执行和 official8 中间门禁使用不同冻结输入，分别保留。official8 的 [输入](validation/macos-official-8-inputs.json)、[Rust 门禁](validation/macos-official-8-gates.json)、[Python 门禁](validation/macos-official-8-python.json) 与 [构建](validation/macos-official-8-bundle.json) 均按 `/tmp` 原文件字节归档。

official8 固定 60 文件，基准提交仍为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，工作区为 dirty。记录中的 `cargo check -p warp`、`cargo test -p warp --lib i18n::tests`、受影响模块 nextest、七个 Python 测试模块和 macOS bundle 均退出 0，门禁报告源码清单未变化。构建主二进制 SHA-256 为 `6e1801d768bdcb3e4ae9e2de2dba2137386631294baa79c2e262efc29f8abb78`，不能用它替换 source6 GUI 副本的二进制身份。

[official8 归档审计](validation/macos-official-8-archive-audit.json) 保存四份原字节文件摘要和清单关联核验，SHA-256 为 `73691cc5ac692c1e5acb9ab32b812b2f6f080b539267b373ccd707a777b531dd`。根代理另外确认深层严格 codesign 验证退出 0；审计明确标为根代理报告，本归档代理没有执行 codesign。四份源 JSON 与新审计的定向凭据签名扫描零命中。

## 仍未满足的验收

第一代精确交互夹具和第五代取消必须保留失败事实。source6 六代执行的原始阶段证据不包含应用重启；下面独立追加 source8 的实际恢复证据，不能改写 source6 的二进制身份或失败结果。

完整英文与简体中文 GUI 功能矩阵、其他 CLI 的完整新建到恢复链路、Linux／Windows／SSH／tmux 验证，以及包含实际修改的同一干净提交门禁仍由根代理推进。本文只新增阶段证据文档与安全归档，没有修改产品文案，无需本地化变更。

## source8 应用重启与权限布局的独立证据

根代理断开并退出 source6 应用后，以 `InfiniShellParityRecovery3.app` 的主二进制固定副本读取原任务数据库。source8 的来源主二进制 SHA-256 为 `6e1801d768bdcb3e4ae9e2de2dba2137386631294baa79c2e262efc29f8abb78`，重签主二进制 SHA-256 为 `678379ab30db5de9bb46f2e7fdd26fefd87d6b560c4f12242eb19e3fc17c75f6`；这两个摘要来自私有创建 manifest 的安全白名单投影，归档代理未运行或再次读取二进制。[source8 manifest 安全投影](validation/claude-gui-runtime-chain-source8/manifest.safe.json) 不含环境、认证、配置或私人日志。

根代理随后核对，这次普通 debug 构建未启用 `rust-embed/debug-embed`，语言资源会从编译时的资源路径动态读取。创建 manifest 中 `immutable_private_bundle=true` 的原值因此只作来源声明，不能扩展为 FTL 或所有运行时资源均不可变。截图 01–13 在 source8 加载相应语言、source9 冻结之前或仍使用旧中文缓存时取得；后续英文 PID 64825 在 source9 冻结后重启，英文资源包含 source9 的新 Grok 摘要，不能称其全部资源仍为 source8，也不能把新摘要出现计为 Grok 产品入口已验证。该资源边界来自根代理检查，归档代理没有操作 GUI 或构建来补验。

source8 关联的来源清单 SHA-256 为 `5302b0dee495aa82b081e7d08bcaa7a8ba12416fe1008ac3083e93444e2c0af0`，即上面的 official8 原字节输入归档。归档核对 60 个唯一相对路径和有效 SHA-256，清单内容与 official8 bundle 中嵌入的来源快照完全一致，来源 worker 摘要与应用 manifest 一致。此清单的基准提交仍为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，构建前后 dirty；没有将后续工作区重新散列结果、干净提交或跨平台验收附会到这次 GUI 执行。

[重启后显式输入前的安全状态](validation/claude-gui-runtime-chain-source8/restart-before-new-input.json) 观察时间为 `2026-09-17T12:53:03.420661+00:00`：任务仍为 `90319eee-f61f-4ecf-bcfd-beca68d58d59`，原生会话仍为 `a815d581-b7ac-4a42-80bd-8c7f22ed397c`，当前第七代处于 `queued`；消息数为 7，本代无结果或完成证据。根代理投影明确记录未观察到自动重投。此处没有仅凭历史消息存在就显示新一代完成。

[显式新输入后的安全状态](validation/claude-gui-runtime-chain-source8/restart-after-new-input.json) 观察时间为 `2026-09-17T12:53:22.982493+00:00`：第七代原生 `TurnFinished::Completed`，结果精确为 `GUI_RESTART_RESUMED`。新消息 `66713b63-5863-4846-9e57-9f18cd2a9acc` 保存为 `acknowledged`、`receipt_kind=native_protocol`，历史七条回执字段逐项保持不变，消息数增为 8。模式明确为 `new_process_continue_native_history`，`active_process_reattachment_verified=false`；它证明显式继续历史会话的实际完成，不能算活跃进程重新关联。未自动重投来自根代理状态投影和仅增加一个显式消息的证据，归档代理没有读取整段原生流来另作全量重投审计。

固定策略 profile 摘要为 `ea736674ac91f01ae4b9adf3249e82918b074c0abc3d64286183f449d92a9a75`。根代理从实际存在的 `config_json.claude_profile` 生成相同性和摘要；第八代安全投影另明确 `profile_field_present=true`。没有把两个缺失字段比较相等当作策略继承验证。

## source8 中文拒绝、追加与配置拒绝

[第八代拒绝及追加结果](validation/claude-gui-runtime-chain-source8/zh-deny-queued-after.json) 观察时间为 `2026-09-17T13:09:52.643192+00:00`：第八代原生 `Completed`，精确结果为 `GUI_ZH_DENIED\n\nGUI_ZH_QUEUED`。A 消息 `0a69b7e5-15a7-49f5-951b-a50b21ee1f63` 与追加 J 消息 `9a0d3871-fef3-459c-be9c-198a05faf175` 都是本代 `acknowledged`、`native_protocol`；实际完成证据的 `turn_id` 为 A。根代理记录拒绝后文件保持 `GUI_ALLOWED\n`，安全投影中的文件 SHA-256 仍为 `acfa758900501297030a7b3a7d376dc34455046b045cb6c7582f10f87cdafcbf`，profile 与第七代相同。归档代理没有读取私人项目文件。

两条同代原生 ACK 和双结果标记已核对。根代理另从实际配置追加 [合并输入安全投影](validation/claude-gui-runtime-chain-source8/zh-deny-queued-joined-inputs.json)：本代 J `9a0d3871-fef3-459c-be9c-198a05faf175` 明确关联 A `0a69b7e5-15a7-49f5-951b-a50b21ee1f63`，`submission_generation=8`、`outcome=Completed`。投影同时保留第三代历史合并，归档只将第八代这一条算作新增。它证明实际保存的本代合并关系与完成结果，没有提供原生结果完整批次 UUID 数组，不能称本次归档独立审计了全量原生协议 ledger。[合并回执安全投影](validation/claude-gui-runtime-chain-source8/native-receipts.json) 保存同一主任务的十个唯一原生 ACK；十条输入接收不等同十次原生执行。

[中文配置拒绝状态](validation/claude-gui-runtime-chain-source8/zh-config-rejection.json) 属于另一个新任务 `673415dc-a9ff-468b-8b80-50f3b180534c`：第一代在配置 preflight 被拒绝，状态为 `failed`，结果为完整的中文权限验证错误，原生会话 ID 为 `null`。数据库仍有一条已持久化的输入 envelope `7a5435ad-ad3f-446f-aa4f-0d6c561ca7bd`，状态为 `cancelled`、`receipt_kind=null`；不能写成零消息，也不能将其计入原生 ACK、模型回合或完成。界面保留草稿，同时显示通用“指令是否送达尚不确定”提示；此提示不改变投影所示的没有原生接收确认事实。

[英文配置拒绝状态](validation/claude-gui-runtime-chain-source8/en-config-rejection.json) 是独立新任务 `27ec3bee-4987-4003-9aa9-9ec6d03f48ca`：第一代同样 `failed`，原生会话 ID 为 `null`，只有一条持久化 `cancelled` envelope `f5feede7-5ba0-426b-a371-ce51ff4cecb0`，`receipt_kind=null`。英文权限错误完整可见，正文没有作为输入夹具目标的 `GUI_REJECT_SHOULD_NOT_RUN`。该次使用 source8 主二进制及动态加载的 source9 英文资源；只证明对应错误布局和拒绝状态，不计模型回合、Grok 入口或最终同提交验收。对应代码消息键为 `cli-agent-claude-file-policy-unverified`；旧 source8 FTL 内容未独立逐字节比对，不能声称 source8／source9 文本精确相同已经验证。

## source8 截图与归档边界

14 张截图分别经过 `view_image` 逐张查看及本机静态 Swift Vision OCR，识别语言为 `en-US` 和 `zh-Hans`，accurate、关闭语言修正。OCR 只扫描存量图片，没有操作 UI、访问网络或调用模型，全文未输出或归档。[OCR 方法与计数](validation/claude-gui-runtime-chain-source8/privacy-ocr-scan.json) 与 [JSON 扫描方法与计数](validation/claude-gui-runtime-chain-source8/privacy-json-scan.json) 对提供方密钥、Bearer token、凭据赋值、私有 API 端点、API 环境文件路径、JWT 和私钥七类定向签名均零命中。

| 截图 | 实际可见内容与检查边界 |
| --- | --- |
| [01 英文重载列表](validation/claude-gui-runtime-chain-source8/screenshots/01-en-reloaded-task-list.jpg)、[06 中文重载列表](validation/claude-gui-runtime-chain-source8/screenshots/06-zh-reloaded-task-list.jpg) | 两种语言的列表标题、按钮、CLI 版本和历史状态可见；不是三款 CLI 的实际任务链通过证明。 |
| [02 英文折叠](validation/claude-gui-runtime-chain-source8/screenshots/02-en-permissions-collapsed.jpg)、[03 英文展开](validation/claude-gui-runtime-chain-source8/screenshots/03-en-permissions-expanded.jpg)、[07 中文折叠](validation/claude-gui-runtime-chain-source8/screenshots/07-zh-permissions-collapsed.jpg)、[08 中文展开](validation/claude-gui-runtime-chain-source8/screenshots/08-zh-permissions-expanded.jpg) | 固定策略说明、显示／收起权限详情按钮完整，长权限 JSON 随容器滚动；没有以滚动视口之外内容不可见算整个文档截断。 |
| [04 英文历史失败](validation/claude-gui-runtime-chain-source8/screenshots/04-en-restored-failure-and-result.jpg)、[05 英文新输入完成](validation/claude-gui-runtime-chain-source8/screenshots/05-en-restart-new-input-completed.jpg)、[09 中文恢复结果](validation/claude-gui-runtime-chain-source8/screenshots/09-zh-restored-result.jpg) | 历史第 5 代仍为 Failed；第 7 代新结果与第 6 代历史结果分别显示，没有覆盖旧失败。 |
| [10 中文审批](validation/claude-gui-runtime-chain-source8/screenshots/10-zh-approval-request.jpg)、[11 中文排队帮助](validation/claude-gui-runtime-chain-source8/screenshots/11-zh-queue-help.jpg)、[12 中文拒绝与追加结果](validation/claude-gui-runtime-chain-source8/screenshots/12-zh-denial-and-queued-result.jpg) | 真实 Edit 请求、仅允许／拒绝本次按钮、活跃追加帮助和最终双标记完整可见；不证明原生批次取消。 |
| [13 中文配置拒绝](validation/claude-gui-runtime-chain-source8/screenshots/13-zh-config-rejection.jpg) | 中文权限错误完整可读，失败状态与保留草稿可见；没有原生会话或原生 ACK。 |
| [14 英文配置拒绝](validation/claude-gui-runtime-chain-source8/screenshots/14-en-config-rejection.jpg) | 英文权限错误完整可读，原生会话等待确认，草稿保留；主二进制为 source8，英文资源来自 source9，不称整个 bundle 资源固定。 |

[source8 独立归档审计](validation/claude-gui-runtime-chain-source8/audit.json) 保存六份安全 JSON 和每张 JPG 的来源映射、字节数及 SHA-256；均为原字节复制，没有转码。私有创建 manifest 只投影必要字段，并保存该次读取的原文件摘要 `4312bf3edc5c8eeb7a9fb7edccd1a0642f2935697c45828112624091a13193cd`，不能把投影称为原字节 manifest。根代理报告断开 CLI、退出 source8 PID 63178 后又以英文重启 PID 64825，最终也退出 PID 64825 并确认进程退出；这些操作注明由根代理执行。manifest 的 PID 原值仍为 64825，不能把创建记录当作最终进程仍在运行，归档代理没有执行退出或补验原生 cleanup。

source8 的显式继续历史会话、所选双语权限布局和中文拒绝与追加有阶段证据；活跃重新关联、历史第 5 代取消成功、完整双语功能矩阵、其他 CLI 任务链和同一含修改的干净提交跨平台门槛仍未通过，不能据此将 Goal 标记完成。
