# CLI 对齐验证记录

> **最终状态（2026-09-21）**：Goal 已完成。最终受测代码 SHA 为 `38a611773b8ee53860f9ab731b2476b9a40d1819`；精确平台计数、真实 CLI／GUI／SSH／tmux 证据和没有扩大的限制见 [最终验收](FINAL_20260921_CLI_PARITY.md)。下文所有失败与“未完成”声明保留其历史时间点含义，不用于否定最终精确验收，也不会被回填成当时通过。

阶段性收尾与新会话入口：[2026-09-19 续接交接](HANDOFF_20260919_CLI_PARITY.md)。source56 已统一 Claude 固定策略的受支持版本判定，修复 2.1.278 父任务被旧 2.1.273 字面量阻断的问题，并以官方 Claude Code 2.1.278 完成固定策略恢复、真实父子派发／双向 ACK／结果回收、运行批量取消和待编辑审批取消；原父子失败与批量验收脚本误判均保留。Grok 1.0.34 P0 生产链、GUI 原生读取审批及本机 SSH／tmux worker 已按各自快照通过；最终 Goal 仍未完成。
[自动升级与最新版当前验证](OFFICIAL_42_AUTOUPDATE_VERIFICATION.md)：source50 同源测试程序与签名监督程序已通过 Claude 2.1.278 Latest → Stable 2.1.267、反向 Stable → Latest及2.1.278同版本渠道同步；source56 又以精确测试程序补齐最新版固定权限、父子和两类取消原生链，并以启用 `local_cli_managed_tasks` 的签名 GUI 核对官方 2.1.278 检出、固定审批入口及双语布局。source56 尚无 GUI 任务运行／应用重启或 Linux／Windows 复验，不能回填为最终同提交通过。Codex／Grok 已有渠道结果、最新版协议任务链、完整工作区及最终跨平台仍按各自证据边界继续验收。

## source56 Claude 2.1.278 固定策略与取消验收

[安全收据](validation/macos-official-56-claude-278-native.safe.json)绑定官方 darwin-arm64 Claude Code 2.1.278（217695408 字节，SHA-256 `bd245662fb8a0e321b3bf133e930371d6563c387527885f30b2613aef3ba14d6`，严格签名通过）、source56 测试程序（867291224 字节，SHA-256 `9357bc46c877ad1e4544156afceb8d84774398805481dc0d390571d959232b0f`）及 source50 签名监督程序。监督路径在 source50 到 source56 的本轮相关实现未变；后续 source56 签名 GUI 是同一 dirty 候选的补充构建，仍不是已提交同 SHA 的证明。

第一轮真实父子夹具保留为失败：父任务已经以 2.1.278 固定策略通过预检，但权限上限仍硬编码 2.1.273，`run_agents` 在创建子任务前拒绝并最终超过 450 秒；清理回执已取得。source56 复用 Claude 适配器既有支持版本表，权限上限和任务管理器不再各自保留旧字面量。修正后 61 个公开事件包含 49 个生产运行时事件、5 次审批、真实子任务创建、父子相同固定策略、双向原生消息 ACK、三组原生结果关联、自动结果投递及两个清理回执，整体退出 0。

运行批量取消首例的 Rust 产品测试已经退出 0 并发出完整通过事件，但外层验收脚本把并入既有批次的输入错误要求为 `turn_started=true`，因此总体失败且不计成功。脚本修正为精确要求运行中／并入／继续分别为 `true/false/true`；28 项离线测试通过，第二次真实运行取得 138 个事件和 110 条原生协议记录：2 个输入取消、1 个输入继续、原生 interrupt ACK、取消终态、完整批次结果、同一原生会话和清理回执全部通过。待编辑审批取消另一次通过 96 个事件和 70 条原生协议记录：审批未放行，文件四次核对未变，原回合以 `aborted_tools` 取消，下一输入在同一会话完成且正常清理。

固定策略恢复使用此前 source50 测试程序完成：7 次 profile 核对、允许／拒绝两次审批、同策略恢复及三种越权恢复拒绝通过；source56 的版本统一改动不改变该流程。source56 本地门禁另通过权限模块 8 项、任务管理器模块 36 项、两个新增版本用例各 1 项、Python 28 项、i18n 11 项和 `cargo check -p warp`。真实 GUI 暴露原说明仍只写 2.1.273，因此同一消息键已同步更新英文和简体中文，重新通过 i18n 11 项与 check；1230 × 768 下两种语言的版本说明及审批按钮均无截断或重叠。完整工作区没有因本轮 3 个生产文件、2 个 Rust 测试文件和 2 份 FTL 重跑；source53 的 7297 项 `warp` 全量结果只作为继承背景，不冒充 source56 全量门禁。

