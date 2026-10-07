# G09：转 Windows 继续开发

## 当前接续状态（2026-10-07）：省去second令牌观察后仍有残留

i9 v2两臂同一新first及原leaf，均ACKoff、controller不观察second；只切换first经原second PI的OpenProcessToken/TokenStatistics/Close整组。A first20440/leaf26188、LUID2861719682/0，open/query56B/close均原BOOL1/error0且同LUID。A完整自然测量与原/post present后才B。B first18056、已绑定first LUID2861735663/0；leaf16744的AuthenticationId与same_luid明确null，三API not_performed，未把原0槽或first身份当成second观察。

两臂124B handoff v2与324B completion v3保留原PI PID/birth/TID、Resume1、自然0、原thread/process关闭。各50条原LSA样本均status0、Size272、匹配对应first LUID及free0，elapsed3031/3016ms；owner自然退出并关闭原柄后的唯一查询仍present/free0。外部second实际映像、精确Job及外柄终态均not_observed。省去令牌查询整组仍有本次残留，没有识别单API或系统持有者。

pair SHA `6a4f2a23a9f02b97bbb8f3f3d5a8bdffc17518c97a16d06718cbe8217060e705`；独审SHA `bc5389b2213c85a68dbe6e60fcd4e683e06436dbaad995247975644269f33923`，146份原件稳定。未执行的v1共43文件已归档，v2只修复解析器在查询失败时仍将auth槽标成meaningful的问题，native字节未改；该失败分支本次未动态覆盖。wrapper0是测量完整，不是清理成功。

下一项仅仓外准备leaf嵌入Windows清单的有界对照，计划保持i9原控制器/first字节、native arm B和token off，并在同一路径一次切换、保全A实际文件；尚未执行，不预判PCA首因或修改服务。原runner8e84正常五场景/三次冷恢复通过但均自动站，本机b1实际失败及d2清理反例保持。最终同源Linux/Windows门禁未完成，G09开放、共10项关闭、新增关闭0，PR22草稿；V01/V02/V05移交不计通过。

详见CURRENT_STATUS的 `g09_second_token_control_20261007`；证据保留，cleanup_ready=false。无需本地化变更：仅证据文档，7203份非文档源码匹配本地check、i18n11和定向20项已通过来源。

## 历史记录：取消ACK写读等待后仍有残留

i8共用本轮新first和原leaf，两臂均为两段、controller均不打开second。A保留ACK写入、读取、等待和逐字比对；其原及owner退出后查询均present、完整自然测量后才启动B。B省去ACK整组，268B completion v2明确记录mode/read_attempted/matched均0，实际无ACK文件或event31；结合完整原PI生命周期才记not_performed。

A first21864/leaf8744、LUID2858694979/0；B first15512/leaf7768、LUID2858706723/0。两臂原PI均Resume1、leaf自然0、thread/process关闭，controller自然0、Job空且原柄关闭。各50条原LSA样本均status0、Size272、正确LUID及free0，elapsed3016/3015ms；owner退出并关闭原柄后的唯一查询仍present/free0。两臂外部second的独立身份、实际映像、精确Job及外柄终态均not_observed，不能用父Job空补证。

pair SHA `ae705dca971e7de7869676661e791ea6e1ae562a6ddcbb71fc2f834f71f16594`；独审SHA `cc451ad7569e052fafc5df956c894bff44a0bc0df1f598733453d60170aaf101`，102份原件前后稳定。wrapper0仅表示测量完整；取消ACK文件写/读、等待和恢复时序整组没有消除本次残留，尚未识别单API或系统持有者。原i6/i7时间窗内已有应用兼容性日志无事件；这不能排除兼容性服务，未改日志或服务配置。

下一项仅在仓外准备second原PI令牌查询整组开关，尚未执行；B身份缺测将明确记录，不补查或回填。原runner8e84正常五场景/三次冷恢复通过但均自动站，本机b1实际产品失败与d2曾清理的反例保持。最终同一冻结源码Linux/Windows门禁仍待完成。

G01–G08、G10、V03仍共10项关闭，G09开放，新增关闭0；V01/V02/V05移交不计通过，PR22保持草稿。详见CURRENT_STATUS的 `g09_ack_wait_control_20261007`；证据保留，cleanup_ready=false。无需本地化变更：仅证据文档，7203份非文档源码匹配已通过本地check、i18n11和定向20项来源。

## 历史记录：省去外层二代观察仍有残留

i7共用冻结i6 first/leaf，first始终两段并保留原PI身份、相同ACK及自然退出/关闭；只有controller对second的额外观察开关不同。A观察开启，在原及owner退出后查询均present、测量完整后才启动B；B省去外部second OpenProcess、完整token/image/精确Job查询及其句柄后续操作。两臂原各50条LSA样本均status0、非空、Size272、返回本臂LUID、free调用及返回0，原elapsed3031/3015ms；owner自然退出并关闭原柄后的唯一查询仍present、free0。

A first11956/leaf30016、LUID2853456782/0；B first29432/leaf16672、LUID2853461019/0。B的second身份、Resume1、自然0及原PI关闭来自first原件；外部独立身份、实际映像、精确second Job成员及外柄终态均明确not_observed，父Job为空不能补足。B确无second-created和对应外部事件。

pair SHA `cd3f504f47a3e28e46db157b6539ef05f71adc8e28db62cffe67443fcdb46e23`；独审SHA `9c4da4aec92ff5622b4f4dfaa21a78dfb0a20b507f3307c3c447fedf817d566a`，92份原件前后稳定。wrapper0只表示测量完整，两臂清理均未通过；该外部观察整组在本次不是残留必要条件，尚未定位单API、ACK或系统持有者首因。d2曾两段清理的反例和b1实际产品失败保持，不用诊断代替产品修复。

下一步只读设计取消ACK等待/读取的有界对照，尚未准备或执行。原runner8e84正常五场景/三次冷恢复已通过但均自动站，最终同一冻结源码Linux/Windows门禁仍待完成。G01–G08、G10、V03仍共10项关闭，G09开放，新增关闭0；V01/V02/V05移交不计通过，PR22保持草稿。

