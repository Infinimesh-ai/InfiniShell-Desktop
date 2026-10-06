# G09：转 Windows 继续开发

## 当前接续状态（2026-10-07）：PATHEXT单键修复未解决原故障

`67089b6098915c85f5d6947a613f15d391b67134`的[原runner22运行37525407224](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37525407224)已正式失败，只有真实npm步骤62失败。编译检查、1146项warp及93项command普通回归、其中i18n11项和142项诊断普通回归通过，不能替代产品验收。唯一产品变化为PS候选PATHEXT=.EXE；本轮仍为updated/ProbeFailed，libtest退出101。

实际pre181→182取得17744/0x4550，Node190后op4191在原Complete IL0x293仍读到Boolean local0=true及PSI local5.useShellExecute=true，所需API全部成功；post分类仍未观察到。PS在12924ms零输出退出1，Codex17669ms、Node18033ms均退出0，无取消。原PS/Node字节与FileID、实际CLR/DAC身份及SMA/System MVID保持；不是换机器结果。pre/op4各32帧partial；CLR193的28帧完整栈和LastThrown候选不证明异常freshness。新增两条CLR仅体现溢出计数，不能定位到pre→Node或认定Win32回退。

三份官方工件及完整日志已保全；候选ZIP SHA为`6c27ea1660a7bbee90a4ffaaa6e5cba57b0123e09590bfefd996fb87c02c2c82`，90成员CRC/SHA通过。终态独审`5451dffd7c668bdf5d873a416570181102a1b597bb1a7669a5c726ec8913a84f`、op4独审`0228962017edfbec12627ac40aa5e60ebcad7e434632c4a2944168f26ad5b0c6`，路径和完整绑定见CURRENT_STATUS.json的g09_pathext_original_counterexample_20261007。67项来源仅执行前逐Git字节核验；失败后driver末尾复核未执行。全部423个PS/202个CMD事件已继续，无pending，三个reader与进程/Job/站/LSA已核清理；90次live恢复与31个先退出线程分开记录，cleanup_ready=false保留。

下一步拟区分同一Complete首次Start前的PSI初值与现有终态，而非再重采原26异常：IL0x46可复用字段及已初始化Boolean local4能力，但必须有真正pre-Node阶段，不能直接套用等待Node的暂停协议。初始false/最终true才支持后续ShellExecute切换；初始true仍需区分环境与直接分类。该观察方案尚未实现或原生验证，不算关闭。此前“.EXE即可修复”的预测按失败保留，不把本机机制实验当原机首因。

**新增关闭0项，G09仍开放，PR22保持草稿。** 正常witness=false五场景、三次独立冷恢复及最终同一冻结源码Linux/Windows门禁仍欠；V01/V02/V05移交不计通过。无需本地化变更。

## 历史记录：取得真实等待分支，PATHEXT最小修复待原机验证

G01–G08、G10、V03共10项关闭；G09开放，新增关闭0项；V01/V02/V05仍移交后续实机、不计通过。PR #22保持草稿，当前不满足合并条件。

同一提交d7610c5在原runner22先完成固定能力37499069138（124普通、唯一native一次通过），再执行真实PS37517610154。真实产品仍ProbeFailed，但pre180实际返回17744/0x4550，Node185后的op4 186在原Complete方法IL0x293成功读取Boolean local0=true及PSI local5.useShellExecute=true，全部所需读取HRESULT=0。实际分支跳过输出读取、WaitForExit及LASTEXITCODE更新，PS13111ms零输出退出1，Node/Codex随后0；不能再把它表述成没有运行时分支证据。op4仍是32帧partial，首CLR188的28帧完整栈及LastThrown候选不证明异常freshness。

414个PS事件全部继续，三个reader/Jobs和进程、desktop、DeviceMap、station、LSA清理已核；累计78次live恢复与28个先退出线程分开记录。执行前67来源逐字节绑定，失败后末尾复核未运行，不称全仓冻结。独立op4审计SHA 52b33ca3946e5d4bcceca537435087fd3e6031dafbd60d3cc10ac186618d151d；生命周期终审SHA 21d42ccf0a509d46b62e6076aef77b570a99cd9a4bf907b5101eca4b8f65b656，均位于仓外ps-classification-37517610154。