第一次同源 GUI 主程序构建在 `warp` rlib 归档阶段因本机磁盘空间耗尽（errno 28）退出 101；此前 target 中的旧 `infinishell` SHA-256 `19a094a00e65cb3c60a7e34edf5e3415442178ec029c176e96a32983275732b1` 保持不变，该失败未被后续结果改写。清理可重建缓存后，无 feature 构建通过但不含任务管理器入口，因此不计托管 GUI 验收。随后启用 `local_cli_managed_tasks` 并在双语文案修正后重新构建通过：main 为 743121984 字节、SHA-256 `871e8ab4666530d12a0b824f8668e4b249bcee47c0cdb901ff73976546662f35`；隔离应用内重签二进制 SHA-256 `decefd1b65edecbc92338ff378177125d919a27e9e0076f4a52de51449a68098`，deep/strict 签名核对通过。

该签名应用通过 `PATH` 精确绑定官方 Claude 2.1.278，任务管理器显示真实版本与固定路径；中英文分别显示 `Claude 文件审批任务` 和 `Claude reviewed file tasks`。只执行安装检测、策略选择和布局检查，没有发送模型请求、启动任务或使用认证，也没有验证应用重启和持久化。验证结束后语言偏好已恢复简体中文，临时应用进程已退出。

所有模型用例使用各自新建的私有认证目录；运行器未记录环境值、未读取或复制原生凭据文件，项目设置与 CLI 文件前后保持。私有原始记录继续留在本机临时目录，仓库只保存脱敏投影的摘要。source56 仍为未提交 dirty 候选；未获提交／推送授权，因此没有派发要求精确远端提交的 Linux／Windows workflow。新增 GUI 结论只覆盖安装检测、策略入口与双语布局，不覆盖真实任务、应用重启或持久化。

[source41候选](OFFICIAL_40_RECOVERY_AND_INPUT_GUARDS.md)已通过check、i18n11项、受影响Rust1826项和桌面构建；Grok文件允许／拒绝及待审批取消五阶段通过，冷继续恢复与输入ACK已到达，但测试代理连接预算耗尽，整体仍失败。普通Grok复合启动工具栏、英文与简体中文完整提示、两种语言多行草稿显式复制往返均通过；自动PTY输入仍为明确降级。source41a另修父子自动结果的时序及跨代恢复，已通过 check、i18n 11 项及受影响 Rust 1834 项；尚未运行该快照的原生链。新增[消费者自动升级与开发最新版对齐](CLI_AUTOUPDATE.md)属于当前Goal；历史固定版本证据保留，不能替代最新版或最终同提交三平台、完整工作区及SSH／tmux验收。

[内部维护兼容修复](GROK_INTERNAL_MAINTENANCE_COMPATIBILITY_FIX.md)、[缓存认证握手修复](GROK_POLICY_AUTH_HANDSHAKE_FIX.md)及[Grok 0.1.1 插件升级修复](GROK_PLUGIN_011_MIGRATION_FIX.md)已合入新的 source32 候选快照：父提交0059，119项输入、18处变更，其他101项逐父Git对象保持一致，新增38项Rust回归包含8项tokio异步测试。诊断测试同步为合法内部维护帧无业务效果且不消耗pending，未知ID、错误帧及夹带会话身份仍拒绝；原source31快照和失败记录保持不变。source32独立门禁已真实全部通过：check退出0／140.859秒、i18n11／320.445秒、Rust1458／25.620秒、Python15组453项无跳过；38项新增Rust（含8项tokio）逐项各1PASS，119项输入与850624232字节测试库前后保持一致。main已真实构建退出0／329.100秒，严格签名退出0、完整中英文资源逐字节嵌入，测试库身份保持一致。[source32原生部分复验](OFFICIAL_32_NATIVE_PARTIAL_VERIFICATION.md)：SDK12（29.281秒）与policy4（13.074秒）均Rust101／runner1，原失败保持；SDK12已实际走通现代发现、tools/list和一次inspect业务调用，2次原生审批允许、transport正常结束及清理收据通过，但六种carrier均无原生S/P/T，身份关联验收未通过，子任务和父权限门禁保持关闭。policy4实际仅发3RPC，initialize及cached_token authenticate成功，session/new关联rpc_error，未完成两阶段接口验收；失败取消的null退出码投影缺口另行修复，不补成退出0。认证副本／连接／目录已清理，配置与策略夹具保持不变，不能将14RPC预算写成已发出计数。原生六阶段插件迁移尚未执行；另发现0.1.1配方的通知仍上报0.1.0，后续候选须同步脚本版本及严格历史夹具，并独立验证真实安装后的通知。[SSH／tmux执行计划](SSH_TMUX_FINAL_ACCEPTANCE_EXECUTION_PLAN.md)仅盘点可复用入口和待验边界，没有新增真实SSH链通过。

更新日期：2026-09-19（北京时间）。Goal 未完成。工作分支 `codex/cli-agent-parity`；当前实际 HEAD 为 `af1040dc8e7522ccafb09d8f0d4c6b219bd9d607`，source39 及后续修复尚未提交。历史检查点与中间 dirty 快照仍保留各自结论，不计为最终同提交验收。

[真实SDK11与后续兼容修复](GROK_INTERNAL_MAINTENANCE_COMPATIBILITY_FIX.md)：source31唯一一次在线探针仍FAILED（Rust101／runner1），但严格成功形状及私有账本对应校验真实为true，认证副本与隧道已清理。确认固定skills-reload双层封闭成功结构后，后续两路径生产候选仅作内部维护空操作，不消费数值pending，不发出生命周期或提升子任务／权限门禁；新增6项Rust回归未执行，不将原失败追认为修复后成功。