详见CURRENT_STATUS的 `g09_second_observer_control_20261007`；证据保留，cleanup_ready=false。无需本地化变更：仅证据文档，7203份非文档源码仍匹配已通过本地check、i18n11和定向20项来源。

## 历史记录：最小两段及身份握手出现残留差分

i5自身TokenStatistics对照两臂自然退出，原首次LSA及owner退出后唯一查询均不存在；独审SHA `d34beb3855c3b06f3f820ee7ebaefb570038414009dbad30d61dff78a2939119`。B的冻结流程与退出0支持open/query/精确长度/关闭成功，但没有child统计字段或返回LUID原件，不以外层身份补写。

i6 v2两臂仅各一次：A单段first15804、新LUID2849550670/0原首查0xc000005f/null、elapsed0，postowner也不存在；其清理门槛满足后才B。B first3472/birth2269458504-31282748与leaf2512/birth2269694691-31282748，同LUID2849551984/0；120B交接与ACK一致，260B原终态确认Resume1、leaf自然0及原thread/process关闭。controller确认两代自然0、Job空、原second/first/thread/Job关闭后，原50条LSA样本均status0、Size272、返回同LUID，逐条free调用及返回0，elapsed3031ms仍存在；owner自然退出并关闭原柄后的唯一后验也存在且free0。

pair SHA `a1016c60e9a1d1e3dcac2e882889a72ee0779737f6f35b6fd4fc658504996d0a`；独审SHA `43d8e91bae340838a821779d58d2088638f1c8f833e45d76d314958847260d64`，168份原件前后稳定，含未执行v1的42份归档。wrapper0仅表示测量完整，B清理未通过。首末query tick差3016ms，3031包含原日志和粗时钟，不是额外查询；v2快速退出最终读取分支本轮没有动态覆盖。

本轮将复现范围缩到最小两段及必要身份交接，没有显式USER32或窗口对象操作；未排除系统依赖/外部注入。增量仍包含controller对second的独立打开、完整身份/镜像/Job观察及ACK，不能直接归因某个API或持有者，也不能替代b1产品同源复现。d2曾两段清理的反例保留。下一步只准备i7外层second观察整组on/off，复用相同first/leaf及ACK，尚未执行；off缺少的外层second独立观察明确记not_observed。

原runner8e84正常五场景/三次冷恢复已通过但均自动站，本机b1产品失败保持，最终同一冻结源码Linux/Windows门禁仍待完成。已有Unix有界只读ACL支持和Mac验收保持；最终Linux逐名核现有ACL回归，Windows核owner/DACL保持，readonly命令不冒充Windows专项只读ACL验收。

G01–G08、G10、V03仍共10项关闭，G09开放，新增关闭0；V01/V02/V05移交不计通过，PR22保持草稿。详见CURRENT_STATUS的 `g09_minimal_two_stage_difference_20261007`；证据保留，cleanup_ready=false。无需本地化变更：仅证据文档，7203份非文档源码仍与已通过本地check、i18n11和定向20项来源一致。

## 历史记录：三组单段对照可清理，产品缺口仍开放

i2的A不加载、B加载系统USER32并核路径后释放自有引用；i3两臂共同加载并解析GetSystemMetrics，只有B调用一次SM_CXSCREEN；i4的A不写，B唯一新建文件并写入一个字节、flush及关闭。每组共用其冻结child二进制、各臂仅一次，A原LSA和owner退出后查询均确认不存在后才启动B。六代均自然退出0、Job为空、原thread/process/Job关闭，原第一次LSA即0xc000005f/null，owner退出并关闭原柄后的唯一后验也不存在。三份wrapper独立收据均0，没有重试或强杀。

i2 pair SHA `ae9666f68bfbfa52487e9c0b7806e53f19b011b2d04d31ef98f43a1c4fad611b`，独审SHA `ec689ebc0b13fb8716f6e88b62d75514ed5803636835d811e96d47f9503d4b97`；i3 pair SHA `0f0e18b15a6a069397942297f97dec0920beea38f8dce285046c38b34b5ba0ab`，独审SHA `6f16a9e7163bd486f41a835a0b8a99bce0c691b5c1ea58a04ab747c266f7b029`；i4 pair SHA `e524c565011b3183778bf425c60f4f5369c038cf65bed955fb953bf79519c880`，独审SHA `2b543281e84340ece6fcf773bdf05dfb6e6d446cbe3f342a80f0afe8ea9d0e95`。i4单字节0x49及文件身份只在原LSA和postowner之后核验，原文件保留。

三组结果只缩小本次单段路径范围，不是b1产品修复或一般性因果排除。i2的退出0与冻结流程支持加载/路径/释放成功，但不证明模块彻底卸载或运行时映像哈希；i3没有捕获metric具体数值；i4后态FileID不冒称子进程原句柄身份，也不将整体I/O拆成单API首因。静态导入不排除系统或外部加载；null返回没有实测LSA成功buffer路径。

下一步只准备i5子进程自身TokenStatistics查询对照，尚未执行，继续要求A双absence和完整测量后才B。原runner8e84正常witness=false五场景/三次冷恢复已通过但均自动站；本机b1显式站失败及h2等残留证据保留，最终同一冻结源码Linux/Windows门禁待完成。

G01–G08、G10、V03仍共10项关闭，G09开放，新增关闭0；V01/V02/V05移交不计通过，PR22保持草稿。详见CURRENT_STATUS的 `g09_single_child_controls_20261007`。证据保留，cleanup_ready=false。无需本地化变更：仅证据文档，7203份非文档源码继续匹配已通过的本地check、i18n11和定向20项来源。

## 历史记录：最小单段基线可清理，产品缺口仍开放

i1仓外最小基线只执行一次：child静态仅导入KERNEL32!ExitProcess，无CRT、USER32、TLS、CLR、子进程文件收据、TokenQuery或第二段。实际child9676/birth425602446-31282741、新LUID2837222244/0自然退出0，Job为空、原thread/process/Job句柄关闭后，原首次LSA查询即0xc000005f/null、elapsed0；owner23956/birth424940179-31282741自然退出并关闭原柄后的唯一查询同样不存在。wrapper独立收据为0，无强杀、超时或重试。

