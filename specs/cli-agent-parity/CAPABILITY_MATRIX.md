# CLI 能力矩阵

> **最终状态（2026-09-21）**：Goal 已完成。最终代码 SHA `38a611773b8ee53860f9ab731b2476b9a40d1819` 的 macOS、Linux、Windows 门禁和独立 SSH／tmux 收据均已收口；共同能力已对齐，上游差异及明确降级继续保留。详见 [最终验收](FINAL_20260921_CLI_PARITY.md)。下文阶段性缺口只描述各历史快照。

阶段性收尾与新会话入口：[2026-09-19 续接交接](HANDOFF_20260919_CLI_PARITY.md)。source56 已统一 Claude 固定策略的受支持版本判定，并以官方 2.1.278 完成固定策略恢复、真实父子派发／双向 ACK／结果回收、运行批量取消和待编辑审批取消；原父子失败与验收脚本误判均保留。Grok 1.0.34 P0 生产链、GUI 原生读取审批及本机 SSH／tmux worker已按各自快照通过；最终 Goal 仍未完成。
[自动升级与最新版当前验证](OFFICIAL_42_AUTOUPDATE_VERIFICATION.md)：source50 同源测试程序与签名监督程序已通过 Claude 2.1.278 Latest → Stable 2.1.267、反向 Stable → Latest 及同版本渠道同步；source56 又补齐最新版固定权限、父子和两类取消原生链，并通过启用托管任务 feature 的签名 GUI 核对官方 2.1.278 检出、固定审批入口及双语布局。整个 source56 尚无 GUI 任务运行／重启或 Linux／Windows 复验，不能回填为最终同提交通过。Codex／Grok 已有渠道结果、最新版协议任务链、完整工作区及最终跨平台仍按各自证据边界继续验收。

[source41候选](OFFICIAL_40_RECOVERY_AND_INPUT_GUARDS.md)已通过check、i18n11项、受影响Rust1826项和桌面构建；Grok文件允许／拒绝及待审批取消五阶段通过，冷继续恢复与输入ACK已到达，但测试代理连接预算耗尽，整体仍失败。普通Grok复合启动工具栏、英文与简体中文完整提示、两种语言多行草稿显式复制往返均通过；自动PTY输入仍为明确降级。source41a另修父子自动结果的时序及跨代恢复，已通过 check、i18n 11 项及受影响 Rust 1834 项；尚未运行该快照的原生链。新增[消费者自动升级与开发最新版对齐](CLI_AUTOUPDATE.md)属于当前Goal；历史固定版本证据保留，不能替代最新版或最终同提交三平台、完整工作区及SSH／tmux验收。

[内部维护兼容修复](GROK_INTERNAL_MAINTENANCE_COMPATIBILITY_FIX.md)、[缓存认证握手修复](GROK_POLICY_AUTH_HANDSHAKE_FIX.md)及[Grok 0.1.1 插件升级修复](GROK_PLUGIN_011_MIGRATION_FIX.md)已合入新的 source32 候选快照：父提交0059，119项输入、18处变更，其他101项逐父Git对象保持一致，新增38项Rust回归包含8项tokio异步测试。诊断测试同步为合法内部维护帧无业务效果且不消耗pending，未知ID、错误帧及夹带会话身份仍拒绝；原source31快照和失败记录保持不变。source32独立门禁已真实全部通过：check退出0／140.859秒、i18n11／320.445秒、Rust1458／25.620秒、Python15组453项无跳过；38项新增Rust（含8项tokio）逐项各1PASS，119项输入与850624232字节测试库前后保持一致。main已真实构建退出0／329.100秒，严格签名退出0、完整中英文资源逐字节嵌入，测试库身份保持一致。[source32原生部分复验](OFFICIAL_32_NATIVE_PARTIAL_VERIFICATION.md)：SDK12（29.281秒）与policy4（13.074秒）均Rust101／runner1，原失败保持；SDK12已实际走通现代发现、tools/list和一次inspect业务调用，2次原生审批允许、transport正常结束及清理收据通过，但六种carrier均无原生S/P/T，身份关联验收未通过，子任务和父权限门禁保持关闭。policy4实际仅发3RPC，initialize及cached_token authenticate成功，session/new关联rpc_error，未完成两阶段接口验收；失败取消的null退出码投影缺口另行修复，不补成退出0。认证副本／连接／目录已清理，配置与策略夹具保持不变，不能将14RPC预算写成已发出计数。原生六阶段插件迁移尚未执行；另发现0.1.1配方的通知仍上报0.1.0，后续候选须同步脚本版本及严格历史夹具，并独立验证真实安装后的通知。[SSH／tmux执行计划](SSH_TMUX_FINAL_ACCEPTANCE_EXECUTION_PLAN.md)仅盘点可复用入口和待验边界，没有新增真实SSH链通过。