## 实际436提交的source26验收

[独立本地门禁](OFFICIAL_26_LOCAL_GATES.md)核对actual436、干净验证树、111路径清单SHA `3ff03385c6f9635c1e60304d3220c49cdb281f314e73735a7f90a3f3298875c9`。check退出0／61.973秒；i18n11通过／129.135秒；Rust1393通过、5483 skipped／18.358秒；Python15组410通过／5.091秒。main构建退出0／241.004秒、严格签名退出0、两份完整FTL嵌入核对通过。原main SHA `d8102b9d02cd9f104d0d2c2098ca805c99abf6ac60441bd51b84356d3fd12de5`，lib SHA `bb019bc5e8e4c30582767e9a89685d46c515211c73b0b8914d04a5ee5b0e1cf1`。

[有限原生验收](OFFICIAL_26_NATIVE_PARTIAL_VERIFICATION.md)：Claude待Edit取消2真实通过，完整原生ACK／Cancelled生命周期／完整输入UUID结果／interrupt归属聚合后可信取消，并同原生S继续完成；Grok SDK9生产transport拒绝未知字符串response ID，inspect0、来源unknown、整体失败；无输入policy1虽initialize通过，但session/new未取得关联回复，未知通知及退出码缺失导致失败，不能宣称审批／取消／恢复策略已经核实。根代理前后分别核对clean436、111源码、固定CLI、lib与main身份，原生记录自身的`sameCommit_verified_by_runner=false`不改写。

[实际GUI部分验收](OFFICIAL_26_GUI_PARTIAL_VERIFICATION.md)使用重签隔离副本，SHA `2c0d7c02c7dd6cf21ee6088b0aa04dd8b4ea66287d6fb4be4dc3f4132a0a05ad`，通过Computer Use操作真实窗口。已验证三CLI标题栏图标与版本、双语目标组件布局；Claude同原生会话两轮、Write允许／拒绝的真实文件效果、等待审批时追加且后续实际执行、当前轮取消且文件保持、显式历史继续、正常应用退出重开后的任务／消息／历史结果恢复与不自动重发。重启后的新指令实际返回历史标记，最终结果和第8条原生确认消息已持久化。不覆盖活跃进程重新关联、父子GUI或真正中文输入法。

[CI12首核](CI_12_INITIAL_VERIFICATION.md)及[有界接续](CI_12_FINAL_VERIFICATION.md)的实际head均为436，full_workspace=false。两平台Python失败、固定原生准备跳过；Windows check、SSH worker、受影响Rust等步骤已通过，但已观察run终态failure；Linux真实日志定位Grok coordinator离线夹具直接使用macOS的/private/tmp目录，Windows真实日志另定位同一临时目录、SQLite连接占用以及平台路径哈希断言三项问题，整次未通过。source27另冻结5Rust＋4Python诊断候选，只增强私有形状证据／事务账本及安全公开摘要，不放宽生产pendingID和子任务门禁；其[门禁结果](OFFICIAL_27_LOCAL_GATES.md)已通过check、i18n11、Rust1410和Python426，不回填26。

检查点 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 已实际提交、推送并核对远端一致。该提交干净验证树的 [source23 门禁](OFFICIAL_23_LOCAL_GATES.md)通过 check、i18n11、Rust1301、Python363、main构建、严格签名及完整双语资源嵌入。[source23 在线验收](OFFICIAL_23_NATIVE_LIVE.md)：Claude PNG3三输入、两个正常退出及同原生会话历史继续通过；Grok SDK8整体失败，但现代2026-07-28 MCP发现、工具列表及原生ready实际已观察；Claude等待Edit取消1整体失败，原生interrupt ACK、审批撤销、错误result及cancelled生命周期均已观察，未取得应用可信取消聚合。[CI11](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35248824124) 同SHA：Linux的Python超时夹具失败、Rust未执行；Windows的Python与Rust定向步骤失败，check和SSH worker构建通过；真实日志已取得，分别定位为纯测试的路径分隔符断言和私有JSON反斜线编码断言，原生准备因前置失败跳过、证据上传缺失为继发失败。两平台均不能计完整通过。后续修复不在该提交：[source24候选门禁](OFFICIAL_24_LOCAL_GATES.md)已冻结111路径并通过check、i18n11、Rust1378、Python410；包含Claude真实取消枚举补丁、Grok测试夹具对生产传输真实错误的诊断、标题栏普通终端启动及本地安装路径回退。只读预检已预编译并核对唯一ignored入口，准备标记已解除，原生接口尚未执行；该候选验证树dirty、未提交，不计新的同提交／跨平台／GUI通过。同一source24快照另行补跑实际`terminal::model::session::test`模块15/15通过，含新增4项，主1378计数不回填。source25已以同一111路径冻结Windows两处测试修复与两平台正确Session筛选，[正式候选门禁](OFFICIAL_25_LOCAL_GATES.md)已通过check、i18n11、Rust1393、Python410；该候选修改后来形成436提交，source26与CI12分别独立复验，不回填当时候选。