pair SHA `27fad4f9999ed4ce6bb5e6963752a46d3258fceb8ea6f037528d9683bf1d184b`，独审SHA `4072ae73206aa49f0ce3ec992e20c668d0b25c9af69fbd5ec24513a972c24c1a`，82份所读原件前后稳定。该结果仅说明本次最小新登录会话可清理，不是稳定成功基线或b1首因结论；controller仍有文件I/O和TokenQuery，不能与child同名操作混淆。null返回没有实测返回Size/LUID/free成功路径，coarse tick0不等于绝对零耗时；静态导入也不排除系统加载或外部注入。

下一步仅准备同一child两臂的系统USER32加载对照，尚未执行：A不加载，B仅加载已绑定DLL、核路径并释放本次引用，不调用USER32函数。A原LSA及owner退出后结果都必须已知不存在且测量完整，才启动B。原runner8e84正常五场景/三次冷恢复已通过，但均自动站；b1本机显式站失败、h1失败和h2残留均保持，最终同一冻结源码Linux/Windows门禁仍待完成。

G01–G08、G10、V03仍共10项关闭，G09开放，新增关闭0；V01/V02/V05移交不计通过，PR22保持草稿。详见CURRENT_STATUS的 `g09_minimal_single_logon_baseline_20261007`；证据保留，cleanup_ready=false。无需本地化变更：本轮只有证据文档，7203份非文档源码继续匹配本地check、i18n11与定向20项的冻结来源。

## 历史记录：USER32顺序对照未解决本机LSA残留

原runner22在8e84的正常witness=false五场景、三次独立冷恢复和八代候选清理已通过，全部走自动站；本机b1显式建站后的LSA失败仍是阻塞，最终同一冻结源码Linux/Windows门禁尚未完成。原验收和失败证据均保持，不用后续诊断覆盖产品结果。

h1只执行A：新站创建成功，step11错误5；原收据不能唯一分辨UOI_FLAGS与GetUserObjectSecurity。句柄遗漏读取OWNER/DACL所需READ_CONTROL是诊断入口缺口，但不补造错误子API。second退出3、控制器实际强制回收，原wrapper因object_sd_bound记unknown；B未启动，50次LSA查询/3031ms仍存在。独审SHA `c121fa1819b72b5b8248f5b15567d127f9cf14f70edeb37bd46c58d4fda68a79`，原失败不回填成完整测量。

h2修正该诊断入口后，两臂各执行一次、无重试，共用同一helper，仅改变second的GetSystemMetrics位于共同创建/选择块之前或之后。实际站和桌面ACE分别0x000f037f/0x000f01ff；两代自然退出0、Job为空、原句柄关闭及原站/桌面恢复均确认。两臂各50次LSA返回0/非空，原观测均3047ms，LSA终态未观察到差异；metric自身为2560/3840。pair SHA `83f45ced4659a9894fcae423cd86878504aa25a98416107c461eac033ce88c24`，独审SHA `bc21a715a22d3ae2c7e5502aff5274ce542f3d72e624c9116859057c4402d7ea`。这只完成有界测量，未通过清理，也未定位或排除一般性首因。

下一步仅仓外准备单段、child仅静态导入kernel32 ExitProcess的最小新登录会话基线，尚未执行。h2 first仍读取WinSta0，DLL/TLS及外部系统引用未知；不从静态导入、单对结果或原机自动站成功推定本机修复。产品超时、严格清理、普通安装及已支持只读ACL自动升级要求不变。

G01–G08、G10、V03仍共10项关闭，G09开放；V01/V02/V05移交不计通过，PR22保持草稿，新增关闭0。详见CURRENT_STATUS的 `g09_user32_order_controls_20261007`。仓外证据保留，cleanup_ready=false。无需本地化变更：本轮只有证据文档，7203份非文档源码与已通过本地check、i18n11及定向20项的冻结来源一致。

## 历史记录：原机正常矩阵通过，本机显式站LSA仍开放

原runner22的[37573125367](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37573125367)在冻结提交 `8e84b737eabeefe1ab082dc3be6b6c0fd5067c96` 正式成功，独审确认正常witness=false五场景、三次独立冷恢复和八代CMD/PowerShell候选的输出、退出及清理。updated为0.156.1；old_moved与published_receipt_missing恢复0.155.1；external_change_preserved保留0.156.1并返回RecoveryRequired；candidate_changed_preserved保留0.155.1并在spawn前拒绝。普通warp1159项、command102项通过，i18n11包含在普通回归内。

候选原ZIP为1099138993B、364成员，官方SHA `e0c38d0a8724506b14d5bd2900ef3771d4e63193f610fdf0fa6901886d50c787`；正式审计SHA `4408a9a9997167f4da3ea598c6511278af332a9ec9833981b78bde6af94e3030`，边界复核SHA `228e60e33646b93214f2cf780f76ba9a781bf35d98432a3fff43adba84350c6e`。67份来源绑定实际Git字节，三组独立execute/recover由冻结subprocess调用链及8次调用原件支持；未归档worker PID/birth，不声称逐PID核验。原ZIP/SRI/所用成员可离线复核；未上传的全部二进制、manager树、完整ACL和配置后态依靠成功的运行时断言与摘要，不冒称原件齐全。readonly-*为公共只读命令，不是独立只读ACL场景。

八份ready均 `station_created_explicitly=false`，自动站路径的成功不能覆盖b1普通交互用户显式建站的LSA残留。原b1失败继续保留为本机阻塞，未用原机通过认定本机首因；当前合并HEAD `60c099990f9672af8175fa459ab07252aaa15856` 的最终同源Linux/Windows门禁仍待完成。