本轮只给Codex npm PowerShell候选的空环境显式加入PATHEXT=.EXE，不继承宿主值。官方PowerShell实现会把缺失/空PATHEXT补成.CPL，使.EXE不在原生可执行扩展列表；[官方代码](https://github.com/PowerShell/PowerShell/pull/9828/files)与本机独立实验支持此机制，但本机SMA不同、原目标PATHEXT未直接读取，不能提前宣称首因完整关闭。官方shim、命令、权限、超时和诊断实现均未改。修复前回归实际失败（扩展列表0项，期望1项），修复后该模块8项及i18n11项通过，cargo check -p warp通过；各原件与摘要见CURRENT_STATUS.json的g09_pathext_fix_candidate_20261007。

下一步在原runner验证该最小修复：应实际观察UseShellExecute=false、local0=false及正常输出、等待和退出码；若不符即否定完整原因假设。随后仍须正常witness=false五场景、三次独立冷恢复及最终同一冻结源码Linux/Windows门禁。无需本地化变更：环境键为稳定协议值，无新增或变动用户文案、语义说明或布局。原失败、原26异常和本轮全部仓外证据保留，cleanup_ready=false。

## 历史记录：托管续点本地固定能力阶段

以下内容保留当时状态；最新结论以上文为准。

## 当前接续状态（2026-10-07）：本机托管续点固定能力通过，原环境待验

G01–G08、G10、V03共10项关闭；G09仍开放，新增关闭0项；V01/V02/V05移交后续实机、不计通过，PR #22保持草稿。当前基于374642b的未提交增量已实现Complete原方法的有界托管续点；本机固定能力通过，原runner和真实PowerShell尚未运行本增量，不改变37469893834的ProbeFailed及首因未知结论。

pre原停点只读取得唯一实际MethodInstance、MVID/token、EnC、IL/native映射、代码摘要和原帧身份，绑定单次DR1；op4只在同一原工作线程/帧和批准IL读取Boolean及PSI字段。若父线程续点先于子进程CREATE递送，精确暂停原工作线程一次，以DBG_REPLY_LATER继续原事件；真实Node身份绑定后平衡自有暂停，在同事件重放后读取。helper两端只接受原始首机会单步的延期资格。首次递送、Node、重放分别保存，不改写时序；应用共用原期限、至多四reader，原首CLR预算与NOT_HANDLED语义保持。

本机a8通过124普通（108跳过）和唯一原生夹具一次（1.478秒、231跳过、零重试）。原链为pre71→73→RF skip74→首次续点75→Node76→重放/op4 77→首CLR155→post156→157→其余原异常158/159/160。op4实际MVID 6dfc3e1e-8933-42c8-8a7c-387588962a72、MethodDef100663299、IL162、37项映射/737字节extent；Boolean local4真实类型System.Boolean、位置1、字节1、值true；PSI local2实际引用8字节，System MVID/TypeDef/FieldDef与getter合同匹配，useShellExecute=false。引用GetType/GetName未请求，HRESULT保留E_PENDING。自有Suspend前计数0、Resume前计数1、最终不再拥有暂停；170事件全部继续，原进程/后代/readers/Jobs均回收。独审managed-continuation-a8-audit.safe.json为168947字节，SHA 593f0b4c640ae84e9780ab60f94a651db3eb74d84d6dd081fb050c6f5881e36e。

固定分类方法明确使用NoInlining|NoOptimization，准备收据核实际MethodImpl flags=72，不能外推真实PowerShell的优化后局部可读性。a5的ref和a6的按值参数在IL153均返回Boolean零位置；a7调用返回后的IL162中Boolean与PSI均零位置，全部为unknown/null并失败，不能将其当false或首因。三轮独立失败审计与原件保留；a8仅证明明确固定编译条件下的读值能力，真实PowerShell不改优化、不force JIT、不写IL。a6应用测试编译的两处闭包生命周期错误已修，失败原件保留。

本轮command特性check、Warp check、应用诊断回归和i18n门禁已通过，详细计数及日志摘要见CURRENT_STATUS.json的g09_managed_continuation_local_20261007；20份变动来源和明确测试二进制/reader/fixture前后绑定不等于全仓冻结。无需本地化变更：只有内部诊断、夹具和证据，无用户文案、产品语义或布局变化。仓外证据根C:/Coding/InfiniShell-Evidence/g09-20261006-a，cleanup_ready=false，原失败与26异常均保留。

下一步提交后先验证原runner新固定能力，再运行同提交真实PowerShell，依据真实分支与同停点证据精准修复；正常witness=false五场景、三次独立冷恢复及最终同一冻结源码Linux/Windows门禁仍未完成，合并条件未满足。

## 历史记录：374642b原runner固定能力及真实PS失败

以下保留当时原文；其中“尚未实现”“当前”和“下一步”只描述该历史阶段，最新状态以上文为准。

## 当前接续状态（2026-10-06）

G01–G08、G10、V03共10项关闭；G09开放，V01/V02/V05移交后续平台实机验收、不计通过，PR保持草稿且不得合并。同一提交374642b的原runner固定37469139712通过88普通及唯一native；真实PS37469893834仍为ProbeFailed。pre179→180取得完整17744/0x4550及局部SMA栈，Node185后首异常186取得28帧，首帧映射到ThrowInstruction.Run的rethrow抛出点；ExitException仅为last-thrown候选，freshness未证明。post分类仍0/unknown；PS12228ms零输出退出1，Node/Codex随后0。67来源仅完成执行前逐字节绑定，失败后全套复核未运行；事件继续及清理已核。新增关闭0项；后续直接观察Complete启动后等待决策的只读设计尚未实现，精准修复、正常witness=false五场景、三次独立冷恢复及最终同一冻结源码Linux/Windows门禁未完成。

同一提交 `374642b149c9f38e13d9ea6702a965ab4be0885b` 的[固定能力37469139712](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37469139712)在原runner22/同CLR-DAC通过88普通（114跳过）及唯一native一次（1.373秒、201跳过、无重试）。实际链为pre72→74（完整17744、caller IL70）→RF skip75→Node76→首异常153（operation1、Win32 native code1234）→reader回收/同停点恢复→post154→155（完整17744、IL243）→其余原异常156/157/158。原四CLR异常均NOT_HANDLED，169事件全部继续，21次累计live DR恢复、dirty0，原进程/后代/reader/Jobs清理通过。13准备来源与Git逐字节匹配且前后一致；固定能力不代证真实PS成功。独审 `fixture-37469139712/independent-fixed-audit.safe.json` 为102108字节，SHA `e04d3613409b3c9ffe46e96a73076323835db358a75815e2073174e86635da26`。

随后同HEAD的[真实PS37469893834](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37469893834)在原runner22、job112290389147终态failure，仅首updated为ProbeFailed，model_inputs_sent=0。pre179→180完整u64=17744/0x4550；Node185后原worker首异常186为CLR `0xe0434352`、first-chance=1、HRESULT `0x80131501`。operation1取得28帧真实module/MVID/token/IL，SMA MVID `0a210000-3870-4dec-b53e-175f62acb623` 下首帧MethodDef `0x060035e8`/IL24映射到 `System.Management.Automation.Interpreter.ThrowInstruction.Run`：原合同IL0x13为newobj RethrowException，IL0x18为throw。reader的单个 `last_thrown_object_candidate` TypeDef `0x02000238` 按同MVID合同解析为ExitException，freshness未证明；不能将它等同于本次当前抛出的对象，不能据此认定首因。名称与IL语义来自原静态合同，运行时返回的是身份/token/IL；不把静态SMA文件SHA当作本次加载字节SHA。

28帧中没有Complete/Process.Start/Console，不能从缺帧推出未执行；末三帧IL=4294967293为特殊映射值，不当普通IL偏移。post分类selected/returned仍为0、entry/return为null、结果unknown。seq186的DR采样仅证明原root当前事件线程在该停点的地址/配置相符，不证明全线程或整个区间持续性。两个reader（pre180与首异常186）均按原身份回收后恢复，未重新逐项采集原26异常；last-thrown对象链完整不代表tracker完整或当前对象已证。

实际时序为Node11076ms创建，PS12228ms退出1且stdout0字节，Codex15337ms创建/16964ms退出0，Node17337ms退出0，无取消。PS415、CMD198事件received=validated=continued，pending为空；两代原进程/后代、reader/Job、desktop、DeviceMap、station及LSA清理通过。pre和首异常reader前live DR恢复累计56次，最终29线程以原进程退出确认、dirty0，不能称29次live读回恢复。

67份来源与374642b原始Git blob逐字节相同，仅证明执行前绑定；失败后末尾source/binary/manager复核未运行、summary.safe缺失，不称执行后67来源或全仓冻结。reader构建13来源前后一致、11份归档源及实际180/186所用reader原件摘要已核；未归档worker/supervisor/node/npm原件，不冒称离线重算。实际LOAD、180与186读取器的CLR/DAC FileID/大小/SHA均与原失败一致，版本4.8.9310.0；SMA运行时字节SHA未核。

仓外根 `C:/Coding/InfiniShell-Evidence/g09-20261006-a` 保留全部旧失败、原26异常和本轮原件；真实终审 `ps-classification-37469893834/independent-terminal-audit.safe.json` 为176680字节，SHA `92edf1339fc02bd0b1ef06f0a006600771d6a3b438d4e5119c0ebd3afd4a7468`，cleanup_ready=false。首异常186完成收据SHA `61c330fd325300b4a3ad8cb0ed4472b901392072bf483bcddd010b8f91abaa77`；名称/IL解析使用原 `original-evidence/members/11353475918/contract.safe.json`，SHA `d6ae4082fe989adf7622b326bddab86a14deeefa17166578a03d0fd5bdc01baf`。

下一步需直接观测NativeCommandProcessor.Complete在Process.Start之后的实际等待决策；不能从静态路径、Node CREATE、未命中post或缺失栈帧推断原PS首因。后续只读动态设计尚未实现、未执行，不列作已有能力；不能调用目标方法、force JIT、写IL或重采原26异常。先由真实停点与实际可用映射决定最小观察范围，再精准修复；正常witness=false五场景、三次独立冷恢复和最终同一冻结源码Linux/Windows门禁仍待完成。

无需本地化变更：本次只同步内部诊断证据与状态文档，没有用户文案、语义或布局变化。既有本机88普通、command特性check、warp check、i18n11、应用诊断54与唯一native收据保留；不作为G09关闭依据。

## 历史记录：374642b提交前的本地门禁与f3b失败

以下保留当时原文；其中“当前工作树”“待验证”“下一步”仅代表该历史阶段，最新结论以上文为准。

G01–G08、G10、V03共10项关闭；G09开放，V01/V02/V05移交后续平台实机验收、不计通过，PR保持草稿且不得合并。f3b原runner固定77普通及唯一native通过；真实PS37455611629仍ProbeFailed：pre173→174返回完整17744/0x4550并取得同停点SMA局部栈，Node179后post返回仍未知。PS12406ms零输出退出1，Node/Codex随后0，原CLR/DAC身份和退出清理已核。当前工作树新增单次postNode原worker首异常诊断；command普通88、command特性check、warp check、i18n11、应用诊断54与唯一native全部通过，原runner当前增量仍待验证。八源码/测试exe前后绑定不代表全仓冻结，不继承f3b真实候选通过。新增关闭0项，精准修复、正常witness=false五场景、三次独立冷恢复及最终同一冻结源码Linux/Windows门禁未完成。

当前工作树已实现单次postNode原worker首EXCEPTION诊断：core独立一次性票据在原停点核身份并撤下全部自有DR，合法CLR首事件复用operation1，reader与精确Job回收后在同停点复核原完整上下文与期限再恢复观察；非CLR或不合法首事件记unknown并消费该槽，不追逐后续异常。pre/post分类各一次及原选择/成功条件不变；应用三槽共用原deadline，至多启动三个reader，不重采原26异常。唯一固定夹具改为pre→Node→原Win32(1234)异常→post→原其余三异常，沿用四异常而不扩矩阵。本轮command普通/特性check、warp check、i18n、应用诊断特性测试及唯一native全部通过；原runner当前增量仍待验证，这些本地限定门禁不作为产品修复或缺口关闭。无需本地化变更：仅内部诊断、夹具和证据，无用户文案、语义或布局变化。

同一历史验证提交 `f3b7476a4df2bf4b3c32f5867073cd51c1e0638f` 的[固定能力37454947662](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37454947662)在原runner22通过77普通及唯一native（1.338秒），独审SHA `80292e174f9edf4cf1aa9d70f1bfa1069dfb59c58394a7a9e7cc3f90193de361`。随后[真实PS37455611629](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37455611629)在同一runner/HEAD终态failure；本次固定能力不代证真实候选成功，更不代证当前未提交诊断增量。真实终审 `C:/Coding/InfiniShell-Evidence/g09-20261006-a/ps-classification-37455611629/independent-terminal-audit.safe.json` 为141703字节，SHA `c98724e210ee0e240b851a57393e18db353d4130ff31e24fa49767c0d7197fb5`。

PS generation `e7631c2c-a9fc-43c6-9fed-dab864b39bf5`，root30224/birth134357632352062392、原worker12004/birth134357632408728120。pre entry173→return174，flags0x2000、路径匹配，完整u64及low32均17744（0x4550）；原执行上下文不变、寄存器恢复确认。174同停点reader真实加载配对DAC、核原身份；读取3893484字节/3331次、未耗预算，结果为partial/S_FALSE、32帧，不能称完整栈。SMA MVID `0a210000-3870-4dec-b53e-175f62acb623` 下实际token/IL为 `0x06001275/8`、`0x0600127b/172`、`0x06001270/0`；同MVID/token与原独立静态合同解析为IsWindowsApplication、CalculateIORedirection、Complete。首PInvoke stub保留E_FAIL及空IL，其余31元数据帧映射成功；方法名称不是reader当场读取，静态SMA文件SHA不冒充本次加载字节SHA。

Node4204/birth134357632456878607在seq179创建；seq180仅证明原worker当时DR地址和配置相符（DR6=4294905840、DR7=1025），不证明其他线程或整段执行期间持续有效。post selected/returned均0、entry/return均null；first_skipped_entry为null、skip/RF计数均0，本次pre已被独立选择，不能要求出现固定夹具的故意第二次pre。没有post分类返回、同停点post CLR或Complete未来JIT分支命中，结果仍unknown；不能从Node创建推Process.Start正常返回，不能从未命中推UseShellExecute或原PS首因。

真实时序为Node11377ms创建，PS12406ms零输出退出1，Codex15537ms创建/17276ms退出0，Node17680ms退出0；无取消。PS405、CMD200个调试事件received=validated=continued，pending为空，CMD输出codex-cli 0.156.1。两代manifest/launch/exit摘要闭合，原进程/后代/Job、reader、desktop、DeviceMap、station及LSA清理通过；pre前累计25次live DR恢复，最终26线程以原进程退出确认、dirty0，不能称26次live读回恢复。原26异常仅保留16条摘要及10条省略计数，未重新逐项采集。

实际原LOAD与174 reader的CLR/DAC、PowerShell/Node清单身份均与原失败相同；CLR/DAC版本4.8.9310.0，CLR SHA `8aa62b9be054d79b6575bef1dcedcb2a707741e0245ec876af4fa0dab71594ff`、DAC SHA `b53550c6288be21fe6e485b17de71b191e7cd756635763b696db3d11b38180ef`，FileID、大小和摘要逐项相符。67来源与f3b原始Git blob逐字节一致；reader构建13来源前后一致、11份归档源已核，reader实际174摘要与构建工件相符。失败后末尾source/binary/manager复核未运行、summary.safe缺失，不称执行后全源码冻结。未归档worker/supervisor/node/npm原件，不冒称离线复算其二进制。

原npm ZIP SHA `926d2f9c2716fa641bdc0dcbac8bf339021280f1de03f44de6db4b931bdfe34b`，82成员全部CRC/SHA通过；其余工件与日志摘要见验证报告。所有旧失败、原26异常和本轮唯一原件保留；本轮本地门禁已完成，下一步由主代理决定原runner固定能力及真实取证，不能据代码接线关闭G09。

本轮已核本地command普通a2共88项通过/114跳过、正确 `native-probe-witness` 特性check a2、`cargo check -p warp` a1及i18n a1共11项通过；八源码与唯一native使用来源逐份相同、各门禁前后摘要不变。应用诊断特性测试a1已54/54通过、8305筛除、4.734秒、exit0，八来源前后不变；本轮全部本地门禁通过。原runner当前增量的固定能力及真实PS仍待后续验证。

综合本地门禁收据 `C:/Coding/InfiniShell-Evidence/g09-20261006-a/post-node-exception-local-integration-gates.safe.json` 为8166字节，SHA `424f90b2387074c042f8acb4f5382cbaac96460c4e5379df2a76856563260f20`；逐份日志、收据和八源码当前值一致。应用诊断完成收据SHA `6d0baae244bee0fba1f6d026db2939ab735ceee65ce92103abcdab6db66e983e`。这些本地结果不回填后续提交SHA，不代表全仓冻结、原runner或真实PS通过。

唯一native a1实际执行一次，1.206秒通过/201跳过、无native重试。真实链为pre entry71→return73（完整0x4550、caller IL70）→RF skip74→Node75→原worker首异常151（operation1、Win32 native code1234）→reader回收并同停点rearm→post entry152→return153（完整0x4550、IL243）→原其余三异常154/155/156。四CLR异常全部DBG_EXCEPTION_NOT_HANDLED，166事件全部继续，21次累计live DR恢复、dirty0、原root/三后代/reader及Jobs回收。独审 `post-node-exception-a1-audit.safe.json` 为139520字节，SHA `623c405eaca8d2a1f7c5f5d17dfe68499f32bf6bec33ae265bbb23369868ad6d`；精确测试exe SHA `ae23a9696a3b3fa7199f4210bdae18070e097279dbff62748525549283b6ecf7`。13准备来源中只有Fixture.cs相对f3b不同，本机CLR/DAC4.8.9345.0不代原runner4.8.9310.0或真实PS；八源码/测试exe绑定不代表全仓冻结。

普通a1六处union访问E0133已修，错误command特性名的check a1在命令层exit101；两份失败原件保留、不计通过。当前源码没有新冻结提交SHA；正常witness=false五场景、三次独立冷恢复、精准产品修复及最终同源双平台门禁仍未完成，G09开放、新增关闭0项，无需本地化变更。

以下为此前阶段按各自提交保留的历史记录；其中“下一步”“待验”不覆盖以上当前状态。

本增量已接入独立一次启动前分类返回及同停点CLR；先恢复全部自有DR、回收reader，再以一次性原停点票据核身份/完整上下文及期限后重新布点。启动后选择条件、一次预算和验收字段保持，新增采样只证明首次受支持root活停点的当前事件线程。唯一夹具扩展同一条执行链，不增加原生用例矩阵。本机77普通、command特性check、warp check、i18n11和应用诊断45项通过，八份修改来源在每项门禁前后相同，综合收据SHA `80d693dc1a242955a6959ddee40f87e1262de540d9f44d7f1abe0a82a949c81b`。

唯一native实际执行一次，1.615秒通过：pre entry71→return73（完整0x4550/IL56）→reader回收后rearm→RF skip74→Node75→原worker DR采样/entry150→post return152（完整0x4550/IL198）→原四异常153–156；166事件全部继续，14次live DR恢复，原进程/后代/reader及Job回收。执行文件与八源码前后摘要一致，独审SHA `b10413dd0ef38b22973d1074548f763461f2dfc0cb5ffd04f1bb3f80a176e79b`。本机准备a1因继承模块路径缺WinPS Utility而在任何native启动前失败；原件保留，a2仅在受控子进程绑定已核系统模块后准备成功。13份准备来源中修改的Fixture.cs与02fb不同，明确记为本机dirty工作树；本机CLR/DAC4.8.9345.0不能代证原runner4.8.9310.0或真实PS。

下一步在原runner先复核该提交固定能力，再取同提交真实PS返回/CLR和DR收据。启动前证据不能替代Complete中的启动后门禁；首次未命中原因及原PS首因仍未知。G09开放，新增关闭0项；正常witness=false五场景、三次独立冷恢复、精准修复和最终同源双平台门禁仍待完成。无需本地化变更：只有内部诊断、夹具与证据，无用户文案、语义或布局变化。

以下记录按各自提交与当时范围保留。

同一冻结提交 `02fb1cde5e20fc7863927bc720e9d796821646b3` 的原runner固定能力37442360399通过64普通及唯一native；真实PS37442785591仍失败。首次未选择调用seq182明确在Node187之前，flags=0x2000且路径匹配；启动后selected/returned仍为0，未取得分类返回或同停点CLR。实际PS/Node与原LOAD CLR/配对DAC身份和原失败相同；后者不代表reader已加载DAC或取得SMA栈。PS在9414ms退出1且零输出，Codex/Node随后退出0；420事件全部继续，30线程原退出确认，CMD/PS清理通过。67来源逐字节绑定，终审SHA `0620ef990ad0896908e4b5942753a6a9ec6b0db8eb0e2a77ad6d7df90cbc7aed`。首因仍未知，G09开放，新增关闭0项。

下一增量单独观察首次启动前分类返回及同停点CLR，并在读取器完全回收后恢复启动后观察；仅采一次支持的原root活停点DR配置。启动前证据不替代Complete中的启动后分类门禁，不从无命中推断没有执行。保持唯一原生夹具和四异常，不重采原26异常。精准修复、正常witness=false五场景/三次独立冷恢复及最终同源双平台门禁仍待完成。无需本地化变更。

以下记录按各自提交与当时范围保留。

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