## 当前候选与历史门禁

[source29本地门禁](OFFICIAL_29_LOCAL_GATES.md)基于实际e687冻结111输入／4路径修改，其他107项与父Git对象一致，check退出0／131.245秒、i18n11／314.571秒、Rust1420／22.966秒（新增10逐项各1PASS）、Python436／8.151秒均通过。四路径随后已形成实际0059并推送；[提交后的字节核对](validation/source29-actual-commit-identity.json)验证111个Git对象与原候选、主树一致。原候选门禁的dirty范围不改写为当时已经clean。另在干净source30树实际构建0059 main，退出0／323.458秒，二进制866053040字节、SHA-256 `ab33468aa09057ee58f71031aa5026430a8bdf3db9e34a776b02d1bbac1488ea`，严格签名退出0、完整英文及简体中文资源字节均在二进制中；只证明构建与资源嵌入，[policy3真实复验](OFFICIAL_30_GROK_POLICY_VERIFICATION.md)整体FAILED，同0059平台仍待验；初始化及封闭空目录通知兼容成功，New取得关联RPC错误，未完成历史继续或完整原生退出收据。探针漏掉cached_token握手已另行修复为后续候选，不将缺口等同实际错误原因或授权失效。CI13仍针对实际e687，不能随分支新HEAD变动追认为0059。

实际436的[source26 Claude PNG GUI专项](OFFICIAL_26_CLAUDE_PNG_GUI_VERIFICATION.md)已完成首轮真实附件识色、同原生ID第二轮文本历史继续、正常退出App重开后的第三轮文本继续；三轮均取得native_protocol接收确认和完整43／50／51字节结果。唯一248字节PNG附件引用在重启前后保持一致，旧输入不自动重放。[独立公共归档审计](OFFICIAL_26_CLAUDE_PNG_GUI_ARCHIVE_AUDIT.md)核对31文件及根报告、实际解码96×96夹具全部像素为左红右蓝；审计没有重新执行GUI、OCR或验证原生wire。验收发现混合附件历史展示原始JSON与内部路径，后续source31四路径产品候选改为可读文本、图片附件与技能名称，英文／简体中文同键同变量同步，[source31候选门禁](OFFICIAL_31_LOCAL_GATES.md)通过check、i18n11、Rust1430、Python443，111输入／7修改前后稳定，新增SDK7与UI3各一次PASS；main构建及严格签名实际退出0／321.646秒，完整双语资源已核对嵌入；[双语附件历史布局及复制实测](OFFICIAL_31_GUI_PREVIEW_VERIFICATION.md)通过，无新原生输入，历史三个表的行数量及完整哈希保持一致，真实技能历史布局仍未覆盖。后续缓存认证探针四路径和Grok插件0.1.1迁移不在此冻结范围，尚未Cargo或原生复验。

[CI13第三窗口终态](CI_13_THIRD_WINDOW_VERIFICATION.md)实际查询到e687的Linux／Windows两个job均completed/success；Linux32成功步骤／2跳过、Windows38／2，均为步骤计数，不能当作测试case数量。真实job日志各取得一次失败，精确case计数保留未知；full_workspace_tests=false及GUI编译跳过保持原范围。[actual0059的Grok插件离线补查](validation/source30-grok-plugin-offline.json)使用显式TAP格式运行11项全通过，0.337秒；四个输入前后均与实际Git对象一致，仅证明转换／去重／降级／转义，不证明在线hooks实效。

[发布支持与回退草稿](RELEASE_SUPPORT.md)已补受测版本、普通终端与托管区别、插件信任与修复、审批／取消／确认、历史继续和恢复失败处理。该配套文档不是已经发布或Goal通过的声明。

历史候选[source25](OFFICIAL_25_LOCAL_GATES.md)为111路径，manifest SHA `3a722bb3f6f2c0c4282b628c4e0671f6a8e4c17c972b2b407b667b681eba1101`；check52.750秒、i18n11／142.748秒、定向1393／5483 skipped／19.892秒、Python15组410／4.999秒均通过。当时候选dirty；其修改已形成实际436提交，source26另行验证。下列source20–22记录为历史，source23实际dec构建与有限原生结果仍按独立专报保留。

source21 已冻结87路径，manifest SHA `966dfd9ff2bced152808fb6ee75bb2a7b41e6f87eabb57157fdd0a947f24dff0`，通过 check（63.194秒）、i18n11（178.715秒）、定向1270／5534 skipped（18.283秒）及 Python14组363（4.794秒）；lib SHA `10755f06d9e0d004c99551f24063660139f34bc8e9f07d08f3f9dd328d81a09e`。没有 source21 main／签名／真实CLI，见 [独立归档](OFFICIAL_21_LOCAL_GATES.md)。随后发现的旧独立Claude AgentDriver 已在主工作区关闭，固定绕过与全局写入实现已移除，普通终端和托管任务保留各自入口；source22 的93路径新快照（manifest SHA `2179e1a3add0f5507440f565b9fe2b4382ab68112555c313d9fc758073cb1482`）已通过 check（85.881秒）、i18n11（169.478秒）、定向1301／5493 skipped（18.693秒）及 Python363（4.831秒），lib SHA `752f2bd512cbe2bc874904a0b647350979a2b46c702254c43e37beecc96f450a`。五项新增配置与分派/auth回归全部实际PASS；编译有两项测试桥导出unused_import警告，退出0。见 [source22 独立归档](OFFICIAL_22_LOCAL_GATES.md)；没有22main、实际CLI或同提交平台结论。