g1仓外原句柄观测已执行一次、无重试：三次NTSTATUS0/56B，HandleCount均1，PointerCount原值32762/32768/32754；在原53/54/55关闭前取样、原90之后写盘。原句柄、Job与私有桌面关闭、两代自然退出0后，50次LSA查询仍存在，原预算观测3046ms。独审SHA `b9471968776fef5afb500a367b0c1aafbdf4666eb6cf490bcd8994772422ec31`，72份所读原件前后稳定。此结果只排除三个采样对象当时的额外打开句柄，不解释PointerCount为泄漏量，未定位其他token、内核引用或b1首因；测量完成不是清理通过。

10项关闭、G09开放、V01/V02/V05移交不计通过，PR22保持草稿，新增关闭0。下一项仅在仓外准备第二段USER32调用顺序的有界对照，尚未执行，不据静态顺序修改产品。详见CURRENT_STATUS.json的 `g09_original_normal_matrix_and_local_handles_20261007`；所有原失败及证据保留，cleanup_ready=false。无需本地化变更：本轮只更新证据文档，无产品文案或布局改动，原main合并check/i18n11/定向20项收据继续有效。

## 历史记录：观察对照与main合并本地门禁

e1 复用旧8e84测试程序，无stdio与无写入stdio各一次，原断言均exit101/candidate_independent_station；两代helper自然退出、Job和原句柄关闭后，原三秒及测试进程退出后LSA仍存在。独审SHA `349a2ebf95d91e8a7b0bc85961642fe32aa4bb88ac1d76c99c37d8aec2b5c0f5`。该stdio控制路径不是本轮残留的必要条件，未定位b1产品持有者。

f1 早查询off/on各一次，终态分别50次/3031ms与50次/3015ms仍存在；B早查询返回已绑定LUID、free=0。exit0仅表示观测完成，不是LSA清理或G09通过；独审SHA `c7ea32060582bc7cff2904fb020db18c13398840bb4c6a90a251932bbe11fcd3`。off本身残留，无法给出早查询因果结论；d2 A单次成功不是稳定基线。f1 A还新增一次event98写盘，不声称与d2 A逐条原生调用相同。

已整合main `4907ebd35eb4d60d11f387db4aca2a600f31d622`，保留ConPTY重置与terminal-binding安全路由，新增两项交错回归。索引树 `71f81cbdc329ba4159e645f9dccecfe1588db70c` 的7207份来源前后不变，check、i18n11及定向nextest20项通过。初次nextest因PATH无工具而未启动测试，原exit101保留；后续仅对子进程使用已有且与归档一致工具。无需本地化变更，无新增文案或布局。

原runner22的[37573125367](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37573125367)截至 `2026-10-07T06:57:19.399938+00:00` 为 `in_progress`，只验证8e84。完整收据见CURRENT_STATUS的 `g09_local_observer_controls_20261007`，最新产品结果仍为b1失败。10项关闭、G09开放、V01/V02/V05移交不计通过，PR草稿。正常五场景、三次独立冷恢复与最终同源Linux/Windows门禁尚未完成。仓外原句柄引用计数入口仅在准备，尚未执行；证据保留，cleanup_ready=false。

## 历史记录：桌面对照与原机开始验收

新增关闭0项：G01–G08、G10、V03仍共10项关闭，G09开放；V01/V02/V05移交不计通过，PR22保持草稿。b1首CMD的LSA收尾失败和c3两项原生失败保持，不用后续诊断覆盖。

d1在原caller预创建NULL/CWF_CREATE_ONLY窗口站时返回183、controller exit1；未启动helper、未产生候选新LUID、未到LSA测量，B未执行。不能由此推断新LUID第一段创建同样失败。d2 v1因第二段空lpDesktop不能保证私有桌面干预而未执行并保全；v2使用同一派生helper，两臂各一次、无重试。A两代实际为WinSta0/Default，B两代为WinSta0/本轮nonce桌面；原句柄/Job关闭后，首次LSA查询均为0xc000005f/null、elapsed0ms，controller和两个helper自然退出0。

A本身没有残留，因此该结果仅反对“连接Default足以导致残留”，不是B修复A。与c3相比，controller、stdio keeper/借管道、预建桌面、派生helper及进程时序不同，不能认定单一首因或产品修复；原子新建desktop及对象销毁未分别证明，cleanup_ready=false。只读引用审查未找到新的自有token/process/receipt句柄泄漏正证，不据静态调用顺序更改产品。

d1独审SHA `b99d8bba2c3e11b391b703b55b08b71d778c68233e87e81e4749aa72e347c673`；d2 pair SHA `70498f71a4739f456d6752eb9f23faa29d4ad2d0d58404de13e2329238472969`、独审SHA `ddaab9b25a43b9c6ff7b25f76be90f078dbfd56fe3ec81cbd29b3f5c4cfd8597`。证据根 `C:/Coding/InfiniShell-Evidence/g09-20261006-a/`，细项见CURRENT_STATUS.json的 `g09_local_window_object_controls_20261007`，最新产品结果仍指向b1。

截至 `2026-10-07T06:10:41.427719+00:00`，原runner22的 [37573125367](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37573125367) 为 `in_progress`，固定验证8e84，不替代合并main后的最终冻结源码。仓外e1正在准备复用原8e84测试程序，比较已有无stdio/无写入stdio两项；尚未执行，原断言和三秒预算不变。当前7203份非文档来源与通过check/i18n11的合并内容一致，本次仅更新证据文档、未重跑门禁。正常五场景、三次独立冷恢复和最终同源Linux/Windows门禁均未完成；无需本地化变更。

## 历史记录：合并main本地门禁与c3原始失败

本轮新增关闭0项：G01–G08、G10、V03仍共10项关闭，G09开放；V01/V02/V05移交不计通过，PR22保持草稿。正常 witness=false 五场景、三次独立冷恢复和最终同一冻结源码 Linux/Windows 门禁尚未完成；普通安装、已支持只读ACL自动升级及特殊权限安装的既有决定保持。