更新日期：2026-09-19（北京时间）。此表分别记录实现路径、真实接口证据与最终产品验收。Goal 未完成；历史任务链回归版本为 Codex `0.147.0`、Claude `2.1.273`、Grok `1.0.30`；新正式版 `0.155.1`、`2.1.278`、`1.0.34` 按上述升级、协议与任务验收分别记录，不能互相替代。最新实际已推送提交为 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`；下文此前快照结论仅代表各自受测范围。

[source26 门禁](OFFICIAL_26_LOCAL_GATES.md)：同实际436提交，check、i18n11、Rust1393、Python410、main构建、严格签名及完整双语资源嵌入通过。[原生部分验收](OFFICIAL_26_NATIVE_PARTIAL_VERIFICATION.md)：Claude待Edit取消2和继续通过；Grok SDK9及无输入policy1整体失败，子任务仍关闭。[真实GUI](OFFICIAL_26_GUI_PARTIAL_VERIFICATION.md)：三图标与安装检测、目标控件双语布局、Claude同原生ID两轮、允许／拒绝、追加随后执行、取消、历史继续、应用重启记录恢复及不自动重放已核对，重启后新输入的最终记忆结果也已回收。CI12同head的两平台Python失败、原生准备跳过，Windows部分Rust及check已通过，不能据此计整次通过；全工作区未跑。source27诊断候选的独立本地门禁已通过check、i18n11、Rust1410及Python426，不回填source26失败。

检查点 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 已实际提交、推送并核对远端一致。该提交干净验证树的 [source23 门禁](OFFICIAL_23_LOCAL_GATES.md)通过 check、i18n11、Rust1301、Python363、main构建、严格签名及完整双语资源嵌入。[source23 在线验收](OFFICIAL_23_NATIVE_LIVE.md)：Claude PNG3三输入、两个正常退出及同原生会话历史继续通过；Grok SDK8整体失败，但现代2026-07-28 MCP发现、工具列表及原生ready实际已观察；Claude等待Edit取消1整体失败，原生interrupt ACK、审批撤销、错误result及cancelled生命周期均已观察，未取得应用可信取消聚合。[CI11](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124) 同SHA：Linux的Python超时夹具失败、Rust未执行；Windows的Python与Rust定向步骤失败，check和SSH worker构建通过；真实日志已取得，分别定位为纯测试的路径分隔符断言和私有JSON反斜线编码断言，原生准备因前置失败跳过、证据上传缺失为继发失败。两平台均不能计完整通过。后续修复不在该提交：[source24候选门禁](OFFICIAL_24_LOCAL_GATES.md)已冻结111路径并通过check、i18n11、Rust1378、Python410；包含Claude真实取消枚举补丁、Grok测试夹具对生产传输真实错误的诊断、标题栏普通终端启动及本地安装路径回退。只读预检已预编译并核对唯一ignored入口，准备标记已解除，原生接口尚未执行；该候选验证树dirty、未提交，不计新的同提交／跨平台／GUI通过。同一source24快照另行补跑实际`terminal::model::session::test`模块15/15通过，含新增4项，主1378计数不回填。source25已以同一111路径冻结Windows两处测试修复与两平台正确Session筛选，[正式候选门禁](OFFICIAL_25_LOCAL_GATES.md)已通过check、i18n11、Rust1393、Python410；该候选修改后来形成436提交，source26与CI12分别独立复验，不回填当时候选。

source21 的87路径中间快照已通过 check、i18n11、定向1270、Python363；没有 source21 main 或真实CLI。source20构建与严格签名保留历史。随后发现旧独立Claude AgentDriver 的绕过与全局写入，已关闭该旧入口并移除相关实现；source22 的93路径新快照通过 check、i18n11、定向1301、Python363。下表中间证据不代表最终实际修改提交、所有平台或整个能力通过。完整来源、失败与历史入口见 [验证报告](VALIDATION_REPORT.md)。

[source28本地候选门禁](OFFICIAL_28_LOCAL_GATES.md)已覆盖11路径修改，check、i18n11、Rust1410、Python426真实通过，含CI12的临时目录夹具、SQLite关闭与平台哈希断言修复；代码已形成实际e687提交。[CI13第三窗口](CI_13_THIRD_WINDOW_VERIFICATION.md)在e687的Linux／Windows两个job均实际completed/success，所选门禁通过；full_workspace_tests=false、GUI编译步骤跳过，没有完整工作区或在线三方链结论，精确测试计数未从日志取得。[source27原生诊断](OFFICIAL_27_NATIVE_DIAGNOSTIC.md)的policy2、SDK10仍为FAILED；[所有权复核](GROK_SKILLS_RELOAD_OWNERSHIP_REVIEW.md)已找到候选CLI内部watcher发送 `skills-reload`，应用没有发出该请求，固定native内层形状仍待核验。空MCP目录的四路径受限修复已形成实际0059；[source29门禁](OFFICIAL_29_LOCAL_GATES.md)check、i18n11、Rust1420、Python436通过，[实际0059的policy3](OFFICIAL_30_GROK_POLICY_VERIFICATION.md)整体FAILED：初始化及空目录兼容成功，New关联RPC错误，继续／完整退出边界未完成；不推断认证或权限原因。同0059平台验收仍未执行。

[source31本地候选门禁](OFFICIAL_31_LOCAL_GATES.md)通过check、i18n11、Rust1430、Python443，111输入／7修改前后稳定，新增SDK7及UI3逐项各一次PASS。内容为安全成功形状布尔诊断与中英文可读附件／技能历史预览；main构建及严格签名已通过；[双语附件历史布局与复制](OFFICIAL_31_GUI_PREVIEW_VERIFICATION.md)实测通过，无新模型输入，历史三个表完整行哈希不变；真实技能历史布局仍待验。后续四路径探针cached_token握手修复及Grok插件0.1.1完整旧配方迁移尚未Cargo／原生验收，不追认为source31通过。

[真实SDK11与后续兼容修复](GROK_INTERNAL_MAINTENANCE_COMPATIBILITY_FIX.md)：source31唯一一次在线探针仍FAILED（Rust101／runner1），但严格成功形状及私有账本对应校验真实为true，认证副本与隧道已清理。确认固定skills-reload双层封闭成功结构后，后续两路径生产候选仅作内部维护空操作，不消费数值pending，不发出生命周期或提升子任务／权限门禁；新增6项Rust回归未执行，不将原失败追认为修复后成功。

## 身份、普通终端与上下文

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 命令／管理命令、安装及版本识别 | 专用规则及受限版本探测；最终提交回归待验 | 同类专用规则，安装不等于登录 | 独立身份、规则及 `.grok/bin` 探测；默认标题栏已有双语 [入口实测](GROK_TITLEBAR_ENTRY.md) |
| 普通 PTY 工具栏与富输入 | macOS 隔离 GUI 两轮多行有 [证据](validation/macos-gui-report.md) | 延迟 Enter 策略；认证后完整普通 PTY 组合待验 | 括号粘贴候选；普通 PTY 完整链待验，托管 GUI 证据另列 |
| 中文输入法／中英多行 | macOS GUI 逐键拼音组合、混排与多行已有证据；候选列表及其他平台待验 | 托管 API／GUI 有多行输入阶段证据；普通 PTY 与三平台待验 | source11 托管 GUI 中英文多行通过；普通 PTY 输入法与三平台待验 |
| 图片 | `localImage` 原生适配器及 macOS GUI 已实际读图；最终提交与其他平台待验 | 实际436的[source26 PNG GUI](OFFICIAL_26_CLAUDE_PNG_GUI_VERIFICATION.md)三轮识色、同原生ID历史继续、App退出重启、SQLite及唯一磁盘附件引用核对通过；[独立归档审计](OFFICIAL_26_CLAUDE_PNG_GUI_ARCHIVE_AUDIT.md)实际解码夹具并核对31份证据。其他格式、最终提交与其他平台仍待验 | 固定 1.0.30 ACP 明确声明 image:false；托管入口拒绝并保留草稿／附件，不能泛化为普通 CLI 没有读图能力 |
| 文件与代码评审 | macOS GUI 文件实际读取及有效评审的路径／行号／内容传递有证据 | 复用上下文；普通 PTY 与完整组合待验 | 复用上下文；完整组合待验 |
| 技能来源与语法 | `$` 与 Agents 来源；新项目发现、选择及原生注入有 macOS GUI 证据 | `/` 与 Claude 来源；source37 普通 PTY 实际选择技能并取得正文标记，托管组合及最终提交仍按各自证据验收 | Grok／Claude 来源，Agents 仅已支持的 Home 来源；source38 原生裸 slash 展开与输入前唯一 canonical 路径已证明；后续候选接入 Inherit 每轮一个显式技能、当前会话唯一路径匹配、不自动授予信任，默认入口组合待验，固定策略禁用技能 |
| 自动升级与渠道选择（实施中） | 正式 latest／预发布 alpha；原生 update 默认只跟正式，应用需解析目标与匹配来源 | 官方默认 latest／延迟 stable；临时渠道覆盖真实双向更新通过且配置字节不变；npm 组织版本策略待验 | 正式 stable／预发布 alpha；原生升级会重写配置，产品事务及恢复待验 |
| 原生模型、推理与登录 | 按实际版本／配置保留差异，不从安装推断已登录 | API 鉴权已可用于实测，不将 Inherit 模式视为固定文件策略 | 官方在线授权及根任务已有成功证据，旧额度失败不再作为当前阻塞 |

## 插件、事件与状态

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 插件安装／更新／禁用／恢复 | 固定 rev4、持久来源及原生信任有中间证据；完整负向组合与最终提交待验 | 固定插件版本及来源修补、禁用和事务恢复有阶段记录；未知版本不放行 | 随附独立插件；真实生产安装、同版本修复、禁用及事务回滚有阶段记录；全平台组合待验 |
| 普通 PTY 完成依据 | Stop／OSC 9 仅候选响应，显示 Unknown；同回合后续有效事件继续处理 | 同样降级 Unknown，不能仅由 Stop 推断聚合完成 | `PermissionDenied` 不表示等待审批；缺可信终态降级 Unknown |
| 托管终态 | 原生 turn 终态与任务结果分别核对 | 原生输入 UUID／执行归属与完整 result；取消要求完整证据聚合 | 完整原生历史与同回合 completion watermark 核对；不从 `end_turn` 单独推断正文完整 |
| 重复／乱序／旧回调 | turn／事件 ID、代次与实例约束，定向回归已有证据 | 保留 prompt／UUID 语义，不混用其他 CLI 身份 | 独立身份与来源，兼容 hooks 及乱序的完整真实负向链待验 |
| Windows hooks | CI10 五项原生注册通过；SessionStart／UserPromptSubmit 两项真实到达 ConPTY，其余三项实效待验 | 无凭据原生边界与事务各按原报告范围；完整模型／GUI 待验 | ACP／插件边界各按原报告范围；完整模型／GUI 待验 |

## 托管任务、权限与恢复

| 能力 | Codex CLI | Claude Code | Grok Build |
| --- | --- | --- | --- |
| 传输 | 官方 app-server JSON-RPC | 双向 stream-json／control | ACP；根任务 leader 与 SDK direct 对照分别记录 |
| 新建／两轮／允许与拒绝 | 完整固定包 [原生适配器](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md)和中间 macOS GUI 有证据 | 实际436的[source26 GUI](OFFICIAL_26_GUI_PARTIAL_VERIFICATION.md)同ID两轮及Write允许／拒绝真实文件效果通过；此前失败保留 | 实际436的[source26 Grok GUI](OFFICIAL_26_GROK_GUI_PARTIAL_VERIFICATION.md)两轮及精确Write允许／拒绝通过；拒绝后原生Cancelled，保留差异 |
| 运行中追加 | 原生 `turn/steer`，必须已有 started；macOS GUI 实际采用追加内容 | 原生可并入当前工具执行或独立下一轮；必须逐 UUID 核对 ACK、合并及完整结果，不宣称 steer | 实际436 source26第5代排队提交、第6代原生ACK与24字节完整结果，提交／执行／原生回合逐ID核对；后续回合排队 |
| 取消 | interrupt ACK 不等于取消；原生 Cancelled 与下一轮完成已有中间证据 | source56 官方2.1.278运行批量取消与待Edit审批取消均通过：原生ACK、终态、完整结果、文件不变、同会话继续及清理逐项核对；首个批量外层脚本误判和source23取消1失败均保留 | source11 GUI 正文输出后真实 Cancelled，随后同连接继续通过；SDK7 清理不等于取消或完成成功 |
| 默认权限及配置副作用 | 已移除固定绕过审批／沙箱；普通子 pane 使用可见原生终端 | 可见子 pane 与托管入口已移除固定绕过；旧独立 AgentDriver 已关闭并移除全局信任／onboarding 改写；source37 普通 PTY 完整链六次用户全局配置快照保持不变；托管继续按独立证据验收 | Inherit 根任务保留 CLI 配置；固定只读／文件策略限定应用配置和逐次审批，非 OS 沙箱。source38 只读两输入冷恢复通过，文件策略与子任务待验 |
| 父子派发／双向 ACK／回收 | macOS 中间 GUI 生产协调器有派发、子进度、父追加、原生 ACK 与实际结果回收 | source56 官方2.1.278固定 `ClaudeRestrictedFilesV1` 真实生产链通过：子任务创建、父子同策略、双向原生ACK、3组结果关联、自动结果及双清理收据；旧2.1.273字面量导致的失败保留。签名GUI已核对2.1.278检出和固定审批入口，但未从GUI运行父子任务；最终提交待验 | source34b lease1 注册／inspect／回收通过；source38 child3 在父首输入 ACK 后目录变化被严格拒绝，尚无子任务；固定策略候选可显示入口但整链验收未完成 |
| 权限上限 | 保存创建时父代完整快照，首输入前核对；未知／漂移／跨 CLI 不等价拒绝 | Inherit 不提供可证明的完整规则上限，派发拒绝；source56 由同一支持版本表验证 2.1.273／2.1.278 固定策略继承并拒绝2.1.279，非OS沙箱 | 固定策略保存创建契约并校验目录，未知或变化拒绝；父子工具集必须完全相同，仅本地任务能力可缩减（Files→Read 也不等价）；source38 child3 实际拒绝目录变化，未证明成功继承与派发 |
| SQLite 任务／消息／结果 | 父子、代次、revision、原生 ACK 与结果历史；中间故障注入有证据 | source26文本GUI共8输入／8代（7 Completed、1 Cancelled）；PNG专项另有3输入／3代Completed，重启前后消息、结果及附件引用核对；父子完整GUI仍待验 | 实际436 source26 GUI为9消息／9代（6 Completed、2 Cancelled、1 Disconnected），消息／回合／结果逐ID核对及[公共归档审计](OFFICIAL_26_GROK_GUI_ARCHIVE_AUDIT.md)通过；第7代中断不能追认为完成 |
| 当前连接重新选择 | 复用活跃连接，不启动第二进程 | 同类路径；完整故障恢复待验 | source11 取消后继续复用原连接；历史继续才创建新连接 |
| 历史继续／应用重启 | GUI 完成与活动退出后的原 ID 显式继续、无重复执行有证据 | 实际436的source26 GUI正常退出重开、任务与消息恢复、无自动重投、原ID新进程历史继续及明确输入记忆结果回收通过；活跃重新关联及父子GUI仍待验 | 实际436的source26 GUI重启后72字节记忆结果与第9代独立历史继续71字节结果均回收，同原生ID、旧输入不重放；第7代仍Disconnected，完整故障链未通过 |
| 已绑定 PTY 未确认结果 | 保存 Unconfirmed，占用原会话／代次，不能生成最终成功结果 | 共用约束，与托管 Completed 独立 | 共用存储约束不等于子任务已开放 |
| 真正进程清理 | macOS 资源域的真实 Codex 工具／空闲崩溃有独立证据，最终提交复验待验 | source23 PNG3两代stdio／0与资源域清理有收据；Edit取消1stdio／1、清理true保留失败，真实异常工具崩溃待验 | source11／13 根任务正常 stdio／0 清理有证据；SDK7 exit=null、原始 wait=9、cleanup=true，不能记原生退出0 |

## 产品开放边界与消息契约

`LocalCLIManagedTasks` 同时控制入口与托管启动，默认关闭；开发验收使用显式开关。启动前重新探测受测版本，未验证版本只开放已经证明的模式。普通 PTY 不转换成协议任务，本机安装状态不证明 SSH 远端状态。

Grok官方根任务已有阶段GUI与生产coordinator证据；source34b生产SDK租约真实通过一次inspect与完整结果回收。SDK回调仍无原生S/P/T字段，派发归属依靠独占进程、注册nonce及真实工具与审批租约；旧来源探针失败不追认为成功。source38 子任务父首轮因目录变化被严格拒绝，成功派发、双向消息及回收尚未完成。Claude PNG在实际436的source26 GUI完成三轮识色及App重启恢复；这是独立于source23原生wire探针的产品证据，不代表其他格式或最终提交通过。

消息保存明确的 `receipt_kind`：`native_protocol` 表示原生接收；`application_history` 只表示进入应用历史供正常后续请求读取。两者都不能独立证明模型执行完成。发送未确认记录、创建时父代、历史结果和分页查询保持关联；恢复不能自动重投已经执行的输入。完整实现边界见 [消息交付](OZ_LOCAL_MESSAGE_DELIVERY.md)、[Unconfirmed](UNCONFIRMED_TASK_STATE.md) 和原协调器报告。

## 最终验收仍未满足

| 要求 | 当前边界 |
| --- | --- |
| 三款 CLI 各自完整规定链路 | 中间有成功与失败阶段，最终实际修改提交未完成整链 |
| 普通 PTY 与托管模式分别覆盖 | Claude source37 普通 PTY 13 输入与 PNG 专项 GUI 已有证据；Grok 普通 PTY 完整上下文链、其他格式、子任务及最终提交待验 |
| 负向与故障组合 | 已有定向与部分真实记录，完整插件／崩溃／重投／恢复失败组合待验 |
| 英文／简体中文 | source38 完整 FTL 嵌入、i18n11 与权限控件六张双语截图通过；source56 又实际核对 Claude 2.1.273／2.1.278 说明及固定审批按钮，两种语言均无截断或重叠；输入法和完整三方组合布局待验 |
| 同提交三平台／全工作区 | CI14 实际 af1040 的 Linux 所选门禁通过、Windows 探针失败；source56 是后续 macOS 未提交候选，最终同提交三平台及完整工作区待验 |
| SSH／tmux | 既有原生通知与传输边界保留，三方完整交互和恢复链待验 |

根代理最近核对 Linux／Windows runner 均 online、busy=false，派发前重新留证；不把可用性当门禁成功。source22 不可用入口提示已同步英文与简体中文；i18n门禁11项通过，最终双语检查待验。