历史已完成构建的 source20 为83路径清单 SHA `b3b3e7a61530b1314ef5e4f8de5f2c2b5bfb62f883e7b3e6109d42d2590fc693`。实际结果见 [独立本地归档](OFFICIAL_20_LOCAL_GATES.md)。

| 实际门禁 | 结果 | 墙钟时间／范围 |
| --- | --- | --- |
| `cargo check -p warp` | 退出0 | 60.240秒 |
| `cargo test -p warp --lib i18n::tests` | 11通过、0失败 | 146.350秒，内部4.45秒，6770 filtered out |
| 受影响模块 nextest | 1248通过、5533 skipped | 17.632秒，内部14.685秒，不等于全工作区 |
| Python离线回归 | 13组／329项通过 | 4.565秒；SDK组77项，Codex准备器27项 |
| source20 main构建 | 退出0 | 196.531秒 |
| `codesign --verify --deep --strict` | 退出0，双输出0字节 | 0.480609秒，不证明发布者／公证／公开发布 |
| 双语完整资源嵌入 | 原构建报告记录两份完整FTL存在 | en 374753字节／zh-CN 361658字节；不等于布局检查 |

source20 lib SHA 为 `c5819a0d7e73031b3ce19716e27e143b3188e03a5c6e7b0bc5e1a8b2a88c7716`，main／监督worker为 `8b1ad22b400b71b13e08ca6d9f8fdcc3b78da69c965d3fd9366a8cfa1ed3ddea`。身份来自原运行／构建报告，独立归档核对报告及日志，没有重算二进制或重新查询内核。当前文档整理也未重跑门禁。

source21当时只编译现代MCP、PNG Resume就绪／身份分离及待定Edit取消夹具，没有原生验证；旧独立Claude入口修复在source22新增回归中通过，不属于source21范围。后续source23真实PNG3已完成第三输入历史继续，SDK8及等待Edit取消1整体失败，均保留原报告。

## 当前真实CLI证据

| 原报告／实际来源 | 已证明步骤 | 未扩大为通过的范围 |
| --- | --- | --- |
| [Codex完整包原生复验](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md) | 37b0bc732干净macOS适配器的新建／两轮／审批／追加／取消／同ID进程历史继续、工具恢复、localImage | 当时提交的原生适配器证据，不替代当前GUI／三平台／最终提交 |
| [Codex隔离GUI](validation/macos-gui-report.md) | 中间bundle两轮、允许／拒绝、真实Steer与取消、应用退出重启、原ID显式继续、技能／图片／评审、父子消息和实际回收 | bundle3–6各自边界保留；候选列表、普通其他CLI、完整故障组合及最终提交待验 |
| [Claude API适配器](CLAUDE_API_ADAPTER_VERIFICATION.md) | 固定2.1.273真实两轮、精确文件审批、排队确认、取消与原ID新进程恢复标记 | dirty中间快照；不替代完整父子GUI和最终提交 |
| [Claude固定策略GUI](CLAUDE_GUI_RUNTIME_CHAIN.md) | Edit允许／拒绝、实际合并追加、source8原ID／同策略历史继续与回收 | source6第一精确夹具及第五取消失败保留；英文source8／9动态FTL边界保留，最新完整双语未验 |
| [Claude父子协调器](CLAUDE_COORDINATOR_ACCEPTANCE.md) | 第2轮生产执行5输入／3执行／2合并，双向原生ACK、inspect与自动结果，修正独立审计通过 | 原libtest退出0，但旧wrapper退出1／metadata=false不改写；修正审计不是新真实重跑，不覆盖GUI／全重启／跨平台 |
| [Grok source11 GUI](GROK_GUI_RUNTIME_CHAIN.md) | 根任务9消息／9代、7 Completed／2 Cancelled，Write允许／拒绝、排队、正文后取消、同连接继续、实际应用重启、原ID历史继续、无自动重投及显式记忆回收 | 固定1.0.30、Inherit根任务、source11 dirty；36张实际JPEG与对应EN／ZH布局，不覆盖最新全界面、SDK／子任务或其他平台 |
| [Grok source13生产coordinator](GROK_COORDINATOR_ACCEPTANCE.md) | 生产runtime commands与SQLite实际8输入／ACK／Started／完整结果，6 Completed／2 Cancelled；原提交5到执行6关联、原ID新进程load及记忆回收、两次正常清理 | App::test不是GUI；不是完整桌面重启／活跃重关联，不验证父子权限或SDK；审批运输回执不冒充原生审批ACK |
| [Claude PNG2／SDK7](SOURCE20_NATIVE_DISCOVERY.md) | 下列source20真实改善与失败均原字节保存 | 两次test101／wrapper1、整体FAILED；不计任何整轮成功 |