本机 c3 使用固定 `8e84b737e` 的原测试二进制，两项 stdio 原生测试各一次、均 exit101。实际两级 helper 使用调用方的交互站，在 `candidate_independent_station` 合同失败；写例原生写入44字节，但均未到 Rust 读取、nonce内容核验或模拟检查。helper自然退出0、Job及原句柄/管道关闭、keeper自然退出且caller对象未变；原三秒、关管道后和测试进程退出后的新LUID仍存在。预测站原本不存在，其 gone 不是实际站已回收的证明。该局部现象无需 Node/Codex/AppContainer 即可出现，但尚不能归因于管道或定位 b1 的持有者。

已整合 `main` 的 `cc855835428b8585baed50d3015f314a4f2848ea`。合并内容的7207份来源绑定 index tree `8854d87d3b52ec0d0456314b7accbf8efa40185d`，`cargo check -p warp` 和 i18n11项通过、来源前后不变。中英文各5425个原键与main新增15键完整保留，变量一致；本轮G09证据无需本地化变更，无额外文案或布局改动。本轮状态文档在门禁之后更新，最终冻结同源双平台门禁仍待完成。

原 runner22 的 [37573125367](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37573125367) 固定验证8e84；截至 `2026-10-07T05:28:33.888035+00:00` 仍为queued，不能记通过或替代合并后验证。下一项d1仅在仓外准备默认/私有首次窗口站两臂入口，尚未执行。证据根为 `C:/Coding/InfiniShell-Evidence/g09-20261006-a/`；详细摘要见 CURRENT_STATUS.json 的 `g09_local_stdio_control_and_posthoc_tokens_20261007`，原失败保留，cleanup_ready=false。

## 历史记录：局部修复通过普通门禁，真实 CMD 收尾仍失败

原 runner22 的 [37556769074](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37556769074) 已结束为 failure。首个 updated 的实际错误已由冻结源码对应到 `SetFileInformationByHandle(FileRenameInfo)` 返回 `0x80070003`，阶段为 inactive_rename；CMD/PS 候选均输出新版本、退出0并完成原生清理。普通测试1153项中1152通过、1失败；失败测试三次真实尝试的错误依次为 rename 0x80070003、open os_error2、rename 0x8007007b。只归档到 Prepared，原机尾部 buffer 字节及回滚终态仍未知，不能以静态几何认定唯一首因。

本机原生合同对照证明未补 NUL 的 FileRenameInfo 可将已分配缓冲区的尾部字节纳入目标名；现在明确补 UTF-16 NUL，提交长度包含 NUL，FileNameLength 不包含它，保持原身份、父目录锁、映像冻结和不覆盖约束。同时仅在实测使用 caller 站的分支显式创建本次新 LUID 私有站，保留原自动站路径；失败的 station 所有权先转交外层，异常回收最多尝试一次，未确认回收不得删除 profile。

本机 a4 的两候选及站/桌面/DeviceMap/LSA 清理已通过，也到达发布和最终版本检查，但公共 PowerShell 查询仍 CommandFailed；原 worker 未保留该子进程原生码。后续固定输入对照分别定位到公共 powershell.exe、官方 codex.ps1 和验收环境 SYSTEMROOT 的 verbatim 路径表示：三者采用同一对象的安全 Win32 表示后，实际公共 PS 查询退出0、输出0.156.1，未修改执行策略。产品改用 dunce 安全表示并复核 canonical/stamp 身份；驱动仅调整 SYSTEMROOT 的同对象表示，特殊尾点/空格、设备名和长路径继续保留原语义。这些事后对照不能补写 a4 原退出码或其未执行的最终检查。

本轮 b1 冻结7198份来源，基于 aa3b314bc09022f87810a5718668fe299f4af3f8 加修复增量，diff SHA为 `465d37db980606c8fba43b80a3f649d85ff01ba62a569eec26694969f3ec5dbe`。Python31、command40、npm52（1项真实 native 显式忽略）、i18n11、cargo check及默认构建均通过。默认正常 witness=false 五场景实际在首个 updated 的第一代 CMD 收尾停止：CMD/Node/Codex/console均退出0，输出0.156.1，205个调试事件均继续；两级 helper 退出0、Job为空、桌面及 DeviceMap 已关闭、站已不存在，但期限内 LSA 会话仍存在，正常关闭与异常清理均 TimedOut。最终受控退出收据为1、cleanup_confirmed=false；产品 RecoveryRequired，driver退出1、worker退出101；未到 PS 候选、发布或冷恢复。不能将窗口站消失当作 LSA 清理成功，实际持有者尚未确定。

本轮源码在 wrapper 运行前后快照及独审截止2026-10-07T04:38:59.963135Z均匹配；5个程序、3个归档程序和2324份 manager 文件的准备绑定与独审后验重算匹配。原 driver 未到末尾复核，这些后验检查不补造产品已完成步骤。原失败、a4、差分对照与 b1 原件分开保留；详细收据、独审及摘要见 CURRENT_STATUS.json 的 `g09_station_publish_repairs_20261007` 和 `g09_original_native_rename_20261007`。这是局部验证，未替代原 runner 或最终同源双平台门禁。

**新增关闭0项；G01–G08、G10、V03仍共10项关闭，G09开放，PR22保持草稿。** 正常五场景、三次独立冷恢复、最终同一冻结源码Linux/Windows门禁尚未完成；V01/V02/V05移交不计通过。特殊权限安装决定不变，普通安装及已支持只读ACL自动升级要求保持。无需本地化变更：既有中英文错误语义适用，无用户文案或布局变化，i18n11项通过。下一步取得本代 LSA 收尾的持有者证据并验证修复；原始证据保留，cleanup_ready=false。

## 历史记录：本机确认第二段使用调用方站，修复当时待实施

`430b8be416ed6593387dfebc7a93c836a4571cd9` 加内部诊断增量在受控本机入口执行一次正常 `witness=false` 的 updated：第二段返回 `new_logon_station_name_mismatch`／`InvalidData`，`caller_station_matched=true`。实际名称匹配调用方而不匹配预期新LUID站；不能仅凭同名认定继承或自动选择机制。候选root_pid=0、调试事件0，尚未查询站的非交互属性/owner、修改站ACL、发送Ready或执行发布；产品仍为RecoveryRequired，driver退出1、libtest退出101。它没有复现原runner的PersistenceFailed。

