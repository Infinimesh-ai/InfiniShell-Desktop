# G09：转 Windows 继续开发

## Windows 接续最新状态（2026-10-06）

本提交已补首个未选择调用的有界收据（原身份、时序、Node状态、flags、路径比较四态），并保留原LOAD时已核CLR/配对DAC身份；选择门槛与成功条件不变。本机64普通、command特性check、i18n11和应用诊断36通过，唯一native实际执行一次通过并核源码/测试exe前后摘要，独审SHA `6cee15a6e10cf8a57bb843cfe9a897819c705d0c3666d7821de69cda4a02c789`。本机不同CLR版本不能代证原PS。下一步先原runner固定能力，再同提交真实PS；G09开放，新增关闭0项，无需本地化变更。

最新 `b12f59967` 原runner固定能力37395058332已通过60普通及唯一native，独审SHA `b66385a07627f61f9ec18e30d36650010120e3d18638caa512b1d0937ef1d821`。同一提交的[真实PS37395641796](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37395641796)再次复现零输出退出1、Node/Codex随后退出0；67来源逐字节匹配、CMD/PS两代进程及station/LSA清理通过。selected/returned为0，skipped为1；唯一skip的时序、flags及路径比较未保存，不能称为preNode或推UseShellExecute=true。没有选中分类返回或同停点CLR，本代实际CLR/DAC/SMA身份也未取得。终审SHA `ef7f790e963ef56e562ad26ebb58d2a9ea0da013770102457d5bd59eaa623539`。下一步先补首skip有界证据、复核观察器，再继续真实取证；不重跑相同无信息候选，不改变成功条件。G09仍开放，正常五场景/三冷恢复与最终同源门禁仍待完成。以下为历史顺序及原失败记录。

[真实PS运行37352397472](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37352397472)绑定 `7f6d9d6e3`，在Node创建前seq177因诊断器RF写回的EFlags固定bit1读回差异中止；没有分类返回或同停点CLR栈，原PS首因仍未知。原件独审SHA `e2cc748e7234967ca5cafa0e18be30eff78f90deb1d6b66cde11fe950295c24f`。26线程DR恢复及原root退出已证，但登录会话未消失，整体清理未确认；65/67来源逐字节匹配，两个准备PS脚本的CRLF差异独立保留。

本机60普通及唯一原生夹具通过：preNode skipped71、NodeCREATE73→entry148→return150、完整0x4550、同worker caller IL179、原四异常及164事件继续和退出清理均已核；另有36应用诊断特性及21准备入口测试通过。这些绑定本轮明确dirty源码，不能标成7f6冻结执行，也不覆盖真实PS/LSA清理。下一步先在原runner复核固定能力，再恢复真实PS取证；G09仍开放，正常五场景/三冷恢复和最终同源双平台门禁仍待完成。无需本地化变更。

以下接入与固定能力记录按各自历史提交保留：

真实PS接入的 `b1150f4a4` 已合入main `d155f1daf`，本机31项诊断特性测试通过。其运行37349685626在源码检查阶段主动取消，reader及候选均未启动，取消独审SHA `9c782642a6f0bc9039a3d34b8a81220312f2144b34eba6f4626e9b2e479210b7`。原因是旧诊断只绑定Node宿主路径，未绑定候选本代私有DeviceMap根；现已接入同一spawn的原目录/Node租约及已验证映射根，本机warp check和34项诊断特性测试通过。这个诊断缺口不解释原PS首因，仍须取得真实分类返回及同停点CLR栈。

原runner复核已完成：37344743491 / `e4e690dd7e9e6986237d5f9c43d6107dc84f62f7` 的55普通与唯一native全通过，独审SHA `5c4c331f2fd700d77d669c3dee60bb7d47e6de4c9ad845244b39ae1795e7fef5`。与原PS失败同机器、同CLR/DAC字节，SMA不在本固定夹具采集范围。现在可以接入首个真实PS分类返回及同停点CLR栈；G09仍开放。