### source20 PNG2：恢复判据提前失败

PNG识色与中文多行两次完整结果及原生来源摘要通过，首代明确 `stdio_closed`、exit0、Job／CID清理。之后磁盘附件哈希／大小和typed引用一致，`image_replayed_to_native=false`；只是文件引用恢复，不是原生图片重投或全继续链通过。

Resume最新startup的原生ID为null／关联false，夹具以 `resume_ready_identity_failed` 提前拒绝，第三input未发。第二代回执也是stdio／0／清理确认，但不能据此确认恢复到同一原生历史或回收成功；metadata中的整体acceptance和durable restore仍为false。新候选须在后续实际ACK／输入／结果确认同ID，不能把启动就绪自身当成会话身份。

### source20 SDK7：首次发现回调，不是业务工具结果

首次真实 `_x.ai/mcp/sdk_call → server/discover`，outer／inner ID为0、SDK请求1；SDK参数键为message／serverId，内层只有_meta。公开版本投影null不能证明_meta内没有版本；三个散列键的标准发现关联来自根代理官方接口核对，实际值没有在独立归档读取。

六个carrier的session／prompt／tool来源字段均缺失或null，但这是发现阶段，不推断后续业务工具也无来源。应用submitted1，原生输入确认0，initialize／tools-list／inspect／审批0，没有业务ACK／Started／Result。来源unknown、产品SDK／子任务gate关闭。退出回执 `exit_code=null`、原始wait9、cleanup=true；不可描述为原生退出0。SDK6旧失败保留，direct首次回调不证明leader根因。

## 完整验收矩阵

以下是用户规定的最终门槛。“中间证据”仅证明原快照相应步骤；三款CLI均未在最终实际修改提交满足全部组合。

| 验收 | Codex | Claude | Grok |
| --- | --- | --- | --- |
| 新建／两轮／完整结果 | 原生与macOS GUI中间通过 | API与GUI有阶段证据，source23 PNG3三输入完整结果通过 | source11 GUI及source13 coordinator根任务通过 |
| 审批允许／拒绝 | 中间GUI与实际文件效果通过 | 固定策略API／GUI通过；完整父子GUI待验 | 根任务精确Write通过；子任务上限未验 |
| 运行中追加／原生接收／模型采用 | GUI真实Steer及父→子采用有证据 | API排队、GUI／父子实际合并与完整UUID结果有证据 | 根任务后续回合排队有证据，不宣称steer |
| 取消／继续 | 中间真实取消与继续有证据 | 批次三证据取消有证据；source23等待Edit取消1已原生运行但应用可信聚合失败，source25补丁待真实复验 | source11正文后取消与同连接继续通过；SDK8发现／ready及清理不替代业务取消 |
| 应用重启／恢复／回收 | bundle历史与活动退出后显式继续有证据 | 实际436的source26 GUI原ID继续、正常App重启、SQLite任务／消息／历史结果恢复、无自动重投及明确新输入记忆结果回收通过；父子和活跃恢复仍待验 | source11真实应用重启与历史记忆通过；最终提交整链待验 |
| 父子双向消息／进度／结果 | macOS中间GUI有实际闭环 | 固定策略独立协调器执行与修正审计通过，完整GUI待验 | source23 SDK8发现／列表／原生ready已观察，inspect0／来源未验证，未开放子任务 |
| 插件缺失／版本／禁用／更新失败恢复 | 各原生／GUI／平台子集有记录，完整故障组合待验 | 固定缓存与升级／强杀重试有记录，完整组合待验 | 安装器事务子集有记录，完整平台组合待验 |
| 崩溃／重复／旧回调／重投／恢复失败 | 定向故障与macOS真实崩溃清理有证据，最终提交待验 | 共享回归不能替代真实异常工具崩溃；完整组合待验 | 共享回归及根任务正常清理不替代真实崩溃／SDK故障整链 |
| 双语布局 | 旧bundle布局有证据，最新全功能待验 | 固定策略阶段布局有证据，动态FTL边界及最新GUI待验 | source11内嵌EN／ZH实际布局通过，最新完整布局待验 |
| 同提交macOS／Linux／Windows、全工作区、SSH／tmux | 最终组合未完成 | 最终组合未完成 | 最终组合未完成 |

## 平台、发布与实质限制

[CI10 b3d8](TENTH_PLATFORM_RUN_B3D8.md)的Linux x64／Windows x64所选门禁通过，`full_workspace_tests=false`；不包含其后固定策略、完整历史及布局修改。Windows实际五项Codex hook注册、两项ConPTY通知通过，另外三项实效不能从注册推定。固定完整运行包的新准备器27项离线回归与异平台静态提取见 [准备说明](CODEX_FIXED_RUNTIME_PREPARATION.md)；异平台提取不是异平台执行，Windows ACL及ARM64执行边界仍按原报告保留。