本轮异常清理仍失败：aborted-cleanup为TimedOut，两个helper退出码为1；预期站查询已不存在但LSA查询仍存在，两侧无查询错误，cleanup_confirmed=false，缺少first-child-exit收据。退出码1不能区分自然失败与异常终止，预期站不存在不能泛化为全部窗口站回收。旧baseline及a1失败原件保留，不将其结果覆盖为本轮通过。静态审查另发现启动失败的station尚未转交外层即析构，清理错误未传递；拟先保留所有权，再返回原启动错误，并避免析构开启第二轮异常等待。

诊断仅保留固定check_stage、error_kind与已读名称的匹配布尔；不输出站名/SID，不新增原生查询、不改变原校验或返回。command29项、npm46项（1项真实native显式忽略）、i18n11项、cargo check及默认程序构建通过。7198份来源在五个门禁、实际运行前后及文档更新前独审均相同，dirty差异SHA为 `d6d603e00f275d08b903016549f2cbd0b9d8e724e5491d44c26a32eaf674771d`；这不是最终合并源码门禁。独审 `local-station-diagnostic-a2/station-name-failure-audit.safe.json` SHA为 `9710dcfee6aea38f2f83e1ed2451c1c2ca9818fe679dbb6e18e600cb211032c6`，三程序原字节已独立归档。细项、各门禁与归档摘要见CURRENT_STATUS.json的 `g09_local_station_classification_20261007`。

原runner22的[37556769074](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37556769074)仍验证提交430b8be41，已知step44普通回归失败，真实npm结果待定，不预填结论。下一步仅对已观察到的caller站分支显式创建严格私有站，保留服务身份已成功的自动站路径，完成失败所有权修复后再验。本机CLR4.8.9345.0与原runner4.8.9310.0不同，未伪造GITHUB_ACTIONS，也不以换机结果补证原失败。

**新增关闭0项；G09开放，PR22保持草稿。** 正常五场景、三次独立冷恢复、最终同一冻结源码Linux/Windows门禁仍未完成；V01/V02/V05移交不计通过。特殊权限安装可提示原安装工具，普通安装与已支持只读ACL自动升级要求不变。无需本地化变更：仅内部诊断，原用户错误语义、双语资源及布局不变；本轮i18n11项通过。证据保留，cleanup_ready=false。

## 历史记录：原机恢复与事务仍失败，安全日志本机门禁通过、原机待验

提交 `d131dd13f3ddb38713e17dd7258241ec901792be` 的原runner22正常验收[37553787071](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37553787071)失败。step44运行1153项，1152通过、1失败、7108跳过；新增恢复测试 `inactive_image_recovery_keeps_stage_frozen_until_backup_is_restored` 在第92行实际三次返回SourceChanged，汇总重复的TRY3不计第四次。step53的command93项通过、166跳过；i18n11项包含在默认普通测试中，不能因此把整个普通门禁记为通过。

step62首个updated仍为 `PersistenceFailed`：0通过、1失败、8260过滤、110.37秒，退出101。CMD与PowerShell候选均输出 `codex-cli 0.156.1` 并退出0，原字节分别为18B/LF与19B/CRLF；201／372个事件全部继续，根进程等到Node/Codex退出，Job、profile、station、desktop、DeviceMap与LSA均有清理成功收据。探测成功不等于更新事务或回滚成功；后续四场景和三次独立冷恢复均未运行。

可用原件只有Prepared前态checkpoint，没有当前持久化journal、OldMoved、Published或after-tree。只读CMD0.155.1和npm prefix来自更新前准备，不能证明回滚后版本。失败可能处于PS observed日志保存、首次rename或OldMoved日志保存，具体API与原生错误码均未知；不能认定首次rename为首因，也不将控制流推论计为动态回滚验收。67份来源仅完成执行前绑定，末尾源码、二进制与manager复核未到，仍不是最终全仓冻结。

独审 `normal-five-37553787071/normal-updated-failure-boundary-independent-audit.safe.json` SHA为 `17c553fbf0b960423d2ddfa1fdcb642dfc8f69f6f7f27a4a08673e483c630d18`：重算56个所用ZIP成员、67份源码及4个官方SRI，1513项为审计检查数。候选工件11453324653共77成员，SHA `287194e0ebc528581e062b3436eb901eaa893929771195241e4bef7bb1358ec2`；官方日志ZIP SHA `ac62da6ff52c80d538de4aa1b2baf175241ca8a48de16cea2961c9a6245d329d`。证据根仍为 `C:/Coding/InfiniShell-Evidence/g09-20261006-a`，当前指针及细项见CURRENT_STATUS.json的 `g09_normal_publish_failure_37553787071_20261007`。

当前工作区仅对两份产品文件补充有限安全日志：journal create/write/sync/persist的phase与原os_error；tree open、parent、freeze、rename原错误，身份失配维度及inactive阶段。原返回值、身份校验和guard生命周期保持；这是定位缺失证据的改动，不是已验证的修复。本轮格式检查、focused46通过（1项真实native显式忽略、8214过滤、0.07秒）与i18n11通过（8250过滤、5.68秒）均有exit0收据，7份来源与Cargo输入前后相同；`npm-publish-errors-check-a1`的cargo check已exit0（1分37秒、14条既有warnings），7份来源与Cargo输入前后一致；收据SHA为 `ec964b1c3b6d2ea1cf5a99510c52138d1eecdf9858cb2976b390f49d804f4356`。本轮安全日志的本机门禁通过，原机仍待验证，不预填后续提交或运行结果。本机真实负例已显示freeze_open原os_error32及rename HRESULT 0x80070005，证明日志可见，不将这些本机负例错误码当作原机失败码。main的38fb011已在本次运行前合入，既有本机合同、504cf65及a4a6等历史证据全部保留。