已在用户Windows工作区从起点 `28d0922519b02b0690626edc148638c0150cda1c` 安全切到实际分支。原45普通项已在本机及原runner37340942346通过；原runner唯一native的DR读回失败保持。受控本地准备入口已建立，不伪造CI环境。本机诊断定位无关CONTROL写入及API/硬件DR固定值视图差异，精准修复后a8的55普通项和唯一native均通过；真实返回0x4550、同停点PInvoke首帧与caller IL160、原四异常及清理全部核验。运行时桩的原E_FAIL/空IL完整保留，未知帧不能通过。

本机CLR/DAC4.8.9345.0与原runner4.8.9310.0不同，固定能力通过不证明原PS首因。原runner已复核这组修复，下一步接真实PS首候选；没有新增关闭项。证据根 `C:/Coding/InfiniShell-Evidence/g09-20261006-a`，a1–a7失败及a8通过原件、原PS工件和模块审计均保留，详见CURRENT_STATUS与VALIDATION_REPORT。以下历史接续顺序的原始约束继续适用；“尚未Windows执行”的旧状态由本段更新。

保存日期：2026-10-06。分支 `codex/cli-agent-parity`，PR [#22](https://github.com/Infinimesh-ai/InfiniShell-Desktop/pull/22) 保持草稿。本文件所在提交包含最新修复；开始时核对实际 HEAD、远端分支、PR 和工作区，不覆盖当地改动。

## 当前完成边界

- 本次 Mac 范围已关闭 G01–G08、G10、V03，共10项。G09开放；V01/V02/V05由用户后续在其他平台做实机验收，移交不计通过。
- 用户允许额外共享写权限、文件保护标志或特殊链接权限安装提示使用原安装工具升级；普通安装及已支持只读ACL的自动升级要求保持。无需再请用户决定这项范围。
- G03真实首拖已由用户确认，不重复要求用户验收。
- G09当前主要问题是实际 Windows PowerShell 候选提前退出1、零输出；不是缺少用户授权或Windows交互桌面。最终同一冻结源码的Linux/Windows源码门禁仍必须满足。

## 最新代码与验证

原生核心 `crates/command/src/windows_shell_classification.rs` 已实现一次精确绑定Node的SHGetFileInfoW入口／返回观察。共享 `windows_clr_reader.rs` 与 `script/ci/g09-clr-reader/reader.cpp` 的operation 3从同一个返回停点读取CLR栈；尚未接入真实PS候选。

本次修复从`8bdbeec00`继续：固定夹具不再把首个任意后代当作测试目标。路径／FileID／SHA及原CREATE身份全匹配者才是唯一目标，其他后代不贡献正例，也必须确认原句柄退出和birth。固定路径身份改变或未知立即失败。核心显式复制所需等待权限，保留原WAIT_FAILED错误码；清理完整drain并按发生顺序保留首错，不能继续的pending立即停止。

已通过本机warp check、i18n11、Windows MSVC普通测试目标及诊断特性metadata检查、格式与源码独审。新源码的45普通项（夹具19／核心13／reader13）和唯一原生项**尚未在Windows执行**；本轮为迁移保存进度，没有派发新CI。

- 本机门禁汇总SHA：`451f0250c8d4db16f9e94322386a4581b12dd898235532162da456bec043e5ad`。
- 源码独审SHA：`eb08ecb6fc496e6334ac51b3484cda1a2ac8b2913f374a53c1c806301c93f845`。
- 无需本地化变更：本次只有内部诊断和固定夹具，无GUI/TUI文案、语义或布局变化。

## 必须保留的原失败

1. [实际PS运行37299690314](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37299690314)，源码`a74ba3c8f38c75e5f8991e4f9b02b1df9db4b3f5`：首updated为ProbeFailed。PS generation `f961a206-028e-4f4c-a1c0-7ab642db1176`在13917ms退出1；Node在12303ms创建、19109ms退出0，Codex在17112ms创建、18746ms退出0。后4场景／3次独立冷恢复未执行。26异常记录为8 observed／18 partial，不能重复原样采集或用可恢复启动异常猜首因。原生ZIP SHA `65c109949aea234133dd0dd9eb0379f7fec9cebb35810b54d7c6d1fd784c1b17`。
2. [固定能力运行37335113246](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37335113246)，源码`8bdbeec004277cb2a3222d00e6d8e9c8dd6f3896`：38普通通过、唯一native失败。seq6发生在根工作线程创建／CLR绑定前，未知后代被复合身份守卫拒绝；实际失配分项未保存，不能给该进程命名。没有分类入口／返回、CLR观察，DR从未修改。根reaped和Job empty为true，但debug drain及后代退出未确认，cleanup_ready=false。独审SHA `d2b261fceccb0d30d248e71556e5b506ada52797a95a14b40fd069362c33e6a8`。
3. [实际模块静态合同37328199520](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37328199520)：新增39方法／9类型采齐，原两个no_body仍使整体failure/partial。合同SHA `d6ae4082fe989adf7622b326bddab86a14deeefa17166578a03d0fd5bdc01baf`。静态分支不是运行时命中的证明。
4. [Linux源码门禁37305012082](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37305012082)已通过独审，5526 nextest、23离线、hook23通过／2跳过；它不解释旧超时，也不替代最终新源码同源门禁。

## Windows 接续顺序

1. 阅读仓库AGENTS和跨平台验证skill。核对Windows x64、仓库Rust工具链、MSVC/Windows SDK、Windows PowerShell5.1及Framework4.8；优先使用已有工具链。比较原工件里的OS、SMA/CLR/DAC版本、MVID和文件摘要；不同机器通过不能自动解释原runner失败。
2. 先验当前固定能力，再动真实候选。远端仍仅通过`.github/workflows/cross-platform-preflight.yml`，范围`windows_atomic_debug_scope=g09_clr_fixture`、仅`run_windows=true`，其它输入保留默认／false。若在Windows本机调试，先建立受控的本地准备入口；现有`script/ci/g09-clr-fixture/prepare.ps1`限定runner，不能伪造GITHUB_ACTIONS环境绕过。保留私有目录、原句柄、Job、期限、预算、45普通合同、唯一native正例和原四异常，不扩相近探针矩阵。
3. 固定能力通过后，把同一能力接入`app/src/ai/cli_agent_runtime/managed_process_atomic_windows_witness.rs`及`managed_process_atomic_windows_clr.rs`的首个授权PS候选。只采已绑定Node真实CREATE之后的分类返回及同停点栈；没有命中保持unknown，不推断UseShellExecute。不再重跑原26异常。
4. 原SMA MVID为`0a210000-3870-4dec-b53e-175f62acb623`。NCP.Complete token100668016在IL0x229调用IsWindowsApplication(token100668021)，后者用flags0x2000调用SHGetFileInfoW。返回0、0x4550或0x5a4d判false，其它判true。必须确认实际栈为Complete中的启动后调用，不能用启动前CalculateIORedirection调用替代，也不能把公开静态实现当成当轮首因。
5. 获得实际分支证据后精准修产品；正常Windows验收保持witness=false，完成updated、old_moved、published_receipt_missing、external_change_preserved、candidate_changed_preserved五场景及三次独立冷恢复。前四场景核CMD/PS真实入口的版本、退出与清理；变造候选必须零进入。权限、来源、树快照和恢复条件不变。
6. 最后运行同一冻结源码的Linux/Windows门禁，再判断G09关闭及PR合并。继续提交、推送、更新PR；只以完整关闭缺口计进度。

## 证据存放

仓库内的KNOWN_GAPS.md、CURRENT_STATUS.json和VALIDATION_REPORT.md可随Git同步；原始Windows工件从上述GitHub运行获取并按摘要核对。Mac完整归档在`/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20261001/g08-build-preparation`，不会自动随Git迁移；不要将大型原件或凭据提交。

Mac观察器临时根`r-ifr8kx2r`因出现归属不明RustDesk launchd标签保留；不能擅自结束该服务或宣称已清理。其它本轮门禁与独审登记根已清理。Mac短TMPDIR规则不硬编码到Windows。