根代理最近实际API核对Linux／Windows runner均online、busy=false；00:01的Linux离线观察保留为 [历史可用性](validation/runner-availability-20260918-0001.json)。真正dispatch前须重新核对留证，当前可用性不计新的平台门禁通过。不存在整个Goal因此无法独立推进的结论。

SSH／tmux的 [构造通知](SSH_TMUX_VERIFICATION.md) 与 [真实Codex原生TUI传输](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)分开记录；模型HTTP为零的探针不等于三方完整在线交互、输入、审批、重启和恢复。远端只通过仓库cross-platform-preflight workflow，在本地新门禁通过、实际修改提交推送且headSha一致后验证。最终按仓库要求执行全工作区门禁，不能以选定筛选覆盖替代。

Codex默认shell_snapshot在含引号与命令替换字符CODEX_HOME中的上游路径限制仍有 [独立原记录](validation/macos-codex-shell-snapshot-path-limitation.json)，隔离探针关闭特性的正向结果不改变默认限制。Claude固定文件策略不是OS文件／网络沙箱，Inherit观察不能证明任意完整父权限上限。Grok SDK8现代发现版本注册已有真实部分证据，业务来源、子任务权限及兼容Claude hooks完整链仍待验；旧额度／未登录失败保留历史，不再把已完成的官方在线授权和根任务成功说成当前无法使用。

## 下一轮交付必须补齐

1. Claude等待Edit取消聚合、Grok诊断及标题栏启动已形成实际436提交，source26另行真实验收；Claude文本根任务的App／SQLite／GUI基本链已通过。CI12真实平台失败的三类Python原因已精准修复；source28的111项／11路径候选check、i18n11、Rust1410和Python426全部通过，代码已形成实际e687并推送。CI13同e687两平台所选门禁已completed/success，full_workspace_tests=false及GUI编译跳过边界保持；一次原生日志取得失败，不从步骤状态编造case计数，最终全工作区门禁不能省略。
2. Claude PNG专项GUI／App重启已在实际436完成，其他格式、最终提交、三方父子完整GUI及活跃重新关联仍待验。source27真实policy2／SDK10失败已分别保留主次失败与事务上下文；精确比对识别 `skills-reload`，但不是安全接纳或业务来源证明。全局空目录通知的受限修复已形成实际0059，source29门禁通过、source30真实policy3整体FAILED，探针cached_token握手修复与两阶段14RPC预算为后续候选，Rust及原生复验未执行。零模型内部watcher探针未观察目标响应且保留失败；source31三路径诊断候选只采集闭合成功形状的strict bool，不修改生产未知ID拒绝、pending请求或权限门禁。实际436的source26 Grok GUI已有两轮、Write允许／拒绝、主动取消与追加回收；第8代重启后的72字节记忆结果、第9代独立历史继续的71字节结果均已真实回收，消息不重放，不能覆盖第7代中断记录。
3. 普通三方终端、附件／上下文／技能／评审、插件负向与故障、父子GUI与恢复的完整验收；最新英文／简体中文布局分别留证。
4. 包含实际修改的同一提交macOS／Linux／Windows相关验证、完整工作区与独立SSH／tmux链；真实CLI证据与Actions head SHA相符。
5. 完整能力矩阵、插件／发布配套、文档和验证报告；所有外部限制及失败准确列出，只有全部门槛满足才完成Goal。

source22 新增旧独立入口不可用提示，英文与简体中文同键、同 `$cli` 变量同步；i18n11与编译门禁已通过，最终双语布局待验。用户排除的两个网页搜索文件与整个validation/gui-7e065085目录继续保留并排除本目标暂存。

## 历史原始报告索引

历史检查点和中间快照只保留当时结论，不作为当前HEAD或最终验收。下列独立专报、原始JSON、失败和修正审计均留在原文件；本次未修改或重新运行它们。具体受测源码、二进制、错误及未覆盖项以原报告为准，不能因为存在文档链接而自动视为已审。

### 平台历史

[TENTH_PLATFORM_RUN_B3D8](TENTH_PLATFORM_RUN_B3D8.md)、[NINTH_PLATFORM_RUN_E4738E1EA](NINTH_PLATFORM_RUN_E4738E1EA.md)、[EIGHTH_PLATFORM_RUN_494569582](EIGHTH_PLATFORM_RUN_494569582.md)、[SEVENTH_PLATFORM_RUN_37B0BC732](SEVENTH_PLATFORM_RUN_37B0BC732.md)、[THIRD_PLATFORM_RUN_328D5ED35](THIRD_PLATFORM_RUN_328D5ED35.md)、[WINDOWS_CONPTY_NOTIFICATION_PROBE](WINDOWS_CONPTY_NOTIFICATION_PROBE.md)、[SIXTH_PLATFORM_RUN_7E0650855](SIXTH_PLATFORM_RUN_7E0650855.md)、[FIFTH_PLATFORM_RUN_94A412EB](FIFTH_PLATFORM_RUN_94A412EB.md)、[WINDOWS_VERIFICATION_FIXTURES](WINDOWS_VERIFICATION_FIXTURES.md)。

### 本地冻结与构建历史