**新增关闭0项；G01–G08、G10、V03仍共10项关闭，G09开放，PR22保持草稿。** 正常witness=false五场景、三次独立冷恢复及最终同一冻结源码Linux/Windows门禁未完成；V01/V02/V05移交后续平台实机验收，不计通过。特殊权限安装可提示原安装工具升级，普通安装及已支持只读ACL自动升级要求保持。无需本地化变更：仅增加内部安全日志，既有用户错误语义与界面布局不变；本轮i18n11项已通过。原件保留，cleanup_ready=false。

## 历史记录：持久化修复局部门禁通过，原机待验

提交 `504cf6595f600cff6d351313ea83c55565f6ab60` 的原runner22正常验收[37548517669](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37548517669)按witness=false、scope=all运行，仍为failure。首个updated结果由原ProbeFailed转为 `PersistenceFailed`，accepted=false、model_inputs_sent=0。两个公开候选均输出 `codex-cli 0.156.1` 并退出0：CMD保留18B/LF，PowerShell保留19B/CRLF。PS的Codex在10674ms退出、Node在11024ms退出、PS根在11841ms退出，已正常等到Node结束；这证明本轮版本探测通过，不等于updated事务通过，也不补证旧267候选的freshness或原PSI.WorkingDirectory精确值。

可用原件保存了首个Prepared前态快照，未见后续OldMoved记录；上传工件没有持久化事务journal，不能凭这一缺失指定失败API、宣称根因已确定或回滚已完成。原机冻结句柄相关解释仍未确认；下述本机合同证据和已实现的修复不能补证原失败点。下一步合入main后在原机验证修复，完成正常五场景和三次独立冷恢复；不将探测成功或文档更新计作关闭。

本机针对发布/恢复时映像句柄与目录改名的合同补充了真实文件系统基线：4/4通过，保留SEC_IMAGE但原File已关闭时，底层rename实际返回Ok，freeze仍拒绝。因此修复不能只释放句柄后直接改名。基线收据 `npm-publish-tree-baseline-a1.safe.json` SHA为 `4565162a82810fbdb2170b77c9215708e92a4c7c3096c45afd5c6b4e69da76e1`；这是本机合同证据，不证明原37548517669的具体失败API。

当前工作区已实现 `tree::rename_inactive_images`：父目录租约覆盖freeze/drop、原有不覆盖目标且校验完整身份的rename、目标重新freeze；重新freeze失败返回RecoveryRequired。execute把backup映像guard保留到第二棵树发布后；recover把反向移动后的stage guard保留到旧包恢复后，并在删除stage前释放。已有待恢复事务的recover失败统一为RecoveryRequired，继续保持启动保护。新增6项树测试和1项pending目录日志错误回归均包含在focused实际通过的46项中（1项真实native显式忽略、8214过滤、0.07秒）；另有19项启动保护回归（0.20秒）和11项i18n（7.15秒）通过。三轮均exit0，7份变更来源与Cargo输入前后摘要一致。`npm-publish-check-a1`的cargo check已exit0（1分38秒、14条既有warnings），7份变更来源与Cargo输入前后摘要一致；收据SHA为 `4ad6d5631ef47b1d307e7ef5c2ab7bc1d00a75b9920a7b4ce8a7680080fff021`。本轮局部门禁通过，原机修复仍待验证。格式检查已通过，收据 `npm-publish-format-a1.safe.json` SHA为 `e4ba78e7c4bea16e92917dd4df4ae8f33977cb5a70626e04ab439382a09f62c8`。本机增量、收据和验收边界见CURRENT_STATUS.json的 `g09_npm_publish_local_repair_20261007`。

main已前进到 `38fb011d9c374f37016fd31a88c86c974cd04969`（PR23仅修改macOS ARM64发布架构名校验）；已在局部门禁通过后合入。本次合并未变更 Rust 产品或测试输入，原机正常验收和最终同源门禁仍待完成。

本轮默认nextest1146项（含i18n11）与command93项通过，没有运行witness组。CMD的200个、PS的380个调试事件全部继续，两个候选cleanup及station清理通过。67份来源仅完成执行前Git字节绑定；首个事务失败后，末尾源码、二进制和manager复核未到，不能称全仓冻结或最终同源门禁通过。本轮没有新增运行时模块或异常对象取证，原失败与原双阶段证据继续保留。

权威独审为 `C:/Coding/InfiniShell-Evidence/g09-20261006-a/normal-five-37548517669/normal-candidate-failure-audit.safe.json`，SHA `2dc2baca500720c6ca8663e95ac36c6957f97956c0677946f818bcd3c8efb136`，149项审计检查不计为测试数。原件摘要、版本输出字节/摘要、生命周期及当前指针见CURRENT_STATUS.json的 `g09_normal_candidate_persistence_failure_20261007`。此前a4a6固定能力、实际CreateProcess回退、PATHEXT单键未解决原故障及所有历史失败均保留，不重写为通过。

**新增关闭0项；G01–G08、G10、V03仍共10项关闭，G09开放，PR22保持草稿。** 正常witness=false五场景、三次独立冷恢复和最终同一冻结源码Linux/Windows门禁均未完成。V01/V02/V05移交后续平台实机验收，不计通过。特殊权限安装可提示使用原安装工具升级，普通安装及已支持只读ACL自动升级要求保持。无需本地化变更：本轮发布/恢复失败沿用既有settings-cli-updates-recovery英文与简体中文语义，已复核适用，无UI尺寸变化；本轮i18n11项已通过。仓外原件保留，cleanup_ready=false。

## 历史记录：原机双阶段实证CreateProcess回退，产品验收仍失败

同一提交 `a4a6b4ebd8ff03679f2e1c5e4822ce26c2293c08` 在原runner22完成两轮独审。固定能力[37537472413](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37537472413)通过151项普通测试和唯一native一次（1.694秒、无重试），独审263项检查全部通过；263是审计检查数，不是测试数。准备范围13份来源执行前后逐字节一致，原CLR/DAC身份匹配。该固定轮未触发pre-Start异常，也未触发late延迟重放，不能将这两条原生路径记为本轮通过。

