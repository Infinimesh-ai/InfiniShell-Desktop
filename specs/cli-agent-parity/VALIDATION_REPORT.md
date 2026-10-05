# CLI 支持与能力对齐：验证结论

## 2026-10-05：四异常定位器取得部分字段，完整原生能力仍失败

精确 `9578948db948248747f1c46b5290ce6fdf0c947b` 的 [37272950186](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37272950186) 为11普通PASS／唯一原生FAIL，4份reader均partial。原生43/44两个相同HRESULT停点取得1234/5678，仍在_HResult/inner遇到E_INVALIDARG及读取预算不足。45/46的根类型正确，内链却呈声明类型System.Exception，不能冒称实际派生类型；拆分结果确认context均S_OK、部分IL mapping为E_NOINTERFACE。原fixture自然退出0、53事件均Continue、所有reader及Job已回收，无pending；这些不代替完整字段/帧断言。14源码、7构建输入、34官方ZIP成员及原日志独审SHA `94acfa2d8c652a8137a7d7d5b7fd82890af8a863278a9dac7bafd74257022d32`，远端证据cleanup_ready=false保持。

固定官方源码与本轮原件共同确认GetAssociatedValue保留声明类型。后续需从实际MethodTable核动态类型和限定字段，不读异常消息或正文。另修正两个明确栈调用问题：CreateStackWalk初始化可使用旧filter context，故以原hThread的CONTEXT_FULL重置读取器内的walker；Init/SetContext2已定位首帧，应先读再Next。SetContext2只修改本地walker状态，目标写接口继续拒绝；不加入IP减一回退，不以这两处静态问题冒称解释全部映射失败。[固定官方实现](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/stack.cpp)。

固定官方 `GetFieldDescData` 经 `MetaSig`/`GetSig`/`GetMDImport` 整块载入模块metadata，失败会返回E_INVALIDARG；不通过猜测私有FieldDesc布局绕开。原4MiB只是新诊断读取器的工程参数，不是产品关闭条件。已有System.dll 3,541,424字节、mscorlib.dll 5,445,664字节与本轮所见MVID对应；两份完整IL文件大小加原4MiB其它读取量及1MiB夹具上界合计14,229,968字节，据此一次性选择16MiB固定诊断额度。未取得当轮metadataStart/Size，MVID也不代替当轮完整文件SHA，不声称16MiB已经实测通过；PowerShell的SMA不在本夹具容量规划内。8192次调用、4层异常、32帧、所有绝对期限、当前对象/完整链/同帧MethodDef与IL/回收断言保持。

本机Windows目标command类型check（`r-lk84e_2p`）、warp check（`r-9nkols20`）、build-reader的PS7静态解析（`r-0o2v4agh`）、C++既定格式检查（`r-veyn5eow`）通过并完成短目录清理；不代替Windows MSVC编译和Framework原生行为。

**新增关闭0项，G09开放；无需本地化变更。真实PS故障及最终同源门禁仍未完成。**

## 2026-10-05：真实CLR定位器反证与Linux外层超时

Windows精确 `112adcc74998b43663aba8b6c0645eed9da23d39` 的 [37271419406](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37271419406) 仍正式失败：MSVC `/W4 /WX`、PS5.1准备和7普通项通过；原生单项失败，三次CurrentExceptionState读取为空／前一Win32／前一InvalidOperation。实际目标与原线程出生绑定、54次Continue、reader和fixture退出／Job空已核；这些不代替当前对象、内链和方法帧验证。官方32成员ZIP、源码及二进制绑定已独审，收据SHA `e44d71fb034c937b74366032f968b172a5bc63161ee1426fe77dc136129ff428`。

固定官方源码中Current读取EH tracker；last-thrown在RaiseException前更新。修复只把后者明确作为候选来源，保留PARTIAL与tracker不完整，不能清位或仅凭相同HRESULT宣称当前对象。夹具强化为Win32(1234)→catch内新Win32(5678)→InvalidOperation[5678]→Application[InvalidOperation,5678]，四次独立方法与原事件严格有序；实际字段、链尾、帧、预算及回收仍须通过。[固定DAC合同](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/debug/daccess/task.cpp)、[固定抛出顺序](https://github.com/dotnet/runtime/blob/5535e31a712343a63f5d7d796cd874e563e5ac14/src/coreclr/vm/excep.cpp)。公开CoreCLR不等于本机Framework二进制，后续真实夹具不可省略；尚未接入PS。

Linux精确74dd的 [37267752920](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37267752920) 正式失败。5525 nextest PASS；hook22ok/1ERROR/2skip。唯一失败为`test_failed_tmux_query_never_falls_back_to_outer_terminal`外层Node超过原8秒；preload与原生trace均missing，不能证明worker已进入。旧refreshed用例本轮ok，另一真实main正例的18阶段及46.648005ms只属于该次成功发送，不解释b5旧失败。33 Git来源、2官方ZIP与完整日志已核；未归档该轮Node/worker原二进制SHA，不能事后补证失败阶段。终审SHA `1c1e8d7940fb8d4f51db3c856597edaa15aa644eefe057306b43d1136db65f5b`。不为此重复相近探针或改期限。

本机Windows目标类型check（`r-d4imb4or`）、warp check（`r-c83yje47`）、workflow18项（`r-0455ftye`）、PS7静态解析（`r-j14_hn8p`）及原显式C++风格检查（`r-frbcx9a_`）通过。新增同HRESULT旧对象、乱序、隐藏PARTIAL及祖先IL替代本次throw帧的4项普通拒绝回归仍待真实Windows执行，不冒称原生已通过。

**本轮新增关闭0项，G09仍开放；无需本地化变更。最终同源双平台门禁未完成。**

## 2026-10-05：固定 CLR 读取器 MSVC 类型失败，原生夹具未执行

精确 `bd6ecb498bc8e36affd1ffa7c34f027e282aea9a` 的 [37269594308](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37269594308) 正式失败。Windows类型门禁及7项普通测试各一次通过；MSVC `/W4 /WX` 在`reader.cpp:419`报C2664：`IXCLRDataExceptionState::GetFlags`需要`ULONG32*`，而`Result.state_flags`是`ULONG`。固定官方头与7份构建输入的前后／副本SHA全部一致，没有生成reader.exe，prepare／真实CLR夹具均未执行。

现仅将字段类型改为`ULONG32`，保留官方头、警告等级、候选期限与全部固定夹具断言。原完整日志SHA `f6b7e73f315a1bd2d0d54367f35b9d94c5a9eb0cbcf488a3e0d027773bb9a2bb`；官方ZIP 11328401931 的API大小、SHA及全成员CRC一致，SHA `ce17e5fc191d0f3f173d196564532a58c8a8abc96aa5841937f17467c8c9fdb5`。原件位于仓外 `g08-build-preparation/g09-clr-fixture-bd6ecb498-ci`。修复须经新源码Windows复验，不将编译修正、7普通项或该诊断夹具计为G09关闭；**无需本地化变更**。 独审SHA `813e2d83ad5a27bb4daeb53eb1b7a66665729f886ce5835ade678d5953a00be6`。本机 `r-25410g6b` 的 `cargo check --locked -p warp` 通过；格式检查沿用原显式LLVM风格（110列及既有短语句规则），`r-ei6gwn9c`通过。默认格式检查`r-75ltj6v1`命中既有样式，保留失败且不改无关格式。

## 2026-10-05：Windows 固定 CLR 原生读取夹具准备，尚未执行

以 `74ddcaa42` 为基线，仅新增内部固定 Framework 4.8 夹具、只读 DAC reader 和独立工作流范围。原调试停点的进程／线程句柄、出生身份、CLR/DAC 原文件及版本绑定后，读取当前异常链、Win32 原码和有界方法帧；固定 C# 工作线程的三层异常必须与实际 MVID／MethodDef／IL 逐项对账。reader 已退出且精确 Job 为空后才能继续原事件，失败保留首因和 pending；不改变真实候选的期限、命令或安装权限。

本机 Windows 目标 `cargo check --locked -p command --tests --target x86_64-pc-windows-msvc` 通过（`r-38o8h37r`，日志SHA `61d36e0c85e23e92031f30e6d27398078c8f22cc3fe93871afd67b355cd0b431`）；`cargo check --locked -p warp` 通过（`r-8_40sk0o`，日志SHA `47ad3c6a8f0e8d02481e8743929dcde2730edac5e383dbb826aeab729d399574`）。工作流18项和actionlint通过，本机PS7仅解析两份PS脚本。新 `g09_clr_fixture` 保持25输入，仅选择独立Windows，先普通7项、再固定ignored夹具一次，无重试，并保留失败原件。尚无MSVC编译或真实CLR读取结果；夹具通过也不等于原PowerShell失败已修复。**无需本地化变更，新增关闭0项，G09及最终同源双平台门禁仍开放。**

## 2026-10-05：Mac 显式空 ACL 真实更新与独立冷恢复通过，G09仍开放

精确 `74ddcaa42ada3e3f7306adab1fd5b7b69df30d13` 的 `r-f6ta9tcm` 使用版本化 `readonly-inherited-empty-v1`，仅运行 `updated` 与 `swap_receipt_missing`。前者真实公开入口升级至2.1.280；后者保留交换意图后，由独立新libtest冷恢复2.1.278，完整after与before相同。两场景before／prepared／after的README均精确保留 `{"format":"MacV1","extended":{"flags":0,"entries":[]}}`，包根及两CLI文件仍是非空只读ACL；同路径uid／gid／mode／ACL逐项相等。没有新增相对路径，不将此结果计为新路径继承验收。

独审逐一核对19份精确Git blob、四份官方tgz的SRI／完整成员尺寸／SHA／mode，以及完整归档的签名后worker和supervisor原件。两代原候选均输出22字节 `2.1.280 (Claude Code)\n`、退出0；manifest／launch／exit／coalition身份和清理绑定闭合。冷恢复278由同源真实公开入口检查、产品收据及官方完整树计证，没有单独归档其公开stdout，不冒充额外stdout原件。私有配置保持不变，未执行npm install或生命周期脚本，也未发送模型输入。

归档位于 `g08-build-preparation/g09-claude-npm-empty-acl-macos-runs/r-f6ta9tcm`：独审 `independent-native-empty-acl-review.safe.json` SHA `7571567a5e135478b59ae0ce92f734603d0021c1b3edafa419f95d5a53e5a9d2`；driver收据 SHA `8121e01a34bd1f657492016c57968607a80f59a2a3951c4675a779797ac36ff6`、ACL收据 SHA `eb4d0afcff872713054700116e9e7ab3339103b6f6d79d2688e1160a47c782b7`、退出证明 SHA `e4e8024dcabc353c8a8068090489e052c66d8ecac78a99876fad61403c956829`。根任务另重核40份小原件及19份Git blob，`root-empty-acl-recheck.safe.json` SHA `ca3d7d4faa42cfad6f3588fae5c6568d64fe723b08d98314b5e7a794c73a7426`。登记执行exit0／cleaned：先归档，再确认原进程、相关launchd及FD释放，仅对两个精确0500私有快照目录补owner写位且原fd ACL保持，最后按身份清理原短根；未移除安装树README的空ACL来通过验收。

本轮严格为两场景／一次独立冷恢复，未重跑原五场景／两冷恢复、GUI、模型或插件复检。普通显式空ACL不套用用户允许的三类特殊安装豁免；历史红灯及旧失败仍按原件保留。Linux后续 [37267752920](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37267752920) 正在运行，不能提前覆盖b5通知发送超时的原失败。Windows固定CLR夹具尚未原生验证，a5原矩阵首updated失败及后四场景／三冷恢复未运行的事实保持，最终同冻结源码双平台门禁仍未完成。**新增关闭0项；10关闭／G09开放／V01、V02、V05移交，PR草稿。无需本地化变更。**

## 2026-10-05：Mac 显式空 ACL 漏读修复准备（历史阶段，当时真实事务待验）

同一个原fd的公开 `fgetattrlist` 返回56字节完整属性（44字节显式空ACL、0条ACE），旧 `acl_get_fd_np` 却返回缺失；真实红测 `r-7nu39tcd` 失败日志SHA `fc5f2240de83b3c1bcd21178cc7daedada4a2a4d792ff1a9ca165c69503cfb43` 已保留。先前“APFS将空ACL归一化为缺失”的解释被该原生证据推翻；旧失败不改为通过，普通文件空ACL也不套用特殊链接权限的范围豁免。

现已改用原fd的有界扩展安全属性读取，严格校验引用、长度、条数及原ACL字段；无ACL与显式空ACL保持不同。25项ACL回归通过，含真实复制到新inode、读回和移除；另2项真实包树回归确认普通链接不跟随带空ACL的目标、未跟踪ACL的独立消费者拒绝显式空ACL。日志SHA分别为 `200837f4bfc71c7d77b7def5c25ff42fa432769d2d1ddbad54cc361271fa6831`、`076471fab596a805e108456f40a437ea0129a939f3833c8ffd1e3e9744b87e46`。

新增版本化 `readonly-inherited-empty-v1` 仅在Mac把README设为显式空ACL，包根及两CLI文件仍为非空只读ACL；保留旧v1和原两场景／一次独立冷恢复的断言。Mac更新385项（`r-lio7dh96`）、i18n11项（`r-3u9bmogv`）、warp／warp_tui check（`r-eq5a5xgc`）、Claude driver23项、通知driver20项、workflow15项和actionlint通过；Linux原生trace 7项未在Mac执行。真实事务尚未完成，不能由门禁通过推断升级完成。Linux仅新增专用feature的有界通知阶段记录，不连带启用test-util，不改既有发送期限；原失败和最终同源门禁仍待完成。**新增关闭0项，G09开放；无需本地化变更，既有英中提示语义和布局保持。**

## 2026-10-05：固定 Windows PowerShell 实现取证完成，运行结论仍为失败

[37264336924](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37264336924) 精确绑定 `d0d61069c3d9fb7f23e57a5ebf783b8ed4dbf5b0`，只读采集实际 PowerShell 5.1.26100.7462。71个目标方法中69个托管体解码成功（8243字节IL）；`FindExecutableW` 与 `SHGetFileInfo` 是无IL的原生PInvoke，现有严格规则因此返回 `partial`／exit1，**正式整轮为failure，不改记成功**。四个原文件前后SHA相同，句柄均释放，没有调用目标候选；官方ZIP大小、摘要、CRC、唯一成员及所有IL字节／分支／异常区域均已复核。根复核SHA `41970d6b52c672b1ed5d1172594053990b4fdf07b0632443cb33b106c76dbce3`，独审SHA `2bf88c99c0b72273c327816b72b0b745eab2be7839def023eca635a2598e8b5c`。

`powershell.exe` 摘要与a5失败原件一致，但a5没有保存三个托管程序集的实际SHA／MVID，本轮IL不能反推为a5相同DLL或已执行分支。静态实现确认 `Process.Start` 在CreateProcess后仍构造标准流，PowerShell的部分启动／分类异常又发生于等待finally之前；Node已经CREATE不等于托管启动返回成功，也不证明发生ShellExecute回退。

a5的原始stderr已核到完整seal／replay链，候选捕获流没有错误字节；这不排除内部ErrorRecord或隐藏控制台输出。原先保存的16次CLR异常全部早于Node CREATE，后10次仅记录丢弃数，不能用启动期HRESULT定位失败窗口。下一步需原事件线程的只读异常对象和方法／IL位置，既有原失败、原期限及后续未运行场景保持。**新增关闭0项，G09开放，PR草稿；无需本地化变更。**

## 2026-10-05：用户确认特殊权限安装可使用原安装工具升级

用户明确允许：带额外共享写权限、文件保护标志或特殊链接权限的安装，可提示使用原安装工具升级；普通安装和已支持只读ACL安装继续保留自动升级要求。该决定不取消来源、身份、权限、恢复或清理核验，也不把安全拒绝记为自动升级成功。以下逐轮“范围待用户决定”保留为当时的历史记录，当前安全属性范围以本节为准，不改写既有源码与验收事实。

**新增关闭0项；仍为10项本次Mac范围关闭、G09开放、V01／V02／V05移交不计通过，PR草稿。** Windows `37209945616` 首updated失败及未运行的后四场景／三冷恢复、Linux `37215103046` 真实通知send超时和最终同冻结源码Linux／Windows门禁仍须解决或完成。已复核 `settings-cli-updates-unsupported-source`：英文 `app/i18n/en/warp.ftl:6082` 与简体中文 `app/i18n/zh-CN/warp.ftl:5894` 均明确提示原安装工具，**无需本地化变更**；本次只更新范围文档，未新增测试或验收结果。

## 2026-10-05：Linux ACL 源码门禁通过相关回归，真实通知超时使整轮失败

[37215103046](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37215103046) 已结束，精确源码为 `b5d9d9fb689d470c40c2abb50bfc0c11bcb63767`，仅 Linux／source_gate_only。新增ACL18、树5、npm事务3、Grok回滚3和Brew绑定3共32项逐名各一次PASS，含Linux原生fd ACL往返；普通nextest共5514 PASS，i18n11项在两配置各一次PASS，无nextest FAIL／LEAK／RETRY／FLAKY／TIMEOUT。以上不包含后续Mac父gid修复，也不是最终双平台门禁。

唯一失败是step37的 `test_refreshed_shell_terminal_beats_inherited_ssh_terminal`：真实hook共25项，22PASS／1FAIL／2SKIP（fish、zsh缺失）。原Node诊断为preload后send在2612.677138ms遭SIGKILL／ETIMEDOUT，stdout／stderr均0字节，目标PTY也为0字节；0.074秒协议预检属于另一个成功进程。尚无失败worker内部阶段证据，不能判定Fluent、冷启动或调度为根因。只读源码核对发现TUI在发送预算开始前加载本地化，但当前复制路径不满足managed versions布局，不会取得版本共享锁；这只能排除该锁等待，不能证明其他卡点。没有重跑、提高期限或跳过用例。

完整日志1985437字节，SHA `ee2523027f4876c149738dac730291165f532e873fb87e86faff10304b1439e0`；独审 `g09-unix-acl-linux-source-b5d9d9fb6-ci/independent-review.safe.json` SHA `1dcc3ecf7e93a4d692b6962dff8bb0e8f4ee5ec67ef644297a5226ba50284688`。根任务另核9份原件摘要及22份b5 Git源码绑定；观察登记 `r-0z4w7_9d` 已退出并清理。**新增关闭0项；G09、Windows原失败、额外安全属性范围决定及最终同源门禁继续保留，PR为草稿。** 本轮仅更新验收结论，无需本地化变更。

## 2026-10-05：Mac 只读 ACL 真实升级与冷恢复通过独审

`b5d9d9fb689d470c40c2abb50bfc0c11bcb63767` 的 `r-t3ha378w` 仅运行 `updated` 与 `swap_receipt_missing`：公开入口升级至2.1.280；交换收据缺失后由独立进程恢复2.1.278，恢复后完整快照与更新前逐字节相同。两场景的before／prepared／after均为15节点、11文件；原权限和非空只读ACL逐项保持，实际四份官方tgz的SRI及成员内容已独立重核。真实路径集合未增加，不将本轮计为新增路径继承验收。

两代候选的manifest、launch、exit、coalition和原生输出配对，均退出0并确认清理。签名后worker和supervisor完整原字节均已归档；19项绑定源码逐个对照b5 Git blob，不使用后续工作树。短根先保全小证据，再确认进程／launchd／打开文件释放；只对两个精确0500私有快照目录补owner写位且复核ACL不变，最后按登记身份清理。独审 `g09-claude-npm-acl-macos-runs/r-t3ha378w/independent-native-acl-review.safe.json` SHA `c0a7d802e3143f687b494a61a5b8ff88f585e8ae1bd6f2b45211557383ed5ad5`；根复核30份小原件、冷恢复完整快照及清理登记，SHA `111a3076ef0b08918a312fe942c053194378505f49f6665bc1b12f287f5e9842`。这是两场景／一次冷恢复，未重跑原五场景、GUI或模型。

新增路径审查发现Mac子目录与包根gid不同的确定缺陷。先运行真实内核组继承基准的红测 `r-bh6r4shq`，新文件错误取得20而非父目录12，失败日志 SHA `15f9f1b3e4a0da02670fce6168c80b79b56d9be0d701997ca24c10fe56a02377` 已保留；现仅将Mac新节点组来源改为最终直接父目录，既有节点原gid及Linux历史包根组策略保持。修复后更新379/379（`r-hcnt3_8v`，日志SHA `0ca9e538df973d3d884da3c8c5d039f883ce4fe22cbb2c2b90d48471ada22a29`）、i18n11/11（`r-t50ilrd0`）及 `cargo check --locked -p warp`（`r-ubbxsw_k`）通过，短根均已清理。两文件独审SHA `5fcab89654b1393e4a5abf6e7b9d91325076aabb6da074b65a51a915dc7db2c1`，本轮门禁索引 `g09-mac-parent-gid-local-gates-v1.safe.json` SHA `a9cc25cc51cad662325d81e6078c0a5f7c0d75732f9c6cb599b5515307c84510`。不将前述b5真实事务重标为新源码重跑。

Linux源码门禁 [37215103046](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37215103046) 绑定b5，仅Linux／source_gate_only，25输入已核；本节记录时尚无终态，后续失败及逐名通过范围见上节，不是最终双平台门禁。Windows `37209945616` 原完整矩阵失败仍保留，缺少PS启动Node时具体失败操作／错误码，或实际重定向与标准流对象事实；不凭CREATE顺序推定ShellExecute回退，不重复相近探针。额外安全属性范围仍待用户决定。英中安装来源、权限、变更和恢复提示已复核，无需本地化变更，无布局变化。**本轮新增关闭0项；10关闭／G09开放／V01、V02、V05移交，PR保持草稿。**

## 2026-10-04：G09 Unix 只读 ACL 实现通过本机门禁，真实事务待验

Windows a5 原生终审补充：三个正式ZIP均重核API摘要、长度和全部CRC；71份小原件257716字节已独审。首 `updated` 仍为 `ProbeFailed`：CMD退出0／18字节正确版本，PS根7836ms退出1，Codex在10732ms才创建，后续Codex／Node分别退出0；PS捕获stdout为0字节。202／415个事件均验证及Continue，pending为空，未观察控制取消；两代原生退出与Job、AppContainer、helper、desktop、DeviceMap、窗口站及LSA清理全部确认。后四场景／三冷恢复未执行。新空输入／流式输出参数由精确源码和实际启动代次绑定，未单列最终命令行、ExpectingInput或管道句柄，不能把语法意图当作同步捕获证明。与旧2d9相比，PS根由0变1而零输出及提前退出仍在；不由此推定ShellExecute分支、保留初始LASTEXITCODE或CLR异常根因。独审 SHA `f9ea149c550e3eeaa258bd6a7367ef0806e8ba3cf06c38470302b55f16fe27fc`，小原件索引 SHA `bca8978a66be934b840cd8bad7448f0ab0996a0fdaa6406f56a2f8be84a3c9c5`，均位于 `g09-ps-pipeline-full-matrix-a5ecc6848-ci`。

本增量为自有包树保存Mac扩展ACL和Linux访问／默认ACL，原fd前后复核身份与权限，限制条数及账本预算；既有节点保留ACL，新节点按最终父权限继承，旧无ACL序列化与历史摘要保持兼容。Mac未知标志和写入allow、Linux非owner原始写位及未支持安全属性继续拒绝，不移除旧树权限来完成更新。Mac显式空ACL写入被APFS归一化为缺失时仍返回读回不符，不能计为精确复制成功；链接ACL仍未支持。

首次378项前的ACL回归为369通过／5失败，修复旧测试摘要、旧拒绝所有ACL断言、Mac空ACL预期及链接打开标志后374项通过。独审又发现快照后增加树外硬链接会先执行chmod/chown；真实红测 `r-a6o6qn5v` 确认外部别名从0644变0500，日志 SHA `bd878a91ee75560b2aefb062b18eed7547c088278b5f75de3da3b40003a78286`。现已在权限修改前核单链接，保留后续复核。Grok恢复另在交换包树／公共链接前核已登记镜像及ACL，同时保持已部分清理现场的恢复能力。

最终本机更新回归378/378（`r-6op9wcv_`，SHA `a5e6485844c3b68d5d626cc28926a683f0eacc902f5b62a3a7af415731d55c42`）、i18n11/11（`r-so83ujcn`，SHA `7869b388dd0e5ac2e80dab7e9bdfee9e60e22718a6e268f61f4be09fe9f6b663`）、Python driver22/22与 `cargo check --locked -p warp`（`r-j9t06rai`，SHA `7e1d5bcfb68f8fd3c54afc66c7527b0ae556102c881bbdbbb18b6fccecdf93bd`）通过。登记短根均已清理，原失败保留。21份变更源码与门禁索引 `g09-unix-acl-local-gates-v1.safe.json` SHA `be0bd7b83f5d3fdeb42c41bd3441836cf77fa0322c17a91fc8060b6c57368595`。英中不支持安装、安装变化、恢复提示语义保持准确，无需本地化变更，无布局变化。

真实Claude ACL验收仅新增可选 `updated`／`swap_receipt_missing` 两场景及一次独立冷恢复，默认原五场景不变；此时尚未执行。恢复只核验首次保存ACL，不重新设置。实际固定包版本未必新增相对路径，不能用这两场景代替新路径继承的原生文件测试。Linux ACL原生执行、其他安全属性范围及最终同源双平台门禁仍待完成。

同时，`a5ecc6848` 的 [Windows原矩阵37209945616](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37209945616) 已正式failure。完整官方日志独审为普通1233 PASS（warp1142／command91），66必需及i18n11各一次PASS；普通结果无FAIL／LEAK／RETRY／FLAKY／TIMEOUT。唯一失败为step60真实npm执行返回 `product_test_failed`；具体首场景与原生退出证据见本节首段，不由普通日志单独推断根因。90条Rust诊断、5条Rust摘要、1条GitHub工具warning和另6条Node弃用提示保留，不声称零警告。独审摘要 SHA `e8b0931f5154a14a05789a37329dad9be72dcaa9b3e08a7a5af155eb684842d3`。**本轮新增关闭0项；10关闭／G09开放／V01、V02、V05移交，PR保持草稿。**

## 2026-10-04：G09 Mac 与 Linux musl 平台绑定原矩阵通过独审

`a5ecc6848bc66b9be11be8d6247bff764ef2f3be` 的 Mac `r-kgcnezst` 五场景最终全部接受：实际更新到280、交换收据缺失后独立进程冷恢复278、外部改动后独立冷恢复保留280、候选改动保留原278，以及 Latest 降级真实 `ChannelMismatch`／无计划。两次冷恢复前的执行阶段按合同标记 `accepted=false/needs_cold_recovery=true`，不能单独当作最终失败或跳过恢复。四代候选原生退出0、coalition资源销毁与退出绑定均已核验，18份验收源码绑定a5。独审 `g09-claude-npm-platform-macos-runs/r-kgcnezst/independent-success-partial-cleanup-review.safe.json` SHA `f22d22d83bc0ff82e73267b72948287c0e3981c70e8e5367a78e0447b1097280`。这是后端更新／恢复回归，不增加GUI或在线模型验收范围；忙碌模型测试单列，未发送模型输入。

子命令成功后，通用wrapper删除0500私有快照目录发生PermissionError；原外层失败保留。剩余546个文件、736对象扩展属性与身份均在外置保全，再核原进程、四个launchd标签和打开文件释放，仅对四个精确快照目录补owner写位并清理原根。保全 SHA `36cff712a999fcb6fb054995a49b1e517ddc217c574a04a0e19030d0ec5e0e03`，清理 SHA `a4cc51f6df56d68d3ab3de63b52227bd9f3127fd5f39acb7cf26fd9605a3cb07`。两轮旧失败根也完成全字节／元数据保全和逐根清理，登记仍保留原exit1。成功轮监督程序从旧轮取得精确同字节签名归档，关联 SHA `ecdb2bf9c13e3f43e2f26091955570e4119d0b14ecacbf37b8348bd66fec2164`；并非恢复原inode。成功轮测试worker原签名字节未归档，仅来源／执行摘要留存，不用重建或重跑覆盖该边界。

精确 `926af2b65` 的 [Linux源码门禁37204698590](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37204698590) 完整日志独审通过：5482次nextest PASS，新增12项Linux原生模块与18项npm回归逐名各一次PASS；i18n11项分别在普通及feature配置通过，FAIL／LEAK／RETRY／FLAKY／TIMEOUT均0。Rust诊断、工具和Node warning原样保留；hook为23 PASS／2 SKIP（缺fish、zsh）。独审 SHA `58d69973ca853470ce265f9b761392052f7b31bba9ba0f00170e1dae1b0d231e`；这不是原生事务或最终双平台门禁。

同 `a5ecc6848` 的 [Linux musl原生事务37207590339](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37207590339) 已通过原件独审：25输入完全匹配，仅Linux实际执行；三个正式小工件已核API摘要、长度、全成员SHA与CRC，18份driver源码及五份fixture清单无失配。五场景全部接受、两次独立libtest冷恢复完成、四代原生退出0并确认清理；候选篡改场景在启动监督者前拒绝，不能计为运行并清理。普通2060项PASS，Linux12／npm18／i18n11各一次通过，FAIL／LEAK／RETRY／FLAKY／TIMEOUT均0。busy模型测试通过但保留15行EOF错误（API keys6、Grok tokens6、cloud sync tokens3，无凭据值）；Rust66项诊断／7条摘要、Node10、Git1及GitHub1条warning保留。完整日志 SHA `057f6709149535f577a4f37fce932409c5d28a0cefd17cecee12cbd01578df31`，独审 SHA `cf3e709bc96e391fa506f35bde1be4be4c03b00834069a93a8c8458863a037cd`，交付索引 SHA `e4704b7cf12402dd97f5a7805eff1414cca7c0e96d1ee1a64c7324d1d6989ad1`。归档有四代exit记录，没有原process manifest／launch／loader动态绑定全链；不外推GUI、285／287、合法降级或远端整个夹具目录已删除。

Linux终态后已串行启动同提交 [Windows原五场景／三次独立冷恢复37209945616](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37209945616)，witness关闭；精确25输入、50源文件、66项普通必需回归及11项i18n已冻结。此时没有Windows功能结论。**新增关闭0项；累计10项本次Mac范围关闭，G09仍开放，V01／V02／V05移交不计通过。额外ACL／安全属性范围及最终同冻结源码双平台门禁未满足，PR保持草稿。** 本轮仅同步证据与支持边界，无需本地化变更。提交前 `cargo check --locked -p warp` 在短根 `r-r6x6ua7y` 通过并清理，日志 SHA `fd8aa134f6c451355864f3ffa215aa756f672ac5ab23d2ebcb5e56ff92a611ce`。

## 2026-10-04：G09 PowerShell 管道兼容候选与 Mac 降级断言修订

Windows 受管版本入口改为通过官方 `codex.ps1 --version` 请求空输入和流式输出管道，保留显式原生退出码；初值1避免未取得退出码时沿用成功状态。盘根严格限于 D–Z，拼接路径固定，官方包／Node选路、环境、Job、AppContainer、映像授权、取消期限与完整清理标准均保持。静态独审 SHA `b046e46011afa93d08f678cfcdc8fe3a864bbc0f43d8a192cf31d0ea1d4f858b` 无阻断，但不证明禁止系统回退、旧零输出根因或真实修复；仍须原五场景／三次独立冷恢复，`witness=false`。

`cf18c6cdd` 的 Mac `r-75gduwee` 前四场景通过，含两次独立冷恢复；最后 `unreviewed_downgrade_rejected` 在旧 `InvalidRelease` 断言失败，整轮仍失败。既有产品合同对 Latest 的280→278精确返回 `ChannelMismatch`；现仅同步该断言和结果字段，保留无更新计划及安装树不变，新增小型实际拒绝字段收据。原失败没有保存 `refused.error`，不把源码推断回填为运行观测。64份小原件、18份cf18来源及四代原生退出0／完整清理绑定已独审，SHA `9ff10abfc80baf116200168af06d10c4d55626a7e64ee18b8af04bb8a6119448`；`cleanup_ready=false` 是失败现场保留机制，不等于已证产品资源泄漏。两次历史失败现场均保留，完整矩阵仍须重跑。

本机 `cargo check --locked -p warp`、i18n11项及三文件定向格式通过，短根已归档清理。门禁索引 `g09-ps-pipeline-mac-channel-local-gates-v1.safe.json` SHA `8a7becbd1667b34ba05f7702aa09ced8d31d8d0905445be495323ecbc487ba75`；Windows专属参数回归没有在Mac执行，不计目标平台通过。无需本地化变更：内部版本入口和开发验收，不改变GUI／TUI文案与布局。

Linux Claude npm 工件补保存已有无凭据夹具的 `manifest.private.json`，以保全精确平台和源码输入；不改变执行范围。工作流12项范围回归（`r-ycdp5d90`）与完整actionlint（`r-ka0ob89z`）通过，原自托管标签配置保持。Linux源码 run37204698590仍运行；musl原生事务尚未派发。环境中Landlock ABI1及userns不可用不会直接阻断Claude npm的sealed ELF路径，完整加载器绑定和真实运行仍待验。**新增关闭0项；G09、额外ACL／安全属性待决范围及最终同冻结源码双平台门禁仍开放，PR草稿。**

## 2026-10-04：G09 Windows 版本输出未满足；Mac 候选拒绝验收器修订

精确 `2d9b3d4f49c26e7464a5666c8c0db833d48c9bdf` 的 Windows run37200286598 原件已独审：普通1224 PASS（warp1133／command91）、66必需及i18n11各一次PASS，96条warning保留；真实首 `updated` 为0通过／1失败、113.08秒、`ProbeFailed`。CMD输出18字节固定版本；PS已经创建Node／Codex及两个console，均退出0，但捕获stdout为0字节。PS root先退出、后代随后退出，不再表述为未启动或取消超时。两代Job、AppContainer、helper、desktop、DeviceMap、窗口站和LSA清理确认；CMD拒绝访问／PATH警告及runner清理孤儿进程原文保留。后四场景与三次冷恢复未执行，发布后的入口、配置和完整目录不变性也未验。

官方三ZIP按API大小／SHA与全部CRC核验，小原件72份逐项独审，50份源码重新对应2d9 Git blob。根索引 `g09-version-stdin-eof-full-matrix-ci/root-delivery-index.safe.json` SHA `fc65bafb1f3ae018a8bf8389ecee99a252b89800501e4b9765a506cab66f09a6`；原生独审SHA `dd9277c49e3945c3d5f84efa460bc355d42433a3774770c98131738002e4e26e`。目前不能确定PS内部首次启动错误或子进程标准流目标；旧公开PowerShell桌面参考源码的ShellExecute回退可解释现象，但没有精确5.1运行证据，不能当根因或据此盲改启动参数。

`926af2b65` 的新平台绑定已进入Mac真实五场景：正常更新与两次独立冷恢复完成，第四个改写候选按预期返回RecoveryRequired，旧安装／候选／Preparing账本保留、公开版本仍为278。账本已有probe generation，但新平台摘要校验在生成监督者目录之前拒绝；验收器无条件要求该目录存在，整轮因此失败，第五场景未执行。`r-bn8_xz96` 全现场与原失败保留，cleanup_ready=false，不改记通过。

仅修订该负例的验收器：允许真实缺失或空监督者目录，拒绝可解析／悬空符号链接及读取错误；存在的收据仍须cleanup_confirmed且not_started，其他场景仍要求非空收据。原树、候选、账本和公开旧版本断言保持。源码SHA `5f5e8990eba32725b8e2d1113dd5658140767f9db524b6f10b55d183606a29e1`，独审SHA `5cab039129febfe05b58fae9a5e3bc0d273626336d942efa71b6a06c490be828`；完整五场景仍须重新绑定运行。无需本地化变更，仅真实验收合同调整。

验收器修订与 musl 工作流接线的本机 `cargo check -p warp`、i18n11、作用域／实际参数组装12项、定向rustfmt及配置既有runner标签后的完整actionlint通过。原actionlint缺少既有自托管标签配置的失败日志保留；没有关闭lint规则。工作流仍为25输入，`claude_musl`仅允许独立Linux原生Claude npm范围，原默认和完整五场景／两冷恢复保持；尚未派发该原生范围。门禁索引 `g09-musl-harness-workflow-local-gates-v1.safe.json` SHA `59a0846d649dd090d76e20bbbe721f61d8bfaf8fe02508c68711e2fc794bd92d`。

新musl源码的 [Linux run37204698590](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37204698590) 绑定926并运行中；仅源码门禁，不代表最终双平台或真实musl交易。官方环境工件确认固定musl加载器存在、目标root所有且0755，execution_verified=false；Landlock ABI1等原环境边界保留。**新增关闭0项，G09、额外ACL／安全属性待决范围和最终同冻结源码双平台门禁仍开放，PR保持草稿。**

## 2026-10-04：G09 补齐 Claude musl 实现，本机门禁通过

Linux x64 固定 musl 映像保持密封 memfd／execveat 路径，新增 `/lib/ld-musl-x86_64.so.1` 的系统身份与摘要绑定，仅接受自包含加载器及主映像唯一 libc 依赖。npm 计划从真实安装 manifest、原入口摘要和完整快照取得平台，执行重验并将平台写入事务账本；冷恢复沿用保存的平台。旧 schema 1 无平台、bin-only 账本保留原恢复合同，新 musl 树不能删除平台字段冒充旧账本。显式 `2.1.280→2.1.278` 与 `2.1.287→2.1.285` 降级合同均保留。

四版 `2.1.278/280/285/287` 的八个官方 wrapper／musl 原始归档已逐项复核 SRI、归档摘要、成员长度／摘要／模式；没有执行安装脚本。产品合同独审索引 `g09-musl-official-contract-review.safe.json` SHA `410f2d3058e5e99b973e0edc964721b9afd4b358125ab670638be2f6a3d1409d`。25 源文件冻结 `g09-musl-source-freeze-v2.safe.json` SHA `92b8da321b5f0c38ec5267feb8d24c837285169c71e2c7729d6e91d974d50774`。

本机 `cargo check --locked -p warp`、更新回归 342 项（含新增 npm 18 项及旧账本恢复）、i18n 11 项、共享 npm Python 117 项、环境 Python 10 项均通过；初次测试编译 E0308 原件保留，修正反序列化类型后重跑通过。全部登记短目录完成归档清理。门禁索引 `g09-musl-local-gates-v1.safe.json` SHA `2ebb1fab9b6de2370b61a62f9d1a6e15ed84850b37002ba7cd43669e4f5b5498`，位于仓外 `g08-build-preparation`。Linux 专用新增 12 项尚未在 Mac 执行，真实 musl 执行／更新／降级及新增平台捕获的 Mac 真实事务回归仍待验，不以静态原件审查代替功能验收。

无需本地化变更：内部加载、安装平台和恢复合同复用既有英中错误提示，无新用户文案或布局变化。额外 ACL／安全属性范围仍待用户决定，没有获得豁免。EOF 修复所在 `2d9b3d4f4` 的 [Windows run 37200286598](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37200286598) 已正式失败：普通 1224 PASS（warp1133／command91），66 必需各一次 PASS，i18n11包含其中；真实 npm step60 失败原件正在独审，不提前认定完整矩阵或 PowerShell 成功。**新增关闭 0 项，G09 和最终同冻结源码 Linux／Windows 门禁保持开放，PR 保持草稿。**

## 2026-10-04：G09 无输入版本候选提供独立 EOF

修复限定`windows_appcontainer.rs`和对应测试：私有站版本候选使用新的匿名输入管道，启动前释放唯一写端；原宿主stdin、TCP取消授权、三流继承及远端关闭流程保持。新增3项Windows真实管道回归，先用非阻塞查询核无写端再读取EOF，并检查原输入未消费、输出通路及普通继承保持。实际Windows执行尚待验收；本地交叉类型检查不冒充测试通过。

`cargo check --locked -p warp`、Windows目标`command --tests`类型检查、i18n11及定向格式通过，登记短根归档清理。门禁索引`g09-version-stdin-eof-local-gates.safe.json` SHA `d41416c9b4c6f5940b16570c4a264590d46e308dea072d4a9a705da95f25f5a2`；两文件独审SHA `202e07daaae16df5d7bf5f2a52b42051b984f67b7fc8b6b8328984301fec84ba`，无静态阻断。无需本地化变更：内部句柄生命周期和开发测试，无GUI／TUI文案与布局变化。

下一步直接恢复既有五场景／三独立冷恢复功能验收，`witness=false`，不新增相近探针。原生矩阵不自动重试；普通Windows nextest保留原配置，实际RETRY／FLAKY逐条计证，不称整轮零重试。此修复尚未证明a274的ReadFile句柄就是stdin或PowerShell已恢复。**新增关闭0项，G09及最终同冻结源码双平台门禁仍开放，PR保持草稿。**

## 2026-10-04：G09 取得原线程读取栈，更新仍未通过

精确 `a2743b4d9ee3317d008378a4904bddb52c0da3bd` 的 [Windows run 37194438248](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37194438248) 正式 failure。普通 1221 PASS（warp1133／command88），63 必需各一次 PASS，i18n11包含其中；feature89及新增7各一次ok，两种配置分别计证。普通 FAIL／LEAK／RETRY／FLAKY／TIMEOUT 均0，95条Rust warning及1条Actions提示保留。唯一失败步骤为真实npm更新，单updated为0 PASS／1 FAIL、395.13秒、ProbeFailed；其余四场景和三次冷恢复未执行。

本轮成功取得同出生身份的PS原主线程：15.247秒与240.237秒CPU累计计数相同、32帧及寄存器相同，暂停／恢复平衡。重新核验既有同SHA原PE、微软PDB的GUID／age／节头及唯一pdata范围，顶两帧分别为`NtReadFile/ZwReadFile+0x14`和`ReadFile+0x8d`。具体读取句柄、整个间隔连续阻塞及其他线程状态未证；不把两点零CPU推断成全进程死锁。单个异常表865980字节超过768KiB预算被省略，未放大预算；PE32／容量遗漏、32帧上限及desktop查询拒绝继续记录为部分覆盖。

源码独审确认无输入版本候选仍继承宿主开放stdin，消费者等待stdout结束后才关闭输入；提前关闭监督者stdin又会触发原取消路径。下一修复只给私有站版本候选独立EOF管道，保留监督者与TCP取消生命周期、三流白名单、原权限、官方入口及期限。当前ReadFile句柄未直接绑定stdin，因此此修复针对已证生命周期缺陷，实际PowerShell效果待验。

CMD真实Node／Codex退出0、输出18字节固定版本，但拒绝访问／CODEX_HOME／PATH警告保留。PS无Node／Codex，210调试事件全验证并继续、pending空，292651ms原控制取消；取消stderr原文件total/captured均0且无截断。两代Job、AppContainer、desktop、DeviceMap、窗口站、LSA和helper清理确认；内层Interrupted和runner终止vctip原文保留，不外推整个远端目录已删除。

终审索引`a2743b4d9-windows-ci/final-delivery-index.safe.json`（47612字节，SHA `7f3d8da15d7d455cf66a7b6a5cc568747728da5e5f11952b681ed731ba1dbbc5`）绑定50份精确源码、两官方ZIP、79成员／72份小件283909字节、逐名日志、原生／输出清理／符号独审；本机观察与审计短根已归档清理。符号独审SHA `9df5fd983dea2493684503d7b5f69fa9f8fd62fa9f3117db33dba968740dd855`。无需本地化变更。**新增关闭0项，保持10关闭／G09开放／V01、V02、V05移交不计通过；完整矩阵、待决范围和最终同冻结源码双平台门禁仍未完成，PR保持草稿。**

## 2026-10-04：G09 部分解栈准备修订，Windows 待验

针对 9d31 两次 `invalid_exception_directory` 阻止原线程采样，仅修订四个 feature 诊断／测试文件。PS 先核验全部模块来源、大小、身份及地址集合，坏表仍参与重叠拒绝；随后省略坏表并记录固定身份、异常目录数值及具体失败分支。部分或零可解栈模块都能进入原 CREATE 线程身份、CPU 和 context 路径；未知 PC 在 StackWalk64 前停止，不把省略模块算作可解栈。CMD 仍严格拒绝。单表 768KiB、累计异常表请求 4MiB、64 模块、32 帧及 256KiB 栈读取预算保持；异常表请求在读取前扣除，短读／IO／解析失败不退还，PE 头／节表另有原限额。生产授权、租约、权限、取消、截止和暂停平衡均未改。

Mac `cargo check --locked -p warp`、i18n 11 项及定向格式检查通过，`r-mfad4wlb`／`r-nxtoe9gi`／`r-4fhf4zkx` 均完成进程、FD、launchd 与目录身份核验后归档清理。门禁索引 SHA `a9d015c90620dd6ac8526ded6781624df78b79135847b08dbe763dc65df630cd`；四源冻结 SHA `da9c28ecb4db6fc6948a67ee0232c56af9f31f9bbb0d687a1ea1c10ca27bd1cc`，独立静态审查 SHA `04963a0c8af9cfb00d173c42d588cc7457ff875218503e3d21ff475365b6378c`。新增 7 项 Windows 回归，feature 总 89（19 snapshot／16 temp／14 creation／21 witness／2 authorization／11 Claude／6 env），本机 Mac 不编译这些 Windows cfg，实际执行待精确新提交 CI。

无需本地化变更：没有 GUI／TUI 文案、布局或用户界面语义变化。本修订只消除明确的诊断前置失败，不能宣称 PowerShell 停滞已修。上一轮 9d31 原件最终索引 SHA `37d775f7d641c349a02394c6b3f76d82f51b16ff38955e3467f3f4204b47b0af`，原生首因审查 SHA `dcc214967d73626b240609caefdde1e11484768c9c5732288b6e4771346bbc82`，输出／清理独审 SHA `60bbdd45fcd7013cf81286b163a9b48b93fd5cb3597ada5d7ce030e6e49df60a`，均保留失败边界。**新增关闭 0 项；G09 开放，原完整矩阵、待决 ACL／musl 范围和最终同提交双平台门禁尚未完成，PR 保持草稿。**

## 2026-10-04：G09 模块绑定通过，PowerShell 原生事务仍失败

精确 `9d31cdf462e96b42b32a87453837a6ea52c4c1f2` 的 [Windows run 37188635472](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37188635472) 正式 failure，唯一失败步骤为真实 npm 更新验收。普通 nextest 1221 PASS（warp 1133／command 88），63 必需全名各一次 PASS；feature 七组 82 项各一次 ok，含新模块绑定 8 项，i18n 11 已含在普通 warp 内。普通组无 FAIL／LEAK／RETRY／FLAKY；95 条 Rust warning 记录及其他工具警告保留，runner 最后清理 `vctip`，不称全部进程自然退出。完整日志 683807 字节、SHA `a8b7fab9d17ddb8e99db0c0281d825e05d9664b032dc3f44497c12d36b6d3f75`；逐名独审 SHA `9fb96d66bdae6330aac2c66291a549ef0d868233b59cf02390dd2c14db6bbd90`。真实单 `updated` 为 0 PASS／1 FAIL、395.55 秒，返回 `ProbeFailed`；其他四场景／三次冷恢复未执行。

PS 代次 `0fb75e7c-2b9b-4b1d-be17-c1a127d8594f` 的 212 个调试事件均验证并继续，pending 为空，额外 `module_binding` 不再失败。6 次 PE32 记录为 `non_x64_native_unwind`，模块上限后 17 次跳过保留为部分覆盖。两次采样分别开始于 15006ms／240000ms，均在 `root_prepare_modules` 以 `invalid_exception_directory` 返回，实际没有 CPU／context／栈；`complete=true` 只说明这次取证流程结束，CPU 差仍为 `early_root_cpu_unavailable`。源码将目录零值、单表预算和映像边界等分支归为同一错误，原件没有失败模块身份或目录字段，不能指认具体 DLL 或通过放大预算猜测修复。当前明确缺陷是单模块异常表准备失败阻止原 CREATE 主线程身份、CPU 和 context 采样。

CMD 代次 `e2fc6fee-2276-4c34-8857-06413c9a98dc` 创建 Node／Codex，版本输出 18 字节、退出 0，CP936 拒绝访问及 `CODEX_HOME`／PATH aliases 警告保留。PS 未创建 Node，约 289248ms 因原控制取消停止；本轮 `cancelled-candidate-stderr.safe.json` 的 total/captured 均为 0、truncated=false，证明封存时该文件为空，不外推系统无错误或旧 SPM 已修。两代 Job／AppContainer／desktop／DeviceMap／窗口站／LSA 清理确认，helper 均退出 0；PS root／console 在清理中退出 1，内层 Interrupted 与最终资源清理分开保留，不称自然成功或整个 fixture 根已删除。

仓外 `g08-build-preparation/9d31cdf46-windows-ci` 已按官方 API 大小／SHA 与全部 ZIP CRC 核验，79 个成员逐项哈希，安全提取 72 份小原件、249929 字节；提取清单 SHA `ffed289c3ca9c1c2092dc97f232ae97352c4df0b439120e0d189b3049eaf9831`。观察器及提取／审计的本机登记短目录均已归档清理，远端失败业务证据保留。无需本地化变更。**新增关闭 0 项，10 关闭／G09 开放／V01、V02、V05 移交不计通过；原完整矩阵、额外 ACL／Claude musl 待决范围与最终同提交双平台门禁仍未完成，PR 保持草稿。**

## 2026-10-04：G09 PowerShell 诊断模块绑定修订

仅修正 feature 门控的 PS DLL 额外诊断：缓存的包／组件租约沿用普通 LOAD 的来源授权，重新核验身份、摘要及祖先；System32 DLL 从原事件文件与锁定目录建立拒写租约，不再套用另一套 PE 导入授权。已授权 PE32 DLL 标记 `non_x64_native_unwind` 及覆盖不完整；身份／IO／损坏头仍为失败，CMD/root 路径继续拒绝 PE32。普通加载授权、权限、取消与期限均未改。926 没有失败模块身份或子阶段，此修订不能确定当时触发分支，也不能证明旧 SPM／停滞已修复。

新增 8 项 Windows 回归，witness 组总 19 项、feature 总 82 项，均待 Windows 实际编译运行。本机 `cargo check --locked -p warp`、带规定本机 features 的 i18n 11 项及定向格式检查通过；三个登记短目录 `r-5la1tbq5`／`r-cr8n6je0`／`r-h183ozs0` 已清理。源码冻结 SHA `8c53ed7dc2537acd79171b2896ac367b95bdd842ad9a51e5ab4e1d1272a85bd9`；门禁索引 `g09-ps-module-binding-local-gates.safe.json` SHA `41c8f0dc443f3b504f4dc7e9edc6224ec816ec46b07ad1026ed60efc8eab19f8`。Mac 门禁不编译 Windows cfg 分支，不替代单 PS 诊断、原完整五场景／三冷恢复及最终同提交双平台门禁。

两文件独审无阻断，收据 SHA `cbe530d02f93bd6f4bdd181ced6c2b65f3ba0ddd95dc5df81ad5c21f10e62250`；静态审查不替代 Windows 实际运行。

英中审计：无需本地化变更；仅诊断绑定、安全数值字段及回归，无 GUI/TUI 文案、布局或界面语义变化。**新增关闭 0 项，10 关闭／G09 开放／3 移交不计通过，PR 保持草稿；额外 ACL／Claude musl 待用户决定。**

## 2026-10-04：G09 首 PowerShell 诊断在 LOAD 绑定提前失败

精确提交 `9267294457f1fbe0cc6c3ee4a55579ef7138c728` 的 [Windows run 37183471233](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37183471233) 正式 failure。本轮为首 PowerShell 代次的单 `updated` 诊断，原生 0 PASS／1 FAIL、107.31 秒，返回 `ProbeFailed`。普通门禁 1221 PASS（1133 warp＋88 command），63 个必需测试各一次 PASS；feature 七组 74 个必需测试各一次 ok，i18n 11 已含在 warp 内。同名测试按普通／feature 两种配置分别计证，不相互替代。普通 FAIL／LEAK／RETRY／FLAKY／TIMEOUT 均为 0，96 次编译 warning 保留；不把单元门禁通过写成整轮通过。完整日志独审 SHA `8c7c0b5dd0663d5b5c08546022d38d02281835f2d03e49e7b3f2aea581fa7fd7`。冻结离线审计器原先要求步骤名前缀，与官方纯时间戳日志不适配；独审按官方步骤起止时间逐项关联，未把准备器匹配问题记作测试缺失，也未修改原日志或审计器。

CMD 代次 `282fa13e-1320-4a78-b5de-e37c583c6645` 真实启动 Node／Codex，输出固定版本 18 字节并退出 0；CP936“拒绝访问。”及 `CODEX_HOME` 规范化／PATH aliases 警告仍保留。PowerShell 代次 `02a04afb-6681-4d74-a297-a4008dbafdd6` 在 LOAD 序号 91、约 4138ms 的诊断 `module_binding` 阶段返回 Other，Win32／HRESULT 均未取得。此前 90 个事件已验证并继续；本次未到 15 秒，无早晚线程快照、CPU 差、CLR 或 Node／Codex。原普通 LOAD 授权先通过，失败发生于额外诊断绑定；源码已确认额外 `protected=false` 租约与仅 PE32+ 限定同生产授权不等价，但没有失败模块身份或内部子阶段，不能确定具体 DLL 或哪一差异触发。诊断修订正在进行，本地门禁及 Windows 实际结果待验，不称旧 `ServicePointManager`／停滞已修复或被本轮复现。

两代 manifest／launch／exit／helper 身份绑定闭合，原 Job 空、helper 双退出 0、桌面／DeviceMap／窗口站／LSA／AppContainer 清理均确认。PS root／console 由错误清理退出 1，不是自然启动成功。`witness.cancelled=true` 为错误清理标记，本轮 `cancel_observed=false` 且不是 seal Interrupted；没有 `cancelled-candidate-stderr.safe.json`，不能据此认定原 stderr 为空或旧错误消失，也不证明取消收据路径已验。原生审计 SHA `9eae08158f5bbdcfee072f390726bde30680cfcc87b2d9b8630942d2900ed277`，输出／清理独审 SHA `ff52e1be875d72b337041acb627e16e6fca47626ae2b4f6900711f5dc5dbbb0b`，首因审查 SHA `f9c93f2cc4b09f928a6a23e973fa9f79c4ed7436d0af3e44e031f57922c8aed0`。仓外 `g08-build-preparation/926729445-windows-ci` 安全提取 71 份小件、241763 字节，清单 SHA `8479796f816b89ac3464e83eab112dd110e8b2300a75a7cc4c7af97e16f022bd`；不外推远端全根删除或其他前缀／配置全树已验证。

本轮总索引 `926729445-windows-ci/final-delivery-index.safe.json`（42627 字节，SHA `5216af442b6068145b7c232a67c2b51b86fa504081ceb082576669d4a3ab1cf5`）另核 50 份精确提交源码、71 份小原件、两份 ZIP 及归档／审计／观察器清理记录；不把本机观察器清理等同远端全根删除。

其余四场景和三次冷恢复未执行，原完整矩阵与最终同提交双平台源码门禁仍未通过。原关闭条件映射 `g09-final-condition-map-20261004.safe.json` SHA `b946f1667c72a76ecc5fe8ef9de4ee77d0866c24031eae1be2eba43f203d613f` 及根复核 SHA `ee2a7fb93cfe5be9b700f2481f4987c1f964638dadf499ea1ead21ef4058c083` 保留 Unix 额外 ACL／安全属性和 Claude Linux npm musl 的原范围冲突；用户尚未回答，不自行豁免或无条件收敛为仅剩 Windows。**新增关闭 0 项，10 关闭／G09 开放／V01、V02、V05 移交不计通过，PR 保持草稿。** 以下历史结果与准备阶段记录不回写为本轮通过。仅更新验收记录，无需本地化变更。

## 2026-10-04：G09 取消错误输出保全与首 PowerShell 取证准备

取消发生后，固定版本候选最多保留 8192 字节原始 stderr 到本代私有新收据，记录总长、截断和原始字节；不覆盖旧文件，不回放已取消输出，不改变原错误优先级，并在原生清理前释放候选输出句柄。PowerShell 取证只授权测试首个匹配代次与模式，15/240 秒读取原 CREATE 主线程及 CPU 差，不启用 CMD 的寄存器断点，不枚举其他线程。诊断模块超过 64 时仅 PS 标记覆盖不完整，普通 CREATE/LOAD 授权、权限、期限和严格清理保持不变。

工作流保留 25 个输入与默认 CMD，通过既有 `windows_atomic_debug_scope=g09_npm_powershell` 和原四个布尔开关选择一次 `updated`；其他原生、源码、全量和 Linux 模式不能混入。源码绑定为 50 项（28 生产／22 验收），新增授权测试来源纳入验收闭包。

本机 `cargo check --locked -p warp`、Windows command 含测试类型检查、中英文 11、Python 驱动 22、workflow 范围 8、actionlint（显式已知 runner label）及修改块 PowerShell 语法检查通过。Rust 格式仅保留与 HEAD 相同的既有差异。全部登记短目录已清理，门禁索引 `g09-ps-snapshot-local-gates.safe.json` SHA `43406b6dc7537b6afae035d1c31958e931f63ce921844e86fcd9d0e4029d11cb`；源冻结 SHA `e5f87e4895e1be32b103aa5c4750c8b4fbcb5fffa000c14bdf3961c8406cff77`。中英文审计：**无需本地化变更**，本补丁不改变 GUI/TUI 文案、布局或用户语义。

上述仅为诊断准备；实际 Windows 快照和取消收据待验，不能声称 PowerShell 已修复或 G09 已关闭。原完整 5 场景／3 次冷恢复与最终同提交双平台源码门禁仍待通过，PR 保持草稿。

## 2026-10-04：G09 启动路径修订后的 PowerShell 仍未完成候选启动

精确 `542a2d81af96b87d5eb179e65f963d5920ae3e98` 的 [Windows run 37178488175](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37178488175) 已正式 failure。普通 nextest 1217 PASS（1132 warp＋85 command），59 项必需测试各一次普通 PASS，包含路径 5 项、CLR 3 项及 output 8 项；i18n 11 包含在 warp 计数内。普通 FAIL／LEAK／RETRY／FLAKY／TIMEOUT 均为 0，跳过项及编译 warning 不改记通过或无警告。日志独审 SHA `13a317bbab6c69d4fcd6e1bfca48021ddb44fe55333822b1b1115b2d29c985a5`。

原生矩阵仅首个 `updated` 执行，0 PASS／1 FAIL、386.17 秒，返回 `ProbeFailed`。CMD 代次 `4cc2211c-d6aa-4579-8a62-1e667c84a1c7` 的 Node／Codex 均真实启动并退出 0，输出固定版本 18 字节；仍保留 `CODEX_HOME` 拒绝访问及 PATH aliases 警告。PowerShell 代次 `54beaf30-3c19-4408-9cb8-04a40b9fe436` 未创建 Node；其 210 个调试事件均校验并继续，无 pending 事件，在约 284760ms 记录控制取消，最终 `stop_requested`／exit 1，不能称自然退出或启动成功。最初 16 条 CLR first-chance 事件和 3 条溢出已记录；HRESULT 不证明具体异常对象、访问目标或完整内部异常链。

两代最终 Job 空、AppContainer 清理确认，helper 退出 0，桌面／DeviceMap／窗口站／LSA 均释放。PowerShell 内层 `cleanup_failed`／`cleanup_unconfirmed` 的 Interrupted 原文仍保留，与外层最终资源收敛分开记录。另核实：取消标记会在 `CapturedOutput::seal` 读取原 stderr 前返回，随后关闭即删除的候选文件被释放。因此本轮没有保留 PowerShell 自身的 stderr，不能据宿主日志缺少旧 `ServicePointManager` 文本推断该问题消失。原生停滞原因继续未证，不能试探性放宽 ACL、能力或超时。

根独审逐份核对 23 份小原件和 49 份精确 Git 源码，SHA `0a773146053f7c80b804cff52221e9ce1d146bf116d6b4603bb65fa907640b4c`。主 ZIP 大小 463714233 字节，SHA `689225148b1a2f0ec316a166e8f668b69d4138c69b3ff0792a8e2d61601446b9`，API 大小／SHA／CRC 已核；唯一观察器短根 `r-75mzlga6` 已退出并归档清理，远端失败原件保留。后四场景与三次冷恢复未执行，模型输入 0；本次不证明失败后的其他前缀／配置全树不变。

现有 10 项关闭依据另经 35 份关键收据及 54 组源码比对复核，收据 SHA `959ab1b234df039547a692a5cd5ada44184196ef60f12c1ba42264fd4db64b34`，未发现新增 Mac 业务验收缺项；不把历史模型／GUI 结果重标为当前二进制重跑。**本轮新增关闭 0 项，G09 与最终同提交双平台门禁仍未完成；V01／V02／V05 移交不计通过，PR 保持草稿。** 本节仅更新证据，无需本地化变更。

最终交付索引 `542a2d81a-windows-ci/final-delivery-index.safe.json`（54,991 字节，SHA `48475ad2e23d560ca1e1cb01c4dfcb35cce26cbe4e5b2b592b579ed277931b12`）已复核三份 ZIP、49 份源码绑定、59 个必需测试及原生失败与清理边界；该索引没有新增关闭结论。

## 2026-10-04：G09 启动路径兼容修复已实现，PowerShell 实际结果待验

窗口站 helper 的最终 `lpApplicationName` 对齐 Rust 1.92 短路径规范化规则：只有 Win32 解析逐码位不变且直接 `CreateFileW` 核同 FileID 时才使用 DOS 表示。canonical 请求、argv0、原租约、摘要和首 CREATE 审核保持；长路径／不同语义保留原表示。官方标准库与原 helper 的路径差异已证，但尚未取得真实 PowerShell 内层异常，不能据此认定 `System.Net.ServicePointManager` 根因或修复结果。失败摘要另外保留最初 16 条 CLR HRESULT 和对应校验／Continue 状态、溢出计数；不采其他地址、字符串或堆，也不改变原期限、权限和事件处理。

新增路径 5 项及 CLR 3 项回归，全部 59 项必选和 49 份验收来源已接线。独立最小 Rust 1.92 编译发现测试 union 索引赋值 E0133，已改整体数组赋值且未新增 unsafe；旧失败收据保留。Mac `cargo check --locked -p warp`、Windows command `--tests` 类型检查、i18n 11、Python 21 通过；四个注册短根归档清理。门禁索引 SHA `69ae482c4d7d794714afd56f3e975c190371bce4a6bf05c30315fe61ac7e6097`，源码审查 SHA `df60c98214ac40df830dc2b1d673fdfbaf1ed5ebd164a9374f38c1f63c09e2ee`。变动 Rust 格式已审，bootstrap 两处既有格式差异与基线相同。无需本地化变更：既有英中错误映射、文案及布局不变。原五场景、三冷恢复及最终同提交双平台门禁仍待验。**新增关闭 0 项，10 关闭／G09 开放／3 移交，PR 仍草稿。**

## 2026-10-04：G09 输出隔离越过 LSA 阻塞，PowerShell 初始化仍失败

精确提交 `5d8075c524ee33fedef71c301484a9441a7679f9` 的 [Windows run 37175216018](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37175216018) 正式 failure，只有原完整矩阵的首场景 `updated` 执行并返回 `ProbeFailed`。普通门禁 1209 PASS（1129 warp＋80 command），51 项必需具名测试各一次 PASS，包含新增 output 8 项；i18n 11 通过，普通组 FAIL／LEAK／RETRY／FLAKY 均为 0。原生部分独立为 0 PASS／1 FAIL，104.90 秒；不把普通门禁成功代记为完整更新通过。

CMD 代次 `553971f6-df80-425d-b13a-b7d1ddc5f479` 实际创建 Node／Codex，输出 18 字节 `codex-cli 0.156.1` 加换行，进程退出 0；仍保留 `CODEX_HOME` 规范化拒绝访问及 PATH aliases 创建失败警告。PowerShell 代次 `fbb75fc7-2d7d-4b13-a87c-199edcda82dc` 在 `System.Net.ServicePointManager` 类型初始化时失败，退出 -65536、零输出，未创建 Node／Codex，未取得内部异常链，不能先定底层原因。两代调试事件均完成验证和继续，无未处理事件；原 Job 空、AppContainer 清理确认，helper 退出 0，桌面／DeviceMap／窗口站／LSA 回收均成功。此结果证明本轮两候选输出隔离后的清理成功，不能把新 PowerShell 失败继续归因旧 LSA 残留。

实际来源为 Node 20.9.0／npm 10.1.0，模型输入 0。后四场景 `old_moved`、`published_receipt_missing`、`external_change_preserved`、`candidate_changed_preserved` 和三次独立冷恢复均未执行，无完整矩阵成功收据；未证明失败后其他前缀／配置全树不变或完整事务发布／回滚。三份官方 ZIP 的 API 大小、SHA 和全 CRC、79 个成员及官方包完整 SRI／成员均已核验；展开共 466771676 字节，安全提取 76 份小文件共 3669710 字节。唯一观察器 `r-5aykha7x` 已退出、归档并清理，远端原件根保留。

终审索引 `g08-build-preparation/5d8075c52-windows-ci/final-delivery-index.safe.json` SHA `2295a35ec843716440c0ea8a75694285351d1715b14a3e6ee593f5d8b37bbcbc`，根独立复核 SHA `a71d4433a6baa0ab142163eedffe010ec29001d195fc0857a49b89c9214cbab2`，原生独审 SHA `c4237179922ca8433491406bc90b9252bcbcfa3dcf529227bc4b28423e1eea91`。本轮文档引用的 `cargo check --locked -p warp` 收据 `r-1mgigg_5` 为 exit 0／cleaned，日志 SHA `c8c11fff895a449f5b3143a07c029122b7cde5b595457e1b6d72d26ba315579f`。无需本地化变更：仅更新验收记录，既有用户文案和布局不变。**新增关闭 0 项，保持 10 关闭／G09 开放／3 移交；完整原矩阵与最终同提交双平台源码门禁仍待完成，PR 保持草稿。**


## 2026-10-04：G09 候选输出生命周期修复已实现，完整原生矩阵待验

固定 Windows npm `--version` 候选的 stdout／stderr 改由私有普通文件捕获；`create_new`、不共享、关闭即删除，祖先目录和文件身份被绑定。原候选进程退出且严格 Job 为空后，分块封存精确输出到候选从未继承的 worker 文件，并关闭候选输出句柄；原窗口站／LSA 清理完成后才重放宿主流。保留原 stdin、官方 CMD／PowerShell shim、ACL／零 capability、取消、stdout 1 MiB 上限和原清理期限，stderr 不新增长度上限。输出复制、关闭、重放或 flush 失败均不标成功；没有修改全局监督管道。

源码独审 SHA `39e7576b1cf47feaadc20c6865bf42807370c89038c621ee733eb8c669064986`。本机 Mac `cargo check -p warp`、command Windows 目标 `--tests` 类型检查、i18n 11 项、Python 来源合同 21 项及工作流范围／actionlint／PowerShell 语法均通过；五个注册短根均归档清理。总门禁收据 SHA `9719596188e21d33bf4167bd9b658afcbb60dda1a0ca49986762f611f3e86c88`。改动源码格式通过；`sources.rs` 的五处既有格式偏差与 HEAD 一致，没有扩大改动。

新增 8 项 output 文件行为回归已纳入 Windows 门禁，**尚未原生执行**。新生产／测试文件分别进入监督程序和验收来源闭包，生产源码嵌入 93 份。无需本地化变更：内部捕获与清理沿用既有英中错误映射，无文案或布局变化。真实 CMD／PowerShell、原五场景／三冷恢复和最终同提交双平台源码门禁仍待完成。**新增关闭 0 项，10 关闭／G09 开放／3 移交；PR 保持草稿。**

## 2026-10-04：G09 写读后管道缓存身份复现登录会话留存

修订 `3c470921a` 的 [Windows run 37173643101](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37173643101) 正式为 failure：MSVC helper 编译通过、普通 command 72 项 PASS；两原生病例各一次，**不写 PASS／写入并读取 FAIL**，无重试。两例新 LUID 的 helper 自然退出、Job 已空、窗口站消失。写例完整读出 44 字节 nonce，管道缓存 AuthenticationId 与新 LUID writer 相同、模拟级别 2；取证 token 关闭、Revert 和线程 join 均确认，原三秒 LSA 仍在。只关闭原管道、保持旧宿主原身份存活后，单次后查为 LSA 不存在；宿主之后自然退出。原 FAIL 和后查结果分别保留，不把整轮改写成成功。

官方 ZIP `11292496945` 为 37223 字节，SHA `e20dacb62af0afe4934d8313c864c81d6e2d48de19d2b0e40f20fc63d5c0a17d`；35 个展开成员及 26 个实际二进制收据已按长度、SHA、ABI、nonce、进程出生身份和 LUID 绑定核验。正式终审索引 SHA `26f48e2dc49a8d9cd30c75c9e64b1a78e1bb899a90e46ddf97779146fcce9f5f`，原生核验 SHA `d7db1d764bf53c66d9f6ff58b371f9016dabe377186d20bc0ff9bd78af5d3eaa`，根独立 JSON／原件摘要核验 SHA `645ed813a7d07543cc23ee2c79b7e7fc3e991f0a755c1cd6e28132532a0d29b5`；35 成员／26 raw ABI 独审 SHA `b35ee5585e21c5239fc22493db294da09d8f184c465d863235be0dca4528597e`。唯一观察器 `r-zthflo64` 已退出、归档并清理；远端原件根保留，`cleanup_ready=false`。

此对照证明写入／读取后的管道安全上下文可以持有新 LUID，不能单独归因一次 WriteFile，也未重跑生产 AppContainer／console／debugger 组合。实际生产 worker 与 supervisor 持有原标准输出管道；正在为固定 `--version` 候选隔离 stdout／stderr 生命周期，保留原输出、stdin、取消、严格 Job 及原清理期限。完整生产更新矩阵和最终同提交双平台源码门禁仍待完成。诊断无用户文案变化，生产修复另作英中审计。**新增关闭 0 项，仍为 10 关闭／G09 开放／3 移交，PR 保持草稿。**


## 2026-10-04：G09 标准流对照被 MSVC 初始化告警阻止

`17c583371` 的 [Windows run 37173102898](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37173102898) 已正式失败；失败发生于固定 C helper 构建，`read_stdio_record` 的局部 `LARGE_INTEGER size` 被 `/W4 /WX` 报 C4701／C2220。两例原生诊断均未执行，无原期限或后置 LSA 结果。官方 ZIP `11292381264` 仅有编译脚本、编译日志和两份阶段收据，SHA `b1ed599418d0abf89399db41527f961b4f004fcfcc08519aacabcbf9ae9597bb`；根独审 SHA `66c7f0ee90e7e9a95005bd1f254b9f1696be0271a21f3845212fcea87ba87adc`。普通 command 72 项逐名 PASS，零 FAIL／LEAK／RETRY／FLAKY；唯一观察器 `r-4ctywwsm` 已退出并清理。完整终审索引 SHA `68a61af4df27e884832756967324415c8766f8bd8b22a1c7bb826a89ddf8188c`。之前 runner 离线和随后恢复在线的 API 原件分别保留。

修订只把该局部变量显式零初始化，保留 `GetFileSizeEx` 成功判定、文件大小合同、原期限和全部告警门禁。Mac `cargo check -p warp` 通过，`r-j94zarn5` 已归档清理，日志 SHA `5fcc2c323ffb4bff5515761017fc135d6c8f9a10d418110a78c2da0516a3b466`；Rust／工作流／PowerShell 源码未变，先前对应类型、语法、范围和 i18n 11 门禁仍按各原件计证。修订的 MSVC 编译和实际两例结果待验，不把新源码验证称为原失败重试。无需本地化变更。**新增关闭 0 项，G09 与最终同提交双平台门禁仍未完成，PR 保持草稿。**


## 2026-10-04：G09 标准流生命周期差分准备完成，原生结果待验

`543cfe27e` 后的测试增量针对生产路径中仍保留的 Rust `Stdio::piped` 原读写端。沿固定 C helper 新增旧 LUID 宿主，两个新 LUID 病例分别不写／写 44 字节 nonce；新 LUID helper 已退出、原 Job 已空后，保留管道完成原三秒 LSA／窗口站查询，再只关闭管道并确认原宿主仍存活，独立保存单次后查。原失败绝不被后置消失覆盖。写例还读取 marker 并在独立线程查询管道缓存 AuthenticationId／模拟级别，显式关闭 token、恢复身份和 join 后才查 LSA，因此最多定位写／读后安全上下文路径，不能单独归因一次 WriteFile，也不能替代生产 AppContainer／console／debugger 与三标准流继承组合。

本机 Windows `command --tests` 类型检查、Mac `cargo check -p warp`、i18n 11、PowerShell Parser、actionlint 及工作流范围门禁均通过；范围门禁接受唯一限定组合、拒绝 28 个冲突、保留 5 个旧范围。四个短根已按身份核实退出、归档并清理，索引 SHA `62ad3baa33c34b78b7d734bb952c9c4b2b543cb516fc91f94c1be1873aa0f11f`。四源独审未见阻断，审查收据 SHA `e790e0d9afa7626ed38a9cf9e6766c224b95d0183f8013f10e2cee7f54bb2f43`。Windows 实际 MSVC 编译和两例执行尚未发生。

现有工作流的 `g09_stdio_lifetime` 仅接受 `run_windows`／`run_atomic_windows_debug` 为 true，其他布尔项为 false、其他枚举默认；独立小 job 只检查 command、运行原普通库回归与两例 Stdio，避免重编无变动 app。原 MappingJob 四例入口及最终同提交双平台源码门禁保留。仅测试与验证工作流变化，无需本地化变更。**新增关闭 0 项，仍为 10 关闭／G09 开放／3 移交，PR #22 保持草稿。**


## 2026-10-04：G09 四组生命周期对照完成，生产阻塞保留

**2026-10-04 设备映射／空 Job 对照已核，仍开放**：`999ff00d6` 的[Windows 37168344636](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37168344636)正式成功，637 项普通 PASS（565 warp＋72 command）、i18n 11，以及四个 ignored 原生病例各一次通过。四例使用互异新 LUID／nonce，8 个 helper 自然退出 0；26 份原始二进制收据独立解码、身份和摘要吻合。四例原查询均为 LSA `0xc000005f`、站 `0x80070002`；保留空 Job 的两例在关闭 Job 前已经回收，两映射例精确创建／删除及 FileID 绑定通过。独审 SHA `cf99093f259d58dc413e19a0a2faf2e59cfc4897ae191c82ff96776bc7222307`，根复核 SHA `170b7778baf2378530493900e39934558b8162b937c268aabe520051308881f6`。只排除这些因素在本轮高 IL、Session 0 两段 helper 对照中足以重现留存，不外推生产 AppContainer／控制台／调试／标准流及多层 Job 组合。生产代码未改；4ff 的 LSA 残留、原五场景／三冷恢复和最终同提交源码门禁仍待解决。不再重复相近映射／一般空 Job 对照。**新增关闭 0 项**；无需本地化变更。

原件位于仓外 `g08-build-preparation/999ff00d6-windows-ci`。主 ZIP `11291565394` 为 38318 字节，SHA `5151e24da50c7c23f6a95fd1489b6f6b03e1b43cef0672b6b570761aedc43103`；大小、SHA、CRC、37 个解压成员和冻结源码均复核。四例整项耗时依次为 0.224／0.208／0.236／0.197 秒，不以关闭 Job 后的观察覆盖原失败。普通日志无 FAIL／LEAK／RETRY／FLAKY／TIMEOUT，日志审计 SHA `fe51faa29c63a9f52f6fc46ba91e21c3440832bdc84f4a049ca92623d158a44c`。唯一观察器 `r-wj9uvy0n` 已退出并清理；远端证据根仍保留 `cleanup_ready=false`，不把日志成功外推为整根删除。文档提交前本机 `cargo check -p warp`（既定取证／测试嵌入 features）通过，`r-p48ptfjo` 已清理，日志 SHA `5bbbc90a74a649aa33cc92970f5f72f9690bdcd4e50d71e8c72906f1d959e24a`。

## 2026-10-04：G09 分项证据确认 LSA 残留

`4ff41b614` 的[原完整 Windows 事务矩阵 37163186787](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37163186787)终态 failure，唯一失败为 step60。普通 1193 PASS＝1121 warp＋72 command；43 项必需具名测试各一次普通 PASS，i18n 11；普通门禁无 FAIL／LEAK／RETRY／FLAKY。首 CMD／console／Node／Codex 四个候选进程均正常 EXIT 0，输出 `codex-cli 0.156.1`；196 个调试事件收到、验证和继续，pending 为 null。`updated` 97.97 秒后返回 RecoveryRequired，不计事务成功。

新正常／abort 分项收据均为 `station_absent=true`、`logon_absent=false`，两个查询错误字段为 null，原件 SHA 均为 `0b98c92df109ac1612c942c6cb077224d1695c1b73100aaa87eeecbed2dd6ae2`。双 helper 退出 0、Job 空、桌面关闭与局部 DeviceMap 删除已证，阻塞明确为新 LSA 登录会话仍存在。App 和 command 的普通持有者只读审查没有找到确定的 token／process 句柄遗漏；本轮未启用 native witness，不把取证复制句柄、空 Job 或文件租约猜作根因。原件不含剩余 token 对象和持有者清单，仍需有区分力的生命周期证据，不能靠延长等待或放宽清理获得通过。

三份官方 ZIP 的大小／SHA／CRC、47 份源码引用及 53 份小原件均复核。终审索引仓外 `g08-build-preparation/4ff41b614-windows-ci/final-delivery-index.safe.json`，SHA `301a6428fc1941b6965e215850dc63597131b62f2449caaabe598a20dda464b7`；原生独审 SHA `ce4d3f73d1d26e8491fc2580212b337167204f741a2d1e73c6659e447afa448b`。唯一观察器 `r-v_otj1x7` 已退出并清理。PowerShell、其余四场景及三次冷恢复均未执行；没有重派完整矩阵。**本轮新增关闭 0 项；当前 10 关闭／G09 开放／3 项移交，PR 草稿。** 无需本地化变更。

仅在既有两段 helper 夹具加入一次 2×2 限定对照：无／有局部盘映射，分别在原 LSA 查询前／后关闭已确认空的 Job。四例各用新 nonce、目录和登录会话，顺序一次、零重试；原 3 秒查询结论保留，关闭 Job 后仅另记一次观察，不覆盖失败。生产逻辑和原期限不变。Mac check、Windows command 测试目标类型检查、i18n 11 项和 PowerShell Parser 通过，四个本机短根清理；本机门禁索引 SHA `b31045e263e16944d03d41b359ccc642dd51d080795840310c7f1d834061b7d7`。Windows C 实编和原生结果仍待冻结提交执行；不能用受限对照替代新 LUID AppContainer／console／debugger 组合及原完整更新矩阵。

## 2026-10-04：G05 原条件补验完成

**本轮补验关闭 G05；当前 10 项关闭、G09 开放、V01／V02／V05 移交，PR 仍为草稿。** 固定 Claude `2.1.280/claude-opus-5-5`、Inherit 根任务，按各真实构建计证，不将旧索引不可达推断为功能退化，也不称当前 HEAD 重跑所有模型链。联合关闭索引位于仓外 `g08-build-preparation/g05-proof-refresh-20261004/g05-current-scope-closure-20261004-v1.safe.json`，SHA `924275d81ad7c606250e631541f38e90f1cf707a7a448043e06af68be658f902`。

- **纯 PNG**：`4ff41b614`／`r-fb8vkfxb` 的原生图片输入恰一个 image 块、零文字，PNG 原字节相同，正确识别四象限；换进程后同 SID 回忆一致。三条产品输入各一次、零工具，两代自然退出 0 并清理。独审 SHA `b508e9c6503eb3512b43c3bbcec25a39dd1a188730dceae606a28887c978e7e4`。
- **PNG＋单技能**：`7a8c03bd2`／`r-45i17a5t` 的两次不同 PNG、完整原生数组与登记附件逐字节对应；每轮唯一所选 Skill、精确一次 AllowOnce，正确返回技能独有标记与图色。同 SID 两代、首代清理后恢复，零额外工具和零重投，两代自然退出 0。独立复核 23 份原件，审计 SHA `2d55af46a3f6077afd5d00c47e654fae4cf2b55e087001171ce37f843cb93f08`；原生历史 SHA `014ba917e7a047557ebc224b3b8394c35d9116b6b0d19812828136618d8aaae1`。
- **GUI 拒绝与恢复**：`087283721` 产品构建／`r-rp5ac3x9` 缺技能时拒绝并保留文字、图片和技能卡，前后数据库任务／消息均零；实际 PNG＋Skill 经一次审批完成。正常退出后重开保持原任务、host、原生进程、SID 和运行代，owner epoch 1→2，原生历史字节不变且未重投；断开后精确进程、coalition、launchd 均退出，小证据先归档再清理本轮目录。28 份归档索引 SHA `6235452288f1aff0a8dd1d2698ba1a3b44f1bae2af0cdf772a8f55bdee6b68d9`，根代理独审 SHA `0a1e4d8853500f87cbde0e9eebfce557056e2883c256fba76deb0a9140a2896b`，清理 SHA `a763dfab9a92ea24700af36084269734a80fb81660c9dbca7018a531830689c7`。英文审批／结果和中文恢复布局由委托代理原始 CUA 实屏核验；未伪造磁盘截图或声称根代理另看截图。拒绝前 host 目录 glob 错误不计证明，保稿实屏与前后数据库零任务／消息独立保留。

补验中两次失败不改写：首轮夹具把 queued 早于 system/init 错判为缺失身份，`5533cbfa2` 仅等待原配对合同、继续拒绝重复／错身份；第二轮实际额外调用 ToolSearch，原严格运行器正确失败。`7a8c03bd2` 仅在本次私有项目 deny 增加 ToolSearch，Skill 仍逐次询问，生产代码、模型、输入及全部严格断言不变。该准备符合固定二进制和[官方工具搜索配置](https://code.claude.com/docs/en/mcp#configure-tool-search)，不更改用户默认配置。

对应本地门禁 cargo check、68 项相关 Python 回归、8 项新增时序回归与 1 项既有身份回归、i18n 11 项均通过；所有完成短根已按记录清理。门禁索引 SHA `6098155b97bb07dc7faf4133f6a07ae089dde6c8414b845cb457af69cb58f8d7`、`92fba6d941dda0bbb5c947df53e7526c1a7dbc317426737a7bb0922c174c9d12`。**无需本地化变更**：仅验收时序和私有配置，产品英中语义不变，真实英中布局按 GUI 构建计证。不外推所有格式组合、固定权限、父权限或多技能；其他平台实机仍移交，最终 Goal 同提交双平台源码门禁仍待完成。

## 2026-10-04：G09 首 CMD 正例与回收阻塞

**2026-10-04 首 CMD 原生执行成功，完整回收失败，仍开放**：`087283721` 的[完整 Windows 任务](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37160171946)已终态 failure。普通 1191 项 PASS，41 项必需测试逐名一次通过，i18n 11 通过；首 CMD→Node→Codex 实际输出 `codex-cli 0.156.1`，四个候选进程及两个 helper 均正常退出 0，203 个调试事件均继续，桌面关闭及 DeviceMap 移除已确认。但窗口站／LSA 登录会话的联合消失确认超时，正常和 abort 清理均未通过；原收据不能区分两侧。`updated` 为 RecoveryRequired，未执行 PowerShell、其余四场景或三次冷恢复，不将局部正例计为更新成功。三份官方 ZIP 已校验，唯一观察器已退出并清理，终审索引 SHA `1d0cc1eb67fd8abfbd843f26c99b683665ba6c2685b56c92f0d8cb691e6c6506`。

本次仅补两侧独立查询及各自终态安全收据，查询失败仍为未知；保留原绝对期限、错误优先级、严格 Job 所有权和两侧均消失的成功条件。源码审查尚未找到可确证的句柄遗漏，不将诊断改动称为原生修复。新收据已被现有工件选择覆盖，两条新测试纳入原 Windows 筛选；实际回收仍待下一次完整原矩阵。无需本地化变更，产品文案、权限与布局不变。

提交前 cargo check、command Windows 测试交叉 check、i18n 11 项和 Windows 来源合同 Python 21 项通过，相关短根均已清理；新增两条 Windows 收据测试仅完成类型检查，尚未原生执行。门禁索引 SHA `8f0eebf36e18f6da8a55f3a208e7983f0d40a8fd8b54288df9adc62d2871bde9`，仓外 `g08-build-preparation/g09-release-observation-v1/local-gates.safe.json`。

## 2026-10-04：G05 最终证据复核与补验准备

G05 当前移回待补验，历史通过结论保留，不认定产品功能失败。纯 PNG 零文字、同轮 PNG＋原生 Skill、新建／冷恢复和特定 GUI 重关联的旧原件链尚不可完整复核；现存 v7 分轮收据及一次插件刷新负例不能替代这些条件。审计 SHA `dda27a5f4d8974f76b888c405e925c3782c1794d2d93faef9505fd61c61c1a19`，仓外 `g08-build-preparation/g05-proof-refresh-20261004/evidence-sufficiency-review-v1.safe.json`。不再重复搜索旧索引，按原条件补齐真实证明，不降低验收范围。

单技能 Mac 运行器原先仍使用 `/private/tmp`，现与双技能共用已登记的短目录及 TMPDIR/TMP/TEMP；其他平台、原模型、认证、输入、超时和收据断言不变。16 项离线回归通过，含单／双技能短根、失败原件、UTF-8 和其他平台路径；`r-94tw_4tv` 已清理，日志 SHA `21100e5d5f59ab493783a02836da103b4d3c53bd77146c6acf3253e2fd2c39bf`。无需本地化变更，仅开发验收帮助与状态记录。新模型输入尚未执行，这些检查不计功能验收或缺口关闭。**新增关闭 0 项；9 项关闭，G05 待补验、G09 开放，3 项移交，PR 草稿。**

## 2026-10-04：G09 Windows 两处测试 Debug 约束修正

**2026-10-04 Windows 测试编译错误已修，本机门禁通过，仍开放**：`73fc6bb3a` 的[原 Windows 任务](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37154098574)生产 check 通过，但 step44 在执行用例前编译失败。IAB 原日志两处 E0277 位于 `managed_process_atomic_windows_tests.rs:1463/1481`：`unwrap_err()` 额外要求成功值 `DEBUG_EVENT` 实现 Debug。仅改两处为显式取错，保留取消／截止错误种类、trace 边界及零事件消费全部原断言；生产源码不变。本机 cargo check 和按仓库 2024 配置的定向格式检查通过，短根 `r-gpty5j95` 已清理；无需本地化变更。修复收据 SHA `e4c9dce85c33d27c13877c22ffc1a097ed31135195b8ba92e35926c21236a20c`。原 job 后续步骤按实际结果另计，不将尚未执行的 app 回归、五场景或冷恢复记通过；完整 Windows 修复门禁和最终同提交两平台源码门禁仍待完成。**新增关闭 0 项，10 关闭／G09 开放／3 移交，PR 草稿。**

首失败元数据 SHA `e188daad614d76835ffb68678429d348611b26dff855952ec27d64050c13912e`；本机 check 日志 SHA `5ba1153ce2165a88f4e9123141875ebfba20e12a87fa51172ba847e5d29b5510`。首次手工指定 edition2021 的 rustfmt 检查与仓库 edition2024 不符，报告既有格式差异但未修改源码；按仓库配置修正检查后通过，不为此重排无关代码。Mac check 不证明 Windows cfg 测试已编译，仍须原生 Windows 完整 libtest 与原矩阵验收。

## 2026-10-04：G03 本次 Mac 范围关闭

用户确认新会话未先打开富输入时真实首拖满足预期：`6296fb36a`／固定 Grok `.5`／`r-r6gr9mus` 首次拖入终端即可出现图片卡；人工收据 SHA `07afe5f25c1f5b1f182a9dce0827444d0b33471eaee089f44838b641224b096f`。原普通未绑定 PTY 的 PNG 粘贴／拖放、正确 SID／prompt 与一次 ACK、多图原字节及顺序、焦点／审批等待保稿和英中布局均按各原构建满足；三平台原生合同及固定工件已绑定。功能条件审计 SHA `66fe07f96c53ec5955d708cc85375ef572b3279ac6cd9c7fa9b0dd0f61ebf7d3`，独立关闭审计 SHA `35d962eb93191715cc9c5cc24df8d6fb21cd09a9d8a2086dd97ff2291e296226`。不要求用户重复验收，不新增模型输入或截图声明。

精确 `ce49b12c7` 的 [Windows job 111282010601](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37150103944/job/111282010601) 成功，完整日志 5085 次普通 PASS，FAIL／LEAK／RETRY／FLAKY／TIMEOUT 为 0，9 条通知／迁移和 3 条结果桥逐名一次普通 PASS；i18n 11、rust-genai 81、Node 19 通过。首拖回归受 Unix cfg 限制，不计 Windows 执行。终审 SHA `a23ea8f686c8e4a1a090df2ca8aed612f692a0bb584c5a520145ea3ee32fda37`，官方完整日志 SHA `e14674c7b5f713400f727a5b3a455857c5cbb140a492784ef5e7272f1fa15007`；13 份来源及 3 个官方 ZIP 的 API 摘要／大小、成员 CRC 已核。ce49 Linux 实际 1 FAIL 与整 run failure 保留，Windows 成功不改写它。

精确 `bfb29a8eb` 的 [Linux job 111291798714](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37153388911/job/111291798714) 5444 次普通 PASS及首拖／桥／通知具名用例通过，完整终审 SHA `2b194c1de98232e39b8f570ae140c42fde52da30a522c7667e408d8da556928d`；真实 hook 23 通过／2 缺失 shell 跳过，不计应用 ACK 或冷机启动。ce49→bfb 仅结果桥测试同步及 3 文档变化，生产字节一致；8 份 G03 关键源码在 ce49／bfb／73fc／7c977 一致。此为 G03 阶段关闭，最终 Goal 仍要求 G09 稳定后同一冻结源码提交的两平台相关门禁。历史通知失败与菜单 LEAK 保留，不因本轮普通 PASS 推断根因已修。

两源码观察器均正常退出，短目录按记录清理；首拖私有 GUI 已退出，唯一现场仍 `cleanup_ready=false` 保留。CAPABILITY_MATRIX 与 RELEASE_SUPPORT 的当前 G03、G08、G10、V03 及 G09 Mac 证据同步，历史轮次不改写。本轮仅关闭记录与支持说明，无需本地化变更。G09 的 [73fc 完整 Windows 原生验收](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37154098574) 已进入源码检查；五场景／三冷恢复仍未计通过。**本轮关闭 G03；10 关闭／G09 开放／V01、V02、V05 移交，PR 继续草稿。**

关闭汇总原件 `g03-preparation/g03-current-scope-closure-20261004-v1.safe.json`，SHA `1e3057074514cbb2afa28bf5fbfb1c41ae1a485db34a5b8c6f9f2eda128d6454`。提交前 `cargo check --locked -p warp --features cli-agent-native-witness,warpui/test-util,rust-embed/debug-embed` 通过，短根 `r-4ri5418v` 核实退出并清理，日志 SHA `7cac323e4c1e57986f4bd96b20aff9afdb86c32a9f9a5d05bb575c284943b2a6`；既有编译警告保留。

## 2026-10-04：Linux 修复门禁通过与既有关闭证据复核

`bfb29a8eb` 的 [Linux 源码门禁 37153388911](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37153388911) 成功，完整原日志、工件及 14 份源码已核验。普通 nextest 5444 通过，FAIL／LEAK／RETRY／FLAKY／TIMEOUT 均为 0；结果桥 3 项、通知协议／迁移 9 项和首次拖放回归逐名一次通过，i18n 11、rust-genai 81、Node 19、诊断 Python 14 通过。真实 hook 23 通过、fish／zsh 缺失 2 跳过；原 SSH 正例及 guarded 成功收据通过，不扩为应用 ACK 或冷机启动验收。终审 SHA `2b194c1de98232e39b8f570ae140c42fde52da30a522c7667e408d8da556928d`，完整日志 SHA `c70f4754cb34977d10268c749768779ca09982fb1b06662f616008dc49be9adf`。ce49 Linux 的原失败保留；ce49 Windows 尚待结束，不能将不同 SHA 冒记最终 Goal 同提交门禁。

对 9 项既有关闭条件的只读复核确认 17 份关键安全收据／索引摘要匹配。矩阵 SHA `45a42b89670935ea58bf93874a39e26f1f17669649b1c2140eda212e4e6f662a`，仓外 `g08-build-preparation/final-merge-audit-73fc6bb3a/requirements-evidence-matrix-v1.safe.json`。G05 的 `claude-image-skill-integrated-wip-v1/index.safe.json` 与 `gui-claude-hot-skills-20260926/index.safe.json` 在旧本机归档根不可达，限定外置归档及存储迁移记录未找到新入口；保留历史 Git 中的真实验收记录和精确摘要，标记位置待定位，不推断功能失败或唯一原件丢失，不撤销既有关闭。本轮未重跑模型或冒称已查看这些原图。PLAN 和 RELEASE_SUPPORT 中的当前计数及 G07 开放旧表述同步更正，历史轮次不改写。

G03 当前 HEAD 的 8 份关键源码仍匹配已验原构建，影响复核 SHA `bf39b1df3c12d1308cc750f37f616fdef5e4b10bb54e1d3b027d6980167c9f1e`。首拖私有现场独立审计 SHA `46ede650af51991202f1718acee41d6028a22b27be3be1e7a132558e6f1e1c3d`：GUI、后代及打开文件已释放，但私有记录不足以唯一界定默认原生 SID／输入范围，保持 `cleanup_ready=false` 并保留，不读取默认认证或原生历史，不新增功能关闭条件。

G09 的 `73fc6bb3a` [完整 Windows 原生更新门禁 37154098574](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37154098574) 等待 runner；CMD／PowerShell、五场景／三冷恢复及原生资源回收均未计通过。**本轮新增关闭 0 项；仍为 9 关闭／G03、G09 开放／V01、V02、V05 移交，PR 草稿。** 本轮仅更新证据与支持说明，无需本地化变更。提交前 `cargo check --locked -p warp --features cli-agent-native-witness,warpui/test-util,rust-embed/debug-embed` 通过，短根 `r-n3xee0bv` 已核实退出并清理，日志 SHA `de901d5d0617c30ef3400a8ddd3faa44df6b01b3935a33d0e9ce9972a7514ce9`。

## 2026-10-04：G09 由目标登录 helper 实际创建并持有调试端口

**2026-10-04 目标登录会话内创建与调试已接通，Windows 原生验收待验**：第二 helper 改为在同一真实线程创建候选并执行 Wait/Continue，经认证管道转交原事件句柄，原登录身份、映像/FileID、Job两阶段授权、零 capability、三标准流及绝对期限均保留。复制失败可续取原事件，在途 Continue 不重放；首次绑定失败先终止原挂起候选再终止子 Job，正常 Close 要求排空并释放共享句柄。最终独审确认私有桌面名称正确传入；曾针对中间态提出的遗漏结论已撤销。本机 cargo check、35项进程/控制回归、i18n11项、Python来源合同21项及actionlint通过；command Windows测试交叉check通过，新增10项command及5项app回归尚未原生执行。来源绑定42→46、生产嵌入90→92，原f576失败保留。收据 SHA `342b6ccc5c719f65845c281d92771cd89b53683facd0c08ecafbac6eb7abd093`，仓外 `g09-station-debugger-v1/local-gates-final.safe.json`。完整app Windows编译、CMD/PowerShell、五场景/三冷恢复和资源回收均待真实门禁，G09仍开放。无需本地化变更：既有英中失败提示语义和布局不变。

15份实现/测试/来源/工作流文件冻结 SHA `196ae2a661c5db9a43d411cdca337f24c7db6df241c7af9ad40b8cd9d40551d5`；独审 SHA `940373da3b04d354f12c875f2df63dd583a7f1b429deda6b3c3989726432a830`。六项完成门禁的短根均核实退出、归档并清理；Mac不能证明完整Windows app类型或原生执行。G03的bfb测试同步修复已推送，Linux精确源码门禁37153388911运行中。**本轮新增关闭0项；9关闭／G03、G09开放／V01、V02、V05移交，PR仍草稿。**

## 2026-10-04：G03 最终门禁测试同步修复与 G09 原生失败终审

**2026-10-04 功能验收齐备，最终源码门禁仍开放**：真实首拖用户确认和原普通 PTY 图片、焦点、审批保稿、多附件顺序及英中证据已逐项审计，收据 SHA `66fe07f96c53ec5955d708cc85375ef572b3279ac6cd9c7fa9b0dd0f61ebf7d3`，不再要求重复人工操作。`ce49b12c7` 的[源码门禁 37150103944](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37150103944) Linux 官方日志确认真实通知 23 通过、fish/zsh 缺失 2 跳过；生命周期步骤 3767 通过、结果桥重复交付测试 1 失败。失败发生于第二次等待固定轮询耗尽，未证明 CPU 或 SQLite 为根因。测试现改用生产桥完成通知和已有私有 SQLite 回执期限，本机 8 项及 cargo check 通过，生产逻辑与时限不变。修复提交的最终源码门禁待验，Windows 原轮仍运行；新增关闭 0 项。无需本地化变更。

**2026-10-04 完整 Windows 原生更新失败，仍开放**：`f5763d34e` 的[37145335699](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37145335699) 普通 1173 项及 i18n 11 项通过，但首个 `updated` 原生场景在恢复候选前失败，报“候选进程没有继承局部盘映射的登录身份”；第二 helper 的局部盘/FileID 已核验，原件不足以细分候选身份失配字段。清理再等 5007ms 后未确认；候选尚未恢复，不计 Node 执行、PowerShell、其余四场景或冷恢复通过。官方原件、三工件和 42 项来源均已审计，索引 SHA `040dbe35d7937575a4dbc2cc1c99c5d711de83f03cf5ea414e1d5dc82e008856`，原生审计 SHA `fd31ff65766b5b2041ee5a8cc2169401b9665e9a67118192824aa322f4639cd2`。需修复实际创建进程的登录上下文及首个绑定失败时的挂起候选清理，再完成原定更新/恢复验收；安全拒绝和测试通过均不关闭 G09。

本机短目录 `r-ymefnr36`（8项定向）和 `r-8wn6jhqv`（cargo check）均完成并按记录清理，门禁收据 SHA `568b66392d53033f4730f566021ccdb8f9c91c7f477fa49d4119e953f546f1ba`。ce49 Linux 完整日志 SHA `de80cc5e1e542dd57af452bce626e4ae3ccfdad4ec6602ba6cd1e46c7fbb267c`；FAIL 在进度和汇总重复打印不计两次执行。Mac 既有警告保留，无本次测试等待代码警告。G09 并行原生实现尚未包含在本次提交或计为通过。

首拖验收私有 GUI 已正常退出，所属进程及打开文件均已释放；原现场按生命周期合同保留待独立 SID/提示审计，`cleanup_ready=false`，不新增功能关闭条件。生命周期收据 SHA `d4eddf73d9b6f1f685a2aa2f61d8fb40246ec119e67be4525aff013dc8648525`。本轮只修测试同步并更新证据，无需本地化变更；仍为 **9关闭／G03、G09开放／V01、V02、V05移交，PR草稿**。

## 2026-10-04：真实首拖确认与通知协议启动修复

用户确认修复后真实首拖已满足预期：`6296fb36a` 的私有 Mac 窗口 `r-r6gr9mus`、固定 Grok `1.0.41+infinishell.session-notifications.5`，未先打开富输入，一次拖入上方终端即可展开并出现图片卡。人工收据 SHA `07afe5f25c1f5b1f182a9dce0827444d0b33471eaee089f44838b641224b096f`；未据此声称新截图、模型 ACK 或新的隐藏重拖操作。G03 的真实首拖阻塞解除，仅待最终源码门禁。

`f5763d34e` 的 [Linux 门禁 37145283547](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37145283547) 终态失败。官方完整日志确认 SSH 用例在独立协议查询启动 508.508874ms 后 ETIMEDOUT/SIGKILL，stdout/stderr 均零字节，发送未调用；慢启动原因未知，不回填 6296 的旧故障根因。终审 SHA `d13a4285d59dcffbf16a732f80b84706bcb8e4a621f9ef68d3947c3e1400c3e6`。本次将协议核验合入一次发送，完整写入后才返回精确协议收据；原期限、帧预算、锁和终端绑定不变、不重投。插件升为0.1.6，0.1.5原配方及备份保护保留。当前最终两平台源码门禁待完成；G09的Windows完整原生更新验收仍运行。无需本地化变更，既有英中失败/恢复提示语义一致，README同步中英文合同说明。

本机冻结源码门禁通过：cargo check/build、97项定向nextest、i18n11项、Node19项、Python诊断14项及安装器合同15项；真实worker 25项中24通过、fish缺失1跳过，包含严格成功收据和目标PTY原帧。新复制worker冷启动协议预检7.558秒，后续仅验已就绪路径，不称冷hook启动或应用接收确认。首轮安装器版本负例过时导致3处断言失败，修正旧/未来版本集合后通过，原失败保留。门禁收据SHA `828ec7a8fe1057352f245cd8a61d6fa788c8156f954a5ab2b9fbec4e2114fc70`，仓外 `notify-guarded-send-v1/local-gates-final.safe.json`；完成短根均清理。G09冻结42项来源逐字节未变，不把f576原生结果冒充新整二进制重跑。最终两平台源码门禁仍待验。

本轮新增关闭0项；9关闭／G03、G09开放／V01、V02、V05移交，PR保持草稿。

## 2026-10-04：6296 首拖源码门禁终审及 f576 待验边界

**2026-10-04 首拖源码门禁终审，仍开放**：`6296fb36a` 的[37140741806](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37140741806)已结束，Windows success、Linux failure，整轮 failure。Linux 普通 nextest 5428 项通过，新首拖用例逐名普通PASS；原生通知 Python 24项为21通过／1失败／2跳过，SSH无bootstrap用例PTY零字节超时。Windows 普通 nextest 5056项通过，另1条 `terminal::input::tests::test_ai_context_menu_preserves_lock_state` LEAK单列保留；首拖夹具受Unix cfg限制，不能计Windows执行。两边check及各11项i18n通过，另各81项rust-genai通过；无nextest FAIL／RETRY／FLAKY不代表整轮成功。完整终审SHA `e7718d52358061a948207c871dba9253d554c2ef7bede47e0e7c708534b3a3f4`，仓外 `6296fb36a-ci/final-review.safe.json`。新 `f5763d34e` 的[Linux37145283547](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37145283547)已通过编译／生命周期组，但通知步骤37再次失败；具体用例及新诊断待官方完整日志，不假定同因。其[Windows37145335699](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37145335699)完整原生更新验收已开始，尚无结果。Mac修复窗口及未操作的首拖现场保留待用户确认。**本轮新增关闭0项，9关闭／G03、G09开放／V01、V02、V05移交，PR保持草稿。**

此轮只更新已结束门禁的证据和正在运行任务的状态，无产品、文案或布局变化，无需本地化变更。提交前 `cargo check -p warp` 通过（`r-7nu0xe2o`，日志SHA `7bca83a1fcde7a0e60fa4d80ff2db19ae48d1a03245d3303576728fc42b38cf4`），短根按记录核实清理。

## 2026-10-04：G09 私有盘根实现与待完成的原生验收

**2026-10-04 私有盘根原生实现已补，本机门禁通过，仍开放**：固定 Node 对原卷根 `C:\` 的路径查询仍按旧实测记 EPERM；新实现在第二 helper 的独立登录会话内，将原候选目录按 FileID 绑定为局部 D–Z 盘根，以该 helper 原句柄提供候选父进程及 DeviceMap。仅三路标准句柄复制到选定父进程后进入继承白名单，创建后精确释放；物理 cwd、固定 Node／npm shim、零 capability 和时限保持原约束。监督程序保留唯一外层 Job 句柄，经有界双阶段控制协议先接管 helper、再核验候选确属原 Job；候选创建后绑定失败保留调试清理所有权并禁止恢复。退出时释放原进程／线程句柄，再完成盘符、站和登录会话清理。Mac check、35项定向回归、i18n11项、Python来源合同21项通过；Windows command及测试代码交叉check通过，但不计原生执行。来源绑定35→42、嵌入生产表87→90，新增盘映射测试已接入Windows门禁；actionlint通过。无需本地化变更：内部路径与归属修复沿用英中既有失败提示，无文案或布局改变。收据SHA `6f6281994111e692bf5d4e963bfd2455cfea8d575e0e946780ec4068e42b845c`，仓外 `g09-private-device-map-v1/local-gates-final.safe.json`，本地完成短根已按记录清理。**完整CMD/PowerShell更新、5场景恢复矩阵和原生清理仍待Windows实测，新增关闭0项。**

**2026-10-04 精确新构建已准备，真实首拖待用户确认**：`6296fb36a` 的 Mac GUI 已构建并启动，右侧固定 Grok `.5` 标签尚未打开富输入，保留现场等待单次首拖；未产生新模型提交或ACK。构建收据SHA `8a9c6c96a656c444230efc79aad4589c60acf52d9eea57da90c9465b25d24488`。同提交[源码门禁37140741806](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37140741806)的Linux新首拖回归普通PASS、普通nextest5428项通过，但真实SSH通知测试在2秒内PTY收0字节而失败；该失败根因未定，不能归因旧EAGAIN或提高超时。Windows仍在执行。本轮仅为失败夹具接入既有固定字段诊断，12项合同测试通过；诊断不修复或关闭通知故障，原失败保留。**G03仍开放，新增关闭0项。**

首轮Python合同因嵌入表预期仍为87而失败，修正为实际90后21项通过；首轮actionlint遗漏已有自托管标签配置，使用既有配置后通过。两次失败原件均保留，不改写为绿灯。当前9关闭／G03、G09开放／V01、V02、V05移交，PR保持草稿。

## 2026-10-04：首次拖放图片的初始化顺序修复

**2026-10-04 首拖初始化修复，本机门禁通过，仍开放**：在已打开CLI输入的附件路径中，同步刷新图片选项后再检查容量并读图；修复非空启动命令保留旧Disabled状态导致首图丢失。旧代码下真实WarpUI窗口事件回归先超时，修复后121项通过，覆盖首次开框、隐藏重拖保留原图/中文草稿/顺序以及失效目标零PTY写入。i18n11项和`cargo check -p warp`通过，独审无阻断；无需本地化变更，既有英中提示语义已复核。门禁收据SHA `1df6ca6d131d6341fd3265608661c07270e1cf7f07769942bbcf1289c64f1dc4`，仓外`g03-preparation/first-drop-fix-local-gates-v1.safe.json`；三条绿灯短根已清理，红灯及夹具历史保留。精确新构建的真实首拖、最终Linux/Windows源码门禁尚待完成；**新增关闭0项**。

## 2026-10-04：人工首拖复现与 Windows Node 卷根访问边界

- **2026-10-04 人工拖放补证与首拖缺陷，仍开放**：用户在固定`.5`普通PTY确认直接拖入富框，以及富框已展开时拖入上方终端区域均出现图片卡；另开`1.0.46`的观察不合并为固定工件验收。新会话尚未点击Rich Input时，第一次拖图只展开富框没有图片卡，再拖一次正常；初始化后隐藏再拖也正常。该明确复现需要修复，不能把已有入口正例当作整项完成。人工/GUI收据SHA `faf8751e3e68ec41dcbdbe9119e78639135886d45d559ba2d40248a86c5df517`，用户原截图SHA `f9da8e77200e53af1da591108059c78c6a7bd21b6bb1c4425249d1c1cba06f89`，仓外`g03-preparation/r-zvc31qhz-ordinary-images-gui`；本轮未发送模型输入或新ACK。**新增关闭0项**，待修复后新会话复验及最终门禁。

- **2026-10-04 profile修复越过CMD停滞，Node卷根访问仍失败**：精确`d5ec5dbc9`的[37132673681](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37132673681)为failure；Windows check、普通1153项和feature68项通过，含新增8项及增强清理回归。真实更新0通过／1失败、85.85秒；Node在2049ms创建并进入CJS模块加载，`lstat`卷根`C:\`返回`EPERM/-4048`，Node2489ms、CMD2497ms退出1，无取消及旧超时停滞，PowerShell未到达。35项来源精确匹配、113个debug事件均验证并继续；Job/profile/站/LSA及helper清理获证，整夹具根删除未证。独审SHA `a7758755bcf90e462ba0cd73a16d202ff62eb98e826c4673c87e442c739fa47a`，根核15原件及35源码SHA `02620a0911358784504c014155f521d657591939630136f6269167ec6b6e7b64`，仓外`d5ec5dbc9-ci`。本轮未采新TMP/TEMP长度，不沿用旧283单元；首行CP936拒绝访问仍未归属具体调用。所需受限路径元数据能力尚待实现，不放宽共享ACL或重复相近候选；真实CMD/PowerShell成功及最终源码门禁待完成，**新增关闭0项**。

此前CGEvent跨应用未交付的原件保留；人工后来成功不能回填旧手势。G03本次补到真实入口后发现首次自动开框丢附件，继续按用户功能修复和验收，不计关闭。G09原生路径修复效果限于越过旧CMD停滞，Node显式路径查询仍失败；不以1153个普通测试、68个feature测试或清理成功代替更新事务。两官方ZIP已核API大小、SHA及全CRC，完整日志SHA `c36dad299d7b0a6ce9ee6c4d95cbc54812698f46e7f736580cda8cd079d1408c`。原件和元数据收据均归档，未再次运行同一候选。

本次仍9项关闭／G03、G09开放／V01、V02、V05移交，PR保持草稿。本段记录无需本地化变更；G03修复的功能、英中和提交前门禁另行记录，未提前宣称通过。

## 2026-10-03：原生 profile 路径修复及提交前门禁

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。针对下述50c真实TMP/TEMP扩长证据，候选LOCALAPPDATA改用原进程token查询的原生基址；仅申请TOKEN_QUERY|TOKEN_IMPERSONATE，无线程模拟或环境回退。fresh SID路径由GetAppContainerFolderPath取得，原路径从卷根逐级以只读、不共享DELETE句柄锁住，拒绝reparse并比较严格FileID祖先链，启动前重查映射及身份；不猜测固定AC父级。API分配路径和SID文本按各自合同释放。

只替换候选环境唯一既有LOCALAPPDATA值，大小写重复或缺键明确拒绝；HOME、USERPROFILE、APPDATA及CLI配置等其余原字节不变。零capability、Job、继承句柄、取消及产品时限未改。原进程/Job/站/ACL全部清理后，先释放profile目录句柄，再调用DeleteAppContainerProfile。新增8项纯回归及原真实空Job/ACL失败清理测试的持有/释放断言，Windows尚未执行。验收源码表33→35，补齐两份受影响的command测试文件，生产嵌入表不删减。

本机cargo check `r-l8jpa2nz`通过，日志SHA `8e0a3c1accd156296242a12e2050e532b728ca1d35bf0b7b1e9af94fb697255c`；i18n11项 `r-86uezubi`通过，日志SHA `cb8d450a83e347d1c7f229748afb3d0289540b00e3fd0404ab9b69137d2efbaf`；Python来源合同21项 `r-1ecxfzln`通过。三个短TMPDIR均核实进程/launchd/打开文件释放并清理，门禁收据`96473f38bcf62c06de8b71e7b78130abfbb0c27a8f47202b56839478a26e409a`，根源码审查`64cc807ace4bb2c928cb99a8e7a23b7020c70faa8ece9440dbac2ad0b50c866d`、独审`1e32229409362fe6dc025ce9fd23c0d264721240a7c793445a2adf1ef34c76ee`，仓外`g09-native-profile-path-fix-v1`。已有编译警告保留。无需本地化变更：原生内部错误沿原ProbeFailed英中用户映射，不新增用户文案或布局。

以上不证明Windows API权限、原生profile映射或真实更新成功；需精确提交的Windows编译/回归及CMD、PowerShell更新通过，再做最终源码门禁。G03跨应用真实拖放独立开放，旧50c和a53失败证据不改写。

## 2026-10-03：原生临时路径实际改写与 GetTempPathW 不足重试

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。精确`50c64e9e31f2ee014da10cc944534ebca727f75f`的[37125688198](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37125688198)为failure。Windows check通过，普通nextest为warp1102＋command43共1145个唯一PASS；feature libtest的68个预期全名各一次ok，六组14/16/14/8/11/5，不用汇总数量替代逐名核验。实际ignored更新0通过／1失败，366.85秒，首CMD代次`5475a58e-d4cc-4fa5-845f-5ae5653b57aa`返回ProbeFailed，PowerShell未到达。原日志保留90条Rust诊断、5条编译警告汇总、1条Actions Node20警告，以及sccache停止时10061连接拒绝；没有nextest FAIL/LEAK/RETRY/FLAKY/TIMEOUT，不将前置非致命异常改写为无警告。

新观察使用原来同进程/线程句柄、暂停计数、取消和256KiB共享读取预算，没有增加暂停次数或产品权限。15.065秒与240.111秒的样本均核三段函数代码SHA和三项IAT目标与固定映像一致，PBI原始NTSTATUS为0；运行TMP/TEMP各唯一、283个UTF16单元，均不匹配构造的208，USERPROFILE唯一209且匹配。两遍读取及指针一致只记observed_stable，process_atomicity_proven明确false。早期可信GetTempPathW重试释放返回帧的保存状态为`0xc0000023`，Length与MaximumLength均566字节：当前283单元的值超过初始261容量，重试容量等于值长而没有NUL余量，与固定代码不足重试机制闭合。此帧不是第一次查询现场；晚期无可信重试帧，明确skipped，不沿用早期栈。原身份绑定的225.046秒间用户CPU增加224.859375秒、内核增量0。

微软[AppContainer合同](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer)明确TMP/TEMP会重定向到独立profile。源码将LOCALAPPDATA指向私有cwd/data；结合profile与AC/Temp后缀的长度推导与283吻合，但观察故意没有采集环境原值，不能把这一推导写成实际路径字符串或具体改写调用点已证。下一步按本轮fresh SID绑定原生profile目录，保留配置隔离、零capability、严格Job及现有清理；修复效果尚未验证，不缩短某条CI目录或延长超时充当完成。

33项来源与冻结提交/启动记录匹配，51调试事件全部验证并Continue、pending为空；原暂停均0→1→0。creation仍为delay_import_coverage_unsupported，未安装创建断点。GetThreadDesktop失败保留未知，未外推Node已执行JS。原manifest/exit/绑定、Job和AppContainer profile清理、双helper退出及站/登录会话消失证据齐全；远端整夹具删除未证。两份官方ZIP的API大小、SHA及全CRC已核，53份小文件安全提取，3份大TGZ留在完整ZIP中。主ZIP SHA `51a95443c5504861834394880f96bd94dce42815f9132c994f173816f6d01fca`；完整日志SHA `3b938bb1019cb079918911b362c80f2cef200c5a411f7b0b5da81cd5d232d03b`，日志独审`2fd07195c0fb5a6b8958546470a06452da6917d4e8b3a7d350536da15ad19fec`；根原生审计`b51cde2422a4e0c4d87cb2096365ee55cd271fc6fd64b0140f984c50cdae1796`，独审`21699a9deff7563f99b8d3c0e6e117a87d2f3298f9fa78ad3a44476e39536824`。原件在仓外`g08-build-preparation/50c64e9e3-ci`。

50c提交前本机含取证feature的cargo check、i18n 11项及Python合同21项通过，短根均已核验退出并清理；收据`d010ea3a2d1daec8f7378ff4c4208d3b5bedb02d923eede89489d9b85df08b60`。本观察及审计无需本地化变更，没有用户文案或布局变化。真实更新失败不改记成功，G03拖放仍独立待验，最终源码门禁尚未运行。

## 2026-10-03：标准堆修复后的真实更新仍失败

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。精确`a53b3c49eca3afa4fa972984ec090badbc7c48e6`的[37118252587](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37118252587)已终态失败。源码检查、warp普通配置1102项及带native-probe-witness的command43项共1145 nextest通过，无FAIL／LEAK／RETRY／FLAKY／TIMEOUT；feature libtest仍为12＋14＋6＋11＋5共48项通过。新增CMD/PowerShell环境两项在实际Windows各普通PASS，32项取证仅feature执行，环境5项双配置通过；这些结果不代替原生更新。真实用例0通过／1失败，372.38秒、exit101、ProbeFailed，仍停于首CMD代次`514763af-d55f-49fa-97e6-dde2310dea45`，PowerShell未到达。

原主线程PID9856/TID19308的两种创建身份在15.109／240.071秒样本一致，224.962秒墙钟内增加224.25秒用户CPU及0.46875秒内核CPU。早期23帧由普通RtlFreeHeap+279经GetTempPathW+546、应用兼容检查至CreateProcessW/CMD；旧调试堆帧不再出现在这次早期样本中，但不据此声称观测全部堆标志。晚期仅1帧，指令不在已验证模块范围，必须保持未知，不能沿旧轮符号或调用链外推。五快照共33帧，31帧与原SHA映像、微软PDB及`.pdata`绑定，2帧未知；暂停恢复均平衡。配对审计`80bca86a403ab883f20a3239c7cc9dd2ba08405bc482c692d4adb8814dfa49fc`，根审`db1c5263080caa5b8283f588ab42e38e60bb16148bb2cc30a275ba1fb7f96ebf`，独审`1d6c246f7c610981c6ef916fd48a7c85ee8a643c05e2099c2cfee5fd31200576`。

31源与started、冻结提交一致，manifest／launch／spawn／exit／receipt摘要闭合。49事件全验全Continue、owner31432、pending为空；Node早期已生但主线程原暂停1，本次观察仅1→2→1恢复，CREATE仍只在取消清理出现，不能证明Node已执行。原exit1、Job清空、AppContainer/profile删除、helper双exit0、窗口站及登录会话消失均有原件；整个远端失败夹具删除未证。两官方ZIP API大小、SHA及全部CRC通过；主ZIP`348e956fe45bb24933413ab56eacd56c959146e48ee999c2aa5beb2e04fffab5`，日志独审`82280a68ee08ead6e3b030f14d4e1289f69f5d81ca3d40c20dcae790ca624ffe`，仓外`a53b3c49e-ci`。

固定磁盘映像的613字节GetTempPathW与ntdll查询分支已只读复核：早期返回PC `0xef802` 对应容量不足后的释放/回跳，普通清理返回PC是 `0xef827`。静态分支要求STATUS_BUFFER_TOO_SMALL及长路径分支；下层所需NUL空间与重试按Length分配存在条件性不增长关系。源码TMP/TEMP/TMPDIR为208个UTF-16单元，初始内部容量261，首次不足尚无法解释；运行中实际环境、UNICODE_STRING、IAT和函数字节未采，不能归因AppContainer重写、扩展路径或确定Windows缺陷。未运行新探针或改产品。静态收据`7d82e6f121d2751b0f53fa40395fb1719b1b9a9ad8a6b6315d9208543da9f48d`，根代理核17件唯一引用文件并复读完整函数。

此次实测否定“仅选择标准堆即可解决首CMD阻塞”。私有环境从构造到CreateProcessW的静态传递完整，与[LLDB同类实现](https://lldb.llvm.org/cpp_reference/PlatformWindows_8cpp_source.html#l00622)一致，仍不能冒称生产修复成功。停止重复同一候选，继续只读核查已获GetTempPathW调用点及环境参数；不改权限、超时或关闭条件。本轮仅证据与审计更新，无需本地化变更。提交前cargo check `r-qmszwg7s`通过，log SHA `992a9327d058c393b193aa1873f2d5519487f7d937418f20a07399868becf63e`，短TMPDIR已核验退出并清理；产品源码未再变，复用a53已验回归及i18n。

## 2026-10-03：2e9 原主线程配对结果与修复依据

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。精确`2e9f8d60f365e786fe0e9e05ea5c7dee3928d0bf`的[37112613018](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37112613018)源码检查及两组nextest通过：warp未加witness feature的1100项，加native-probe-witness的command43项；不能把全部1143项描述成默认feature。witness配置libtest为12＋14＋6＋11＋5共48项通过，32项取证测试逐名通过，环境5项在两配置各通过一次。真实用例0通过／1失败，耗时368.63秒、exit101、ProbeFailed；上述单元结果不覆盖它。

首CMD原主线程PID7940/TID5888的完整创建身份在两次样本一致。15.118秒至240.046秒的墙钟间隔224.928秒，用户CPU从133281250增至2380312500个100ns单位，差224.703125秒，约99.90%单核，内核CPU均0。早期栈为RtlCompareMemoryUlong/调试分配，晚期为RtlpCheckBusyBlockTail/RtlpValidateHeapEntry/RtlDebugFreeHeap；两者均处于CreateProcessW应用兼容检查的GetTempPathW链。实际模块SHA、微软PDB及对应`.pdata`范围已绑定；两次failure=null，暂停均0→1→0。配对审计`6a2e117ce5fb87d6a87cc85fad354fd7b135c40b5fd2d793289893b1b639fe82`；它证明两次状态和区间CPU消耗，不独自证明完整根因。微软说明调试进程可显式[选择标准堆](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/debugging-a-user-mode-process-using-windbg)，据此推进仅CMD私有候选环境的修复，效果仍待真实更新验收。

原49个调试事件均验证并继续、pending为空；Node CREATE仍仅在清理时到达，不能据此说取消后才创建。延迟导入覆盖拒绝保留且未安装创建断点，CP936拒绝访问未归因API。31份源码与started记录符合冻结提交，原manifest、launch、spawn、exit及receipt摘要互相匹配。本代Job、AppContainer/profile、私有站、两个helper退出和对象释放均有原件；整个远端夹具删除未证。根审`b99873d2467ad91b3e7cc6ca6a2b2cd024b6160f02dc3f201d2e9b8f3d7055c1`；完整日志审计`f0baa7644fdd4b239454095e2fcea17ade58a780b33e8d00547b154f390560ca`。两官方ZIP大小、SHA、CRC已核，主ZIP`c5e56c7e77041dba34bc19919d5a179433a451bb69fea10bea6890854b4be3b6`，仓外`2e9f8d60f-ci`。没有改记历史失败或扩大关闭条件。

代码修正仅在已经验证的`mode==cmd`候选中从原私有环境增加唯一`_NO_DEBUG_HEAP=1`，不接受调用方环境覆盖；PowerShell及共享构造保持原映射，DEBUG_PROCESS、Job、令牌、桌面、取消、超时及清理均不变。两项回归核CMD唯一值1及其余映射、PowerShell完全相等。无需本地化变更：内部堆配置没有用户文案、提示或布局变化。新修复的真实Windows效果与最终源码门禁仍待验。 本机含取证feature的cargo check及i18n 11项通过；两项原字节映射测试在父环境缺失/值0两配置共4次通过，只执行Mac非Windows分支。三个短根均已核验退出并清理，根门禁收据`013bd25d7042b91c4a7693cd89939b89a616aaf7c26f2063caea171f52efe8a7`。

## 2026-10-03：固定 CMD 调用点与原主线程配对观察

新增关闭0项。对同SHA CMD的独立静态复核确认：旧返回帧`0x24a6c`对应`0x24a65`经IAT `0x3af10`的CreateProcessW调用，前置块第六参数为`0x80000`，没有CREATE_SUSPENDED；IAT `0x3af00`的GetStartupInfoW输出lpDesktop被复制到新STARTUPINFOEX。根代理逐字节核验13处指令，收据SHA `fc3030bc9884b8b7e2fed9ae78cab8a8b6cdbb6ddfd59b68a9409c08cbe243c9`，仓外`g09-fixed-image-contract-v1`。这削弱“CMD显式暂停或丢弃桌面”的解释，但没有读取动态参数、IAT或桌面字符串，不能证明实际桌面或最终停滞原因。

据此不实施普通IAT子集观察；仅在现有精确generation、默认关闭的取证功能中补15秒/240秒原主线程配对。两次使用原CREATE事件复制的同一process/thread句柄，早期优先采根主线程并从既有枚举去重，计入原16线程额度；晚期不枚举其他线程。CPU保留GetThreadTimes累计user/kernel的100ns单位，仅在PID/TID及两种创建时间相同、采样时间递增、累计值单调时给出差分。准备后和暂停前均复查取消/截止，原EXIT Continue后释放诊断句柄；清理路径不会进入晚期观察。

同步修正触及的identity：微软[GetThreadTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getthreadtimes)和[GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)均未定义存活对象的exitTime，删除据此判活的两处比较，输出明确不从timing推断活体。旧枚举句柄未申请SYNCHRONIZE，不为诊断新增等待权限；原Suspend/GetContext失败及EXIT/Job清理保留。CPU有效也不代表栈或活体检查通过，所有未知和失败单列。

四文件已独审并冻结；Mac原字节纯逻辑7项通过，完整snapshot的12项Windows测试仅元数据编译通过，不代记执行；完整Windows应用glue及真实候选待唯一限定CI。本机含取证feature的`cargo check -p warp`与`i18n::tests`11项通过，短根`r-suzrsvnh`、`r-n7t6bl_c`均已核验退出并清理；根代理另核19件交付文件大小/SHA，收据`12c8e74e27a19e4728ac2ed23f42296cecace5138d5f8dcf43f1ef9037840742`。无需本地化变更：仅内部取证，无英中用户文案、产品权限、超时或验收条件变化。G03真实拖放、G09原生停滞及最终源码门禁仍开放，PR草稿。

## 2026-10-03：固定系统映像及微软符号缩小旧栈范围

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。精确9773abe49的[37109159914](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37109159914)只运行范围校验和固定映像收集，其余任务跳过，没有Cargo或原生候选执行。四份cmd/ntdll/kernel32/kernelbase的读取前后身份及SHA一致，并与b3原快照匹配；官方ZIP大小、SHA、全CRC通过，ZIP SHA `84bd74f5c8ab368959d19b80289eaac0a6c15bde52fa5b944a0629d329791219`，仓外`9773abe49-image-ci`。

从微软官方符号服务器取得对应PDB，四份GUID匹配PE RSDS、Info age不小于映像age、DBI age与映像age均为1，符合[微软OpenValidate4源码合同](https://github.com/microsoft/microsoft-pdb/blob/master/PDB/dbi/pdb.cpp#L745-L821)；不误称Info age也都是1。PDB section表逐字节匹配PE，无OMAP。只读公共符号和同SHA PE的`.pdata`交叉核对，约15秒CMD原主线程的CreateProcessW→BasepQueryAppCompat→CompatCachepLookupExeProcessImpl→AslPathIsTemporaryDirectory→GetTempPathW→堆分配链获证，顶帧位于RtlpAllocateHeap对应范围；另两个CMD线程采样在worker等待。最近公共符号不自动等于函数范围，Node无已核模块的帧仍未知。该单次采样不证明约286秒时仍在同处，不证明死锁、debug heap或TEMP/ACL根因。

CMD普通IAT有CreateProcessW/CreateProcessAsUserW；5个延迟描述符的18项中，17项具名、1项为未识别的Wldp ordinal 2，未建立完整延迟导入语义。没有放宽现有拒绝或把普通导入子集冒称完整创建观察。公共符号收据SHA `587171d4910faf7f168f386817aef03fa6ea7b911e87f8b0317abf57ae828b30`，仓外`g09-fixed-image-contract-v1`；根代理独立复核PE/PDB分析和映像共20件大小/SHA，收据SHA `421a5faa99a25d69fb0838b9fef7ef9ae47e31b4040469676eb5ddf2978d4ff8`。初始解析器过严age假设及报告遍历失败原件保留，修正后纯解析通过并清理短根；未执行系统映像。现有单独普通IAT观察或追加晚期采样尚不足以指向生产修复，暂不运行相近候选，不改权限、环境或超时。G09原生失败及最终源码门禁继续开放。

收集入口提交前actionlint、7组范围校验、5项PowerShell文件合同及cargo check通过，短根均已清理；Mac合同测试不冒称Windows API运行，实际Windows收集另按上述CI计证。本次审计提交前cargo check `r-c279tw0a`通过，日志SHA `cace9cfac1dd44b97c06721ea3d7a4f64d5611cd53cf8c58188f6da165444f5e`，短根已清理。仅内部取证入口与审计，无需本地化变更。

## 2026-10-03：首 CMD 已进入，创建观察受延迟导入限制

新增关闭0项。精确b3c8cc839的[37103876288](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37103876288)已失败：普通warp1100＋command43共1143 PASS，无普通FAIL/LEAK/RETRY/FLAKY；feature取证25＋ce6 11＋环境5共41 ok，五个新环境合同按关/开两配置逐名通过。真实ignored入口1 FAIL，execute返回ProbeFailed，已进入首CMD代次e7c9aa44-b1f2-46b3-81a6-519abca693fa。31来源、started及manifest/exit/launch摘要链与冻结提交一致。终审索引 `afb174088621343e4c3d8e0a99d1cff1440fb445a27b93b7d181e7ad884c1335`，完整日志SHA `c47122f5704466443a57a41c83b4566f249f724299ceb2388ee64bce150234a9`，仓外`b3c8cc839-ci`；两官方ZIP API大小、SHA及全CRC通过，根代理独立核验索引22件大小/摘要。

原生stderr及其审计（SHA `c2fcf77348f7bf2695c58d78efb5d7695d1de897863e31f24ed3c5d5bf5ae56c`）确认：creation在new阶段报delay_import_coverage_unsupported，NotModified且未装硬件断点，不能称已观察创建API入口/返回。取消前快照15076→15232ms，CMD PID2196的三个线程原暂停计数0，Node PID28684/TID7996原暂停计数1；本轮逐次恢复原计数，Node模块为空/指令未归到已核模块，所有线程GetThreadDesktop失败且win32=0。snapshot.complete仅表示快照流程完成，不证明桌面或完整取证。主调试线程6876收/验/continue均47，pending=null；约286秒取消后才收到Node CREATE，出生FILETIME证实取消前已存在，不能将迟到事件写成迟到创建或JS已执行。CP936拒绝访问字节未归属具体API，原根因仍未知。

本代exit cleanup_confirmed、严格Job清空、无能力AppContainer profile删除、两级helper exit0、desktop关闭、logon/station消失均有同代收据；整个远端fixture根删除未证。PowerShell及其余事务未据此计通过。不扩大权限、超时或条件，不重复同一原生探针；先核对尚缺的创建观察与静态映像合同。此次仅证据和审计文档，无需本地化变更。提交前cargo check `r-twlo79wt`通过，日志SHA `e60985991e6adb632bcf7ca7b9b9b2401a092e24729d68d291f9763a36b1e53f`，短根已清理；仍9关闭／G03、G09开放／3移交，PR草稿。

## 2026-10-03：授权 CGEvent 的实际拖放与跨应用对照

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。用户明确授权CGEvent后，`r-wyjvrfq1`的唯一Finder内部手势实际将source.txt移入destination，原inode94728370及SHA `b2ea998f3717190298c51de7e40fdfc2f3e4b0069c0763a801afaa035f6031a8`保持。随后`r-6ufdaiwj`的新普通未绑定Grok GUI使用69c17宿主和已绑定`.5`原生工件；相关Mac图片路径与当前b3等价，不称当前HEAD整包重建。唯一目标手势`r-l1omtgbb`结束后，图片卡与PTY路径均未出现，无Return、模型输入或新ACK。原生bridge安全投影与私有数据库均无新增消费；17个登记进程身份已退出，唯一失败现场保留。23件证据索引SHA `d19190ac85f7c202229840fe43277c4e04fe8528772975109b1681c8d9d3876c`，仓外`g03-preparation/r-6ufdaiwj-ordinary-images-gui`，根代理逐件核验大小与摘要。

最初“1秒yield可能与末段观察重叠”的推测被原始调用时间排除：helper外层07:36:33.328→07:36:35.046Z，首次GUI观察07:36:47.052Z。源码只读复核未发现确定可解释零图片卡的断点；原件未记录Cocoa拖放回调或Rust事件分发，不据此归因产品、坐标或观察时序。

为划清交付边界，在`r-hc94lfma`的新建TextEdit富文本空文档做一次独立跨应用对照；同helper `r-5sjpv3j1`完全结束后观察，文档仍空，无实际图片插入。源454字节PNG及SHA `f886d60f893a05c913fd63b1acf5bc0a68be83ccd410bc03fd75fb306eaff711`未变，未打开用户文档或改偏好。17件索引SHA `f3187508f58ad02a439d331fb635558316507d7d917f824f010698ee7af214fc`，仓外`g03-preparation/g03-native-drag-v1/textedit-control-v1/r-hc94lfma`；根代理核验大小/摘要及空文档截图。原TextEdit进程经正常菜单退出，两个短根已cleaned/cleanup_ready=true；快捷键Quit未退出的历史保留。此对照不证明InfiniShell产品失败，43个事件/exit0也不算实际交付。不继续重复手势；仍需可交付的跨应用原生拖放或人工实测，再完成最终源码门禁。仅验收及文档，无需本地化变更。

## 2026-10-03：修复首 CMD 取证入口的环境合同

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。精确33f8d91cc的[37098941047](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37098941047)普通1138项、feature取证25项及ce6 11项通过；runner的21份生产源码嵌入检查通过，完整31摘要匹配冻结提交。真实ignored入口随后在validate返回 `inherited_environment`：runner显式传入的固定ALLOW开关不在验收允许列表。没有started、supervisor或原生代次收据，调用顺序证明首CMD和exercise未执行；worker后续绑定与执行后复核未到达。两份官方ZIP的API大小、SHA和全CRC已核，终审索引 `28233569189db2e4802b83842419ac9d95fa01f93fd088f4d602db0085219552`，仓外 `33f8d91cc-ci`。私有npm登记是夹具准备，不是产品候选成功；远端现场保留，未证完整根目录及外部npm后代清理。

现仅验收入口在专用feature、updated、execute与固定ALLOW值同时成立时接纳该键；GENERATION、未知键、错误值、其他case/recover仍拒绝。五项回归直接调用实际validator，普通Windows筛选与feature步骤均接入；生产权限、超时及CLI环境白名单未改。独审核实31来源、17路径及三份实际npm shim模板一致，未发现第二个确定前置合同错误；未执行的动态身份与manager树仍待真实运行。源码索引 `c566e3146b843d96ff9eeb70c4a26ee2347a388ac6f8d56c60b7fb1275b97559`。本机check、i18n11、Python21与actionlint通过；隔离原字节合同在Mac四项回归按feature关/开各通过一次，Windows五项仅双配置元数据检查，不能算Windows运行。六个短根已清理，首轮提取器自检失败保留；门禁索引 `07a0c5158a1b71bdf40f8abdf00d6f6040ea4e47c92c54be3d9ebf3f0e6bb22c`，仓外 `g09-witness-environment-fix-v1`。无需本地化变更。Windows精确修复复验及最终源码门禁待完成，旧原生停滞仍开放。

## 2026-10-03：来源绑定前置修复与固定 Codex hooks 实测结果

新增关闭0项，仍9关闭／G03、G09开放／V01、V02、V05移交，PR草稿。精确363b17a77的[37094078558](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37094078558)Windows check、监督程序和feature libtest构建通过，普通1138项、snapshot8＋creation14＋glue3共25项及feature ce6 11项均通过；随后Python verify_embedded报 `supervisor_source_binding`，首CMD、原生事务与其因果取证尚未启动。原错误不含缺失文件名，不能称已逐项扫描失败二进制；终审索引 `205b5f267425d47b1e8e343138a9f3939ace02912cd230ca39bff1ad27aa7d00`，仓外 `363b17a77-ci`。原822夹具失败和更早原生停滞均保留。

当前四文件修复保留完整31来源，显式区分21份生产源码与10份验收来源，监督嵌入表81→86补齐5份生产源码，worker编译字节绑定19→31；失败列出全部缺失固定相对名，不靠后缀删检查。本机Python首轮20通过/1个静态表正则遗漏&失败保留，修正后21通过；check及实际Mac监督程序构建通过。真实二进制扫描确认21份生产源码全部嵌入，原31文件集合保留、扫描前后SHA不变，收据 `edeeafb1fa6649e6e10c40b02d34d06848fd6601ddadfcf0dfbb657041eda521`，仓外 `g09-supervisor-source-binding-fix-v1`。四文件源码索引 `22823692340c0ec2a09ecee9b3963a55a7c267bdf1b576e0526f06ee9cb5a011`，独审 `2e86654185606530d4fdf1d312b30a28afaf79f4a2e9e37e5328c0dc070a368e`；i18n11项通过，七个短根全部清理。首轮监督构建包选择错误退出101的原记录保留；最终门禁索引 `3e3284e388c53fff5fca6ddfb671f8cd35586ab4166803a5d3151c6f8113648f`。Windows运行及最新精确源码门禁待完成；Mac扫描不代记Windows执行。无需本地化变更：仅来源校验及内部诊断，无用户文案、权限、超时或退出行为变化。

独立34e0f2f51的[37095262392](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37095262392)正式hooks复验为success：51合同测试通过，8个真实app-server均自然退出0，实际等待41–523ms，原5s/3s时限不变。关闭收据、stdio_connection_closed、输出EOF、完整Job及配置/私有目录回滚清理相互印证；未进入强制等待分支。原生JSON SHA `be137bd24bc9e0c86c2f97c6eb5f97471bae83daf0071574ca25396c7dcf4f63`，终审索引 `bdec8d0e6b3e3d27725ca1b8fc63c0f350cfe5cc741dab30ead68b9ad46f2940`，仓外 `34e0f2f51-hooks-ci`。该正例不证明日志变化解决了6726旧失败根因，不覆盖ConPTY、npm更新或G09整体；原失败原件保持。

## 2026-10-03：单独复验固定 Codex hooks 的最小工作流范围

新增关闭0项。现有原生补验会连带运行旧版hooks、两版ConPTY及其他CLI和Rust门禁，首CMD专项又不包含hooks。现仅在既有Windows调试choice增加codex_hooks，保持25个input；前置校验要求仅Windows和该专项启用，独立job沿用固定Python准备、既有合同测试、同字节0.156.1探针命令及JSON归档。5s/3s退出判定、20分钟探针步骤上限和全部原验收条件保持；不把新增复验范围当成功或关闭。

源码索引 `c8e489a1c209defc79200b4857f9be9c046f35084be1169952bc8beeb7570f53`，actionlint、28项范围组合、旧工作流结构等价及原命令同字节检查通过；cargo check通过，两个短根已核验清理，门禁索引 `11c0552ca3e21fc9547d6ccd5888c4db8f65acead4e330f1b08a45dd62e09931`。仓外 `g09-hooks-only-workflow-v1`。无需本地化变更：仅CI选择范围，无用户功能、文案或布局变化。当前363首CMD运行继续，不重复或取消；hooks另以精确辅助ref排队复验，原退出失败仍开放。G03/G09保持开放，PR草稿。

## 2026-10-03：修复 Windows 取证测试的排他文件夹具

新增关闭0项。精确82204ea4f的[37089624639](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37089624639)已结束为failure：普通warp1095与command43共1138项PASS，ce6与i18n各11唯一项；取证feature的libtest已成功编译，snapshot首组8项实际4PASS/4FAIL。三项原始错误为ReOpenFile Win32 32/HRESULT -2147024864，截断用例也提前得到api_failed而非预期解析错误；后续creation14、glue3、feature ce6再验及首CMD均未执行。完整日志SHA `48316691936faaf3fb31c8f21d9ca4928433b4a0f98c83ff86804cafd9347f4c`，唯一官方环境ZIP的API大小、SHA与CRC核验，终审索引 `a8b15e6dd5bb424fea6017f418634d52e698ccb3e03f449ffc8995a6a36ae01b`；96条可见warning保留，普通组无LEAK/RETRY/FLAKY。

锁定tempfile3.23.0的Windows匿名文件使用share_mode(0)，阻止持有句柄期间再次只读打开；生产映像租约则允许共享读取。现仅四处snapshot夹具改为临时目录写完关闭writer后，以生产等价GENERIC_READ/FILE_SHARE_READ取得租约。保留原解析、截断、模块归属、回调和游标断言，首例补持租约期间拒写/改名；File先于TempDir析构。creation现有测试为内存解析，无同类改动；生产读取逻辑、权限、超时和通过条件不变。源码索引 `173fb523057b9cbbed6edb9d1667d9b1e06d8b12b654a8d64e00ab84583dd738`，独审无阻断。

本机cargo check、i18n11项通过；Windows creation/snapshot22项仅元数据类型检查通过，不等于实际执行、不覆盖app glue或原生行为。三个短根均核验清理，门禁索引 `a4847fd637c2f208979acd299fd9d163b888ff732ca2437dd82118f36b28469f`，仓外 `g09-witness-fixture-sharing-fix-v1`。无需本地化变更：仅测试夹具，无用户文案、行为或布局变化。修复后目标复验待完成；G03真实Finder拖放、G09原生CMD停滞与固定Codex关停失败仍开放，PR保持草稿。

## 2026-10-03：为原有 Codex 验收保留服务端关停阶段

新增关闭0项。固定b412上游源码的stdio读取结束、processor退出和outbound退出日志写入stderr，但原验收默认日志级别未启用这些阶段。现仅复制start_codex的私有app-server环境，设置warn基础级别及两个已核模块的info/debug，采用JSON格式，由既有Recorder保存；ConPTY复用case也适用。调用方环境、原生RPC、超时、自然退出条件及清理均不变；读取错误和转发失败也可能到达“EOF”日志，不能以该文字独立证明真实EOF或成功退出。

源码索引 `c932ae36a66386e9d55cb6f0d6ef24daf10121da37d63ce7f25450a3fdafe442`；既有hooks/ConPTY回归66通过、2平台跳过，check及i18n 11项通过，四个本机短根核验清理（单独hooks重跑不加算覆盖）。门禁索引 `24938245002f43b2e9be3588614860927a58d6086e60789ca625e7a141953e73`，仓外 `g09-hooks-shutdown-logging-v1`。无需本地化变更：内部验收日志，无用户文案、错误语义或布局变化。未执行新的Windows hooks验收，也未解决原退出失败；当前[37089624639](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37089624639)仍绑定822并限定首CMD，不含此增量、不扩范围。G03/G09继续开放，PR草稿。

## 2026-10-03：修复取证 feature 的测试接线，保留编译诊断

新增关闭0项。精确234dbf777的[37086236420](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37086236420)已结束为failure：普通warp 1095与command 43，共1138项PASS，含ce6降级11项及i18n 11项各一次，无测试FAIL/LEAK/RETRY/FLAKY；取证feature的libtest构建汇总3 errors，25项取证测试及首CMD原生均未启动。具体compiler-message被旧脚本截获而未输出，不能将源码推断当成已恢复的原始诊断。完整日志SHA `5e15771a70779684ddb01e72fd18b44e0e7c9b3ad0ff8db46827a1b78459200a`，唯一官方环境ZIP的API大小／SHA／CRC已核，终审索引 `5c0ad94045b480012d22ca013192f17e179a5393175ef577d7e1608ea2665607`；可见警告及未取得的编译警告明细边界保留。

源码复核定位三处确定遗漏：测试session构造缺取证条件字段，两个next_event调用缺同条件的第三参。现仅按生产相同cfg补None，旧断言、ignore、权限及超时不变；该构建改用Cargo的json-render-diagnostics，产物继续JSON、编译诊断直接渲染，失败仍停止。两文件源码索引 `a5d2fabefc3a8954f99c728e5e39fd94935b9301779f85fe6b05b77f77b7ce49`，本机check、i18n 11项、actionlint与6个作用域条件均通过，三个短根已清理；门禁索引 `f73990142a626c5c4819f3935d2595a33ae9128068049b7b3782c6716cdf6a8f`，仓外 `g09-native-witness-test-wiring-fix-v1`。无需本地化变更：仅测试接线和CI诊断输出，无用户文案或布局变化。修复及ad20句柄释放均待下一精确Windows复验，原生停滞与Codex退出失败未解决；仍9关闭／G03、G09开放／3移交，PR草稿。

## 2026-10-03：退出获证后释放原生取证句柄

新增关闭0项，仍9关闭／G03、G09开放／3移交，PR草稿。精确234dbf777的[37086236420](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37086236420)已通过Windows编译、普通生命周期回归、command归属及CLI监督程序构建，进入限定npm原生步骤；该步先构建libtest并执行25项取证纯测试及ce6降级组，步骤启动不代表首CMD已执行。整轮及原生结果尚未取得，不提前计通过。

源码审查确认新增CreationWitness仍跨过窗口站／LSA清理持有复制进程和线程句柄。现仅在双原句柄退出获证后消费对象、保存安全Summary并释放句柄；确认失败完整交回对象，先恢复持有再报错。结果字段保持兼容，退出前的恢复责任不变。此为诊断生命周期修复，尚无证据证明它导致原来的LSA或CMD停滞，不将其记为原生故障已解决。

两文件源码索引 `728f59633cff9b2263e138095d516e4af391353303054321c25c7d57d09c378d`，本机check `r-4goa1feb`、i18n 11项 `r-8c6zmdy_`通过；Windows creation/snapshot 22项仅元数据类型检查 `r-tefnba2s`通过，未执行测试、不覆盖app glue及实际句柄释放。三短根均核验清理，门禁索引 `19124db872c95279e4d43f6bde12cb7a748ad47971d9c573bbac9d757e3d4dbe`，仓外 `g09-witness-handle-lifetime-v1`。无需本地化变更：仅内部取证对象生命周期，不改变英中用户文案、错误语义或布局。当前运行不含此增量，目标复验留待下一精确提交，不中断现有取证。

## 2026-10-03：修复 Windows 取证模块可见性

新增关闭0项，仍9关闭／G03、G09开放／3移交，PR草稿。精确ae9790698的[37084517646](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37084517646)已结束为failure：Windows步骤35出现3个E0624，兄弟npm_windows_probe调用的绑定、取消与结果方法仍仅对atomic_windows可见。25项Rust纯测试、ce6断言及首CMD原生均未开始，辅助脚本通过不代替这些验收。完整日志SHA `d666c46c8a2fa7bafb1c6941d18f89c3036828d00a48fadabe6f4b0d64d6a3fc`，官方唯一环境ZIP大小／SHA／CRC核验，终审索引 `39e0082e1a4ea7dcce0c7a37d8154e056590f0b9f0bf515da0ce215a5ecaf50b`。

仅将3个方法改为 `pub(in super::super)`，限定到共同managed_process祖先；独审确认无运行逻辑、平台或feature门禁变化。本机check `r-7v93d7_z`通过并清理短根，源码索引 `4a3544816577f2c08dc8a304b3898b63ed99321a21d6f93247250ff38b82c00f`、门禁索引 `b89b6cb2c918d35f5695b88e963cd73d79c96d1a74463d1016f2e4ce56619c3a`，仓外 `g09-native-witness-visibility-fix-v1`。无需本地化变更：仅内部方法可见范围，无用户文案或布局变化。原Windows编译失败保留；修复后同范围目标复验及原生取证仍待执行，G09原生失败不变。

## 2026-10-03：保留 Codex 关闭错误及独立计时

新增关闭0项。固定Codex hooks驱动现在记录stdin关闭前后状态、正常返回或异常类型与errno/winerror，并分离关闭与实际等待耗时；不记录错误正文，不将closed=true当作服务端EOF，不改5秒自然退出、后续监督清理或通过条件。两文件冻结 `fdcbff3b9e939aae2f312ac98ef87e3611a3407438a1241d3427dc541e05211a`；本机Python49通过／2平台跳过，check通过，两短根已核验清理，门禁索引 `a61a41747824c0f17433384fa23e8f4ece1ab5fd4e604830a8b714debe86a997`，仓外 `g09-hooks-close-receipt-v1`。无需本地化变更：内部验收收据，无用户文案或布局变化。

此修复仅保留原来丢失的诊断，不解决6726的原生退出失败。精确ae9790698的[37084517646](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37084517646)仍独立执行限定首CMD取证，不包含本增量；未扩运行范围或重复全矩阵。当前仍9关闭、G03/G09开放、3移交，PR草稿。

## 2026-10-03：Windows 原生停滞取证接入，原源码门禁终审

新增关闭0项，仍为9关闭／G03、G09共2开放／3移交，PR草稿。原fcd的[37074002886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37074002886)已自然结束为failure；两份官方ZIP大小、摘要和CRC均核验。Windows全日志5041次普通PASS、预期28项逐名PASS，i18n22次执行／11唯一；唯一失败仍为步骤44旧平台拒绝断言，三次尝试共两次自动重试，无LEAK/FLAKY/TIMEOUT。完整Windows日志SHA `54e6ba6524c53b110882c90e6ccc45168da62435986eb19ee5a0e968447248e0`，终审索引 `129e2d87ce557eaf5d49e56d5c671a8538741ac496697c8197a00060efe66a2c`；Linux旧编译失败保留，873修复复验通过。ce6的Windows断言修正仍待目标复验。

基于ce6接入默认关闭的原生取证功能，仅专用测试显式选择的首个固定Codex npm CMD generation可启用。使用原Job内身份复核后的进程、线程、已验证映像句柄及原helper持有的实际桌面；原调试线程一次采集取消前上下文和有界栈，核验实际CMD创建入口／返回并恢复调试寄存器，映像卸载先撤销断点。只输出结构化身份、映像摘要／偏移及数值错误，不输出原始栈、路径、环境或参数；未知身份和恢复失败仍走原严格清理。不修改子进程权限、令牌、产品超时或通过条件。此为根因取证接入，尚非原生修复或G09关闭；20文件冻结索引 `05074a741d6e2e8b5b285ae32870c6488905372acd9abae8cc6a58d6c320318a`。

冻结后本机check `r-3ys3j6i0`通过，定向377普通PASS、i18n11项、Python16项及workflow作用域检查通过；command的Windows目标交叉check通过。应用完整Windows交叉check在aws-lc构建因本机缺windows.h/SDK失败，尚未检查应用代码，不重复该环境。22项Windows纯测试仅通过元数据编译，25项实际执行及应用接线编译仍待runner。各本机短根均已核验清理；本地门禁索引 `728251fe1f54eb6e3200e87474964b2d21340f2b42797f8995ac51bff8bf2f21`，仓外 `g09-native-witness-integration-v1`。无需本地化变更：内部诊断、编译开关与专用验收入口不改变英中界面、错误语义或布局。

下一轮只选择同一首CMD候选，先执行25项原生纯测试及ce6降级断言，再获取创建返回、线程和桌面对象证据，不重复全矩阵。G09另一固定Codex hooks失败经CPython3.13.15与固定Codex源码核查，确认关闭错误被吞、关闭与等待计时混合；但刷新异常仍关闭底层，不能据此断言stdin写端泄漏，仍缺服务端收到EOF后的阶段证据。G03真实Finder拖放仍待完成；当前两项原生失败不能归结为设备缺失。

## 2026-10-03：Linux 修复复验通过，Windows 平台拒绝断言修正

新增关闭0项，仍为9关闭／G03、G09共2开放／3移交，PR草稿。精确 `873a7eb44ce754d2efd36ed859f1e970769e3d62` 的 [37076566107](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37076566107) Linux源码门禁成功：5427普通PASS，新增/增强17项逐名通过，另rust-genai 81项通过；无FAIL/LEAK/RETRY/FLAKY/TIMEOUT。updates275，i18n22次执行／11唯一；步骤范围与770一致。两个官方ZIP的API大小、SHA及CRC全部核验，最终索引 `228a6a620d06bffd284f56616d3e2f08cf45817544fbf210ae605513e248e583`，完整日志 `7b0e0e84c6a802afdd361f2c48ec9bb1b88b0505fe63d66a1372a003c8fc2acb`。Windows/macOS未选择；远端辅助临时根因ValueError仍retained、cleanup_ready=false，不称清理完成。此轮只证明源码门禁，不覆盖原生专项失败。

原fcd的 [37074002886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37074002886) Windows步骤44已失败；认证Actions界面显示3359项中3358通过、1失败，第三次尝试仍为 `new_consumer_migrations_are_unavailable_on_other_hosts`。`sources_claude_downgrade_tests.rs:376` 实际 `InvalidRelease`，旧预期 `UnsupportedPlatform`；当前Windows x64已实现限定降级，未经审核的280→285应继续拒绝为InvalidRelease，280→287的validate应拒绝为RecoveryRequired。现将Windows两条拒绝单列，其他未支持主机保留原断言；生产允许集未变，独审无阻断。原run继续自然收尾，完整官方日志及其他用例终审待齐，不以界面片段代替全量通过。

本机check `r-8a38tho1`、聚焦 `r-qxb634of`14普通PASS／8705 skipped、i18n `r-ks2e0nws`11项通过，三个短根均核验清理；门禁索引 `e89f6959238c94b7ca77fa923cb80bbc77d0dde235f995f58c565fa43b4ccda3`，仓外 `g09-windows-consumer-test-fix-v1`。Mac未执行新增Windows cfg分支，修正仍待精确Windows源码复验。无需本地化变更：仅测试平台分类和错误预期，无用户行为、文案或布局变化。

G03仍缺真实Finder拖放；G09仍有Windows首CMD候选停滞及固定Codex hooks未自然退出。已找到现有runner可采集取消前原生线程、实际创建调用返回和桌面对象关联的方案，归档实现尚未接线或执行；不是已修复证据，不关闭G09，也不把问题归结为缺少设备。

## 2026-10-03：精确源码门禁捕获 Linux 原子改名遗漏

新增关闭0项，仍为9关闭／G03、G09共2开放／3移交，PR草稿。精确770的[37063514848](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37063514848)两平台成功：Linux5411、Windows5015普通PASS，新/增强5/4项逐名通过，updates261/157；i18n各22次执行、11唯一用例，无FAIL/LEAK/RETRY/FLAKY。3个官方ZIP按API大小/摘要和CRC全部核验，最终索引SHA `1041bba3460c38feeb735633f82cb04e88d5675525dc86b879594a8da62c71a8`。Linux Grok辅助临时根仍retained/cleanup_ready=false；只有收尾ValueError类型，无消息/栈，源码可证未进入递归删除，不能宣称远端已清理。外部原生专项按模式跳过，6726失败保留；此结果不外推后续三批降级代码。

最新fcd的[37074002886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37074002886)中，Linux check因Snapshot::file_manifest/verify_file及Directory::rename_noreplace仍限制Mac ARM而失败，共6个E0599和4个级联E0282，退出101；后续普通Rust回归未执行，新增16个预期用例均不计通过。完整Linux log SHA `e8fbf51a67b6132d773b1ab97045230f66c0968a7b54da415da06a077910c21d`，早期审计 `1b6051c1671779fa8c44a0c169b5f07f06a1972574d32266fa5dd37e6352cbbb`；Windows同run仍在check，不取消或重派。

修复将三个辅助方法的cfg与migration的Mac ARM/Linux x64范围对齐；Linux使用同一已打开目录fd上的renameat2(RENAME_NOREPLACE)，名称仍仅单叶，失败不回退到覆盖式rename，成功后同步目录。Mac RENAME_EXCL和Linux严格Caskroom权限边界保持。原不覆盖测试现纳入Linux，增加空目录目标身份保持断言，能够识别普通rename对空目录的覆盖。两文件独审无阻断，源码索引 `ac2567cc5af7c2fc1efa3e453a5faab3d52115c68a28f86d04eecac40dbb7b96`，仓外 `g09-linux-cask-rename-fix-v1`。

本机check `r-okvoc561`、定向 `r-51p24t1p`372普通PASS／8347 skipped及i18n `r-eem538cw`11项通过，无FAIL/LEAK/RETRY/FLAKY，增强空目录回归逐名通过；三个短根已按退出和打开文件证据清理，门禁索引 `c81be511610e35ce51455c15ac5913b3fff4168e19ea4991629cbcf7b49a7f64`。无需本地化变更：底层平台实现修正，既有英中错误/恢复文案与布局保持。Linux新增syscall及修复后目标编译仍待平台复验，未执行真实CLI更新。原run自然结束后验证最新修复提交；G03真实Finder拖放和两项Windows原生失败继续开放。

## 2026-10-03：WinGet 合法降级、登记与配置恢复

新增关闭0项，仍为9关闭／G03、G09共2开放／3移交，PR草稿。Windows x64 Claude WinGet已接固定286→Stable285；用户明确Stable、实时渠道一致且原PE/登记/配置完整绑定才执行。ConfigPublishing前恢复旧PE和ARP两字段，外部配置/策略变化保留并留账报错；提交点后验证候选/退出收据，再幂等发布Stable，PE/ARP/配置三方完整一致后才清理。PreparedCandidate摘要在发布和恢复时重算，意图、版本、映像、源配置/策略和收据均绑定；旧280账本及成功语义保持，新285只有完整前滚成功才清除活动失败标记。

固定WinGet提交 `f0fb65e5263ef3f223bb0a750b6d1246189c5956` 的285和 `863c5234aebe4890d1e79addfce9fa8178edcfeb` 的286完整728B manifest分别匹配SHA `49a958e559951e096ad13db8e07a2a0529388ecaf9349af8ab31f18da878eebd`、`16baff02f9f3aca7d8c5a7ba706b556c9a9da9290dc2d4b151ddf45a5d324d8b`。285原生243751072B／SHA `121fc8151ed40bd9c144d68aa1cea23427803628ffab65e23da1cceda155697e`，286原生245092000B／SHA `0b6ecec8edef1ce3069682f696c894d1553239faa6b84e56e167a3ced88fa333`，完整字节与两官方来源一致。285复用已核npm映像重新哈希，286完整下载；不存在的287 WinGet manifest未拼造，ARM条目不代表准入。静态PE导入/证书检查不代记Windows签名信任或原生执行。原件审计 `808583c3a143b6b7e998672af67bc18dc78a81fb39165d37f3c2ba8912431dab`，13文件冻结索引 `a4cd2a0ec7d4ff1007fb82f90af8601c1b79e1097b4e86ac2028aa568fc24fb0`，仓外 `g09-windows-winget-downgrade-source-v1`。

基于56b729001的12文件源码索引 `3e5519bb649c442d6359617e399f7f0ccd920340a9fccd628cd9e96cd48cc9b7`，独立审查 `d9d5f1caa60a8b0c7a7b1616d450770011f8d7ac8bf6441819014404ff367dc8` 无阻断。Mac check `r-qa39z5n7`通过；定向 `r-4hvheo4b`372项普通PASS／8347 skipped，无FAIL/LEAK/RETRY/FLAKY，log SHA `8f218c5fd40f9ed722fb779e27b365fa61221cedd89f02c895540b09348038dd`；i18n `r-8plntuea`11项通过，log SHA `42083610e93169f1d5fd782a0666ec7b8f8d304fd0674f0d56980b1584067eac`。三个短根均已核验清理，门禁索引 `a40dc0b73cd4f2613c9f9115c3946964197f58ebd230634b44f005a95685cda3`。无需本地化变更：既有英中更新、渠道、失败及恢复语义和布局适用。

新增Windows专属文件/配置阶段恢复、ARP字段和旧账本兼容回归仍待精确平台门禁；测试不实际修改注册表，也不代替消费者WinGet原生事务。旧770门禁自然结束后仅派发包含Linux Homebrew、Windows npm及WinGet的最新提交源码门禁。有限源码复核未再定位明确未实现原生分支；Windows实际npm首CMD候选和独立固定Codex hooks自然退出失败根因未定，继续单列，不能写成仅缺设备。其他平台实机验收按授权移交，G03真实Finder拖放仍欠，不关闭G09。

## 2026-10-03：Windows npm 合法降级与旧账本兼容

新增关闭0项，仍为9关闭／G03、G09共2开放／3移交，PR草稿。Windows x64的Claude npm已新增固定287→Stable285合同；只有用户明确选择Stable且派发前实时渠道仍为285才能执行。原287完整树、候选285全成员、双shell探针输入/实际版本/退出收据与账本相互绑定；恢复先复核意图和完整版本合同，再按已有ConfigPublishing提交点向前收敛或回滚。旧schema1缺少downgrade字段按None读取，旧三参探针仍仅280，新四参仅285，原278/280→280事务保持。不放行287候选、未知升级/降级或混合目标，不修改AppContainer、系统权限和超时。

完整官方wrapper与win32-x64归档的SRI、11个成员、mode及native均已核。285 native为243751072B／SHA `121fc8151ed40bd9c144d68aa1cea23427803628ffab65e23da1cceda155697e`，287为247736480B／SHA `6d5be51f0edfe3429cd1a9885950db31c1e59eb9a2c9a30a47e64179d2ff98b4`，与官方manifest一致。安装脚本和shim与280相同；PE两版均7直接导入、12延迟导入，静态证书解析不等于Windows信任/吊销校验通过。原件审计SHA `32bfada515686525d34bc9aa7a2ef5511c71fe4213c5d16954492965db6c82c5`、52件冻结索引 `5fde811a2877b5892620ab0918a8aa6f496b9a882e4c39a6eb40ae910922feca`，仓外 `g09-windows-claude-downgrade-source-v1`；未执行CLI、安装或真实更新。

基于9ef55fb02的16文件源码索引 `975a39232cae2fe4ec6be891165a829bb913fa4b0bf8e96ed62e669946563ee9`，独立审查 `903f26c8e7ff5c5b1a3d9a3012f2274fbcd6f096852b09656447f6fb5a2ff3b5` 无阻断。Mac check `r-mcxcqubd`通过，log SHA `7cac323e4c1e57986f4bd96b20aff9afdb86c32a9f9a5d05bb575c284943b2a6`；定向 `r-q0ca8_fl`为368普通PASS／8347 skipped、无FAIL/LEAK/RETRY/FLAKY，log SHA `e54e32ecfa28857234076c09565b13b3c0d8a022e9d166908628b80beee2a9a0`；i18n `r-iixudjym`11项通过，log SHA `f25a5a9b9755a54c877ffa7cdbe3ec5bc14940a64ad2cc937bde0d1c76d70021`。三个短根均已核验清理，本地汇总 `461ba8b910984eeb535b1d95f352189f08fe959c3587959f64a1e30395724276`。无需本地化变更：既有英文/简体中文渠道、更新、失败及恢复语义仍适用，无文案或布局变化。

对应Windows专属编译、旧账本真实文件恢复与硬链接回归尚待精确平台门禁，Mac合同测试不代记Windows通过。待旧770源码门禁自然结束后，按最新完成实现的精确提交运行两平台源码门禁，避免同分支并发取消和重复已知原生失败。WinGet固定286→285官方manifest和完整PE已核，实现继续推进；Windows实际npm首CMD候选及固定Codex hooks自然退出失败仍保留。G03真实Finder拖放仍欠，本轮代码和测试不关闭G09。

## 2026-10-03：Linux Homebrew 合法降级实现与原生范围复核

新增关闭0项，仍为9关闭／G03、G09共2开放／3移交，PR草稿。Mac消费者链原证据保持；原生实现不能随其他设备验收移交。源码复核确认Linux Homebrew此前所有合法降级都被拒绝；Windows npm和WinGet也缺少受审核来源、渠道、配置和恢复绑定的Claude降级合同，独立于Windows实际npm运行失败，继续列为实现待办。

官方Stable历史由277直接升到280，没有Stable278，未从已有278映像拼造cask。固定Homebrew提交 `8d9df9ae501d586458789ecbf01115d91fe838c0` 的两份完整Ruby原件及Claude官方manifest已核；Linux285为240327864B／SHA `33dad1ec615a2e08cc78b494f05c110e49916de2c79d78ec8799ebf46b233d29`，287为244317368B／SHA `3920489a5109cff5786a1a392c25277408ff22bc796d5edb9c16a60e5a1718f0`。完整下载与两上游摘要一致；静态ELF使用既有六库和解释器，无新增加载属性或系统目录权限。合同SHA `d72cadf4e1d2501d7991450a716f8f6a11321561fd334713820df700ab660917`，16件原件索引 `09111509a170b1dcef2d78fc83784ee690f9f3d1324b7e8945b2161224aa9982`，仓外 `g09-linux-brew-downgrade-source-v1/current-287-to-285`；未执行CLI或真实更新。

新实现接通Linux来源发现、精确287→285 Stable意图、双cask事务及无网络冷恢复；发布前重查Claude Stable和Homebrew Stable，固定原cask下载严格校验。旧树287、候选285、登记/入口/配置及恢复账本逐身份绑定；仅候选285获新增原生准入，公开285/287及stage287继续拒绝。普通280和旧恢复合同保持，Linux npm未扩展。审查发现的新版本检查/执行不一致已在生成计划前修正：280→285/287与285→287不再产生可执行计划。

12文件冻结源码索引 `5a4dd0641fac942342a0880a3fa2e79e01a049e0bb25a4688ea9987f077416b0`，基于7707733b6。check `r-2xo4z2jd`通过、短根清理，log SHA `7bca83a1fcde7a0e60fa4d80ff2db19ae48d1a03245d3303576728fc42b38cf4`；本机定向 `r-o9v7lkls` 共364项普通PASS、8347 skipped，无FAIL/LEAK/RETRY/FLAKY，log SHA `f3e4e1168a9bc25698ec4e03a802f6f103155c24707020a9f924a396bc0c2dce`；i18n `r-lii11vxs` 11项通过，log SHA `6a435e27e418d8ae2fef8e130d09290a32ee61486d38a2c00c07073b03f3bb26`。三个短根均清理，本机门禁索引 `8303bb015be46163608ce1b1503fadd3dab702da2d9c2f9496f3196b3fd6299d`。精确提交Linux/Windows门禁待完成，新增7个Linux专属回归不能用Mac检查代记通过。无需本地化变更：沿用已审英中检查、更新、失败与恢复语义，无文案或布局变更。其他平台真实验收仍后置，不宣称本轮Linux功能运行通过或G09关闭。

## 2026-10-03：Grok 真实 Homebrew 多入口更新、应用冷回滚与中文重试

新增关闭0项，保持 **9关闭／G03、G09共2开放／3移交**，PR草稿。复用v4构建 `r-9a71ln4q`，`0d65c2b88` 的9份产品源码逐SHA等价，不称当前提交重建。真实官方Homebrew7.0.4安装Grok41，沿用原GROK_HOME；GUI `r-tfdpxo3a` 实时发现Stable46。事务 `b5dae3b4-68d4-4dac-9805-42d6cd0fa1bf` 在ExchangeIntent已发布完整46树、`grok/agent`双入口和bash/zsh/fish三补全，旧41回滚树及入口仍绑定；9个已注册原生探针全部退出0且清理闭合后，仅向精确出生身份和签名映像的GUI64863发送一次SIGKILL。中断独审SHA `04dd704588fb7fb29aa77ac3d0dadea1f2681999f0fb662ef0b063ebb97b9f9f`。

同环境正常重启66934，无检查/更新点击，自动还原全部旧41成员的身份/权限/大小/摘要、两个原入口及三补全字节/权限，活动journal/stage消失，GROK_HOME保持；完整恢复SHA `e7a09b8e18bcc8b31e1feb2d56c3e624b7e89a76086031e9c33a71832caedfe1`。两个公开入口在GUI终端各一次返回 `grok 1.0.41 (4220f3b224a6)`。英文失败提示完整保留；经UI选择简体中文、正常退出并以原环境重启69404后，失败提示仍在，只点击一次空闲更新。事务 `aa41b66e-ec20-4d93-b33a-17c36c6e8d71` 成功：固定官方映像150374256B/SHA `e8daa302364c9c3b6a5546d511cfbd1ab5e5d407a9b04282f660665ea405f9f3`、完整4文件树和metadata/tab、双入口、三补全全部核对；11代原生均退出0、绑定摘要及PID/launchd清理闭合。成功独审SHA `8f219008c482038312040be3ab25ca7ee42fb0319076a48bd12a09c4b93a734a`。

中文GUI显示Installed/Target46、Homebrew/Stable、渠道匹配，原失败消失，独立通知插件缺失提示仍在。真实终端公开 `grok --version` 和 `agent --version` 各一次均返回 `grok 1.0.46 (2765805b9442)` 并回到prompt；观察SHA `4d75f0766b2e08f609efddfdf3096e11eecb4cd2261a9b12d943147d6da52f3e`。无需本地化变更：英文/简体中文失败、重试、版本、来源、渠道及插件提示完整可读，无截断重叠；原AX/截图保留于会话工具输出，仓外JSON不冒充截图。没有模型请求，插件缺失不计兼容通过。真实Brew准备时缺官方tag导致tab不符的失败保留，补齐官方tag并真实重装后才验收；独审初版JSON字段顺序导致摘要失败亦保留，仅修正审计器，未改产品收据或重复恢复。

按G09原包管理器关闭条件复核，Mac消费者、来源绑定、渠道、忙碌、合法降级、更新、失败回滚和应用中断恢复已齐备。Codex/Grok同token Homebrew共用 `recover→rollback_publication`，本轮Grok还覆盖双别名；不将Codex完成后重启写成它自身中断恢复，也不新增每款乘场景矩阵。Claude跨cask独立后端有自身真实冷回滚证据。最新源码门禁仍待运行，Windows真实npm及固定Codex hook自然退出失败继续独立开放，不能用Mac通过或清理通过关闭G09。

两次正常重启GUI均退出0，原GUI计划中断使outer如实exit247；合并544条PID/出生身份及20原生代的40个内核身份和launchd全部退出。小证据归档后，root/profile/state/专属Preferences和observer短根精确清理，20个已归档只读快照目录只为删除加owner写权限；真实brew46及GROK_HOME保留。最终604件索引SHA `fead2230435b5afa2378295ec557baa6c9ea319274df94ce18784196ea17f790`，清理SHA `54a8603633f0bb82dbe287ee2bf25a336aec9da2e63a041b757acefd4e1f63c7`，仓外 `g09-grok-brew-gui-v1/independent-tools`。

提交前check `r-2crokqb2`通过并清理短根，log SHA `c9fba70a63cb4ce1d05e20f24a9ddb79cc8c4dd4abfc0087172676d3a369829c`；产品源码未变，沿用原v4定向364和i18n11，不重复同范围测试。

## 2026-10-03：源码门禁与真实 CLI 专项分开执行

新增默认关闭的 `source_gate_only`，完整保留原普通广筛和更新器、两端cargo check/i18n/command/IPC及平台辅助回归；只读Node绑定、合成Grok hook和自建SSH参数仍运行。固定外部CLI准备及真实hook/更新/恢复/GUI专项单列，避免重复没有新因果证据的已知失败。独立scope前置job拒绝与full、任一专项及非默认专项选择混用，所有执行job依赖其成功，不能通过步骤的 `!cancelled()` 绕过。默认、push和合法full行为不变；51个输入组合（22合法/29拒绝）、actionlint和diffcheck通过，原普通run/filter字节未变。远端尚未派发，不计源码通过。

此为测试阶段拆分，原关闭条件不减：6726的Windows固定Codex hooks退出和真实npm首CMD候选失败继续保留。源码门禁即使通过也不表示原生失败修复、G09关闭或PR可合并。无需本地化变更。

## 2026-10-03：Grok 真实 npm 消费者与用户镜像更新

新增关闭0项，保持 **9关闭／G03、G09共2开放／3移交**，PR草稿。真实官方npm安装1.0.41及其postinstall用户镜像由GUI `r-4dr6ibtn` 自动发现实时Stable1.0.46；保留原GROK_HOME，只点击一次更新，无额外Check、重试或模型请求。沿用原v4构建，9份非审计源码与279630逐SHA等价，不称重建当前提交。

事务 `e306fcea-59ba-414f-b8e8-f1ac8d90fd66` 完整29官方文件的SHA/大小/模式与prepared树身份全部匹配；公共链接原inode不变，包内第二级链接正确，GROK_HOME新映像/新链接与journal一致，旧41镜像和原config SHA保留。Node/npm等12组件5928项完整重核不变。唯一代次 `4a4f7af5-b7c4-41f6-b688-bba46a232146` 的原生绑定摘要独立重算，exit0、同代claim/launch/exit与CID销毁闭合，精确launchd查询113，活动journal/stage及mirror暂存项消失。完整独审SHA `4ac1f006ce26839113e462905983f9c7eaf80c1b4ac82898fd2960abce4defbb`。在原GUI真实终端执行npm公共 `bin/grok --version` 与原GROK_HOME `bin/grok --version` 各一次，均实际返回 `grok 1.0.46 (2765805b9442)` 并回到prompt；GUI观察SHA `2d67e6bf85d1c47edfbc4d705b356d6fc016e6efd2d36033ebb576fba2042dd3`，原AX/截图位于对话工具输出。

本轮英文版本/来源/渠道匹配、独立通知插件缺失提示完整，无需本地化变更；既有共用英中审计按原构建计证，不称本轮重验中文或插件已安装。两次独立审计脚本因原schema字段/APFS目录计数差异失败的原件保留，修正后完整核验通过，未重跑产品或修改安装。14个已见进程及observer退出，GUI和outer均exit0，root/profile/state/精确Preferences及observer短根清理；真实prefix46和GROK_HOME按登记保留。最终独审SHA `2180c2d5e5e45d4621811feae01ac2241cda10a5e9a16c9118b79da10c46191e`，86件冻结索引 `09b08e18bd688ed872a22e76a5f7cb173dfbcd37755d1fcb06e095e3441742d9`，清理 `a6d25a04c93ffc243f28119e5769af7888f0ea303b3c1fe9af076f8873c32aba`，仓外 `g09-grok-npm-gui-v1/independent-tools`。

本次仅3份审计文档变化，提交前check `r-3wfftna5`通过/短根清理，log SHA `0a5f05242b6dd7fe51f8e8dbd5d8c2f8d59612c6e841e2d7b81babb90d9ca140`；原v4定向364项/i18n11项及源码绑定沿用，不重复同范围测试。Grok Homebrew双别名/三补全的更新及应用中断恢复、Windows原生npm及最终源码门禁仍欠，不关闭G09。

## 2026-10-03：6726 两平台门禁终态与 Windows 原生阻塞

精确 `6726e735994568e0f2b0995a068d6c5bb32924f1` 的 [37045744557](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37045744557) 终态failure：Linux success；Windows failure。普通nextest为Linux5406、Windows5011普通PASS，未见FAIL/LEAK/RETRY/FLAKY；Linux17新/增强、7项Python读端、Windows3新调试及1增强回归逐名通过。i18n两端各11唯一用例，22次运行不加倍计数。Linux旧installed Grok hook失败同名本轮通过，历史失败保留。Windows独立原生专项失败不被普通回归覆盖。

第一处为固定Codex0.156.1在首次untrusted hooks/list后未于5秒内自然退出，驱动最终终止exit1；未发模型请求，Job在Resume之前已绑定且清理确认。同源驱动和CLI旧轮曾自然退出，不能冲销本轮；官方源码EOF关闭合同未证明缺显式shutdown RPC，父端EOFclose错误被吞亦使EOF真正到达子端未独立确证。第二处为真实npm首updated CMD候选ProbeFailed/Interrupted：实际spawn/Wait/validate/Continue均thread20544，已接47事件均验证并继续、pending为空，2689次Wait中2642超时；匹配Node租约文件身份的进程按FILETIME记录已在终止请求前创建（creation_age_at_stop_ms=283802），其CREATE事件仅在取消清理阶段收到；不将独立计时起点相减，也不证明父CreateProcess已返回或Node用户代码已执行。不能据此继续声称debug循环停滞、线程迁移、漏Continue或Node从未创建。stderr原“拒绝访问”未绑定具体API；本次清理退出3221225738不等于旧Node初始化0xc0000142。

19份源码及helper映像、generation/manifest/退出摘要链和Job/AppContainer/私有窗口站/桌面清理原件已逐项核；清理通过不计npm成功。仍缺取消前root/Node线程原生栈及等待原因、CreateProcess调用返回边界和实际station关联；CSR/console等待只是未证假设，不能据此修改权限、控制流或超时。PowerShell及其余四场景/冷恢复未执行，不重跑近似探针。原生独审SHA `502a878c67233048659227a75f74615dddf9b29e6eeac6142237d19de4df9796`，因果边界 `a067b8a978287fb86ffa45e9b61d317469e62843094f5efe9e5fbd134fa596b3`。

14官方ZIP/105条目全部按API摘要、大小和CRC核验，终审SHA `23afb980553d6b8c0c779c745b0eaceb3fc3d35577cff4e6ec2e9805ccb72396`，最终索引 `754a149e13cfaabd416c764dd2ec8827b4804b3e578987a6935e43b465dc95ec`，仓外 `6726e7359-ci`。大ZIP首次传输EOF的partial保留，只有工件只读重取，没有CI/原生重试。本轮仅计6726；新增279630应验两端cargo check、updates全模块和i18n，含Linux5/Windows4新用例。现有workflow无法独立关闭固定Codex原生专项同时保持普通广筛，暂未重复派发或为绿色调整范围。G03/G09保持开放，PR草稿，新增关闭0项。

## 2026-10-03：Codex 真实 npm 消费者更新

新增关闭0项，保持 **9关闭／G03、G09共2开放／3移交**，PR #22草稿。真实官方Node22.14.0/npm10.9.2前缀由签名GUI `r-1bvid119` 自动发现Codex0.156.1与Latest0.160.0；仅一次空闲更新，成功后公开 `bin/codex --version` 实际输出 `codex-cli 0.160.0`。沿用上一节原v4构建 `r-9a71ln4q`，`279630e44` 的9份非审计源码与原冻结逐SHA相同，不称该提交重建。

事务 `742a5777-d7dd-4e1c-a3a2-8b25ee733084` 的完整47文件新树匹配固定官方SHA/大小/模式及journal prepared身份，公共symlink身份不变。代次 `0b69add8-e8ee-4d79-8cc4-d7349624fe9b` 实际绑定原Node执行暂存 `codex.js --version`，48项文件闭包和摘要独立重算，原生退出0、claim/launch/exit及coalition/launchd清理闭合，journal/stage移除。15个保护组件5953项完整重核，Node/npm/Claude及私有配置未变。完整独审SHA `bb8f2aa8a0ef1d1a4b7a9f689dd1274b15e967260ba353a0005224d1c33ae21e`，最终独审 `49ed70daf4a4a7e9fa0500ece6fab4cd259a4240002505c6ea15d0ec29f78b0d`，76件冻结索引 `a4d2c5370a5b12d26a8cf383b01375bfb80f451a3b84c4d1e5e778f563e69a0b`，位于仓外 `g09-codex-npm-gui-v1/independent-tools`。

无需本地化变更：本轮英文版本、来源、渠道匹配和两个独立插件提示完整，原英中共用语义和布局按既有构建保留，不称本轮重新验中文。未启动模型或宣称插件已安装，未扩展忙碌/中断矩阵。GUI、已知后代与observer正常退出，外层exit0，log SHA `775d899652628fe328af14f74daf6e978a1ae141e7b1a228f0b0c95829ea2523`；本轮root/profile/state/精确Preferences/observer短根清理，真实npm前缀保留160。清理SHA `f4fc969788068d0aa96e5e876cbe7fa38217fb227e7173fea3894f32b5562ae0`；20个已归档私有只读克隆目录经绑定后加owner-write删除，3个内部arg0链接只unlink，不跟随或改真实前缀。归档初版因默认Follow未显式持久化update_channels而KeyError，原失败保留；v2仅如实记录字段缺省，未重跑产品。Grok真实消费者、Homebrew多入口恢复、Windows原生npm与最终源码门禁仍待完成。

## 2026-10-03：Claude 真实 Homebrew 跨 cask 降级、忙碌延期与应用冷回滚

新增关闭 0 项，保持 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 草稿。官方 Latest 与 Stable 使用互斥的 `claude-code@latest` 和 `claude-code` 两个 cask，既有同 token 更新不能承接实际 287→285。现以独立迁移 journal 绑定两个 cask 的固定官方源码校验和、原生 SHA/长度、完整树、公共链接及配置；只接受显式 Stable 及实时匹配渠道。双锁下先准备并探测候选、发布新 token、交换入口，再退下旧 token 和发布配置；恢复不访问网络，提交点之前完整回滚，持久化配置发布意图之后向前收敛。原 journal 格式及未知版本拒绝保持。应用首次检查前的启动保护补齐所有实际活动 journal，已退下的失败记录不冒充待恢复事务。

冻结 `g09-claude-brew-build-v4/source.safe.json` SHA `460576cfd5f7a03d6fd68d0e0ce552aaf850804f3e41cd58972ae67e76218e59`，基线 `6726e7359` 加明确 diff；构建 `r-9a71ln4q` 的 raw SHA `f3a9f9b416a5a0e74bcd276bc7b7cfd36928aa746a5ca92fbfa26e38f256faf4`，签名 GUI `r-drgt6zkc`。本机定向 `r-wk6se3vt` 364 普通 PASS、8347 未选，零 FAIL/LEAK/RETRY/FLAKY；i18n `r-f7lji1oe` 11 项、最终 check `r-qvwg_o9d` 和 build 均通过，短根已清。门禁汇总 SHA `bca56ac633325eb4fa1df3955cc0bed76e1ddbd579a9d5c030e0070ac0e6e3c8`。此前两个导入编译失败和已被 v4 替代、未做 GUI 的 v3 构建原件保留，不回填为本轮结果。

真实 Homebrew 7.0.4 安装的 Latest 287 由实际 GUI 自动发现 Stable 285。旧 Claude TUI 活跃时仅一次点击空闲更新，完整旧树、公链和配置不变，零 journal/原生代次；通过应用确认关闭本轮旧终端后自动继续，没有第二次检查或更新。旧 CLI 退出由精确 PID 消失证明，不称自然 exit 0。事务 `59acca1c-37fb-4aa3-907f-21b672d3387f` 达到 RetiringSource，公开入口已换至285、配置仍 latest；仅向绑定出生身份及签名映像的本轮 GUI 40935 发一次 SIGKILL。完整现场独审 SHA `491c6b81edebe254026144f781ec8d2fee85a03b6c44aed28c4a4d2d05a0866e` 确认原/新树和链接均匹配 journal，候选退出0、原生清理闭合。原环境正常重启44643，未点击检查或更新，自动将全部原287树及入口的身份、权限、字节/摘要和原配置还原，移除新token、stage、backup及活动journal；完整恢复 SHA `58bfddf487f7e55bf4b5ec96347fdff9bd2cb3fe452301164950529de5e8bf4b`，英文保留可读失败/重试提示。

正确环境的中文实例45681只主动重试一次，事务 `e0ffbaab-18cf-47d4-9ca1-3d3cce139c1e` 成功提交；完整目标树等于 prepared，公共入口指向285，原 token、所有暂存/备份和活动 journal 消失，私有配置仅 settings 的渠道改为 stable，其他成员与原 Codex 65 项不变。新候选 `3c6799d6-4c67-46b8-81d6-4a4bc24012ca` 的7份身份/退出收据闭合，exit0、双PID退出、coalition销毁及精确launchd job不存在；完整独审 SHA `1eb44201ce90eedecac82dbaf3422fd7c35e7cc3720bafcd911fdb36dbd47df5`。真实 GUI 新终端公开 `brew/bin/claude --version` 返回 `2.1.285 (Claude Code)`，设置页 Installed/Target285、Homebrew/Stable、渠道匹配和两个独立插件提示完整可见。中文实例正常退出0。

无需本地化变更：既有英文/简体中文渠道、忙碌、失败、重试、成功与插件文案仍适用，实际英中布局无截断或重叠；CUA原截图在对话工具输出，仓外JSON只作观察记录。英文退出后误调用已退出绑定的读取接口，导致工具通过 LaunchServices 意外重启44986，未保持私有PATH/TMPDIR；该实例曾显示未知来源并做一次手动检查，未点击更新。已核实其退出后才绑定正确45681，不把错误实例计入产品验收。相关原件及旧PATH仅捕获前缀的限制均保留。当前真实消费者来源余项、Grok Homebrew 双别名/三补全的中断恢复、Windows原生npm和新提交两平台源码门禁仍待满足，G09不关闭。

本轮4代GUI及全部已见后代、2个观察器、两代原生job/coalition均退出，root/profile/state/精确Preferences和观察器短根已清理；仅2个已归档0500快照目录在退出证明后由原fd加owner写位以删除，真实安装权限未改。第一代计划中断使外层如实exit247，日志SHA `18bbb01a0ad862d6ddda1c9dfaa3adf8f522d3fa12918c262f2242d0b604d2bb`；后两正确GUI正常exit0。清理收据SHA `8bbb7cafed44d1aac7964e23f5924e6fbf1264e6623dd4811ecf2183709cb72d`，观察器清理 `60a6770114d054fee1514c45e0154bab969b6cd979daba72865efd15f214d27d`。真实Homebrew前缀按登记保留，小证据位于仓外 `g09-claude-brew-gui-v1`。

## 2026-10-03：Claude npm 实际会话忙碌延期

新增关闭 0 项，保持 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 草稿。精确 `6726e735994568e0f2b0995a068d6c5bb32924f1` 构建 `r-_wghh92i` 的签名 GUI `r-2pmgk97b` 中，真实 Claude 2.1.285 会话活跃时只点击一次空闲更新；界面等待会话结束，独立观察确认完整旧树身份及摘要不变、无 journal/stage/原生代次。Ctrl-C/Ctrl-D 没有使原 CLI 退出，随后通过应用 Close pane 确认关闭测试会话；不声称旧 CLI 自然退出 0。其精确 PID 消失后，应用自动完成 285→287，未第二次点击更新或检查。

唯一代次 `88638e0b-6bba-49df-a8b4-53009e27fafc` 的 manifest/launch/exit 摘要及原生身份匹配，候选原生退出 0、launchd job 和 coalition 清理确认；新 11 文件／4 目录完整树匹配固定官方合同，journal/stage 删除，设置页 Installed/Target 均为287，公开入口实际输出 `2.1.287 (Claude Code)`。独审 `65061910e72fdadcff2a9da340fb3eb1442f2ffba387edec039cb21ce30e3ba4`，最终61工件索引 `4b1472296b8e985c528aa878f39a448c3e61c875e649bb8418f7d1f581115dd7`，位于仓外 `g09-npm-busy-gui-v1/r-2pmgk97b-en/independent-observation-v1` 及同轮 closeout。GUI/已知后代/observer均退出，短根及精确profile/state/Preferences已核验清理，真实npm安装保留。无需本地化变更；本轮英文等待/自动更新/成功状态完整，既有英中语义及布局审计按原构建保留，不称本轮重新验中文。Homebrew 合法跨渠道降级与应用中断恢复、Windows 原生 npm 及最终源码门禁仍待完成。

## 2026-10-03：Claude 官方 npm 硬链接布局修复、真实升级降级与应用冷回滚

新增关闭 0 项，保持 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 草稿。真实官方 npm postinstall 的两名硬链接已按固定平台／版本、完整 SHA 与长度、同 inode／owner／mode、恰好 nlink2 支持；普通快照、Codex／Grok及新候选仍要求 nlink1。旧 journal 格式不变，恢复由旧版本推导合同，已持有两名的清理支持 2→1 及中断后的未变子集；第三链接、外部链接、名称替换或改写仍拒绝。

源码冻结 `g09-claude-hardlink-build-v2/source.safe.json` SHA `c7af0704da084a7fc1fa20da509941593d225cf770015b0b2b6133d564f4f847`，基线 `4e4758dcc` 加明确 diff；构建 `r-0p2zouxy` raw SHA `77e3629f40abc6fa46d97de0f00a99afa3bf1783a4099878cdf8b08096907927`，签名 GUI `r-w3vkj6p2`。未重装或改写原 npm 安装，真实 GUI 287→285 Stable、285→287 Latest 均成功，公共命令分别实际输出对应版本；独立完整 11 文件／4 目录、配置、入口及原生退出/清理核验通过。降级索引 `658b912caa296d130a56f70876b843e3666f7defd1ce766807e085f8d5dd8919`，自然升级 `8be967cbdfe5fbe887f57ba7ffb01dce0b8b023dcda3a77baaadeee470f8b0ab`。

随后再次合法降级，在真实 SwapIntent 且两侧目录身份已交换、候选版本探针退出后，只向本轮原 GUI PID 13884 发一次 SIGKILL。独立现场审计确认公开 285 树等于 prepared、备份 287 树等于 original、配置尚未发布、全部原生收据闭合，SHA `bd09b3012ed022ae4aaaf5f5394e817cc0d43e18007a52b8e287a34dbfdb9c18`。同签名映像、原 PATH/TMPDIR/profile/state 正常启动 PID22561，未点击检查或更新，应用自动将全部原 287 树的身份、大小、摘要及原配置逐字节还原，清理 journal/stage；恢复树 `6e92019b416badfd4387636ce0a1a58637b700b10da60dfdaeb853b31e028a00`，新终端实际输出 287。通过 GUI 选择中文并正常重启 PID23315，失败提示完整保留；单次主动更新成功 285，failed 标记、journal/stage 清除，原生退出0及身份清理闭合，独审 `25d68cf5b3a6a1b20ba498fc99469cf866db383c13a569bf7ec5eab20ff0df73`。此为真实应用交换后冷回滚，不借用后台 worker 测试替代。

无需本地化变更：原英中版本、来源、渠道、两插件、失败及重试状态文案仍适用；英文与简体中文实际布局完整无截断，双语收据 `b3586d8ee2f5c83bd17fdf993e76cb55497c6048a0e5c29334abdfd1b39f0754`，中文重试成功 `a8e9ce7572fbd0fb340f4cb11d7093d3523a2438d08f7346635f4d2d3ba7c4d5`。CUA截图只在对话原始工具输出，仓外 JSON 不冒称截图。原4e安装拒绝保留；中断辅助v1在kqueue的with语法预检TypeError、v2错误比较源文件与隔离快照执行路径，均零信号，不能算中断成功；v3才取得上述实际中断。第一代GUI被计划中断，外层exit247原样保留；后两代正常exit0。四个原生generation、全部本轮进程/launchd/打开文件核空后，小证据归档，短根/profile/state/精确Preferences已清理；两个真实安装前缀继续按登记保留。统一索引 `609f00328a0a2b225a466ece64d899d580e622e87cef24133f3702b6e1a4f023`，清理收据 `b3292c9241b8743a26352b0ad88a169ecffd7d763c7ab0b957d40ae347e5077f`。

本机 `r-hk4cx3lm` check与 `r-73t5rrjc` i18n11项通过；`r-f0ii2606` 定向333项为332普通PASS＋1 LEAK，8354未选，17条新增回归均普通PASS。LEAK为 `homebrew_ancestor_exception_rejects_generic_native_authority`，未定位残留输出句柄/后代，不称全体普通PASS；本轮进程、launchd与打开文件核验后短目录已清。门禁收据 `a87e21654adc4844f81ef6af8444b65a8c04547950075f58148fa4512e9e4d3b`。同签名应用的已安装Grok通知真实测试23PASS／1fish跳过、独立诊断10PASS；PTY测试读取改为有界等待完整OSC，7项读取合同测试通过，不放宽产品时限。该变化不能仅凭Mac通过认定Linux旧失败根因已彻底解决；新精确提交两平台门禁仍待完成。G09其余消费者来源、真实Homebrew中断恢复和Windows原生npm失败仍开放。

旧提交 `4e4758dcc` 的 [37031319203](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37031319203) 已终态 failure：Linux 5389普通PASS（新增34均普通PASS），唯一实际FAIL为已安装Grok通知测试；Windows作业success，5007普通PASS＋1 LEAK（新增18均普通PASS），LEAK为 `test_ai_context_menu_keeps_workflow_reference_in_ai_input`，不称已解决。i18n两端各11个唯一用例，重复门禁的22次通过不加倍计数。13份官方ZIP的摘要、大小及50条目CRC均核验，终审 `816cb9769ceeb8212104ba94ffdc4346445906514a71102d58e0619186b8beed`；新修复不回填旧轮。

Windows实际事件诊断另补真实generation、派生及API调用线程、接收／验证／继续计数和耗时，失败在清理前保存最近16个固定字段事件，Wait超时仅累计；非零原生退出仍为失败，不输出路径／环境／原错误全文，不改权限、超时或控制流。新增3回归和既有Continue失败断言增强待Windows执行。最终Mac check `r-yuiu1xs8`通过，日志 `c8c11fff895a449f5b3143a07c029122b7cde5b595457e1b6d72d26ba315579f`；单次本机Windows应用交叉check `r-ysabgecz` 在aws-lc-sys C依赖缺 `windows.h` 失败，日志 `7cfa677c4c3fcd71e3ce2f6e8d8fedccff967d0cd1e693523944bcbc7050ba0b`，未达到应用类型检查，不再重复本机调用或冒充通过。两短目录均按身份清理；新Windows类型、回归及同真实npm事务由最终提交的既有工作流核验。该诊断无需本地化变更，本身不关闭G09。

## 2026-10-03：Codex 真实 Homebrew GUI 链通过，Claude npm 原生布局缺口已复现

新增关闭 0 项，仍为 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 草稿。精确 `4e4758dcc` 构建 `r-vtn4rx14`，raw binary SHA `963e4a9f1ab0187b7967c45de8eb0bf7989111fee200adfcaf37774ce1523ecf`，对应真实 GUI `r-2l5quutv`。选定真实 Homebrew 7.0.4 私有前缀中已安装的 Codex 0.156.1，界面查询到当时 Latest 0.160.0。实际启动旧版原生 TUI 并单击空闲更新：忙碌提示期间原公共链接、映像及版本目录保持不变；正常 Ctrl-D 退出后无需二次点击，应用自动更新。实际公开入口 `--version` 输出 `codex-cli 0.160.0`，完整新包 42 文件／10 目录的大小、模式、摘要和正式 tab、三种补全匹配；7 个原生探针退出 0，身份／清理收据闭合，journal／staging 已移除。独立后置审计 SHA `9dde348b1dddb2303a9de536b5fea1ea0169d59d183f4932cb988790c571a05c`；观察开始于发布之后，不声称完整中间 journal 序列或额外内核 CID 查询。

原登记 PATH/TMPDIR 的新进程重启正确显示已安装／目标均 0.160.0、Homebrew／Latest、渠道匹配及两个独立插件缺失提示；英文与简体中文实窗完整可读，既有文案无需本地化变更。重启审计 `0c00d8e8a11c6083124c74c6f6e7d948d4803cc3eaf1a732101f717b929674af`，中文 `4211221b451000ab07a0a765dcbad380a24a2bd9779a163b0a08e636a3a74844`。中间一次工具自动重启没有保持 PATH/TMPDIR、界面显示未知安装，独立负例 `1627f08cb335d70bfecfa7973ba77e7eb68477bd7c51ca57f2fecdb8287c2d92` 保留；后续原环境才通过。完成后正常重启不等于中断事务恢复。四代 GUI 及 54 个已知进程退出，root／profile／state／精确 Preferences 已清；仅 7 个本轮已归档 0500 快照目录在退出核验后通过原 fd 加 owner 写位以完成即时删除，原失败保留，不改安装权限。最终索引 `fc597fa306bb8012a07c13e153405a7dc0f8d225d9a811c32a4fe7e7c8bc8290`，清理 `68e2de778dff537240fbcc9c94c37761a75b330e234e7646f3520630f0ea943b`；真实 brew 前缀按登记保留。

另使用官方 Node 22.14.0／npm 10.9.2，在 `r-u6fdf2y_` 私有前缀真实安装 Claude 2.1.287，官方 postinstall 未改写。实际公开入口版本正确，但两个官方名称 `bin/claude.exe` 和包内平台文件为同一 inode、nlink=2、SHA `6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea`；现有 Unix 快照及清理只接受 nlink=1，确定缺少真实安装布局支持。GUI `r-3zes6cit` 正确识别 npm／2.1.287，用户入口选择当时真实 Stable 2.1.285 并实际单击更新后，被拒为不可安全更新；原始树完整、没有 journal 或原生候选。失败收据 `8e9519938f796ef166165adc16d5276a95ef39f6e6e0f48bc34f51c6521d568b`；不将界面拒绝单独认定为唯一失败点。GUI 已退出清理，npm 安装保留供修复后直接复验；其外层安装和准备命令均退出 0、短 TMP 已清。真实合法降级仍未通过。

Windows 只读审计 `ee8550643a86df8942c0c9b59219d6bde90a885ef05df2a6b7150ffe7dbdc1a5` 核对固定 Node／Codex 上游及微软合同：`lpDesktop=NULL` 的父站继承说明构成反证，不能由根 HANDLE_LIST 只有 stdio 推导后代必回旧站；e9 正常阶段尚未收到／放行 Node CREATE，原生阻塞事件仍不明。未加猜测继承、ACL 或 timeout 补丁，未重复近似原生探针。解除条件是同一真实链的完整有界调试事件阶段和原生错误证据，随后修复实际阻塞并通过 CMD／PowerShell 全事务。精确 4e 的 [37031319203](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37031319203) 两端编译已过，但 Linux 已安装 Grok hook 步骤失败；完整终态审计待补，不能以部分步骤绿计最终源码门禁。所有证据位于 `resume-20261001/g08-build-preparation` 对应版本目录，CUA 原截图在对话工具收据中，仓外 JSON 为观察记录。

## 2026-10-03：G09 当前消费合同与真实私有 Homebrew 支持，验收待完成

新增关闭 0 项，仍为 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 草稿。Mac ARM64 的 Codex 0.160.0、Claude 2.1.285/2.1.287、Grok 1.0.46 已按官方npm/cask完整树接线；Claude显式Stable的2.1.287→2.1.285使用独立意图，旧版本、旧降级意图、journal/schema及未知版本拒绝保留。Codex恢复额外将完整探针闭包绑定到账本目标版本，156/160双向误标均拒绝；更新合同不扩大托管运行或模型白名单。

真实 Homebrew 7.0.4 在私有前缀 `/Users/zhishi/InfiniShell-Tests/r-mbqmnwc7/brew` 安装Codex0.156.1，真实tab、42文件/10目录及三份补全已独立核验，收据SHA `8bfd73d3d9254cb980c91abf42ef4fd3ae8a7345438340c76655a73a49b80f2e`。首次安装缺少两份补全的旧失败保留；只创建私有补全父目录后由官方brew重装，没有手写tab或修改现有安装权限。安装保留供GUI验收，cleanup_ready=false。

真实Caskroom由Homebrew创建为0775，暴露事务与原生快照祖先两处拒绝。现仅Mac ARM64当前用户0700私有前缀中的固定Caskroom可接纳该模式；同UID、同设备、无ACL、原句柄和路径身份前后复核。普通包目录/文件、G01及快照状态目录保持严格检查；其他可写祖先、公开前缀、链接、权限或身份变化继续拒绝。该修复尚未通过真实GUI更新链，不将单元正例计为更新成功。

冻结基线e9d93e133的58文件索引 `g09-consumer-contract-v3/source.safe.json` SHA `46f8926b012d4ee7c4846aa9f067d6732a8870c6ce025e1685166bcd5388b8a6`；本机check `r-8foebn90`、i18n11项 `r-fw5u2ksf`、定向316项 `r-emt25uhf` 均通过，8354项未选，三个短目录均核验并清理。定向范围包括更新/恢复、版本探针、npm worker及Mac原生快照；9项新增快照例外回归均通过。汇总SHA `1e3a9cca0f12275000b9fdcf5a63d0912b2a21832a64861b5f74873bb3dbf9b2`。早期v1测试编译E0308及后续子集结果保留，不回填成本轮结果。

无需本地化变更：复核既有英中版本、渠道、忙碌、更新、错误及插件复检文案，语义无新增，i18n门禁通过。精确提交Linux/Windows源码门禁和Mac真实消费者更新、忙碌预约、合法降级、应用重启恢复仍待完成；Windows生产窗口站通过但npm首候选失败按下一节单列，不关闭G09。

## 2026-10-02：G09 生产窗口站通过，真实 npm 首候选仍失败

新增关闭 0 项，仍为 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 草稿。精确 `e9d93e133` 的 [Windows 37018916543](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37018916543) 已终态失败：编译、1043项 lifecycle 与43项 command 普通PASS，含7项bootstrap、i18n11项及updates135项；nextest无FAIL/LEAK/RETRY/FLAKY，后续真实npm ignored libtest为1项FAIL，不能合称门禁成功。

首个 updated 场景的CMD候选返回 ProbeFailed、execute exit101且stdout为空，PowerShell及其他四场景未执行。生产helper两段均ready并实际exit0，新窗口站匹配；桌面关闭、Job空、站与LUID消失及AppContainer profile清理确认。`drain_before`之后在WaitForDebugEvent循环停滞约292秒，取消清理时才取得此前已创建Node的CREATE事件。该循环不是stdout ReadFile；原件不足以认定唯一根因或ACL拒绝，不以改超时或相近探针补计成功。

三份官方ZIP按API摘要、大小及CRC核验，19份追踪源码均匹配e9 Git blob，helper映像一致。终态审计 `g08-build-preparation/e9d93e133-ci/terminal-audit.safe.json` SHA `8537ee1b346579a29822d340c3e1afec1bb40cb7a51a98ce07efa2f0c255fb44`，原件索引 `c1031703d77c33f5207434fa52dfcc5464ccbfc6d82a122254a7a2dff2cbbeae`，完整日志 `0ae7da51290fe01da055b461da6597ee2a203f7c98738c9f333ee549d7aa308b`。历史pending与失败保留；此结果不覆盖后续Mac消费合同、不证明普通非提升交互用户，也不关闭G09。仅补记审计，无需本地化变更。

## 2026-10-02：G09 Windows 生产引导器接线，真实 npm 待验

新增关闭 0 项，仍为 **9 关闭／G03、G09 共 2 开放／3 移交**，PR #22 保持草稿。Windows 版本探针已接随 GUI/TUI 打包的 `infinishell-station-bootstrap.exe`，只取已核监督程序同目录文件，文件身份和摘要进入事务并持租约。两段新 LUID helper 的挂起原句柄、创建时间、用户/Session/完整性、精确 Job 及本地命名管道 peer 均绑定；第二段仅从保留的第一段进程复制原句柄。磁盘 JSON 只作证据，不作为授权或恢复入口。原业务进程 AuthId、zero-cap AppContainer 和调试映像约束不变。

新站必须与新 LUID 预测匹配、非交互且在创建前不存在；派生 AppContainer SID 并核对新站/桌面的 owner、DACL、Low MIC。关闭桌面前恢复本新站原线程桌面。业务 Job 退出后才关闭两段 helper 和 Job，并证明站/LSA 会话消失，再回收原 ACL/profile；关闭失败的强制回收写独立失败收据，不把失败更新变为成功。GUI/TUI 安装器均包含 helper，旧版无 helper 的 TUI 仍可保留为 previous；安装器实际运行尚未验收。

20 个产品、测试和打包文件绑定基线 `798f6c6cb585e39662e18129fbefebd95e9fa5b7`，冻结源码索引 `g08-build-preparation/g09-station-production-v1/source.safe.json` SHA `a7fd229cccfd495eb4eae958094a3ff916a6efdc4604b4c44632cee2d8702e9e`。Windows 全目标类型检查 `r-d_ga6u2p`、Mac 应用 check `r-mabw7vwx`、Python 15项 `r-unepdqf3`、PowerShell 解析 `r-3h51qmn1` 和 actionlint `r-8qb25hra` 通过，i18n `r-bc7zzsfn` 11项通过；短TMPDIR全部归档清理。本地门禁索引 `11f6d7e005ec5b74a46a2dbb0a77566aa109efa928aa03644c5b0ecde697d714`。首次 Windows E0658 失败保留，已用稳定文件句柄 API 读取硬链接数。无需本地化变更：复核既有 SourceChanged／ProbeFailed／RecoveryRequired 英中消息，无新用户文案或布局变化。Windows 类型检查不能证明运行时 Node/npm；下一步仅跑生产双入口和五场景事务，不再重跑窗口站诊断候选。Linux 未选：本增量原生实现与安装器仅影响 Windows；Mac 编译覆盖其他平台条件分支。

前一插件提交 `798f6c6cb` 的[双平台门禁 37012577482](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37012577482)已完成 success。20项净新增和1项改名增强回归两端均普通PASS，i18n各11项；Linux nextest 1932普通+6显式ignored PASS，Windows1612+5，两端rust-genai各81。无FAIL/LEAK/RETRY/FLAKY/TIMEOUT，warning分别73/95行保留。完整日志及13份官方ZIP/50份原件摘要、大小、CRC已独立核验，终态审计 `4f231826362e1cf15f9d6818a9a910490ed256478a43ae2b2ef63b67ab579724`，原件索引 `6500b50310f8bee11d08770c2225462216cd492dda3dae31c0af8c1a44990902`。此门禁不覆盖本轮新引导器，也不替代GUI或全工作区。原pending与失败按历史保留。

## 2026-10-02：G09 插件完整性复检与窗口站前置能力，未关闭

本轮关闭 0 项，累计 9 关闭／2 开放／3 移交，PR #22 继续草稿。通知与编排插件分别返回缺失、禁用、待更新、文件不符、未验证或已验证；检查及更新成功后均重读完整文件树，绕过界面缓存。Grok 桥使用本次实际入口与版本，Claude 编排插件因缺完整内容合同继续显示未验证。复检期间保留更新操作和启动保护；旧回调不发布结果，插件问题不覆盖已经成功的 CLI 更新，也不开放未知版本托管能力。

Mac `r-r5la1r7o` check、`r-kcacn0wa` i18n 11项、`r-qr0x1wtu` 定向484项及 `r-5afjdbmf` GUI构建全部通过。首次 `r-7m7w3ptb` 为483过/1失败：新复检遗漏既有 Codex rev3–rev5 精确完整树合同，现复用该合同，旧树待更新、篡改仍不符；原失败保留。构建对应 `04a874d7a` 加16文件实际差异，diff SHA `fcddec5943660417ce915789ff599ac4600d8ea4ab50e07fe17e504897da5aa5`，raw binary SHA `1fe706e0d61c14e6245141cfa1af96842ef96cc8e2e75f3eef8da1970410ff63`。

英文 `r-77mzjt2i` 与中文 `r-kgi_eahg` 独立实窗均关闭自动更新，保持 HOME/CODEX_HOME，GROK_HOME 为无认证私有目录。实际设置页显示两类缺失提示；英文由真实检查按钮重读禁用和损坏夹具，中文由损坏文件恢复后显示未知 Grok 1.0.46 的未验证状态。两个语言的2560×1600布局无截断、重叠，文本及检查按钮分列；窄窗拖动未改变尺寸，不计该项。私有树前后摘要一致，无安装、更新或模型请求。全部GUI和已知后代退出、launchd/lsof及目录身份核对后，包、私有夹具、小证据归档并清理短目录和独立profile。CUA原始截图留在本对话，仓外收据为观察记录，不冒充截图文件。总审计 `g08-build-preparation/g09-plugin-recheck-audit-v1.safe.json`，SHA `3aa29a7f0c6bc80206eb1e024fc615ea81eb679ca18e4dd4093d49ce8fb19dec`。英中文案及变量同步，更新后的跨平台源码门禁待完成；这些结果只证明插件复检增量，不是完整G09更新验收。

[Windows 37008880591](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37008880591) 精确绑定 `04a874d7acb63d95ebfb3cff2ca86e19ec1953aa`，591普通PASS及1显式ignored原生候选PASS，无FAIL/LEAK/RETRY/FLAKY/TIMEOUT，37行warning保留。三份官方ZIP、完整日志和六份binary收据独立核对。第一段仍处原站，第二段实际新站与挂起原句柄AuthenticationId预测一致，恢复前及退出后同名查询明确不存在，新LUID退出后消失；精确Job和所有自有句柄清理确认。调用者为有效Admins=false但已提升、高完整性Session0服务身份，不能称普通非提升交互用户。原件索引 `e8039d475e9a2e73cbaddd8a49d2fe265f90f74212d5cdda9cda355db88d6534`，独立审计 `615dbcad2d25f85eca3a1b7078d8d024ddc72844656dcb255af07cff40af8774`。此前C4701和全局枚举前置失败均保留；该成功尚未证明生产完整宿主、新站AppContainer授权或Node/npm链，下一步接专属bootstrap与本轮新站生命周期，保持普通lease身份及零capability隔离条件。

## 2026-10-02：G09 窗口站候选的精确不存在证明，待原生执行

新增关闭 0 项。`596d439c13bb00f1b25a0945f24d9b995cfd5360` 的 [Windows 37007022817](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37007022817) 已修复 MSVC 编译，但测试在全局窗口站枚举的前置检查停止：返回失败并记录 hresult 0，没有 `first_created` 或任何六份原生收据，不能称 CreateProcessWithLogonW 失败。调用者对象不变，精确 Job 已空且关闭确认。完整日志及三份官方工件均保留于 `g08-build-preparation/596d439c1-ci`，索引 `ee0526ea72a38c335b66af523b10be0199f4777193294258ea89598659f81b91`。

[Microsoft 枚举合同](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumwindowstationsa)只覆盖拥有 WINSTA_ENUMERATE 权限的站，因此旧列表不能提供全局不存在证明，也不据这次 hresult 0 推断唯一失败原因。新测试在第一段原创建句柄、映像、同会话和精确 Job 已核后、Resume 前，依据新 AuthenticationId 预测 `Service-0x高-低$`；对单一名字的 OpenWindowStationW 只接受 ERROR_FILE_NOT_FOUND。第二段实际名字必须匹配这一预测，退出查询必须仍使用同名。[官方创建说明](https://learn.microsoft.com/en-us/windows/win32/winstation/window-station-and-desktop-creation)的 logon session／logon SID 术语不被视作二者恒等保证；预测不匹配仍失败，不回退旧站或修改 ACL。第一段新 LUID、双有效非管理员、原句柄、挂起身份、非继承／不可见和有界清理条件全部保留。

此次只修测试证明方式；并非生产能力已修或完整 G09 通过。此前两轮原件、失败及 Mac 消费者待验条件不变；新源码尚无目标平台原生结果。无需本地化变更。

## 2026-10-02：G09 完整条件复核与新窗口站候选，未关闭

**本轮新增关闭 0 项，仍为 9 关闭／2 开放／3 移交，PR #22 保持草稿。G09 不只差 Windows。** 按原关闭条件复核，Mac 三款 npm 的固定后端事务不能替代真实来源／渠道、实际忙碌延期、合法降级、GUI 更新及插件复检、应用重启后的恢复入口。Claude npm 历史五场景包括降级拒绝，不能计为合法降级；本轮未找到其旧 Documents 索引位置，不宣称重新核验原件通过。Codex npm run-03／04、Grok npm 私有 APFS run-03 的历史范围保留，不改为同一当前提交整体验收。

本轮重新读取三份既有独立审查及其明确排除项：Codex run-04 `5a638607ad57175e6a9296a51175d9644e8a76113345f1927b68a3fb1ce49d03`，Grok run-03 `b98361dcee870bc76685cf7dc18917085eb968d9a67925ae8252847f7348a771`，Claude cask run-01 `a735d548a6e43775c70d68f80502dc185c6eb73a92cc1c5476783b8021d7fb5a`。Claude cask 明确为人工登记／派生元数据的后端四场景，不是真实 Homebrew 来源发现；Codex/Grok Homebrew 实际升级、失败回滚和冷恢复仍未见通过记录。只重核审查原件，没有重新执行或重新核对全部底层原件。

2026-10-02 11:59 UTC 直接读取 npm 官方端点，Claude [Stable](https://registry.npmjs.org/@anthropic-ai/claude-code/stable) 为 `2.1.285`、[Latest](https://registry.npmjs.org/@anthropic-ai/claude-code/latest) 为 `2.1.287`；HTTP 200、原 URL、原始 JSON 与摘要均已保留。现有包更新合同只审核升级 `2.1.280` 和受限降级 `2.1.278`，本轮没有增加版本白名单、放宽布局或用固定测试指针替代生产实时指针。此观察只证明当时的公开渠道，不执行安装、升级或模型请求。

另已提交测试候选 `1ac53f4dabc0599d8940883cfd5607649f89bab2`：以无真实凭据、无网络调用的固定 C helper 验证新 LSA 会话与第二段不继承句柄的窗口站选择。保留双有效 Admins=false、原创建句柄、映像身份、挂起恢复前精确 Job 核验、失败清理及原始错误。现有 runner 为非管理员服务身份，提升／完整性独立记录，不能称普通交互用户通过。即使候选成功，新站 AppContainer SID 的 DACL／low label、正式 worker 初始化、zero-cap Node/npm 仍需证明；生产 `WindowsProcessLease` 的 AuthId 合同不变。

本地 `r-g5fk3c2f` 的 Windows 测试目标类型检查、`r-tpe_rto3` 的应用 cargo check、`r-i4z18k2d` 的 actionlint（仅忽略已知自托管标签提示）和 `r-23nlq1nx` 的 PowerShell 解析通过，短目录全部完成身份核验并清理。首次 `r-o9nfut9f` 的 Windows Error API 类型错误原件保留，改用已核依赖的 `from_thread` 后才通过；应用既有 5 条编译警告保留。本轮没有产品用户功能或界面文案变化，**无需本地化变更**。唯一新范围的 [Windows 37001996114](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/37001996114) 已结束 failure，准确源码为 `1ac53f4dabc0599d8940883cfd5607649f89bab2`。失败发生在 MSVC 构建 helper：`TOKEN_ELEVATION elevation` 报 C4701（可能未初始化），`/WX` 将其作为错误；尚未链接 helper 或调用原生测试，不能推断窗口站候选失败。完整 Windows 日志及三份官方工件原件归档于 `g08-build-preparation/1ac53f4da-ci`，索引 SHA-256 `9fd5a56d49b1ac5e3dc56825c4f3577f022ef73d7f0aa3dc7ec6c369ad8ee8c3`。修复仅显式零初始化这一结构，成功分支仍要求 GetTokenInformation 返回成功和准确长度，不改变权限条件。修复提交前 `r-b1ux436a` 的 Mac Rust cargo check 通过（5 条既有警告），日志 `9b40e1868db51bbe17ddd721349ad75a0df22aa1d2ffd0a1f8e8106d02e81d2c`，短目录已清理；该检查不编译 MSVC C helper。新源码的目标复验待完成；不重复旧 NUL/Node/未命名站矩阵。

完整复核索引位于 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20261001/g08-build-preparation/g09-scope-audit-v1.safe.json`，SHA-256 `fe2aa3f9623d217e39956c5d8b4c9da1173a363496e876f6d401605dd10f9980`；公开渠道原件索引 `c4306be2c543776688c5e0f7bbaaea204834baea915415c571c6a80054932d6e`，本地门禁索引 `20e4533fd6f8ef6b964db62c03da0771b3baff60e908b31887ad4294862e273b`，7 个提交 blob 与验证字节的绑定 `3251d6ec458309b6cbe428a2e948dd1e057db5d62395cd773dbf9216129886e0`。原关闭条件及旧失败不变。本次三份说明提交前 `r-qjgteqic` 的应用 cargo check 通过，日志 `b4c6e578d1304848bcc4fdf0a3398e6993f8abf0f43b71040505d83253306d9b`，5 条既有警告保留，短目录已清理。

## 2026-10-02：G10 按本次 Mac 范围关闭，Windows LEAK 独立保留

**本轮关闭 G10；当前 9 关闭／G03、G09 共 2 开放／V01、V02、V05 共 3 移交。PR #22 保持草稿。** 原关闭条件的真实父权限、命令／技能允许和拒绝、越界拒绝、运行取消及资源清理、双向 ACK、冷恢复与结果均已满足，前节所列实际宿主审批英中布局完整可读。各轮证据继续绑定原构建，Mac 关键收据汇总 `c8760542659c246919ac44f21b2dfd76550d4e31fdf1bbd3e6387a6388f55f06`；没有重跑模型或扩大模型／策略排列验收。

精确产品源码 `04ac0d46f847b7c9246fbf9193f4ac2c76f6b9af` 的[最终源码门禁 36985767809](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36985767809)已 completed/success，Linux／Windows 两作业均成功。完整双日志及逐项清单已核：Linux nextest 5335 次普通 PASS；Windows 4962 次普通 PASS，另 1 次 LEAK。两条新增 Grok 回归 `fixed_search_policy_accepts_native_grep_and_preserves_its_scope`、`native_permission_cancelled_waits_for_matching_history_and_never_reports_completed` 在两平台均普通 PASS，分别见 Linux 原日志 6556／6447 行、Windows 7272／7159 行。无 FAIL／RETRY／FLAKY 状态。两端 rust-genai 各 81 项、TAP 各 18 项；Python 共 57／50 套、1313／1087 次执行，含 12／36 次显式跳过；编译警告 125／180 行。计数为执行次数，不是去重总数。Windows sccache 未运行服务连接拒绝 10061 等原日志诊断保留，不称全日志无异常。

Windows 原日志 12984 行是 `terminal::input::tests::test_ai_context_menu_preserves_lock_state` 的 LEAK，13072 行原汇总为 `670 passed (1 leaky), 8402 skipped`。该用例检查菜单开关保持 Shell 模式与锁状态，自 Goal 起点没有修改；历史同名已检索记录为 PASS，不能称同名旧泄漏或确定间接关联不存在。日志没有残留进程／输出句柄身份，根因及最终清理未证实。只读源码审查发现假终端 bootstrap 仍启动生产 metadata 查询，部分为 detached；Windows 进程组清理为空，但查询显式 stdout/stderr=piped，尚无本轮捕获句柄因果证据。独立保留这一问题，后续应取得精确子 PID、spawn/wait/drop 与句柄归属，不用延时、放宽门禁或重复跑绿替代。仓库及 G10 原规则没有全仓零 LEAK 的额外关闭条件；本次如实记录门禁 success 和风险，不把该条计为普通 PASS。

根代理终态索引 `9335eca2537bfe5dd55a9c0056c732ad5a569a34675cc501eb9de5d4a01a52b0` 位于 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20261001/g08-build-preparation/04ac0d46f-ci/terminal-log-audit-v1/terminal-audit-root-v1.safe.json`，引用冻结完整日志、清单和负面诊断；Linux 日志 `6ee69ad88e56cd3fc9517c65f9c48cd79d33f7365f88773b0efe91d5377afda8`，Windows 日志 `210031164cb96c4b342bb8430c978b4247c68d3725d2422074188ab19796dd0b`。Windows 两条逐名收据 `88f46ccc66c795bbdb316e076a6c7e3fb1f890caeac408cfc09f1aee71942f3b`。此前 pending、失败和 cleanup_ready=false 现场不改写；资源回收证据不等同目录删除。

本次只改三份验收文档，产品源码与 04ac 一致；提交前 `r-m_f4050a` 的 `cargo check --locked -p warp --features warpui/test-util,rust-embed/debug-embed` 通过，日志 `ebdea4fd9a721a7547ef8b9c7a6fccf91f58e6ed4445359da4d9a41159a39534`，短目录已按身份清理。**无需本地化变更**；既有英文／简体中文语义与实际受影响布局沿用已验收据，不称完整中文冷启动或其他平台实机通过。G03 仍缺 Finder 原生拖放证据；G09 仍缺普通用户原生私有交互环境及真实 npm 事务，不能以管理员 Node 正例替代。

## 2026-10-02：G10 Mac 真实功能和英中审批齐备，最终源码门禁运行中

**新增关闭 0 项，仍为 8 关闭／3 开放／3 移交，PR #22 保持草稿。** `04ac0d46f847b7c9246fbf9193f4ac2c76f6b9af` 的修复后真实功能和英中审计已满足，G10 只剩 [最终 Linux／Windows 源码门禁 36985767809](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36985767809)及逐名审计。该轮原生补验、Grok host 窄筛均关闭，普通筛选包含 `test(cli_agent)`，覆盖 Grok profile；不能用此前 4a 门禁替代新修复。Mac 九份关键收据重新核对摘要，汇总 `c8760542659c246919ac44f21b2dfd76550d4e31fdf1bbd3e6387a6388f55f06`，路径见 CURRENT_STATUS.json 的 `g10_macos_real_acceptance_20261002`。

Grok `r-0aa9cs1x` 使用官方 `1.0.41/grok-4.7`，67 条安全事件核对真实父子 Skill、Grep、命令拒绝后新请求允许、命令退出与完整结果、父自动接收子结果及未授权 beta 子任务拒绝。父 3 次和子 1 次真实输入、原生会话、回合及持久代次逐项匹配；生产 Rust 场景和清理均成功。原外层 exit 2 保留：旧审计器要求 user_message_chunk 带 promptId，而公开原生合同没有该字段。新离线审计按输入片段的唯一终态关联，继续要求所有显式回合 ID 一致，并核对完整输入、promptIndex、ACK 和持久身份；原有 167 项 require 全部保留，增加 3 项。只对同轮原件重新对账，无模型重跑。离线审计摘要 `a14ad8ad920ab1d33b5d3e3da023f52e0b0dcc36ff96cc5317f6323e23768cc7`，原生链 `44a43227058cd60701b33bd385f2e4917d5d65b59f3ecf9a0aa5573fe0a378ce`，独立清理 `625c41b4b13d7db23a47292736b4f90ef1b4b5d98d0202455469e3afd637fe8a`。

真实 GUI `r-1u9ov31_` 只向固定 Claude `2.1.280/claude-opus-5-5` 提交一次。原生工具与宿主命令分别一次 AllowOnce，同一宿主待审批任务先在英文、再在切换语言后的同进程新窗口审计中文；两行说明、实际 executable/argv/cwd/timeout、允许和拒绝按钮均完整。模型将 npm-cli.js 请求规范化为已允许的 Node＋argv，审批确认并执行的是展示的实际形式，不称字节相同的原请求。允许后实际 `npm run verify` 退出 0，随机输出、原生工具结果、最终回答、保存状态一致；唯一输入及 ACK、2 次允许、1 次 completed 和完整事件链均由独立审计确认。原生与命令两个资源域清理确认，29 个已知进程退出，4 处文件占用检查为空，GUI 外层退出 0。最终审计 `8b1d281f791b4ad81dbfcbaea0b373082207ea6c23ae5220917c481cff06de76`；英中现场观察 `ab96d34e30e7e0c5b2f8ce6e6dcbbaf3a614f440e22def698f0876fd63909eee`。截图只在 CUA 工具回执，不虚构独立 PNG。新轮测试私有 PATH 仅增加可信 `/usr/bin/security`，没有复制或修改默认认证；原未登录失败保留。

原 G10 关闭条件逐项复核：工具范围及父权限上限由固定策略和真实越界链覆盖；命令／技能允许与拒绝由此前 Claude 及本轮 Grok 链覆盖；运行中取消和清理由 `4a/r-agu5bntu` 独立正例覆盖，父后续未确认回合不计完成；双向 ACK、结果回传及冷恢复按此前 Claude/Grok 原构建真实证据和本轮结果核对，不扩大到所有策略排列。旧 Grok V8 的 dirty snapshot、`native_effective_policy_verified=false` 和无 GUI 边界保留，本轮固定权限证据另立。**无需本地化变更**：本次合同修复恢复既有承诺，既有英文和简体中文策略、审批、取消及结果语义已复核，相关静态与实际审批布局完整可读。完整中文冷启动和其他平台实机不在本轮通过声明内。

真实短目录、私有 profile 与唯一原件保留，cleanup_ready=false；原生资源清理通过不等于目录已删除。G03 仍缺 Finder 真拖放验收；G09 仍缺符合普通用户产品约束的 Windows 私有交互环境，既有管理员 Node 正例不能替代。最终源码门禁结果待补记，当前不关闭 G10。

此前 [4a 源码门禁 36977301774](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36977301774)完整日志已独立核验：Linux／Windows nextest 分别 1917／2267 次普通 PASS，四项受审命令取消回归在两平台逐名通过；次数不是去重测试总数。两端 rust-genai 各 81 项、TAP 各 18 项通过，Python 分别 57／50 套共 1313／1087 次执行，含 12／36 次显式跳过。无 FAIL／RETRY／FLAKY／LEAK 状态。Windows 清理未运行的 sccache 服务时连接拒绝 10061、编译警告、Linux 缺 fish/zsh 和 Windows 符号链接条件等均按原日志保留，不称全日志无异常。审计摘要 `a26e72ff784f2ee205063e613beee2c7c2e3a8eaaf9c7c179e3dedf7645bac91`，只证明 4a 原源码；不替代正在运行的 04ac 最终门禁。

本次仅三份验收文档变化，提交前 `r-59_hi84s` 的 `cargo check --locked -p warp --features warpui/test-util,rust-embed/debug-embed` 通过，日志摘要 `2c5bb0cee3684f2284ce24c127c261d0090661cd87424eb82d5a9afd103329b5`，短目录已核验清理。产品源码仍与 04ac 完全一致，不因文档更新重复已完成的模型和 GUI 链。

## 2026-10-02：Claude 真实运行取消通过，Grok 搜索与取消协议修复

**新增关闭 0 项，仍为 8 关闭／3 开放／3 后置，PR #22 保持草稿。** `4a78fc3f0/r-agu5bntu` 的固定 Claude `2.1.280/claude-opus-5-5` 真实搜索取得随机答案、命令父子成员已运行后，child journal 序列 15 的 Interrupt ACK 直接接序列 16 Cancelled，两任务均无 RequestFailed。两原始输入与子结果分别只接收一次；取消结果、三个原生/命令资源域销毁及无晚写通过独立审计 `8c8f03120838ae1f33df604c1f0396ac9fe7ba67acfa4d9c038bf46463e7b818`。父后续第 2 代为 unconfirmed，不冒称完成；短根和唯一原件保留，cleanup_ready=false。

同源 Grok `r-m9z049qc` 已在官方 `1.0.41/grok-4.7` 实际派发子任务、完成受审 Skill，随后搜索被错误拒绝。公开 `ToolInput::Grep` 与原生 rawInput 都明确输入 variant 为 `Grep`，当前产品却检查 `GrepSearch`；后者实际是输出名。公开 PermissionCancelled 常量、ToolLoop::Cancelled 映射和本轮原生取消记录也一致，适配器原先未接受该类别。现在仅修这两个精确合同，保留固定版本、路径/字段边界、逐次审批、父权限和超时；新增回归使用原生原始行，验证正确输入、越界拒绝、取消等待同会话/回合历史及冲突终态拒绝。未知类别仍拒绝。因果审计 `1770b16462f27d76fba44d156efb657b1d0153d54780242ce9fa9204d45677ff`；原 exit 2 不改写。严格 cleanup_ok=false 来自 child adapter_succeeded=false，父子原生清理均确认且精确进程/标签已核空，不误报为进程残留；现场保留。

`r-ik6s0205` 的英文/中文四个权限按钮完整，两种共享说明英文各 5 行、中文各 3 行均无截断，画面只在 CUA 回执中。唯一实际 Claude 任务返回 Not logged in，未取得审批，不能计真实审批布局通过。测试私有 PATH 缺少 Claude 用于读取已有登录的 security 命令，而成功取消驱动保留系统 PATH；静态依赖因果审计 `0216cd43eab78264589e545e8df8ce2707f0f39c5a8ac2644c7dbfc75d2ee426`，修复后登录仍待实窗；后续仅修夹具依赖。视觉观察收据 `e5c09005f9d283e1547e4a850c3f8beb87f28ddadb1ad98df0b3548e4e08afe6`。两语言打开面板画面均显示私有固定 CLI，收尾另观察到同 bundle 额外 GUI，不据此声称全程独占；两代和已知进程最终退出，唯一证据保留。

本机 Grok 476 项通过、18 项显式忽略，i18n 11 项通过；源码/收据见 CURRENT_STATUS.json 的 `g10_grok_native_contract_repair_20261002`。提交前 cargo check 通过，三轮短目录均核验后清理；门禁汇总 `207596f6b66dc1365151a727fa2ed8d9f65119840174d328ecc779843ab9b1d8`。最终源码门禁仍待完成。复核英中搜索、取消及命令说明语义，**无需本地化变更**。修复后的 Grok 完整真实父子链和实际宿主审批双语布局仍待验，不因新增回归或静态画面关闭 G10。

## 2026-10-02：Windows PNG 身份补齐，真实取消缺陷与验收夹具修复

**新增关闭 0 项，仍为 8 关闭／3 开放／3 后置，PR #22 草稿。** 精确 `95d16846d` 的 [原生步骤 36967571751](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36967571751) 已分别审计：native `92b51c9b5040`、重建树 `b2ca7be22f08`、补丁 `74abeb50…` 一致。Linux 32 条命令、542 次测试／533 个唯一名，Windows 31 条、520 次／516 个唯一名均过；两平台新增 PNG 17 项逐名核对。实际 Linux ELF 为 `23fecc04a15884d6ea88e08518c74cd17e0f73da3d98e0241fe63d7ad7c3b39f`，Windows PE 为 `168fa15df74a15b8e2487e02ce875aba2c3ac23eefd06302bde0ac1f439450d9`。版本、帮助与异步补全启动通过，零模型；根代理随后观察两平台整 job 元数据 success、无 failed steps；完整日志 FAIL/RETRY/LEAK 审计另行补记，不能作为当前宿主增量的最终门禁。Linux/Windows 独立审计摘要分别为 `d630cd442e5733cfb04dbd3b11351b4fd9015339e2f5dca06624becbb29990bd`、`eeb7d074a13e95230ea7d1405a37368c80cc1b047abfcbc47196970e1db0341e`。原 69c17/f3cbb 失败和既定排除/后置范围保留；Linux 因进程引用可见性不足保留临时目录，Windows 工件没有目录清理收据。

Windows 普通桥现在加入上述 `.5` 精确映像身份，继续要求真实进程/ConPTY 与运行时图片能力；旧 `.11` 文本 pin、Mac/Linux 原有 `.5` 身份及远程 owned `.4` pins 不变，新 Linux 工件仅登记原生构建证据。G03 仍缺 Finder 原生拖放事件及绑定后的最终宿主门禁。

`c53965568/r-2_vfuh68` 真实命令已启动并收到 Interrupt ACK；命令完整清理后，内部工具回复被旧协议当成外部晚回复，公开 RequestFailed，再发布 Cancelled（journal 序列 15→16→17）。因果审计 `00722c63ff9e460f7bdf76afe29ea8b430779115b4aa6ed48949abf492475276` 核对实际命令清理摘要链和精确 launchd job 消失。产品修复仅对同 generation/turn/call/tool、尚未响应且已取消的内部受审命令清理回复丢弃晚结果，保留清理后终态；外部回复、错误身份及活跃调用仍按原合同处理。原真实失败不改写为通过，修复后真实复验待执行。

Grok `r-_gr57x9b` 已核固定 `1.0.41/grok-4.7`、父输入与两次审批，但测试未向 SkillManager 注册 alpha，派发返回技能缺失，尚无子任务、技能/搜索/命令执行。失败审计 `6b8f73e054c6f2cdbae80f74c4e0512478e729f26e5f9adbb8bdb87d6a38236a` 保留。仅修 ignored 夹具：两技能使用生产 `.agents/skills` 目录和真实目录注册，模型前按生产函数校验；原生 call ID 与 MCP 内部 ID 按同 task/turn/generation 的已完成审批关联，不放宽父 alpha 上限、beta 越界或结果断言。Claude/Grok 两失败现场均为 `completed_retained/cleanup_ready=false`，原生清理证据不等同目录已删除。

零模型 GUI `r-hfb9go2h` 的原运行器 PID 33370 实际探测私有固定 CLI，另有 PPID 1 的 33388/35310 使用全局 CLI；安装扫描直接读取 GUI 进程 PATH，并非登录 shell 重采。诊断 `ff6c20937c454b2a619109e311b520f943c99985cc01179a10adfb2b72bfda3e` 未据过程表断定哪次 UI 操作触发额外实例。后续先以 listApps 确认已运行的精确 bundle，再选择已有应用并核窗口/运行器 PID 与固定版本，避免直接 getApp(path) 再开实例；随后进行真实审批双语验收；未改默认设置或全局 CLI。

本机新增 4 项回归 `r-vqbu1xyh` 与完整 Claude runtime 141 项 `r-lf8lz7hj` 均零失败、已清理，日志摘要分别为 `6c92b1e42623072b9dc3c06060ad87eb0779ef35de7f4cc6b73c2d48a67e437f`、`faf257164052b2f7c7694e4319dc87f86e55710ddd5f231eb8dc27ef41a5b464`。i18n 11 项、原生来源元数据 6 项和 cargo check 同样通过；含最终 driver rustfmt 的本机门禁汇总 SHA-256 `304e21a6de4e5439f8b7fd5f4f7a3c7063b28f11b551633a199313096972a946`，全部短目录已清理。最终跨平台源码门禁仍待完成。复核英中取消/已取消、命令审批/等待清理及图片能力/保稿提示，修复恢复既有承诺，**无需本地化变更**；G10 真实审批双语布局仍未通过本轮验收。

## 2026-10-02：两条真实验收首轮失败，按生产合同修复驱动

新增关闭 0 项。`87821e107` 的同提交构建与签名已完成，完整 253 项来源一致。Claude `r-m8dmj72e` 的真实 Grep、隐藏答案与两层命令允许均到达，但测试把 CLI 授权前的 PID version 与 exec 后全等比较，未进入预定 Interrupt；原失败不改写为取消通过。生产 `validate_after_exec` 只要求 PID、unique_id、CID 稳定；驱动据此修正并记录授权与实际完整身份，命令成员的连续实际身份及取消后清理断言保留。不能从已退出原件补造失败当时的 PID version。

Grok `r-dl4kzrdx` 在 `validate_options` 拒绝测试传入的模型覆盖，尚无原生 Ready、审批或模型回合。驱动改用已支持的原生默认模型、空子任务 model_id；父用 `start` 登记 alpha 创建上限，再于唯一 Ready 后提交同初始 UUID 的文本，避免纯文本初始输入清空技能以及 Skill 卡片要求父额外工具调用。独立审计仍严格核实际 `grok-4.7`、双方所选 alpha、原创建上限及全部原生结果；未放宽产品权限或超时。

修复后 no-run 与 `cargo check -p warp` 通过，短目录已清理。另修 Grok 原生来源映射遗漏的摘要声明：129 项映射真实摘要为 `4489e25b1fbaf9435617c9f7bbe226fc6a6d55ef8a58d63efcb86fa324843142`；实际仓库回归先复现失败，修正后 6 项通过。仅元数据变化，不更改原生补丁或二进制。门禁汇总 `8f1b92443a8cb70a952a342003cbcbd7c352910db320e3082bb36195d480d4b5`，不替代真实复验。

两轮原件保留，精确收据见 CURRENT_STATUS.json 的 `g10_live_contract_repair_20261002`。本次仅验收驱动修复，无需本地化变更；新的真实复验和英中实窗仍待完成，G10 不关闭。

## 2026-10-02：命令审批提示字段修复，G10 剩余真实验收待完成

生产命令审批写入 `reviewedProjectCommand`，任务面板此前检查不存在的 `appCommandContext`，导致既有专用提示未显示。本轮只修正该字段判断。英文与简体中文 `cli-task-manager-reviewed-command-approval` 已逐句复核：均说明精确程序、参数、目录、超时、单次允许及清理后重新审批，没有变量差异；无需本地化文案变更。新构建的两语言真实布局尚待验，不以源码检查替代实窗。

另集成两条按需运行、默认忽略的真实验收驱动：Grok CommandsSkills 父子链包含唯一原生文件搜索、技能、命令拒绝后独立允许及越界子技能拒绝；Claude 子任务包含唯一原生搜索与真实 Node 父子进程运行取消。两驱动 no-run 类型编译、i18n 11 项、任务面板 46 项及 `cargo check -p warp` 均通过，短目录全部清理；提交前精确增量收据 SHA-256 `35888c794f4eaed296803a8164a979d5f23560603224db2ff1e05f31f843883d`。真实模型与 GUI 尚未执行，不计通过。当前新增关闭 0 项，G03/G09/G10 继续开放，PR 保持草稿。

## 2026-10-02：G10 已完成父任务实际接收子命令结果

本轮新增关闭 0 项，仍为 8 关闭／3 开放／3 移交。`r-1cem30qq` 使用同 `f3cbb6093` 提交新构建并独立签名的 libtest／worker，摘要分别为 `20343d6b3596c127a25d22498f3e8bcded8f641a308338d209669ca5689f3d30`／`e3dbb7e1be4aaccae0d77db4843be37169196233e7f0726d25aa49d355ee7ef8`，不存在旧 worker 等价替代。只读核对本轮两精确原生 SID，实际模型均为 `claude-opus-5-5`、CLI `2.1.280`。宿主三次命令决定为 DenyOnce／AllowOnce／AllowOnce，拒绝代没有派生记录；A、B 两命令实际退出 0 且 macOS resource coalition 清理确认，A 清理先于 B 审批。

父第 1 代 `PARENT_QUEUED` 后，同 SID/runtime 第 2 代接收唯一自动结果并输出 `G10_COMMAND_PARENT_COLLECTED` 及 A/B 实际标记。父 2 输入、子 1 输入的原生 UUID／正文分别与两个 user_input 和一个 local_task_result 持久消息匹配，均 NativeProtocol ACK；自动结果保留原 recipient_generation=1，无重投。两宿主及两原生退出收据逐份匹配，原生均 stdio_closed／exit 0。独立审计 `139342271058682a9849f5d845baff24eb7334a7f3f1a3b6b53625e098af1967`，助手 `ebdf20d0f6476ab02ce01ac9c86aab511e699a10ed969c5869752e909d556a25`。

libtest exit 0；外层原 exit 1 与收据 `442fa5e3a41afbf25e748aad5b752dada13809e4e562094f8b941010b797ce9f` 不改写。失败来自外层假设恰好两次 SessionReady，实际父／子／父三次中父两次五项身份字段完全相同，各自属于独立已完成回合。新离线判据要求恰好两个身份组、重复字段全等和逐轮 start→Ready→completed；四种身份漂移及同轮重复 Ready 均拒绝。原 46 事件没有过滤或重放，其余所有原 audit 断言通过；修正助手 `ceee9141c2d8ee93a84e9dc10c2d660691fdb5513e66e3f45c124dc44b23164b`，差异 `509f3464275c4b7f8f67961bdbb63e82b4d64b1bed6f89493efdd26acdad448d`。

外层观察已知进程为零、打开文件为零；唯一现场与历史仍保留，cleanup_ready=false。该轮未覆盖 GUI、冷恢复、运行追加/取消或 Grok，也不自行新增每命令族与多子排列为关闭条件。既有其他真实链按原构建独立复用；G10 剩余原条件与最终源码门禁仍须完成，PR 保持草稿。本轮仅验收记录变化，无需本地化变更。

## 2026-10-02：Grok 源码包绑定修正，原生门禁仍待重验

新增关闭 0 项，仍为 8 关闭／3 开放／3 移交，PR 保持草稿。精确 `f3cbb6093038dde43393b8f5cce77b477b93ec10` 的 [CI 36940578820](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36940578820) 已终态失败：两平台 `load_source` 读取的 `cross_platform_build` 仍为旧 `8644` 三身份字段，与已更新的 `92b` 补丁不符，原生 Cargo 尚未执行；Windows 上传失败是工件目录未建立的后果。Linux 同名 artifact 只有临时目录收据，不含原生构建日志或二进制，不能据上传成功计原生通过。

宿主 nextest 的 Linux 5329 次／5070 个唯一 package＋name、Windows 4958 次／4711 个唯一 package＋name 均普通 PASS，G10 新增 12 项在各平台各一次通过；两端 desktop／TUI 各 670 普通 PASS、rust-genai 各 81 项另列，无本轮 LEAK／FLAKY／重试。逐项与跳过审计 SHA-256 `9468a2e41ac001354fd893d5de8af32e53bcc8275c7da7bd341af5f7593cf883`。旧 69c17 的 Windows Claude 通知失败、Grok 5 秒审批通知超时和桌面 1 条 LEAK 保留，未由本轮原生前置失败覆盖。

本次只同步 `cross_platform_build.native_commit/native_tree/patch_sha256`，历史实际工件的来源逐字段保持不变。新增回归读取两个实际仓库源码包：`r-c7qxeka2` 在修正前复现同一错误（4 过／1 失败，日志 `53e00ab1dc183f8b28e6c830f3da7a680d7ef9b582de68c0e083b9c42c8c8722`）；`r-qdeajcxl` 修正后 5／5 通过（日志 `a04ef3c4df9602b0f4b936a455b1fbe11fb33acac7b5d24283eee50352d887bf`），`r-dxvcenat` cargo check 通过（日志 `356c07ef9b2af4c3995b9ad97bd1b38af6d9fd9dc7e45c65e2ee5daaba145caa`）。三轮已核实退出、占用和归档并清理；修复收据 `e5b9d8470359b90601236e42371c60d79c394de41b97f0c212804d6f7ab29dcb`。无需本地化变更；目标原生构建、旧超时复验、工件绑定与最终门禁仍待完成。

## 2026-10-02：G10 父结果回传修复，本机门禁通过，真实复验待执行

新增关闭 0 项，累计仍为 8 关闭／3 开放／3 移交。源码冻结 `739193b2dc970d78c35635bb6704ac13d294cd743c312d56dcbc1db7ed3fa407` 补齐 Claude Completed 父任务的专用结果消费；领取与单槽共用 SQLite 事务，原结果代与当前准入代分开保存，跨代仅接受同 runtime／SID 中逐代由真实 NativeProtocol ACK 驱动的自动结果。多个子结果保持 Queued 后串行交付；普通消息不扩权，显式用户新轮、新进程、失败或取消断链；在用户换代后才生成的旧结果保留正文但标记 Cancelled。

`r-f1idf8ok` 的协调器／任务持久化／邮箱 166 项为 163 过、3 失败，日志 `badff2a3a79a592ffd75af3822ee5ae2146277e128e4989b331783844ec8cf3f`。失败分别是两子夹具复用原生 SID 触发唯一约束，以及旧夹具使用非真实自动结果正文；只修夹具，不改生产校验、断言或超时。`r-y8cm90ip` 三项定向复验全过，日志 `f8f8f1a7a606e76a30d42fa16e6fecdb604c28d07f1ad64271182ca9377c62a2`，不回填首轮为全过。`r-63_zvl1g` i18n 11／11，日志 `35473dc839d4c08d8e10084a820f9cb0ca38e7c2333abdec0cf9aecc0f361522`；`r-h1tjavt5` 提交前 cargo check 通过，日志 `356c07ef9b2af4c3995b9ad97bd1b38af6d9fd9dc7e45c65e2ee5daaba145caa`。四轮均核实退出、占用与归档后清理。独立源码审查未发现新的确定阻断；汇总收据 `1b5a06138f987a47a802588d7abdcb3a0537a6807321553ab3823098286cd098`。

本次沿用的英文及简体中文错误、结果标签已审计，无需本地化变更，也无布局变更。Grok 公开补丁构建器 Python 4 项另通过（`r-m8tun0oz`，日志 `e73c3805f810fdcf3d0167ced7085c48306c9482a952ffb710551d230cdb3e51`）。G10 新 worker 与 libtest 的同提交真实受审命令复验、G03 Windows 工件和最终源码门禁仍待完成；PR 保持草稿。

## 2026-10-02：G03 双语拒绝布局通过，G10 自动子结果回传失败

本轮新增关闭 0 项，累计仍为 8 关闭／3 开放／3 移交。`r-goa_c41x` 复用 `69c17fce3` 的产品构建，旧 `.3` 文本原生会话在中文和英文分别一次 Return 后显示完整图片能力拒绝提示：中文在 2093／5012 ms 为单行，英文在 2094／5013 ms 为两行，均无截断且文字与图片保留。首尾约 0.5／9 秒画面无 Toast；这是有限采样，不是连续录像，也不将旧轮未观察到提示改记通过。视觉收据 `366f8c8647f890e305ede598419a88f4bc21362bfd0093d3bfbe37548b6ab227`，截图只在 CUA 回执；三代 GUI 正常退出，Preview 打开对话框导致的图片映射在 CUA 退出后释放，最终 finish `a4b17a803aaf398de7c8e979eccfa14643ef76c4dff6d8fb0950bfc37aecb73d`。同 SID 最终局部只读审计确认真实用户输入／助手输出、宿主消息与领取均为 0，三代九条 SessionStart／SessionEnd／Stop 全部成功，收据 `8ac7494197603026c59627905a79c86304d7fb2bdbdb100bc284e4e58663371b`；不将这些计数外推为 HTTP 请求统计。现场保留，cleanup_ready=false。Finder 原生拖放仍欠证。

`d49c24be6` 的 G10 受审命令入口已实际执行 `r-62nv09__`：两 SID 的原生模型均为 `claude-opus-5-5`，一个父输入派发唯一子任务，子命令 A 拒绝后新请求获准、B 获准，两条真实 exit 0、macOS resource coalition 清理确认，子结果包含两标记。父首轮 `PARENT_QUEUED` 后进入 Completed，自动 `local_task_result` 永久 queued、无 ACK，最终 450 秒超时，libtest exit 101／外层 exit 1；诊断 `a715c43c9c840bf76d094071111512698e91742d8fab7142931f24635c2b0387`，原生最终审计 `22518c319458643c85b0c211ee4cf9db4218bd92bea1d9b02dafa1de96c982fc`。两个 runtime 均有退出及清理回执；并行 G03 服务不算作 G10 后代，原失败不重写。失败构建的 endpoint、worker 请求准入和领取事务对 Claude Completed 父回传均有阻断，后续修复及验收按本报告最新条目计证；本轮命令正例不关闭 G10。

[69c17 门禁 36927434162](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36927434162) 的 Linux `.5` 官方工件已核：ELF SHA-256 `e3b1bea1e10ded36c70f1d641acf2e4a73a9dfcffd9ca3236f51ea6ef46dc6f2`，真实版本 `1.0.41+infinishell.session-notifications.5 (07e35a3dfeed)`；32 条构建命令通过，542 次测试／533 个唯一名称，17 项新 PNG 用例各一次通过。审计 `5393dcaa2f2d767553c9d6b1ff4b4a080b281e540233ede01b2521b9bcaee9b7`，未在本机执行下载工件；审计后已将该精确 Linux 工件摘要加入普通桥白名单，绑定后源码门禁待验。Windows `.5` 四库通过 58＋32＋4＋4 项，通知组在 `hook_ask_under_yolo_fires_permission_prompt_notification` 的外层 5 秒等待失败，无 Windows 新工件；审计 `40f504be360961e4e984480930158d0a3fcc2c136f222b5ada4ba503c0dc459f`。相关七文件与此前通过 `.4` 相同，但日志没有 hook／审批阶段，不能确定根因或归因环境。另一个 Windows Claude SessionStart 失败继续保留；整体源码门禁尚未通过。

Grok 原生 `92b51c9b5040` 相对 `.5/8644e7f68a5c` 仅改两份测试，给单一超时用例增加白名单阶段计数和耗时，生产行为、审批、5 秒／3 秒限制及断言均保留。Mac 通知组 5／5、共享 PreToolUse 组 15／15 和 `cargo check -p xai-grok-shell` 通过，日志分别为 `856dc1c671fc1e3f7a2c1449ada353eafe0779231b826565e86e90026d96a965`、`0af289ac4b107617575ce4019753ed879b754995fa1ddee145be0ac7f2ecd0fc`、`eba9e918b39f044330fc1a45fb18a9fa5a2d1ab49d8aeef01534fe18f2c64ff8`，三轮目录已清理。公开补丁 `74abeb501baa213423b17a0c90638fb9f70b6eb3c8da62c0064fac835e205c24` 以隔离索引重建精确 tree，收据 `ddf74e7759bae48edf5da3601b601a7ed0b50331ed0d82b55a80db81658df585`；旧原生工件继续按原源码计证。无需本地化变更，诊断不计修复或关闭，Windows 复验仍待完成。

## 2026-10-02：G10 受审命令父子链验收入口

新增 Mac arm64 的显式忽略测试 `real_claude_g10_reviewed_commands_parent_child`，调用生产协调器与真实固定 Claude `2.1.280`，限定一个子任务和两条项目脚本：首条拒绝后须由原生另起请求，再允许 A、等待真实退出清理后允许 B；分别核原生工具审批、宿主命令审批、持久父权限上限、输入与结果 NativeProtocol ACK、实际标记及两代退出。入口编译与现场执行分别计证，新增测试不计 G10 关闭，也不覆盖 GUI、冷恢复、取消及其他命令族。此次只增加测试入口和证据记录，生产行为与英中资源未变，无需本地化变更。

`r-jofm7jzr` 的 `cargo +1.92.0 test --locked -p warp --lib --no-run --features warpui/test-util,rust-embed/debug-embed` 通过，日志 SHA-256 `20a7dca4431fdb5730b9cf8fdd2bf38cc3ce876cfe8a110c91ffaf70a57928e0`；`r-rmk7_gxo` 的提交前 `cargo +1.92.0 check --locked -p warp` 通过，日志 `b419cd4461ce6d5116e27b0d10a43c75c0fefb95faea585d3b7525def33c5e8e`。两轮临时目录均在核实退出和占用后清理；编译前两次语法／生命周期失败保留在仓外 `g10-preparation/compile-source-v1` 和 `v2`，修复后源码摘要 `ee164d90c25d3ce08072065615b386aa4ab58f5d33f4a0f58e03eabba77bada4`。尚未运行真实模型，不更新缺口关闭计数。

## 2026-10-02：G03 普通 Grok 图片实窗取得正例，仍开放

本轮新增关闭 0 项，累计仍为 8 关闭／3 开放／3 移交。`69c17fce3/r-idckjva9` 绑定 Mac `.5/8644e7f68a5c` 普通未绑定 PTY：纯 PNG 的唯一原生 ACK、宿主清卡已验；模型识别红蓝后自行探索，由主代理取消，不能计完整回答。随后文字加红蓝／绿黄双 PNG，原生输入顺序和规范 PNG 字节一致，完整回答正确识别四种颜色，文字与卡片清除。分别核原生收据 SHA-256 `4564c37c2fabe7b477e6aa359ae82fdea0be2a5fbdfa2a1bcc709317e68a439b`、`70639814c8091f02afa23cb1cbb925d349a88ac5e6e20516e54ffb8fea2ac379`；不把原夹具 454 字节冒称剪贴板规范编码的 1044／1045 字节。

原生待审批时，另一文图草稿经过焦点往返及一次 Return 后完整保留；本 SID 人类输入仍 3、图片 claim 2、原生 journal 6 条。随后 Ctrl-C 拒绝原生请求，文字也被按键清空而图片保留，这一步不计文字保留。英文和简体中文的卡片、既有占位及富输入控件可读；新图片能力拒绝 Toast 尚未观察，不计双语拒绝布局通过。旧 `.3` 负例在三次主动 Return（两次纯图、一次文图）后 human 0／journal 0、宿主 claim 不增加；未见 Toast 原因未定，不猜测生产修复。指定两 SID 最终只读诊断 SHA-256 `d07a6f4aab17708f88ed1e2fee8da1e8cd66f7fe29dd65a3d5dfe14c73452edb`。

GUI 观察 SHA-256 `eae1971deed36c4e9206956224edc4c47f78701ac9ebf7a38b4e6bc04aa116a0` 明确保留两项操作边界：CUA 中文 typeText 错误已在提交前全选粘贴纠正；菜单误路由曾提前提交原计划第二条输入，不能据此声称该条提交前切换窗格保稿。收尾 SHA-256 `710bf723a55702a7baf9591ba1dc1ece96d5be28127b34f67357bafe7df6ee4c`：两代 GUI exit 0、已知后代退出；首次 finish 因 Preview 仍映射图片失败，关闭已无文档窗口的 Preview 后打开文件释放，第二次 finish 成功。现场和唯一历史保全，cleanup_ready=false，不冒称整棵原生服务树清理。

Finder 原生拖放仍缺事件。精确 [69c17 两平台原生源码与宿主门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36927434162) 尚在运行：Windows Claude SessionStart 的空格路径 case 已失败（原生附着后 45 秒无通知并强制清理），另一特殊字符路径 1.405 秒收到通知且自然退出；两者分列。23 份对应来源与此前通过的 b47 一致，不据此归因 G03 或认定随机故障；没有重试，零模型／凭据输入，失败收据 SHA-256 `99f3b040597f6fb7422ecdcbaf1f3857a8824b49b628cf6cfbd356a13e5e5e86`。Linux／Windows `.5` 实际工件与最终门禁未提前计通过。

## 2026-10-02：G08／V03 在本次 Mac 范围关闭

**本轮关闭 G08、V03；累计 8 关闭／3 开放／3 移交，PR #22 保持草稿。** 仍开放 G03、G09、G10；V01、V02、V05 及其他平台实机验收按用户决定后置，均未改记通过。以下历史“未关闭”结论保留当时范围，不回填旧失败。

G08 的三款真实 SSH／tmux 首图、完整 SSH 断开后原 SID／消费者恢复及第二新输入，共六次 typed PNG、双方确认和引用释放按 `7b7fc36fc/r-hx4at14_` 原构建计证。Claude Read 待批重连后的同文同卡再次提交没有新增 claim／queue／原生输入；旧请求完成保留新稿新卡，随后不同内容正常消费。拒绝与退休清理沿用 `c3ff/r-u10vfmtu` 的真实 Read No，不改称本轮重跑。137 字节夹具规范编码为 328 字节，原生 typed PNG 与规范上传字节一致；不声称原编码未变。Codex 逆序问题仍答正序的模型语义失败保留。英中菜单、恢复、附件及相关提示按原实窗审计；功能审计 `g08-final-functional-audit-v2.safe.json` SHA-256 `300f309fcb7c2baf9aa1aca0eea6d8a9655635d42d7ba13c5f3314b167909085`。

V03 的八项原条件逐项核验：三款真实通知／双向交互与恢复，实际重复提交及晚到事件，真实运行回合取消，以及 Codex 关闭 tmux 透传的独立双端负例。`b47/r-o5rrajvm` 正式 Claude 通知就绪后的流式 Ctrl-C 和 Codex 工具运行 Escape 分别取得原生 Interrupted／turn_aborted；Codex 同 turn 的迟到 ToolComplete 后 GUI 仍为 Blocked。Grok 取消与 `.4` 正常退出分别沿用 `51b0/r-myk6vj3w`、`r-dpopymbl` 原证据。不开新模型轮次，不把 Claude／Codex 的 Blocked 当作收到 cancelled hook，也不声称停止整棵工具树或现场证明 listener 内存退役顺序。完整审计 `v03-full-conditions-audit-v2.safe.json` SHA-256 `dbbe7602c89b5c4068e84c458c4fa809491836ed030aa1fa065d2b57d820b43f`。

精确提交 `b47a86ff71d64eb033030da8f80205fd7e00e0c6` 的 [最终宿主门禁 36916171947](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36916171947) 已终态成功，并逐份核对完整日志：

| 平台 | 实际通过 | 明确边界 |
| --- | --- | --- |
| Linux x64 | check；lifecycle 1872；IPC 2；glibc 1；command 14；监督者 5；rust-genai 81；原生通知 worker 的 10 项诊断与 15 项功能，含真实 tmux pane／TTY 写入 | installed hook 缺 fish／zsh 的 2 项跳过；本轮 desktop／TUI 步骤按所选 scope 跳过，不计执行 |
| Windows x64 | check；lifecycle 1532；IPC 2；原 ConPTY 1；command 36；监督者 5；desktop／TUI 670；rust-genai 81；SSH worker 构建及固定 CLI 通知／安装／退出边界 | 本轮 host-only，未重建原生 `.4`；Windows 原生 broker 与源码工件沿用已验 51b0 来源，不冒称产品 remote-owned Windows pin |

两平台本轮均零 FAIL／RETRY／FLAKY／LEAK；Windows 前轮两个 context-menu LEAK 用例本轮同名均普通 PASS，**不据此认定旧泄漏根因或永久修复**。`b47a86ff7-ci/host-final-results-v1.safe.json` SHA-256 `22f33836c2ed44e640e29264fd3afdc576ce2ecdb19b924960294fe9758ba1e8`；Windows 全日志 `05a5de52291982446f3dd7c0a11ba4c5885e780037f977db6447dff546ad92d3`，Linux 全日志 `c0ab4668c9dccb9cc4e850cede3a9d928cbfbe141fa50c32e548aaad811a9a84`。日志 API 空响应及一次 gh 日志缓存 ZIP 读取失败另行保留，随后标准读取成功；没有重派或重跑门禁。

Linux `.4` 的真实 SHA／完整版本已绑定，Mac `.4` 原工件不变；原 `.3` 图片证据结合已审 `.4` 影响范围沿用，未虚构同一工件全量重跑。G03 `.5` 仅普通 PTY 图片路径，仍独立开放，不增加为 G08／V03 的关闭条件。历史 Mac 原生 LEAK、7b7 原失败、51b Windows 两条 LEAK及现场保留约束不删除；已知自有进程退出与唯一历史保全分别计证，cleanup_ready=false 不被改写为已清理。既有英中功能审计有效，本次仅更新状态文档，无需本地化变更；提交前主代理另以短 TMPDIR `r-swy1h4_m` 完成 `cargo check --locked -p warp --features warpui/test-util,rust-embed/debug-embed`，exit 0，日志 SHA-256 `7b61007231a2ce5948224fcc55eb1ea4bfa644a81e87b0a20f6885da4f1717f9`；进程、launchd 与打开文件核验后已清理。

## 2026-10-02：普通 Grok 原生 PNG 实现与宿主门禁通过，实窗待验

**新增关闭 0 项；6 关闭／5 开放／3 移交，PR #22 草稿。** 固定公开上游 `07e35a3dfeed` 加完整 128 文件补丁可重建原生提交 `8644e7f68a5c7803e3d1b3cc174ae56b93552ef2` 的 tree `c49bef7a6d381709ca7bc6de8b0f9fc9c4bd5478`。补丁摘要 `0a8fa3da52bea3c19fa57e7d3725bec20c3721960a155dd2d596cd27cf8dbd92`；Mac `.5` 签名工件 SHA-256 `b5432ea1a6b4fec7d898de5f3d289ec55a838b70cb7797ecacaeb982f79ce444`。只将它加入普通未绑定 PTY 桥，远程 owned 继续绑定 `.4`，旧 G01 文本工件不变。

普通桥新增 `typed_png_images=1` 能力及有序 PNG 批次，纯图允许空文本；原字节、CRC、解码、像素与完整 256 KiB 帧预算先验完才领取，不缩图、丢图后部分派发或借路径补图。原生 actor 复核本次准入 ID／整批摘要后保留 typed 图片；原文本摘要与 `@文件` 语义保留。宿主将独立 rich 主题保存为当前数据库 scope 的内容寻址引用，历史 ACK 不因旧图丢失阻断无关查询；实际新领取仍完整重读核验。图片粘贴、完整拖放批次和双 revision 原子清稿已接线；文字或附件任一改变时，旧 ACK 保留整批新草稿。

原生 `r-tmi2mncu` 新增 17 项串行及四库 91 项默认并行、check 均通过，17 是 91 的子集；`r-xym2qmet` 原 18 项断言通过但 1 项 LEAK、根因未知，不回填为通过。独立 Mac 构建／签名／三个零模型入口通过。主仓最终 `r-3cjni7xg` 131/131、`r-1f0ltp5s` i18n 11/11、`r-76bttzv8` check 通过，无 FAIL／FLAKY／LEAK／重试；短目录均核验清理。收据 `3edc7062a01deed42f4d0a4f6f255321eb91f83a8be19415cd8a2519cb26d073`，128 份完整整合来源冻结 `a2dd970cbdc35f01eb305dc39434038c584003a3190c124f6fb57a6106e9df28`。两次导入编译失败、两轮 129/131 的 fixture 失败均保留：先补 ordinary shell 元数据，再按源码补合法开始→完成→Precmd 初始化，未放宽生产守卫或延长超时。单独复验两项通过后，完整 131 项重新通过，不把分轮结果拼成首轮通过。

英中同步复核三键：受支持 Grok 版本的图片入口、当前连接不支持图片的保稿提示、重复“输入”提示（覆盖纯图与图文）；均无变量，真实布局待验。原生只增加协议字段和稳定码，无需本地化变更。公开 patch 的第 1867 行为统一 diff 的空白上下文行，外层 `diff --check` 会提示空格；原生实际新增源码与其余主仓文件检查通过，冻结 patch 原样重建 tree 已核，不删除上下文空格破坏补丁。

G03 剩余：普通 PTY 实窗纯图／双图顺序、焦点、审批等待、双语布局，真实 Finder 事件交付，以及 Linux／Windows `.5` 实际工件和宿主源码门禁。既有 Finder 自身校准也未交付拖放，不能把模拟窗口事件当作 Finder 实测，不再重复同类坐标探针。

G08／V03 的 Mac 功能已按原条件完整审计，尚等 [b47 最终源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36916171947) 终态逐项核对。V03 新 `r-o5rrajvm` 的正式 Claude 通知就绪后实际流式 Ctrl-C，以及 Codex 工具运行 Escape／迟到 ToolComplete 后保持 Blocked，分别与原生 Interrupted／turn_aborted 对应；不冒称两款存在 cancelled hook 或按键停止整棵工具树。Grok 取消沿用 `51b0/.4` 原证据，未重跑。完整审计 `dbbe7602c89b5c4068e84c458c4fa809491836ed030aa1fa065d2b57d820b43f`，明确源码相等比较是封存门禁与 b47 Git 字节，不覆盖本轮 dirty G03。G08 审计 `300f309fcb7c2baf9aa1aca0eea6d8a9655635d42d7ba13c5f3314b167909085` 仍按六次原构建计证；不新增模型轮次，不提前关闭。

## 2026-10-02：真实取消定位宿主按键观察缺项，修复待新构建复验

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `51b0e4aed/r-myk6vj3w` 的签名中文 GUI 经真实 SSH／tmux 产品入口运行固定 Grok `.4`、Codex `0.156.1`、Claude `2.1.280`，人工输入数分别为 1／1／4。

Grok 在真实 shell 工具运行中收到一次 Ctrl-C，同 SID／prompt 的 StopCancelled 和宿主 ANSI `cancelled/user_interrupt` 一致；原生收据 `a5a20d6b668609f8b968ea0852fc8b16efb906e03b7b0b39fd316b810a6e5a29`，接收审计 `9105550791ff482c1c2f4b2f7244da746d5a74f794e6ecf71170652217d331fa`。后台 sleep 自然 exit 0，不声称进程树被按键清理；随后正常 `/exit` 四项结束 hook 成功。

Codex 单次 Escape 取得原生 `turn_aborted/interrupted`，收据 `c3c783454f04e09ebf7a520e250be9ca07cb5c276a347e7c9670f21e51a25bab`；宿主只有启动、输入和迟到 ToolComplete，没有原生取消通知。源码确认旧版只有共享 viewer 观察 Ctrl-C，本地输入未接线；本轮未直接读取宿主内存状态。修复在已通过控制权检查并实际转发的共同边界观察 Ctrl-C／Codex 独立 Escape，仅启动确认等待，不伪造 Cancelled；迟到工具完成不能解除等待或把 Unknown 提升为运行，新输入或明确终态正常解除。

Claude 第四次人工输入在真实生成中 Escape，原生明确 `[Request interrupted by user]`，本次工具为零，收据 `658ae43baad34e6ca751a01fe8a88a212393a01838c8b2f7ef48a573b591035c`。前三次分别为工具已结束、回答已结束及未证明生成中的早停，不计取消正例。旧快照采集到第四条刚入列却标作第三轮，新收据更正而保留旧件。本轮私有配置没有 settings、installed_plugins 或项目插件设置，只有官方 marketplace 注册；helper 未安装通知插件，因此宿主精确 SID 为零。前置审计 `75b6f680f9f41143640b1592e283ebd500bf71af0fa02c3ed360ce562d68deeb`；仅计原生取消，不计完整宿主链。

四文件 nextest 83/83、i18n 11/11、最终 `cargo check -p warp` 全通过，无失败、重试或 LEAK，三短目录均已核验清理。总收据 `v03-interrupt-local-gates-v1.safe.json` 摘要 `3c4a6c3d347dc675adf5645c04f753a676e470104ae6747d9e9cf99f2b68db70`。actionlint 加实际自托管 runner 标签配置后通过；初次未配置标签的错误保留。两平台定向过滤已纳入会话模型及按键回归，尚待新提交实际运行。复核既有英中 Unknown／Cancelled 文案及语义，无新增控件或文案，无需本地化变更。

本次 CLI、tmux、SSH、GUI 正常退出；专属 daemon 及其 defunct 子进程仍按原宽限等待自然退出，现场与唯一历史保留、cleanup_ready=false。新构建真实复验、Claude 正式插件前置及最终源码门禁完成前，V03 不关闭。

## 2026-10-02：绑定已核验的 Linux 通知原生工件

**本轮新增关闭 0 项；6 关闭／5 开放／3 移交，PR #22 保持草稿。** [51b0 源码门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36898008829) 的 Linux 原生构建产物 `11184345103` 已核官方 ZIP 摘要、公开基线加 125 文件补丁的来源 tree、实际 ELF64 x86_64 字节和三个零模型启动入口。二进制 608,773,152 字节，SHA-256 `d128a7b8f624368f8ae16cba1c40f11d16962b0ce5eee0b37450b889087fd8c6`，实际完整版本为 `grok 1.0.41+infinishell.session-notifications.4 (07e35a3dfeed)`；宿主只将这两个已验证值替换此前未绑定的 Linux 常量，Mac 绑定不变。

32 个构建命令全部成功，16 个测试步骤实际执行 525 次、516 个唯一测试名称，均零失败／忽略；重复筛选不计独立用例。四库桥 50/20/4/4、通知 4/178/203、owned exit 12/4 等逐名核对。来源审计 `51b0-ci/linux-native-audit-v1/audit.safe.json` 摘要 `001f4bb378bf66fc7555d880e3b3963d6745e9fc3846fb8c9a6a3cb0cb46058b`，逐名计数审计摘要 `222f101b0f2cc709e513750de8d197adf6372ac8d42e4dfff7599ad904b911c7`。Linux 临时目录因为进程引用可见性不足保留，cleanup_ready=false；本轮 cargo test 不证明旧 Mac nextest LEAK 已解决。

绑定提交前 `cargo check -p warp` 的 `r-9ik7wddp` 通过，日志摘要 `a2e24beefa0a1d243262c6239330950c80adf273cd128a20cf8f3426c04e31a7`；i18n `r-c_0vh2fh` 11 项通过，摘要 `92841df85882c155c9fcad8107182cf4b1484ba50acabcbed5f1b0b1aa439f74`，两短目录已核验清理。无界面或文案变化，无需本地化变更。Linux／Windows 最终宿主步骤仍在执行；绑定后的源码门禁及正在修复的 V03 中断状态链尚待验证，不提前关闭缺口。

## 2026-10-02：Grok 原生正常退出实窗通过，最终源码门禁进行中

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `51b0e4aed` 的冷启动中文签名 GUI 在 `r-dpopymbl` 经实际 SSH／tmux 产品菜单启动 Grok `.4/ba8ce6d346aa`。目标 SID `02068f66-965b-4e46-b9e8-1121b34f576b` 的 global/plugin SessionStart 成功；在原生提示符输入并目验单斜线 `/exit` 后只按一次 Enter。global/plugin SessionEnd 成功（157/242 ms）、Stop 成功（82/209 ms），目标会话用户／模型／完成回合计数全为零。原 pane shell、TUI、leader 已退出，retired/released 与同一 manifest 匹配，私有 socket 及其目录回收。启动、退出收据 SHA-256 分别为 `a01508b2ae49921b75e11ed92802edf2e284ea07e94053e26f6bbcff6acfac44`、`1be89a860de809d0d7c07f89ffe1e40bbd114224c69bc9740e63a0fc9f7a6f83`，均在 `g08-preparation/r-dpopymbl-tmux-owned-gui`。

宿主日志中同 SID／cwd 的启动及两条退出通知均到达 ANSI 接收入口；零回合的 SessionEnd／Stop 按既有插件合同降为 `notification`。该共享日志没有 PID，不用于证明 listener 内存退役或路由线性化；源码守卫、既有受控断言及真实文件／进程退役分别计证。接收审计 SHA-256 `38fdeeed063f92bfd36a7dc3591580f654ab7befb1704a53b4ad57a7fadf4bb3`；CUA 观察收据 `13a3caebd15ec412ed8fe5b870033cd66c3e54ad721cc5ca8e63f6299ae3c088`。随后 tmux、SSH、GUI 顺次正常退出，GUI exit 0；专属 daemon 及其僵尸子进程沿既有十分钟宽限退出，finish 确认全部自有进程结束（摘要 `16dcb87233a857eeaffeed42d96d01ce64fa7c246a00aa6d951b18099cab736c`）。核实无打开文件后，remote slot 同设备无覆盖保全到原短目录；收据 `a317c0bd7bbef180c818811bf5539f1232e0f1e5856024d9a7af882542778887`。唯一历史与 profiles 保留，cleanup_ready=false。中文通知提示和富输入控件可读，本轮无新增文案，无需本地化变更。

同一现场此前误点全局 Grok 入口，启动了日常 `1.0.46`，输入被转为 `//exit` 并触发一次模型请求；已立即取消并关闭该标签。此错误独立保留，不查询其历史，不计入验收，也不把整个现场称为零模型输入。原 `.3` 退出通知失败与 pager 超时测试 LEAK 均保留；后者原因未知，真实 `.4` 正常 ACK 正例不覆盖 LEAK。

[旧门禁 36881884266](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36881884266) 已终态失败：Linux 18 项 lifecycle 夹具失败及通知写入超时；Windows lifecycle 1440 项中 1425 通过（含 2 项重试通过）、15 失败，17 个初次失败均指向旧 Codex bootstrap 顺序。Windows 另有 Claude 第二子进程 stdin EOF 5012 ms 未退出、Grok console broker 拒绝访问；桌面/TUI 670 项通过不能覆盖这些失败。Windows 终态审计 SHA-256 `fa014fc3f0660b5828c36f079fdb9c6fb1e13b48827594804568108b922decb3`。已完成 Mac 门禁的 `51b0e4aed` 正在运行 [新门禁 36898008829](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36898008829)，明确选择 Linux／Windows 通知源码及宿主回归，不增加超时、不把新运行当作旧失败已修复。

V03 剩余条件逐项审计 SHA-256 `5d80ceac99115922f8e0547a8942b82e47dd9a7f9a03b5243748fd637235e4f0`：三款真实 SSH／tmux 运行回合取消仍缺证据；空闲 `/exit`、审批拒绝和受控取消回执不能代替。当前继续完成 G08 最终源码门禁。本文提交前 `cargo check -p warp` 在 `r-d1zsoiow` 通过，日志摘要 `11d98597b65cb6e41f1b24198f9898e86f8ee44c8f088d1891a82c39a0416d64`，短目录已核验清理；其余源码／i18n 未变，不重复模型输入。

## 2026-10-02：三款真实 tmux 图片、重连与旧请求保护完成，源码门禁仍开放

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `7b7fc36fc41d79be842ecda87e2fc7c7dc521419` 的签名 GUI（SHA-256 `123a4112528cbd2eaee76bdd5d98a56acfc1dbb5153f816be3ada6c3b028728b`）在 `r-hx4at14_` 实际回环 SSH／tmux 中完成三款固定 CLI 首图、完整断连重接和原消费者第二条新内容。Grok `.3/975d815fec8e`、Codex `0.156.1`、Claude `2.1.280` 分别保持原 SID／进程生存期；六条真实请求均有原生 typed PNG、独立请求及两端 ACK／引用释放。137 字节源 PNG 规范化后为 328 字节，上传、原生图片及像素内容分别核对，不称原编码字节不变。索引 `resume-20261001/g08-preparation/r-hx4at14_-tmux-owned-gui/functional-review-v1.safe.json` SHA-256 `a869d1ff4b0dda5cb675a8356c320a286f803a0e43f6ca46413686f5c957be85`。

Claude 原 Read 审批在 SSH 断开后仍保留。恢复后同内容确实产生新 intent／上传，但没有新 claim、queue 或原生输入；改为新内容后正常消费，证明并非所有重连后输入都被拒绝。先写入新草稿，再仅允许原 Read；旧请求完成及清理没有清掉新稿或附件。此为真实旧请求跨新连接回执证据，不冒称人为强制旧 socket 乱序或 daemon 冷重启；连接替换另由原提交的受控协议回归覆盖。对应重复及消费收据 SHA-256 为 `3f1bca51325222e704f6cec942484ed87ed1aaf8d6ef759f9567615941f4cf57`、`ccd5a8b17e8b0ed5438c18d5096a5deed67ced86fdb7c3a839066364fa259c65`；新内容消费为 `fc5c4a508d8e6f536059cbd17123aaa7fee04261704a05a842cfdd2b896b651a`。本轮两次 Read 都仅允许当前图片，没有授予目录权限。历史 `c3ff/r-u10vfmtu` 的拒绝与退休释放正例仍按原轮计证。

模型结果分开记录：Grok 两轮正确，Claude 两轮正确，Codex 首轮正确；Codex 第二轮收到完整的“右到左”新提示和实际图片，却回答正序“红色，蓝色”。第二轮传输／确认通过，模型语义失败保留，不重跑相近图片掩盖失败。Codex 第二轮收据 SHA-256 `1e2ff884444a89657c4f1ab2d0a7805369c35e9d06e637d60de288715631fe75`。三款消费后富输入自动收起，重开为空稿／零卡，没有手工清稿；Claude 新草稿保护场景另列。菜单和恢复控件的英文／简体中文、中文输入占位及长图片名本轮均可读；热切换语言后旧通知设置提示仍为英文，不外推完整冷启动中文应用。本轮没有新增用户文案，无需本地化变更。

三款均经原生 `/exit` 退出，tmux／SSH／GUI 顺次正常结束，专属 daemon 沿既有宽限自然退出。finish 收据 SHA-256 `ae9a777efe6fdde1f48aa5c63d7ffef23c5e936edf6ac2b25755355952f2175d` 确认全部自有进程退出；本轮 remote slot 核实无打开文件后，同设备无覆盖移入 `r-hx4at14_/remote-server-retained`，收据 `f9a8a6779eb41011ba041d8eb6f3f2620f5747081003569c2499a3151b215f90`。唯一历史与 profiles 保留、cleanup_ready=false，不称整轮目录已删除。Claude 插件安装首轮在写入前因受 Git CRLF 规则影响的文件摘要不符而失败，第二轮同时绑定 Git blob 和工作区字节后完成官方安装、补丁应用及检查，失败原件未覆盖。

Grok `.3` 的 SessionEnd／退出 Stop 在通道撤销后执行而失败，资源退役成功不等于通知成功；该原生缺陷继续在 V03 修复，不新增 G08 图片关闭条件。[7b7 平台门禁 36881884266](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36881884266) 已出现 Linux lifecycle 测试失败和 Windows Claude 权限探针／Grok console broker 失败。Windows Grok 不是编译失败：真实 broker 正例退出 1／`0x80070005`，原始日志未带具体 API 阶段。新源码修复和最终门禁仍待完成，G08 保持开放。

Grok `.4` 原生修复提交 `ba8ce6d346aa36fc7adf67d8e3c3c3130aac250d` 已完成 125 文件公开补丁重建（tree `58b0b69b353d51218043f4c0b6cbd5898a27573d`），Mac 工件 SHA-256 `b196c3a073a37a109af57ddb4d12d45ca40eaa2a99d6b08c5c5a9c88ddab2ad3`。正常 owned 退出等待原 actor 的结束 hook 后再拆除原路由；繁忙时保留后台任务，不补发 Cancel；断连仍立即撤销。真实 actor 的 global/plugin SessionEnd/Stop 四项成功，本机构建与零模型启动通过；旧 pager LEAK 保留。Windows broker 改为创建未附着控制台的进程，仍保留三管道、最小权限句柄及原 Job；待新 Windows 实测。Linux 18 项 lifecycle 失败已定位为夹具 bootstrap/FIFO 顺序，修复后待统一回归；通知 worker 只补充固定超时阶段，保留原预算，尚不声称根因已解决。上述原生及诊断改动无需本地化变更，GUI `/exit` 与最终来源门禁仍待完成。

本轮主仓 11 文件冻结后的 Mac nextest 54/54、i18n 11/11、Python 14 项及 `cargo check -p warp` 全部通过，五轮短目录已核验清理；汇总收据 `g08-build-preparation/owned-exit-host-local-gates-v1.safe.json` SHA-256 `5a2acf28c2ec1d3a7a6ce9ff0b4c23e9bf192a40473b1aa113b3f8ed49a85f39`。这只补齐本机源码门禁，尚未复验原生 GUI 退出与两平台源码，不关闭缺口。

## 2026-10-01：修复 Grok 原生扩展查询的线上方法名

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `4aeab1a51/r-e2yx7g07` 的真实 tmux 产品菜单单次启动 Grok `.3`，原生 SessionStart 成功；首张图片只提交一次，草稿和卡片保留，无原生问题／typed PNG，也无服务端 claim。持久 pane 结果为 started，客户端 start-unknown 文件只是写前防重标记。源码核验确定 Mac／Linux 将扩展方法裸发为 `x.ai/session/info`，锁定 ACP SDK 仅接受 `_x.ai/session/info`；查询在 claim 创建前失败，临时 input 随后被清理。未捕获现场 RPC 错误码，不将源码推断改称原生零入队回执。根因收据 `resume-20261001/g08-preparation/r-e2yx7g07-tmux-owned-gui/grok-native-wire-root-cause-v1.safe.json` SHA-256 `b60c7088015062f9339895ff3cca86c28955afdb1b6c112208509a5f5ba06a60`。

现仅修正两处线上方法名，原生 `.3` 工件和权限／身份守卫不变。另修复 Codex 内存测试夹具：等待构造器本地事件 FIFO 完成后再建立远端状态，防止旧本地 metadata 覆盖新远端缓存；不增加 timeout 或重试。Mac 定向 54／54、i18n 11／11、`cargo check -p warp` 一次通过，三轮短目录均核验清理；收据 `g08-build-preparation/grok-wire-local-gates-v1.safe.json` SHA-256 `d85565eb09daf8fcc1bea4116e86afdd08bf63a71287fe99100ae5aabfebd110`。无需本地化变更。真实修复版图片／恢复回归及同提交两平台源码门禁待验，Windows 下载超时和 Vim LEAK 历史仍保留；G08 不关闭。

## 2026-10-01：Grok原生会话证明与tmux恢复接线通过本机分轮门禁

**新增关闭0项，仍为6关闭／5开放／3移交，PR #22草稿。** 完整SSH命令结束后，Claude/Grok恢复现在重新核对原native SID、当前tmux绑定、listener和连接代次，只重建Unknown／Closed身份；旧状态、审批与输入租约不会回放。Grok原生新增仅查询驻留actor的只读身份、当前权限和通知描述符；实际输入在同一队列接收临界区复核default模式及活动描述符，保留原有繁忙排队语义。明确的接收前拒绝保留草稿、释放当前租约，同一message不得重投；真实权限事件撤销缓存和在途证明。

原生提交 `975d815fec8e`（tree `7c4ee8aeae62aa1f4291f35ae98d19a539237c65`）的117文件公开补丁可重建，补丁SHA `79977a05dd2378219b66eeb6e2d9a03ec355a2b616135010589e647479e3f048`。新 `.3` Mac工件SHA `5c1bdc369735850fe26a6a86544b0fa1457d98788873d49bfd44a8f17b054b00`，构建/签名/三个零模型入口及短目录清理通过，工件收据 `c10d211638741db782fcea2586855ec17552f37e346a0b95b3d1d27c9fe75ffa`。原生新增17项普通PASS和check通过；516项并发断言通过但一项LEAK仍保留，单项复验PASS不抹去未知根因。旧a9/.2工件未重标为新来源。

宿主前两次测试编译失败（AppContext和枚举转换）保留。实际488项首轮486通过/2项夹具能力缺失失败，补齐后487通过/1项夹具遗漏真实BlockCompleted目标清理失败；最后仅改该专项夹具两行，`r-0b9wbc3f` 两项Grok恢复独立普通PASS，无重试，不能称单轮488全过。新增9项UI回归均有逐名PASS；`r-t6fk2vr0` i18n11项及 `r-4mzkqakt` 最终check通过，Python来源合同4项/actionlint通过，短目录全部核验清理。107文件分轮门禁收据 `c3573942ccfdfb5abdd348dfd12361b2c8972ac0535c68f896ddaa6b3f5e41f6`。新增一条英中“当前会话状态尚未确认”提示已审计；原生协议无需其它文案修改，真实双语布局待验。

[b38平台门禁36866084183](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36866084183) Linux通过、Windows失败。Windows固定Codex准备仅到下载阶段：两次HTTPError后第三次读HTTP响应超时；另一独立内存协议用例在两次Observe之后等待最终proof/window invalidation超时，三次均失败，不能归因为原生下载。lifecycle为1407通过/1失败；desktop/TUI670项含1条Vim补全Escape用例LEAK，根因未定。该轮未构建新Grok原生，不替代`.3`平台门禁。新版真实SSH/tmux三款图片消费、异常恢复、英中GUI及最终源码门禁仍待验，G08继续开放。

## 2026-10-01：tmux 产品整合通过本机回归，完整 SSH 恢复和真实消费待验

本检查点补充门禁：环境增量后的tmux定向19项、真实API1项和最终check通过；其余101文件与前轮396项/i18n快照一致，分轮收据 `fb2143a28aae68efbfc2701b06bbdb5d89509b4b89ea7ea852e74280872417f6`。源码检查不关闭G08。

**新增关闭0项，仍为6关闭／5开放／3移交，PR #22草稿。** 新窗格启动、持久意图、当前pane绑定、三款图片输入及精确旧引用回收已接线。跨daemon恢复原先会因旧host引用阻止初始化；现所有host未释放图片共同计入既有全局500000000声明字节预算，错误/重复释放不能腾出额度。旧连接回调原先按同session_id覆盖或移除新连接；现在每轮connect/reconnect独立UUID，替换时同步撤销旧host索引和父引用，所有安装/握手/退出/重试/EOF核同代次，全局精确请求的失败收尾保留。

首次整合check两处借用临时metadata失败及242项中12项失败均保留；其中包括上述真实恢复缺陷和UI Toast/事件夹具错误。修复后243项通过，随后预算49项通过，最终合并manager增量的 `r-iv1v21o2` 完整remote_server lib与Warp定向396项一次全过，含6项真实内存协议迟到回调回归；无重试、无LEAK。i18n `r-vq0k1j4b` 11项、check `r-7x9tc651`、公开源码合同Python4项与actionlint通过，全部短目录核验退出后清理。对应103文件快照及收据 `cec5f49756678c9d3022dcb1fc79a70333958102a515558246dd20436440f3cc`；它不覆盖后续环境增量。该原收据的本地化数量误写11组，实际新增9条对应消息，已在后续收据明确更正，原件未覆盖。

源码进一步确认旧tmux server缺宿主协议/版本变量时Claude/Codex不会发有效会话通知。新pane通过tmux原生 `-e` 使用与普通PTY相同的受信产品来源，只作用于本次pane，不改全局/session环境。`r-j95bnz9z` 真实tmux 3.7c单挑战/单split通过：原pane和普通split缺两键、新owned pane值准确、server/session保持缺键；原有argv/cwd/SID/TTY/进程代次、持久target、错误pane拒绝及自有进程回收继续通过。日志 `a34641bf65a53feea559a211622011470f2913bdf48d6d7f9275b3f814fff682`，收据 `5818779c26e16abf354014b28ae49b232f55c98fc01be5afa0c24efc557aa128`；没有CLI或模型输入，不计图片验收。

本轮9条菜单/提示已完成英中对应与资源测试；真实双语布局未验，内部连接及pane环境增量无需本地化变更。另确认SSH用户block结束会清除CLI session/listener，Claude/Grok持续存活时没有新的SessionStart，当前Restore只保留launch不足以重建输入；Codex已有只读Observe路径。该真实恢复缺项仍在修复，不能用新注入的测试hook或重放旧idle/审批状态冒充恢复。任意冷态多意图选择尚未实现，不扩大现有已选中意图的范围。

`737b07f32` 的 [平台运行36856818302](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36856818302) Linux check因两处Netlink比较cast失败；Windows app/native check及旧桥81项通过，通知组因测试 `read_to_end` E0034未编译。已主动取消旧源码后续构建，整轮cancelled；Mac Intel跳过。9份官方工件摘要与ZIP CRC核验，完整日志归档摘要 `fbf03c9ada1e1c18c87915c8e20011dd87a6bb934237f6cdaa3ae1c0167572fa`。Linux显式类型和原生Grok `8855bbe86680` 单行限定调用修复已合入，精确新源码目标门禁待验。补丁重建树 `d71b38fcd6036a2692ac4503460711b6ef05b3de` 通过；相对a9五份测试之外4155树项一致，旧Mac工件仍保持a9来源，未重标为新构建。历史失败不回填通过。

## 2026-10-01：Codex SSH 首图、退出与重连通过，tmux 实现仍在推进

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `879d719be0792de3dda972c7ca83975e249f85e0` 的签名 Mac GUI（SHA-256 `c52ef94a466237852e454c32b842b0e5a8947eb3f54e430d9b9602ae1612766a`）在 `r-9ojwgeap` 实际 SSH 中通过产品菜单单次启动官方 Codex `0.156.1`。原生项目 trust 正常确认后，未发送暖身回合即出现可用富输入；一张图、一次 Return、一个完成回合、零工具调用，回答 `Red, Blue`。原生 `input_image` 与规范 PNG 均为 328 字节／SHA-256 `40e362f2828d701266b69c824f700610ca07e8cd1855fcdcea10462ba4597b2d`，宿主／服务端 confirmed、released、done 与原生队列 ID 一致。原 137 字节图片经过规范化，不称原编码字节未变。

自动收起后重开为空稿和 0 卡，没有手动清稿。一次原生 `/exit` 后 TUI／app-server 退出，票据 released 且 control socket 消失；同一 SSH 重连后只有远端 shell，唯一提交及原生历史摘要不变。消费收据 `7242e781cb44480d6e51e5c8a0cb78f88c1de5bcd13210eaca49df3bb04a7061`，退出／重连收据 `efe857bbea22d64a96bf272972dbdca0ecf4143841846ba75ef00cc767029d01`，均在 `resume-20261001/g08-preparation/r-9ojwgeap-grok-codex-owned-gui`。新短菜单英中两组均完整可见；在私有 GUI 设置中切语言且未重启，不扩为完整中文流程。截图仅 CUA 输出。App／sshd exit0、daemon 沿原宽限自然退出，finish 摘要 `e0633e77a77a7366624abe5ae649885ed3ad8e1bfcbe89726257f07ff0119a6f` 确认全部自有进程结束，外层 exit0／日志 `a4042cbdaa22e48559c1de1a3bc33757d6acbc8dbcc781154be3352f8389fcd4`；实时进程、打开文件、服务及目录身份复核后，原 slot 同设备排他保全到 `r-9ojwgeap/retired-remote-server`，收据 `1c47102113199025e7fe802c3d6e9b80d9819418c2c241beaa0be9dcfab2bea6`；原生历史及 profiles 保留，cleanup_ready=false。

旧 [ea74 CI 36835106269](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36835106269) 已完成且失败，15 份工件的官方摘要、ZIP 摘要和 CRC 已核验。Linux host 1586、新 59、旧 20、client 9、desktop 670 通过；原生 bridge 78／notifications 376 通过；installed hook 17 项为 14 通过、1 错误、2 缺 shell 跳过，真实 main 的控制 TTY 为 0 字节。Windows host 1354、新 27、client 9 通过；原生 check 和 bridge 81 通过、notifications 371 通过／11 失败；desktop 669 通过及 1 条 LEAK 单列。终态审计 SHA-256 `97b18c039fdd1205375430be9577c0ccceaebd6483bc028bf363441e63ab2f69`，不以本机正例回填失败，也不外推到 879 或 aa 测试修正版。

tmux 终端挑战、内核进程／socket 身份及新 pane 基础已实现，初次旧 fixture 缺字段失败保留；修正后 49 项及新增 guard 后 53 项定向回归分别通过。真实 API 三轮曾在同 pane 生存期比较失败：白名单诊断确认同 PID／出生 ID／TTY，仅 macOS `pid_version` 递增。本机 `/bin/sh` 是再次执行 Bash 的选择器，现直接使用系统 Bash 并禁用启动配置，等待最终映像后发布回执；未放宽身份比较。定位收据 `c6e6730cbdefc0f6863ed35005dd756fb21990df08599fd776d0066179955a0e`。修复版 `r-l93zb7q4` 单挑战、单新 pane、带特殊字符 argv／cwd、SID／TTY 与后续完整进程身份一致，旧 pane／撤销／重复保护、仅关闭自有 control、原生退出及清理均通过；日志 `9afe4d7fddb351633b10d6a73a7ff1f67275ed820c984b3c5388f377ee1e501a`。这是实际 tmux 产品 API 验收，尚未启动三款 CLI 或提交模型，不能计 G08 关闭。 基础源码 45 文件冻结摘要 `be36860b27f0740ec4c194caa39a4fdf1f2f23d0becf4191669433aa84613434`；最终 warp 定向 48 项、i18n 11 项及 `cargo check -p warp` 通过，remote_server 前轮 5 项源码未变，分轮记证。Python 诊断 9 项和含自托管标签配置的 actionlint 通过，汇总 `87f56cbe3debeaee12b2b6c8a82a318a747bde1e495499a7ff2e002badad989d`。本基础增量无需本地化变更。完整 tmux GUI 启动／图片消费、断连／拒绝／重复／旧回调组合、中文成功流程及最终源码门禁仍待完成。

## 2026-10-01：Codex 首回合只读绑定已实现，真实首图仍待验

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** 专属 Codex 在首个模型回合前，经原生 WebSocket 分页查询唯一 loaded thread，并用不含 turns 的 metadata 核对 cwd。原票据、TTY、进程出生、socket peer 和连接代次均须匹配；首次 SID 排他持久化，后续消费及恢复保持同一 SID。界面区分只读证明与真实 rich hook，未伪造 SessionStart，也未发送暖身回合。尚未就绪时保留单个有退避的观察请求；旧 client、epoch、block、ticket 或会话不可恢复旧图片绑定。

34 文件冻结 SHA-256 `69228b42ec4167708e6fff1c256b8a10b809b44b460240bf8f6378ad7dad0834`；本机 `cargo check -p warp` 通过。定向原轮 59 项中 58 通过，1 项因内存协议夹具未回应真实目录导航而失败；修正后共享夹具影响的 17 项 UI 用例全部通过，包含真正 Observe 失败后重试成功。其余 42 项源码未变且原轮通过，不合写为同一轮 59 项通过。i18n 11 项通过；受审门禁收据 `g08-build-preparation/codex-zero-turn-local-gates-v1.safe.json` SHA-256 `971e9f6ce5e4d81f824b3ce9f3ce3badfab360d2752a6921bfddb74507f2b765`，短目录均已核验清理。早先导入及事件夹具失败保留。两平台普通和定向源码门禁已纳入新增 UI 用例，尚待新提交实际运行。

Grok 公开补丁推进至 `aa584b262081293d84b6fe78e027df7a3ddd4be0`，相对 `a9c27a25fe22` 仅五份测试文件变化，4155 个其他树项相同；补丁 SHA-256 `1a31ad1713ec038e46c52ba24aaec5146406a87efea25e1491bea7f93adce893`。修正 Windows 路径夹具，并让真实 console broker 用例确认目标已附着且保留失败错误码；不声称 Windows 原生问题已修复。Mac 16 项串行及原生 check 通过，原并行运行的两条 LEAK 保留。Windows 定向门禁只排除经双源码摘要固定、无生产调用的旧 POSIX pager 通知模块，仍保留其四项原失败，新增 hooks runner 和真实 broker 必须通过；Mac 实际工件仍绑定 a9，不回填 aa 构建。旧 [ea74 门禁](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36835106269) 的 Linux 控制 TTY 通知缺失及 Windows 原生通知测试失败均未解决，不由本机通过覆盖。

英中两条菜单标签已同步缩短，修复版真实双语布局及 Codex 首图待验。tmux 的产品入口／原生消费、断线与旧回调异常组合、最终源码门禁仍开放；原生能力实验不计功能关闭。

## 2026-10-01：Grok 首图清稿及退出回收通过，Codex 首轮绑定和 tmux 入口仍缺失

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** 签名构建 `0a7ab86e9fe8276e40afdb6a8be97376b70a553c`、二进制 SHA-256 `5aed4adb2d2c027c376c877a6026896c2b127b5ba6f893fb01c21c90619d39e8` 在 `r-ezc4sbpx` 的英文实际 SSH 中复验。固定 Grok `.2/a9c27a25fe22/grok-4.7` 经产品菜单一次启动，初始全局／插件 SessionStart 均成功。单次图片提交产生唯一 text＋typed PNG、一次 EndTurn 与 `Red, Blue`；328 字节规范 PNG SHA-256 `40e362f2828d701266b69c824f700610ca07e8cd1855fcdcea10462ba4597b2d` 与上传后的内容一致。原 137 字节夹具经过规范化，不称原编码字节未变。

宿主／服务端 finished 与原生 ACK 精确一致，图片输入文件已删除。自动收起后重新打开，草稿为空且卡片为零，没有手动清稿、编辑或重投。真正 `/exit` 后回到同一 SSH，产品确认原生生存期退出并保存 retired／released；socket、leader.lock 及私有 socket 目录均不存在。没有外部信号；leader 可由产品 reaper 回收，不称全部自然退出。消费收据 `d5dc0991614bc28f741211c88f6864e8981d139a5c5305c8173d299a1ebb7b67`、退出收据 `f42de49ab54bbf4504476c1d67691d6a2aa1ce5e63bdd31f25fa09f6adb00963`、CUA 观察收据 `46665a08c9c8137f3dcd4b46f05215164e8c92c3d8d74a78b3f8f46ce5fb61ea` 均位于 `resume-20261001/g08-preparation/r-ezc4sbpx-grok-codex-owned-gui`。截图仅在 CUA 输出，没有外置原图。

Codex `0.156.1` 的通知已由原生显示 installed=1、active=1、SessionFlags Trusted；无需手动信任，但富输入仍未出现。固定官方 `b412ff32` 的新线程先登记 pending SessionStart，直到首个 `run_turn` 才执行 hook，因此原实现不能让首张图片触发首个回合。源码还明确支持零回合的内存 loaded/list 与 metadata-only thread/read，后续以专属票据、原生进程／socket 身份及唯一线程查询修复；不发送暖身回合、不伪造通知。当前零图片／零模型，原生 `/exit` 后 server／TUI 消失、released=true、两种 socket 路径均移除，收据 `a8247892e1da24b191d1e35b1129acaab41b92d8bffc2802ca5aa24e900be7e0`。

私有 tmux 3.7c 的真实单窗格保持同一 SSH、相同 cwd，并开启 allow-passthrough；tab 下拉确实没有 Grok／Codex 当前远端专属入口。原生 server／client／pane、两层 TTY 与 socket 已记录，收据 `88983684ec7a33e08543665f18f3611a03bb35cd50a32ec24eddb0f44f5f6250`。该轮没有 CLI／模型／图片操作；唯一 shell 原生 exit 后返回 SSH，精确三个 PID 均消失，tmux 残留 socket 保留，不冒称整个目录已清理。产品入口和窗格绑定仍是实现缺口。

英文 Grok 入口与清稿流程可读，Codex 菜单末尾截断；已缩短同键英中标签，仍须新构建和两语言布局验证。App／SSH、私有 sshd 与 remote daemon 均已退出；daemon沿原空闲期限结束，000004 finish与外层exit 0确认，日志摘要 `585afb838159919d48597d7bf18bd096c7e15fce9f12385177760c8b9314189b`。实时进程、FD、服务及目录身份复核后，原 slot 同设备排他保全到 `r-ezc4sbpx/retired-remote-server`，收据 `733135cd5b6236c4ab3d3f3247960e946c5a8aa525e420db4ae6e8a1bc0691d3`；现场 cleanup_ready=false，历史、profile和tmux残留socket保留。旧 `r-co3bb00k` 已完成身份、进程、打开文件审计并将原 slot 无覆盖保全到其 `retired-remote-server`，收据 `4b52248a3184ba59f9b3fcf88342ccd84595deaa2d2b704ae8ea7a8bab457d5d`；唯一历史／profile 保留，旧失败不回填。

## 2026-10-01：Grok 真实首图与清稿／锁回收缺陷、Codex hook 参数解析

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `ea74c1d03` 的中文实际 SSH 现场 `r-co3bb00k` 通过产品菜单启动固定 Grok `.2/a9c27a25fe22`。初始插件 SessionStart 生效，选择器单张红蓝 PNG 一次提交后回答“红色，蓝色”。原生唯一 text＋typed PNG、同原生 prompt 的 EndTurn、宿主与服务端 finished／ACK 一致；typed PNG 为 328 字节，SHA-256 `40e362f2828d701266b69c824f700610ca07e8cd1855fcdcea10462ba4597b2d`，与规范上传字节一致。原 137 字节夹具经过 PNG 规范化，不声称原编码未变。白名单收据 SHA-256 `bc757fa89f45e7e913f1d8fe8a9fb4adf6a6c09b15c5e88fec717555cf437955`。

自动收起后重开却仍有原稿和单卡，真实缺陷保留。原生 `/exit` 回到原 SSH shell，TUI 与 leader 均已退出；leader 由产品 reaper 回收，不称全部自然退出，也没有外部 TERM。cleanup-requested 已落盘，但 retired／released 不存在，原生 `leader.lock` 实际为 0644，内容精确绑定原 leader。原生默认 OpenOptions 受 umask 影响生成该模式，宿主却套用私有 JSON 的 0600 合同。退出收据 SHA-256 `00eee9c3af4f2e6b2fe56f8d44a0bcc2409a5c7cc80105ceb7b4fbec1a7f7d6c`。

同 SSH 的 Codex `0.156.1` 已进入真实 TUI，但五类通知均 installed=1、active=0、review=1；F2 明确五个 SessionFlags 状态键被忽略。固定官方 `b412ff32c417f855c2b2d1581b77058eed87c84b` 的 `config/src/overrides.rs` 对左侧仅 `path.split('.')`，不解析 TOML 引号，导致含 `config.toml` 的键被拆错。未手动信任 hooks，零图片和模型输入；真实 `/exit` 后 server／TUI 不可见、released=true。白名单 SHA-256 `851fe2ac991dab98fef16d71116c0bf08672feeafa5bedc6297ef93388765e58`，官方四文件源码与来源摘要已独立核验。

当前修复复用原提交的消费收据处理自动收起／恢复，仅精确 finished ACK 清原稿，取消、编辑、附件变化、旧会话和旧提交不影响新稿或新租约。Mac／Linux 原生锁单独校验私有父目录、UID、单链接、无组他写、O_NOFOLLOW、inode、精确 PID 字节及绑定生存期退出；私有 JSON 读取合同不变。Codex 将五个精确状态键放入 `-c` 右侧表，保留原生逐层 hook 状态合并，不改项目、审批、沙箱或用户配置。25 文件冻结摘要 `042eb15401a7a056f204d9d4848862b1e4d0de356c6c6cfc8756c9f522eef65a`；Mac 串行零重试 `r-61atygcd` 85 项通过且无 LEAK／FLAKY，i18n `r-csgi9402` 11 项通过，最终 `r-5ucqpf5v` cargo check 通过，三轮目录均已核验清理；汇总收据 SHA-256 `cb3f84f5571bfb39c566bfba49e0d387bcbd59b770d10a562b0f898b62321c43`。新增 Linux 专属回收测试未在 Mac 执行，仍待同源码 Linux 门禁。

无需本地化变更；原英中状态与失败提示语义已复核，修复版真实双语布局仍待验。截图仅为 CUA 工具图像，没有外置原图。App／sshd、两款原生进程均已退出，daemon 按原 10 分钟空闲期限退出，000006 finish 与外层 exit 0 已确认；旧 status 保留此前存活快照。现场保留、cleanup_ready=false，独立保全审计尚待完成。上述正例不关闭 G08 的清稿／回收、异常恢复、SSH／tmux 与最终源码门禁。

## 2026-10-01：真实启动暴露的 Grok 初始 hooks 与 Codex TUI 接线

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `400fe985f` 的独立中文 GUI `r-vpukvpd7` 通过产品菜单分别启动 Grok 和 Codex，均未提交图片或模型输入。Grok `13a09e440e29` 已到原生首页，但通知只有全局 SessionStart，插件未进入初始 registry；富输入未出现。Ctrl-G 进入原生 vi，随后关闭窗格，TUI／vi 退出而 leader 留存。Codex 此次已通过 server readiness 并保存 socket.json，但没有 tui-birth；包装器 19.418 秒后结束，server 已退出且 released=true。两份失败白名单 SHA-256 分别为 `c8793e022c77c6eb30748e2b4bdd4ada4fa89a98c19fa890bf85196c6976f286`、`caa9c4376694f0aef0180a15e59488ac1ce7c1004d227943dcc32554c8c62264`；截图仅保留在 CUA 回执，不称图片或成功布局已验。

源码核对确认 Codex helper 的 spawn 默认把三个标准描述符置 null；现对 TUI 显式继承，server 仍独立。固定官方 `b412ff32` 的 TUI 也不解析长 socket 别名，现从持久 SocketStamp 验证后传入物理路径，精确 argv 校验同步使用该路径。官方 12 文件审查摘要 `972e4b7c3078cb3fe31bcc241bfe0a22b5aebea27d591f7ef1c1850b0199e216`。Grok 宿主从成功 Reserve 登记连接所属票据，EOF／Drop 原子封闭登记并请求原票据回收；覆盖 Reserve 晚于 EOF，保持新连接、输入 Revoke、活跃 TUI 和内核身份边界。Service／Connection 发送端释放后排空回收队列，不丢弃未落盘请求。

四文件冻结 `50a6f61c43b75951510b540339212bda9e4d1f87ef930f46874a366f0eb7add1`；真实 PTY 用例先在未修复 stdio 上失败，修复后通过。整合 `r-yzvob9qe` 21 项断言通过，但 `input_revoke_does_not_request_launch_cleanup` 有一条 LEAK；外层最终未见进程、服务或打开文件并清理目录，不能据此推定 LEAK 根因。i18n 11 项和最终 cargo check 通过，汇总收据 `owned-startup-local-gates-v1.safe.json` SHA-256 `24011125c9a2e79f03e5510f9e33682073c4b0743ab78740a57b5efaa78336cb`。首次并行编辑期间因测试模块文件尚未创建而编译失败的记录保留。无需本地化变更；既有英中通知、失败语义不变，成功流程的双语布局仍待真实验收。

原生 Grok 真实 ACP session/new 红基线确认预期 10 个通知 hook 实际为 0；`a9c27a25fe22` 在 SessionStart 前装配 active plugin 文件与内联 hooks，并替换继承的 plugin 命名空间防重，`.2` 的 45 项回归与完整原生 check 通过。验证摘要 `f6bc0f379f2da6f3ac3265e20124f9fe4b07e12e476edbc53f33cfe4bb595f54`；Mac `.2` 工件已构建、严格签名和零模型启动检查并绑定宿主，SHA-256 `2e1397f1587a34297195b7ddfed6da16bee4bc0944f984a8a6401055a8ae1923`，构建收据 `ee1090dd5dbcee1e318cff0276403c2a422ec864c5520557c7e212cea2521f86`；105 文件公开补丁精确重建原生 tree `36e0a1768607c2dc72c4880df9cc97189bbf7c88`。Linux 真实 ACP 回归已加入原生源码门禁，Linux 工件尚待实际构建；不将原生正例改记为 G08 完成。

最终整合冻结21个源码／资源／来源文件，摘要 `05b40229c02e5411d64178a224194cc0f244f59cef85ba7030ca2d54cfe6f37a`。`r-47_btdcn` 串行零重试21项通过且无LEAK，日志 `1e290e2daf4209a0a85c81a0d1645217bb665abe33910f5b8e47b846a4b38f6f`；不据此解释旧并发LEAK。整合后i18n `r-yv_7vg06` 11项通过，日志 `b6289d382dc7358918e38407053dbdf16f53385566c57e4b8b2b0712f3712f1c`；最终check `r-zegvl7kg` 通过，日志 `b927881e04cad61fe44dc5b54317561b939349ce90d44beddf16cb02169a387f`。三轮短目录均已核验退出并清理；Python构建脚本4项通过。

失败现场的 Grok leader 经本轮 ticket／清单、已退出 TUI、内核完整生存期、映像和 argv 核验后仅发送一次 SIGTERM，确认原生生存期退出；收据 `3f1e85abbb6faeb1d691229533c7915ac312af454a77cd577351935b0baaa2e3`。这不是自然退出或产品回收成功，原 ticket 无 released；GUI／sshd 退出且现场保留，cleanup_ready=false。旧slot已在重新核验进程、服务、打开文件与目录身份后同设备无覆盖保全，收据 `1b4fc2158f9715401244484e64bbc10d1ba6711beae4cb35a4c12424979242b0`；物理socket和四个profile仍原样保留。旧 `r-9u32vjqt` 的 Codex 孤儿同样经精确 TERM 后退出，slot 已同设备无覆盖保全，收据 `6df231fd04a7007abd012f7572d6f8c595652d417a6ffb85f52a91ab0608c6e4`，旧失败不回填。

上一源码 `c3ff` 的 [CI 36822357366](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36822357366) 两平台作业成功；Linux host 1563／client 9／desktop 670，Windows host 1354／client 9／desktop 670。Windows `test_closing_tab_context_menu_restores_active_tab_focus` 首次 60.138 秒超时、第二次通过，不能称最终无异常门禁。13 份工件摘要与 CRC 通过，最终审计摘要 `66b553938bb1b2bc58dda5e7283ccc57c36fdeaf72d766d8b97ac7fc4f713686`；不覆盖本轮新源码。G08 的新构建图片消费、回收、SSH／tmux 异常恢复、英中实际界面和最终源码门禁继续开放。

## 2026-10-01：Codex 真实远端启动的长 socket 别名修复

**新增关闭0项，仍为6关闭／5开放／3移交。** `c3ff0bba8/r-9u32vjqt` 的中文真实 SSH 产品菜单单次启动 Codex，官方原生已成功创建94字节物理socket及121字节rendezvous别名。宿主却用过长别名调用UnixStream::connect，ready等待20秒后失败；首次bound_socket/reaper同样使用别名，原生app-server残留。没有TUI、提示提交或模型请求。白名单收据SHA-256 `ed4efbd300331e3f5633d09ba2a8316d24bd573f1b0c58cb78aa4cc60ac5a552`；CUA观察收据 `7fafbe55f7f856e99d4aadfe8df065ed1b0652e4ef7217e9c5a20b273ece5748`。截图只在CUA工具输出，没有外置原图；App与SSH正常退出，stop因原生残留拒绝，现场保留待精确回收。

修复仅将真正的连接目标改为受审物理路径，连接前后验证别名、物理socket和父目录；readiness、首次绑定/回收、shared与owned图片消费共用，peer、代次、前台组和通知资源检查保持。5文件冻结SHA-256 `30ab2d2741e35028cfc381612e8f46f34acb9bc957687156e14a873464c15332`。`r-dhli2d_1` 定向12项通过，包含长别名真实监听器读写8字节及两种替换后零连接回归，日志 `9ca3530898d9a6626c94bff5b6b6ecee83e2f0b35493b54b2739bb662a6a7a65`；`r-b0aj3u_5` i18n11项通过，日志 `4c1b3cd25619377f216c2112c52d6773af75b101d2b29845ad3580d46844abd4`，两轮短目录已清理。最终check `r-uh0w0jy3` 通过，日志SHA-256 `0734ca67fa982dd062c0d2b9d25070241a55503ec9bffbb8cf1a651cd2950643`，短目录已清理。无需本地化变更；中文菜单/错误可见不代表成功布局、SessionStart或图片消费通过，修复版真实验收和新源码跨平台门禁仍待完成。

Grok原生通知及宿主整合已提交推送为 `73d5a34c8c4a20debf4e8ed7eb623fe9cb203525`。此前Claude `r-u10vfmtu` 全部自有进程退出后，slot已同设备无覆盖保全到本轮 `retired-remote-server`，收据SHA-256 `125f8383eae124bf55c21bce93b86acc4a1670be7a417a9da1fbff2eaf7b023c`；历史/profile仍保留，cleanup_ready=false。

## 2026-10-01：Claude `c3ff` 英文 SSH 复验与 Grok 通知工件整合

**新增关闭0项，仍为6关闭／5开放／3移交，PR #22保持草稿。** `c3ff0bba80a65acf9eb0fe090b54783a71d4f80d` 的签名 Mac GUI `r-u10vfmtu` 在实际 SSH 普通 Claude `2.1.280/claude-opus-5-5` 中，首图通过系统选择器成卡、一次提交和原生 Read AllowOnce；typed PNG与规范上传 PNG均为328字节、SHA-256 `40e362f2828d701266b69c824f700610ca07e8cd1855fcdcea10462ba4597b2d`，回答 `Red, Blue`。宿主和服务端confirmed/释放/完成一致，自动收起后重开输入区已空、卡片为零，没有手动清稿。原137字节夹具经过PNG规范化，不能称原编码字节未变。

第二次选择同图并单次提交，在原生Read选择No后得到错误结果且typed图片为零，重新打开仍保留原中文草稿和单卡；没有再提交。真正执行 `/exit` 后，原生进程消失，宿主及服务端均保存retired，宿主release/done为true，发布图片已删除；退休仅回收文件，不确认消费。退出原生后重连同SSH且未启动新CLI，两笔提交、宿主和服务端记录及原生历史均未变化，请求各一次。这一组合不覆盖活跃消费中断线、拒绝后重投、其他CLI、tmux或所有旧回调场景。

本轮英文界面由CUA实际观察，中文未验；截图仅存在工具回执，没有归档外置原图，消费Toast未观察到，不能声称Toast验收。签名应用SHA-256 `15fbed9977f6909db881051c161ae3fcfe0a5740694a2ebd68616eee05873756`。归档目录 `resume-20261001/g08-preparation/r-u10vfmtu-claude-ssh-gui` 中的原件摘要：

| 原件 | SHA-256 |
| --- | --- |
| `whitelist-first-consumed.safe.json` | `0f33428e316c16437051f84a1bbe0c30e2dbe77740e599e318d8e90fc06ff498` |
| `whitelist-second-denied.safe.json` | `8398225d4ac4c147107eda1b25763b72fefc196f58cb66f66381dbae6c059d0f` |
| `whitelist-second-native-exited.safe.json` | `f75671d8d63036e973ffc34eecf8e6ec02ccb202f4dff4b323b9396567e0b2ef` |
| `whitelist-ssh-reconnected.safe.json` | `216fd50a7f2d89930ce2f12c5255a0d95e2327ae2f522aee06735f2e814a255c` |
| `whitelist-reconnect-comparison.safe.json` | `e81be712bb7bb1da3bc456b5a6e6c5ff27aa874b8d6c054e1407c872373b3ff8` |
| `cua-observations.safe.json` | `72bd663e7f06b6fc467e04f85701b2638711e42efd8ab2f895013a5c61d2dc99` |

App和sshd均自然exit 0；早期状态收据列出的remote daemon两个进程，在宽限等待后的 `finish.safe.json` 中已确认为全部自有进程退出，收据SHA-256 `5c0ed5a9708225984cce2f0ebf66dfec6637cbaf312279ea6352e931dfe0545b`。外层 exit 0、日志SHA-256 `f20728f827a7fd127aace6eace92ebd87aea5ae6cc12ce504d500e0b01afdb44`，状态completed/retained，cleanup_ready=false，现场和历史保留，不改称已清理。[CI 36822357366](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36822357366) 本次记录时仍运行，只绑定c3ff，不覆盖后续Grok整合。旧367源码运行36814148707的Windows桌面LEAK继续保留，不回填为无异常门禁。

Grok原生通知增量已提交为 `13a09e440e297bf52b8a0941b7af5bf42e882013`。固定公开基线 `07e35a3dfeed2f200d319ef6c893b5ea286d9a51` 加独立补丁SHA-256 `2bca441da55912b1f463e732e8a0889bb505c2706941993ee055cc8f9aaf1019`，通过归档隔离索引精确重建tree `e8537e6cf1b788320882c4d65eb6dbf9d429da0a`，104个差异文件逐一取摘要，实际工作索引不变。来源见[新来源清单](../../native/grok-build/session-notifications-source.json)。最终原生 `r-ihrmtr2u` 通知44项（hooks4/pager1/shell39）与check通过，日志SHA-256 `f03082ab7d0a1c77a7f9e87bb122b6a158aa45dcab1f275729d917dbf5abb691`，安全收据 `08f456883034c8c7175a43bf98debc4d8f6b333c3f497a1519200005f0480ba6`。旧 `r-u0rvam6k` 仍为73通过/1个旧namespace金样失败；修正后 `r-1hv5iwt5` 单项及check通过，不合并成最终源码一次74项通过。

Mac原生 `r-_v2vq8kh` 构建、codesign严格校验、`--version`、`--help`、真实async completions均通过，零模型输入；完整版本为 `grok 1.0.41+infinishell.session-notifications.1 (13a09e440e29)`，工件SHA-256 `0f3d4aa695e1d596c120be296dc68d1942dacc8c2a6320dc887cda127d0ea3f0`，CDHash `95444e4ea5436ac5bba72febc3f698fceaad3806`，构建收据SHA-256 `ccb9493f1f0b1a558968758733fa8564c5666548699545ca44717ef94b7e27e1`。这些原生轮次短目录均已清理，仅Mac远端owned入口绑定新工件，旧G01来源/工件/tb1保持独立，Linux通知工件仍为None。

18个宿主/CI/来源文件已经整合主树但未提交，冻结 `g08-build-preparation/host-source-freeze-v1.safe.json` SHA-256 `ff79d75fc90d455e86f60ea250219bf015ad868318d1e6e254655a43d4908aad`。`r-qbicqrss` 的 `cargo +1.92.0 check --locked -p warp --features warpui/test-util,rust-embed/debug-embed` 通过，日志 `fc1b7b9f6d2971efeece3eec5f832b4a58462608d762e8900d030b2d5b3b4d24`。随后 `r-tq_tvb8i` 同特性、零重试定向nextest为67跑/54过/13失败，日志 `c46b0aaeebb11c614d6d4a1a5848e25765c0abcff155df1432982010a0af2605`，exit100且已清理。12项在NotificationPlan::create、1项在TicketStore::new失败；锁定tempfile 3.23的目录默认权限为 `0777 & umask`，本轮得到 `0755`，不满足私有 `0700` 要求。根代理仅把三份测试文件的六处新夹具创建改为 `tempfile::Builder.permissions(0700).tempdir`，生产代码未变，没有修改既有目录权限。`r-nc7llcn1` 单独复验新增及版本关联 14 项全过，日志SHA-256 `048a32e06604b3c64a2df5194ab031098902d61fcd1ab852a59ec784b86a8b69`，exit 0且已清理；没有重复其余54个已通过项，不将新结果回填为原运行67项全过。i18n `r-zleu1c4y` 11项通过，日志SHA-256 `14dd1e18bff05840bd0deac554f183cfca96ed16a34b8b21c2e9da4421bafb4a`，exit 0且已清理；最终check `r-vbxjs18d` 通过，日志SHA-256 `2b84ae77fe1b0127a7eef29dd24292a7470a69c6718d813d3d66ca830ec18d27`，exit 0且已清理。最终 `host-source-freeze-v2.safe.json` SHA-256 `fe3f747818dc9de3b5665e2e7288ecc4f81f9291675abafa1b820fa9fe65093e`，相对v1仅三份测试夹具文件变化；这些本地门禁不替代新增量跨平台源码门禁和真实验收。Grok两组英中文案已同步，真实双语布局未验；实际Grok pager→resident sidecar→typed PNG→通知归属链仍待验证，G08保持开放。

## 历史记录：2026-10-01 G08 清稿、退休响应与 Codex 会话通知源码门禁

**2026-10-01 清稿、退休响应与 Codex 通知接线修复，真实复验待完成**：Claude 自动收起/恢复后仅保留原提交的只读清稿凭证，写租约仍撤销；原笔回调按 generation 与 submission ID 释放租约，不能清新稿或释放新提交。客户端接受原消费者已退出的 retired 回执用于回收，不把退休当成消费。Codex 专属入口冻结五类通知、八份脚本及 SessionFlags，app-server/TUI 使用相同固定参数，不改用户 HOME、CODEX_HOME 或审批配置。客户端 9 项、本机 app 原轮 229 项通过；10 项新连接绑定用例因共享夹具遗漏真实 BlockMetadata 而失败，修正夹具后 10 项独立通过，原失败保留。i18n 11 项及最终 `cargo check -p warp` 通过；收据 `g08-ui-client-local-gates-v1.safe.json` SHA-256 `61e792488c73bcc94bc836f72d49945962fbc32f823c7929d080bf76e9cb007f`，短目录已清理。无需本地化变更；新构建真实英中 GUI、Codex/Grok、tmux 和异常链仍待验，新增关闭 0 项。

Grok 公开源码通知扩展在隔离工作区通过 Mac 通知定向 41 项；原原子桥回归 73 项通过，1 项旧命名空间金样已修并等待单项/最终检查。Windows 静态审查定位并修正仅继承 ACL 条目的误拒，实际 Windows 测试仍待运行。该原生增量尚未绑定工件或进入本 PR，不能算 G08 关闭。


本报告保留截至 2026-10-01 的固定版本验收结论、源码来源和未验边界，不保存逐轮日志或截图。**这是阶段交付，完整 Goal 尚未完成。** 当前状态以 [CURRENT_STATUS](CURRENT_STATUS.json) 为准；剩余功能、优先级和关闭条件统一维护在 [KNOWN_GAPS](KNOWN_GAPS.md)。

## 历史记录：2026-10-01 G08 消费和文件释放已实证，界面清稿及客户端退休状态仍需修复

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** 精确提交 `367279c7aa97df1a85689602115a119f4173e7cc` 的新 Mac 构建、独立签名 GUI 在真实 SSH 普通 Claude `2.1.280/claude-opus-5-5` 中，首图经本文件 Read 允许后返回 typed PNG，与规范上传 PNG 同为 328 字节且摘要相同，回答 `Red, Blue`。服务端和宿主均记录 confirmed／释放／完成，远端图片已删除；自动收起后重新打开仍显示原稿和卡片，证明这是界面清稿缺陷，不能再归因为消费解析失败。

第二图 Read 选 No，原生结果报错且无图片数据，没有替代读取。第三次相同 Return 没有新增原生请求，但 CLI 等待守卫也会拦截，不能单独证明持久 Unknown 防重。菜单粘贴 `/exit` 首次被该守卫拒绝；后续实际原生键入 `/exit`、PID 消失才证明退出。服务端由精确生存期产生 retired 并释放图片，客户端却把它拒为 UnexpectedResponse；宿主状态仍空、done=false，实际退出 SSH 并重连后也未恢复。两处缺陷分别补独立消费清稿凭证和仅 Claude 的 retired 响应校验；退休仍不等于消费成功，也不清草稿。

独立业务审计 `resume-20261001/g08-preparation/r-caeh0l9f-claude-ssh-gui/acceptance-review.safe.json` SHA-256 `414055d94ab25cf1c86b9426f07ba474bb27a0f35e282af0dd99fbb17c73d470`。自然退出后，精确进程、打开文件及服务归属核验完成，remote slot 通过同设备无覆盖重命名保存在本轮目录；收据 `56824240b737c503acf5c3b9219ebf8b65cfabf27eb534c00468055caaacda51`。唯一历史及 profile 保留，cleanup_ready=false，不导出完整历史或凭据。审计辅助脚本沿用旧文件名导致的首次失败和服务分类前的首次拒绝均保留，不覆写原件。

客户端定向回归 `r-viza9eze` 为 9／9 首次通过，exit 0、目录已清理，日志 SHA-256 `a241609f639771868ead2f9757a488ce219f7393bc702ba350481d0e4933a3e1`。界面修复首次 check `r-22gsv0t7` exit 101，捕获跨线程 Rc，日志 `40b792f1ff22b102822c51d7b6b033413389ba2520930daa52f7d5b23cc748ab`；改为必要修订号并补同代次旧回执精确释放后仍待新门禁。现有英中文案语义无需变更；真实新构建清稿、退休回收和双语界面尚未复验。

[367279c7a 的 CI 36814148707](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36814148707) 两作业 success。Linux main 1531、新 23 项，Windows main 1331、新 4 项均首次通过；Windows desktop/TUI 670 中有 1 条 LEAK，具体为 `terminal::input::tests::test_ai_context_menu_closes_when_space_immediately_after_at_symbol`，不是最终无异常门禁。13 份 ZIP 均通过 GitHub digest／CRC 独立核验，最终审计 `44dc32031d75a115f8429d08090eb08d883d071d101629cde15b435c840d838c`。本轮原生专项跳过独立 remote_server 客户端组，下轮设 native_acceptance_only=false。Linux TMP 因打开引用可见性不足保留，不用清理拒绝代替功能结论。

## 2026-10-01：G08 真实 SSH 首图读取成功，消费关联及异常回收修复待复验

**新增关闭 0 项，仍为 6 关闭／5 开放／3 移交，PR #22 草稿。** `e8233ac67e0382d388f806d3e3aba10a4b20953f` 的独立新 Mac 构建及签名 GUI，在真实 SSH 普通 Claude `2.1.280` 中完成一次首图提交：新 SID 起始无 `<SID>.jsonl`，原生自行创建历史，仅本文件 Read 审批后返回 typed PNG，328 字节与上传规范 PNG 完全相等，最终答案 `Red, Blue`。原始 137 字节 PNG 被产品重新编码，已有像素相同证明；不混同原编码与规范 PNG 的字节一致性。选定请求／祖先关系、Read、结果摘要和最终答案收据为仓外 `resume-20261001/g08-preparation/r-p7h47_62-claude-ssh-gui/first-image-native-consumption.safe.json`，SHA-256 `c66dd5eb6c92da18425d376396e29f3c1e475c7789908f91b8af7d56508fdf6d`；没有导出完整历史、系统提示、隐藏推理或凭据。

该实测发现消费解析只接收 user／assistant，真实请求至 Read 的四个原生 `attachment` 因而断链，服务端未产生确认或回收记录。草稿和卡片保留是独立观察，延迟回执本来也不能清除已切换代次的草稿。另在旧 142c 首图失败现场确认普通 Claude 没有可靠退出回收，第二次相同图片／正文被防重挡在宿主领取之前；旧截图不能当成第二次原生预检失败。

本轮精准修复仅让同 SID、非 sidechain 的 `attachment` 连接祖先图，其内容不提供任何工具或图片证明，`system` 不扩围；同时把首个原生 write 之前可证明的失败持久为拒绝，写入开始后仍为 Unknown。新提交保存独立的只读原生生存期，Linux 还固定观察者 PID namespace 与 `/proc` 根身份；原消费者明确结束才退休图片引用。退休不确认消费、不解锁同 SID／正文、不清草稿，旧无生存期记录不补猜身份。既有英中消费／未确认／远程 Read 提示的语义已复核，**无需本地化变更**。Mac check、相关 nextest 136／136（新增 22 项）、i18n 11／11 均通过；成功门禁的短 TMPDIR 均已清理，逐名 PASS 无 FAIL／LEAK／FLAKY／重试。收据 `resume-20261001/g08-preparation/g08-consumption-release-local-gates-v2.safe.json` SHA-256 `542d123c414997aa4cf8fa599c2fc4a8374953f8279d97910f376953da83b624`。首轮闭包生命周期编译失败及测试 Command 导入失败均保留原日志，分别精准修正后通过；其后仅 rustfmt 合并闭包换行，逆变换与测试源码摘要一致，最终 check 再通过；没有修无关警告。新构建真实回收、双语布局、Codex／Grok、SSH／tmux 异常组合和本轮跨平台源码门禁尚未完成。

前一提交 `e8233ac67` 的 [Linux／Windows 源码门禁 36808124672](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36808124672) 已完成：Linux 主组 1508／1508，新增首图 20 项逐名首次 PASS，i18n 11；Windows 主组 1327／1327、桌面／TUI 670／670、i18n 11。两平台完整日志无 FAIL／LEAK／FLAKY／重试，13 份小工件与 GitHub 摘要一致；审计收据 `resume-20261001/g08-preparation/ci-run-36808124672-e8233ac67/audit.safe.json` SHA-256 `f240cc955b1234086d3490f2bbc0e53876c4505f08ab3fa823286e7dd5f07b35`。独立 remote_server 图片客户端步骤、Linux 桌面／TUI、全量工作区及认证 GUI 均跳过；不外推为本轮新修复通过。

## 2026-10-01 历史：G08 Claude 首图历史延迟创建实现，缺口继续开放

本增量新增关闭 0 项，累计仍为 6 关闭／5 开放／3 移交，PR #22 保持草稿。Claude 首图不再要求原生提前创建历史；Pending 领取保存既存目录锚，首个原生 inode 绑定须持久化后才能按既有 Read／typed image 原字节证明确认。原 claim 不改写，旧格式不降级为 Pending，未知提交不重投。首次观察长度只提供长度下限，不能证明所有离线截断后重写。

Mac `cargo check -p warp --features warpui/test-util,rust-embed/debug-embed` 通过；同特性下定向 nextest 114／114（含新增 20 项），`cargo test -p warp --lib i18n::tests` 11／11。三轮短 TMPDIR 均 exit 0、cleaned、cleanup_ready；源码 11 文件摘要与完整命令绑定在仓外 `resume-20261001/g08-preparation/g08-first-image-local-gates.safe.json`，SHA-256 `a92596f30bfd4ccbff12adc463dd6cfda69cc5d0ade7cdd1d76be5e64ab174dd`。最初 check 因遗漏 `Component` 导入失败，原记录保留，修正后重跑通过。既有英中远端图片提示、消费和未确认语义已复核，无需本地化变更。

真实 SSH 首图正在独立旧 142c 构建现场验收，当前不计修复前复现或新实现成功；新构建、双语布局、SSH／tmux 三款消费与异常恢复，以及本次 Linux／Windows 源码门禁尚未完成。以下 G01／G07 关闭记录保持各自原来源，不外推为本增量已经通过。

## 2026-10-01：G01／G07 本次 Mac 范围关闭

**本轮关闭 G01、G07，累计 6 关闭／5 开放／3 移交。** 仍开放 G03、G08、G09、G10、V03；V01／V02／V05 的其他平台实机验收移交用户，未改记通过。PR #22 保持草稿，完整 Goal 尚未完成。

G01 按各自原构建合并计证：Mac 定制 `.3` 普通标准首页八条真实输入覆盖中文 400 行长文本、草稿／审批保护、编辑竞态、明确同文新轮及同会话重启不重投；后续英文／中文粘贴各一次明确提交和最终中文提示布局通过。Linux `.6`、Windows `.11` 的原生实现、独立工件及宿主摘要绑定已补齐。G07 的三款固定 CLI 普通 GUI 分别完成中文／英文空格路径卡片、原生实际读取和独立审批拒绝；失效／不可读文件保稿，以及 `142c635412` 共用 Toast 英中完整布局已满足。文件卡语义为本地路径引用，实际模型证据为 UTF-8 文本，不承诺任意二进制解释。各原件、版本与来源见下文和 `CURRENT_STATUS.json.current_goal_scope`，没有重标为本轮重新运行的 GUI。

精确源码 `e3d5e58a05153d4e7fabdc1fc6528c8c96c598d7` 的 [最终门禁 36796894945](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36796894945) 两平台完整成功。独立审计逐名核验账本模块各 19 项，补齐旧筛选漏测；Linux host 1488／1488、command 14／14，Windows host 1327／1327、command 36／36、真实原 ConPTY 1／1、桌面 670／670，均无本轮 FAIL／LEAK／FLAKY／重试。G07 附件 Linux 5／Windows 4、文件提交各 6、i18n 各 11 个唯一用例均通过，Windows Toast 10 项通过；组之间有重叠，不累加为独立总数。Linux 本轮桌面按输入跳过，Toast 10 项沿用 `142c635412` 相同产品源码的原记录，未宣称在 `e3` 重跑。其后 `5569bd99b` 仅三份支持文档变化，本次也只更新验收状态，产品源码、Fluent 及 workflow 与已验来源一致。

最终归档为仓外 `resume-20260930/g01-cross-platform/ci-36796894945/terminal-final-audit/run-final.safe.json`，SHA-256 `8ab217095271a6bebe6f39621a8a0f4b8ac973ccf31128d39aa30d20a3436955`；13 个工件 ZIP 均与 GitHub API 摘要一致，56 份原始日志逐文件建索引。Linux／Windows 逐名审计摘要分别为 `565173db6530bf749a5735bd5c1740e898bff57132324ae4244b661538bc4dbc`、`efb84452ec5813a295777853dd0c004ef5822a2fa66d2458dc3e9f5936bafd75`；完整 job 日志分别为 `7e0bf6a45d3c7890a1130c0b52d8d756d02471965d9ca0080d556dd787801531`、`0efc4150821a628f0638af243d0ff3ee065572096b4dc141dd03d10b8362b1a8`。主代理复核当前测试源码摘要与逐名覆盖，不只依据绿色 job。

本轮 Windows Claude 两个真实 ConPTY 场景分别在 1.747／1.408 秒取得唯一匹配通知，两份原 PTY 已保存，原生 exit 0、Job 空且未强制清理；独立审计摘要 `ba1e433d96154f5dc5474245809e90898559060387d6665ea6b681be1d5ce6a6`。旧 `36789334970` 已终结为失败：首场景 45 秒没有有效匹配通知且原 PTY 缺失，其根因仍不明；旧账本漏测及 Windows `test_vim_escape_with_history_menu` 的 LEAK 均保留，不能由新一轮成功改写。固定 Codex Hook／ConPTY 各 8 条关闭记录通过，Hook 一代有界回收后代，不称全部自然退出。

Linux TMP 因 `/proc` 可见性不足、Claude Mac 现场因共享服务归属不明继续 `cleanup_ready=false` 并保留；未终止共享服务或宣称目录已删除。这些保守留存不改变已有功能结论。此次状态收口无需本地化变更；功能变更的英中审计与 i18n 门禁已按上述来源完成。以下逐轮记录保留当时判断，当前关闭状态以本节为准。

## 2026-10-01 历史：G07 Claude 双语及 Codex 原生拒绝补验，最终门禁当时未满足

**新增关闭 0 项：4 关闭／7 开放／3 移交，G01／G07 仍开放，PR #22 保持草稿。** 新 Mac GUI 实际构建来源 `142c635412a21e92161111fba5148b6583c8334a`，源二进制 SHA-256 `afd13b4b69e596a51b5fbd52c630e9b05cb012e4993e4616db3ec48eb40f5f48`，构建绑定 `1fc0d81765fb28a83dd0c488330bd6332cae2a2b46d93e1f23e6e873a838c03f`。下述新证据位于仓外 `resume-20260930/g07-preparation`；旧 Grok `.3` 与固定 Codex 双卡正例继续按原构建计证，不重标为本次重跑。

Claude `2.1.280` 的 `r-9g0qj9u3-claude-gui` 完成普通 GUI 双卡单次输入：仅传原绝对路径 JSON、无夹具标记，两次真实 `Read` 原字节结果与两行精确答案匹配；正例 `positive.safe.json` SHA-256 `2409ca91302d78babc95c2a462247bb54f769efc401dc2f9109d6d48f78b942b`。随后独立一次 Read 审批 No，精确 tool ID 的 `user-rejected`／`User rejected tool use` 与原生 interruption 对齐，零成功内容／替代工具／拒绝文件标记泄露；没有 `READ_DENIED` 模型回复，也不冒称正常 `end_turn` 或 managed ACK。拒绝 `refusal.safe.json` 摘要 `21134913098a79a2d4c00db5556b8a35106a4a2f3a8161862ba8d59e3b04143a`。

同一新构建的英文与简体中文分别在双卡形成后删除长文件名目标再提交：完整错误说明和文件名分两行可见、无裁切／省略，原草稿及双卡保留，持久活动零新增。`bilingual-layout.safe.json` SHA-256 `bb2f4cf0123bab36b0c65a9dca596d6eb9e108a4faafd0694a43ca6f743ae073`，对应第 09／10、18／19 图；未声称读取编辑器原始缓冲区、测量 PTY 字节或在该轮运行 mode 000。共用 Toast 高度修复完成英中真实布局审计，旧失败原件保留。Claude 索引 `acceptance-index.safe.json` 为 `393fcc8081f99bb25628f973909b5fa1480fed4c9d267c4519d46adf6ff4284b`；两代 App 均自然 exit 0 且自有进程已退出，但 System Events／FolderActionsDispatcher 的共享归属不满足清理条件，**`cleanup_ready=false`、短目录保留**。不终止共享服务、不豁免门禁；`cleanup-deferred.safe.json` 为 `46bee2e555e68fccea99530b1701f4787c6b8fa9828bdfe906dd71b7ea6d0249`。

固定 Codex `0.156.1` 的 `r-__g9v72o-codex-gui` 保持真实 HOME／CODEX_HOME，仅操作本轮私有项目。一张中文空格路径卡、一次输入，真实审批显示精确 `/bin/cat` 因策略需要批准，操作者选择 No。唯一 cwd 定位的 SID `01a0f4ad-fb08-7652-b426-fa605a93342d` 中，同 call 拒绝输出 `aborted by user after 42.5s` 与同 turn 的 `turn_aborted/interrupted`，结合第 08／09／10 图的审批、No 与取消终态，证明原生拒绝；零成功读取、替代命令、重投、第二输入和标记泄露。UI “Ran” 标签不证明执行，未出现 `task_complete` 或模型拒绝终答；输入前 SID 尚未落盘，仅记录精确 cwd 索引为空，不宣称原生全历史零输入。独立复核 `codex-denial-reviewed.safe.json` SHA-256 `92cf37d1ae5da83a762fd32e0762df5a6db36129510559d2f91083932a7b248f`，四条相关原始行 `codex-denial-native-selected.jsonl` 为 `210799d18e6f33bd33ea4a771e97020942c46bb4f7c18d987b53612149b55eeb`；不导出其他会话、完整 system／reasoning。28 文件索引 `9ac093a4beb0dcbbdec65868381ace2ec9c9430380472c9410a8b19b5db52271`，外层自然 exit 0、`cleaned`，日志 `830882ccff904e12dbb1d899082fedb3d0fae96775e724f5a7c465b9ae5ca2b9`。

[最终 CI 36789334970](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36789334970) 仍进行中，不能计整轮通过。Windows Claude 第 0 场景已启动原生，但 45 秒内未取得有效匹配的 SessionStart；第 1 场景成功。旧循环会吞掉解析／匹配异常，原 PTY 仅留字节数和摘要且私有目录已删除，不能断言零 OSC 或归因于 Grok。原通知收据 `g01-cross-platform/ci-36789334970/claude-windows-notifications/contents/claude-windows-notifications.json` SHA-256 `91e5b5c8eaae98311b9b99940a28d5fe9111ed290baa77ee82b3374b56002379`；最小原 PTY 保全改动仅在 Job 已空后导出本轮两份有界原件并绑定字节／SHA，失败保留现场，不延时、不重试、不降低匹配，**诊断改动不等于通知故障修复**。

Linux 作业已 completed／success，独立逐名审计确认普通桥 36、host 1469、desktop 670 项真实通过，但漏跑账本 19 项；审计 SHA-256 `6df4725f9c5b54b009f587cf5334ef9963586ba5743aef3d91097063de5d0e79`，完整 job 日志 `7e194f1d5aa9c1c7f9bbed4f98276b628f7622dfcd7d8fab201a0b437891fab8`。Windows 仍在 host 测试。两平台 host 筛选将 `local_cli_tasks_grok_native_bridge` 文件名当模块，已替换为 `persistence::local_cli_tasks::grok_native_bridge::tests::`，不能回填其跨平台覆盖。实际应用证据脚本改动后的本机门禁：check `r-y9abd9lq` 日志 SHA-256 `fe5279323daf61303e907cef32149304d678622844e27391e6fffea8cc90db77`；Python 27／27，`r-y1k4zzq7` 日志 `cb44ebee5ee84c93a3c582611a8b52d8d1f8ea417525ed8bd60ff150428f54f8`；精确账本 nextest 19／19、retries 0、8243 skipped，`r-g3gcq1gy` 日志 `d2d2d8b78590c28041f1e8a24ce2b112bdc7ca23a0c9be8e3601d5bf98153b1f`。三轮均 exit 0 且已清理；修正筛选后的 actionlint `r-gtjqciby` 亦 exit 0 且已清理，空日志 SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`。仅证据脚本／测试／CI 筛选变化，无需本地化变更。G01／G07 仍等待精确最终源码门禁，Claude 现场另按归属证据收尾；其他平台实机移交不替代实现或源码门禁。

以下同日记录保留当时结果及待验边界；最新补验以上节为准，不把历史失败改写为通过。

## 2026-10-01：G07 短两行提示实窗仍裁切，修正高度约束待复验

**新增关闭 0 项，仍为 4／7／3。** `8f860ed43506f02b1cd713acc1f5c0233fc148ec` 的实际新 Mac 构建源映像 SHA-256 `462b03876135ef2c44526ca3a348ea57758d60eed3467913236685621d47817d`、构建绑定 `1a052d73354e1ff34021450d59d4485afb060f60b6ad6292b4f741a405a184f1`。`r-0zfrzk4y` 通过真实选择器建立两卡，再删除长文件名目标；68 字节草稿及两卡保留，原生／宿主 14 项计数与增量均为 0，历史／桥账本字节与宿主逻辑行不变。英文第 07 图仍将两行文本拼为单行，文件名被渐隐裁切，**布局失败，未继续计中文通过**。

根因定位到 Toast 对未提供展开入口的短文本也施加两行高度上限；macOS 排版在剩余高度不足时将剩余文本交给单行排版并渐隐。仅对真正需要折叠的消息保留高度上限，让短提示按自然高度换行；长消息折叠／展开与共用配色不变。英中 Fluent 文案语义复核保持，仍需新构建英中实窗审核，不能用字符数或纯单测代替。

仓外 `g07-preparation/r-0zfrzk4y-grok-gui` 的 after 收据 SHA-256 `217b76b44db738e643845d2b6921615eb88f9b20cf604fb113f90ba46e498c9f`，失败证据索引 `8b113f8a550e2dd491dc4c45dbd4a1cba5bf7ed99bd883e9b0b985688faac04e`。GUI／原生自然退出、stdout EOF、无自有存活进程；finish 在认证副本身份门禁拒绝，停止本轮待机 launcher 后外层 exit -2、`cleanup_ready=false`，私有现场保留。外层日志 `c18b79f2305e34c4851b2a055560cb9f267c8d1958fda3e61f28d31541918914`，不称清理通过。旧 Grok 正例和原生拒绝证据不重复运行。

本次渲染修复本地门禁已通过：check `r-6kzgj8ye` 日志 SHA-256 `fff1543a1e69a9c09369563a0b95bf7d7349bb2bd3ea46e006f15d36d0f9e5bd`；i18n 11／11，`r-4c9b5yoe` 日志 `408c5b0265648f2099ef9ada974b114557c08f9108d0b675af7c87e7e2b389e9`；Toast 10、附件 5、提交 6 共 21／21，`r-iovr_hyv` 日志 `d4b2c6fe62f72dc24b93920ef50126f3bcca4ed08a008b9494f8b729deb0eb34`，retries 0。三轮 exit 0、短目录已清理。现有 Linux／Windows desktop 门禁加入 Toast 同组，actionlint 通过（仅忽略既有 `infinishell-ci` 标签声明告警）。无需新增本地化文案，英中内容保留；实际布局仍待复验。

## 2026-10-01：G01 Windows `.11` 可用工件绑定，最终宿主门禁待验

**新增关闭 0 项，保持 4 关闭／7 开放／3 移交，PR #22 草稿。** [运行 36780256382](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36780256382) 精确绑定宿主 `8e1f1fcaf5a07500c5cbd19c4ee37fa8fcbbed66`。Windows 原生 check／build 与全部 21 条命令通过，四库 53／20／4／4 共 81 项真实执行；protobuf 6 项、登录解析 2 项、资源整理 2 项及管道名称上界 1 项通过。此前被 Unix cfg 排除的四个终端守卫测试本次在 Windows 真实执行，不能追记旧 `.10` 为 81 项。PE 栈 reserve 为 8 MiB、commit 为 4 KiB，实际 `--version`、`--help` 及进入异步初始化的 `completions bash` 均成功，零模型输入。

受审来源为 native commit `77d8004b18b5d7c7f7ced42f084dbe5a54048a23`，tree `52afcf3ebad44daaef887cbe2cfe3ed003d6d44e`，补丁 SHA-256 `7690e16a7bd42fcdd591d453acb6434571c92b45561139b87eb31cb2d20fa408`。原生版本输出 `grok 1.0.41+infinishell.terminal-bridge.11 (07e35a3dfeed)` 的括号保留公开基线标识，不冒充 native commit。Windows 映像 245,529,600 字节、SHA-256 `acb9a34e9371285e1d5ce5f932dc4697e54927aea366048d361675075d4dbefc`，已绑定宿主身份校验；独立命名副本位于仓外 `g01-cross-platform/windows-terminal-bridge.11/grok.exe`。原 ZIP 与 GitHub 元数据摘要 `3e02067603835a136644cac4c3ef302db9f3042dbd59fcf0f996660173a1f06a` 相同，独立逐日志／源码／PE 核验收据 `windows-native-verified.safe.json` SHA-256 `01c01f12aff6be766393242f2477c855a55b1afeedc9032e5a3f4f653251d061`，原构建收据 `c27e5f47743c5051c1b1c5715e693abb069f39581c9e52a0204611093e0b7bfb`。

构建和上传步骤成功、工件独立归档后主动取消剩余重复验证；**整轮为 completed／cancelled，不能称 workflow PASS**。终态收据 `ci-36780256382/terminal-final-audit/run-final.safe.json` SHA-256 `90fcfcf6aef3a5b6ea25ccb1f2a494e61196482741ff67af4d062e6479e5d062`，整轮原始日志 ZIP `4813b50d6aa9bf13e9b0d53301c3f4c5a8e4ff8fd2d434fe4bb26121d60ae703`。额外符号链接资源测试仍是 Win32 1314 的 `deferred_not_passed`，旧 `.10` 栈溢出与此前失败不改写。

同轮固定 Codex Hook／ConPTY 原件各有 8 条关闭记录，共 16 条，根均自然 exit 0、stdout／stderr 真实 EOF、创建即严格 Job 归属、Job 清空且关闭；Hook 一代在宽限后有界回收 5 个自有后代，不称所有后代自然退出。ConPTY 8 代均无强制后代，4 条真实 OSC 与原始 RPC 中 LF／CRLF、路径和正文逐字相符。Hook 私有根与 ConPTY cases 清理完成，加载 DLL 的宿主目录明确保留；不推断旧 WinError32 根因已解决。独立审计 `fixed-codex-cleanup-independent-review.safe.json` SHA-256 `9b7aa445275da7638e41d6a152d5c92931f6e1c03d137949291370459f770e81`。

Mac 保留已验 `.3`、Linux 保留已验 `.6`，本次 Windows 摘要绑定无需本地化变更。G01 的 Mac 功能与双语条件已满足，三平台实现及工件已补；剩余为绑定后的精确提交 Linux／Windows host 门禁。既有 `run_grok_native_bridge_host=true`、`run_grok_native_bridge_source=false` 可复用原生工件；须设 `native_acceptance_only=false` 覆盖 Linux 桌面／TUI，并实际核对完整 local_tty、Windows stale environment 回归、同提交主程序／SSH helper 及原 ConPTY 验证。源码门禁与其他平台实机边界继续分开，当前不关闭 G01。

本轮绑定与英中文案修改后的 Mac `cargo check --locked -p warp --features warpui/test-util,rust-embed/debug-embed` 已通过（`r-4ezwcxsq`，日志 SHA-256 `95a2caaa02e8f26804a3c06d61cfe55bb30365a8eec778cbf490a7e174d3aea9`），i18n 11／11 通过（`r-2igyr7c_`，日志 `6fc879c83db7f04c7967ef11e4ccd831147e6e2d4a72ed850b3dd318f51f9314`）；两轮 exit 0、`cleanup_ready=true` 且短目录已清理。普通桥与持久化 55 项、文件附件 5 项在 `r-xcknnh99` 共 60／60 首次通过（日志 `75c3599fd2fd1cc3ba075ed84d1bb13a46296acefa0ed513573f7d45b035d895`）；首个筛选未覆盖带中间 tests 模块的提交测试，另用精确筛选在 `r-gjgn894d` 补齐 6／6（日志 `a0555d0fe32dec8cb7354f7aa5ac8229ab7c0b7fe6c8b5788bd6be6487b57ce6`），均 retries 0、exit 0 并清理；新 GUI 构建／布局和最终跨平台源码门禁待完成。

## 2026-10-01：G07 普通 Grok 双文件与拒绝通过，Toast 新布局及 Claude／Codex 剩余链待验

Mac 独立签名 GUI `r-i4txg6p2` 使用构建提交 `582596a090f9b0042ccc349fc9568e3fe072e6d1`，源二进制 SHA-256 `255fd673d78ab939d55a9156c22b2eb597a1605ec1725e5770af01000cde7206`，签名后二进制 `15cef3f5b4ad66d7932a6dc1de8935dec8f6144bd8f823217885c376cbea316c`。后续 `8e1f1fcaf` 为只读改动审查，**没有在该 HEAD 重建 GUI**；绑定收据 `e7cfb6839f3a4ae76b3618adb185f26e0002f7ea08216fa5b7e3dc353e6cef65`，后续审查归档 `a301433cce743a5d25e8eb05db077c2cabaf371c365460e7c87616561bbfab86`。普通 shell 启动固定 Mac `.3` 映像，不使用 owned 入口。

| 场景 | 真实结果与边界 |
| --- | --- |
| 收起入口双文件正例 | 系统选择器形成英文空格名、中文空格名双卡；单次提交的原路径 JSON 顺序与可见卡片一致，输入无隐藏标记。宿主原文、原生原文、两账本摘要和 ACK 对齐；两次真实 `ReadFile` 的 typed 路径和原字节输出分别核对，答案精确两行、`end_turn`，稿卡清空。不是仅匹配 prompt 中的路径。 |
| 原生读取权限拒绝 | 同 SID `01a0f448-713b-79b3-8aa2-5ff84b3a3562` 的后续一次 `read_file` 尝试，由用户明确拒绝，零成功读取／零替代工具／零拒绝文件标记进入历史。终态 `permission_rejected/cancelled`，没有模型 `READ_DENIED` 回复；已 ACK 输入不重新恢复为未发草稿。 |
| 成卡后删除／mode 000 | 双卡先建立，再删除第二文件得 ENOENT；另恢复原字节后设 mode 000，实际 open 得 EACCES。提交后本地稿和双卡保留，宿主消息、原生回合／工具、桥回执均零新增，历史和账本不变。未测精确 PTY 字节；早期驱动截短 20 字节的尝试不计拒绝验收。第 27 图真正提交前已恢复完整 69 字节中文稿，对应 d0bb 审计；第 28 图为 mode 000 独立拒绝。 |
| 双语与布局 | 当前错误 Toast 存在截断，不计完整双语通过。英中提示已改为短两行，须新构建分别核验全文、换行与控件布局；旧模型正例按旧工件保留，不冒称新 FTL 已验。 |

仓外目录 `resume-20260930/g07-preparation/r-i4txg6p2-grok-gui`：正例 `positive.safe.json` SHA-256 `e2240262d915d4dc03092a9d665442aad36c6ff88d42bd505932bb649722e12e`；拒绝 `refusal.safe.json` 为 `cf4f3a8e67e69521181606ceeeb2a8da3b201983d42aa31ce4e2b92aa5c012ec`；删除后比较 `d0bb1740acd59b8ab2adc0fd446bf78de18b2e25a407ad96099736f771c682ec`；mode 000 后比较 `45b1d74454e9529df7579808084e88ae5db9067175f3f529385a1e28fefe0f95`；白名单索引 `acceptance-index.safe.json` 为 `e70e0e66e7487a5c12d8183a26c8ee5b4561d8773ab5d10e5fa7d71bfdea6528`。未读取拒绝文件正文作审计，也未导出完整原生 system／reasoning／history 或认证数据。

`finish` 确认 App 与已拥有进程退出、私有认证副本删除，profile／数据库保留；小证据归档后外层 exit 0、`cleanup_ready=true`，短目录状态 `cleaned`，日志 SHA-256 `9b8996c38bdb44f6901ccd0856273ce994f904a7f7d81fe55aeec65312aeaf5e`。本轮仍缺新 Toast 英中布局、Claude 普通 GUI 双文件／必要失败拒绝链，以及固定 Codex 0.156.1 的原生读取审批拒绝；旧 Shell 模式保护不代替原生拒绝，旧 TextEdit 拒绝原件未复核，不作为本轮关闭依据。G01 绑定后的最终源码门禁也待完成。**G07 不关闭，计数仍为 4／7／3。**

## 2026-09-30：G06 本次 Mac 范围关闭

**本轮关闭 G06：当前为 4 项关闭、7 项开放、3 项其他平台验收移交，PR #22 保持草稿。** 既有 Claude 多技能、热新增、原生注册确认、恢复、权限上限及 GUI 证据按原构建保留；本轮补齐 Grok 用户来源与固定技能剩余 Mac 条件。既有证据重新索引见仓外 `resume-20260930/g06-closure/existing-evidence-index.safe.json`（SHA-256 `1246eda3e5795065ff6fc8c9862c4b84922fd6c146d038252a279ab6bd3f6d5a`）；旧 GUI 结论按固定 Git 及记载索引复核，没有声称重新打开旧图或重跑模型。

| 原关闭条件 | 本轮证据与边界 |
| --- | --- |
| 多技能真实接口及调用顺序 | `r-z26hw4l7` 官方 `1.0.41 (4220f3b224a6)/grok-4.7`、Inherit 私有 leader：local alpha＋user beta → 热新增 user gamma → 冷恢复 user gamma＋local alpha；首技能原生展开、其余精确 read_file 原字节和实际答案分别核对。Grok 工具执行不保证严格串行。固定策略改为保留用户选择顺序，权限集合仍规范排序；原有 Claude 顺序审批／执行证据保持。 |
| 会话内新增、准确注册确认 | 用户链核实原生 `reloaded:1`，并按同会话目录中的 scope、qualifiedName、原绝对路径与字节核验。新增前不可见，确认后模型真正使用；不是仅凭目录有文件计成功。 |
| 路径与权限边界 | Inherit 多技能在原有独立 slash 块和文本／图片之后追加已核目录引用 JSON，仅含所选 qualifiedName/path，正文与隐藏标记不进入用户输入。`r-j2y3ivom` 固定技能三轮真实 Skill 拒绝／允许／冷恢复允许；两代未选 beta 均经真实生产控制器拒绝，原生历史无额外输入。固定来源副本、策略快照及合法／非法子集合按生产函数校验；没有创建真实子任务，不是 G10 父子全链成功。 |
| 历史恢复与失败原子性 | 用户链同原生 ID `01a0f27c-a1b6-72a3-be76-5076c80f9e18` 三次输入、三次精确接收、零重投；固定链同 ID `01a0f29b-598c-7f61-8162-6cc52aa3ae9b` 三次输入，两代快照不扩张。两条链各两代自然 `stdio_closed/exit 0`；固定链另核原生 wait status 0。既有注册失败回滚、回执与清单落盘顺序及历史重放回归继续按原来源计证。 |
| 用户功能英中审计及目标源码门禁 | 英文总说明改为 “These read and file policies…”，简中同步限定读取／文件工具策略，独立已选技能帮助保持准确。`r-vjchihj0` 四张真实 GUI 图在 1280×800 逻辑窗口逐张审阅，目标说明换行及控件可见；滚动边缘不是文案截断，零模型输入。最终 Mac 门禁通过；最终提交 Linux／Windows 门禁仍待验。 |

用户链收据 SHA-256 `b2f6db4f0c8789dbbff5ec832a7279c2ea8bf5c94d5c19cc3ef782f05a239a55`，原生审计 `dd7de44bb5b3064ce179177399683f09b35100e71420892a059427a2cdc56535`。其模型运行通过、外层封装因清理判据曾返回 1；随后精确归属、进程／资源组／launchd／打开文件与认证副本复核完成清理，独立完成收据 `904ab0af99f7382ab2d6e4d8ab08e983036f681658b994a72ff82ce3bb94fc5e`。原外层失败未覆盖。用户链不证明 GUI 或协调器 SQLite。

固定链 `r-j2y3ivom` 收据 SHA-256 `b22d69f670c19b83aa0f0b1da7746883b771eccb606ce40de74ef31cc93470f6`，原生审计 `249b4d3cac5d25ac25937e0b2f6d2c63206570d66881d979a55cd2a42f03e0df`；监督程序 `r-b0dvdqb5` 签名后映像 `f676cea8bf4af97d17a70615aed7df481d9e35c0b32e0decd9fa2dd7b3f16915`，构建绑定 `a03a9ce241d6c21bdd0354bf87b0dc5f5606718a4d60ef0d95f9ee9c75d83055`。原生两代清理确认、认证副本已删除；外层曾因 16 个系统 mdworker 新标签保守保留短目录；这些服务自行退出，未由验收脚本停止。随后精确核实两代原生 wait status 0、资源组／job／PID／PGID 均退出、打开文件与认证副本为零后完成清理；独立完成收据 SHA-256 `8c8845e767fd1d2eb9b95f1ef4118d4398c586418bbc441e1cedd5370c7d1371`，清理最终索引 `b9a12f1bd393edbfd503857bc1c8b3fa7320c804e3004b0f453fb9376bb87845`。该保守保留不是产品失败。独立复核三轮原生回放与两代进程原件，保持严格正文、最终答案、未选权限和退出规则；审查收据 SHA-256 `d06e2c7b1ad89c78097978f3e60773de9ba9104589854f01fa9254e183bfb58e`。本链不证明 GUI、协调器重启、真实子任务或 OS 沙箱。

旧用户轮 `r-ph4ikgm5` 先尝试未选 local beta 路径并得到 FileNotFound，之后正确读取 user beta；原严格审计仍失败，没有放宽额外工具／路径限制。旧固定轮 `r-spl5ejar` 原收据的“技能完整正文或实际最终答案不匹配”保留；后续检查又确认两代 `exit_code:null`、native wait status 9，不能把清理 SIGKILL 当自然退出。`r-4fii2p5z` 在监督程序四段 bundle ID 初始化时失败，原生 session 为空、零模型输入；只修正仓外三段 ID 封装后复验，不改写旧收据。

退出修复只适用于固定受审 Grok `.41 --no-leader --agent-profile … stdio` 的已拥有进程：启动时核固定平台工件、文件身份与精确参数；仅 StdioClosed 提供 8 秒有界收尾／40 秒退出确认，普通进程及取消、宿主断连沿用原期限。公开 1.0.41 源码的 EOF 至少有 100ms＋2 秒固定等待，已列有界排空合计约 7.1 秒，旧 2 秒宽限存在冲突；公开 SOURCE_REV 与官方 stock 工件不同，不声称两者二进制等价。源码审查 `fixed-stdio-eof-source-review.safe.json` SHA-256 `af6ecfd48d0be90e2b8cca3aa759c635303d532fde0fce3c2b407dea56d1cfa8`。这项收尾修复本身无需本地化变更；本轮技能策略说明的双语变化与 GUI 审计另行计证，独立布局收据 `c9015fec93497668073c8c546ff6ffae3aa173d313b5ec5f6260c865b4a05368`。

最终 Mac `r-y_b99yo6` 七步通过：cargo check、libtest 编译、564 项 Grok／固定策略／托管进程定向 nextest、11 项 i18n、34 项 Python 审计回归、actionlint（仅忽略既有 `infinishell-ci` 自托管标签未登记告警）、应用构建；7695 项未选／跳过不计通过。套件收据 SHA-256 `80f1538419ae7a40112de80bd71075495705ec82ec5b5a5cbc325e6ffb2a6232`，未签名应用 `cd82decbde36c94bbd026c9fa003960e64e73bbdb8ad104c606e3f520869e416`。它绑定 `e36adfd6c272c7533a2f600b259e754e5a507f47` 基线加冻结工作区逐文件摘要，不能称干净 HEAD 构建。用户链与 GUI 保留其原工件；与上一套件共有文件有 8 个变化，技能编码、目录逻辑及两种 FTL 摘要未变，不称最终程序重跑旧链。

实现提交 `625ffcb98b09d0ecbefa9e21356ecb8401a7407c` 已推送，84 份冻结源码逐文件匹配，绑定收据 SHA-256 `7c6cc80dee08e07d3af09510eaf7b7b1eebdcfbb2243001ad1ac5fb6325a2280`。[本轮 Linux／Windows 源码门禁 36726214615](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36726214615) 的 headSha 已核对，两平台官方作业均已成功，Mac Intel 按范围跳过；临时 ref 仅用于避免取消前轮门禁。完整 job 日志分别确认 Linux 12 个 nextest 命令组 4927 次 passed、Windows 11 组 4739 次 passed，两平台 rust-genai 各 81 次；次数不是跨命令去重测试总数。新增技能选择顺序及四项 EOF 合同回归在两平台各实际普通 PASS 一次，Grok profile 组分别 30／29 PASS。未见 FAIL、FLAKY 或重试状态行。Windows 桌面／TUI 组为 660 passed（1 leaky），用例 `terminal::input::tests::test_ai_context_menu_keeps_workflow_reference_in_ai_input` 明确标记 LEAK；根因未确认，不能当作无句柄／后代残留，也不能认定与历史相邻用例同根因。该警告保留为整体验收限制，不改写为 G06 新回归失败；G06 的功能、恢复与自然退出另有上述实际证据，现按原条件在本次范围关闭。其他平台已认证模型及 GUI 实机按用户新范围移交，不计通过，也不删除三目标平台实现条件。新固定轮独立清理已归档。G10 的父子任务、命令与邮箱整项范围没有随本轮缩减或关闭。

本轮完整原始日志 ZIP 的 SHA-256 为 `0b4b7db366bedb81a6de25f3eca509caaec81611a7f504e49d4ec3a58bbe995d`；仓外 `g06-closure/ci-36726214615/verified-summary-v2.safe.json` 为 `a4dd47d503c26fb8a5b6d36b82127eac74ab620a108ed8afc7622cec16a4cd57`，最终索引 `786dd1b4259d909e367265d96618318cb3b9df132342d937074a4057388fc4be`。12 项官方工件仅保留索引，不宣称已下载原件。v1 分析器漏识别带序号的 nextest 行，原错误汇总保留，v2 重新逐名核对原日志；原日志未变。提交前文档门禁 `r-syzne9y1` 的 cargo check 通过，日志摘要 `68aae46269cf9baff3c2f127263c64f487e1d1e0f00b39f0e1d4d7d03889c3d7`，短目录已核验清理；当前 84 份冻结源码与 `625ffcb98` 逐文件一致。关闭文档同时纠正当前能力表中的过期策略／图片组合概括，未修改产品或历史失败记录，无需本地化变更。

## 2026-09-30：G01 旧源码门禁通过，外平台接入仍待实现

[源码门禁 36714435483](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36714435483) 精确绑定 `5629588b77b15e27c9e046cc6326171370297dc6`，Linux／Windows 官方作业均成功，Mac Intel 按范围跳过。完整 job 日志按每平台一份统计：Linux 12 个 nextest 命令组共 4917 次通过及 rust-genai 81 次，Windows 11 组共 4731 次通过及 rust-genai 81 次；这是命令调用计数，不是去重后的测试数量。两平台未见 FAIL／FLAKY／LEAK／重试状态行。Windows 终态 run/job 均为 success，但部分步骤元数据仍旧 in_progress/pending；原件保留，完整桌面／TUI 日志另证 660/660、rust-genai 81/81。仓外 `g01-native-source/ci-36714435483/verified-summary.safe.json` SHA-256 `f98ece1d688d03a39a9c00eac056258c65440dde36a9002894b75b80f5ebef62`，归档索引 `91fdefb1fd244752fd2fe1079422ae6f7d56b139d4a51e3918c1541768a46622`，完整日志 zip `1ee16db7f9fa7cfe5b5113d69f78372a47d947771ce0311c7e85a0d43d077ad0`。12 项官方工件仅保存索引，未宣称已下载原件。此结果不继承到后续 G06，也不证明被平台条件排除的普通桥已实现。

实现前置只读核对绑定产品 `625ffcb98` 与原生 `1491b486`。Linux 已有 `LinuxProcessHandle` 的 SO_PEERPIDFD／SO_PEERCRED、真实运行映像 dev/inode 及 TIOCGPTPEER；仍需读取并保留实际映像 FD、接入普通桥、构建定制 Linux ELF，以及为账本增加平台身份形状并兼容旧 Mac JSON／task_id。不能把 Linux starttime 填入 Mac unique_id，不能复用官方 stock ELF 摘要；内核能力缺失时不得降级裸 PID。收据 `g01-linux-feasibility-625ffcb98.safe.json` SHA-256 `d470d2733a1def2773f09e27d0d53785affca42695bb93a2883c7731736b583d`。

Windows 的 [DebugSetProcessKillOnExit](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-debugsetprocesskillonexit) 文档要求先建立调试连接，且设置影响同线程当前及未来调试目标。据此推导：专用 OS 线程先调试私有 helper、确认 FALSE 成功，再附加用户 CLI，并保持 helper 到目标脱离；避免在用户 CLI 已附加后才关闭默认终止行为。[CREATE_PROCESS_DEBUG_INFO.hFile](https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-create_process_debug_info) 可提供真实主映像文件句柄，NULL 或无法核验时拒绝，不退回 pathname／自报 SHA。现有 Windows 进程 lease、SID／登录会话、pipe peer 及映像句柄哈希可参考；现有 owned 调试控制器会终止 debuggee，不能直接复用。原生桥仍须新增 Windows named pipe 和事件循环接线。收据 `g01-windows-feasibility-625ffcb98.safe.json` SHA-256 `f0ba319f35014a1cfe080bf14e99693e65e8e29681d14414e23851e2d72fa223`。这是官方合同推导，尚未实现或实机验证；不证明所有运行内存未被修改。

本段只记录已完成源码门禁及下一步可实现的原生合同，没有新增模型／相近拒绝探针。G01 仍开放，当前计数不变；没有用户功能变更，无需本地化变更。

## 2026-09-30：G01 Mac 功能与双语审计满足，外平台实现仍开放

**Mac G01 功能与双语审计已满足；Linux 宿主普通桥接入、Windows 原生传输及宿主接入仍未实现，G01 必要缺口保持开放。** 原生 `.3` 提交 `1491b486fbaa4bff3b124db2893a413c7019e2fd` 的 check、74 项库回归及构建通过，映像 SHA-256 `edcdc3d8729cc657080e6a266e26a6590ec2b275f4545a5b93c1dd5e26bf08f1`。纯显示动画不再无条件撤销输入 epoch，首页预创建允许命令同步代际 0/1；Agent 代际与审批／后台任务等危险待处理守卫保留。宿主每连接先捕获内核对端凭据，再持续核验进程、签名与 PTY；完整 SHA-256 校验保留，开发配置仅提高 `sha2` 优化级别。响应读取改为 nonblocking＋poll 的绝对截止时间，修正对端关闭后再次设置 `SO_RCVTIMEO` 返回 `EINVAL` 的回包丢失。

宿主 `r-hgmsd_pn` 的 check、48 项桥回归、11 项 i18n 和 GUI 构建通过，收据 SHA-256 `c0b6dedf24299269128c88a72c9f6512d74aaae8f09f8a8ddd121fc3878c38ac`；其 peer 过滤器最初命中零项，不能作通过依据，另由 `r-joflf1h5` 的两项有效回归补齐（日志 SHA-256 `0d7dd80266ee4522c6502bd4f37fdc7b2073e77a8962d13c68b199b43c8b8fa9`）。独立 GUI `r-wudipm8z` 绑定 `c3b509622542efb529fd92af2645ebc938e55bcb` 基线加冻结源码摘要，未冒称干净 HEAD 构建。源宿主映像 SHA-256 `5aaac1b40db8c446d7ae2746dff69b67f8d2ece1beae8dac4395cfc361e616a4`，签名 GUI 映像 SHA-256 `d52e438de1cd2790ab0f38a33b38562c21ff502b26b9d637f86c19c6ce587958`。

该 GUI 从普通 shell 启动原生标准首页，无 `--minimal` 或 owned leader 参数；实际完成以下闭环，原文、宿主账本、原生历史、回执及模型结果分别核对：

| 场景 | 实际结果 |
| --- | --- |
| 英文两行、标准首页首次提交 | 171 字节原文、精确回答及 `end_turn` 各一次，原生 ACK 后清稿 |
| 原生编辑器已有草稿 | 新富输入拒绝且原草稿保持；用户清空原生草稿后明确发送，原文一次、非空回答与 `end_turn` |
| 中文多行长文本与双 Return | 41,751 字节、400 行逐字节接收一次，精确回答与 `end_turn` 一次 |
| 真实 Bash 审批与保留稿 | 审批期间富输入不提交；用户在原生明确拒绝，回合 `cancelled` 且目标文件未创建；保留稿随后明确发送并完成 |
| 相同正文与明确新一轮 | 已 ACK 正文再次 Enter 不重投；用户明确选择再次发送后产生新 ID，原生恰新增一次并取得精确结果 |
| 准备期间编辑 | 旧输入快照取消且无新增原生输入；新草稿保留，后续明确发送并取得非空结果与 `end_turn` |
| 宿主与原生双重重启 | 同 native SID、新 instance，旧七条不重投；中文界面再次提交旧正文提示已接收，新 137 字节正文取得精确答案与 ACK |

最终八个唯一 message ID、八条带 `prompt_index` 的真实原生输入和八个 ACK；七个 `end_turn` 与一个用户审批拒绝后的 `cancelled`。五个预设精确标记结果通过，另外两条普通输入没有预设标记，按非空结果与 `end_turn` 计证；无 `prompt_index` 的初始用户型环境前缀不计真实回合。新旧原生实例分别承载七条与一条输入。英文及简体中文提示和结果在所存视口可读。

证据目录为 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20260930/g01-native-source/r-wudipm8z-gui`，独立摘要 `acceptance-summary.safe.json` SHA-256 `3c72d7d6bd6496ea5537b30e3cda385aa98afbd5875429f477e6b90aa47b63fa`，逐项索引 68 份证据。保留第 39 张截图点错旧坐标的操作边界；它不证明已接收提示，正确第 41 张截图与 `cold-rich-old-message-guard.safe.json` 才证明防重。旧 Unknown 不自动跨原生 instance 转 ACK；本轮没有冒称此能力。两代 App 自然退出 0，原生和后代退出，私有认证副本删除且原配置不变；核明系统自动化服务不持有短目录，外层 `cleanup_ready=true` 后已清理。

本轮发现关闭富输入后的文本粘贴被拒绝，却提示用户粘贴到原生 Grok。随后源码已改为打开富输入并恢复／追加本地草稿，不写 PTY／socket、不自动提交，英中提示明确此语义；未核验桥时改提示原生键入。该改动不在上述 GUI 工件内。`r-zxk8iqk7` 的 check、52 项桥、两项 peer、两项既有粘贴回归、11 项 i18n 与构建已通过并清理；收据 SHA-256 `e73eda855d6b092c8320e3af80e81f62067f73b64742f33cd004b7b93e73d66c`，新源映像 SHA-256 `40d27692222d470ee3c9f65e9d033f0f83f4efccd69f4de313d1ccc88a818ed9`。后续新工件 GUI 结果见下一段，不把旧 GUI 作为新源码验收。旧 [CI 36699745396](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36699745396) 的 Linux／Windows 成功仅绑定 `c3b509622`，不能继承至当前源码；Mac Intel 跳过。G01 继续开放，整体仍为 3 项关闭、8 项开放、3 项外平台验收移交，PR #22 草稿不变。

后续粘贴 GUI `r-g91qynam` 已结束并清理，绑定上述 `40d27692…` 源映像与签名映像 SHA-256 `666bd7c472303f16f62ac7cab27e5d07178f6faa715750fbe65c76bf9ecf41a4`。英文先后粘贴 90＋41 字节、中文 84＋44 字节，关闭富输入后再粘贴会恢复并追加草稿，没有自动发送；原生已有字面草稿 `123` 时提交拒绝且两份草稿均保留，用户 Ctrl+U 清空后明确发送。合并后的英文 131 字节与中文 128 字节各有一次原生原文、ACK、精确答案及 `end_turn`。关闭桥的回退在私有测试 shell 独立 unset 后直接启动同一原生工件，仅证明保留稿与零新增输入，不宣称无桥发送成功。最终宿主／原生输入均为 2。

该轮英文第 12 张回退提示完整；中文第 20 张提示末尾“内容”截断，**双语整体验收未通过**。现仅将中文 `cli-agent-grok-input-manual-copy-required` 缩短为“未发送，草稿已保留。关闭富输入后可在 Grok 中键入。”，后续 `r-fdaad6hh` 重建门禁与 `r-ni936aln` 受影响布局已通过（见下段），不重跑或重计模型矩阵。英文第 10 张曾通过 `env -u` 包装启动而未识别 CLI，未发送正文，不能作产品正例；改为直接启动后的第 11／12 张才计回退验收。旧只读审计器两次因首页尚无 `updates.jsonl` 失败，非产品失败；v2 明确记录文件缺失，不改原 v1。两代 App 自然退出 0、认证副本删除、11 件保留原件核摘要、短目录清理。独立摘要位于 `r-g91qynam-gui/acceptance-summary.safe.json`，SHA-256 `5576b8c0b7378cbf7556a71b5b758fe70a0aa2da9b5844892b114a719620bd6b`，索引 46 份证据；该历史截断已由下一轮修复复验；G01 的外平台实现缺项仍须继续完成。

最终 `r-fdaad6hh` 的 check、52 项桥、两项 peer、两项既有粘贴、11 项 i18n 及构建全部通过并清理；收据 SHA-256 `05ed738634c73dae51643bc619ffad1efcbdc9fdbb45733cd642a40c87365bfa`，源映像 SHA-256 `90cd57c7813c4dbe2d34c92adc68de7d61c1756db56f7020f59b8fa0fdf76bf2`。资源差异收据 SHA-256 `e5499b73fcd72bf643056bee44569935f46bb9d3efb2aee8c0a48ef867ee3874` 证明较前一 suite 仅 `app/i18n/zh-CN/warp.ftl` 改变，英文资源、受测提交逻辑及原生工件不变。

最终中文 GUI `r-ni936aln` 只复验布局：第 02 张占位文本完整，第 03 张短回退提示完整显示句号与复制按钮，宿主消息／原生用户输入／回执均为 0。签名映像 SHA-256 `6d62181f47a9e49fb1c8b4c69d9e3c36d89ae27fb437b10d988a4862f9d661f5`；独立摘要 `r-ni936aln-gui/acceptance-summary.safe.json` SHA-256 `0185ced924964c2fc47871344dad2796128079278a4338000ced2c8045855702`，索引 17 份证据。App 1912 自然退出 0，认证副本删除、进程／打开文件清理确认，短目录已清理。原八条及粘贴两条模型正例分别保留原工件身份，本轮不重计功能回合。**Mac G01 原生能力、真实普通 GUI 与英中审计均已满足；Linux 宿主普通桥接入、Windows 原生传输及宿主接入仍缺实现。用户仅后置外平台实机验收，G01 必要缺口仍开放，3 关闭／8 开放／3 移交及草稿 PR 状态不变。**

现时范围更正：此前摘要及文档的“仅待 CI”结论遗漏外平台实现，不作为关闭依据。`app/src/terminal/cli_agent_sessions/mod.rs` 的普通原生桥仅编译 macOS arm64，原生 `app/mod.rs` 的桥传输限定 Unix；Linux 宿主接入和 Windows 原生／宿主能力仍须实现。仓外 `scope-correction.safe.json` SHA-256 `33411686223414380743cc838f1d1179cc578d315c740a75ef486476449ae863` 引用三轮不变的原摘要，仅更正剩余工作判断。`commit-5629588b7-binding.safe.json` SHA-256 `812edfc3c49188b00b5edce62cfa4af1e463ec762755a3686271a4cbedf4daf6` 已证明最终冻结源码进入 `5629588b77b15e27c9e046cc6326171370297dc6`，没有声称重新构建或重跑模型；该提交 [CI 36714435483](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36714435483) 正在运行，普通源码门禁不能证明被 `cfg` 排除的能力存在。其他平台实机仍由用户后验；下一步先推进 G07 小项，同时保留 G01 外平台实现工作，不等待上游。

历史失败 `r-rk5mi0us` 中一次 Return 确已进入完整 SHA-256 计算，但 254 秒后两侧账本仍为零且两次 State epoch 稳定，不证明 SHA 是唯一原因。下面保留 `.1/.2`、owned 路径及早期门禁各自范围，不覆盖本次工件。

## 2026-09-30 历史：G01 新宿主源码门禁与 GUI 未投递

定制原生 `.2`／`8948fc8c2c15` 通过编译、68 项定向库回归及独立构建；最终映像 SHA-256 `51c152147fe4c611010e37c798a82165a5997a461d9f7672911a580014a6e28b`。宿主冻结收据 `r-9t4medj5` 的 `cargo check`、45 项桥、82 项持久化、441 项会话（9 项忽略）、54 项富输入、11 项 i18n 及 GUI 构建全部通过，合计 633 项通过。收据 SHA-256 `70c425b96d38fab4ef525c586c3c1f14330326ec6e4cf51386efce227df62c48`；逐文件源码绑定，旧门禁不冒充本工件。

独立标准首页 GUI `r-_b9gy4r4` 已在默认提交设置下完整粘贴英文两行 171 字节，但 Return 后未观察到投递；宿主消息及原生桥回执均为 0，草稿保留。ABC 对照同样未投递，原拼音及剪贴板已恢复。源码确认首页动画每 83ms 可撤销输入代际，现场只读 epoch 为 5029；尚无证据将它认定为这次按键无响应的唯一根因。失败收据 SHA-256 `f24705326cf58863ed81ec300f0229d182d917f23bc71443b8ddd771feb68c13`。英文新版占位提示无遮挡，中文布局待验；不计 G01 关闭。测试 App、原生和后代自然退出，认证副本已删除，小证据归档；核明 Apple 自动化基础服务未持有本轮目录后，短 TMPDIR 已清理。

## 2026-09-30：G01 标准首页失败边界

独立 GUI `r-8fh747ei` 使用旧定制原生 `2a13b486`、标准首页，明确得到 `ready=false / no_active_agent`；原生 Ctrl+N 创建会话后可得到 ready，但本轮宿主消息与原生桥收据均为 0。屏幕上的英文模型回复未走桥账本，不能计自动提交成功。定向 CGEvent 驱动存在快捷键和中文分块不可靠现象，且测试中修改过提交快捷键设置；后续用默认设置、系统按键与完整粘贴复验，不据混合驱动结果判产品成功。失败收据 SHA-256 `e8978af44a79234a84db5b3b770c45d40737c0dc55fe4471b3febd9a2809d973`。

已新增首页显式准备请求，复用正常原生创建／工作区确认，准备阶段不携正文；回包固定 agent、session 和 binding，等待后重新取得同目标租约，再进入持久领取。纯输出 ACK／重绘不撤销租约，输入、ACP、任务及可改状态的动画仍撤销。英中富输入提示同步去掉“普通会话一律需复制”的旧语义。新源码门禁与真实双语 GUI 尚未完成，不关闭 G01；旧门禁仅对应下文冻结工件。

测试 App 和原生 CLI 自然退出，认证副本已删除，原输入法和剪贴板恢复。核对系统自动化服务不持有本轮目录后，短 TMPDIR 已清理；失败截图、账本和原生历史保留在仓外。

## 2026-09-30 用户调整范围后的关闭结论

本次 Goal 的其他平台实机验收已由用户明确移交后续执行。按保留的原关闭条件逐项核对，**G02、G04、G05 的 Mac 范围关闭；V01／V02／V05 移交，其他 8 项继续开放，PR #22 保持草稿**。这些条目采用已经完成的功能与真实证据，不把文档调整计为新增实现。全平台原始验收仍未完成。

- G04：下节 `47ac3a79e` 的 JPEG／WebP／静态 GIF 逐项满足 MIME、真实格式、原字节、图片回答与冷恢复；大小、像素、整帧预算和非法格式的输入门禁已有回归。PNG、JPEG GUI 和相关英中布局保留 `84102bb1c` 等原构建来源，新收据未覆盖的 GUI／SQLite／应用重启不回填。
- G05：后文图片生产适配器和 GUI 恢复记录分别证明纯 PNG 与 PNG＋单技能的新建、接收、图片回答、精确 Skill 调用及冷恢复；GUI 重关联后同原生会话三条输入保持不变。最终输入／恢复源码由 `6790b185a0c0bc581f804edf975960047df85db7` 的 36 文件快照绑定（收据 SHA-256 `613cb19968667392a08829f4f831030af6da4a2fca7d9fa58ef538e4e6b80060`）。旧模型轮次与修复后恢复程序分别计证，不外推所有格式组合、固定策略或多技能。

英中格式、版本、静态 GIF 和图片加技能提示已复核，既有布局证据的视口范围不扩大；本次无产品代码或界面语义变更，**无需本地化变更**。提交前 `cargo check -p warp --features warpui/test-util,rust-embed/debug-embed` 通过，短目录 `r-oeigx2ze` 已按记录归档清理，日志 SHA-256 `9637fb0320c0aeb335014989a99cc682bdb891dd518aa79ddf7de9f4cdf285be`。原实现的 i18n 和相关回归来源保持不变。

以下“0／14”“仍开放”是范围调整前的逐轮判断，不能覆盖当前用户授权；原始通过、失败及未验事实继续保留。

## 2026-09-30：G01 原生源码判断更正

官方 [xai-org/grok-build](https://github.com/xai-org/grok-build) 已公开 CLI／TUI／agent runtime 源码。[README](https://github.com/xai-org/grok-build/blob/07e35a3dfeed2f200d319ef6c893b5ea286d9a51/README.md) 提供本地构建入口，源码采用 [Apache-2.0](https://github.com/xai-org/grok-build/blob/07e35a3dfeed2f200d319ef6c893b5ea286d9a51/LICENSE)；[CONTRIBUTING](https://github.com/xai-org/grok-build/blob/07e35a3dfeed2f200d319ef6c893b5ea286d9a51/CONTRIBUTING.md) 不接外部 PR，但允许按许可证本地构建。此前从 npm 发行包没有源码推导“缺少可修改原生源码、必须等待上游”的结论错误，现予更正。旧 `idle_prompt`／审批／草稿探针及发行包内容事实保留。

原生工作树已固定公开 `1.0.41` 快照 `07e35a3dfeed2f200d319ef6c893b5ea286d9a51`，其 [SOURCE_REV](https://github.com/xai-org/grok-build/blob/07e35a3dfeed2f200d319ef6c893b5ea286d9a51/SOURCE_REV) 为 `84745de98b3d3996729aefcefd518890ffb73930`；已验官方发行版是 `1.0.41 (4220f3b224a6)`、SHA-256 `9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d`。两者没有相同源码／二进制身份依据，后续定制构建必须独立绑定与验收，不回填既有发行版结果。

该次判断更正时尚需实现可认证的普通 TUI 接入、原子核对编辑器修订／模态／审批状态并提交的一次性事务，以及应用接入、独立构建和 Mac 普通 PTY 真实验收。后续原生实现和门禁见下一节，普通入口拒绝继续保留。该次判断更正本身没有改变用户功能或文案，**无需本地化变更**；随后源码补丁与门禁单列记录，不能把本段当作整轮未执行代码的结论。新增关闭 0 项；当前仍为 G02／G04／G05 关闭、8 项开放、3 项移交，PR #22 保持草稿。

## 2026-09-30：G01 官方源码原生输入桥检查点

原生修改保存在 [可重建补丁](../../native/grok-build/terminal-bridge.patch) 与 [来源清单](../../native/grok-build/source.json)，不是通知探针。公开基线 `07e35a3dfeed2f200d319ef6c893b5ea286d9a51` 上的独立源码提交为 `2a13b48618b93dea773953e835e74f4fd7a31429`，补丁 SHA-256 `2d06b640fe3b7f66f8df0ea690388cd8231c89abc88eb4ddbf282fffea26138c`。使用独立索引从基线应用补丁后，所得 tree 与冻结验收源码一致。上游 Apache-2.0 及本仓新增修改的 AGPL-3.0-only 声明均保留；没有向上游推送或覆盖用户默认 Grok 安装。

普通 TUI 显式启用私有 IPC 后，在自身事件循环核对草稿、附件、模态、审批、会话代际与一次租约，再由原生 actor 同锁检查队列、后台 Bash 和子任务并接收。用户消息进入原生历史且持久化屏障成功才确认；未知结果只读查询，登录、额度恢复、Hook 拒绝和其他订阅 TUI 均不得自动重投。定制版本 `1.0.41+infinishell.terminal-bridge.1` 使用独立 leader 命名空间，并拒绝旧客户端注册。

最终原生门禁 `r-cidusm0v`：`cargo check --locked -p xai-grok-pager-bin`、四包 `--lib terminal_bridge` 定向回归 **61/61** 和独立 `cargo build` 全部通过。逐文件源码摘要在门禁前后相同；二进制 SHA-256 `1da3cf195aa3ed2778fa2efe05fa211f21d6520514ef776fc5a6e6b53739568f`。原始日志 SHA-256 `9ccca3c90a55898ad781c3c2b66ecbd952cb0a4e6204152e1df0b84e4145fabd`。本机短 TMPDIR 的身份、进程退出、打开文件释放及归档均有记录，已清理。主仓同产品源码的 `cargo check -p warp` 与 i18n **11/11** 通过，实际使用 `warpui/test-util,rust-embed/debug-embed`，不冒称发布构建。

失败边界保留：初版 coordinator 的 Box 引用不匹配已修；上游库测试缺 `base64::Engine` trait，补丁只补必要导入；上游完整 `--tests` 仍因未公开的 `docs/internal/25-enterprise.md`、`22-environment-variables.md` 缺失而失败，本轮没有伪造内部文档，定向库测试不等于上游全测试通过。错误文案审计中发现的三条新增内部错误已复用既有通用错误；协议原因码不直接当作 InfiniShell 文案，**无需本地化变更**，未声称新增英中布局通过。

**新增关闭 0 项。** 原生代码和离线门禁不替代 InfiniShell 普通终端的生产接入、可信 peer/PTY/映像核验，以及中文、多行长文本、审批、旧会话、重连与重复点击的实际回合验收。G01 保持开放；Linux/Windows 实机验收按用户范围后置，Windows 原生桥传输尚未实现。PR #22 保持草稿。

## 2026-09-30：G01 普通 PTY 原生实际验收

2026-09-30，普通 PTY 原生桥接验收 r-7j9xe_3c 绑定定制源码 2a13b48618b93dea773953e835e74f4fd7a31429 与二进制 SHA-256 1da3cf195aa3ed2778fa2efe05fa211f21d6520514ef776fc5a6e6b53739568f；对应 suite 的 cargo check、61 项定向回归及 cargo build 全部通过。独立历史审计区分了 1 条旧格式环境前缀、3 条 system_reminder 和 3 个有 prompt_index 的真实用户回合，无未分类用户行。26,597 字节中文长文本与原生历史逐字节一致、只出现一次，并返回精确答案；同实例重复提交只返回原回执。审批期间 state 为 session_busy，旧租约提交被拒且无新增输入；通过原生 UI 拒绝后完成原因为 PermissionRejected，目标文件未创建。首代 TUI 自然退出后以同一 session ID 恢复，第二代新输入及精确答案各出现一次，两代均正常退出，外层记录已核验后代、launchd 和打开文件并完成清理。英文首次丢连接仍为 Unknown、原生输入 0 次，不能计作正例；恢复后的旧实例请求为 instance_mismatch，旧回执查询 Unknown，也不证明跨实例回执恢复。此记录仅覆盖原生普通 PTY 驱动链，不替代 InfiniShell GUI 真实链、中英文界面审计或 G01 完整关闭。

安全索引 `resume-20260930/g01-native-source/ordinary-pty-validation.safe.json` SHA-256 `fdfb486c7a14088b7db28d92184c77144300fb6d90ec1f7a537213b5af3c441f`；独立历史审计 SHA-256 `dd2fbf277de986a5597737a6c2638fbc4c4420f58c7f45af8b58bbc1a8acf3b8`。本轮主仓已写入普通 shell 接入、固定工件／内核 PTY 身份、持久领取与 GUI 分发，联合门禁和新的真实 GUI／英中布局仍待验收，不外推旧版本或 owned 路径。

## 2026-09-30：G01 宿主生产接入源码门禁

冻结源码联合门禁 `r-e8nhc_09` 通过 `cargo check --locked -p warp`、新桥 39 项、既有持久化 82 项、既有会话 441 项（9 项专用环境忽略）、富输入面板 54 项、i18n 11 项和 `infinishell` 构建；本机统一使用 `warpui/test-util,rust-embed/debug-embed`。command 内核接口另有 20 项通过、3 项内部夹具忽略。新二进制 SHA-256 `2fb24a98590be124b5dc932cd353d7dea3fdcca68feea96d7cf34afa1539aa07`；冻结源码／命令／产物收据 SHA-256 `c7e69cac0551f3f48b39b8cc90d5497972413dcb205a7a83fb2b96bf935c302f`，日志 SHA-256 `2e0bc3567c46ac8b13ea266c3a9d477617e0d41cdb0e00d7ca98f3ba6daa0652`。短 TMPDIR 已核退出并清理。

保留此前失败：测试 Cow 推断不明确已修；同一原生会话在新实例提交不同正文曾触发活动任务唯一键，修正为内部账本不占用任务所有权字段，仍在配置／投递记录／稳定任务 ID 中严格绑定会话。原恢复测试未弱化，新增两种任务共存顺序及 owned 邮箱可用回归后通过。两条新增英中提示与变量审计通过，实际布局及宿主 GUI 回合仍待验；本轮新增关闭 0 项，跨平台源码门禁待提交后执行。

## 2026-09-30：G02 Mac 图片与技能组合关闭

干净源码 `d290fd7137165223de8d762cf65a257117c6bf9a` 在 `r-58hxh5i1` 第一次真实运行通过。固定官方 Grok `1.0.41` 的 SHA-256 为 `9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d`，实际模型 `grok-4.7`，Inherit 根任务使用生产私有独占 leader。本次只扩展既有真实验收用例和审计运行器，没有修改生产权限或图片合同。

| 回合 | 实际技能消费 | 原生图片 SHA-256 | 实际结果 |
| --- | --- | --- | --- |
| 新建，PNG＋alpha | 原生展开 alpha 正文 | `0513db23f7d504563037ffa8fb3225a2cd6eba59153e99cfe2438650e5421388` | alpha 随机标记＋RED BLUE |
| 同连接发布 beta，PNG＋alpha/beta | 展开 alpha、原生 read_file 完整读取 beta | `b17982cc99e54574da84929209a5997f0f69c32295b611106f722f7175c4cfec` | 两标记按选择顺序＋GREEN YELLOW |
| 同原生 ID 冷恢复，PNG＋beta/alpha | 展开 beta、原生 read_file 完整读取 alpha | `1979c530b4227cc5b2b0e71127e8c5db7ee876d01ccebdb35153d2156cd9f9e8` | 两标记按新顺序＋BLUE RED |

每轮同时核对持久图片引用的保存／恢复等价、typed 原图 MIME／原字节、原生实际模型、回合和消息身份、一次 ACK 与精确最终答案；原生用户历史只有三次输入，重复 message ID 没有额外执行。两代分别自然 exit 0，cleanup 与 ready 的代次和新建／恢复状态精确对应；原生 PID 消失，launchd、资源域和打开文件均核验后清理短目录。默认认证及设置未改变，授权的同机私有认证副本已经删除，不归档认证或含控制令牌的 manifest。

本机构建 `r-ka7i1qap` 的关键源码均在构建前定稿，逐文件摘要与 `d290fd713` 一致；监督程序复用 `282f5c4a8` 的等价产品源码签名程序，不称当前提交重建。`cargo check`、i18n 11／11、managed_input 29／29、Grok 图片技能与入口 24／24、Python 审计 14／14 通过。首轮定向过滤只选中 managed_input，随后按准确模块名补齐图片技能与入口组，不把零选中的组计为通过。英中固定版本、模型、Inherit、PNG 和技能目录提示已复核，本轮无用户功能变更，**无需本地化变更**。

仓外证据为 `resume-20260930/g02-image-skills/r-58hxh5i1`：收据 SHA-256 `de37c2de73bfc2e1f3a2e1475ed8753d925134fc40e8077caf09f0faba9c2ee4`，独立原生审计 `035ffb8f9fc46b6bef7f95e6abae457f26a72b68232c13fe35778b7050238f24`，归档清单 `3d9a3911dca8e3d88bdf16cb81e06656239024eac129943d242baf15caf6799e`。两代原生身份与 Mac 清理原件均保留。本轮不含真实 GUI、应用重启或协调器 SQLite；这些基础 PNG 路径保留原历史证据，组合入口由现有回归覆盖。按 G02 原关闭条件，本次 Mac 范围满足；固定权限、用户级技能来源、G06 其余条件和其他平台未外推。

## 2026-09-30：G04 当前源码复验与环境阻塞

本轮关闭 **0／14** 项。起点 `c2140a8736eff3e8d67e9ca6d1645d12541072cd` 的分支、工作区和 PR #22 HEAD 一致，工作区干净、PR 为草稿。先比较 G04／G05／G06 剩余条件，选择范围最小的 G04；14 项全部满足前不合并，也不以验收运行器、离线门禁或安全拒绝计完成。

`50dd59b09` 让图片运行器用同一授权默认账户环境检查认证并启动原生 CLI，同时要求 Mac 短临时目录与逐轮记录匹配。真实 JPEG 首轮 `r-m9e_v1r3` 收到固定 CLI 的配对前 queued ACK 后被旧验收判为 `wrong_or_duplicate_native_ack`，没有取得图片答案；退出清理确认成立，但原生 wait status 为 9，不能当作自然 exit 0。原失败、原生退出文件和后续精确清理证据均保留。

`47ac3a79ea5ba738c01338eaf09a68e155637748` 仅修正验收合同：先暂存配对前 ACK，在固定版本与原生 ID 配对后确认；保留观察时 ID 为 null 的事实，仍拒绝错消息、错回合、错代次、身份变化和超过一次缓存重放。旧 PNG 收据不补造新字段。Mac 本地 `cargo check -p warp`、10 项回执回归、Python 15＋24 项及 actionlint 通过；英中资源门禁 11／11 已通过。本轮无用户可见文案或界面语义变化，**无需本地化变更**；静态资源审计不替代目标平台双语布局。

第一批修正 `50dd59b09444cc15a3feae805b70293938ea4da4` 的 [run36599403575](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36599403575) 在 Linux x64／Windows x64 均成功：图片与技能离线组 37／37、36／36，Python 各 15 项；Windows 关联组 509／509，Linux 双语资源 11／11。跳过项不计通过，没有已认证模型图片输入；工作流附带的 Linux 零模型剪贴板检查不计真实 IME 或 G04 GUI 识图。4 份官方工件与 41 个日志成员已核摘要及 ZIP CRC，清单 `resume-20260930/cloud-run-36599403575/manifest.safe.json` SHA-256 `fd17c7fd891aa4da59f63d829a5da23dae4b6847162c4fdbc90915a0832be1ee`。

后续验收时序修正的 [run36602515307](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36602515307) 精确绑定 `47ac3a79ea5ba738c01338eaf09a68e155637748`，Linux x64／Windows x64 作业均 success，两平台 `cargo check` 通过。图片与回执组分别 47／47、46／46，新增 10 项回执时序回归在两平台逐名 PASS；每平台 Python 15＋24 项，Windows 关联组 509／509，Linux i18n 11／11。所选回归未见 FLAKY／LEAK／RETRY 标记；跳过项不计通过，Mac Intel 按条件跳过。Linux 工作流附加零模型剪贴板步骤成功，不计已认证图片、真实 IME 或目标双语识图布局。4 份官方工件和 41 个日志成员已核摘要、CRC 及成员 SHA-256，清单 `resume-20260930/cloud-run-36602515307/manifest.safe.json` SHA-256 `a2ad8f8c5629b852a514ad3228a7419452c2f3387514f616ac27acc18546224a`。这是本轮同提交的定向门禁，仍非 14 项全部关闭后的最终源码门禁。

同一干净提交的三次独立真实链如下；固定官方 Claude `2.1.280` SHA-256 `387a5c5dcdbb815085edf0baf79591f9d8894efe922bceaf3d75b1b08055229d`，默认账户认证状态由原生 CLI 确认。监督程序复用 `282f5c4a8` 签名二进制 SHA-256 `59cadfddd26e17ac32510b69e8daa16735b409b0463cf1d763ade7e9d3308c44`，其后至本次仅测试／运行器／文档变化，不称重新构建当前提交产品。

| 格式 | 独立运行 | 原生图片 SHA-256 | 实际结果 |
| --- | --- | --- | --- |
| JPEG | `r-ee684g78` | `e2313adfb465e828166bc715cdf08e4d80656b08c784f81f76b67dff861165fc` | 图片回答与冷恢复答案精确匹配 |
| WebP | `r-qkxpl7je` | `c23780513cab9fc2dd5ed002e97243e6777e74d17c781bd7543ba15fdbf8f1c5` | 图片回答与冷恢复答案精确匹配 |
| 静态 GIF | `r-bjrl2akb` | `ea0bf2267e2eaf1e12851dbaa9121608040e9c88d0dca2043d8d56833921f748` | 图片回答与冷恢复答案精确匹配 |

精确原生会话历史另证实际模型均为 `claude-opus-5-5`，声明 MIME 与准备的格式／字节一致，两个产品输入 UUID 各出现一次；每条历史额外有一个明确标记 `isMeta/turnCompanion` 的图片元数据项，单独列证，不误算重复输入。六代原生进程均 `stdio_closed/exit 0/cleanup_confirmed`。已核精确进程、launchd 标签和打开文件，短目录及本轮精确原生图片缓存已归档清理，原生会话历史保留。93 件小证据索引位于仓外 `resume-20260930/g04-readiness/index.safe.json`，SHA-256 `adf6a7b4cddc2e2c65ac89defd3f8489e87a24a50d0e5f9d7d658148d0879916`。不包含应用重启、SQLite 或 GUI 验收。

两平台 runner 在线，但本轮只读盘点没有取得 Linux／Windows 固定 Claude `2.1.280` 的可用认证；Windows 在 Session 0、没有交互登录用户，RDP 未启用，仅找到的旧 `2.1.273` 报告未登录。需要用户提供专用已登录账户／配置及可用 `claude-opus-5-5` 的交互环境，才能完成两平台逐格式原生消费、识图、恢复和英中 GUI 检查。G01 普通未绑定 PTY 仍缺可信原子提交能力；G09 仍缺适用 Windows 交互环境，本轮未重跑已有硬阻塞探针。14 项继续开放。

## 此前证据（保留原提交和边界）

精确源码 `79bfce90fb1b87be40743ee8b67211e9dd1ff7ae` 的 [Windows 复验 run36572437241](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36572437241) 已整体 success；只选 Windows x64，Linux 与 Mac Intel 作业按条件跳过。文件提交目标用例在扩展异步等待和超时诊断后首次 PASS（0.730 秒）。Windows 主回归最终 3080／3080，4814 项跳过；另一项 `history_model::test_initialize_output_for_response_stream_persists_updated_conversation_state` 首次失败、重试 2／3 通过，日志标记 1 FLAKY。桌面／TUI 660／660 首次通过，rust-genai 81／81。Windows 完整 job 日志和官方 run／jobs 状态记录已归档于仓外 `resume-20260929/cloud-run-36572437241`，SHA-256 分别为 `f5661f206a8728549becec6bae5a671a0e4efb28f4d2ed4fc63f61dd98a73dc5` 和 `850e0e442d43438f9c5dbeb155dfc0b2f8850a5753f78291e84756d4acb85e3f`。这只证明当前 Windows 离线选定门禁成功和文件提交用例的一次首次通过，不能消除历史间歇风险，也不覆盖已认证原生文件读取或 G07 整体验收；仅测试改动，无需本地化变更。

[run36542534412](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36542534412) 精确绑定 `38f66c88e7e8559f1007cea7d48acb89221ad2a0`，Linux、Windows 作业均成功。固定 Claude 图片／技能离线定向组分别通过 37／37、36／36；Grok 固定 `1.0.41` 的模拟审批卡在两平台各按英文、简体中文及 800×600、1280×800 真窗口运行，真实点击 AllowOnce／DenyOnce 后核对同一 task、generation、approval ID 的命令入队与按钮禁用，32 张原始 PNG 中八组关键画面已目视复核，无明显截断或重叠。收据明确零模型输入、零原生审批请求／解决，`v05_complete=false`，不关闭 G04／G05／G06／V05。Windows 桌面／TUI 组最终 660／660，但上下文菜单用例首次在打印 `ok` 和应用退出提示后超时，第二次 2.96 秒通过，日志标记 1 项 `FLAKY`；不能称全体首次通过或无重试。14 份官方工件按摘要、ZIP CRC、成员 SHA-256 归档于仓外 `resume-20260929/cloud-run-36542534412`，工件清单 SHA-256 `87b768067d99a9ace4938aacefd91b9a5d2693ee0cf84fdb16cbc647d4c186e0`；113 份原始日志单列归档，清单 SHA-256 `1a87098a91418270f1aca6bfc0b6db2f9c8d894172a0708f1e4b1dd8fa7306fc`。本轮只变测试与工作流，没有用户可见文案变化，无需本地化变更；上述八组中英文布局已检查。

后续 `1b6eefb89` 为固定 Claude `2.1.280` 父子 Skill 链新增忽略测试；Mac `cargo check -p warp --tests` 通过。相同源码的独立签名 Mac 应用完成构建与签名验证；安全收据位于仓外 `resume-20260929/mac-parity-build-af2c0ad0/receipt.safe.json`，SHA-256 `e9def26722818a1bbe2540c14beb7966232c43b1f754b67c13271e3a114f5bc7`。该应用随后只作为隔离测试监督程序启动；构建记录本身未取得编译进程的 OS PID，不能称为完整进程身份验收。G10、G07 与 G01 的真实链仍待逐项补验。

G10 忽略测试随后在私有 Mac 环境运行。前三轮在签名包复制或测试初始化阶段失败，未进入模型链，分别归档并清理；两处只含标记文件的旧应用状态另有清理收据 SHA-256 `956c15bdbbbfb144b398f3ea1fcf7146430e8bccfd3a206eccb6bacd0a36267f`。修正后的 `e79152da9` 一轮中，原生宿主 `runtime.journal` 记录 `OwnerClaimed` 与 `SessionReady`，但测试侧未收到协调器事件，父输入在 SQLite 中保持 queued；450 秒等待和 60 秒清理超时，未取得原生 Skill 审批或父子结果。该轮 `g10-skill-live-4432e831` 的原始日志、SQLite、宿主状态和精确进程清理证据归档于仓外同名目录，清理清单 SHA-256 `c46bf00ed7888cd5adcdefbf03c6294e7bf24c227bbfb29f30c54a3cd657a07c`；精确四个遗留 PID、launchd 标签、打开文件均已核空，专属短目录、应用状态及原生临时目录按身份清理。后续 `34167fe3e` 加入协调器错误早读诊断，本地 `cargo check -p warp` 与测试编译通过，但尚未运行。G10 的该项真实链失败保留，不能从宿主就绪推断模型或 Skill 执行。测试及诊断没有改变用户可见文案，无需本地化变更。

`282f5c4a8` 的 Claude 适配器仅在原生 CLI 版本完成配对后向协调器暴露原生会话 ID，符合现有“未配对的首次就绪不得绑定 ID”校验；G10 忽略测试的协调器错误读取移至每次等待之前。Mac `cargo check -p warp`、Claude 协议测试 137／137、i18n 11／11、同源码应用构建和签名验证通过；全仓 `cargo fmt --check` 因大量既有未格式化行失败，本增量未改动那些行，差异检查通过。固定官方 Claude `2.1.280/claude-opus-5-5` 的独立 `g10-skill-live-fixed-3b1f82a0` 在已认证默认账号下通过：父 `run_agents` AllowOnce，子 Skill DenyOnce 后再次 AllowOnce，三次原生审批决定及两次子审批解决、子技能正文标记和父子持久结果均核对；父集合外技能只作派生拒绝，未发送真实越界子任务。父子两份监督退出及原生清理回执有效；运行器安全收据记录 `passed=true`、`source_dirty=false`、两份清理回执、`real_gui_verified=false`。签名应用构建收据 SHA-256 `9c09f907cc509229a79a40859cb4cd46c2550acd9e35cfdec4148eb33a8e4a31`；私有原件归档于仓外 `resume-20260929/g10-skill-live-fixed-3b1f82a0`，精确 PID、launchd 标签和打开文件核空后的清理清单 SHA-256 `527e27a081190e56478052a31061cf5e14db27973ea95163f55706d37b9e1d90`。本链不覆盖 GUI、冷恢复、受审命令、Grok 或 Linux／Windows 在线模型；G10 整项保持开放。没有用户可见文案变化，无需本地化变更。

同源码 `282f5c4a8261747a642c57f70f6aed85d7fa9846` 的 [run36561236364](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36561236364) 已完成：Linux x64 与 Windows x64 官方作业均为 success，`cargo check` 和选定离线回归通过；Mac Intel 作业按工作流条件跳过。Linux 主回归 3269／3269、桌面／TUI 660／660、rust-genai 81／81；Windows 主回归最终 3080／3080、桌面／TUI 最终 660／660、rust-genai 81／81。新增未配对原生 ID 回归在两平台逐名 PASS。Windows 主回归的文件提交用例首次因未等到完成或拒绝事件而失败，第二次 1.801 秒通过，标记 1 项 FLAKY；Windows 桌面／TUI 的上下文菜单引用用例标记 1 项 LEAK。跳过项不计通过，且不能声称 Windows 全部首次通过或无泄漏标记。12 份官方工件的摘要、ZIP CRC 和 47 个成员 SHA-256 已核对，工件清单 SHA-256 `6200acd7602065f1ab9713870cf95b1bba4085503a2a47d0d2110eccb74f22be`；107 个原始日志成员归档清单 SHA-256 `da283d3483d170eed90b934b3d0076e8d78571ce7c205190851b631950ec795c`，均位于仓外 `resume-20260929/cloud-run-36561236364`。本轮仅证明所选两平台离线源码门禁，不含已认证模型、原生父子 Skill 在线链、真实 GUI 或其它必要缺口的完整验收；没有新增用户可见文案，无需本地化变更。

`9ddca1e92` 为上述 G10 Mac 原生正例加入第二回合的真实越界请求，并将隔离运行器纳入仓库。固定 Claude `2.1.280/claude-opus-5-5` 的 `g10-overbound-live-00e35a04` 在已认证默认账号下通过：父子 Skill AllowOnce／DenyOnce／AllowOnce 正例再次成立；同一父任务随后用未选 `beta` 技能精确请求第二次 `run_agents`，原生审批允许、`LocalToolRequested` 到达协调器，当前代各有一条持久 `native_tool_call` 与错误 `native_tool_result`，父回合完成且总任务仍为原父子两条。新回合没有创建越界子任务；这比旧收据仅调用 `derive_child` 多覆盖了真实工具派发路径。测试源码提交为 `9ddca1e92d510c51dcad04b246c74afac253de03`，工作树干净；签名监督程序仍为 `282f5c4a8` 的可执行文件 SHA-256 `59cadfddd26e17ac32510b69e8daa16735b409b0463cf1d763ade7e9d3308c44`，两提交之间没有非测试产品源码改动。测试二进制在私有目录重签后 SHA-256 `5291162db08ceb1e467670208ff860e33f61f530aae0c2f9d83fd98bb613948d`；安全收据 SHA-256 `b9a5c304db9380c882f9b3cb5f2b846111cd8fde87d55ceb293d4e5c3dffe4b7`。原始事件、SQLite、宿主清理文件等 34 件小证据归档于仓外 `resume-20260929/g10-overbound-live-00e35a04`，精确四个原生／包装 PID、两条 launchd 标签和文件占用核空后，短目录及本轮独立应用状态按身份清理；清理清单 SHA-256 `cd205861ba73ce6c2b380e33226ecb411792a65d16341cb8b512ad97d8d71b78`。Mac `cargo check -p warp --tests`、后续 `cargo check -p warp`、i18n 11／11 与嵌入资源配置的 libtest 构建通过；本增量只有忽略测试和运行器，无用户可见文案变化，无需本地化变更。该轮未覆盖冷恢复；后续 Mac 正例见下一段。G10 仍缺 GUI、其余受审工具、Grok 及 Linux／Windows 已认证在线链。

`79bfce90f` 新增 G10 父子 Skill 待审批冷恢复验收及 Windows 文件提交用例的独立等待诊断；无非测试产品源码改动。Mac 本地文件提交目标 1／1、i18n 11／11、`cargo check -p warp`、嵌入资源的 libtest 构建及差异检查通过。固定官方 Claude `2.1.280/claude-opus-5-5` 已认证默认账号的 `r-b60b042f` 真实链通过：第一测试进程在子原生 Skill 待审批点退出，第二测试进程冷重接原父子宿主，核对两条原生会话 ID、保存的权限上限、原审批 ID 和用户输入数未重投；允许后子结果含技能标记，父子原生退出和清理回执完整。安全收据 SHA-256 `705f26cbf0b89f7ad32a7fdd5d5f949404f0254d81a7aff8849ea93f7d6630e2`，小原件哈希及精确 PID／launchd／文件占用清理索引 SHA-256 `8a2ad15d14ba8749507f3e61dcc7299cdd282ace815a6992c8f02d1d3d427a63`，仓外归档于 `resume-20260929/g10-cold-r-b60b042f`；短目录及专用应用 profile 已按身份清理。首次 `r-47b8b60b` 因外层预建逐轮记录而被运行器前置拒绝，测试及模型输入为零，失败收据 SHA-256 `e7eebed2ffe573eb8a236a90d36621554ebe35779f958f413eee59bcad6681f5`，该轮短目录已清理。真实正例只覆盖 Mac 测试宿主的 Skill 冷恢复，不含 GUI、其它受审命令、Grok 或 Linux／Windows 在线模型；G10 不关闭。本批没有用户可见文案变化，无需本地化变更。

目录整理本身没有修改产品或重跑历史测试；此后继续实现的输入增量及新证据单列如下。该阶段相关产品源码提交为 `64bb5f6c3988a9ca463d2d8a6d79a81eb543b447`；含该源码的精确分支提交 `301040fe3c18c3ffc806f1b048f2e448bdc81ca5` 已通过下述 Linux／Windows 普通门禁。历史通过、失败与跳过仍按原提交计证；新增工作区收据不能外推到其他版本或全部平台。

[run36503995096](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36503995096) 精确绑定 `301040fe3c18c3ffc806f1b048f2e448bdc81ca5`，Linux x64、Windows x64 作业均 success，两平台 `cargo check -p warp --lib` 与选定回归通过。Linux 主回归 3267／3267、桌面／TUI 660／660、rust-genai 81／81；Windows 主回归 3077／3077、桌面／TUI 660／660、rust-genai 81／81。两平台日志未见 `LEAK`／`RETRY`／`FLAKY`，跳过项不计通过。G03 新增的 `owned_grok_file_drop_event_reaches_batch_guard_before_editor` 在 Linux 主回归逐名 PASS；该测试使用 Unix PTY，Windows 本轮是源码编译与选定回归通过，不计真实拖放事件。12 份官方工件与 107 个完整日志成员已按官方摘要、ZIP CRC 和成员 SHA-256 归档于仓外 `resume-20260929/cloud-run-36503995096`，日志清单 SHA-256 `6a447d8bfaf0b198cfca988fb14d67b800e45b41a9979e932b83d74c5bdc3c55`。G07 成卡后失效文件回归提交 `611aac0a408d4dd5c7d3d3dc4998257abaffd8c8` 晚于本轮源码，只有上文的 Mac 定向收据；本轮未运行真实 Finder 拖放、G09 原生失败专项、已认证模型或交互式双语 GUI，14 项必要缺口继续开放。新产品代码复用既有英中提示，无需本地化变更；真实拒绝提示布局仍待验。

同提交普通跨平台 [run36490470344](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36490470344) 已结束为 success，Linux x64 与 Windows x64 job 均成功、两平台 `cargo check` 通过。Linux 主回归 3266/3266、桌面／TUI 660/660、`rust-genai` 81/81；Windows 主回归 3077/3077、桌面／TUI 660/660、`rust-genai` 81/81，另含 CLI 与相关固定边界定向组。两平台原始日志未见 `LEAK`／`RETRY`／`FLAKY` 标记，跳过项不计通过；Windows GUI 系统剪贴板步骤本轮按输入被跳过。12 个官方工件与完整 job／step 日志已按官方摘要及 ZIP CRC 归档于仓外 `resume-20260929/cloud-run-36490470344`，日志清单 SHA-256 `cf46b930ee29d735ec6670602e2f15a892cb2f73d1db7950224f5a7904bad3a7`。本轮未运行 Windows G09 的 NUL／固定 Node 专项，也不含两平台已认证模型、物理中文输入法或 Grok 专属双语视口验收，14 项必要缺口不据此关闭。

同一远端提交的 Windows 专项 [run36497300508](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36497300508) 单独结束为 failure：前置定向主回归 508/508、command 22/22、环境 14/14 通过；原生 stdio 三项中普通 CMD 正例通过，NUL 与固定 Node 根各失败一次。新增同 CMD／cwd／AppContainer token 的普通 stderr 文件正对照通过，普通输出、普通 stderr、前后标记、四个文件 FileID 分离均成立；`2>NUL` 后目标输出仍为空，但 `%errorlevel%` 为 0，不能把它记成实际 NUL 创建返回码。固定 Node `20.9.0` 根进程未执行 shim，在初始断点后自然退出 `0xc0000142`，版本输出数为 0；两个失败均有外层严格 Job 和内层 AppContainer／profile 清理确认，不证明运行功能通过。旧 AccessCheck 仅是宿主模拟，并无受限进程实际 NUL CreateFile 或目标 desktop 访问收据，不能据此合并两项根因或更改全局 ACL。三个官方工件与完整日志归档于仓外 `resume-20260929/cloud-run-36497300508`，日志清单 SHA-256 `8b27d5d445d95f5ceda69133f0bae4ed74b18057d15f3054ccc2902e4e11e966`；尝试读取被跳过 Linux job 日志的 API 返回失败，原始请求/回执保留，归档脚本随后只抓已执行 Windows job。此轮未运行真实 npm 发布或冷恢复，G09 继续开放。

后续测试专用 `97a647351fb54493adb86aab6fa76aea456d4b0f` 加入实际 AppContainer 令牌的 `CreateFileW` 普通文件／`NUL` 对照，但同提交 Windows [run36498757238](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36498757238) 没有到达这一步：私有 warp libtest 辅助映像校验后启动，根进程在初始断点后以 `0xc0000142` 退出，`helper_report_present=false`。原生 Job／profile 清理及收据匹配均为 true；外层 nextest 的 60 秒限额又将用例标成 TIMEOUT，故不能从测试标签推断 NUL 打开结果。额外标准流证据工件因没有匹配文件上传失败，其他两个官方工件与完整日志已归档 `resume-20260929/cloud-run-36498757238`，日志清单 SHA-256 `df8d021420a14d971e44a3e7be090821628490c45f0f20e515d1c8962bf5d30f`。本地对后续 `c31d61fa70a2d23b5ecafd4ae2a2d28a3fb01be7` 的 `cargo check -p warp`、改动文件 rustfmt、actionlint 忽略仓库既有自托管标签及 JSON/TOML 解析通过；新提交保留原失败断言、仅给大型诊断独立 150 秒 nextest 预算，并加入现有非交互窗口站的私有桌面测试模式，不改生产默认隔离。该模式的 Windows 原生结果见下文同提交 run；G09 开放。[微软状态码表](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/596a1078-e883-4972-9bbc-49e60bebca55)将 `0xc0000142` 定义为 DLL 初始化失败，但当前日志尚未定位具体 DLL 或失败权限对象。

同提交 [run36499906695](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36499906695) 已在 Windows x64 结束为 failure：`cargo check`、前置主回归 508／508、command 24／24、未命名站 1／1、环境 14／14 通过；显式原生两项 0／2、零重试。大 helper 的 `CreateFileW` 收据仍缺失，根进程 `0xc0000142`，原 Job／AppContainer/profile 清理收据匹配；新的 150 秒 nextest 预算使失败按 FAIL 记录，未再误标 TIMEOUT。固定 Node `20.9.0` 的现有非交互窗口站私有桌面由本轮创建，实际对象名称、描述符与低完整性标签通过读回核验，桌面句柄按本轮所有权关闭；根进程依旧在初始断点后退出 `0xc0000142`、无版本输出，同样完成严格 Job 与 profile 清理。因此私有桌面建立成功不等于 Node 初始化成功，也不提供 NUL 的直接调用结果。没有修改现有窗口站 ACL、产品默认隔离或执行真实 npm 发布／冷恢复。完整 job/step 日志和 3 份官方工件已按源码 SHA、官方摘要与 ZIP CRC 归档于仓外 `resume-20260929/cloud-run-36499906695`，日志清单 SHA-256 `bdd5d3509e7b37c7e67eb5433bedfb92b9931eda61532a981899225949508544`。G09 保持开放；本批只改测试／诊断，没有用户可见文案变化，无需本地化变更。

小型 Kernel32-only `CreateFileW` 夹具的首次 [run36501584604](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36501584604) 精确绑定 `10f99d6df9b8720548c0611c780b07e08d9ae1bb`：runner 用本提交 C 源与 MSVC/SDK 构建，小程序在普通 runner 令牌下实际退出 0，并取得有效 120 字节基线收据；源文件及产物 SHA 均进入日志。随后 `warp` libtest 的收据解析闭包报 E0277：`Vec<u32>` 被 `i32` 索引。Agent 生命周期测试、Grok 安装及原子诊断步骤均因这一处测试编译失败；未启动受限 AppContainer 的夹具，不能得出 NUL 访问结果。Grok 附加工件因未生成收据而上传失败，这不构成 Grok 原生安装失败的证据。完整 job/step 日志与唯一成功上传的官方环境工件归档在仓外 `resume-20260929/cloud-run-36501584604`，清单 SHA-256 `1b84e9652376a3c5270be33f57b195cc1737100b51137a0bd2006bdced922fa7`。修正提交 `2d3297089b550e665423d011c82f8bb2b0c2ebdf` 将两个闭包调用索引显式设为 `usize`；首次失败仍保留。

同一修正提交的精确 Windows [run36502343517](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36502343517) 已整体 success：前置主回归 508／508、command 24／24、未命名站 1／1、环境 14／14、原生 CreateFile 1／1。小夹具源 SHA 与 runner 构建产物 SHA 均记录，普通 runner 令牌基线通过；受限的私有副本在零 capability AppContainer 和严格 Job 内正常退出 0，`helper_report_present=true`、`helper_receipt_valid=true`、清理与收据匹配均为 true。固定 120 字节回执显示：同目录普通文件在 `GENERIC_WRITE` 和 `FILE_GENERIC_WRITE` 下均打开、磁盘类型为 1、关闭成功；`NUL` 与 `\\.\NUL` 在两种掩码下均打开失败，紧随调用的 Win32 错误码为 5。该直接观测明确了此 runner 上受限进程的 NUL 访问失败；未重跑固定 Node 或真实 npm 事务，不能将 NUL 拒绝当作 `0xc0000142` 的原因。完整 job/step 日志及 2 份官方工件按原始摘要和 ZIP CRC 归档于仓外 `resume-20260929/cloud-run-36502343517`，清单 SHA-256 `d6e57a4c32313b37cf913407ce328194c5b9fe251df807f59ed304e4810890f8`。G09 保持开放；这两次提交仅修改测试／诊断，用户可见文案无变化，无需本地化变更。

**2026-09-29 输入防护与验收门禁提交 `2383428dac785fc34ed44120226b596d592ad191`**：补强 Grok 专属普通终端的回调／恢复身份、远端图片旧回调隔离、Claude 图片双技能在线验收、文件卡片与技能入口回归，并加入 Windows 诊断、SSH／tmux 收据和 Linux／Windows 双语 GUI 验收脚本；修复 TUI 中文占位提示按字宽覆盖时的重叠。macOS arm64 最终源码的定向 Rust 8 项、i18n 11 项、`cargo check -p warp`、Windows command 目标检查、相关 Python 脚本测试和 TUI 英中真实 PTY 画面均通过。源码无新增或变动用户可见文案，无需本地化变更；TUI 英中布局已实看。[run36453314472](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36453314472) 已结束：Linux x64 成功，Windows x64 的编译、主回归及桌面／TUI 660 项、rust-genai 81 项通过，但 Windows 原子进程专项诊断 3 项中 1 通过、2 失败，故整个 run 为 failure。`cmd /NUL` 对照的预期文件为空；固定 Node `20.9.0` 根进程在初始断点后以 `0xc0000142` 退出，无版本输出。两例的严格 Job 清理均确认，不能把清理成功计成功能通过。另一个未命名窗口站探针虽测试通过，实际 `created=false`、HRESULT `0x800700b7`，没有建立目标窗口站。原始 run/job 日志及 13 个官方工件已归档，索引 SHA-256 `0b79abff203bf57c78482eabe0d9d36249d9e6f40aa2f88cfda53fa1e4d49500`，Windows 原始 job 日志 SHA-256 `f04fbea67e6bd9a2b4b5b437e167c27f022b9f4c1928df2d5f8156a491771f05`；G09 继续开放。后续当前分支 `12adb9f2eceb155d9e9c32312bdd3c6f428a4c57` 的普通双平台门禁 [run36467106568](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36467106568) 已整体 success：Linux／Windows 主构建和选定回归通过，Windows 主回归 3069／3069、桌面与 TUI 660／660、rust-genai 81／81。Windows 监督清理组 5 项虽全部通过，其中 1 项仍被 nextest 标为 LEAK，不能据整体成功宣称无泄漏。本轮未运行 G09 已知失败的 NUL／固定 Node 专项诊断；亦早于未提交的 Grok 结果落盘改动。原始 run/job 日志、12 个官方工件及逐文件 SHA／CRC 已归档于仓外 `resume-20260929/cloud-run-36467106568/`，日志索引 SHA-256 `afa1de7d8aa4066a8f3bce8b3fad17c0a640d938dd037eb80c096aafe953026d`，Windows 原始 job 日志 SHA-256 `fd0bec2e99b99e7062742719cb823176b01880483277875c49e3249cbe682cf2`。

当前分支的 G09 后续只增加诊断对照：在同一 CMD、目录和令牌的原 NUL 用例前加入普通文件 stderr 重定向，并检查其输出、空错误流和四个普通文件的不同 FileID；原 NUL 失败断言保留。该源码尚未在 Windows 同提交专项运行，普通文件对照不能预记通过；现有证据也不能将 NUL 和固定 Node 的失败判为同一根因。此测试改动无需本地化变更。

同提交的固定 Claude Code `2.1.280`／`claude-opus-5-5` macOS 生产适配器在线验收通过：新建与同原生会话冷恢复各提交一张不同 PNG，两个已登记技能每轮均取得精确 Skill 审批和执行，原生投递、结果及两代清理收据通过；收据中的源码工作树干净，两个程序摘要和关键源码摘要前后保持。原件在仓外 `resume-20260929/claude-multi-2383428-05`，收据 SHA-256 `70856b39cc8280b172d02a46ec8bff01ddc02a053dee6d501b12d757255a615a`，事件 SHA-256 `90854acb55c75dc5fd3eb836790e096fe5405bc14424d5f225469954e1fc69e7`。前四轮因调试版 launchd helper 读取外置源码本地化资源阻塞而在10秒握手处失败，抽样进程栈保留；按[本机测试存储约定](../../docs/local-test-storage.zh-CN.md)用现有 `warpui/test-util,rust-embed/debug-embed` 特性重建后通过，未调整产品超时。五轮测试临时目录均经专属服务、进程与文件核验并在归档后清理。该正例不覆盖实际 GUI、Linux／Windows 或 Claude 其他格式和权限组合，G04–G06 继续开放。

同签名二进制的独立 Mac GUI 又完成一次新建任务的 Claude 图片加双技能正例：固定 CLI `2.1.280`／`claude-opus-5-5`，GUI 依次对 alpha、beta 两个 Skill 选择 Allow once。持久化输入依次为 Text、LocalImage、Skill、Skill；141 字节 PNG 的源文件、受管文件和原生 transcript 中 base64 解码结果逐字节一致。两个原生 Skill 调用与各自结果按顺序出现，最终模型文本与任务结果一致，native／adapter／journal 均正常结束。收据在仓外 `resume-20260929/claude-gui-image-two-skills-2383428/receipt.json`，SHA-256 `b18f344a865ed39e74ac8a75cb75e4cbcb4e8fc1ed2f21ee8d4698c68d697c80`；原生 transcript SHA-256 `04e2886ada0a4f3c11b16e20299666b0f7b5bbd522344a00f79a1485ac04c5d6`。随后尝试 GUI 冷恢复时，CUA 因同 bundle ID 的归属不明旧实例无法精确绑定，在发送原生输入前停止；仅本轮专属进程和临时目录经身份核验后清理，旧实例保留。此正例仅覆盖 Mac GUI 新建任务的一轮 PNG 与两技能，不把生产适配器冷恢复收据外推为 GUI 冷恢复，也不覆盖其他格式或 Linux／Windows，G04–G06 保持开放。

同源码提交的 Mac 独立签名 GUI 版（Grok Build `1.0.41`／`grok-4.7`）从专用新标签入口自动启动受管理 TUI。普通未绑定 TUI 的富输入保留草稿并明确拒绝自动发送；专用入口在同一原生会话完成英文和中文两行文本两轮投递，界面显示原文、两次模型回复及自动清空草稿。第一轮精确标记匹配；第二轮模型将末尾“第二轮”回答为“第二行”，只计完整中文输入投递成功，不计精确内容回答成功。应用及测试进程正常退出，临时目录核验后已清理。独立版二进制 SHA-256 `55bf3ae2cfd9ebc21f1758284fbf6974a2a883f06d7fee36cb6a312a3a307604`，仓外 `resume-20260929/grok-gui-2383428.json` 收据 SHA-256 `48c0f5683568f4602f2c72e45f0fcb1e37677694bccc2c9919d6aec13e8962ca`；本轮未另存画面文件。图片粘贴、审批及 Linux／Windows GUI 仍未验，G01／G03 继续开放。

随后以同一签名二进制和隔离 profile 重启，从“Local CLI tasks”继续上轮已断开的 Grok 任务。任务 ID `82d92393-4956-4506-b5e4-fa53e2ea96c3`，恢复前后原生会话 ID 均为 `c45a7ae3-f82e-4ac6-a01c-c8bbcfead5f4`；原生 TUI 历史仅显示原两条输入各一次，新 run 2 的第三条文本取得原生回执并回答 `4`。历史显示旧会话关闭时的 stop／session_end hook 警告，任务面板在断开 run 1 时没有保存最终结果；第三轮继续链通过不掩盖这一边界。仓外 `resume-20260929/grok-gui-cold-2383428.json` 收据 SHA-256 `4f3a5158d21fd1cab7bd8ff288d787c248a6257942fb427a3ffe9ed9499ea519`；第二轮临时目录也已按身份、进程和打开文件核验后清理。未另存截图文件；本段不覆盖图片、权限审批和其他平台。

对这份私有 profile 的后续只读检查确认：第 1 代两条、第 2 代一条输入均为 `NativeProtocol/end_turn` 已确认，两个 task 代的 `result` 仍为 `None`。专属 TUI 没有经过常规 pane 的 `bind_local_task` 结果链，原生 ACK 也不携带回答正文；不能从终态或屏幕可见文本直接伪造持久结果。需对相同已认证 leader 的 `_x.ai/session/updates` 做完整回放、水位与身份核验，再以对应旧代 CAS 落盘，并覆盖 ACK 后退出和新旧代竞态；单独补绑定有 checkpoint revision 竞争风险。此诊断没有修改产品代码，G01 结果持久化缺口保持开放。

当前分支随后加入完整原生历史核验与任务代次 CAS，修复该窄范围结果持久化：固定 Grok `1.0.41/grok-4.7` 的 Mac 专属 PTY 双回合逐次取得原生 ACK、最终正文及完成水位，已核验结果落入 SQLite 并可由退出后的独立进程读取。另一轮在两次 ACK 后使首个应用侧测试进程退出，保留同一 TUI/leader，再由第二进程重开 SQLite 并仅通过只读历史补写旧代和当前代各 44 字节；消息数及原生用户输入均为 2→2，第二次补查幂等。原生退出后第三进程重新读取两代结果，退役启动记录拒绝重投。此轮 `g01-native-cold/r-G5hcBT9a` 索引 SHA-256 `6264fbdc909f5c25392e33211abd7a1114187f33bc570b5a3808f5f220335bd7`；另有正常双回合原件 `g01-native-live/r-0JKMVTFy` 索引 SHA-256 `f34756cb075b4901ae981cd87f3ea92251b7bc36867a935abbc07c3b3e3e6677`。定向 nextest 44/44、i18n 11/11、`cargo check -p warp` 通过。新英中结果未核验提示已同步，但未做两语 GUI 截断检查；普通未绑定 PTY、图片、审批与 Linux／Windows 尚未验证，G01 仍开放。本段绑定执行时未提交的工作树，不外推为后续提交的 GUI 或跨平台结论。

G01 普通未绑定 PTY 的历史只读可行性复核显示，固定 Grok `1.0.41` 的 `idle_prompt` 可在帮助模态、未提交草稿或后台工具期间出现，空会话超过 66 秒也可能不出现；权限提示不能保证自动 Enter 安全。当时未取得普通 TUI 可认证的 leader socket 接入、编辑器修订或模态／审批状态原子提交合同；当前代码 `app/src/terminal/view/use_agent_footer/mod.rs` 的拒绝分支应保留。此前将后续工作归为“等待上游提供接口”的判断已由本报告的 2026-09-30 官方源码更正取代：现在可以在公开源码实现事务，但仍须独立构建和真实普通 PTY 验收。此历史复核未改源码、未发送模型输入，不计 G01 关闭。

G03 当前分支修复专属 Grok 富输入的文件拖放：两张图片按原顺序形成附件，含非图片的整批拒绝；已打开富输入但 owned 身份失效时也必须在 PTY 前返回。新增测试先在旧路径捕获 1 次 `WriteBytesToPty`（exit 101），修复后同一用例的双图、混合、纯非图和失配零写入 1/1 通过，`cargo check -p warp` 通过。英中拖放拒绝文案已同步。首次私有 GUI 用 Preview 粘贴出两张卡后，因人工提前按 Return 先运行了普通未绑定 Grok，提交触发原有“未发送”保护提示；任务仍 queued／revision 0、消息数为零，`retired.json` 标记未派发。该负例保留，不作为专属图片模型正例。

随后以同源码重新签名的独立 Mac GUI，仅点击专属 preset 并核对 `dispatched`／`exec`／`bound` 三份清单一致，固定 Grok `1.0.41/grok-4.7` 的同一原生会话启动。富输入依次粘贴红蓝与绿黄两张不同图片，保留中文提示，只按一次 Return。SQLite 输入 seq1 为 `acknowledged`、`native_protocol`，原生历史仅一条 prompt index 0，内容为文字＋图片＋图片；两张 inline base64 解码 SHA-256 依次为 `53964b451c618826c8622f1eeb9d910a9895b771421094b10d1aac31af65df84`、`9d1bd4d71b62dd534905031128c0c2ba7880006d3abb5d477c7bc3a6298e1d68`，分别与产品持久附件及原生 assets 逐字节相等。粘贴后的 PNG 编码与 454 字节原夹具不同，但两张 256×128 图片的像素分别完全相等。模型回复精确 `RED BLUE GREEN YELLOW`；同一任务的 `result`、消息的 `grok_terminal_result.output` 和 `completion_watermark` 均已保存，native prompt 身份一致。原件位于仓外 `resume-20260929/g03-dual-image-gui/r-VdeRyH7I/`，`index.json` SHA-256 `1ae3c40a655b6842d1f7e0ed34aa2f81479348855182f6d1f400ceef08d092a7`、`evidence.json` SHA-256 `213d0b126e395bf3c5f46e07ea0155c5e9c99f73501f25d149065105d65857e1`；五个私有 PID 和 launchd 标签均退出、文件占用为空，短目录按身份核验后清理。旧失败轮 `r-2DGwnljo` 因系统 `tccd` 仍以只读句柄占用旧测试包而保留 `cleanup_ready=false`，没有结束系统服务或删除。此正例不证明 Finder 拖放实际触发、普通未绑定 PTY、审批状态变化或 Linux／Windows。新双语拒绝提示的 GUI 截断检查仍待补，G03 不关闭。

G03 另以新私有签名 Mac GUI 只补 Finder 拖放：固定 Grok `1.0.41` 的 `launch`／`exec`／`bound` 清单匹配，富输入已打开；两次 CUA Finder→应用 `drag` 都只移动了 Finder 内的选择，应用侧没有可观察 drop event、图片卡或 toast，SQLite 保持 queued／revision 0／消息 0，模型请求数为 0。安全原件位于仓外 `resume-20260929/g03-finder-drop/r-d_2ajoia/`，`evidence.json` SHA-256 `4524907fdd592b8d8c4cdc3561d6bae2e75f6fbe8ad80e5d985219d012f5e820`、`index.json` SHA-256 `c870674c58bfe26b5e0067591db47f82178928782d95f4b17825b316fdcda5d6`。CUA 工具本轮未交付目标拖放事件，不能判定产品代码失败；英中拒绝文案只作静态核对，未取得真实布局截图。私有进程、launchd 和打开文件均清空，记录先置 `cleanup_ready=true` 再核验删除短目录；旧 `tccd` 占用目录不动，G03 继续开放。

G03 的 CUA 手势又以隔离夹具在 Finder 内校准：列表视图拖到目标文件夹后源文件仍在原位；图标视图拖动后只选中了目标文件夹，`destination` 仍为空。因此该工具没有证明能交付本机 Finder 文件拖放，不能将上述应用零事件归因于产品。测试未启动应用、未调用模型；源文件 SHA-256 和目录身份记录于仓外 `resume-20260929/g03-drag-calibration/g03-drag-656c43b2.safe.json`，收据 SHA-256 `3dbb7e4df18ee5c0f531e74c7fca9da4de05b8c025b9b82078ced9e77847daa3`。Finder 测试窗口关闭、`lsof` 无打开文件，专属短目录按记录清理；G03 仍需真实原生事件验收。无需本地化变更。

后续同分支 `64bb5f6c3988a9ca463d2d8a6d79a81eb543b447` 修复了另一个可定位的 G03 事件路由问题：富输入 editor 原会先消费完整 `DragAndDropFiles`，把混合批次拆成图片附件和非图片路径，导致顶层已有的 owned 身份与整批拒绝守卫没有运行。现在仅在专属 Grok 富输入打开时由外层先派发完整批次到守卫，外层不发 terminal resize。Mac arm64 的真实渲染树模拟窗口事件测试明确聚焦 editor、从 SavePosition 得到拖入坐标：混合文件命中原有整批拒绝 toast，附件与草稿不变；纯图附加一张；随后标记 owned 身份失效，另一张图片命中不可用 toast 且未附加；`WriteBytesToPty` 计数为零。新旧两项定向测试各 1／1、i18n 11／11、`cargo check -p warp` 通过；仓外安全收据 `resume-20260929/g03-event-routing/r-c4ddbd5f.safe.json` SHA-256 `66786d39135ed3a2a09fde8c947e6e0204cc4b42020644e839eeb95d62e28560`，包含当前差异 SHA、进程与文件占用核验，短 TMPDIR 已按记录清理。英文与简体中文两条既有提示的语义和变量已复核，无新增用户文案，无需本地化变更。模拟事件不等于 Finder 原生跨应用手势；双语真实 toast 布局及 Linux／Windows 仍待验，G03 不关闭。

G10 的固定 Claude `2.1.280/claude-opus-5-5` Mac 生产父子链另完成待审批 `Write` 取消验收：父任务 `run_agents` 获单次允许，子任务原生精确 `Write` 在目标文件不存在时进入待审批；子 `Interrupt` 获 ACK 后审批撤销，同一审批 ID 的迟到 `AllowOnce` 由原生请求失败拒绝。持久化父子代次和权限上限核对通过，目标文件始终未创建，父子原生进程各有退出和 Job／资源域清理确认。通过轮 `g10-v2-write-cancel-internal-all-cleaned.safe.json` SHA-256 `7735444452dbf08991f748653ee7455703a3a30bf7194c94a82ee93e2b375386`；6 份小原件已按哈希归档，短 `r-*` 在精确 PID、launchd label、lsof 和目录身份核验后清理。此前两轮外置盘 worker 握手失败、一次外置盘 Claude CLI 初始化超时及对应未确认清理原件独立保留；无模型采样显示外置 worker 在 dyld 打开二进制阶段延迟，内置同 SHA worker 与 CLI 的无模型对照分别通过。通过轮只更换同 SHA、签名及固定版本的私有测试执行位置，未改产品超时、权限或原生 CLI。此证据关闭 V2 待审批取消的 Mac 子场景，不覆盖 G10 的其它固定工具／技能权限、GUI、冷恢复和 Linux／Windows，G10 仍开放。

后续产品提交 `bd4612887a864196744721621f29fa5d5c57e3b1` 修复两处失败边界：专属 Grok 输入会话消失或租约登记失败时显示已有双语错误提示、保留草稿；Codex／Claude 在锁定 Shell 模式附加文件卡时，拒绝把路径说明送入原生命令模式。提交前同内容工作区的两项定向回归、i18n 11 项及 `cargo check -p warp` 通过，测试分别验证明确提示、中文／英文空格路径双卡、草稿与卡片保持和 PTY 零写入。原件位于仓外 `resume-20260929/gates/`，三份日志 SHA-256 依次为 `bd2eefcca51d62189b931db5b9b439da5516349643388712ba3bf11c928c7232`、`1c1dc038d409c8168701e1f9bdef5efbfa741f251e2b9b51b511d45df76d1408`、`f644ac29b8532baf8e638b7b891f902d3f5f28c307bf0b69100e9959fbaedbfc`。测试临时目录均在退出和文件占用核验后清理。复用既有英文与简体中文提示，无需本地化变更；未对新提示场景另做 GUI 布局验收。原有 Grok 普通未绑定 PTY 门禁、G07 三款真实链及目标平台验收仍开放。

G07 的独立 Mac 私有 GUI 随后验证了收起态选择文件入口：系统选择器返回中文路径及英文空格路径，两者分别形成文件卡；锁定 Shell 模式输入 `!printf` 后出现拒绝提示，草稿与双卡保留，原生输入框没有新文本或输出块。本轮 GUI 未获得精确 PTY 字节计数；既有同域单测覆盖 Codex／Claude 的零 `WriteBytesToPty`，不能替代该轮 GUI 的原生计数。原生 Codex 为 `0.155.1`，所以不计固定 `0.156.1` 的文件读取正例，也未覆盖 Claude 首次向导。CUA 画面未保存成本地原图，只有工具回执；私有应用进程及短测试目录经身份和文件占用核验清理。仓外 `g07-gui-r-hq0ITed6/receipt.json` SHA-256 `71d082c6866d8c0e8b5a9b317939863054b946ad10da0dc30bbf6e86356b2a5b`。无产品源码或文案改动，G07 继续开放。

G07 随后在本分支签名 Mac GUI 内使用从官方 G09 归档私有复制的固定 Codex `0.156.1`，原版与复制的签名二进制 SHA-256 均为 `0196e89fe5a7598f816ee54232c3d7c26d75e502ab5cfe2c9240e81d90f7255a`，独立 `CODEX_HOME` 的登录状态可用，日常 `0.155.1` 入口未改。从收起态系统选择器生成 `English spaced.txt` 与 `中文 文件.txt` 双卡后一次提交；原生 rollout 同一 `exec` 调用中的两条独立 `tools.exec_command` 分别 `cat` 两份文件，工具输出返回各自合成标记，最终文本逐行匹配，卡片与草稿清空。脱敏正例 `g07-fixed-codex-r-fdZbGmBG/positive.safe.json` SHA-256 `ea732f8c52e6b277b5b6a45b5c09ffae00dd3310e053545c4be828b72932b062`。同会话再以双卡及锁定 Shell `!printf G07_NEGATIVE_SHOULD_NOT_EXECUTE` 提交，出现拒绝提示，草稿和卡片均保留；原生 rollout 哈希前后相同，用户消息、工具调用、任务启动计数不增加，标记未进入原生会话。负例 `negative.safe.json` SHA-256 `b3ab89b9a16ddac5f4666f6e18d2c0661e78aca6ca5ccd3072e555fdbe18aee2`，整轮安全索引 `index.safe.json` SHA-256 `139bf593d75de9bdf57e8340721f702689d72e77f9ab58ab2524b541e78ef95a`。四个私有 PID 精确退出，launchd／lsof 无占用，测试记录为 `cleaned`／`cleanup_ready=true`，短目录已删除；原生 raw session 因含其他上下文未归档，CUA 截图只在工具回执。此正反链不把 GUI 未采得的精确 PTY 字节数或 Claude／Grok 和其他平台计作通过，G07 仍开放。

G07 固定 Claude `2.1.280` 随后在本分支签名 Mac GUI 的私有配置中进入首次向导，确认实际 `which claude` 指向本轮官方签名版本，显示默认主题后到达 Claude subscription／Console／第三方登录选择。独立 `CLAUDE_CONFIG_DIR` 的原生认证状态未登录；即使复制本地 `.claude.json` 到私有目录也未形成有效登录。该轮没有选择认证方式、生成文件卡或发送模型输入，因此不能作为 Claude 文件读取或拒绝验收。仓外 `resume-20260929/g07-claude-r-5d36ab53/blocker.safe.json` SHA-256 `30226efd0f11c055988735fd7f965531acbb7bcfa2f75477b0367eedad6b3ed9`，索引 SHA-256 `d95d5da8e32099fb27df77fd383965735ca34b245b00428a5cd9a4f11ecebf1f`；GUI、原生进程、launchd 与打开文件均核验退出，短目录删除，日常配置大小与修改时间未变。截图只在 CUA 工具回执，没有归档本地图；无产品文案变化，无需本地化变更。G07 仍开放。

后续测试提交 `611aac0a408d4dd5c7d3d3dc4998257abaffd8c8` 把 G07 文件卡拒绝回归改为真实时序：Codex／Claude 先各建立两张本地文件卡，再删除第二文件；权限用例也在成卡后才把第二文件改为 mode 000。Mac 合规短 TMPDIR 下 `file_submission_tests` 6／6 通过，失效提交均保留草稿和双卡、`WriteBytesToPty` 零事件；提交前 `cargo check -p warp` 通过。安全收据 `resume-20260929/g07-dfaec474/receipt.safe.json` SHA-256 `9eaec29c252d67428dbeb0a84eea77cb321e1f4004c5b44baae56fe78a80ffbf`，测试日志 SHA-256 `5764ee7e7d6947aed36c6ff6c67643b106bb3ffc8884b04a87bd4579db09b315`；记录确认私有进程退出及临时目录清理。此回归证明提交前已失效文件整批拒绝；当前交付是路径引用，校验通过到原生 CLI 读取之间仍有外部文件改变的时序窗口，不能声称三款 CLI 文件读取或 G07 全部条件已验。仅测试变化，用户可见文案未变，无需本地化变更。

[run36521497826](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36521497826) 以 Windows x64 定向模式精确验证 `e745a1a9dec1b8ad38ea7afb764bc251aeaecdc8`：`cargo check -p warp --lib` 成功，主定向组 508／508 通过，其中 `file_removed_after_card_creation_keeps_entire_composer_without_a_partial_pty_write` 逐名 PASS；用例内部对 Codex／Claude 分别成卡、删第二文件、提交并检查草稿／双卡与零 PTY 写入。原始日志未见 LEAK／RETRY／FLAKY。2 份官方工件及 21 个完整日志成员按官方摘要、ZIP CRC 和成员 SHA-256 归档于仓外 `resume-20260929/cloud-run-36521497826`，日志清单 SHA-256 `6552bf404d7c4e5d6e636e1705347271d44086b7b4b46106408339b335d613f5`。Linux 被该轮显式跳过；Windows 实际 GUI、原生 CLI 读取、权限 ACL 和 G09 原生失败专项也未选，G07 与 14 项必要缺口仍开放。仅补验测试，无本地化变更。

Windows ACL 测试首版 `4cde8d8c31e3e8953da21939cfeb7ce52e0e883e` 在 [run36522975704](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36522975704) 编译通过，但把私有临时文件 DACL 设为空后 `File::open` 仍成功，夹具的拒绝前提不成立；主组 509 项 508 通过／1 失败，失败项三次尝试均在前提断言处停止，不能据此判断产品提交行为。2 份官方工件和完整日志归档于仓外 `resume-20260929/cloud-run-36522975704`，日志清单 SHA-256 `d3d7506afb5fc4d89eae35037fa8dcfc2f6225a5c0995b9abd16539dda5d49a4`。修正版 `5b3246c6f15d28f532b4d6f140b9c12f6ccd04be` 改用临时文件独占句柄，先核 `ERROR_SHARING_VIOLATION=32`，再验证 Codex／Claude 双卡提交整批拒绝、草稿和两卡保留、PTY 零写入；释放句柄后原内容可读。[run36523921149](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36523921149) Windows job 成功、`cargo check -p warp --lib` 通过，定向 509／509，其中新用例逐名 PASS；完整日志无 LEAK／RETRY／FLAKY。2 份官方工件与完整日志按官方摘要、ZIP CRC 和成员 SHA-256 归档于仓外 `resume-20260929/cloud-run-36523921149`，日志清单 SHA-256 `f15bb52a05ddc76dbfe7157e6d1dd6d11d6db5b669e4eae222db962224a1af8b`。Linux 被显式跳过，测试只证明 Windows 文件共享冲突的提交前拒绝，不等于 ACL、GUI 或原生 CLI 文件读取；G07 保持开放。仅测试变化，无需本地化变更。

Claude 图片加技能的插件刷新竞态回归 `074cbb5fb` 在 Mac 合规短 `TMPDIR` 下定向 1／1 通过：提交等待 `reload_plugins` 时移除图片，随后即使原生插件返回成功确认，本轮仍以 `RequestFailed` 结束；无用户帧、原生写入或任务轮次。仓外 `resume-20260929/claude-reload-final-b72ed49b/receipt.safe.json` SHA-256 `2c1b63abca706e77905052952acf45253ef1ce380852c60dc412d08d834159a4`，测试日志 SHA-256 `b93ef5e98f41b21134b2f0e715c2167922611c3dba45f4c49008983a0f9c73e3`；短目录在核验后清理。此测试只覆盖输入修改和插件 ACK 交错时的失败原子性，不代替 Claude 在线组合、GUI 恢复及目标平台验收；仅测试变动，无需本地化变更，G05／G06 保持开放。

V05 首轮 [run36525713293](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36525713293) 绑定 `b705868bf2b61d2d186691ced1273a2d8a3525fc`：Windows `cargo check` 和前置选定回归通过，真实集成窗口在等待扫描固定 Grok `1.0.41` 时失败，日志中实际扫描路径为 `grok.EXE`，测试预期为同一路径的 `grok.exe`，失败发生在第一张英文紧凑视口截图前。Mesa 版本及 DLL 摘要已核，但 `renderer.safe.json` 标记 `outcome=failed`、`v05_complete=false`，不能称布局通过。首轮 Linux 的双语资源测试与集成编译通过；启动修正后的同分支工作流触发并发取消，Linux 在旧的系统剪贴板 GUI 步骤中停止，未进入 Grok 视口。Windows 失败和 Linux 取消的原始日志、4 份官方工件按官方摘要和 ZIP CRC 归档于仓外 `resume-20260929/cloud-run-36525713293`，日志清单 SHA-256 `4f69df44e42d14dde58132648e1ac8201b587d22a8bff1d8a6e5a931ed27e400`。修正提交 `15e329fcf54e5fb987ca30bb4ad6fe8c1f9c9d77` 仅让 Windows 测试断言按大小写无关方式比较扫描路径，Mac `cargo check -p warp` 和 `cargo check -p integration --bin integration` 通过；同 SHA Windows 专项 [run36527383544](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36527383544) 已发起，结果另计。Linux 须在该 run 完成后串行复测；无需本地化变更，V05 仍开放。

上述 [Windows run36527383544](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36527383544) 最终为 failure：编译、前置定向回归和固定 Grok 安装均通过，真实 Mesa 窗口已在英文 800×600 产生继承、只读、文件及技能策略四张 PNG；但底部滚动测试直接设定 `100000` 像素目标，`ClippedScrollStateHandle` 的布局路径在目标高于内容时重置到 0，断言记录 `offset=0,tasks=0`，故未采到滚动区三张及中文／常规视口。四张原图显示策略按钮可读，顶部说明因当时滚动位置只露出后段，尚不足以判断完整双语布局。原始日志及 3 份官方工件按 SHA／CRC 归档于仓外 `resume-20260929/cloud-run-36527383544`，日志清单 SHA-256 `a4db571fb4f53172cad87f17c7de18c6358b4b23eb7201fb4b028464d3a8b9c2`。修正提交 `bed8c1276ff950b6fa09ba1767c015363a9655b1` 仅在测试中逐段滚动并要求最后两段偏移相同，调整策略截图位置以包含说明；本机 `cargo check -p warp`、`cargo check -p integration --bin integration`、改动文件 rustfmt 通过。精确 SHA 双平台 [run36529053069](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36529053069) 已启动；其结果与截图人工复核另计。测试改动无需本地化变更，V05 继续开放。

[run36529053069](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36529053069) 的 Windows 固定 Grok 安装、`cargo check` 和 509／509 项前置回归通过；GUI 集成测试编译因新增 `advance_v05_grok_scroll` 未在 Linux／Windows 条件模块重新导出而报 E0432，未启动本轮窗口或截图。Mac 编译没有覆盖这一平台条件导出。Linux 已通过本轮双语资源测试，在已知同一缺陷的 GUI 集成测试编译阶段主动取消，避免继续占用 runner；不能记作 Linux Grok 测试失败或成功。原始日志与 3 份官方工件按 SHA／CRC 归档于仓外 `resume-20260929/cloud-run-36529053069`，日志清单 SHA-256 `874f4214b46411c7d1fe94d4dee2e1494e775d93112b337a664d49973231bb50`。导出修正 `7f847473ff9a02b73334316d5014f3d8af75ce00` 已通过本机 `cargo check -p warp` 和改动文件 rustfmt；双平台精确提交 [run36530752934](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36530752934) 已发起，结果另计。该修正仅涉及验收工具，无需本地化变更，V05 保持开放。

同提交 [run36530752934](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36530752934) 的 Windows x64 job 已 success：测试实际扫描固定 Grok `1.0.41`，在真实 Mesa 窗口内分别以 `en`／`zh-CN` 和 800×600／1280×800 采集四组、每组 7 张 PNG；四份收据的底部滚动偏移依次为英文紧凑 695.6001、英文常规 515.6001、中文紧凑 638、中文常规 458 像素，窗口尺寸与语言均逐项核对。官方截图工件 ZIP SHA-256 `e9be5a5d9baf7d3ebda74f6c2a86c2389b5f1864bbd34035cb7e74bbf1a95d53` 已与 GitHub 报告摘要相同并通过 ZIP CRC；原图目视复核了继承／固定权限按钮、固定文件及技能禁用说明、顶部安装摘要和底部任务动作，在这四组图中未见文字重叠、溢出或操作不可达。滚动位置的局部裁剪是该滚动容器的正常显示；各区域由顶部、中部与底部原图联合覆盖。该测试没有模型输入，`approval_states_exercised=false`、`visual_review_required=true`、`v05_complete=false`；只计 Windows 静态策略布局，不计真实审批交互或 Linux 布局。Linux 静态布局结果与整轮归档见下文；V05 不关闭。

同一 [run36530752934](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36530752934) 的 Linux x64 job 也 success：真实 Xvfb/Mesa 窗口使用固定 Grok `1.0.41`，英中两语的 800×600 与 1280×800 四组各保存 7 张 PNG；收据底部偏移依次为英文紧凑 711.2000、英文常规 531.2000、中文紧凑 653.6001、中文常规 473.6001 像素。官方原图工件 ZIP SHA-256 `fd81ab5664f88cf27fc29ca1c9e7fff0402059ce89fe02a6b763611d6ae88e9f` 与 GitHub 摘要一致，ZIP CRC 及逐文件 SHA 已核；目视检查固定读／文件策略、技能禁用说明、安装摘要、顶部与底部按钮，四组图中未见文字重叠、溢出或操作不可达。Linux／Windows 完整官方 job／step 日志、6 份工件及全部成员已归档于仓外 `resume-20260929/cloud-run-36530752934`，日志清单 SHA-256 `3904682a1a6e3077ea97d6a6d32c1dad0e0a17863ea4f1a8ba20bc2f0c365cca`；两平台原始日志未见 LEAK／RETRY／FLAKY。整轮只通过固定 Grok 静态双语布局，不含模型回合、原生审批请求或实际点击审批；八份收据均为 `model_inputs=0`、`approval_states_exercised=false`、`v05_complete=false`。V05 整项仍开放；测试代码无新增用户可见文案，无需本地化变更。

V03 的 Codex 关闭 tmux 透传负例另以本机 macOS 回环复验：固定原生 Codex `0.156.1`、参考通知插件 `0.4.0`、私有 tmux `3.7c`、独立 sshd 与 PTY，`allow-passthrough=off`。原生 `SessionStart` 和 `UserPromptSubmit` 两个 hook 均退出 0、关联本次 Codex 祖先链，pane 内取得同原生 session/turn 的两条通知；外层 SSH 原始输出为 70,768 字节，其中通知数为 0。模型 HTTP 请求数为 0，Codex 正常退出，私有 sshd、tmux 及测试临时目录经核验清理。产品基线为 `bd4612887`，执行时工作树含未提交的验收脚本与文档；验收脚本随后以 `12adb9f2e` 提交，不能记作该提交的完整产品 GUI 在线链。仓外 `v03-codex-tmux-r-5468824c` 最终索引 SHA-256 `8396e3cda943c939cba7a421e7e11cd3e6405befe2363cf41adcd45047f8aa2b`，双端收据 `9e6c9bdbcde2bdbc55024d02800b1c0d58e14b7d7b88fe95b2472c84000252b6`，插件／PTY 明细 `c08bb592c6d2bbf5db3bf3ced0c45fb80a6ff8e00ccf0e650574f60420b6ec30`；三组 Python 回归 3、7、11 项通过。旧负例缺少内层发送证据的问题在此限定场景补齐，远端 Linux／Windows／WSL、双向交互、断连与取消仍未验，V03 不关闭。

G08 另用同一 `2383428da` 签名产品二进制经 macOS 私有回环 sshd 真实暂存一张 70 字节 PNG：服务端 Verify 和独立 SSH 远端读取 SHA-256 均与源图 `4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5` 一致。并发连接读取未发布传输被拒；未发布半图在断连后精确清理；同 host 重连可恢复已发布引用，旧 epoch 和错误 key 拒绝，显式 release 删除原图。三条 SSH 代理、私有 sshd 与 daemon 均退出；五轮实测加两轮离线测试的临时目录在进程及打开文件核验后清理。最终仓外 `resume-20260929/g08-loopback-fixed/receipt.json` SHA-256 `8d34f00bfdefbdb357b8a25f5fc4bbe441a8ba3930a8d82a779d8450ffbdae19`，索引 SHA-256 `ef31df669e74e99598c0402ec84ca283e44f21a36ae11d54b2145f6fce6ba408`，原始 SSH 帧、源图和远端读取图均归档。验收工具以 `eaee5261e` 提交，离线测试 4/4；执行时源码树另含未提交产品增量，收据只绑定上述签名二进制。此链无 GUI、PTY 或三款 CLI 的原生图片消费／模型回合，G08 保持开放。

G08 后续只读核查选择固定 Claude `2.1.280` 作为远端原生消费候选，其确认条件为交互式同 TTY／内核 peer／会话登记、同请求后代 `Read` 工具事件及历史 typed image 原字节；单独上传或 `--print` 不满足。全新私有 `HOME`／`CLAUDE_CONFIG_DIR`、不继承 API 环境的 `auth status --json` 退出 1，脱敏结果为未登录，因而没有发送模型输入或声称消费通过。仓外安全收据 `resume-20260929/g08-private-auth-4d7a632c.receipt.safe.json` SHA-256 `95caa3053bbef37785a2bebe466cc23701912d1b728b2364eb9ab42f765062db`；原始认证输出未归档，测试记录确认进程组和打开文件清空、短目录删除。后续须在专用私有配置中完成原生登录再测，G08 继续开放。

固定 Grok `1.0.41` 的独立私有 `HOME`／`GROK_HOME` 和 leader socket 也未建立认证：单轮模型标记探针退出 1，stdout 没有标记，私有根未生成 `auth.json`。未发送图片、未取得原生消费或模型回答；脱敏收据 `resume-20260929/g08-isolated-grok-r-6574681e/receipt.safe.json` SHA-256 `f640ca4ad9bded5aad4ca880cb3959869f3e8e1ad847950c50f41bfdb6ba5e76`。进程退出、短 TMPDIR 记录经核验清理。对远端暂存、会话、断连、重复和旧回调路径的离线审查未找到可无认证确定复现的缺陷，不将审查或暂存正例计为三款 CLI 的原生读图通过；G08 仍需专用登录及交互式 SSH／tmux 环境。

**2026-09-28 Homebrew 回滚预检增量 `91fa953f882d54952d81067903e769d42821927e` 与整合提交 `7dd68cc9beba0ee6d0858d1f861ffdc0bf6a7ff8`**：在首次回滚写入前一并检查主树、公共入口、补全与别名的当前/暂存状态，拒绝预先存在的外部修改；已恢复、已领取或合法清理阶段仍可恢复并重复调用。新增六项真实文件系统回归在旧实现上1通过/5失败，修复后6通过；本机更新模块213项、i18n11项、`cargo check --locked -p warp`通过，nextest零重试。此修复只保证在回滚开始前拒绝已存在的不一致，不保证并发写入下跨文件原子性；真实消费者Homebrew来源发现、brew升级、GUI及原生官方映像事务未重验。无需本地化变更，未新增双语布局结论。

专项 [run36417868874](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36417868874) 因后续整合矩阵覆盖其范围而主动取消：Linux六项新回归逐名通过，主套件1395项、IPC2项、Node只读绑定1项、command1项通过，无重试；取消时正在构建监督worker。Windows未执行nextest，不能计作双平台通过。原日志与两份官方工件保留于 `resume-20260928/brew-rollback-preflight/remote-01`，summary SHA `e0645c8a4b2d628f57bdfe9b0c41a50ed5a21ba64248482e7263932f84ed854a`，104文件索引SHA `2e8df160691b66f2d96c43928f963c76f378b948c819a852071b110de1c3a204`。

`7dd68cc9b` 另将不需重绑定的OAuth回调用例移到系统分配端口，避免与取消测试共用低位端口池；取消测试保持100轮立即同址单次重绑定，并增加轮次/地址诊断，生产OAuth逻辑未改。本机OAuth6项及cargo check通过。原Windows10048现场没有端口持有者证据，因此只确认夹具竞争窗口被消除，不认定原失败唯一根因。普通双平台整合验证 [run36419778583](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36419778583) 首轮完成，Linux/Windows官方均成功，两平台cargo check通过；四份Homebrew源码与91fa逐字节一致。Linux六项新brew回归及两平台各六项OAuth均逐名普通PASS，Windows原595组现595通过/7跳过，两平台日志无TRY/RETRY/FLAKY。Windows后续desktop/TUI组660通过中另有1条LEAK，详见下段，不能概括为全体普通通过。本轮临时目录在证据归档、退出及身份核验后全部删除，固定根保留说明与逐轮记录。G09、14项必要缺口及原Windows Node/npm失败均保持开放。

整合批次按套件分别计数：Linux主回归3245、IPC2、Node glibc只读绑定1、图片5、shared76、harness129、双语TUI9、command1、宿主崩溃5、MCP/AI/Vim/补全602、补全v2 128、desktop/TUI660；Windows依次为3063、2、无Node glibc步骤、5、76、129、9、22、5、595、116、660（含1 leaky）。rust-genai两平台各81通过。各筛选可能重叠，不相加为唯一总数，跳过不计通过。Windows异常用例为 `warp terminal::input::tests::test_ai_context_menu_reuses_reference_after_deleted_text`，原始日志明确记录LEAK；[nextest对LEAK的定义](https://nexte.st/docs/features/leaky-tests/)是测试退出后输出句柄仍未关闭，现有配置将其计为成功，不代表内存泄漏。具体句柄/子进程来源待查；runner收尾的vctip22748、sccache9648、conhost22140不能据此归属该用例，也不声明整机零残留。保留此验证警告，后续检查测试上下文及子进程的关闭/等待；没有通过重试或放宽配置隐藏异常。

原件位于 `resume-20260928/oauth-loopback-isolation/remote-01`：summary SHA `e4da9d6bc699945e0c0bbc0117f59c25c42f3beb5f843a8440444df1b823348b`，259文件索引SHA `e07c116817bb360c0dd4633863f1f13658f2937844e364118fcdbaf8353ac265`。12个官方ZIP、47个成员的官方size/digest、CRC、SHA及源码/run绑定已核；独立job全文与run ZIP逐字节一致。仅普通双平台矩阵，未选择Mac Intel、全workspace、真实npm/Homebrew消费者更新、Windows NUL/Node失败对照、真实OAuth登录、GUI或已认证模型。

**main已更新**：远端及本地main从 `693172a260547e216164cd545430608dd2a19fd9` 快进到 `70f65a996e13b405140544f495fc904875259e42`，保留两分支历史。真正同提交测试的源码为 `7dd68cc9beba0ee6d0858d1f861ffdc0bf6a7ff8`；其后的70f65仅改三份验收文档，本段后续记录也只改文档，不宣称最终文档SHA重跑。推广依据为现有编译/定向门禁通过，同时保留上述LEAK待查及14项产品验收缺口，不视为完整Goal完成。


**2026-09-28 main 基线整合 `e5f50d9fcfcc03344b5bfe02360bdf91e11cfd96`**：保留父提交 `7ec006739` 与 `693172a26` 的历史，整合主线8项提交及CLI Agent分支。MCP 2.2迁移、Vim行对象、POSIX `--`补全、Bash取消和GUI焦点修复保留；合并调整仅为7个新增CI步骤继承普通/专项门禁、Bash模板局部SC2157说明和本地化计数。该提交的远验出现Windows失败，当时未更新main；后续修正与推广见上文；不把阶段合并解释为完整Goal验收。

本地 `cargo check --locked`、`cargo check --locked -p warp`、ShellCheck和两种Bash取消回归通过；MCP/AI/Vim/补全602项、补全v2 128项、i18n11项，以及短内盘临时目录的GUI/TUI/CLI联合回归2484项通过。另3项真实多进程用例使用已有 `rust-embed/debug-embed` 嵌入资源后串行通过，分别55.582/54.575/55.244秒；测试运行器总预算180秒包含825MB副本复制与摘要，产品30秒握手不变。不同套件与feature组合不相加为唯一总数。首轮24失败/3超时、第二轮3超时及后续诊断失败与中断均保留：短内盘路径修正0775祖先拒绝、SUN_LEN和EXDEV；launchd helper曾在外置源码本地化文件open阻塞数分钟后自行解除，底层原因未确定，不声明TCC已确诊。旧6个专有服务已精确撤销。原件见 `resume-20260928/main-baseline-merge`，本地摘要SHA `e96fdc2e2174acfd17b352745fc48037a0f8d448692914ad89911dc2dbc50d83`。

远端普通Linux/Windows验证 [run36411332886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36411332886) 最终 **failure**：Linux成功，Windows失败，两平台 `cargo check` 均通过。新增7个上游步骤均实际执行；Windows AI套件失败后，v2与desktop/TUI仍继续执行并通过。实际套件分别计数如下，不相加为唯一测试总数，跳过不计通过。

| 套件 | Linux x64 | Windows x64 |
|---|---|---|
| 主agent/CLI回归 | 3239通过 | 3063通过 |
| IPC帧限制 | 2通过 | 2通过 |
| Node glibc只读绑定（不执行Node） | 1通过 | 未设此步骤 |
| remote image合同 | 5通过 | 5通过 |
| shared CLI合同 | 76通过 | 76通过 |
| CLI harness | 129通过 | 129通过 |
| 双语TUI消息 | 9通过 | 9通过 |
| command进程归属 | 1通过 | 22通过 |
| 宿主崩溃清理 | 5通过 | 5通过 |
| MCP/AI/Vim/补全 | 602通过、4跳过 | 595选中：594通过、1失败；另7跳过 |
| completion v2 | 128通过、4跳过 | 116通过、7跳过 |
| desktop/TUI输入 | 660通过 | 660通过 |
| rust-genai | 81通过 | 81通过 |

Windows唯一失败为 `ai grok_subscription::oauth::tests::cancelling_loopback_wait_releases_listener`，在 `crates/ai/src/grok_subscription/oauth_tests.rs:126:36` 取消回调监听后重绑端口返回 `10048 / AddrInUse`；不是MCP迁移断言失败，底层原因待查，不能仅凭此认定TIME_WAIT。两平台完整nextest日志未见TRY/RETRY/FLAKY，未重试失败；Windows主回归运行680.774秒、desktop/TUI运行159.744秒，其余步骤时间包含编译。Linux Bash取消、两平台通知/安装器与原生恢复清理边界通过，详细脚本计数和范围见摘要。Grok监督四场景均确认清理；ACP私有leader未随stdio EOF自然退出，Linux清理码-9、Windows1，不称全部自然退出。Windows收尾另清理vctip2776、sccache26488、conhost31388，无法归属case，不声明整机零残留。Windows Codex生产安装器使用0.147.0夹具，私有根已删除，其收据不宣称原生hook、GUI或监督进程树清理；固定0.156.1 hook/ConPTY另有独立步骤。

原件 `resume-20260928/main-baseline-merge/remote-01`：summary SHA `89568e89ab470e021ccc876d995bd6cdd2ea2cc357e9160f2c9ec01947532b79`，253文件索引（不含索引自身）SHA `24494b6d1a9a9254236807324f42437a31e0beb7ec9c33544bbd8d518faf6bee`。完整run/jobs日志、12个官方artifact ZIP/47成员已归档，官方digest、ZIP CRC和成员SHA逐项核验；独立job日志与run ZIP全量成员一致。该结果只绑定e5提交，不含后续Homebrew修复；当时阻止main推广，后续7dd提交结果另列，不回填本轮失败。Mac Intel、全workspace、真实npm/Homebrew更新、Windows NUL/Node已知失败对照及GUI/模型未选。无需新增本地化文案，两语各6080键及变量一致；不扩大历史双语布局通过范围。按用户指示采用[固定内盘临时目录与即时/每日清理约定](../../docs/local-test-storage.zh-CN.md)。G09和14项必要缺口继续开放，Windows原Node/npm失败不因基线整合关闭。

**2026-09-28 Mac Claude Homebrew恢复增量 `ae37daf19d86005883b8f64b3bb71f57b336843c`**：修复非Committed事务恢复先回退公共入口、再发现新树或旧backup被外改而留下断链的顺序缺陷。生产路径在任何回滚写入前预检两树及公共链接/link-stage，保留交换前后复核与既有来源/prefix/锁门禁。新增新树外改拒绝、旧backup外改拒绝和正常恢复三项回归；不将其外推为completion/alias等任意跨artifact改动的原子保证。本地Python15项、i18n11项、更新模块207项、cargo check/build、定向格式和diffcheck通过；7923项未选测试不计通过。无需本地化变更，既有状态语义未改，本轮没有新双语布局结论。

本轮新编译worker `0034a2b9d1ba0b600e43f191735c58e4ccf404f9bb3408dfa644008c27d855c9`、新签名supervisor `cbde99a96246d6c7ed5f10ed7e28700f7a41b8c259811c6b125c26609df37efb`，16份绑定文件与该提交Git blob一致；130项资源沿用已核模板，ARM64/ad-hoc严格签名通过。在新私有APFS卷中，固定Claude `2.1.278→2.1.280`、`claude-code@latest`四场景983.931秒退出0：正常更新实际入口2.1.280；交换后缺少完成记录由不同PID冷恢复完整旧树/link身份，实际2.1.278；外部改动恢复拒绝且保持新树/link、marker、旧backup与原journal，实际2.1.280；Prepared候选改动在启动前拒绝，完整旧树及2.1.278入口保持，probe=null且零generation。三条候选原生exit0、退出绑定及job/coalition清理链独立通过；最后一项仅未启动、无候选需清理，不算第四次清理实测。执行开始/结束工作区干净，源码、输入及产物摘要保持，零模型输入。

范围为**固定官方映像与人工Homebrew登记的生产后端事务**。实时API未提供目标历史JSON；使用官方Homebrew固定提交 `ef1bb0b080bda4075f4a8992bb6b10a0f0bc5bf7` 的Ruby字段（SHA `c4173c146d619ef3902640d853af310b4ea8748add84e8ec518de61eb5781555`）构造明确标记`derived_fixture`的604字节JSON（SHA `95f16664adcbd9d74544db71e9d953f3ffd1dc1f39db53a93c41ee1b9cda8fc3`），配合两版官方Mach-O和manifest原件。不执行brew/Ruby，不把派生夹具当历史API原件；消费者真实来源发现、实时渠道、brew安装升级、GUI、忙碌/插件与其他平台Homebrew真实事务仍待验。G09及14项必要缺口保持开放，Windows既有Node/npm失败未修复。

原件根为`resume-20260928/macos-claude-brew-recovery`。四项summary SHA `c682230a37598543ff7ad191394def31088bb3a1a6e88619edae21f412b56164`，独立审查 `a735d548a6e43775c70d68f80502dc185c6eb73a92cc1c5476783b8021d7fb5a`；205份小原件索引 `c6c51151afa8c38c5f8ba01885346a6ca33932fefd628a2011c1e8ae35ae5055`。首次APFS创建/挂载成功但v1误把创建前image UUID当跨首次attach稳定值，验证停止，原件保留；v2绑定首次attach后UUID、backing inode/device/owner、卷/容器/分区UUID及完整设备链后恢复，只初始化本次新卷root为0700，noowners和原卷属性保持，不宣称强多用户隔离。全部读取完成、当前账户可观察打开项为空后正常eject成功，挂载点消失，原路径所核身份/权限属性保持；清理收据 `fefb0799a24e60adfba1c6dd63777b18ea43b97900a8d7af08be15f87d3bf7b6`。完整映像保留，卸载后124普通文件/60个band/3985339510字节索引 `673bf4c667c5980b2b4d158a4f8c31c0a77e8553abfb23b638721f07fcd69299`，16GiB仅虚拟容量。构建源码绑定 `1f9aa30d2e27a70ae2f4f8089682dc08842812a8d83fba0d99f6c570c4030856`，生产修复及live harness另有独立静态审查。

本增量同提交 [CI36403642011](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36403642011) 整体成功：Linux编译、主回归1389项（含上述三项新brew恢复回归）、IPC2项、glibc绑定1项（不执行Node）、command归属1项、宿主崩溃清理5项、rust-genai81项及监督程序构建通过；Windows编译、504项定向回归和Grok生产安装器通过。各套件分别计数，不累加为去重总数；Windows不编译或运行Unix brew三项。68成员索引SHA `0af2613da00ebc3d1718f837d248d80a610abb5a0b45db73b2afbf5e9c4ba6e3`、summary SHA `bb3966ad492fb36119664adef30c105324a6c8a02623615c1a75ae7f6aa3b71e`，完整日志和5个官方ZIP/18成员已归档并核摘要。初版派生汇总漏列两条单数test摘要已修正并保留初版，原日志不变。Grok监督四场景cleanup确认、两平台installer私有根已删；Linux Grok ACP私有leader通过SIGKILL(-9)结束且owned_process_exited=true，不宣称全进程自然退出。仅既有工作流，未选择Mac Intel、全workspace、真实npm/Homebrew更新、已知Windows NUL/Node失败对照、GUI或模型；本次Windows绿灯不覆盖历史失败。最终本地cargo check再次通过，三份验收文档之外的冻结实现源码保持。

**2026-09-27 Windows 取消时序诊断 `b3908488c`**：清理阶段只通过 CREATE 原始文件／进程句柄观察已持有租约的镜像角色及首次终止请求前的创建年龄，不重开 PID、不改变授权或隔离。Mac 本地 check、i18n 11项、定向格式及差异检查通过，无需本地化变更。同提交 [CI 36322313315](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36322313315) 的 Windows 编译、995项普通回归、command7项通过；显式原生12项中11通过、原NUL输出对照1失败，未重试。新增5项诊断测试及加强后的2项取消测试通过，TITLE与GOTO/NUL/TITLE绑定CMD子映像对照仍通过。真实Codex npm首个`updated`返回`ProbeFailed`：约241秒后取消，正常阶段只有root／console CREATE，清理阶段只有两者EXIT1，取得`cleanup_confirmed=true`和Job／ACL／profile清理标记。本轮没有再次出现旧unknown CREATE，不能据此给旧事件补造镜像身份或断言Node从未创建。PowerShell、发布与独立恢复未到达；下一步比较完整shim选路及stdin关闭／保持打开的原生对照。G09和完整验收均未关闭。

本轮原件位于仓外 `resume-20260927/windows-create-timing`；`ci-36322313315/summary.safe.json` SHA-256 `a32b7fa8584370a68d6ce7ce22799e8bce7363bea5e6fdaf552a8dbbbc8d2183`，原job日志摘要 `68576a58b78ebd4626071d8587c5160410abdbf6d02adae5ced65978608568ea`。462436948字节npm ZIP只提取38个小成员并逐成员核CRC，未核整ZIP摘要。未选择其他平台或重建Mac联测包，当前可用包仍见下文。

**完整 shim 选路与 stdin 对照 `a0e6662b2`**：本地check和i18n11项通过，同提交 [CI 36324306564](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36324306564) 的Windows编译、504项普通回归通过；原生16项中15通过，仅原NUL输出对照失败。新增四组显式runtime路径／PATH裸名称×stdin EOF／保持打开均通过，入口、分支、子进程标记准确，原生退出0且清理确认。控制脚本保留CALL/find_dp0/IF/PATHEXT/GOTO/NUL/TITLE顺序，但子映像为绑定CMD副本、参数为写标记，**不是真实Node或npm验收**；这两个变量单独不足以复现挂起，不能据此修改生产环境。没有重跑真实npm事务，也没有关闭G09。原件 `resume-20260927/windows-shim-matrix`，摘要 `a34518b2a16ce6f34aff2ac74bf41c750bb03e6a867e6d3068287eee72bcae8b`，原job日志 `5736b293e94061f2cc10e639cb4eb1a200d105140c3e1a2c1c788bfac295525d`；无需本地化变更。

**固定 Node 与验收标准流对照 `4c4b73c1d`**：本地check、i18n11项、定向格式及actionlint（仅排除既有自定义runner标签提示）通过。同提交 [CI 36325630710](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36325630710) Windows编译和504项普通回归通过；原生18项中16通过、原NUL及新增固定Node对照2项失败，不重试。管道stdin/stdout、磁盘stderr及开放stdin已实际核对；绑定CMD基线输出1次标记、退出0并清理确认。固定Node20.9.0在1856ms记录正常CREATE、1876ms自然EXIT `3221225794/0xc0000142`，root随后同码退出；没有观察到取消，没有版本输出，before／branch标记正确且AppContainer清理收据成立。父层driver101是断言失败，严格Job清理确认；不得当作Node原生码。Windows SDK将该状态定义为 `STATUS_DLL_INIT_FAILED`，尚不知具体初始化对象。未执行codex.js或重跑真实npm，不能等同之前241秒挂起的根因。下一步补有界loader事件观察及同一Node根／子进程对照；G09保持开放，无需本地化变更。原件 `resume-20260927/windows-node-stdio`，摘要 `e17c30272bebe420442e56565974fcb9e56a960269e0479c2a5eb0fe8487abd1`，job日志 `6dc2a90907b20f6721a28f7fae2429ad79bd5b7da9353717f7b3723572fc1d06`；6个artifact成员逐项核CRC／SHA，未核整ZIP摘要。

**固定Node根／子及loader对照 `70c172560`**：本地check和i18n11项通过；同提交 [CI 36327294348](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36327294348) Windows编译、504项普通回归及新增3项loader边界测试通过，原生19项为16通过／3失败（原NUL、Node子、Node根），不重试。两种固定Node拓扑均到达初始断点后自然退出`0xc0000142`，分别观察28／29条DLL事件，未输出版本，也未取消；因此shim不是复现本错误的必要条件，具体DLL与旧真实npm超时根因仍未知。根对照未执行shim且标记确实不存在；CMD基线退出0，三组Job／AppContainer清理均确认、日志未截断、loader摘要无丢弃。conhost的首机会异常与重复GUI DLL加载也出现在通过的CMD基线，不能单凭这些判为致命根因。下一步仅经显式测试入口对照隐藏独立控制台，正式npm路径保持原状；不关闭G09，无需本地化变更。原件 `resume-20260927/windows-node-loader`，摘要 `6b6600c023890cfa8028806a47b511284daf66b2a58642ed1cfb297d4c24917c`；9个成员逐项CRC／SHA，未核整ZIP摘要。

**隐藏独立控制台对照 `e3f44c29c`**：本地check、i18n11项与command Windows目标编译通过；同提交 [CI 36328813817](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36328813817) Windows编译、普通504项、command11项及loader3项通过，原生22项为16通过／6失败，不重试。新增隐藏模式的CMD基线、Node子进程launcher、Node根进程三组均自然退出`0xc0000142`：root只观察到ntdll／kernel32／KernelBase三条DLL事件，尚未到初始断点；两个CMD launcher没有进入shim或观察到Node CREATE。因此该模式不能接入正式npm路径。原NoWindow的CMD基线仍退出0并输出一次标记，固定Node根／子仍在初始断点后失败，不能因退出码相同混淆两种失败阶段。六组均确认AppContainer与父层严格Job清理，每组见到一次已验证conhost正常退出，无取消、超时、输出截断或loader摘要丢弃；安全检查未放宽。runner收尾另清理一个conhost（PID2004），现有日志不能归属至具体对照，不能据六组收据宣称机器级无残留。没有重跑真实npm、执行codex.js或重建Mac包；具体DLL、runner窗口站／桌面访问条件及原npm超时根因仍未知，下一步先核对实际环境边界。无需本地化变更，G09及14项必要缺项保持开放。原件 `resume-20260927/windows-hidden-console`，摘要 `f08710378d0538270be663e560ba1ee70176bdaeb2f0b3481e49be4cf23360b1`，job日志 `bef7c3917c3dd2a26c225cd268b7859a278015de0db4b2204f17e7959108ba30`；18个成员逐项CRC／SHA，未核整ZIP摘要。

**2026-09-28 Windows 实际环境观察 `5ea2614e0`**：本地check、i18n11项、Windows SDK目标测试编译及actionlint通过；同提交 [CI 36334101650](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36334101650) Windows编译、普通504项、command11项、loader3项和新增环境4项通过，原生22项仍为16通过／6失败，不重试。六组对照共14次已绑定CREATE观察完全一致：runner在Session0、高完整性、已提升、非AppContainer，窗口站分类为other且不可见、桌面名分类为default；候选同在Session0、低完整性、未提升、AppContainer。双方Win32k系统调用禁用策略均为0，严格句柄策略runner为0／候选为3；成功CMD基线同样具有该值，不能据此认作Node失败原因。目标线程桌面查询均没有有效结果，返回的hresult为0且无OS错误码，**不能解释为已证实访问拒绝**。这些观察不建立目标窗口站、桌面对象相等或实际访问权；本轮未执行DACL模拟。

NoWindow的CMD基线仍成功，固定Node根／子在初始断点后`0xc0000142`；隐藏三组root仍在首断点前失败，与上轮相同但两种阶段不可混同。六组AppContainer及父层Job清理确认，无取消、超时、输出截断或loader摘要丢弃；本轮runner收尾仅记录vctip PID7876，未匹配14次观察PID，未记录orphan conhost。该事实不回填上一轮conhost归属，也不是整机无残留证明。原件位于仓外 `resume-20260928/windows-environment`；18成员／17923字节完整ZIP已核官方SHA-256 `81cf27e7a13d51d6b82d4ef4228ae65e1fed2eb901ba72ac5b3358f615ef037e`及成员CRC／SHA。摘要 `81897a3638abf2f083a7722d555027a1570351d5d99501ca53f1636433805f77`，job日志 `27f3dde2a6e480569234a51e1f8429e6cb7a37378c22bd98f009938aea3e7110`。下一步仅用原CREATE句柄的实际token对runner窗口站／桌面描述符做有界权限模拟，继续区分API失败、拒绝与允许；不改全局ACL、隔离或官方shim。真实npm未重跑，Mac包未重建，无需本地化变更，G09及14项必要缺项保持开放。

**2026-09-28 Windows DACL对照 `f7e0801f1`**：同提交 [CI 36335721288](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36335721288) Windows编译、普通504项、command11项及诊断10项通过；原生22项为16通过／6失败，不重试。本地check及i18n11项通过。六组14条已绑定进程观察中，实际token对runner当前窗口站固定0x23、桌面固定0x83及各自MAXIMUM_ALLOWED的56次AccessCheck均成功返回拒绝、granted_access=0；成功CMD基线亦相同。该结果不等于候选实际目标对象已确认，MIC未评价；目标线程桌面14次query_failed且hresult=0，不能写成访问拒绝。

NoWindow CMD基线通过；固定Node子／根到首断点后分别观察28／29条DLL事件，再自然退出0xc0000142。隐藏三组root仅3条DLL、尚无首断点便退出同码，两个CMD launcher未进入shim或创建Node。旧NUL marker失败保持。六组AppContainer及父层Job清理均确认，各有一次已绑定conhost正常EXIT，无取消、超时、输出截断或loader丢弃；五组driver退出101与外层完成收据不匹配来自成功断言失败，内部退场收据匹配。runner末尾另清理conhost PID31528，日志中无可关联身份，不能归属case或宣称机器级无残留。

原件位于仓外 `resume-20260928/windows-desktop-access`：19151字节完整ZIP、18成员，官方SHA-256 `f8b1000693fa150300b110ffd682e6302b852eee6b69b9551ee37810a895fe6e`及成员CRC／SHA已核；摘要 `ca1de6a2de63d004c0e6028c8405901abd15a35f2f13c3afee158df21048fa97`，job日志 `6163600269ef53492b0bb2c9dcf6648a24814eb3dade589db686d0be5f84989e`。真实npm及codex.js未运行，生产默认和隔离未改；私有对象对照尚未实现或执行。无需本地化变更，G09及14项必要缺项保持开放。

**2026-09-28 Mac ARM Codex npm 恢复补验**

Mac ARM Codex npm 在 SanDisk 显式执行卷策略下推进：e3a2e5654106e5eae9d6ffff99c07e80ca84d475 的运行器 Python 34 项通过，run-01 candidate_changed_preserved 在派生前拒绝候选篡改并保留旧公开 0.155.1。同批 swap_receipt_missing 在交换断点前失败，launchd 明确报告外置 plist bad ownership/permissions。054ed457e818495ba0c946c8823a9099ad170fc8 将 bootstrap 配置移到既有私有控制目录并保留外置诊断副本；cargo check、Mac 19 项、i18n 11 项和构建通过，但 run-02 仍因外置 stdout EPERM / EX_CONFIG(78) 失败，不能计为恢复或清理通过。证据见 run-01-review.safe.json、launchd-fix-02/gates-progress.safe.json 与独立 run-02-review.safe.json。

bb78f3ac5636d9630db0d704d9ea04268594fae5 修复 debug 临时 stdio 与安全归档后，stdio-fix-02 的 cargo check、Mac 24 项、i18n 11 项、监督程序构建和签名校验通过；129 项资源保持一致，worker/supervisor 分别绑定 19/16 份源码。run-03 两项均 accepted，runner exit 0、705.315 秒：swap_receipt_missing 通过实际 product inspect 确认 Npm 0.155.1、完整旧树恢复且 journal 清除；external_change_preserved 返回 RecoveryRequired，保留外部 marker、与 before 相等的旧备份及 journal。两项公开入口链接身份不变；各自交换前候选 Node→bin/codex.js --version 的原生 exit 0、cleanup true 收据在冷恢复时重验。external 结果中的 0.156.1 为分支固定赋值，不代表恢复后执行被外部修改的公开入口；cold-exit 也是原候选收据的留存。证据见 stdio-fix-02/artifact-ready.safe.json、run-03/source.safe.json、run-03/summary.safe.json 及 run-03-index.safe.json（59 份安全收据/捕获日志，不包含全部二进制）；run-03-swap-review.safe.json 与 run-03-external-review.safe.json 两项独立复核均通过。

历史正常 updated 成功仍属于 2b59c8c62c7c46bc30211a9872f966c52317202e，候选改写拒绝属于 e3a2e5654，本轮 bb78f3ac5 仅两项恢复补验，不能合称同 SHA 四场景通过。新签名包只用于 headless 监督测试，不替换 e2eba GUI 联测包记录。模型输入为 0，未覆盖 GUI、消费者渠道发现或 busy/plugin 重检，未修改用户安装或系统安全配置；G09=false、all_required_acceptance_satisfied=false，14 项必要缺项保持不变。无需本地化变更。上述证据相对根目录为 /Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20260928/macos-codex-npm-recovery。

**Mac 修复后正常与拒绝路径补验 run-04**：复用bb78f3ac5冻结worker与签名监督程序，执行时为干净14fb66d99检出；19份相关源码在两提交和磁盘逐字节相同，但未以14fb重新构建产物。runner退出0、526.171秒，updated实际公开入口返回0.156.1，现场63节点/47文件与prepared/after逐项身份和摘要相符，候选原生exit0与coalition清理证明链匹配；candidate_changed_preserved实际公开入口保持0.155.1，完整旧树before/after原字节相同，未产生候选probe或托管generation，RecoveryRequired保留journal与被改候选现场。两项官方完整包SRI及成员重核通过，独立审查 `run-04-review.safe.json` 摘要 `5a638607ad57175e6a9296a51175d9644e8a76113345f1927b68a3fb1ce49d03`。35份安全收据/捕获日志索引 `run-04-index.safe.json` 摘要 `3b8117a4f5ce093c0d120a02dd5ff84efd2e821acca35e4db869bf1a32bb2301`；原件同macos-codex-npm-recovery根目录。run-03与run-04合计覆盖同一bb78冻结产物的四类场景，仍不能写作整提交同SHA全验收或G09关闭；无模型/GUI/用户安装变更，无需本地化变更。

**2026-09-28 Windows NUL实际令牌DACL增量 `11f27809154fbd685ca860345e946b69613c1610`**：[CI36389274428](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36389274428)／job108821293774编译及504/22/14项通过，原生仍1通过/2失败。7条唯一CREATE观察（CMD3、NUL2、Node2）用实际bound及driver令牌的SecurityIdentification副本，对同一持有NUL对象SD作READ/WRITE/MAX三项AccessCheck：bound共21拒绝、driver共21允许，该模拟无查询失败；未安装模拟token或改ACL。成功CMD也被拒绝，且其中node角色是绑定CMD副本；不能由此认定Node/npm唯一根因，MIC及CMD实际CreateFile参数/错误码仍未观测。NUL的redirected.txt仍为空，Node没有版本行，记录29条root DLL（上一批为28）、37条console DLL，已到初始断点、dropped0，原生0xc0000142；内外清理确认，原失败断言保留。

[微软MXC固定源码文档](https://github.com/microsoft/mxc/blob/311df7b385aeea7f55991a70d324c094f493ca47/docs/host-prep.md)说明NUL默认权限的AppContainer兼容限制，[libuv固定测试源码](https://github.com/libuv/libuv/blob/2cadaa40167050baf7c6905ac897e6fb57afb2c6/test/appcontainer.c)用设备句柄修改测试SID的NUL ACE；二者只提供机制线索，其管理员全局ACL准备/设备修改均未执行，不算当前产品修复。07:10:13Z只读环境收据除时间外与上一轮相同，原服务/注册/防火墙/RDP/NUL SD保持，RDP仍禁用、该时点TCP/UDP3389无监听，不外推全系统或永久不变。runner仍Session0/high/TokenIsElevated=1，普通交互会话待用户；控制台登录即可，NULL+CWF_CREATE_ONLY私有站仍只是可能碰撞服务站的方案。首次本地check误用默认debug缓存后主动SIGINT(-2)的日志保留，02沿用缓存通过1.253秒，rustfmt/diffcheck通过。

原件位于`resume-20260928/windows-interactive-context/service-nul-dacl`；摘要SHA `e211e982bf3ff33e6ae58329394351c26ad0565054ee011fa87d67b1f0eb1480`，NUL差异评估 `a6ba04cef404459a40c17dc2059a16779677ec133cd72cf55fed055808c6b116`，6343字节／6成员ZIP `848a2d12abff0d24a95a984ad96a7bb57c58130d225268dd75d3ff788a8e70a1`，NUL原件另在完整作业日志。33文件索引 `7cb57be7e6bead3647de9aae040ba995616983ccdaee56bdceffb7aa177b6279`，独立审查 `ad5d1b28d65811c597eb74477c773669d9a6ed1ccaf664364e7a3f8fc2ce61cf` 另列；最终本地cargo check再次通过。该诊断轮Mac Claude cask当时尚未执行，后续四场景结果见本报告ae37daf19增量。历史原件、零capability/严格Job/映像绑定保持，未重跑真实npm、GUI、模型或其他平台，G09和14项必要缺口开放，无需本地化变更。

**2026-09-28 Windows 服务上下文标准流复验 `730b63b6a3d4d4e6c565709755dd599498af1055`**：修正NUL夹具的stdout保留名冲突，改为`tmp/redirected.txt`，保留`2>NUL`、errorlevel及完整内容/清理断言；workflow只新增stdio三项精确选择，默认all和零重试不变。[CI36384245535](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36384245535)／job108806177281已结束failure：Windows cargo check、普通504项、command22项、诊断14项及Grok生产安装器通过；三项原生仅CMD通过，NUL和固定Node根失败。CMD真实输出一次routing-child并原生退出0；NUL普通文件仍为空，plain/after匹配，errorlevel和CMD退出码均0，失败落在内容断言，内外清理确认；乱码stderr没有可确认的Win32数值错误码。Node没有v20.9.0行，记录28条DLL并已到初始断点，随后原生退出0xc0000142，driver101仅代表断言失败；内外清理确认、无超时。NUL已消除夹具混淆但功能仍失败，不能认定其为Node或npm唯一根因。

CMD/Node的5条实际环境记录显示runner为Session0/high/`TokenIsElevated=1`、非AppContainer且窗口站不可见；bound进程为Session0/low/未提升AppContainer。该字段取自TokenElevation，不是TokenElevationType；不能称未提升普通交互验收，也不能据此单独推断管理员成员。NUL未启用这些trace，不外推其逐项字段；20/20项runner对象DACL模拟拒绝也包含成功CMD，未评价MIC。作业后只读核得原服务Running/Auto、listener账号infinishell-build、Session0、.runner摘要保持；管理员读取NUL设备SD不等于实际AppContainer访问验证。RDP仍禁用、TermService Stopped/Manual，回环选择API虽返回0但前/中/后读回均All，未建立回环监听或交互会话。06:46:41Z的作业后查询使用ErrorAction Stop且脚本成功退出，确认该时点TCP/UDP3389无监听，不外推全机或永久无端点；旧回环选择器的空数组仍不足以独立证明枚举成功。普通交互会话仍待用户确认，仓外任务脚本未执行；cloud私有namespace的3秒Xvfb探针仅为工具准备，不能证明RDP登录。runner另清理conhost23980、sccache31808及vctip27044，无法归属case，不声明全机无残留。

原件位于`resume-20260928/windows-interactive-context/service-stdio`。6035字节完整ZIP／6成员SHA `ac3c232a98ebefcd0baa523ff98f3d5609420a67e8806f64ed6cbc8a0b7d290f`，摘要 `419f4c67a3755459a5a0303b657eb013b9f4c7e65517916019a8c8e58d8c650f`，独立审查 `f1ddedf29a0adc8494d63325f7a1e570a75f7465e8c92f2644c0c199efd3a0e0`；NUL原件在完整作业日志，非此ZIP成员。34文件索引 `176ccf39e3a5c2ce6b3d205ba2e20346310077731f9e294f20e7b8a7df8f5dd2`，独立审查另列。历史失败和管理员私有Node成功保持；下一步仅测试的实际令牌NUL DACL观察正在准备，尚无新运行结论。未放宽设备ACL/capability/Job/映像边界，未重跑真实npm、GUI、模型或其他平台；G09和14项必要缺口保持开放，无需本地化变更。

**2026-09-28 Windows 临时管理员私有窗口站复验 `61d0747b4162110babcccd598f262fc22c56136f`**：用户授权经cloud SSH使用既有Administrator凭据。原runner空闲后仅停止精确服务，复用注册22以Administrator、Session0、`run --once --startuptype manual`领取一次[CI36379989244](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36379989244)，服务账号/组/全局ACL未改，密码未输出。相关私有对象实现、测试和工作流与eb2b271e3相同。Windows cargo check、普通504项、command22项、诊断14项及Grok生产安装器通过；原生23项为17通过/6失败，retries=0，整轮仍failure。

实际创建站线程的CheckTokenMembership成功返回member=true，私有窗口站/桌面的保留句柄安全描述符与低完整性标签核验通过；固定Node根进程在NoWindow环境真实输出一次v20.9.0，原生与driver均退出0、收据匹配、内层AppContainer/外层严格Job及自有私有对象句柄关闭均确认。root见32条DLL、首断点、零丢弃，无超时或输出截断。原NUL和五组非私有对象对照仍失败，后五组为0xc0000142，内外清理确认；runner另清理conhost PID13384与sccache PID28760，缺可关联创建身份，不归属case。此结果解除诊断身份阻塞并建立私有对象成功对照，不能认定普通用户、正式npm链或具体DLL根因已解决。

任务后管理员Listener正常退出，原服务自动恢复Running/Auto、账号infinishell-build；原.runner注册文件摘要及五个目录根owner/SDDL保持，原低权限账号cargo可用且两个缓存目录和_work读写/删除临时文件通过，runner在线空闲。核验范围不外推全机无残留或全部缓存成员权限。原件`resume-20260928/windows-admin-identity`，21成员ZIP已核官方SHA `923f79f4c5322120411050e72b778d9acea8fa8ccca10d753b0f08358c489efe`；汇总SHA `9a1227c3e99bd1a79ecb8177e5febcbf863a52fce4ee1c5118106e3e9f2526bd`。独立证据复核通过，收据SHA `5259b08ecca58e77039b9a1154b0116c7d7b1144e58c852cedcf1119930d0019`。本轮无产品代码修改、无需本地化变更；真实npm更新、GUI、模型及其他平台未执行，G09和14项必要缺口保持开放。

**Windows 私有窗口站／桌面对照 `14fb66d99358929520428308b9abf81a0c8f6f1b`**：本地check、Windows command目标编译、i18n11项、格式和actionlint通过，同提交 [CI 36342378730](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36342378730) Windows编译、普通504项、command21项及诊断13项通过；原生23项为16通过/7失败，不重试。测试工具实现了仅创建者与精确AppContainer SID的私有对象DACL、低完整性NW标签读回和严格Job后关闭，但新case在CreateWindowStation(create_station)返回0x80070005，尚未读回对象或启动候选Node；environment_control_established=false，原生退出码、AppContainer清理和私有对象关闭均未知，只有外层driver严格Job收尾确认。driver101为断言失败，不能当作Node退出码。

原NoWindow CMD基线通过，Node子/根分别观察29/28条DLL并在初始断点后退出0xc0000142；隐藏三组仍各3条DLL、首断点前失败，旧NUL断言保留。原六组AppContainer和外层Job清理确认，14次观察的56项runner对象DACL模拟仍拒绝，不计为新私有对象可用或根因定位。runner末尾另清理conhost PID22732，无法归属case；sccache18052、vctip29940也只作收尾日志记录。21成员、21171字节完整ZIP已核官方SHA-256 `4f7976394c7366fbc8029be813b664047222fb3fbf53345702f608c8d37ffa2b`及成员CRC/SHA；原件位于resume-20260928/windows-private-desktop，摘要 `fdb940c3e1d36bebef18b2ff4e38825a4d56c216328d29a92c582e8f7d8304fd`。gh组合日志遗漏第38步骤，改以原run日志ZIP中的完整1_Windows x64.txt核对504项，摘要 `9d4307bb618a66b15f14cfe82a680823837627fcf4b88ea41e1556dc33cd9940`。下一步仅只读核验失败调用点有效管理员成员前提，不凭拒绝码改权限或回退既有站。真实npm未重跑，生产默认与隔离不变，G09/14项缺口开放；无需本地化变更。

**Windows 命名站必要前提 `eb2b271e3999817db036d244ed34f40bfad9883c`**：新增只读调用点成员诊断，保留true/false/query_error三态，原创建调用和权限不变。本地check、Windows command测试目标编译、i18n11及格式/差异检查通过；同提交 [CI36344378073](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36344378073) Windows编译、普通504项、command22项、诊断14项通过，原生23项为16通过/7失败，不重试。CheckTokenMembership(NULL, BuiltinAdministrators SID)在失败调用线程成功返回member=false/hresult=null，独立原事件与安全失败摘要一致；CreateWindowStation仍0x80070005，私有环境/对象读回/Node候选均未开始，内层完整清理及私有对象关闭无收据，仅outer Job清理确认。命名创建所需有效管理员成员前提未满足，不能外推为原Node DLL初始化或真实npm挂起根因。

原NUL及五个Node/隐藏组失败保持；NoWindow实际Node子/根本轮DLL数为28/30，均首断点后0xc0000142，隐藏三组root仍3 DLL且无首断点。六组已启动对照的内外清理确认，runner另清理conhost PID12312但归属未知。原件位于resume-20260928/windows-private-desktop-admin，21231字节完整ZIP/21成员已核官方SHA `4a2af5038df0f1a7469882694d4a53c791e79aeb49bfee8ebabb61fdaeed14b9`及CRC/成员摘要；汇总 `b1f6985bcf4ffe74e5d75983a947e9234dc505960c669d2d5f89768c6eb94c0e`，完整raw-logs/0_Windows x64.txt摘要 `c492b3f814a44ea676779e017d91e8b28414ddffbd6f98b5c25778286aa3205e`。独立证据复核通过，记录摘要 `232e2d5d823326d2b7907de7e18fa310ec3cfb88cbccce3eff896c8b35c35864`。等待可用独立管理员验收runner信息，不自动启用权限、修改全局ACL或换用匿名/既有站；Mac Grok npm的外置卷和恢复准备继续推进。真实Windows npm未重跑，G09及14项必要缺口开放；无需本地化变更。

**Mac Grok 私有APFS四场景后续验收（2026-09-28，run-03）**：用户继续所述专用映像方案后，在SanDisk创建16GiB按需增长APFS映像并挂到独立/Volumes/InfiniShellGrok-20260928，只将新卷根0755初始化为0700；原SanDisk0775保持，未修改现有卷ownership、ACL或TCC，noowners未改。这仅证明当前账户下现有mode/uid门禁和事务行为，不表示原SanDisk布局修复或强多用户隔离。复制132个应用文件与冻结worker，SHA、签名及17/15份源码绑定一致；worker仍为486330824相关源码，监督程序仍是bb78冻结构建，执行检出318c5d93f，不计318c整包重建。

整批1007.746秒退出0，四场景与独立复核均通过：正常更新的实际公开入口1.0.41，新版37节点/29文件完整；交换后缺失完成记录由新进程恢复1.0.40，旧树/镜像/配置/链接身份恢复，日志和stage清除；外部变更场景新树+标记、旧backup及日志原字节保留，RecoveryRequired拒绝自动覆盖，实际版本1.0.41；候选篡改场景旧树与1.0.40入口保持，篡改stage和原日志保留，未创建候选代次。三组实际候选均原生退出0、清理确认；两项冷恢复cold-exit保持原候选收据，不冒称新候选退出。拒绝场景的空cleanup不计为清理路径证明。

原件在resume-20260928/macos-grok-npm-recovery/private-apfs；汇总摘要 `1a81329c0518fcd2bafa0d73a69e5978a3ba58b4eadc336d8f5344d962d3c2a4`、四项最终独立复核汇总 `b98361dcee870bc76685cf7dc18917085eb968d9a67925ae8252847f7348a771`。172份收据/日志副本逐字节封存，索引 `3dcfd077f93125219e09b36058d3ba52f58756a3836fcb8b64dc37f94e7682c4`；全部读取结束后核映像设备/UUID和当前账户可观察句柄，普通eject成功、挂载点消失，原卷属性保持。完整稀疏映像保留，卸载后110个文件共3541318774字节索引 `71fa1bdd74bffaa16d87209fcd240641cadcfc1f9d227f2067aa49e629eabb89`，清理收据 `97ee5a7a0459a828d71cd879621d199f026ab926f6ba2374d50a4c7c84168758`。本轮cargo check通过，共享target锁等待日志保留；Python41/i18n11为同源前轮门禁，不重复计本轮执行。无模型输入、GUI、用户安装更新或npm install/lifecycle，消费者渠道/实际忙碌插件/其他来源平台仍未覆盖；Windows现有runner在线但无新管理员身份信息，未重复已知失败CI。G09及14项必要缺口保持开放，无需本地化变更。

**Mac Grok npm 外置卷与恢复准备 `48633082453dc38aef8dd11573bbd839482939f1`**：显式卷策略默认拒绝不变，补真实挂载、设备、规范路径/别名及执行前后复核；worker和runner同步绑定managed_process_macos.rs，共17份。Python41项、cargo check、i18n11项、定点格式及差异检查通过；build-01误用2021 edition的格式失败保留，build-02按仓库2024配置通过，无格式源码改动。新worker摘要 `6f75c417cacd423a86e3887c30db5a0491ac1a52f68041d46c7e86e8e7cfbd1d`，三文件补丁编译后与486330824源码逐字节匹配；复用bb78签名监督程序，实际二进制15份相关源码与包/二进制签名均复核，不能当作当前提交整包重建。五个固定官方包/十份输入原件通过SRI和每版29成员合同，不执行npm install/lifecycle。

同干净486330824检出的run-01计划四场景，首个updated以ProbeFailed失败：子worker在exec Grok之前报atomic_macos_ancestor_unsafe，首个不合格祖先是SanDisk卷根0775。原native退出1属于执行前worker；Job移除、资源coalition销毁及cleanup_confirmed均确认，候选未执行，后三场景未开始。run-02单独candidate_changed_preserved退出0，实际公开入口1.0.40，RecoveryRequired保留完整旧树、镜像/配置及被篡改候选和日志，无候选探针或managed generation；两项busy测试仅限既有模型状态机。原件位于resume-20260928/macos-grok-npm-recovery；run-01失败摘要 `a9bf4a5c3c9ba0935769b2b0c788dd3e56ae0c64672508a8cc233a02b6531387`、39项收据/日志索引 `f827f5350ff021794c5ca22138eb5193ecf0923fbf109e3f69a31236b3c9e095`；run-02成功汇总 `c8c79fcca8cf53455adc3bede5e3217afb891210835969da52e356b700863b01`、32项索引 `8cd6e505c69f16faf9ff442f62dfeff212a887194d069e13d8b886c692484346`。两轮独立复核通过，run-01审查摘要 `bccc96ad552fbb7dae8a6837533d44d67414688e81deeef9e301ec21125a0bb6`、run-02 `434b7f97e9b4babc554df56c728197824118398f205203f901c403596dc9fb87`；run-02完整旧树37节点/29普通文件与before/after/现场一致，prepared/checkpoint/retained日志原字节相等且probe为空。源码/产物前后摘要一致；零模型输入、无GUI或用户安装更新。当前正常与两项冷恢复仍未通过，独立SanDisk backing私有APFS映像方案待授权，未创建或挂载；不改既有卷权限/设置或降低产品安全门禁。历史Mac正常与Linux四场景证据独立保留；G09及14项必要缺口开放，无需本地化变更。

**2026-09-27 当前续作 `e2eba8596`**：修复 CLI 富输入误用内置模型图片能力及跨面板切换模型清除图片草稿；原 CLI 提交身份、版本、格式和权限门禁不变。Mac 本地 `cargo check --locked -p warp`、i18n 11项、富输入相关53项、actionlint通过，无需本地化变更。同提交 [CI 36313924070](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36313924070) 的 Windows 编译、504项定向回归及 Grok 生产安装器通过；显式原生12项中11通过、原 NUL 输出对照1失败，仅执行一次。新增 TITLE 和 GOTO/NUL/TITLE 两项均成功派生精确绑定的 CMD 子映像、原生退出0且清理确认，不能将 NUL 失败归为真实 npm 挂起的唯一原因。真实 Windows npm 更新未运行。Linux 同提交作业通过：主回归1386项、IPC2项、glibc绑定1项、命令归属1项、宿主崩溃清理5项、rust-genai81项通过，另有Grok监督清理4场景及Codex缺失会话／idle-crash边界通过。Linux真实npm更新、GUI及TUI专测未运行。整轮因上述Windows NUL对照失败；不计全工作区、Mac Intel或两平台GUI／模型通过。

**当前 Mac 联测包与实际产品验收**：`target/parity/InfiniShellParity-e2eba8596.app`，独立 profile `parity-sandisk-20260927`，ARM64、临时签名、四份内嵌英中 Fluent 及129项外置资源已核；签名后二进制 SHA-256 `e6e4d81e506a4aaf9d47e4f1965072e05db36748d01c587a056027c83e04fdf0`。固定 Grok `1.0.41/grok-4.7/default` 专属 TUI 经实际 GUI 选择中性文件名 PNG、中文提问并提交一次，得到正确识色回答及 NativeProtocol/end_turn 回执。产品持久引用、原生历史和原生图片消息均与178字节夹具相同，SHA-256 `e0b96cdc63f69c22914913b1d05151b537f5c73e1a57ca8bdb30efc1a7fbe0d3`；文字与图片共用一个 prompt，单 session 快照只有一次提交。ACK 后草稿清空，退出后重启仍只有一条产品消息且输出保留；不是活跃重关联或冷恢复模型回合证明。英文／简体中文专属入口、富输入占位符、按钮和图片卡片在2560×1600可读；中文文件名另验，未发送第二个模型输入。物理输入法、图片粘贴／拖放、多图、审批与跨平台真实链仍待验，G01/G03不关闭。

本轮小型原件位于 `/Volumes/SanDisk/InfiniShell-Archives/cli-agent-parity/resume-20260927`。`local/gui-owned-image-e2eba8596.safe.json` 摘要 `26382f8d123683c12bcf96af2bae54fe6acf34c4e5a6aefb09a50d50c4aa6c98`，独立原生收据 `local/grok-gui-native-image.safe.json` 摘要 `7a6544db95560e3ac2f6c7eb43382ff615831d4e700964d47d69adda2dff10e3`；Windows原日志摘要 `f30b95478b60b58a4e2c1d6c2c41ebe5c2a39c5915e0bd327aebc684be4a93e5`。先前 `91e04eed6` 的NUL首次失败及更早真实npm失败均保留。用户已清理部分旧缓存／工作树／归档，下文旧路径与“保留”措辞仅记录当时状态，不保证当前仍可用；本轮不恢复大体积历史缓存。

**2026-09-27 Windows npm 工作目录增量`05b0b8faa`（基线 `6408131e2`）**：仅在进程启动边界将目录转为经 canonical／卷号／FileID 核验的 DOS 表示；ACL 与只读文件清单仍用原规范路径，cwd／文件句柄及隔离合同保留。新增3项npm路径与2项command入口回归，长中文路径测试仅验证身份转换，不证明实际长路径启动。旧300秒 stdout 超时触发上层 stop，外层worker退出码不能当CMD原生码；新增固定阶段、耗时和计数帮助定位，未调整时限或清理判定。独立审查未发现确定阻断，本地 check、i18n 11项、actionlint及差异检查通过，同提交 Windows [CI 36280073722](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36280073722) 已结束：主组981项、command7项、显式atomic7项通过（含新增5项），真实更新首个CMD候选仍失败。旧UNC提示消失，但原生“拒绝访问”后缺完整清理收据；剩余进程和拒绝对象尚未定位，PowerShell、发布与恢复未执行；原件 `validation/windows-npm-cwd-20260927`。无需本地化变更，G09及Goal保持开放。 本轮诊断原件 `windows-npm-cwd-20260927/ci-36280073722/job-text-01/diagnosis.safe.json` 摘要 `9e0188e77c3c4b333ab20cf89eb44af25f0fba3067d00a090a727e80146e9113`；原job日志摘要 `2dfd0d5c02411c706ee3a3252c8e711cf097d580d79a99b7926aa00a71cacff7`。exit_reason为stdio_closed、cleanup_confirmed=false；239969ms为drain时钟，不能等同外层531.72秒或证明自然根退出。新取消预算审查确认认证worker通道在握手后释放、外层2秒窗口可能先强杀清理worker，详见 `cancellation-budget-audit-01.safe.json`（摘要 `266bdd6586364169bb1c2aa4ebd553b895b3be25fc0aeb32b5467c69a53910bd`）；此为独立实现缺口，不冒称本轮拒绝访问根因。

`0525c9439` 的协作取消已接入原认证通道，本地check、i18n 11项及control6项通过；同提交 [Windows CI 36282335500](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36282335500) 编译通过，普通回归986项通过/1项失败，失败为Drop的2秒回收等待（nextest既有3次自动尝试）；新增原生取消2项均通过、atomic9项和command7项通过。真实npm首updated返回Network、未形成journal/generation或候选执行/清理收据，不能继承上一轮cleanup=false，发布/恢复未验。后续修正 `49ea40561` 已提交推送，监听改为非阻塞读取及可唤醒等待，保留6项契约和2秒断言；独立审查、本地02 check、i18n 11项、control6项及actionlint通过。同提交 [Windows CI 36283908796](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36283908796) 已结束：编译通过，主组987项、control6项、command7项及原生atomic9项通过；Drop首次0.112秒通过，原生取消2项也首次通过。真实npm首updated仍返回ProbeFailed，原生拒绝访问后因取消中断；本轮已取得Job／ACL／profile清理标记及cleanup_confirmed=true。无原生根退出码，外层worker码1不代替它；PowerShell／发布及后续恢复场景未验。无需本地化变更，G09及Goal保持开放。 原始诊断 `windows-npm-cancel-20260927/ci-36282335500/job-text-01/diagnosis.safe.json` 摘要 `8668c0021c9520e23f213c0d41d2f3300b8e0464e18b68601f25c23d79136b2b`；原job日志摘要 `fea2d7a154fa3c93be5c92eeff5b217bad9396ab33a68a1a2ff5921c086cdc5f`。后续接收超时草稿因[Win32接口合同](https://learn.microsoft.com/en-us/windows/win32/winsock/sol-socket-socket-options)不保留，01编译主动中止，不能计通过；最终非阻塞源码与全部原日志存于 `windows-npm-cancel-drop-20260927`。 49ea本轮独立诊断 `windows-npm-cancel-drop-20260927/ci-36283908796/job-text-01/diagnosis.safe.json` 摘要 `b20eabf3f383956cfc96a1ad1f16b7c860ff417dc137d164d3965bbe71efc1f5`，原job日志摘要 `2eafddd0ddaad5262ef3a05e14b363ba0d3505f7378726a5cae73374bbfbf7af`。prepared-journal仅是探针前快照（probes为空）；缺最终journal、发布后树及恢复后版本，自动回收只由代码路径推断，不算恢复正例；39个小成员，两个小ZIP核官方摘要，大Codex ZIP仅按完整目录选择37/44并核CRC，未核整包摘要。

**Windows npm 诊断增量 `ae4279e48`**：固定下载角色与失败阶段、已核验 CREATE/EXIT 角色和清理模式已接入，授权、隔离及失败判定不变。本地 check、i18n 11项及 actionlint 通过；同提交 [Windows CI 36285301933](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36285301933) 已结束：Windows 编译、主组990项、command7项、原生atomic9项通过，真实 npm 首个 updated 返回 ProbeFailed。正常阶段仅记录 root／console CREATE；取消清理阶段记录 root／console EXIT1，另有尚未分类的 unknown CREATE／EXIT3221225738，不能据此认定 Node 从未启动或定位拒绝对象。最终取得 cleanup_confirmed=true 及 AppContainer 清理标记；PowerShell、发布与独立恢复未验。CREATE/EXIT 与 root_exit 使用同一 session 时钟，probe 阶段仍用自身起点，不直接相减。按用户代码优先、修复与联测后置要求封存本轮，不自动重试或追加猜测性诊断；G09和Goal保持开放。无需本地化变更。 原件 `validation/windows-npm-event-diagnostics-20260927`；静态审查摘要 `e0251f594d176b58ecdc059b3b59bc4321a78a1ea0ca1b6f100fe1a74d46db02`，提交绑定摘要 `9cd23ff3b9ad25e636bbe14afc20676d8fde39620fde93a403bf31e846ee4424`。 本轮原件索引 `ci-36285301933/review-summary-01.safe.json` 摘要 `db26756d2cadd5632fc722cfb2150cdeabab72cc7966394ba6a72d9f33d20fec`，原job日志摘要 `ebb87e37bec75b6be2189235af32fdc3b54b7f43c5fa2d361b8d3bb90f31b291`。测试程序退出101、监督收据退出1与清理事件原生退出码分别记录；npm大ZIP仅按完整44成员目录取37个小成员并核CRC，未核整包摘要。

**2026-09-27 Windows npm 精确控制台依赖增量 `43c9709d2`**：本提交纳入 `console-binding-04`，仅 Codex npm 公共入口候选预先绑定系统 API 定位的 `conhost.exe` 单文件、祖先与受保护 owner/DACL，保留 PE、FileID、大小及摘要核验；CREATE 事件继续前还须核真实进程句柄属于本次精确 Job、同 AppContainer SID 且零 capability，否则仍拒绝，未增加通用 System32 兜底。待退出句柄保存在 session，跨错误和超时保留、仅全部 signaled 后清空；Job 外被拒进程先通过真实句柄终止，再继续事件，无法确认则保持 Unknown。正常／中止均经退出确认后才允许 Job0／ACL／profile 清理收据，新增正例先清理再断言。独立只读审查未发现确定阻断，不是原生通过证明。Mac ARM64 `check-01` 已通过（99.26 秒）；`i18n-01` 在链接阶段因 errno28 空间不足退出101（237.07秒），首次测试未运行，原日志保留；按清单核摘要和无占用后清理两份未执行旧应用缓存，重试 `i18n-02` 11项通过（251.61秒），actionlint及差异检查通过。当前8876fa3c5待联测包及全部实际验收程序/原始收据保留。原件位于仓外 `validation/windows-npm-console-binding-20260927`；补丁 SHA-256 `1883326785a274f86abdfeb7db8cbf7426246080d3ab15d48b63a7e66cd29307`，`draft-04.safe.json` 摘要 `16b387586e42e22a6f87e96c039adf5a80b0fc4d54b60c39cdb37e47a1bf640b`。已推送；同提交仅 Windows [Actions 36277108115](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36277108115) 已结束，真实 npm 失败，具体范围见下一段；不继承下文 `ba73c4df3`／CI36275037039 结果，旧 conhost 拒绝及清理未知不回填。无需本地化变更，沿用英中探针失败文案；GUI／在线联测后置，Mac Intel 排除，G09 和 Goal 不关闭。

**上述同提交 Windows 结果**：作业 `108501893434` 编译通过；主回归978/978、command 5/5、显式原生边界7/7通过，其中新控制台普通3项、command 2项及原生2项通过，旧 `unbound.exe` 精确拒绝／清理反例通过。唯一失败步骤为真实 Codex `0.156.1` npm 事务：首个 `updated`／CMD 候选返回 `RecoveryRequired`，journal 为 `Prepared`，仅一个 generation；原 stderr 明确 CMD 不支持传入的 verbatim 工作目录，回退 Windows 目录后拒绝访问。具体拒绝对象和清理原因尚未证明；`exit.json` 为 `cleanup_confirmed=false`／`stop_requested`，缺 AppContainer 清理收据，不能写成清理通过。PowerShell 候选、发布和后续恢复未到达。后续代码定位是 npm 专属 `spawn_package_suspended` 仍把 `cwd.execution_path()` 原样传入 `CreateProcessW`，须保留目录租约／身份和所有隔离门禁处理路径；不认定它是唯一根因。本轮原件在本增量 `ci-36277108115`：原 job 日志 SHA-256 `159339121c0b2f027b3eb9ab48377f0584ae40af6a6ea8168bd700aa67e17ac8`（1965／2048／2238行套件结论），原 stderr `533025afc65a9b6e9ae781e546a1d2c6bd00f149c22d02a1b7baab5433a65824`，退出收据 `b83dd9d038d296d6c6fa54e94155e8ec74ae94a7c31e4fc4b2d9a70cc94ae1d4`。两份小ZIP核官方摘要；462446568字节npm ZIP仅按Range封存37个选定原始成员，完整ZIP摘要未核，不称全部产物完整下载。诊断收据 `job-text-01/diagnosis.safe.json` 摘要 `7d82e2cbedd6a11e4debee0c71c39a868c01e2f9d6a0469f8a99577ae60be5d1`，原成员采集索引摘要 `d48f377a456bc8faa11897ee1315fdc96de79cee230746b3f77a015167dce440`。按用户代码优先要求，本轮封存失败，修复及完整联测后置，未自动重试或弱化门禁。

**历史内置盘联测包（已由上述外置盘包取代）**：`2d159360a` 的 ARM64 应用已重新构建（180.45秒），产品源码与 `ae4279e48` 一致。`InfiniShellParity-2d159360a.app` 使用独立配置 `parity-code-2d159360a`，131项外置资源、四份目标提交英中Fluent签名前后完整字节、ARM64映像及显式临时签名均核验通过。签名程序 SHA-256 `47acf18de7594c3b3383c489efa2d61f1b1ef239f1fdde7901ddfc0e31390b55`；仓外 `validation/code-checkpoint-app-2d159360a-20260927/prepared-app-2d159360a/archive-index.safe.json` 摘要 `4f890da7f41b23ff0e1fe3b0cc94ccb6dea5ed020cb4c57dcb73813755d9c511`。未启动应用、查询认证或进行GUI／模型验收；默认feature的真实check与i18n11项按ae→2d产品源码／资源／workflow等价核对复用，不计debug-embed运行通过。43旧包保留。已被取代的887准备包先压缩为201528616字节原件（SHA-256 `6e0825fbebb671f88bec6f53d0dca0071e20b247ae7eac099a8067356bb458f5`），核196条目字节并另存属性清单后退休缓存副本；旧收据及先前成功／失败原字节保留。新构建完成后仅清理已无占用的两主库缓存，保留新unsigned程序和应用。后续纯文档提交复用此包，产品源码变动另行绑定构建。

**2026-09-27 Grok 图片与技能代码增量**：提交 `ba73c4df359751603133509a016b0ee92c139848` 已推送，固定 `1.0.41/grok-4.7`、Inherit 根会话的 PNG 与单／多技能已接线，组合要求私有独占 leader；统一编码预算，目录完整确认和排队后重验，旧无技能图片门禁保持。十项新增回归、GUI 组合入口与英中说明已加入。首轮编译、i18n11项、Grok464项（15忽略）通过；输入准备旧全拒绝断言导致28通过/1失败，原件保留，已修正过期断言。02门禁：编译、i18n11项、Grok464项（15忽略）、输入准备29项、任务管理器59项、actionlint和差异检查均通过。13个源码／资源／工作流blob与门禁快照一致，绑定摘要 `9f2bd34697722be57ed262db061b79ea67d4454c4bbd467dc7e4ff51cbe479eb`。同提交 [Actions 36275037039](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36275037039) 已结束：Linux 通过（主套件1376/1376），Windows 主套件1208/1208通过，两平台新增Grok组合各10项与i18n各11项通过，伪句柄回归通过；Windows原生Claude首场景45.03秒内未确认精确唯一SessionStart，另一特殊字符路径4.052秒通过，清理确认成功。原PTY只保留摘要，无法区分缺失/重复/额外通知，原因未定；整轮为failure。12份小ZIP均核官方摘要，原任务日志另经官方单job接口取得；诊断摘要 `218c312007ae811ba3fa6b955ce2cc2988f5bd957539f2f1e760ba6b870b6e31`，存于本增量 `ci-36275037039/job-text-01`。Mac Intel／全工作区／GUI模型关闭；未重跑已知失败的Windows真实npm场景，该故障仍开放。另修Windows Continue失败测试的伪句柄夹具，保留生产真实句柄校验。原生校准仅在 Mac ARM64 发出两次模型输入：首轮单技能＋PNG，次轮新 leader 冷加载同原生会话后双技能＋PNG；首技能原生展开、后技能经真实 read_file 读取，原图字节和独立识色均确认。零审批请求不作权限证明，也不证明适配器／GUI／热新增组合／跨平台成功。原件 `validation/grok-image-skill-calibration-20260927` 的审核摘要 `b58d6ba250e8719239fa56d01f59d4e58fcf278128da08bfcd0ad02216d5af08`，源码门禁另存 `validation/grok-image-skill-integration-20260927`；首轮清理路径别名误拒及身份复核后退出原件保留。G02/G06 和 Goal 不关闭。

**2026-09-27 Claude 图片与多技能代码增量**：实现提交 `3dbe79e588c26e0485a9c1f4d834c9dc96886efa`，固定 `2.1.280` 的图片请求已在工作区接通完整有序技能列表，准备预算与实际帧共用同一编码；注册确认、原字节、重投抑制、固定父权限及逐项审批保留。普通 Inherit 零／单技能图片编码保持，固定技能单技能图片改用共享前缀，历史字节收据不自动继承。九项新增回归与英中说明已加入。Mac ARM64：cargo check -p warp、i18n 11项、Claude 214项（9忽略）、托管输入29项、actionlint和差异检查通过。首轮测试编译在重复strip时空间耗尽，损坏产物及原日志保留；源码未变，仅本地warp测试包停用strip后重新通过，不改变依赖与全局配置。真实模型组合、GUI与新说明双语布局后置。另修正 Windows `pending_event` 为实际 `DEBUG_EVENT_CODE`。11 个源码／资源／工作流 Git blob 与冻结快照匹配，绑定收据摘要 `90d1e0ffe91c7c3c60fb7b772765a9bfc1d4a1373d6618c28e3baedda05e8777`。同提交 [Actions 36272432719](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36272432719) 已结束：Linux x64 通过；Windows x64 编译通过，主套件908/909，一项测试把当前进程伪句柄交给真实句柄校验而失败；新增Claude九项在两平台均通过。显式atomic调试4/5，System32 conhost先于目标未绑定子映像被拒；真实npm同一conhost也被拒，本轮清理确认成功，旧清理未知不回填。Linux主套件1310/1310。小产物ZIP完整摘要已核，大Codex ZIP仅保留37个所选原字节成员，完整ZIP/官方整包摘要未核；两平台独立job原文本另存。产物索引摘要 `562a5eb6e6221a77ebca64c19ce7d8d528b23e37238496aeac1b2e8cb45f7f62`、任务日志索引摘要 `9ab02676308921444c163d987cb59942734d0be6c6893a897dc661346741d8c4`；Mac Intel、全工作区、GUI／在线模型关闭。既有 `8876fa3c5` ARM64 联测包不含本次组合修改。本批原件保存于 `validation/claude-image-multiskill-20260927`，不关闭 G05/G06/G09 或 Goal。

**2026-09-27 Windows npm 探针拒绝后的显式清理**：实现提交 `ca37b9cbf2738ea4059c0a865bff2b1492ba1dcf` 保留完整映像白名单，补充固定分类／句柄身份／摘要诊断；未知文件名、完整用户路径、命令行和控制材料不输出。派生成功返回后的失败先终止精确 Job，再继续并排空原线程调试事件；pending 与 EXIT 集合只在 Continue 成功后更新。收据前有界等待根进程句柄退出及 Job 清空，再一次恢复 ACL／删除 profile；原映像拒绝仍失败，清理证明单独记录。派生内部尚未返回句柄的失败仍保持未知。

Mac ARM64 `cargo check -p warp`、i18n 11 项、actionlint 与差异检查通过；12 个文件 Git blob 与冻结快照一致，绑定收据摘要 `f7600b6cccbdda6592f60db1a843f013a742e94c79f83f5926da07b0ab1f52bb`，原件位于 `validation/windows-npm-probe-cleanup-20260927`。三个真实 Windows AppContainer 场景和三个普通负例已添加，尚未计运行通过。[Actions 36270700886](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36270700886) 仅选择 Windows x64：最小相关门禁、五个显式调试场景和真实 npm 事务；关闭 Linux、Mac Intel、全工作区及 GUI／模型。该运行已结束为失败：Windows 编译发现 `pending_event` 的事件码声明为 `u32`，实际 Win32 类型为 `DEBUG_EVENT_CODE`，报 E0308／E0277；后续原生测试及真实 npm 事务均未运行。工作区已修正字段类型，下一增量另验，不覆盖本次失败。完整日志摘要 `832a20ef89f5ed0bddd0e69e5d838fdf9cf7e613babdda3ee26e5dd0b77e2019`，产物索引摘要 `b4914883165cb3c6460bb32f5ace6f71f6945dab8ecf790ac13f298724cc19b7`，保存在本归档 `ci-36270700886`。此前 `bd80cc9ed` 原始失败保留，被拒子映像仍未取得新诊断。此批无需本地化变更，不关闭 G09 或 Goal。

**2026-09-27 远程图片恢复与 Windows npm 模板兼容**：`5aac3f7d0` 补齐 Claude 延迟审批后的图片状态查询／回收，以及 Grok 明确未派发取消后的主动重试。Claude 原生事件最多触发三次只读补查，固定 `2.1.280` 的原字节证明工具完成 hook 可早于最终历史写出；无精确回执仍保留 Unknown。Grok 仅在 Submit future 尚未创建时保存绑定完整 ticket／message／subject 的未派发终态，原领取字节保留；真正开始 RPC 后仍不重发，两路径均不清除新草稿。新 Grok 提示英中同步，布局待验。

`97649deb2` 修正真实 npm `10.1.0` 对命名空间 Windows 路径的递归问题，转换前核验同一文件对象；诊断采用固定官方 Node/npm 原字节的独立纯函数复现，未冒充 Windows 新安装通过。`bd80cc9ed` 另接 `cmd-shim 6.0.1` 与既有 8 的整组三件套精确校验，旧事务三份 SHA 固定模板，候选／退出／恢复不得切换到另一模板；产品仍复制真实入口原字节，不升级 Node/npm、不改写验收 shim，也不放宽 AppContainer。此部分无需本地化变更。

本地 Mac ARM64 第二轮编译检查、i18n 11 项、远程图片 36 项、更新模块 199 项（6 忽略）、footer 47 项、Grok 上下文 7 项及 actionlint 全通过，包含新增 19 项回归；Python 运行器 15 项分别在 3.12.13／3.14.6 通过。19 个冻结文件与 `bd80cc9ed9a6dbc032602e457a64d1c55e7998ba` Git blob 一致，绑定收据 SHA-256 `6265114b19a1c4b517ae4f56f7cf5ae2f00689b089f0c6ff63b6debc6129e1f3`；原始日志、固定 Claude 时序复核和清理收据位于 `validation/remote-image-recovery-20260927`。第一轮测试链接因空间峰值失败，832 字节坏产物及日志完整保留，未进入测试；第二轮通过不覆盖旧失败。仅清理确认停用的项目编译缓存，真实原生程序、应用和证据保留。

同提交 [Actions 36266539274](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36266539274) 已结束为失败：Linux x64 通过；Windows x64 编译、903 项定向 Rust 回归通过，但真实 npm 首个更新场景失败。Linux 定向 Rust 1,068 项通过，两平台各 11 项 i18n、G08 新增 10 项均通过；跳过项不计通过，独立图片客户端 crate 步骤未执行。本轮没有 `LEAK` 标记，不据此关闭旧运行疑点。macOS Intel 明确排除。

Windows 原始 npm 登记成功，三件套与固定 `cmd-shim 6.0.1` 原字节一致；产品在 Prepared 阶段的首个 CMD 候选探针拒绝未绑定子进程映像，返回 `RecoveryRequired`。现有日志未记录被拒映像身份，不能据此增加系统映像白名单；PowerShell、旧版本移动和发布尚未到达，其余四场景未执行。`cleanup_confirmed:false` 保留：错误路径未生成 AppContainer 显式清理收据，Drop 的尽力清理不证明 Job／ACL／profile 已全部恢复。13 份原始产物 ZIP 及完整日志保存在本运行 `ci-36266539274`，索引摘要 `336d2074d748526ba52d3b9de0446f7ab8fa5d32cd3492ab0a4060038e916d2e`，平台逐项审查摘要 `52bf8ffe432abeff199f0781f90521d3029bd62f91dc23a3cf7af6a376320ab2`。实际远程延迟 Allow／Read 回收、GUI 双语、认证模型及完整生命周期仍后置，G08／G09 与 Goal 不关闭。

新版 Mac ARM64 联测包已按 `8876fa3c5a3950154c8569ea1344507f38f964eb` 构建，其产品源码与 `bd80cc9ed` 完全一致；路径为 `/Users/zhishi/Library/Caches/InfiniShell-Desktop/cli-parity-apps/8876fa3c5/InfiniShellParity-8876fa3c5.app`。签名前摘要 `98244b1f7e9686e64f3c45381e5f4ae73260925f8303b850e00ccccae15d06de`，签名后摘要 `e3cf04b650d0b719075e45d270a5b03ea4c25f3bbe8d049d90cf7cd22b6d3423`；四份完整英中 FTL、131 项资源及 ad-hoc 签名核验通过。包装收据位于本运行 `prepared-app-8876fa3c5/bundle.safe.json`，摘要 `5ba79365f640e5e5da5f5133f792f31972609e32a6d4c6457b66ac63af3b8622`。应用未启动，独立 profile `parity-code-8876fa3c5`；未检查认证、未计 GUI／模型通过，旧包不动。此轮仅构建交付 app，默认 feature 的编译／i18n 门禁按同产品源码复用，不冒充 debug-embed feature 的运行测试。

**2026-09-27 Grok 上下文与升级空闲通知补接**：功能提交 `debed8c51cc54f1d017fda809c9254fbc658b80b`，独立夹具修正 `c7ef8d95b48df4a01f64985471b42e80beeda1eb`，均已推送。有效本地／远端专属 Grok 的代码、评审和 diff 上下文现在统一先打开富输入、等待原草稿恢复，再按同代次顺序追加；已经打开的专属会话也走同一队列，连续上下文不会抢在恢复前被覆盖。取消、关闭、换代或失效时旧回调不写入，未绑定普通会话不因此获得 PTY 发送能力。Grok 操作提示已同步英中，实际双语布局仍待共同验收。另将升级器既有 sessions model observe 对齐三支持平台，使 Linux／Windows 无存活 pane 的专属 Grok 清理后能够重新同步空闲条件；没有新增终端锁或第二份观察订阅。

本地 Mac ARM64 编译检查、i18n 11 项、Grok 视图 7 项、footer 47 项、更新模块 190 项（6 忽略）及 actionlint 通过，含四项新增顺序／旧回调／失效／未绑定回归。十份源码／资源／工作流 Git blob 与冻结快照一致，原始日志、独立静态复核和绑定位于 `validation/grok-context-routing-20260927`。旧 Mac `2b59c8c62` 联测包保留但不含本批 UI 修改，新包及真实三平台／远端发送仍待补；G01/G09 与 Goal 不关闭。

`56f6da217` 的 [Actions 36262495627](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36262495627) 已结束为失败：Linux／Windows 基础编译及定向 Rust 分别 958／818 项通过，Linux 固定 Node 原文件的只读依赖闭包 1 项通过。两平台离线合集原始日志均确认 `Reparse` 测试夹具缺 `st_mode`，与已提交的 `c7ef8d95b` 修正一致；CI 实际 Python 3.13.15。Windows 真实 npm 私有登记另返回 `Maximum call stack size exceeded`，尚未生成真实 shim 或进入产品升级事务，原因继续诊断，五场景不能记作执行。完整原始日志 ZIP 摘要 `17bd8e61eec57354bfc196ec02fe065fc91a7d30794c43bc11328cb9eb8494c0`，运行结论收据摘要 `174bda32c1dee1a1fa33f470dd38c20cd60c2aa54ef8d002ba2f3250364bb918`，统一归档于 `validation/g09-followup-20260927/ci-36262495627`。本轮 Windows 定向无 `LEAK`，不据此关闭旧运行的泄漏疑点。此运行不覆盖后续 `debed8c51`。

本地 47 脚本／1,164 项 Python 3.14 全通过，Python 3.12 独立复现上述夹具错误；仅修夹具后 11 项在两个版本通过，生产检查未改。此前诊断字段的 CI 3.11 推断已另存更正，原字节保留；原始批次与更正在 `validation/g09-followup-20260927/offline-batch-diagnosis`。本轮还按已授权范围，仅清理两份停用 Cargo 测试链接输出共 1,604,934,608 字节；身份、摘要和无占用均核验，原始验收程序／日志／源码／工作树保留，清理收据为同归档 `retired-test-link-cleanup-01.safe.json`。

**2026-09-27 G09 Linux ELF 修正与 Windows npm 实测入口**：实现提交 `56f6da21789c2aa90abf2fdff6a2e10aea03283d`。旧 Linux Codex 失败的直接原因是官方 Node 20.9.0 的 `DT_STRSZ=5,291,274` 被误套 1 MiB 动态记录表上限；本次保留整段唯一映射和文件边界，改为每个已引用依赖名至多读取 256 字节。新增七项边界回归及固定 Node 原字节的只读系统依赖闭包测试；依赖名、加载器、网络和 Landlock 门禁均不弱化。ABI1 仍独立限制候选隔离，不把解析修正等同于该主机可以执行 Codex npm 更新。

Windows 新入口通过真实 npm 在私有前缀登记固定 `0.155.1`，并调用产品后端更新至 `0.156.1`；包含正常更新、OldMoved 冷恢复、发布后缺收据冷恢复、外部改动保留、候选改动拒绝。cmd、PowerShell 两候选各需真实 AppContainer／Job 退出收据；三种 shim 保留 npm 原产字节，不为通过而改写。固定渠道和断点只进入测试构建。初始 npm 登记不是产品 Job 隔离证明，超时不得记作已清理；消费者渠道、忙碌／插件重检及 GUI 均不在此入口结论内。

本地 Mac ARM64 `cargo check -p warp`、i18n 11 项、更新模块 190 项（6 忽略）、Python 11 项及 actionlint 通过，八份源码／工作流与提交 Git blob 一致。原始日志及审查位于 `validation/g09-followup-20260927`；此前 actionlint 的 SC2155 失败保留，拆开赋值与 export 后独立通过。同提交 [Actions 36262495627](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36262495627) 的实际终态与失败已在上文记录；本批无需本地化变更，不重包或重跑 Mac GUI，不选择 macOS Intel，G09 与 Goal 均不关闭。

**2026-09-27 G09 Linux 隔离能力前置增量（本地门禁通过，平台及布局待验）**：实现提交 `2b59c8c62c7c46bc30211a9872f966c52317202e`，基线 `6db38e0da2fd0c066eeb31f4f2de9f7b446fd4fb`。仅 Linux Codex npm 和三款 Linux Homebrew 在版本确需更换、且既有来源／版本／渠道检查无错误时查询 Landlock ABI≥3；不可用时不生成计划，既有状态机清除旧计划并阻止手动或自动派发。Grok npm、其他来源、macOS／Windows以及同版本无需候选执行的检查／配置同步不加此门槛；原 worker 的 ABI 拒绝与实际隔离规则安装保留，查询通过不代替执行时核验。新增错误提示已同步英文／简体中文，实际双语布局仍待共同验收。

原始收据位于 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/g09-isolation-preflight-20260927`：`source-commit-binding-01.safe.json`（SHA-256 `14e2ea8349e7273c568ecdd34f601ad750a2104228bb9a669e68ddc30d21ddf9`）确认 9 个 Git blob 与 `source-snapshot-01.safe.json`（`16fa1247cbaf7db9d74db7ea5f2ef7f83f0c52dce530be768e0f38bca46a616e`）一致。macOS arm64 `cargo-check-01` 通过（98.56 秒）、`i18n-01` 11 项通过（252.42 秒）、`updates-01` 190 项通过／6 忽略（4.67 秒，含新增 4 项行为测试）；`supervisor-build-01` 通过（177.29 秒，日志 SHA-256 `c265c002e38c944e2253a86ce79d735e82f08bfb2eb9e902f0a9b1d448834113`）。另以显式 `rust-embed/debug-embed` 运行 `embedded-i18n-01`，11 项通过（262.10 秒），收据 SHA-256 `407e119ffb4c3f16840b9172f51d71672d5cbd43dfff9fbea8e29d8e5b10263b`；嵌入资源应用已按下述独立收据完成包装；实际双语布局仍未计通过。这些本地门禁不证明 Linux 内核能力或真实包事务通过，`2b59c8c62` 的相关平台同提交 CI 仍待补。

同源码的 **Mac ARM64 联测包已准备，尚未启动**：`/Users/zhishi/Library/Caches/InfiniShell-Desktop/cli-parity-apps/2b59c8c62/InfiniShellParity-2b59c8c62.app`。交付构建显式启用 `rust-embed/debug-embed`，四份英文／简体中文 Fluent 文件的提交原字节均在签名前后程序中核对；131 个外置资源与原账本一致。签名前 SHA-256 `2b8525dd45177214659f11d4fed71c763bc30a54029b1ade835b2e96944c6afc`，临时签名后 `a42a0439886fc5174ba6db7d51211949133c4a9232e3e448d22d946bbb6029fd`；严格签名验证通过，使用独立 `parity-code-2b59c8c62` profile，不查询开发者证书，不改旧配置。收据 `prepared-app-2b59c8c62/bundle.safe.json` SHA-256 `6d706e45584115fce4feef72120921937969356820358bb587f77f71e20af6e4`，索引 `92a3d9bfc3f94cb1ac60cbc452430e5910b22c6387aebe9f2b4ffca57e8fef50`。默认后端与交付程序同源码、不同 feature 与摘要，不将后端正常升级结果当作此包 GUI／模型通过。

首次 `embedded-build-01` 虽返回 Cargo exit 0，日志含 `rust-objcopy` 输出空间不足，3280 字节文件不是 Mach-O；包装器在创建应用前拒绝，失败日志和两份无效原字节保留于同归档。按用户授权，仅清理已核对摘要／身份且无进程占用的停用单测及旧主程序编译缓存；`completed-test-cache-cleanup-01.safe.json` 与 `relink-cache-cleanup-01.safe.json` 分别为 `f401c7442cac2a7a161ac6784ee0693ffb213490cc8d06dbb0360631e11f9fd3`／`078ea5d47e36c0c349b790bd1b21d889872604fead82b5fa32509a38b135e4f4`。复用已编译库后的 `embedded-build-02` 用时 7.94 秒，链接及后续 ARM64／资源／签名验证通过，日志 SHA-256 `44b83bcafd01a532a4f3add33b5a5708c2d70fde7c06692074cfb1441b98737e`。原始失败不覆盖，真实运行 worker／supervisor 与旧应用仍保留；此处无需本地化文案变更，也未开展 GUI 布局检查。

`2b59c8c62` 的 **macOS arm64 Codex npm 私有正常升级已独立复核通过**：`codex-live-09-normal` 固定官方 `0.155.1→0.156.1`，运行器 exit 0（354.88 秒），真实生产来源识别、npm 只读 prefix 查询、Node→`bin/codex.js --version` 候选探针和包交换完整执行；原始 stdout 为 `codex-cli 0.156.1`。18 个运行器追踪源码逐个匹配该提交 Git blob 和运行后字节；`working_tree_dirty=true` 对应同时编辑的文档，不能省略此记录。worker SHA-256 `0ecd5d870874e3eb64b89d773a11a664cdf97afa5c4f34a28c8e921389f96d9e`，监督程序 `974cea9a90df40a3c57fbcf0106db8bcbdb12f0fdd4e20db266d6ad3a57e27df`；二者及 Node／npm 摘要再次核对一致。四份官方新旧归档 SRI、47 个发布文件和 69 个候选绑定文件逐项复核；根 inode `93465119→93465346`，uid/gid/mode及公共 symlink 身份保留。generation `59b819cb-c243-4d70-b27b-14ff4625c4a5` 的 exit/binding/manifest/coalition/native 摘要相符，`exit_code=0`、`cleanup_confirmed`／`job_removed`／`resource_cid_destroyed=true`，原 PID 和 launchd 项已不存在；旧 stage 和活动 journal 已移除。私有 fixture 原本缺失的 `.codex/config.toml` 仍缺失，不声称验证了用户真实配置。

该成功的 46 份原始小文件（411,026 字节）逐字节复制到上述新归档的 `codex-cache-normal-success-receipts`，原件、旧失败和大包／程序原位保留；`index.safe.json` SHA-256 `1277e2bb2e980bad3281738115205d51cfe7d223092dcfc390831b32436dfdff`，独立复核记录 `independent-review.safe.json` 为 `29ea4436b3ee4379f7347cb3bc3b30a37649a493924a3b946061edee3e239ec1`，原 `summary.safe.json` 为 `4e2badeb87bf60fbb66b73aaecdb17574863712988e530bff9f9842a216d35a2`。本轮 `recover_invoked=false`、零模型输入、未执行 npm install／生命周期脚本；目标来自固定测试钩子，不覆盖消费者实时渠道、GUI、忙碌／插件重检，以及其余三种恢复／故障场景。不能据此关闭 G09 或 Goal。

旧提交 `39941b281` 的 [CI 36257833211](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36257833211) 已结束：Windows 作业成功，Linux 作业失败且唯一失败为 Codex npm；Linux／Windows 编译和定向模块通过，不能继承给 `2b59c8c62`。其 `ci-36257833211/linux-environment.safe.json` 只读确认 euid 1000、Landlock ABI1／`abi3_available=false`、`/home/linuxbrew` 缺失；ABI1 不满足 Codex npm 和三款 Linux Homebrew 的候选隔离条件，Grok npm 不使用该门槛，须看实际事务回执。`unshare` 返回 127 只说明命令不可用，不证明内核禁止 namespace。Windows 预检的 `winget.command_found=false`、HKCU 可读和非管理员身份，也不能单独证明已登记 portable 来源事务可用或不可用；来源、ACL、依赖、候选隔离和真实事务仍须验证。Linux Codex npm 已取得的实际失败为 `ProbeFailed`，generation `8ace5170-0ab8-4bed-8fc8-300ef3323612` 的 `supervisor.stderr` 为 `managed_process.linux_glibc_elf_invalid`，退出收据为 `stdio_closed`／exit 1／`cleanup_confirmed=true`；尚未观察到该次执行的 Landlock 拒绝，不能把 ABI1 环境事实替代实际错误。原件位于 `ci-36257833211/codex-npm-artifact-10911429927`，ZIP SHA-256 `938191725ef7c9a273f13cb5d546adc169519a326d9acf280ebfe48a1b497fcb`，`failure-review.safe.json` 为 `1c803d57fdd6f0ecd90707b16a09fcd3825b39f52626fcede31a47cf0ae291eb`。Windows 作业虽 success，`windows-job.raw.log:4136` 的 `supervised_cancel_before_control_handshake_confirms_process_tree_exit` 标为 nextest `LEAK`，该步骤为 5 passed（1 leaky）；未扩展诊断或重跑，不能宣称完全无残余进程／句柄。安全拒绝不计升级成功，G09 与 Goal 保持未完成。

`39941b281` 的 **Linux x64 Grok npm 四场景已独立复核通过**：正常 `1.0.40→1.0.41`；交换后缺收据由新进程冷恢复至 `1.0.40`；外部改动由冷恢复保留在 `1.0.41`；候选改动在启动前拒绝并保持 `1.0.40`。三份真实退出收据与 manifest／exit-binding 摘要相符，均确认清理；两次恢复分别有 execute/recover exit 0 及对应前后树。147 个原始 artifact 文件和原 ZIP 保留，ZIP SHA-256 `4ab0266dc82cdf588b2e28cc3d9c58e335d295faa390cceff61fcf9ac21d6428`；16 个追踪源码逐个对应该提交 Git blob。完整收证 `ci-36257833211/completed-review.safe.json` SHA-256 `e7bbda8643cd2ddb8593baab1b66a7a55a5658ad24a37c516ed57807104310ab`，Linux 原日志 `2bc92dd48e6fb761574351725a64e7cfaa7f122900edae90f61a573d61b49791`。Linux 定向模块 947 项和五项无模型清理边界通过；Windows 原 `LEAK` 不改变。此结果限私有固定候选后端，不包含消费者渠道、GUI、模型、插件复检或 G09 整体完成。

**2026-09-27 G09 私有 npm 真实验收入口增量（进行中，未关闭）**：基线 `acc29e1549fb09e6e0dce1549af5ade469df30ef` 的增量已提交并推送为 `39941b2815506997198091082b92d19e11d92761`；`source-commit-binding-01.safe.json` 确认 17 个代码／工作流／脚本 Git blob 与 `source-snapshot-05.safe.json` 一致。新增 Codex `0.155.1→0.156.1`、Grok `1.0.40→1.0.41` 私有 npm 后端正常更新、交换后缺收据冷恢复、外部改动保留、候选改写拒绝四场景入口；Linux 窄工作流复用同源码 libtest／supervisor，Linux／Windows 包来源环境预检只读、独立归档。两新 artifact 仅收集私有白名单，包含隐藏 `.local` 和最新 `codex-npm.json`／`grok-npm.json` 账本，不扩大到用户 HOME。产品修正包括 Grok 新版本 Mirror 继承旧 uid/gid/mode、Codex macOS Node 裸加载路径解析，以及离线探针的根目录 literal 只读、精确系统 dyld 缓存目录读取和空 OpenSSL 配置；签名、来源与身份核验未放宽，未开放 OS／Rosetta 整目录或网络。

该提交的最新 macOS arm64 本地门禁：`cargo-check-07` 通过（97.97 秒）、`i18n-06` 11 项通过（260.46 秒）、`macho-paths-03` 3 项通过（2.74 秒）。此前 updates 186 项／6 忽略、Python 52 项与最终工作流 `actionlint-03` 通过，原始运行记录保留；`supervisor-build-01` 主动中断，build-02／03／04 通过，build-05 亦通过（178.10 秒，日志 SHA-256 `a04e074aa243a6cd8b2b4c93f3566d1871ad5974b37510e1202afab959f12654`）。[Actions 36257833211](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36257833211) 绑定该 `39941b281` 提交，Linux／Windows 编译和定向模块通过，最终整轮 failure；仅选择 Linux／Windows，开启两款私有 npm 运行器与 `native_acceptance_only`，关闭全工作区测试和 Mac Intel。Grok Linux 四场景结果见下文；Codex Linux 失败保留，不能覆盖后续 `2b59c8c62`。

旧失败全部保留：Codex 验收模块误用 Stable，生产按真实 Latest 渠道拒绝，仅修验收模块；`codex-live-03-normal` 的 `UnsupportedSource` 不改写。依赖闭包修正后的 05／07 仍为 `ProbeFailed`／Node dyld 初始化 SIGABRT，`native_wait_status=6`，两轮精确代次清理均已确认，不计升级成功。07 的 PID `12908`、generation `29ce7311-7a17-4066-8b29-e5fee11fb238` 及 `cleanup_confirmed`／`job_removed`／`resource_cid_destroyed=true` 见 `codex-cache-normal-03.safe.json`、原日志和 `codex-live-07-dyld-diagnosis-20260927`；小收据逐字节副本位于 `codex-cache-third-failure-receipts`，索引 SHA-256 `9d9db256d171f8170395391b774168fa039bf864671c74e5041e0d27d0642278`。05 的诊断、32 文件副本和更早原始输出仍保留。Grok Documents 内首轮候选监督首次连接 10 秒失败，候选未执行、仍为 `cleanup_unknown`；PID／launchd 后来退出不补造清理确认。相同监督程序错误入口约 0.0884／0.0847 秒返回不证明加载慢，未扩大 timeout。

后续独立实测将读取范围收敛为根目录 `literal "/"` 与精确系统 dyld 缓存目录：`dyld-private-node-start-02` 的 `root_only` 返回 Node 版本并退出 0；`dyld-private-codex-entry-01` 的原包入口因全局 OpenSSL 配置访问拒绝退出 1，失败保留，同轮绝对路径／openat／符号链接越界读取、包／Node 快照写入、网络及额外执行七项负例均返回 EPERM。`dyld-private-codex-entry-02` 仅将同一私有字节包的 `OPENSSL_CONF` 设为 `/dev/null` 后返回 `codex-cli 0.156.1`、exit 0，原包字节未变。该环境设置只用于 macOS 离线版本探针，不修改全局配置或正常模型会话，也不证明其 TLS／FIPS 配置兼容。静态依据和精确收据索引为 `codex-libignition-static-evidence-20260927.safe.json`，SHA-256 `7c0edf67fcdbf0f180b826d071793fe00e8558b0e58260faacccb997c1a5109e`。**独立入口成功不等于完整更新成功**：Mac 修后完整 Codex npm 正常升级此前因内置盘不足运行器 3 GiB 门槛而暂停，该历史记录保留；精确清理后，`2b59c8c62` 的 `codex-live-09-normal` 正常场景已通过下述独立复核，旧失败不回填，也不降低运行器门槛。

Grok macOS arm64 的 `grok-live-03-normal` **仅正常升级路径已独立复核通过**：固定官方 `1.0.40` 私有 npm 安装升至 `1.0.41`，完整包与用户镜像、npm 只读来源查询、公共入口保留及候选正常退出／清理均有收据；零模型输入，没有执行 npm install 或生命周期脚本，`recover_invoked=false`。该轮 worker SHA-256 `23c0e6639faac76634e22b91b41287a63b9207014492fa9e18be08264bb54e3c`、监督程序 `0b68f31f0bd678357cb7f6ff42803bd6826c12e11ae727352ba1cc7e8c12444e`；`/Users/zhishi/Library/Caches/InfiniShell-Desktop/cli-parity-npm-live-20260926/grok-live-03-normal/summary.safe.json` 摘要为 `d9f7a6b51850be70a3318773f071954dae7fbdf89e56c2d954eac6f4cf713145`。原 Documents 失败不回填，不能外推其余场景通过。

本批主归档为 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/g09-npm-live-integration-20260926`，历史成功、失败及二进制摘要保留在原收据和 `CURRENT_STATUS.json`；源码提交绑定、本地构建及同提交 CI 收证已完成；CI 整体失败，平台与场景结果分别见上文。固定测试候选仅覆盖包更新后端，不包含消费者实时渠道解析、GUI 更新入口或模型生命周期；Homebrew／WinGet 真实升级与恢复仍欠，Windows 本批运行器只纳入脚本可移植性及 cfg 门禁，Mac 仅 Apple Silicon，Goal／G09 不关闭。`39941b281` 增量**无需本地化变更**；后续 `2b59c8c62` 新增原因提示及布局边界见上文。用户授权的两轮精确清理分别保留于 `g09-unused-build-cache-cleanup-20260926/result.safe.json`（6 个旧中间文件、2,051,019,161 字节）和 `rlib-rmeta-unused-cleanup-20260927.safe.json`（2,253 个旧 `.rlib/.rmeta`、1,690,137,365 字节）；依据实际空间监测保留必要余量，不声称始终保留至少 12 GiB，这两轮未删除二进制、证据或夹具。后续 `unit-test-cache-cleanup-20260927.safe.json`（SHA-256 `be9a5e8064eba29e713b5ceadd48aef097f4746d06b3a5083ea60f12c1848f11`）仅清理 7 个已停用单测可执行缓存，逻辑字节 4,936,347,344，实际可用空间增加 4,921,823,232 字节；保留 5 个当前或原生验收角色、全部原始收据／日志／元数据。清理后当时可用 6,597,607,424 字节，不代表后续实时余量，也不改变旧失败。

当前代码批次以 `76dcfdc60254077edaa86175722591333526c05c` 为基线，按用户要求先完成功能接线；代码已提交为 `704bb33bb2943f8a686b24b843c3b3a1ba656c19`，macOS arm64 内置盘 `cargo check -p warp` 及 i18n 资源门禁 11 项通过，该代码检查点当时未运行功能测试、GUI 或在线模型；后续无人值守回归单列如下。三平台 Grok 专属普通终端、远端图片协议、包管理器事务和受限技能的代码范围见 `CURRENT_STATUS.json.current_work_order`；不继承下文历史门禁的通过结论。用户已明确 macOS 仅支持 Apple Silicon，Mac Intel 不再属于实现或验收范围；本轮新增的 Intel 清单与适配已撤回，仓外静态下载资料保留，不计成功验收。

本轮另外归档固定 Grok npm 三平台、Homebrew cask 与完整依赖的官方静态原始材料：仓外 `validation/grok-package-static-contract-20260926`，34 个文件，索引 SHA-256 为 `bab479385537d2a73069e27a8ce832041d3b6b5e9370b6475e41e71a716fec12`。Windows npm 原生映像与官网映像的所有 PE 节内容一致，移除官网尾部签名并规范化 checksum/security directory 后逐字节相同；两种发行映像仍各自绑定摘要。此归档只证明源码/包/映像静态合同，没有执行 CLI、安装脚本、升级、模型或 GUI，不扩大历史验收范围。

本批 Windows 受审命令、Grok 普通历史继续、受限降级及 Linux Codex cask 的代码/静态原始资料分别存入仓外 `validation/g10-windows-mcp-code-20260926`、`grok-owned-history-code-20260926`、`claude-downgrade-code-20260926` 和 `codex-linux-brew-static-20260926`，索引摘要见 `CURRENT_STATUS.json` 的 `current_work_order.code_archive_indices`。Linux Codex 固定官方归档为 145,976,992 字节，SHA-256 `8b711520beddf385467b8da4d2c93736637c6ba1e46811cf0d8606b7c490b6f6`；44 文件/10 目录与固定清单的字节、类型、模式一致，原生 ELF64 x86_64 没有 PT_INTERP/DT_NEEDED。这里只列静态合同，未执行归档内程序或重跑模型，不能据此关闭 G09。Homebrew 的 `LATEST_DOWNLOAD_SHA256` 只在 `cask.version.latest?` 条件下写入，数字固定版本不因此增加该文件；`@latest` 名称不等同 `version :latest`。

Linux Claude cask 和其余 Linux cask/Grok WinGet 官方材料已分别复制归档至 `validation/claude-linux-brew-code-20260926`（29 文件）和 `validation/linux-brew-grok-winget-static-20260926`（25 文件）；原临时资料保持原字节，索引见 `CURRENT_STATUS.json.current_work_order.code_archive_indices`。三款 Linux Homebrew 与 Grok WinGet 的最后共享接线已经合入工作区，源码绑定包含 69 个唯一且存在的文件。上述均为静态资料／源码归档，未运行候选、安装器或测试。

G10 后续增量已接通三平台宿主逐命令监督、同会话顺序多命令、逐次审批和原生 Submit；原生 shell 保持关闭。Unix 独立执行器原始代码存入 `validation/g10-unix-command-code-20260926`，整批代码检查点及编译日志位于 `validation/code-integration-g10-sequential-20260926`。第一次编译因缓存参数不一致主动中止，不记失败或通过；恢复原参数后的第二轮 exit 101，24 条类型/可见性/借用/Send 错误保留，相关接口修正后第三轮 `cargo check -p warp` 通过。i18n 第一轮在测试程序编译时出现 3 条错误，补齐测试构造字段及两处字节断言后第二轮 11 项通过、0 失败；原失败均保留。223 个代码、插件与资源文件逐个核对 Git blob 与本地门禁快照一致，绑定收据为 `implementation-commit-binding.safe.json`，摘要见当前状态。这里仅确认本地编译与英中资源约束，不包含功能、模型、双语布局或跨平台通过结论。代码检查点先按用户指示完成实现，后续仅推进无人值守门禁；GUI／在线模型联测仍后置，Mac Intel 不再选择。

代码检查点后的无人值守本地回归以 `c72313648895e7cb1d0d25a721ea8346403324d5` 为来源：CLI 组 1,832 通过／17 失败／67 忽略；远端图片服务端 1 通过／25 失败；Grok owned 组 50 通过／2 忽略；图片客户端 5 通过。Codex 来源 Python 首轮 10 通过／3 错误，历史 payload 夹具修正后 13 项通过。原始日志保存在 `validation/post-code-local-gates-c72313648-20260926`。失败发现远端图片暂存目录未在创建时显式设为0700，同时旧技能／插件测试夹具和 npm 支持断言未同步；修正未削弱权限或未知版本门禁。首轮修正后编译与 i18n 11 项通过，受影响回归 1,880 通过／1 失败／67 忽略：原失败均消除，仅跨进程重关联夹具的事件断言失败。源码核对发现夹具在 ACK 与后续 Progress 之间即可保存游标，需明确等待两者后再交接；原输出未含实际违规事件，保留证据边界。只修测试 helper 后，受影响的活跃重关联、宿主崩溃和离线回收三条进程用例全部通过，i18n 11 项再次通过；原整批失败记录保留，不改记为整批通过。原始后续收据存入 `validation/post-code-handoff-sync-20260926`，索引摘要见当前状态；无需本地化变更。图片客户端 5 项、通知 Python 20 项、插件兼容 Python 14 项、修后来源 Python 13 项及 actionlint 亦通过。上述修正提交并推送为 `bc9bb1f28d08717f6a5ded8a562571779a653ea0`，同提交 [Actions 36244024291](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36244024291) 首轮 Linux/Windows 定向门禁均失败：Linux 图片验收夹具 4 项失败及权限校验函数私有可见性 E0603；Windows 运行器仍要求旧 rev5、历史候选混用当前 payload，另有 AppContainer 继承标志 ACE_FLAGS 类型错误。两平台编译后的定向 Rust 测试未运行。完整日志和 7 份原始产物 ZIP 存入 `validation/cloud-post-code-bc9bb1f28-20260926`，索引摘要 `c89e7f2edc1182bcd3b2e714c18fd841d27093838abf14136381009ec2cc4b2b`。两平台已完成的无认证原生探针仍按其各自范围有效，不代替模型生命周期。Mac Intel 与全工作区测试不选择；此轮没有 GUI、输入法或已认证模型验收。

首轮云端失败后的最小修正：Linux 仅将既有权限校验函数开放给父模块，Windows 继承标志改为与原数值一致的命名 ACE_FLAGS；Codex 正式运行器精确同步 rev6，旧候选仍核归档 payload 的原 SHA；Claude 图片夹具仅补其 Python 加载器环境，生产 CLI 环境白名单不变。内置盘 `cargo check -p warp` 与 i18n 11 项通过，Codex Hook Python 48 通过／2 项 Windows 专属跳过、Notify 6 项通过，Claude 图片运行器 9 项通过。新增修订混用拒绝与夹具环境回归，未放宽权限、摘要或 UTF-8 原字节断言。本地门禁和源码快照在 `validation/cloud-round1-fixes-local-20260926`，Python 原日志在首轮归档的 `local-script-diagnosis/`；修正已提交推送为 `b78b62a48223235e8c29157e3786747c8db2ff68`，同提交 [Actions 36245112104](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36245112104) 已完成 Linux／Windows 复验：两平台 `cargo check -p warp --lib` 通过，Windows 固定 Codex 0.156.1 原生 Hook／ConPTY 通过；Linux 主应用定向回归 3,166 通过／4 失败／4,840 筛选或忽略：四项均在同一 Grok 输入夹具初始化失败，测试进程未拥有新建 PTY 的控制终端，真实身份校验拒绝；相应功能断言尚未运行。图片客户端 5、共享 action/skills 75、warp_cli 129、TUI 9、命令归属 1 及 IPC 2 项通过，通知 worker、固定 Grok 生产通知插件安装和监督清理等无认证原生探针亦通过。Linux 作业结论仍为失败；Windows 作业全部通过，主应用定向回归 2,973 通过／0 失败／4,772 筛选或忽略，图片客户端 5、共享 action/skills 75、warp_cli 129、TUI 9、命令归属 2、宿主崩溃监督 5、IPC 2 和 rust-genai 81 项通过；固定 CLI 原生通知、恢复/崩溃边界与 Grok 监督清理探针亦通过。完整日志及 10 份原始产物 ZIP 的索引为 `second-run-evidence-index.safe.json`，114 文件，SHA-256 `e71bc6d7c2f4f6f22cc0feb0c8020070351d7c65855295c943d37bc9acfbca05`；不包含已认证模型、GUI/IME或安装来源真实升级。无需本地化变更，不关闭功能缺项。

同一 `b78b62a48` 代码还通过本机 `cargo build -p warp --bin infinishell`，生成 arm64 可执行文件并复制到内置盘后核对摘要 `ca2d6a34cab2e98f810158571fe4d8e5eacb122c382d9131f3067e26eb325997`。源码在构建期间未变，动态依赖列表没有外置卷路径。证据为 `validation/code-checkpoint-app-b78b62a48-20260926`，索引摘要 `c89b0c49199b65a45360e3c5b27777ed4b651f776b13fa7ca0e260f42ea33418`。程序未启动，未制作签名应用包，不算 GUI 或模型验收。

该 arm64 二进制另已组装为内置盘联测 `.app`，资源 131 文件中 bundled 128 文件与当前仓库逐个摘要一致；显式 `codesign --sign -` 临时签名及严格签名核验通过，没有查询开发者证书。签名后程序摘要 `6ff75465683af7d5bf49f765800840e4f4ee526b54c03812b974d70213019327`，原始程序未变；使用独立 `parity-code-b78b62a48` 数据 profile，不改旧配置。归档 `validation/code-checkpoint-bundle-b78b62a48-20260926`，索引摘要 `ea4da1ca09c09161248e86d476976323a4b24264f05d43b3e94de7212896b17c`。应用未启动，此包仅供后续共同联测，不算新 GUI 或模型证据。

Linux 四项失败后，仅修改 `grok_owned_input_tests.rs`：通过统一 command 封装派生持有控制 PTY 的 shell，用真实子进程身份初始化，测试结束或断言退出时 kill/wait；生产校验和四项功能断言不变。本地 `cargo check -p warp`、四项定向测试及 i18n 11 项通过，原 Linux 失败保持；修正已提交推送为 `17c1dcc516a7f45c2ce61243a0bf82487b3afec0`，仅补跑相关 Linux 平台的 [Actions 36248869156](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36248869156) 已全部通过：主应用 3,170 通过／0 失败／4,840 筛选或忽略，四项 Grok PTY 用例逐项 PASS；图片客户端 5、共享 action/skills 75、warp_cli 129、TUI 9、命令归属 1、宿主崩溃监督 5、IPC 2 及 rust-genai 81 项通过，固定 CLI 无认证原生边界亦通过。完整日志与 2 份原始产物 ZIP 位于 `validation/cloud-linux-pty-17c1dcc51-20260926`，60 文件索引 SHA-256 为 `4f6c41baa1889aeaf50e94ff54634f83dd7e793a3e019af07e4b2f4274a80901`。Windows 生产与测试源码未变，复用 b78b62a48 的独立通过结果，不冒充在17c1dcc51重跑。原始收据在 `validation/linux-pty-fixture-fix-local-20260926`，索引摘要 `b5babc7f3476e2fc22c8e3b73d774fc2506103abf25fd0c14ea6019ee93913f6`。无需本地化变更，不扩大任何功能验收结论。

## 验收基线与追溯

| 项目 | 固定来源与解释 |
| --- | --- |
| CLI 版本 | Codex CLI `0.156.1`、Claude Code `2.1.280`、Grok Build `1.0.41` |
| 历史产品行为冻结 | `ee839b4fc7dacd58bac0806aa549500af5776b30`；包含桌面 Grok 默认开放、启动取消修复和双语说明修正 |
| 历史最新相关验证提交 | `50e1515bcd3f20cf120edf5565766975b6a68d5d`；只完善测试与运行器，未再次修改产品行为 |
| Mac 模型与 GUI 实链 | 来自 V2–V13 等实际构建批次及源码摘要；后续按变化域复验，不能重标为 `ee839b4fc` 或 `50e1515bc` 全量重跑 |
| 历史档案固定提交 | `e3ef39689cd8b686dfe040b90217ed1067b4d268`；保存原过程专报、失败、通过、截图与摘要索引 |

原始材料从当前目录移出后，仍可从[固定历史目录][history]追溯，或使用 `git show e3ef39689cd8b686dfe040b90217ed1067b4d268:specs/cli-agent-parity/<历史路径>` 读取。历史总体“全部完成”判断已被当前状态撤回；保留原文件不表示继续认可其总体判断。

关键来源是[固定版本验收表][acceptance]、[Mac 交付记录][mac]和[同提交补验报告][final]。这些固定提交链接用于复核旧结论，不承担当前待办管理。

## 2026-09-26 Claude npm 单包更新增量

以 `fe135e3436c22eaa7857c31bfaf94969eb1aa11f` 为基线的工作区接入 Claude npm 官方完整包树事务，目标限定 `2.1.280`，旧安装验收起点为 `2.1.278`。独立核对 wrapper/platform 归档 SRI、完整树及公共 native 入口，不执行安装脚本；原有忙碌预约、配置约束及监督清理保持。候选使用私有空配置、固定 `--version` 和拒绝网络；新增删除前逐成员校验及保留外部变化的回归。Codex npm、Windows npm、musl、额外 ACL、Homebrew、WinGet 和合法降级仍未完成，G09 不关闭。

macOS arm64 本地 `cargo check -p warp`、i18n 11 项、更新模块 182 项（4 跳过）、监督模块 85 项（17 跳过）、Python runner 16 项及应用构建通过。首两轮测试编译的导入错误已修，原日志保留。工作流 actionlint 在显式登记现有自托管 runner label 后通过。英中版本探测失败提示已在同一签名 GUI 构建实际显示，完整文本、渠道说明、按钮与下拉框可读；合成无效版本只计布局，不能替代更新事务。应用、测试程序、监督程序及缓存均从内置盘运行，两种语言启动均未等待人工授权。

第一轮真实 npm 事务在 `/private/tmp` 私有子目录中构造官方包后，被已有 macOS 原子执行祖先检查拒绝：`managed_process.atomic_macos_ancestor_unsafe`。原始 native exit 1、cleanup_confirmed 收据保留，旧公共程序摘要不变，暂存树已回收。未放宽权限检查。第二轮在用户私有内置目录通过祖先检查，但 15 秒输出等待到期：原子镜像验证发生在监督者就绪之后，空输出和 `stdio_closed` 收据不能证明已进入 CLI 主程序。该轮同样确认清理、旧公共摘要不变和暂存树回收。候选版本探针改用既有更新事务的有界 300 秒预算，并单独返回 `TimedOut`；新源码门禁及后续五个真实场景均已通过，旧失败保留。

本轮 macOS arm64 在私有前缀物化官方包，实际执行 npm 只读来源查询、生产 Rust 更新后端及受监督候选 `--version`：正常更新到 `2.1.280`、交换后缺收据的冷进程恢复到原 `2.1.278`、外部改动及 journal 保留、候选改写后未启动和未审核 `2.1.278` 降级拒绝，五场景均通过。独立 Python 检查逐份核对 SRI、归档成员与最终目录；公共入口链接保留，所有已启动候选正常退出并确认清理。版本拒绝不能外推为合法降级已实现。

最终监督程序 SHA-256 为 `63c34a50b1ce86d3bfdd8774aee3ee9ab070bef348c9d3bc8cda2e9e5c30499b`，测试程序为 `ebc56ba3d43ccb0f8b92838cd777831e4ea8d484b6fcf68aeaebd4eb7f041a96`，零模型输入。忙碌范围仅为另行运行的既有产品状态机测试，未覆盖 GUI 更新点击或插件重检。GUI 文案与资源未随超时修复变化，复用 v3 的独立布局原图，不冒充同一程序全量重跑。349 个原始记录、旧失败、源码与截图保存在仓外 `validation/claude-npm-20260926/`，索引 SHA-256 为 `7f0a70056a1edfafd746a962699d8f2ae70685ce909658bf745c217b7a2ec022`；实现提交为 `529512a354d66fdcf48f70db1bbe63216c36a3da`，独立提交绑定 SHA-256 为 `6d2b4d5c7836803e3d1890998d924edf6af214761145f1ba42475b784ed51be8`。[Actions 36217724224](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36217724224) 已通过同提交 Linux/Windows 定向门禁，Linux 官方 Claude npm 五场景原始收据逐份核对通过，零模型输入。云端 253 份记录归档于仓外 `validation/cloud-g09-529512a35/`，索引 SHA-256 为 `97ca36b14ecbd9eecf2d6313ab4a3fc51a662a26c27953934d0ba2b207a2e991`。本次仅补验收结论，无需本地化变更；G09 的其余安装来源仍未关闭。

## 2026-09-26 文件卡片同提交门禁

文件选择器增量已提交为 `83e5115839b301697f3322092f487ddce9d72dfe` 并推送。本地 `cargo check -p warp`、i18n 11 项、editor 6 项、footer 46 项与应用构建通过；实际 macOS Codex 两张卡片投递、原生 shell 读取及双语布局沿用该增量的原收据。Grok 自动提交、Claude 首次向导后的普通终端链仍待补，G07 不关闭。

[Actions 36210156960](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36210156960) 绑定同一提交，Linux x64 与 Windows x64 定向门禁均通过，Intel 作业未选择。它覆盖相关编译、回归、无认证原生协议/通知/清理；不证明已认证模型生命周期、真实中文输入法或 Grok 完整双语 GUI。47 个原始日志和产物保存在仓外 `validation/cloud-g07-83e511583/`，索引 SHA-256 为 `20373845cd0a8228ae23fca12f57db91aec0e8bb6dc62986d462fa302678a364`。此前 `6790b185a0c0bc581f804edf975960047df85db7` 的 Linux/Windows 整合门禁亦通过，归档索引为 `8239de43dd32999ba0348146d923df701e9d7b97e0b5e079022beff9d2abcdc3`。

## 2026-09-25 继续实现增量

实现来源标记为**输入实现提交 `84102bb1c687f87a2425bc1937784e77250c416c`**。G02 接通固定 Grok PNG ACP 图片；G04/G05 扩展 Claude 静态格式、纯图与精确注册单技能的图片编码；G07 接通普通终端本地文件卡片到明确路径引用。G09 的 Codex/Claude npm 管理器/包清单/前缀/真实入口绑定已提交为 `b6f93f6627738fb83b6be776dd225666a900ac90`，但 npm 更新仍为 `ManualOnly`，尚未完成真实升级、降级及恢复事务。以下均不关闭 Gap。

### 图片生产适配器与原生合同

| CLI、版本、模型、平台与模式 | 本次实际结果 | 证据边界 |
| --- | --- | --- |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、Inherit 生产适配器 | JPEG、WebP、静态 GIF 分别新建识别随机象限色；原生 user 回放的 MIME、字节和数组摘要与持久图片一致；关闭后同原生 ID 冷恢复正确回忆，均自然退出并清理。每用例 2 次模型输入 | WIP 二进制 `6e3e13f07aef46612c90c35b0c67c84163313f00f5e594701746f1f06215d284`；不是 GUI、应用重启或跨平台链 |
| 同上，纯 PNG | 先文字定义后续图片回答格式，再提交没有文字块的纯图片；实际识色、字节回放、持久引用恢复及冷恢复回忆通过，共 3 次模型输入 | 不外推所有纯图片格式；不是图片加技能的生产验收 |
| Grok `1.0.41`、`grok-4.7`、macOS arm64、Inherit、无已选技能的真实生产 connect | 新建识别红/蓝 PNG，关闭后同原生 ID 冷恢复，提交独立绿/黄 PNG 并正确识别。真实最终历史 user_message_chunk 的图片字节、MIME、typed content 摘要与持久输入一致，promptIndex 为 0/1，代次和回合准确关联 | WIP 二进制 `ff821a38c0260c35e9a3210f39090156c6814d2b1327499bb8992669b2b3f8b0`，未用 candidate flag；两代各重发同一消息 ID 共 3 次，仍各仅执行 1 个原生回合。两次退出均 `stdio_closed`、exit 0 且 cleanup_confirmed；不是 GUI/应用重启或其他策略/平台 |
| Claude `2.1.280`、`claude-opus-5-5`、macOS arm64、直接原生 stream-json 图片加单技能 | v5 新建与冷恢复各有精确注册命令的真实 Skill tool_use 及 `SKILL_OK LEFT=RED RIGHT=BLUE` 正例 | 原生合同；保留原生权限流程，审批次数未单独计证，不是完整生产适配器/GUI链。裸 slash 数组未触发技能的失败和 v4 探针“禁止工具”指令冲突导致的失败原字节保留 |

两款生产用例使用同一内置盘监督者，摘要 `a8aae1653910a4625c879caa62f730e5131207eb08bfb68877334f4eb3b1d4b8`。收据明确记录基线 HEAD、工作区脏状态、关键源文件和运行器摘要及运行期间源码不变；**HEAD 只是基线，不能把 WIP 二进制记为该提交已验证**。其后又补严 Grok 已选技能图片门禁，以及仅 Claude 的静态 GIF 首次/恢复校验；上述原始收据没有覆盖这两项后续修正，不回填为修正后通过。最终重建与受影响模块回归已通过（见下节），静态 GIF 后续已用 `84102bb1c` 对应 libtest 完成在线窄复验；20 个构建源文件均与此提交 Git blob 一致，旧 WIP 记录不回填。 Grok Python 运行器后来增加 Linux 临时目录回退，但没有本次 Linux 在线结果；Windows 运行器明确拒绝执行，认证副本 ACL 与在线验收另待实现验证。Rust ignored 测试可跨平台编译不等于 Windows 产品链可用。

### 普通终端、技能与独立图片判断

本次普通终端探针均为 macOS 手动驱动的原生 PTY，产品源码改动未在该探针执行：Codex `0.156.1/gpt-6-luna`、Claude `2.1.280/Opus 5.5`、Grok `1.0.41/grok-4.7`。

- G07：三款分别通过原生 exec/read 工具读取中文与空格路径的独立标记；Claude 第二文件的 No 拒绝未读取。它证明路径引用合同，不证明修改后的文件卡片 GUI 链、拒绝后草稿恢复或 Linux/Windows；Grok 产品仍受 G01 自动发送守卫阻止。
- G01：新空会话超过 66 秒没有 idle_prompt；help 模态、未提交中文草稿及后台 sleep 仍活跃时却可能出现 idle_prompt。审批等待 76.7 秒只有 permission_prompt。该通知不能作为 Enter 安全或编辑器为空的证明，守卫继续保留。
- G03：Ctrl+V 后 bracketed text 粘贴造成同一回合两张图片，负例保留；Ctrl+V 后普通 UTF-8 两行配合 Alt+Enter 得到恰好一段文字和一张图片，并正确识别红蓝。无剪贴板消费 ACK 与可靠输入就绪，尚未接通自动链。
- G06：Claude `2.1.280/Opus 5.5` 的原生 PTY 与 stream-json 取得同轮两技能依次审批/调用、运行中新增技能、reload_plugins 返回准确命令列表及冷恢复读取新技能标记的正例。只限无副作用技能原生合同；产品实现由后续增量处理，注册事务、并发、父权限上限、Grok 多技能及两平台均未计通过。
- V04：Codex `0.156.1` 独立原生 exec 的 `gpt-6-luna` 位置错误保留；同图 `gpt-6-sol` 返回 `YELLOW BLUE RED LIME`，独立目视确认 LIME 对应亮绿且顺序正确。原收据 strict_match=false 不修改，语义判断单列；该正例不是 GUI/产品适配器链，也不解释原 GUI 错误的全部原因。

### 当前门禁与双语布局

集中定向测试曾为 1,600 通过、16 失败、60 跳过；不能称全绿。Grok 旧“所有图片拒绝”断言已修正，内置临时目录复验 atomic 15/15、runtime host 31 通过/1 跳过。2026-09-26 最终输入源码通过 `cargo check -p warp`、`cargo build -p warp --bin infinishell` 与 `cargo test -p warp --lib i18n::tests`（11/11）。复制到内置盘并核对 SHA-256 后，受影响模块通过：managed_input 27、Claude 169、Grok 383、普通终端 footer 45，合计 624；另有 21 个在线用例跳过，不能计通过。该轮覆盖新 GIF 边界和 Grok 已选技能门禁，libtest SHA-256 为 `e2ab62925d6aae589eaf00d6d3d1f3b4feb4056f3b242551b42ec35c5fe0fe44`；构建的 20 个源文件与 `84102bb1c` Git blob 一致，绑定证据见 `input-final-local/commit-binding.safe.json`；Linux／Windows 相关验证运行 [36158594977](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36158594977) Linux 成功，Windows 两项新增附件投递测试失败，整体失败，不计作在线模型验收。Windows 的 `cargo check` 通过；定向测试 2,778 通过／2 失败／4,769 未选或跳过。两失败把原始 Windows 路径与 JSON 转义路径比较；工作区已改为准确数组/字符串断言，本地 footer 45 项复验通过，Windows 同提交复验待补。Linux 主应用定向 2,916 项通过，未选、零模型和旧版 Codex 原生脚本均在各自 scope 中明确区分。

macOS 重启各语言进程后，Claude 图片格式说明与 Grok 顶部新说明的 en/zh-CN 原图均可读，完整换行且相关按钮可见，原图 2560×1600、逻辑窗口 1280×800；只验此视口布局，无模型输入。GUI 使用上述 WIP 二进制（签名后摘要 `8f42725d3ab4c9f4c2ea8d4efb7e71edb29010abf3f0303e3edc3cbbe4d73715`）动态读取当前 Fluent，早于最后两项 Rust 门禁修改。旧 live-switch 混语言截图保留且不计中文完整布局；不计最终 SHA、错误提示/审批全视口、Linux/Windows 或 V05 完成。

### 2026-09-26 后续实现与窄复验

- `84102bb1c`：Claude `2.1.280/claude-opus-5-5` 静态 GIF 新建和冷恢复窄复验通过，原收据因无关未跟踪探针记 dirty=true；另附关键源码与 Git blob 的逐字节绑定，不改原收据。
- 同提交 macOS GUI：JPEG 原始 MIME／字节和四象限回答正确；第二轮 READY 后应用退出再启动，重新关联原进程，消息仍为两条。断开确认原 PID 消失后，历史继续使用新进程和同原生 ID；此时尚未新增消息，随后显式发送纯 PNG，原生内容仅一个 image 块、无 text，独立象限回答正确，持久消息变为三条。区分重关联、冷恢复、发送，不把恢复按钮当作新输入投递。范围为 Inherit 根任务，非父权限证明或三平台完整生命周期。
- 同提交 Grok GUI 选择 PNG 被旧通用选择器拒绝的负例已保留；托管运行时已支持图片，但入口遗漏。当前工作区已修正入口，仍要求固定版本、继承模式、无已选技能和原生模型再次核验，后续修正后的双图 GUI 与恢复正例见下文，旧拒绝不改记通过。
- G06 实现来源 `3b4d8e8a3`：空插件槽、无图多技能、严格 reload_plugins 确认、事务回滚、旧回调隔离及接收后持久化已接入。该来源的 macOS 生产适配器三轮在线通过（多技能→新增→冷恢复，6 次真实 Skill 单次审批），两代正常退出。原模块首轮 1,022 通过／3 失败／52 跳过，单线程复核其中两项通过，一项监督控制通道 os35 仍失败；不称全模块全绿。后续整合门禁已通过；Claude Mac GUI 双技能及同会话热新增实际 native Skill 逐次允许通过，后续热技能恢复修复与 GUI 复验见下文，跨平台仍待补。
- G10 实现来源 `9f5967f62`：独立 V2 文件创建策略保持父权限版本匹配，逐次审批、路径范围和允许时再次校验；固定 `2.1.280` 的原生 Write 允许／拒绝／越界／符号链接探针已通过。V2 父子与 V1 禁升级的生产协调器验收夹具来自 `de135f92c`，后续两条生产协调器链已通过，范围见下文；原生探针与真实父子链分别保留来源。

- 整合工作区内置盘首轮：i18n 11 项、运行时 1,038 项通过／53 跳过，`cargo check -p warp` 与应用构建通过；之前 os35 的恢复场景本轮单线程通过，原失败不改写。外置盘断连导致的编译 SIGBUS/os5 和主动中止重试仍保留；经用户授权改用内置缓存，并仅清理已核对无占用的 3.18 GiB 旧运行程序。后续修正须另补门禁。
- G05 整合工作区生产链：固定 Claude `2.1.280/claude-opus-5-5`、macOS arm64、Inherit 根任务的图片＋单技能新建与同会话冷恢复通过。两次精确 Skill `AllowOnce`、两组独立象限颜色、原生图片字节/工具身份、两代 `stdio_closed/exit 0/cleanup_confirmed` 对应完整；libtest `ec718473…`、监督者 `48018a00…`。不外推多技能图片、父权限上限、GUI、应用重启或跨平台。
- G10 首轮 V2／V1-ceiling 生产父子链均失败：G06 新逻辑把正常邮箱 `MessageAccepted` 当作 self `user_input`，返回错误的启动不匹配。子进程未取得原测试完整清理收据，之后按精确身份补救清理另记；已修正为只有自身用户输入登记技能，其他邮箱保留原回执校验；后续还修正已持久化邮箱 joined 重放的同类误判，两条新构建生产链已独立通过，见下文；原失败和当时缺失的子清理收据不回填。补救前仅确认 launchd 项已登记但 not running，不能推断原生进程当时仍存活或由补救导致退出。
- Grok 整合 GUI 已能选择 PNG 并保留图片卡片，但新建时出现 `post-response setup phase arrived before session/new response`，没有进入模型或确认原生会话，缺项仍开放。原始 journal、未确认输入和清理收据保留；不能把卡片显示算作图片投递成功。原生正负对照已定位为缺省配置走 direct 模式：仅有 `--leader-socket` 不强制 leader。工作区对固定 1.0.41 根任务显式传 `--leader`，SDK／固定策略仍为 `--no-leader`，协议阶段检查不变；后续新构建双图 GUI 已通过，见下文；原始握手失败不改记通过。

### 2026-09-26 整合修正后的限定验收

以下仍为 `84102bb1c` 基线上的工作区摘要绑定，**不是最终提交或三平台完整验收**。内置盘 `fixed` 轮来源摘要为 `b8c832473e548e8213eb83cd29422f6ecebf8879e8ed1718677dda1073e92632`，i18n 11 项、runtime 1,044 项通过／53 跳过、check 与应用 build 通过。随后邮箱 joined 重放修正的 `replay-fixed` 轮来源摘要为 `d5a8790a63fdcf7c267dd8bf99c525f59781f9ca85102226d550c066478177bc`，coordinator 91 项通过／4 跳过，i18n 11 项、check 与应用 build 通过；未无变化重跑整套 runtime。该轮 libtest SHA-256 为 `c7cf03f14893dd82eceb80cdf39fabf1cfe2f2e155076f27f2d86f2922a2f595`，监督者为 `8484a0c900e2c596f95d3d2ff4c473e1814f18713540c8fef8e0191e8e405e95`。跳过项、Windows 原失败与外置盘构建失败保持原结论；同提交相关平台验证待补。

恢复清单、身份隔离及 ACK 顺序修正后的源码快照为 `05e7fcbb8d4e8ef98e924ccf1b17426564239b3b2313639ab34a4fc100306461`（36 个构建源文件）。i18n 11、coordinator 101 通过／4 跳过、`cargo check -p warp`、应用 build 全部通过；此前未再修改的 task-manager 46 项通过。新增真实 SQLite 故障测试验证 ACK 失败不改技能、ACK 成功但清单失败后重放零发送、错误会话不提前 ACK。libtest `7a3955070f01470a00cb2fb970ed2b0757c14325384ac82b6d94b84c3021e05d`，监督者 `db931fe468a13b7a7d30d4850ddee763606f7527b15d24e87af24bd8b6c1f884`。较早恢复构建被主动中止以整合隔离补丁的记录保留，不记作编译失败或通过；GUI 和在线结果仍使用下列各自原二进制来源。

- **G02／V04 Grok GUI**：macOS arm64、`1.0.41/grok-4.7`、Inherit 无技能，两张独立 PNG 的原始字节和实际象限色回答分别通过。首次断开并退出应用后，历史继续使用新宿主/原生进程和同原生 ID，发送新输入前消息仍为 1 条；第二轮完成后退出应用，宿主仍存活，重启精确关联同宿主、同原生进程和代次 2，消息仍为 2 条，没有重投。两代均 `stdio_closed/exit 0/cleanup_confirmed`。两轮分别绑定对应源快照与签名后应用摘要，不能把第一轮改记为后一轮构建。英文／简体中文权限、附件、技能限制、消息及历史结果完整滚动区无截断遮挡；不计 Linux/Windows、真实 IME 或未执行的审批交互。
- **G05／G06 Claude GUI**：固定 `2.1.280/claude-opus-5-5`、macOS arm64、Inherit 根任务，三轮真实 GUI（双技能、热新增、PNG加单技能）共5次精确Skill AllowOnce，原生历史模型为claude-opus-5-5且图片字节摘要匹配；恢复修复前错误及原始记录保留。修复构建重关联同宿主/原生进程、同会话和代次3，仍3条输入，原生历史字节不变；正常断开exit0并确认清理。英文和简体中文相关说明、技能列表、消息与历史结果可读。模型轮次与重关联程序分别绑定各自源码/二进制，不宣称新构建重跑模型。 旧宿主签名后摘要 `b014a98f…`，恢复应用为 `8dd332a3…`（源码快照 `e01dee11d49901d81b1a427916aef5d87e78709546847a2de30f140615694f76`，未签名程序 `b4f56cb5…`）；持久 OwnerClaimed epoch 1→2，原生会话 `33ae1e46-47b6-4865-bf8d-008d6630e0b9` 不变。恢复后通过显式“刷新”加载 alpha/beta/gamma 目录。原生历史摘要 `c6668675b2002c0e53d61c457643d6cbda70375497da4dbf9223551a6fd013af`；原 PNG 与原生图片块同为 `ea9dc58742e14577738c00e60ab53214bf9863b7a0ee6b57545426d79d742c9f`。不证明父权限上限、图片加多个技能或跨平台。
- **G07 GUI 入口复核**：macOS arm64、Codex `0.156.1`、应用摘要 `8dd332a3…`，真实“附加文件”可以选择文本文件，但只把路径插入正文，没有生成 PendingFile 卡片。旧点击未生效时 Open 禁用的截图保留；随后键盘选中并点击 Open 成功，撤回“文本文件不可选”的误判。两会话均零模型输入，清空草稿后正常退出；不计文件读取、投递或 G07 完成。
- **G06 Grok 原生合同**：默认 profile、显式 `agent --leader`、固定 `1.0.41/grok-4.7`、macOS arm64，2 次模型输入证明同轮首技能原生展开、后技能通过 read_file 读取完整正文，以及同会话新增技能须显式 reload 后才注册/展开。另一次零模型双目录探针证明 reload 会刷新同 leader 两会话，即使参数仅写一个 sessionId；不能当作局部刷新。两轮 client 自然 exit 0，按精确归属清理各自 leader。零审批请求，不计 InfiniShell 多技能/热新增产品接通、严格串行多技能执行、父权限上限或 GUI。未信任第二目录的早期作用域观察和离线校验器失败均保留。
- **G10 Claude V2／V1-ceiling 生产链**：固定 `2.1.280/claude-opus-5-5`、macOS arm64，使用上述最终内置 libtest/监督者。V2 精确 Write 允许生效、拒绝后文件不变；实际 5 次允许／1 次拒绝。另一条真实 V1 父记录请求 V2，因 `claude_profile_parent_mismatch` 在建立原生连接和发送输入前拒绝，父记录不变；该链保留 V1 Edit 效果。两链分别 4 次 MCP 工具、5 次输入、3 次原生执行、2 次 joined，双向消息和自动结果 ACK 完整；四代共 12 张原始清理收据齐全，host/native PID 全部消失，launchd 项均注销，无补救清理。
- **G10 运行器修正范围**：V2 Rust 测试及完整私有树已通过，但首版公开投影误把 inspect 明确标记截断的嵌套 JSON 当完整 JSON 解析，wrapper 失败。修正后只对布尔 `body_truncated/evidence_truncated=true` 的有界副本保留原字节摘要，未标记的坏 JSON 和损坏完整证据仍拒绝。原失败 metadata/原始树保持不变，同一次 V2 原生执行重新投影通过，没有新增模型执行；两个 Python 脚本 before/after 摘要另记，Rust blob 未变，离线测试 25+70 项及 py_compile 通过。待审批 Write 取消、活跃宿主重关联、冷恢复、GUI、Linux/Windows 及命令/技能扩展不在本轮范围，G10 仍开放。

### 6790b185 提交绑定与文件选择器后续增量

技能、V2 文件策略和恢复修正已提交并推送为 `6790b185a0c0bc581f804edf975960047df85db7`。最终快照 `05e7fcbb…` 的 36 个构建源文件与此提交 Git blob 一致；绑定收据摘要 `613cb19968667392a08829f4f831030af6da4a2fca7d9fa58ef538e4e6b80060`。相关平台运行 [36176479030](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36176479030) Linux x64／Windows x64 两项作业均成功，Intel 作业跳过。该定向工作流不包含已认证在线模型、物理 IME 或 Grok 专属双语完整布局，不将相关门禁外推为完整验收。

随后 G07 四文件增量接通展开富输入的文件选择器与 PendingFile 卡片，保留 CLI 锁定输入模式、取消和旧会话回调边界。源码快照摘要 `e02d7239b30e43019a209688348eb23bf7b76246b0798267d291d00ebb73477e`，基线为 `6790b185`、工作区 dirty；内置盘本地 i18n 11、editor 6、footer 46、check 和应用构建通过。libtest `a2de24af1e607ae5b6a479fb100cc61521a27cd4e0e85fa8480f675b414670e8`，签名后应用 `d383765f2e24f0f49747e25a1ec3f8d69a9f2237bd9b0586c8a986eeb17d2688`。无需本地化变更，既有“附加文件”语义适用。

- macOS arm64、Codex `0.156.1/gpt-6-luna`、普通终端 read-only/on-request：真实选择器添加两卡片，中文/空格路径以一个 JSON 数组随正文一次到达原生请求。中文首轮模型转向 TextEdit，拒绝后未读取内容，原失败保留。英文独立正例仅关闭官方 computer_use/browser_use 工具功能，保留原安全策略；模型调用原生 shell，工具输出及最终回答均匹配两份文件的独立标记。会话分别为 `01a0d9f8-a040-7950-b03e-82202114b1ec`、`01a0d9fd-cb30-7bf3-8a07-b4d1aef1b6b7`。两语言卡片和正文布局可读，不计物理 IME、允许/拒绝完整生命周期或跨平台。
- Grok `1.0.41/grok-4.7`、普通终端 default：实际创建两张文件卡片，Enter 后未发送提示完整且卡片/正文不丢；零模型回合。该结果仅证明选择器和保留行为，G01 未接通前不算 G07 投递成功。
- Claude `2.1.280` 认证状态仍 loggedIn=true，但普通 TUI 首次启动向导未完成；本轮未发送模型输入。未改全局 Claude 配置跳过向导，托管模式已有证据范围不变。
- 本轮所有执行程序和缓存均在内置盘；应用 PID 55371 及受控进程树未发现打开的外置卷路径。旧实例误选截图、一次命令输入丢下划线导致的零模型启动错误分别保留操作边界，不能算新构建缺陷或有效验收。

### 仓外证据入口

本机持久归档根为 `/Users/zhishi/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/`。以下为根下相对路径；原件逐字节保留，认证工作目录未归档，跨机器不能仅凭本机路径认定已验。

| 证据 | SHA-256 与范围 |
| --- | --- |
| `g07-picker-gui-20260926/index.safe.json` | `1432fbbf90e6b9b0e82a2cffb6826e6082f112c705c74d07eb872e6ca6d59162`；43 文件，GUI 原图、Codex 原始失败/成功会话、内置执行路径与本地门禁 |
| `input-final-local/commit-binding.safe.json` | `84102bb1c` 的 20 个构建源文件与 Git blob 一致；本地门禁原件与运行程序摘要同目录保存 |
| `claude-gif-84102bb1c/receipt.json` | 收据 `8de6ba3946bfb5b4108516898990d49670e55f7712400b3ec46a69283538f5e6`；同目录源码绑定 `88935b4822d11f13a5c0579be07390ef0fda7751e82f83fd9fc64625f95df5d7` |
| `gui-input-84102/index.safe.json` | `53054d274bee66768574f1a556d1c6cd4b3270992e740b57f968bed2cb8a8ef5`；27 个文件，含 JPEG／纯 PNG 原生行、SQLite 投影、英文 GUI 和独立恢复证据；scope `1b7f054843239e966dcc9c6da9a58c93d6e1d4b55432583ee6525856882a2b88` |
| `claude-skills-adapter-3b4d8e8a3/index.safe.json` | `fb448a1a509ec50871993eda2c65c74dd5ff2c5e5c72fd839de244270379b0ba`；12 个原始文件，三轮真实适配器及模块失败/复核日志；scope `6f016b661f7ff58eedabb154b36fcd6abd2098e64cbe68f56a9f979ee818b8ed` |
| `claude-image-skill-integrated-wip-v1/index.safe.json` | `24a2b2367eb5125b05842e1793a11eda54129a8854b6c279492487660f4c7b85`；G05 20 文件，收据 `a1a015dec9718047f26cea2d4cffa0819e3a752c4c43527a548ff18b27a8feb1` |
| `cloud-input-84102/index.safe.json` | `abe10089eb66d5518a7f49bf572e669bd4e4aa1665519bb913c9df70e7b6f282`；Linux 23 文件；采集时 Windows 尚在运行，旧索引不回填 |
| `cloud-input-84102/windows-job-108149210250/index.safe.json` | `181418248d53df176440faabc6a56c74341ee53511451ef58e007ea12e830384`；Windows 46 文件、8 个 Actions 产物摘要一致，整体失败 |
| `internal-integrated-gates-20260926/index.safe.json` | `785f001b60cbb04bbf243261b6981c436e3be6f91809d19d3f7654e1f733933a`；29 文件，fixed 与 replay-fixed 的各自源码、构建和门禁；不是最终提交/云端验收 |
| `g07-picker-negative-20260926/index.safe.json` | `f41dad825484fc447a7fe03a534808011341eaa97b1143d591da4e2475ae5ea1`；原截图/AX保留，类型过滤判断已撤回 |
| `g07-picker-recheck-20260926/index.safe.json` | `653e648cd37a62a1fa1b74d19f726d3edde34af6a20788c0e097b70cbc926b7e`；键盘可选的反证及路径正文无卡片的真实入口缺口 |
| `claude-recovery-local-gates-20260926/index.safe.json` | `c446536dc2eb113f75941666bc562963619aabc48ec975799ba4064e0a9baa0a`；36文件，三轮各自源码/门禁、主动中止、最终ACK故障回归及程序摘要 |
| `gui-claude-hot-skills-20260926/index.safe.json` | `a13e13a571adec83aae6361398313dda3e7cf47342301c091f1840d70079d737`；70文件，三轮技能/图片原生历史、重启原失败、修复后零重投重关联、正常清理及英中原图 |
| `gui-grok-two-images-20260926/index.safe.json` | `64726f010ae77e59c480df88efc704fb5597b984832807c94c5bc755135bd735`；47 文件，双图、两个独立恢复模式、原生字节、两代清理与 macOS 双语滚动区 |
| `grok-g06-leader-default-20260926/index.safe.json` | `c9f7a13d0ca0c63fe4b22d4f78254c390b2dbcce494641b82b11390871bc6413`；368 文件，默认 leader 多技能原生调用及跨会话 reload 作用域；不计产品接通 |
| `g10-coordinator-post-fix/index.safe.json` | `8d30cf54cc0fa3d6c39a0037a020787229364200950aea3d28dd7408f718b42c`；41 文件，V2／V1 父上限两条生产链、投影原失败及零新增模型重验、四代完整清理 |
| `g10-coordinator-pre-fix/index.safe.json` | `6e7921157149f50062b7e39c76c36cebf86142149a8a2e54fb2c7b853dbd6a3e`；V2／V1-ceiling 原失败、源码绑定和补救清理分别保留 |
| `gui-grok-pre-fix-20260926/index.safe.json` | `abaf004d0efefb645718add14529c0575696f087ec8d25ea103414ffdff1a1e7`；GUI 图片卡片正例及握手负例、原始未确认输入与已确认清理，零模型输入确认；不计物理 IME |
| `grok-setup-mode-1041-20260926/index.safe.json` | `08a82a439f5e6cf1e85e912f0e07b9d41da7d5eec73e5fea2318ecb48f3d74cc`；8 次零模型原生模式探针，缺省 direct 负例与显式 leader 正例，56 个索引文件 |
| `image-evidence-sha256.json` | `ad4bb6a4da73f41b44c25c08844a259386a63bbcacda304fd1bd02bd0d288308`；55 个图片证据文件的源/目标字节摘要，含两个完整证据目录 |
| `infinishell-claude-rich-images-evidence/production-image-evidence-wip-index.json` | `3ef53b8a10968af6a9549f4986b2a01228f38e88bb1ada109cd239639b90a39f`；四个格式/纯图生产用例；同目录 native-skill-v4/v5 保留失败/成功 |
| `infinishell-grok-managed-image-wip-v1/receipt.json` | `53f51826fc669de485ebc8a0a1c33f803feca9ea2ceaa0737b3caf8267421154`；原生图片投影文件摘要 `d0ba084e5c437f1b9383378dfdf3ee499861b3cfc7b5ed793ed1f0e4e2fcb05a` |
| `terminal-files-native/scope.safe.json` | `25285aecbd815b9cc3389acd6f22025a6b5491c296fd313013183c9ebcfb2987`；原生 G01/G03/G07 的来源和负例；原 index 为 `25a22584b2ec0cabc3f7143e035c866a188e6f6c5cb4aae9dfaa9bfa8e5e7f3f` |
| `claude-skills-native/scope.safe.json` | `10d7e33219112755c0c72a076ef5941ed48cd1daeb0aa8419686f560be45acf0`；原生 G06；原 index 为 `271c28ffb75cd75c46f09149ad5d4b57cda3eff4648ad62633e6bca8a1162f19` |
| `codex-image-luna-native.archive-index.json`、`codex-image-sol-native.archive-index.json` | 分别 `2c8c10286e4d581f7cbb390520481442ba40c90e143ef929820f02047d5e5259`、`47f7905eadcc9ce7e46c23b991597aa004cf1a4f88bcc88f993d753a88005e56`；原错误及独立语义正例 |
| `gui-layout-wip.archive-index.json` | `feea309ee4d7e61054d6987f17c4838fd8280af0b30dda0de6e8250dfe4bc90f`；四张通过原图、旧混语言负例及 layout-review.safe.json 的 WIP/资源来源边界 |

## 历史三平台相关门禁

`ee839b4fc` 的 macOS arm64 本地门禁与 [Linux／Windows 集中验证 36103244022](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36103244022)记录如下。该次 Windows 作业整体失败，不能由其他通过项覆盖。

| 范围 | macOS arm64 | Linux x64 | Windows x64 |
| --- | --- | --- | --- |
| 桌面工作区测试 | 10,895 通过、96 跳过 | 10,923 通过、100 跳过 | 10,680 通过、112 跳过 |
| `cargo check -p warp`、i18n、默认入口 | 通过；i18n 11 项 | 通过；i18n 11 项 | 通过；i18n 11 项 |
| 实际双语 GUI 目视 | 6 张有效原图通过 | 4 张原图通过 | 4 张原图通过 |
| 原生 CLI 原子升级 | 复用各自固定构建的三款实链 | 12 场景通过 | 三款正式升级通过 |
| 直接 Grok ACP 五秒 EOF | 不与托管清理混计 | 按原探针来源保留 | 5,005 ms 时未退出，之后自然退出 0；五秒观测仍失败 |

桌面工作区集合排除 `command-signatures-v2`、`integration`、`warp_tui`；GUI 与受影响 TUI 单列，不等于所有仓库测试。macOS 的 TUI 消息状态 9 项通过。Windows 4 项 leaky 属于通过子集，不证明后代进程已退出；macOS 3 项、Windows 1 项 slow 按原记录保留，跳过项不计通过。

双语 GUI 只覆盖原收据列明的视口。Linux／Windows 使用 Codex 页面与系统剪贴板，不证明 Grok 专属权限界面或物理中文输入法；Windows GUI 使用实际 Mesa GL/llvmpipe 窗口，不外推为 DXGI 全功能验收。

## 真实 Grok 进程清理补验

`50e1515bc` 的 macOS 提交后实测和 [Linux／Windows 集中补验 36116931025](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36116931025)均通过。该次只补相关原生模式；工作区、GUI、原子升级沿用上节的原来源，没有再次执行。

四场景分别在私有 leader／无 leader 两种拓扑执行显式结束与标准输入关闭后结束，经过生产 `ManagedChild`，核对真实进程身份、代次、stdout EOF、旧代次拒绝及已绑定进程退出。

| 平台 | 已通过结果 | 不能据此推定 |
| --- | --- | --- |
| macOS arm64 | 提交后四场景通过，源码干净；之后同提交 `cargo check` 也通过 | 二进制是核对摘要后复用的内置盘副本，不是四场景前重新编译 |
| Linux x64 | 定向 847 项、原监督 5 项、真实 Grok 四场景通过；取得 `linux_subtree` 清理回执 | 无 leader 场景退出码为空，不能称全部自然退出 0 |
| Windows x64 | 定向 747 项、原监督 5 项、真实 Grok 四场景通过；真实进程句柄与严格 Job 清理 | 两个无 leader 场景为退出 1，只证明清理，不是模型任务成功 |

此补验没有认证或模型输入，不替代审批、用户 Stop、历史继续与应用重启的完整在线链。独立直接 ACP 的自然 EOF 与 leader 强制清理也分别计证，不能冒充生产监督器证明。macOS Intel 备用作业未执行。

## 历史固定版本产品链已验范围

以下主要来自 Mac 的已认证原生执行及真实 GUI，均保留原构建与模型来源。Grok 模型为 `grok-4.7`，Claude 父子链为 `claude-sonnet-4-6`；不同模式不能相互替代。

| 能力 | 已验结论 | 范围限制 |
| --- | --- | --- |
| Codex 托管父子任务 | V9 双向原生 ACK、父权限上限、自动结果 ACK、显式继续后持久结果回收通过 | 不是完成态父任务自动唤醒证明，也没有该用例的子文件效果或 GUI 证明 |
| Claude 托管父子任务 | 审批、工具、子文件效果、双向消息 ACK、inspect、结果 ACK 与清理通过 | 仅相应固定权限策略，不代表任意工具或原生 shell 可用 |
| Grok 托管根与子任务 | 根任务两轮、允许／拒绝、排队、取消和同会话恢复通过；V8 父子 ACK、结果回收及跨宿主冷恢复通过 | 固定读取／文件策略与继承设置模式分别验收，不能混称操作系统沙箱 |
| 三款 GUI 恢复 | 存活宿主重新关联、同原生会话冷恢复及结果回收有实链；重启不重投旧输入 | 源码代次与宿主身份必须核验；不等于所有平台均完成该链 |
| Grok 多轮重启 | V10 两轮后应用重启，输入数未变，第三轮正确回忆；V11 空闲目录刷新不新增回合 | 目录刷新不等于任意新技能均能在运行中使用 |
| Codex 富输入 | 附件类型化通道、技能、评审意见及文件引用实际进入原生请求 | 图片颜色回答错误，不能计图片理解成功 |
| Claude 富输入 | PNG 原始字节摘要一致，图片回答正例、单技能执行及重启恢复通过 | 当时仅 PNG；本次静态格式/纯图的新增范围见上节，组合与完整验收仍有缺项 |
| macOS 中文输入 | 物理拼音、marked text、数字／空格选词、中英混输及多行保留通过 | 此 IME 用例未提交模型，系统候选浮窗未单独截图 |

普通 PTY 生命周期补验共 16 次输入，三款允许／拒绝、继续及应用重启后同会话恢复有证据。Claude／Grok 取消使测试进程退出；Codex Escape 仅中断对话，测试 shell 自然结束，因此保留 Unknown 与显式关闭终端，不能称工具取消清理通过。

插件与原生通知已有三款实链。Grok `0.1.4` 的通知桥接由产品安装器管理，保留配置禁用、冲突与去重门禁；Windows 安装、`0.1.3 → 0.1.4` 更新、同版本修复、禁用保全、失败回滚和再修复在 [36028161259](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36028161259)通过。插件安装测试为零模型请求，模块导出不证明普通 TTY／GUI 通知全链。

## SSH、tmux 与升级

三款固定版本已在 Mac 本地 PTY、Mac 到同机隔离 OpenSSH、tmux 透传开启且断连重接场景取得真实 hook 与产品会话配对。不是远端 Linux／Windows／WSL 验收；Grok 该批模型输入为 ASCII，不计中文输入通过。

关闭 tmux 透传时，Claude／Grok 已观察到内层 SessionStart 与外层零接收；Codex 补输入后外层零接收，但内层原生发送未独立观察，负例不完整。Codex 首轮 tmux 的 Return 换行疑点未独立确认原因，不宣称已修复。

三款官方原生安装的固定升级为 Codex `0.155.1 → 0.156.1`、Claude `2.1.278 → 2.1.280`、Grok `1.0.40 → 1.0.41`。Mac 正式事务及 Linux／Windows 后续原生事务按各自提交通过，包含目标版本／摘要、配置保全与清理；包管理器来源的缺项以 G09 为准。

Claude Mac `2.1.280 → 2.1.267` 的 Latest → Stable 真实渠道降级通过；其他渠道切换只保留各自历史版本和源码域结论。更新偏好在 GUI 重启后恢复，不能由偏好持久化推定升级事务成功。忙碌延期、启动预约、回滚及恢复有对应回归，不等于正式升级样本曾在运行中的模型会话内执行。

## 失败、修正与未验结论

| 原始问题 | 后续结论 |
| --- | --- |
| `9af6de393` Linux Grok 中断升级退出超时 | `ee839b4fc` 修复启动取消窗口并取得 12 场景独立通过；原超时未被追改，也未声称唯一因果已证明 |
| `ee839b4fc` Windows 直接 ACP 五秒 EOF 失败 | 迟到自然退出单列；随后新增真实监督清理门禁通过，仍不承诺原生五秒 EOF |
| `14e0d7d42` Linux 身份绑定／Windows 输入路径检查失败 | `50e1515bc` 修正线程 TID、路径与实际对象身份检查后通过；原失败不能计作已执行的模型场景 |
| Mac 外置盘 worker 启动受阻 | 栈采样显示未进入 Rust main；用户说明启动需要授权，之后核对摘要并从内置盘启动，不放宽超时或权限 |
| 早期 Windows 编译、更新和 Grok 迁移失败；Linux 中文方框 | 分别修正后取得窄范围原生补验及 GUI 正例；旧失败与其真实来源仍在固定历史提交 |
| Grok 技能旧 runner 为 false、早期冷恢复失败 | 技能原生两轮由独立离线复核确认；冷恢复另有 V8 实链。不能把原 runner 或旧构建改记通过 |

完整在线生命周期的 Linux／Windows 覆盖、两平台物理 IME、远端 SSH/tmux 组合及 Grok 专属双语布局仍未完整满足。原生图片、技能、普通终端富输入、远程附件、包管理器升级和子任务工具范围的当前缺项统一见 [KNOWN_GAPS](KNOWN_GAPS.md)，本报告不重复维护其关闭状态。

本报告不计为合并、发布或 Goal 完成。后续结论更新应同时写明提交、CLI 版本、平台、模式和实际通过／失败／未验范围；新增原始日志与截图置于仓库外，只将必要结论与可追溯引用留在专题目录。

[history]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity
[acceptance]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/ACCEPTANCE_FIXED_VERSIONS_20260924.md
[mac]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/MAC_FIXED_VERSION_DELIVERY_20260924.md
[final]: https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/FINAL_SAME_COMMIT_VERIFICATION_20260925.md

## 2026-09-26 Grok 多技能与会话内新增增量

本增量基于 `83e5115839b301697f3322092f487ddce9d72dfe`，仅开放 Grok `1.0.41/grok-4.7`、Inherit、无 profile/model 覆盖／本地父子工具的私有独占 leader，最多 32 项本地技能。首技能原生展开，其他技能由原生工具读取；不保证工具严格串行。热新增等待准确 reload ACK、同会话目录及路径／内容核验，原生接收与技能清单落盘分别持久化。

macOS arm64 生产适配器 v3 三轮通过；真实 GUI v3 首轮双技能、v4 同会话新增 gamma、两次应用重启重新关联活跃任务、停止旧进程后新进程继续同原生会话并执行 gamma/beta 均通过。三轮原生输入、技能正文或 read_file 原字节、模型最终标记、NativeProtocol ACK 与产品结果逐项核对；零自动重投，两代退出码 0、资源组和后台任务清理已确认。v4 原始构建 SHA-256 `29278201780c87a6869efc34729d16e40d71025ea2995bdd0093568df6e3c599`，内置盘签名运行文件 `9ff3a223d5d45610f9ade22e4836c4c2576708aecfcb7d783e3c7c17827fb939`。两次重启无权限弹窗；英文与简体中文长说明、权限、技能卡、消息及结果完整滚动区检查可读。

本地通过：i18n 11、Grok 454（15 个在线专项默认忽略，三轮专项另有真实运行）、协调器 88、本地技能 12、宿主 31（1 个在线专项忽略）、任务面板 59，`cargo check -p warp` 与应用构建。v4 仅修正界面将已完成输入关联误判为忙碌；未变协议／持久化测试按源码摘要复用。v3/v4 的源码、二进制和各自运行范围独立绑定，不冒称单一二进制全量重跑。

保留旧握手阶段顺序失败、未信任项目技能目录失败、热新增入口隐藏、输入工具丢中文的未发送草稿及第三轮尚在运行的中间快照；最终成功不覆盖这些记录。原始日志、截图、负例、审计和源码索引共 284 文件位于仓外 `resume-20260925/validation/grok-g06-20260926/`，`index.safe.json` SHA-256 为 `3606b77b9c1e716e1d85f4a1705213dea5170c8b4ca694ea029635482acb3710`；对应实现已提交为 `fe135e3436c22eaa7857c31bfaf94969eb1aa11f`，独立提交绑定 SHA-256 为 `3419077aa75a1af26e05c9f5114c43cf8190ac179df1ae11ff27b7937dc2e9d2`。[Actions 36213944594](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36213944594) 的同提交 Linux/Windows 定向门禁均通过，47 个原始日志及产物归档于 `validation/cloud-g06-fe135e343/`，索引 SHA-256 为 `323bb6af8d3df103331f0069657e2c6c30bafeee60e7b54e0a85ee4f4403fcfb`。两平台已认证模型与 GUI、用户级来源和固定权限模式仍未完成；G06 及原 Goal 保持未完成。

## G01 owned 普通终端基础增量（2026-09-26）

固定 macOS arm64 Grok `1.0.41/grok-4.7/default`：应用隐藏入口在真实 shell PTY 核对设备、父进程和前台进程组后 exec；生产 leader 侧车和 SQLite 一次领取完成两轮中文输入，每轮均有独立原生 prompt ID、精确最终 ACK 和 hook 模型标记。再次提交同一消息被持久层拒绝，恢复旧启动记录不能再次派发。PTY 身份直接从现有 master 文件描述符查询，不改变启动服务的二进制消息格式。插件 `0.1.5` 增加原生权限观察；通知本身不认证进程、不承担审批。

原始证据归档 `~/Documents/InfiniShell-Archives/cli-agent-parity/resume-20260925/validation/grok-owned-20260926`，146 文件索引 SHA-256 `d49082ee7e6290bf007f2b316c3db5c96b80967fcc40377e38df7cf407fbec33`。v1 两轮输入通过，但运行器关闭 PTY master 的顺序导致 shell 回收卡住；代理收尾及原失败均保留。仅修改验收运行器，v2 57.04 秒完成两轮和 TUI／leader／shell 回收，私有认证副本已移除。独立检查确认恰好两个原生输入和两个正确标记，SQLite 两条消息均已 ACK。复用相同程序和 libtest 摘要，未改产品源码来放行复验。

本地 v7 `cargo check -p warp`、i18n 11、PTY 内核身份 2、CLI 会话 413（6 项专门环境测试跳过）及应用构建通过；独立运行随后显式执行其中一项原生在线测试。未变更的持久化模块复用 v4 78 项，修复后的运行器 6 项通过。构建源文件摘要已逐文件绑定实现提交 `922308af60cbdd7f84ea69fb3e465ae08b303213`，独立提交绑定 SHA-256 `7e88b8fea31de65fa5d58639f8295083cebb78d67b706349d4b3c17579432ed5`；[Actions 36220234098](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36220234098) Linux 通知/安装验收因脚本仍要求 0.1.4、实际插件为 0.1.5 失败；Windows 安装前 `verify_runtime` 的 3 秒版本检查超时，原因与 Linux 不同。两平台原失败另存 `validation/cloud-g01-922308af6`，102 文件索引 SHA-256 `f2846ce145b2e1214b34f5c7a7688eaa70313ee2a2b21d475a5ce67198bde0f0`。后续整合提交独立复验，旧失败不改写。**不是 GUI、真实 IME、工具审批或跨平台在线生命周期验收。G01 保持开放，尚不能解除普通终端无条件拒绝。**

无需本地化变更：该增量尚未改变消费者入口和现有显示文案；后续 GUI 接线须同步英文／简体中文并分别检查实际布局。

本次恢复增量在上述基础上把侧车事件改为异步接收；真实退出先留清单再释放占用，SQLite 读盘前同步置忙，后台恢复仅关联已有启动占用，并隔离旧回调和重复定时器。普通 TUI 记录禁止进入托管新建、继续、自动重连及未确认宿主统计，避免第二个原生进程或重复模型输入。

macOS arm64 v4 `cargo check -p warp`、i18n 11、PTY 身份 2、CLI 会话 423（6 忽略）、持久化 78、完整协调器 111（4 忽略）、运行器 6 和应用构建通过。恢复专项原生两轮 53.09 秒通过；独立审计核对恰好 2 次 prompt_submit、2 次 stop、独立 prompt ID、正确模型标记及 SQLite 两条 NativeProtocol ACK。TUI、leader、shell 退出和私有认证副本删除确认。构建二进制 SHA-256 `f630b6362389f021b9b1c98578378adfc2e7767e220fc039572707d3ef606e02`；libtest `5f3238e4d6b34777deff6b08384d0b38e2ebb8118f514800b17dccf0cbdd07bf`。

内置盘签名后 GUI SHA-256 `0e5d6f237062bbd99a3b810e33a9db8f4b47a205c47f9d8c2ccf67664ca6e174`，独立空白 profile 英文/简体中文设置与 Grok 版本区域实际可读，未出现人工授权弹窗；两测试 GUI 进程均正常退出。无需本地化变更：内部恢复和通道调整复用既有文案。本次不是 GUI 输入、真实 IME、工具审批或有模型会话的应用重启验收；GUI 新启动登记/派发/握手、启动前期异常回收及平台验收仍欠。

原始证据 159 文件存于仓外 `validation/grok-owned-recovery-20260926/`，索引 SHA-256 `0c1fbc47dec6f0a7cd9c2bc7c8c6a6cc59c73a707dc0c95518096cb4386f8402`，含各轮源码和二进制；提交后追加独立 Git blob 绑定。v1 Timer 返回类型编译失败、v2 门禁通过后主动中止构建、v3 未确认宿主统计遗漏的失败均保留，v4 修复后复验。首张名为 chinese 的图实际仍为英文，重启后的中文正例另存，未覆盖原图。G01 与原 Goal 均未完成。

恢复实现提交 `044263a61b38fa95af32de2b758c1101aae08226` 已逐文件绑定上述 11 个编译源文件，提交绑定 SHA-256 `3491a456e67a69582ee2333d8314a78bdb7dca4e46c5f6a0deef4a0d0f1b6338`。基础提交 Linux 的两个失败门禁已定位为 Python 仍硬编码插件 0.1.4：实际安装步骤退出 0，原始 `passed=false` 收据保留。`4ed5b3d80` 同步严格目标至 0.1.5，旧版/未知版/缺失/混合版本均拒绝；本地验证器 15 项、真实原生 worker 17 项中 15 通过/2 个缺少 shell 场景跳过。稳定步骤键沿用历史名称，真实目标由 `current_plugin_version` 核对。没有重跑 Linux 安装或回填旧失败；整合后同提交跨平台复验。无需本地化变更。

G01 与原分支 G09 整合后再次通过本地 `cargo check -p warp`、i18n 11、PTY 身份 2、会话 423/6 忽略、协调器 111/4 忽略、升级 182/4 忽略、受监督版本探测 7/1 忽略、持久化 78、运行器 6、插件验证器 15 及应用构建。105 文件索引位于仓外 `validation/g01-g09-integration-20260926/`，SHA-256 `67b875a435b5408373166375e16adbe24b65cdeaa485b8f72146491666f18c64`；没有重跑在线模型，原先各轮来源不变。整合提交将从原分支执行同 SHA 跨平台门禁。

G01 持久目录增量：新 v2 清单写入当前任务数据库的数据域，socket 另存短路径私有目录并绑定设备号/inode；旧 v1 清单仍按旧路径合同恢复。v2 本地 check、i18n 11、PTY 身份 2、CLI 会话 433（7 忽略）、运行器 6 和应用构建通过。未修改的共享协调器/升级路径沿用 `32a932f95` 整合门禁；该提交 [Actions 36223768480](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36223768480) Linux x64 与 Windows x64 相关定向门禁均已通过；不含已认证模型或实际 GUI/IME。140 文件原始日志与产物归档 `validation/cloud-g01-g09-32a932f95`，索引 SHA-256 `6e9f25d861b94d6927099a734b92577f1e392e4efb309da9da472cb3b6a4c0cc`。后续未提交 GUI 增量不沿用此通过结论。

固定 macOS arm64 Grok `1.0.41/grok-4.7/default` 的持久目录 v1 两轮原生输入通过，独立审计确认 2 个 prompt ID、2 个正确模型标记和 SQLite 两条 NativeProtocol ACK；TUI/leader/shell 与临时认证副本清理确认。新增退出恢复检查因原生 `leader.lock` 残留报目录非空，原 `passed=false`、panic 和文件字节均保留。v2 仅补精确锁归属清理：要求目录身份未变、leader 真实生存期已结束、私有单链接文件原字节等于绑定 PID；未知文件或别名不删除。新 libtest 在同一真实退出现场恢复成功、socket 目录移除、持久清单保留，旧记录仍不能派发；零新增模型输入。两轮来源分开，不把 v1 整体失败改写为成功，也不宣称同一二进制重跑模型。

v1 应用 SHA-256 `e0bbfaa089f795ff92573a2a3b2735540730d350dceddbe73febc53c2f19efde`、libtest `cd34e4ff95be2d05295235806768dcd8939779c6eb2fba60aa2ab82f0bad3d53`；v2 应用 `9cfa85844f368467a34e98edbaac27834eb56da2024d00d36543f41bb4730222`、libtest `f54d7d283807bc4ae4fc7e5fd9fb5e965cd5477fd1b6eab96e63c55d650f34a9`。79 文件索引位于仓外 `validation/grok-owned-durable-20260926/`，SHA-256 `257d7844508552621e8c10637c3c5149642b4d7ac7a6eb5c03b7aafaebc7f050`。二进制采用 gzip 无损保存，并核对解压后的原始摘要；原始失败及各轮源码保留。

生产改动仅在 macOS arm64 条件模块内；Linux/Windows 不执行本项原生路径，共享基础的同提交矩阵单独记录。无需本地化变更，现有用户界面语义未改变。本项未接通 GUI 提交、新启动注册与真实应用重启模型链，G01 仍开放。

## Claude 图片验收运行器跨平台修正（2026-09-26）

生产验收不再要求 `/private/tmp` 或 Pillow；明确 UTF-8 管道、原始文本收据与仓库工作目录。新增八项离线回归覆盖 Windows 旧编码、中文失败/超时原字节、缺少临时目录及 Git 来源绑定，并加入 Linux/Windows 定向工作流。与关联回归合计 75 项通过，主仓 `cargo check -p warp`、脚本语法、工作流检查通过。工作流初次 actionlint 的既有自托管标签未知告警与登记标签后的通过均保留。

原字节归档 `validation/claude-image-runner-portability-20260926`，索引 SHA-256 `89b652a8f47b67cd6e33e9c739c5b35f3c1a4cd62ea3a47b447dca4f93dc4e0f`。本增量零在线模型输入，无需本地化变更；不补记 Linux/Windows 图片理解、恢复或 GUI 验收，G04/G05 保持原边界。同提交云验待与下一实现增量整合后执行。

## G01 跨平台普通桥实现：目标门禁待执行

本轮从 `41c8d2e307ff5850c73c2b3eecea37c6ed2ee151` 干净分支开始。定制原生 `3d1886b111202215ecfef5d6e41c76b75f0daf36` 的树为 `3796aaf2ffebab4fe06703025ae8d256c221535d`，全补丁 SHA-256 `53953bfd738d279ba0991c0cdc2fc4db18415aaa6b13253a1b0967196de9b04c`。Mac 保持原 `.3` 固定工件；新 `.4` 只为 Linux／Windows 独立构建，不继承旧二进制证据。

本机归档 `resume-20260930/g01-cross-platform`：`r-7a_zxhlb` 的 55 项宿主回归通过，含旧 Mac 账本序列化字节／task ID 与新平台拒绝重投；日志 SHA-256 `d522be8c1969d659ec655ba9d0b667e3abf805fd1bfe64a4616c61ed9f09f570`。此前 `r-8wxq2mx1` 拆分测试漏 `Value` 导入导致编译失败，原记录保留。`r-fhslz7cj` 的原生定向库 74 项通过（46／20／4／4），日志 `c4ffd7aa36fc4cd06e4bf0e81b6e016effc931795daf4212b62dd0e24657e208`；Windows 专有用例不在 Mac 执行范围。

`r-noe0exyo` 的 11 项 i18n 通过，日志 `37afa9723aef72936a08971d477ea773794e91bf432146e66fb0c2f557810272`。复核普通提交、未确认、未发送和同文新一轮的英中提示，无需本地化变更；本轮未新增布局，不重复旧 Mac 模型回合。上述短目录均已按进程、launchd、打开文件和归档证明清理。

实现包括 Linux 原 socket pidfd／实际 exe FD／原 PTY，Windows 实际调试映像 hFile（先确认关闭 kill-on-exit）及从原 HPCON 派生的只读控制台核验。用户 CLI 不纳入辅助进程的终止范围。Windows 传输读完完整帧后发送单字节 `1`，它不是投递 ACK；原子领取和持久 native_acknowledged 合同保持。

Linux／Windows 摘要仍为空并拒绝，不能声明能力可用。待唯一 `cross-platform-preflight.yml` 构建实际定制工件、运行新增身份／管道与真实 ConPTY 回归，再绑定摘要并通过最终源码门禁。G01 仍开放，计数仍为 4 关闭、7 开放、3 移交，PR 保持草稿。

同轮冻结源码的 `r-20_7d2or` 已通过 `cargo check --locked -p warp --features warpui/test-util,rust-embed/debug-embed`，日志 SHA-256 `9abbaf94ea9f9414f342772adde5731f9c7bac6b70263d2a5a6b526f1a3d42da`，短目录已清理。公开基线加完整补丁在独立 Git 索引重建得到相同 `3796aaf2…` 源码树；不将该检查当作目标平台编译。

本轮 Linux 私有短临时目录包装器的 11 项离线合同回归在 Mac `r-sz2rmvsi` 通过并清理，日志 SHA-256 `2849fe6a59e7ffbc9f0283a66b647a2c4ed5e33f71d07246adf146a4c8b799bf`。此结果只覆盖目录／取证逻辑的离线合同，Linux `/proc` 与后代收尾将在既有跨平台 workflow 执行；不计作 Linux 实机验收。普通桥源码开关与仅 GUI／仅安装器模式的冲突组合会明确失败，避免省略宿主门禁后误报通过。

## 2026-10-01：G01 目标构建发现依赖不兼容，精确修复仍待复验

源码 `9fdce3e3e661a0429264c1cb26b2b8eedbf554ff` 已推送，[CI 36741541617](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36741541617) 两平台宿主 check、Windows 实际生产辅助程序的原 ConPTY 正例／跨控制台拒绝步骤及 Linux `.4` 原生构建通过；其余宿主回归仍在运行。Windows 原生在 `cc 1.2.48` 调用 `find-msvc-tools 0.1.13::FILE_ATTRIBUTE_TEMPORARY` 处出现 `i32/u32` 类型不兼容，尚未编译到本轮桥。已下载并逐项核对 10 份命令日志，失败日志 SHA-256 `69df4222e816fdd6d948eac7c5c250ae5359323f17c75e1ffa77b8a33f3f76ae`。

修复 native 提交 `732b0e1ee435f265d6e98a51668afb2e214a1a08`／tree `86f266f0067130525e3dac7b08b78178dad85637`，相对前轮仅 Cargo.lock 的 `find-msvc-tools 0.1.13→0.1.10` 版本／校验和两行，以及 Windows 日志失败测试从 Unix File 注入改为真实生产 Journal 权限变化；保留 Unknown、零派发、同 ID 拒重断言。新 `.5` 补丁 SHA-256 `c4f1a024f9d2f0a2ca5fc8f6be7ea2fc2a48f94f12e9d1bd89f1797ffb3c8b1a`，公开基线加补丁的独立索引已重建为同一 tree。Mac 两次 --locked check 与 74 项回归通过，汇总收据 SHA-256 `b55627f26b7e5eb2dd36ab241f3d3203f891ccd2722aa2bde9d1f9664b26b868`，四轮短目录均已清理。

此修复没有改变用户文案，无需本地化变更；既有 Mac `.3` 功能／双语证据按原工件保留。`.5` 目标构建、真实工件摘要绑定和最终门禁尚待完成，本轮关闭 0 项；累计仍为 4 项关闭、7 项开放、3 项移交，PR 保持草稿。

Linux `.4` 的 15 份命令日志与 607,932,184 字节 ELF 已逐项核对收据；实际 SHA-256 `d36be84fc25eb726e51456e644c18ee98f886e99bb5fda8361d828de59a33615`，原生 74 项回归全部通过。此工件只归属旧 `.4`，不能填入修复后 `.5` 的摘要。主仓修复提交前 `cargo check` 在 `r-4dw1o4g2` 通过并清理，日志 SHA-256 `62364d90a096d8fa7389b051d048f7386962ce91674a2edcc7030a899fd554c6`。


## 2026-10-01：G01 首轮目标源码失败及身份／目录修复

- 本轮未关闭新缺口，仍为 4 项关闭、7 项开放、3 项平台验收移交；PR #22 保持草稿。
- [CI 36741541617](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36741541617) 精确对应 `9fdce3e3e661a0429264c1cb26b2b8eedbf554ff`、原生 `.4`。两平台宿主编译通过；Linux 原生 74 项及独立构建通过，Windows 原生依赖错误按前节保留。确定失败后主动停止后续通用作业：Linux rust-genai、Windows supervisor 构建记 cancelled，不能记整轮通过。
- Linux 宿主 3347 项中 3346 通过、1 失败；command 6 项中 5 通过、1 失败。两项都在 `linux_peer_handle` 取得 `SO_PEERPIDFD` 时收到 errno 92 `ENOPROTOOPT`。保持原 pidfd、映像、退出和 exec 回归通过；桌面／TUI 660 项通过。本轮未采集内核版本，不能仅凭 runner 的发行版名称断言具体内核。
- Windows 宿主 3153 项中 3151 通过、2 失败，配置内各重试两次仍失败：已有私有目录的 Windows 错误以 HRESULT `0x800700B7` 传入，未命中 `AlreadyExists`；只读属性句柄不能执行预期的禁止 DELETE／重命名共享约束。修复同时覆盖宿主及原生目录句柄，保留身份、DACL、reparse 和原失败断言，追加持有时拒绝、释放后真实成功的回归。
- Windows command 36 项独立通过，包括实际映像句柄返回、helper 异常退出不杀用户进程；实际原 ConPTY 辅助程序正例／跨控制台和错误生存期拒绝 1 项通过。它们不是交互式桌面或模型验收。
- Linux 改为原生同一次 `sendmsg` 传递一字节身份前导、自有 pidfd 与 `SCM_CREDENTIALS`。宿主连接前启用 `SO_PASSCRED`，直接接管内核传递的 FD，核发送者、连接凭据、存活及原映像／PTY；不按收到的数字 PID 重开授权句柄。此实现由公开 [Linux 5.15 凭据／FD 检查](https://raw.githubusercontent.com/torvalds/linux/v5.15/net/core/scm.c) 与 [Unix socket 传递语义](https://raw.githubusercontent.com/torvalds/linux/v5.15/net/unix/af_unix.c) 支持，仍须目标门禁证明。契约针对普通非提权进程，不声称抵御具备伪造内核凭据权限的进程。
- 归档：`resume-20260930/g01-cross-platform/ci-36741541617`；原始完整日志 ZIP SHA-256 `923a61ed63941a39b0dd8be06fe21f7db5af7041132a2b25e1cfb448598cf8e2`，失败摘要 SHA-256 `3fbbfaef0236db13cb31c4440c7eeac834e7ba599417ccb0bfc2607b44d9ee6f`。原生工件及各日志按此前收据分别校验。
- 本轮身份及目录修复没有用户界面／文案变化，英中审计结论为“无需本地化变更”；宿主本机编译／55 项定向回归／11 项 i18n、原生 `.6` 编译／74 项共享库回归已通过，五个短目录均归档清理；目标源码门禁、固定工件摘要绑定尚待完成，G01 仍开放。

- 修复后原生提交 `373ab736296cbe4cd9fc60f7a77016f19bba324d`、tree `be01e4d0710cf41d5ad354d9ff31d2fe5800e6e9`；69 文件补丁从公开基线独立重建完全一致，补丁 SHA-256 `6e378a1ebd7214e502d17ae0476a482d6f04e2e35e3a62cab7499e0091248912`。本机五门禁摘要 SHA-256 `1f71332b09761c6ad11f4e8ef3180aefbe4210f6419b04568ef4bd3c9d0d8ec4`；Linux 新测试和 Windows 目录反例仍须目标执行，不以 Mac 编译替代。


## 2026-10-01：G01 Windows protobuf 构建修复，尚未关闭

`9f7962626836cea000c93da43656d47480c18ebf` 的 [CI 36752136629](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36752136629) 已通过两平台宿主 check。Windows `.6` 原生在第 10 步失败：上游 `xai-proto-build` 使用 `/dev/stdout` 写依赖、`/dev/null` 写 descriptor，并据后者解析目标；Windows 不存在这两个 Unix 路径。该轮已经越过先前 `cc`／`find-msvc-tools` 类型错误。原始收据及 10 份日志逐份验 SHA 后归档至 `g01-cross-platform/ci-36752136629/windows-native`；失败日志 SHA-256 `ee50ae82a4c51fb6ad2d8365b861c4931c8aa7a899f76970a43196fe36cb5fe5`，摘要 `d2160647e20df2b07b86d7004f9c1f58ccbfddf066b593948b2431bd42b97ca3`。

原生 `.7` 提交 `d3986df9c7155d7a0f83e82afe3a094bee2b6640` 只修改构建辅助及测试：改用临时实体文件，按 protoc 写入的精确目标前缀解析依赖，保留盘符、内部空格、反斜杠、CRLF 和逐项存在性检查；不改变普通桥运行时合同。71 份变动文件的补丁独立重建 tree 为 `db881bc5b8ccb171deb7421e133d978090ca93de`，补丁 SHA-256 `fa4d55c487c8b106022ef198d45ca2155b4797c7265718e10700b2999bceb130`，重建收据摘要 `95b2872e04ee56d8e86eb48905439e86f595fee3de5abb898f09b519a11669be`。

本机 `r-z0k806ej` 的 6 项真实 protobuf 回归通过，包括含空格目录、跨文件导入成功和删除导入后的明确失败；日志 SHA-256 `a6732643c306ef40f51418638e619171ff638d558f3172d06ffa6dc33e27a58c`。`r-vxzrj508` 的完整 `pager-bin` check 通过，日志 `400779ca56f8eecfe98261729e09054fe4fe4a8137e0de2886d0c7a3c560aeb7`；`r-ombzj5vv` 宿主 check 通过，日志 `57eec8d565eb482f20c5187fc9a286e473c7662a683a0e4f2a25803bcab9f602`。短目录均已按身份及退出证据清理。无需本地化变更；原 `.6` 共享运行时 74 项与宿主 55 项按原工件／源码轮次计证，不虚构 `.7` 重新执行。

Mac `.3` 已验工件另存 `g01-cross-platform/mac-terminal-bridge.3/grok`，复制前后完整 SHA-256 `edcdc3d8729cc657080e6a266e26a6590ec2b275f4545a5b93c1dd5e26bf08f1`、CDHash `356fe77c29f339fcaa90e48094fc7af7e0571ec5` 与代码签名验证一致；归档收据 `791a420f10db76e805bfc1ab90b089dcf4c328fb19ac8383681cd19aa30ba2fe`。README 已说明各平台应使用绑定工件、复制命名 `grok[.exe]`，并从 InfiniShell 普通 shell 直接启动。Linux 构建及其他宿主回归仍在运行，Windows `.7` 实际构建及两平台固定摘要绑定仍待完成；不关闭 G01、不计新的模型或 GUI 验收，PR 保持草稿。

提交前独立 i18n 门禁 `r-_omolrwm` 的 11 项全部通过，日志 SHA-256 `319d29668b22d74a38ac7779563e92db270fa39e4e614e85131aa216bbc561d4`；短目录已清理。

## 2026-10-01：G01 Linux 工件绑定与 Windows 库测试修复，尚未关闭

`.6` 门禁 `36752136629` 的结果已完整归档：Linux 原生 78 项及实际构建通过，宿主 Linux 1469 项／Windows 1308 项、command Linux 14 项／Windows 36 项均通过，Windows 原 ConPTY 辅助验证通过。Linux ELF 为 `e2cb765c093fe6381eecfbb3ba8329e4b4edb76ecb6f98f38f88fc8203c76ffb`，独立副本见 `g01-cross-platform/linux-terminal-bridge.6/grok`，权限 0700；主仓 Linux 普通桥已绑定该实际摘要。结果摘要 SHA-256 `dd304bdf3573ce4e26c05a17d565d39553a2a007e07ced722e7d1eb102506b61`。Linux 原生清理记录因其他 UID 进程的 FD 可见性不足而保留目录，不宣称零残留；两平台余下通用构建被取消，不计整轮通过。

`.7` 的 [Windows 门禁 36756854606](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36756854606) 实际通过 protobuf 6 项和完整原生 check，日志摘要分别为 `8fe225859941f6bc4ed9970cc4fca1935afb3b846fbaf9fd219f41ffc7bcad17`、`444aadc227bfeb0ea3b5def60401360555e05c7190d44371b64e16ad2f151ec9`。随后四库回归在测试编译阶段失败：两个无条件测试调用了仅在 Unix 编译的 `parse_login_env_capture`；没有执行桥测试，也没有 Windows 二进制。已停止剩余第 40 步 `Build InfiniShell SSH worker`。完整日志 ZIP 的 41 项校验通过，摘要 `fba238b802e2107c84a8ab31ca3d9ecc2e9bd8a06f094a16b84af9b04cdd17d3`；归档汇总 `6660a594b6b07576a25840da1ae3c1ccd5273e74e09e6f2d86dcddb39d3ec4ec`。

`.8` 仅将两个纯解析函数向测试编译开放；真实登录 shell 捕获仍仅 Unix。只读审计同时发现资源路径测试直接引用 Unix `symlink`，现按平台选择原生 `symlink_file/symlink_dir`，不跳过 Windows 测试或删减断言。构建脚本显式运行两项解析和三项资源路径测试，再执行完整原生 check、四库桥回归及构建。本机解析 2 项 `r-jhhv50tx` 通过，日志 `dfc4bab9e529215de631548719aed274e8cbb446c71e118b1ed2d7f7118b05cf`；资源 3 项 `r-nrzw1sgl` 通过，日志 `2f90cf1840b7b693a8b33148a684d68d07acc16c61a8ab04607749ba6bea94a8`。首次资源筛选 `r-fsil26_g` 命中零项，作为历史保留、不计有效测试。已完成测试的短目录均清理。

无需本地化变更：上述修复仅涉及原生测试可移植性和已验 Linux 映像绑定，没有用户可见文案或布局变化。Mac 继续使用独立 `.3` 的真实普通 GUI 与双语证据，不把 Linux `.6` 或 Windows候选当作新模型正例。Windows 工件及最终源码门禁仍待完成；本轮新增关闭 0 项，累计 4 关闭／7 开放／3 移交，PR #22 保持草稿。

原生 `.8` 提交 `6230c83eed629901b2a713643208d97f9daadf54`，补丁独立重建 tree `3226eb05d5a899db90215676c1ed8ecaa5c903cf`，72 份变动文件一致；补丁 SHA-256 `560e00b60a499e305fb2cbc308f5225fef63240d5c736cbd09a2874740838bbd`，重建收据 `99b27d9120b3cf32a8b54dbb6000aa8789b624ac9d2c018cc7fa74283270cda2`。完整原生 check `r-sd6a2y5f` 通过，日志 `06360788658b34581e35cc5a7c6c9ec856820ab7e3ff03c60c6a82245aeab1d2`；Linux 摘要绑定后的 Mac 宿主 check `r-kfxlggxy` 通过，日志 `2b84ae77fe1b0127a7eef29dd24292a7470a69c6718d813d3d66ca830ec18d27`，不替代 Linux 编译。短目录均清理。

本次提交前 i18n `r-jesnwmuf` 的 11 项全部通过，日志 SHA-256 `ada32a48e05cdff71ea1eea3bd7ba03f3271ea0cbceeb921e08675157f826bca`；短目录已清理。

## 2026-10-01：G01 Windows 管道测试的类型修复，尚未关闭

[Windows 门禁 36763693789](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36763693789) 绑定宿主 `8f0b956b2dc729374dc852c47de7ca20ccee75fc`、原生 `.8`。宿主 check 通过，原生 protobuf 6 项及登录解析 2 项实际通过，日志 SHA-256 分别为 `cd460ffa6963407c945e2cb6d9a1741655b7bba82ca35665f2966e8a6c4ae4c1`、`030751b3ae9aa1ef687a3c84749230b4c59f2d78f4078b0c763b55b094f49eac`。第 12 步 shell 库编译在 Windows 独有管道边界测试的 `String + &String` 处报 E0277，日志 `882328a7939b2759f46be3eb5dee1c1df85a65ba667d4dcd1812752f87ab9b7d`；资源 3 项尚未运行，native check、桥回归及二进制构建尚未执行。12 份步骤日志和精确源码元数据均复核 SHA。失败摘要 `f5145810fba5cbf60cfdabe0f695cef5edcfd256fdf73f0ed5ea59f83b045ada`，完整归档摘要 `3fcb1aa9fc9cf910a4706c2648bbb7b13e7ce199ecdfa8781130a060da0ab1d7`；已停止第 40 步 SSH worker 通用构建，整轮 cancelled，不计通过。

原生 `.9` 提交 `a5d1e2449179bae2f2c51d05120f212f92fa4d03` 仅修改上述一行测试字符串构造，仍为斜杠加 500 个字母，原管道名称上界断言保留；不修改运行时。现有构建脚本另在 Windows 明确执行该一项，Mac 不跑零命中平台筛选。独立重建 73 份变动文件，tree `3ed485f624e84a0268ab6e4be8e4eeacf938987f`；补丁 SHA-256 `1b626e3fc1ef6e008b1d9993a9fe0056acb04dce4172aceeec8d85f648bd583b`，重建收据 `dd3368743fd05340d233de02f041ed41c2c6d57b06a3ea9eca6bddb6a23d19d8`。本机原生 check `r-917s8602` 通过，日志 `c32d62165890f16189abcf8173518be3f8ba75d6b0dbaefda5066b2d95cdb0b6`；它不编译 Windows 独有测试，不能替代目标门禁。短目录已按记录清理。

无需本地化变更。Windows `.9` 实际构建、工件绑定和最终源码门禁仍待完成；新增关闭 0 项，累计 4 关闭／7 开放／3 移交，PR #22 保持草稿。

提交前本机同一短目录的宿主 check 与 i18n 11 项 `r-9inj6l5n` 均通过，日志 SHA-256 `289f3865ed327a78e1a5e177900e184eb856cf6eff5c79c43c08892fd7f480dc`；退出证据核实后已清理。


## 2026-10-01：G01 Windows 额外符号链接测试移交，原生桥继续必验

[CI 36766846873](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36766846873) 精确对应 `c0a3c705c6dfcf4c1aac42c97639a820c8e7f136`、原生 `.9`。宿主 check、protobuf 6 项、登录解析 2 项通过；shell 库测试编译成功，资源整理测试真实执行 2 过／1 失败。失败位于 `keeps_only_regular_files_inside_the_assets_dir` 首次创建文件符号链接，Win32 `1314` 表示该 runner 缺少所需特权。失败日志 SHA-256 `e9f551f19e5a7f23ba20d0cd025281f314c8263c72d7302c52890f1e4b90f2b2`；独立核验摘要 `7182a475d2b9d0b2cf1c684b04ee63e94ef74856a096bfcc7d55685a97e95e31`。余下 SSH worker 通用构建主动停止，整轮 cancelled，不计通过；管道上界、原生 check、桥回归和构建尚未执行，没有 Windows 工件。

该资源测试不是 G01 文本原子桥测试，且相同测试源码已在 Mac `r-nrzw1sgl` 实际三项通过。按用户已授权的外平台实机验收后置范围，仅这一项移交至具有符号链接权限的 Windows 环境，构建收据明确记 `deferred_not_passed`。原测试源码及拒绝断言不改；脚本固定文件 SHA-256 `abb12e06f3d233c794c054d6b80e434efe2f926fd46b29c8eb7b9070e728717e`，源码变化时必须重新审计。其他两项及全部原生桥、管道上界、完整 check／build 仍为必过项，不可据此跳过原生能力失败。

本轮宿主 `cargo check` 和 11 项 i18n 在短目录 `r-f_e0vaas` 通过，日志 SHA-256 `dfa9df27ffba8f47b6451f305ee0114a013e0e30f7d72293dfebaeabb3b1bfff`；完成退出、证据归档及目录清理。无需本地化变更。G01 仍待 Windows 实际工件、摘要绑定与最终源码门禁，本轮关闭 0 项；累计 4 项关闭、7 项开放、3 项移交，PR 保持草稿。


## 2026-10-01：G01 最终宿主门禁与原生构建分离，尚未运行

现有工作流把 Grok 工件构建与 Windows 生产 ConPTY 辅助验证绑在同一个开关下，且原生定向模式跳过含历史 LEAK 的 Windows 桌面／TUI 组。新增默认关闭的 `run_grok_native_bridge_host` 只解开验收耦合：继续完整筛选 `local_tty`、普通桥及账本回归，构建 Windows 主程序和 SSH 辅助程序、执行真实原 ConPTY ignored 用例，并执行 Windows 桌面／TUI 组。原生构建仍只由 `run_grok_native_bridge_source` 控制；仅 GUI／仅安装器冲突检查前移，完整工作区模式仍优先。最终绑定提交的门禁尚未派发，不计通过。

`r-xaxm00n1` 的 actionlint 与本机 `cargo check` 通过，日志 SHA-256 `7cac323e4c1e57986f4bd96b20aff9afdb86c32a9f9a5d05bb575c284943b2a6`，短目录已清理。首次 actionlint 只报告未登记的既有自托管标签；补用仓外配置明确声明 `infinishell-ci` 后通过，未忽略语法／表达式错误。无用户功能或文案变化，无需本地化变更。

此时 [CI 36770502561](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36770502561) 仍在运行，对应 `582596a090f9b0042ccc349fc9568e3fe072e6d1`。一个独立前置步骤失败：固定 Codex 0.156.1 的两个 hook 场景均通过、零模型请求，但清理 Git 临时 pack 文件收到 WinError32。八份关闭收据确认根退出0、输出EOF和Job为空；最后一次强制回收三个后代，不称全部自然退出。占用者尚未确定，不能归因Grok或忽略失败；诊断摘要 SHA-256 `60e63bdb64ac8160508ede66aff8f2867034f7c333bae7adeb03b8ee7af73c67`，原始归档保留。Grok 原生源码步骤仍继续，不记本轮整体通过。本轮关闭0项，累计4项关闭、7项开放、3项移交；PR仍为草稿。


## 2026-10-01：G01 Windows 会话 actor 绝对路径夹具修复

[CI 36770502561](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36770502561) 的原生 `.9` 结果已逐份核对 15 步日志及源码元数据：protobuf 6、登录解析 2、资源测试 2、管道上界 1 项通过，额外符号链接项仍按前节移交而非通过；完整 `xai-grok-pager-bin` check 通过。四库桥门禁实际执行 pager 53 项通过、shell 15 项通过／5 项失败，后续构建和版本检查未执行，没有 Windows 二进制。五项都在 `acp_session_tests/support.rs:287` 的 `/tmp` 转换收到 `NotAbsolute`，尚未到事务断言；tools／shell-terminal 两库在该轮也未执行，不虚构全组计数。失败日志 SHA-256 `29e8939c615e0a6694a8db4795746126dcee6bd179555174bb4dbbddd5e8ed6a`，失败摘要 `8647c7fc4871b2a88c53755f7ccf9a3d9cb4e464e8531fe27785f45c8c5a9da3`。

确认失败后停止第40步 `Build InfiniShell SSH worker`，整轮 cancelled；独立 Codex hook 清理 WinError32 原件保留，不与 Grok 路径错误合并。完整日志 ZIP 的41项完整性检查及 Windows 聚合日志逐字节核对通过，ZIP SHA-256 `b69d5bf8e3f8d6ca352d0fb0a00e3dcc58f356af294ed3d68a5934e13214162a`，归档摘要 `f315e5d86408d41c61b2a8974611a38b296571c92f20a14e12e02361525f7b11`。

修复仅一行测试工作目录：使用系统临时目录，经原 `AbsPathBuf` 验证；不修改生产代码、审批与原子准入或防重断言。原生 `.10` 提交 `3e9796a80e5f99a1a2e170d893aa6ac0b492383f`，tree `ce1d2efa7f57d1a54e8d818203c8b7d9c0722754`。73文件补丁从公开基线独立重建相同 tree，补丁 SHA-256 `1d93b524f2ff22aa30210b30ee4598c863bef49d1203b2c4955b53b3cd4549a8`，重建收据摘要 `64d160d02e6ad4f38f8a32965831706eeb55c9dd37249a567ab7def63f33cb89`。四库命令加入 `--no-fail-fast` 以收集所有库的实际结果，仍要求每库命中且零失败。

Mac `r-otw97gga` 四库 46＋20＋4＋4＝74 项通过，日志 `39a666d45fe6b96a54df8482b7a207a0bb596e401cc6d627b9a4b22d1564dd58`；`r-6_g2n_82` 完整原生 check 通过，日志 `dc9a0f53f070caec8b361da31c6bc1387abf40eca6da65bdeaf8aa23245c5b7d`。两短目录均已归档清理。Windows `.10` 目标复验、工件绑定及绑定后最终源码门禁待完成；无需本地化变更，本轮关闭0项，累计4项关闭、7项开放、3项移交，PR保持草稿。

本轮提交前宿主 `r-xh0t_gjl` 的 `cargo check` 和11项i18n通过，日志 SHA-256 `6ca82e59adcab9e18716231adfaafa1d4bc61093219f76c9c0f85243ee7c1316`，短目录已归档清理；该门禁不替代Windows目标执行。


## 2026-10-01：Windows hook 清理收据核验补齐

复核前述 WinError32 失败发现独立缺陷：私有目录清理入口只核验根自然退出、EOF及配置回滚，没有读取已经产生的后代Job关闭证明，汇总 `descendants_job_verified` 因而一直为false。现要求formal/candidate各阶段的每份关闭收据同时确认暂停创建时Job归属、全部后代已核验、Job活动数为零及Job句柄关闭，全部满足才设置汇总true并进入原目录身份校验。缺失、false或仍有活动后代均保留现场；根自然退出后已确认回收自己的后代仍可清理，不能把它写成全部自然退出。

该改动没有改变WinError32处理：文件占用继续失败并保留现场，不增加重试或修改权限。根因仍未知，也不据此关闭G01或其他缺口。原生hook和ConPTY脚本共享此入口，实际Windows验证待最终源码门禁，不继承正在运行的旧源码 `08f2fff8d` 的结果。

本机 `r-vih3onyz` 共50项hook测试中48通过、2项Windows原生限定跳过；包含formal阶段缺证、Job归属／非空／未关闭拒绝、已确认后代回收正例及文件占用继续失败。日志SHA-256 `86495b5e49c42876ad59596aa721fde08cc577f948574b3360b6f13dc4f810f6`。共享ConPTY回归 `r-6yxlzc4d` 通过，日志 `0b7216c5a0d4fee137b9f3cabec50a1f1d1d5bf1472e65214d54bb38da4ceb20`；宿主check `r-2lyor01q` 通过，日志 `ced779af54b0177965a73360381e6be3a44f3e1149b04ec31993a7e382029ef2`。三轮短目录已归档清理；只有验收脚本及收据语义变化，无需本地化变更。关闭数仍为4，开放7、移交3，PR保持草稿。


## 2026-10-01：G01 Windows 启动栈与零命中门禁修复

[CI 36774428641](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/36774428641) 绑定 `08f2fff8dc5a1b8502b0cfb7750a0a1a5c064ba8`，已结束为 cancelled；原生构建步骤失败后停止余下重复 SSH worker 构建，取消不计整轮通过。19 份日志、公开基线加补丁、73 份源码摘要与实际 PE 均已独立核验。`.10` protobuf 6、登录解析 2、资源整理 2、管道上界 1、完整 native check/build 通过；四库实际为 pager 53、shell 20、shell-terminal 0、tools 4，共 77。先前口头按预期称 81 不准确：终端子进程四项被 Unix cfg 排除，未执行。

实际 PE 大小 245,529,600 字节，SHA-256 `3bec46f7f0941f3b5070600ae19605d53ce5043273ae705074963306ab3164f2`；第19步 `--version` 返回 `3221225725 / 0xC00000FD`，日志明确 main 栈溢出，不能绑定为可用工件。只读 PE 核实主线程栈预留 1,048,576、提交 4,096 字节；沿 CRT 至 Rust main 的反汇编核实 main 单帧 186,280 字节，不能据此断言某个 future 是唯一根因。分析收据 SHA-256 `1b7c3fc9e98acb5d2275db3c9243f03e4fdaa0e75b2d42721b3a80ca24d9158f`。`.11` 仅为 Windows MSVC 的实际 CLI 映像设置 8 MiB 栈预留，采用 [MSVC 的正式链接参数](https://learn.microsoft.com/en-us/cpp/build/reference/stack-stack-allocations?view=msvc-170)，不改变工作线程设置或强制预提交；是否解除启动失败仍须目标执行。

同轮补齐四项终端守卫测试的 Windows 真实子进程路径：direct 调用固定 PowerShell，streaming 沿用生产 shell 检测，含空格与单引号的路径通过环境传递。后台忙碌、持租约时零提前派生、释放取消时仍可见及实际退出后的空闲断言保留；异常收尾只操作本会话登记的真实句柄，不吞断言。构建脚本要求四库分别实际命中且零失败／忽略，并检查 PE 的真实栈预留、`--version`、`--help` 及经过正式运行时和 `async_main` 的 `completions bash`。这些启动检查使用私有 GROK_HOME、零模型输入，不冒充 stdio 初始化、交互 TUI 或模型验收。

失败收据 SHA-256 `962579e3bb012abd053df6d1c0c2cbc2a1e35200b3bd66434b58cdf68f052278`；41 项完整日志 ZIP `eba7f1c433b03f38cdbbb6372222011eb48c2fcc1e22adc35dadaafd6597c7e2`，归档摘要 `85712d16f225a5cc338b8186f282b58b2ca5d8f57fca6356a13559aeed075268`。本轮 Codex 0.156.1 hook 清理未复现旧 WinError32，八份关闭原件独立满足 Job 退出条件；该轮尚未包含后来的清理脚本修复，不记作修复后验收或旧占用根因已解决。

Windows `.11` 启动与四项子进程回归、可用工件摘要绑定、绑定后最终 Linux／Windows 宿主门禁仍待完成。Mac 原 `.3` 功能证据与 Linux 原 `.6` 工件独立保留；无用户界面文案变化，无需本地化变更。本轮关闭0项，累计4项关闭、7项开放、3项移交，PR保持草稿。

本轮原生提交 `77d8004b18b5d7c7f7ced42f084dbe5a54048a23`，tree `52afcf3ebad44daaef887cbe2cfe3ed003d6d44e`；74 文件补丁独立索引重建一致，补丁 SHA-256 `7690e16a7bd42fcdd591d453acb6434571c92b45561139b87eb31cb2d20fa408`，重建收据 `beaf4ae673a15573d79c80cf8c06578e99ffaef1ce3bffc22988f264e00c566a`。Mac `r-pr0y8law` 的完整 native check、四库74项、build、版本／帮助／异步补全启动检查通过，日志 `52c0d97d4ceafaf73314c2b4bb8b6aa152a203fc97904b0a5c9020af63243dcf`；宿主 `r-e2coze3m` 的 cargo check 与11项 i18n通过，日志 `be16cab5a35a093d8633fec60473ddef27032ffdaf37c7a2818cb2675020850d`。均已核验退出和归档，短目录已清理。Mac 本轮程序来自提交前工作区，不替换既有正式 `.3` 工件或冒称新增 GUI 验收。