[OFFICIAL_11_LOCAL_GATES](OFFICIAL_11_LOCAL_GATES.md)、[OFFICIAL_12_LOCAL_GATES](OFFICIAL_12_LOCAL_GATES.md)、[OFFICIAL_13_LOCAL_GATES](OFFICIAL_13_LOCAL_GATES.md)、[OFFICIAL_20_LOCAL_GATES](OFFICIAL_20_LOCAL_GATES.md)、[OFFICIAL_21_LOCAL_GATES](OFFICIAL_21_LOCAL_GATES.md)、[OFFICIAL_22_LOCAL_GATES](OFFICIAL_22_LOCAL_GATES.md)、[OFFICIAL_18_LOCAL_GATES](OFFICIAL_18_LOCAL_GATES.md)、[OFFICIAL_19_LOCAL_GATES](OFFICIAL_19_LOCAL_GATES.md)、[OFFICIAL_15_LOCAL_GATES](OFFICIAL_15_LOCAL_GATES.md)、[OFFICIAL_14_LOCAL_GATES](OFFICIAL_14_LOCAL_GATES.md)、[OFFICIAL_16_LOCAL_GATES](OFFICIAL_16_LOCAL_GATES.md)、[OFFICIAL_17_LOCAL_GATES](OFFICIAL_17_LOCAL_GATES.md)。

### 真实任务与GUI历史

[GROK_GUI_RUNTIME_CHAIN](GROK_GUI_RUNTIME_CHAIN.md)、[GROK_OFFICIAL_ONLINE](GROK_OFFICIAL_ONLINE.md)、[CLAUDE_GUI_RUNTIME_CHAIN](CLAUDE_GUI_RUNTIME_CHAIN.md)、[CLAUDE_BATCH_CANCEL](CLAUDE_BATCH_CANCEL.md)、[GROK_COORDINATOR_ACCEPTANCE](GROK_COORDINATOR_ACCEPTANCE.md)、[SOURCE13_NATIVE_CALIBRATION](SOURCE13_NATIVE_CALIBRATION.md)、[SOURCE20_NATIVE_DISCOVERY](SOURCE20_NATIVE_DISCOVERY.md)、[CLAUDE_APPROVAL_CANCEL](CLAUDE_APPROVAL_CANCEL.md)、[SOURCE18_NATIVE_FAILURES](SOURCE18_NATIVE_FAILURES.md)、[CLAUDE_GUI_E473_A245](CLAUDE_GUI_E473_A245.md)、[CLAUDE_API_ADAPTER_VERIFICATION](CLAUDE_API_ADAPTER_VERIFICATION.md)、[CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md)。

### 协议、权限、插件和恢复边界

[SDK_ORIGIN_PROBE](SDK_ORIGIN_PROBE.md)、[GROK_SDK_ROUTING_DIAGNOSTIC](GROK_SDK_ROUTING_DIAGNOSTIC.md)、[CLAUDE_QUEUE_AND_CODEX_REV4](CLAUDE_QUEUE_AND_CODEX_REV4.md)、[PROTOCOL_EVIDENCE](PROTOCOL_EVIDENCE.md)、[STOP_HOOK_COMPLETION_AUDIT](STOP_HOOK_COMPLETION_AUDIT.md)、[UNCONFIRMED_TASK_STATE](UNCONFIRMED_TASK_STATE.md)、[OZ_LOCAL_MESSAGE_DELIVERY](OZ_LOCAL_MESSAGE_DELIVERY.md)、[CODEX_HOOK_TRANSPORT_VERIFICATION](CODEX_HOOK_TRANSPORT_VERIFICATION.md)、[CODEX_NATIVE_SSH_TMUX_VERIFICATION](CODEX_NATIVE_SSH_TMUX_VERIFICATION.md)、[macos-gui-bundle7-plugin-report](validation/macos-gui-bundle7-plugin-report.md)、[CLAUDE_NO_CREDENTIALS_VERIFICATION](CLAUDE_NO_CREDENTIALS_VERIFICATION.md)、[DARWIN_COALITION_LAUNCHD_AUDIT](DARWIN_COALITION_LAUNCHD_AUDIT.md)、[DARWIN_LAUNCHD_COALITION_PROTOTYPE](DARWIN_LAUNCHD_COALITION_PROTOTYPE.md)、[DARWIN_COALITION_PRODUCT_CONTRACT](DARWIN_COALITION_PRODUCT_CONTRACT.md)、[GROK_BYOK_AUTH_VERIFICATION](GROK_BYOK_AUTH_VERIFICATION.md)、[GROK_TITLEBAR_ENTRY](GROK_TITLEBAR_ENTRY.md)、[CODEX_FIXED_RUNTIME_PREPARATION](CODEX_FIXED_RUNTIME_PREPARATION.md)、[macos-gui-bundle8-report](validation/macos-gui-bundle8-report.md)、[GROK_FIXED_ACP_BOUNDARIES](GROK_FIXED_ACP_BOUNDARIES.md)、[CLAUDE_PLUGIN_UPGRADE_TRANSACTION](CLAUDE_PLUGIN_UPGRADE_TRANSACTION.md)。