真实PowerShell[37538211705](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37538211705)仍为首个updated的 `ProbeFailed`。实际顺序为pre179→initial181→pre-Start CLR182→Node188→late189：Complete IL0x46的local4（soloCommand）为true、PSI local5的useShellExecute为false；IL0x293的local0为true、同帧PSI为true。首个pre-Start CLR栈顶落在原System的 `Process.StartWithCreateProcess` IL0x3f0，Complete调用帧为IL0xf7。结合原冻结IL合同，这证明实际先尝试CreateProcess、随后走ShellExecute回退及跳过等待的路径；尚未证明失败的具体输入或最终首因。Win32Exception的267仅是LastThrown候选字段，freshness=false，不能写成当前错误码；32帧为有界局部栈，不能称完整栈。实际PATHEXT环境字符串未读取，post分类保持unknown。

PowerShell在12517ms零输出退出1，Codex在17334ms退出0，Node在17726ms退出0。418个PS调试事件全部继续；累计98次存活线程DR恢复和27次恢复前已退出线程分别记账。4个reader及精确Job、进程、station和LSA清理已核，但证据仍保留，cleanup_ready=false。该轮默认nextest1146项（含i18n11）、command93项、诊断150项、Node19及Python15/1/28通过；67份产品来源仅在执行前完成Git字节绑定，失败后末尾复核未到，不能称全仓冻结或最终同源门禁通过。

仓外证据根为 `C:/Coding/InfiniShell-Evidence/g09-20261006-a`。固定独审 `fixture-37537472413/independent-fixed-two-stage-audit.safe.json` SHA为 `2fa4fdf5069e5022bec82cd631ea7acd0eecefe8594dde77b30f0f6723e4c04a`；真实独审 `ps-two-stage-37538211705/independent-two-stage-actual-audit-v2.safe.json` SHA为 `6a7b0852ba41999870d544204ad30dd91971e8c92be6e9ab3b2e619fc5d5cbbd`。生命周期和reader构建独审摘要、原件摘要及边界见CURRENT_STATUS.json的 `g09_two_stage_original_result_20261007`。实际审计v1的4项失败源于审计假设错误，v2按有界栈和产品非零退出的真实合同修正；生命周期脚本v1错键及错误说明保留，v2修正。没有据此修改产品源码、覆盖原件或抹去候选失败。

**新增关闭0项；G01–G08、G10、V03仍共10项关闭，G09开放，PR22保持草稿。** 当前工作区已加入最小产品改动：PowerShell调用原shim前Set-Location到已绑定映射根，本机默认构建的10项npm probe测试、11项i18n与cargo check已通过，三份收据及源码前后摘要见CURRENT_STATUS；原机验证仍待完成。提交后直接在原runner执行正常witness=false、scope=all五场景和三次独立冷恢复；固定观察器13份来源未变，复用原机固定能力的来源验证，不重复固定native或诊断witness候选。原PSI.WorkingDirectory精确字符串和267 freshness仍未知，PATHEXT单键修复未解决原故障的反证保持。正常验收及最终同一冻结源码Linux/Windows门禁均未完成。V01/V02/V05移交后续平台实机验收，不计通过。特殊权限安装可提示使用原安装工具升级，普通安装及已支持只读ACL自动升级要求保持。无需本地化变更：本轮调整受管PowerShell工作目录并同步内部验收证据，用户失败仍通过既有本地化错误路径报告，无用户文案或布局变化。

## 历史记录：双阶段读取本机固定能力通过，原环境待验

基于 `6fe81afb56ec4453ab1d9ab637e92d49c505d119` 的本轮增量已实现同一 Complete 方法首次 Start 前及后续返回点的实际读取。pre-op3 使用固定400B双规格一次交付同帧、同方法版本、同代码/map摘要的两张私有票据；initial op4 在 IL0x46读取已初始化的 local4（soloCommand）与 PSI local5，回收reader并恢复原上下文后换装晚期DR1、正常继续，不等待尚未创建的Node。后续点沿用原批准集合。四reader硬上限不变：pre、initial、late各一次，第四槽由early恢复后Node前首个原worker CLR与post分类互斥使用；后者未读时仍保存原生raw，明确CLR未满足，unknown不折成false。

本机a4通过222项command普通测试（9显式忽略、28过滤）和唯一原生夹具一次（1.37秒、258过滤）。实际顺序为pre73→initial75→late首次76→Node77→late重放78→post156，两个op4的固定Boolean均true、PSI均false；四诊断槽与原四异常链分别核验，169原事件全部继续，目标、后代、readers与精确Jobs回收，无pending。独审SHA `fa4b1d70590f25cb362544a1d6222929c8841b6d27f350f76e55e23beb099d52`；详细源码与日志绑定见CURRENT_STATUS.json的 `g09_two_stage_managed_local_20261007`。这些是本机当前CLR的固定能力，不替代原runner与PowerShell验收。

a2、a3失败原件保留。a3实际证明完整方法代码SHA、allocation/base/type/protection/样本字节相同，仅VirtualQueryEx连续页范围从4096增至8192；已仅在托管映射复核中投影到原方法extent，读取前截断样本，继续严格核allocation、类型、保护、完整覆盖和代码摘要，原生分类返回合同不变。新增真实内存页回归验证方法外增长可继续、方法内代码/权限变化拒绝。受控Local准备入口保持，外部启动器只避免WinPS继承PS7模块，未伪造CI环境。应用测试旧字段编译失败也已保留并修正；带特性诊断回归、i18n与cargo check通过。本机缺nextest的失败另存，定向普通回归采用cargo test，远端仍用已有nextest流程。

**新增关闭0项；G09仍开放，PR22保持草稿。** `67089b6`原runner的PATHEXT单键修复反证不变。下一步先验原runner同提交固定能力，再接真实PS核初始与终态及同停点证据；精准修复、正常witness=false五场景、三次独立冷恢复、最终同一冻结源码Linux/Windows门禁仍欠。V01/V02/V05移交不计通过。无需本地化变更：仅内部诊断、固定夹具和证据，无用户文案或布局变化。仓外原件保留，cleanup_ready=false。

## 历史记录：PATHEXT单键修复未解决原故障

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
