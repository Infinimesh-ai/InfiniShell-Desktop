# Source40 恢复与输入保护候选

父提交 af1040，source40 冻结204文件／134个相对父提交变更；清单SHA f1235c189bb798ab46d38a09cb972b11de5f73d8f14c81b375ece0e548930f25。仍为未提交候选。

## 实现范围

- 普通PTY：三方在可信等待回应状态下拦截应用代发，并在延迟正文、图片和回车处复核。Grok 1.0.30缺乏可靠输入就绪事件，普通富输入保留草稿、附件并提供显式复制，用户回原生提示符确认；不再自动CR。文件、技能与评审代发共用守卫；不报告假发送或ACK。此为明确降级，不能计Grok自动提交或运行中追加通过。
- Grok托管：只接纳完整builtin目录或builtin与本次真正发送的MCP工具目录之精确并集，绑定当前nonce、进程epoch、代次与原生session账本；旧回放、缺项、未知名、重复项仍拒绝。另配零模型预检，逐个名称验证，尚未运行原生。
- Codex插件：Windows x64正式通知安装使用固定官方exe摘要、MSYS Bash／jq／系统PowerShell等真实依赖预检。编排插件未开放。新增版本化安装journal、锁和实际缓存／限定配置状态恢复；用户改动与未知状态仍拒绝。已完成同版本安装不重复调用原生add。
- Claude插件：正向替换与回滚暂存移到活动缓存树外，同文件系统原子替换，保持未知文件与并发修改保护。Windows正式安装仍关闭。
- 原生夹具：Files取消可包含本回合审批前文字，但拒绝工具后输出；空Grok info初始值严格使用已观察的1／0，不把计数当作历史正文证明。

## 实际门禁

[source40输入](validation/macos-official-40-inputs.json)、[门禁](validation/macos-official-40-gates.json)、[离线回归](validation/macos-official-40-offline.json)：check退出0／104.061秒；9组Python178项通过。i18n在测试编译阶段退出101／137.539秒：新增审批保护测试把AppContext订阅回调写为4参数，而实际签名为3参数；测试未运行、focused未执行、main未构建。

后续source40b仅修正该新测试文件，包含3参数闭包、循环捕获及关闭夹具自动收起富输入，避免将发送守卫误测为关闭状态；清单SHA 1279248b1fdf49de1f106c4c63b0bcc4c97fd0eff7f0048f789bd935b08401b4。[实际门禁](validation/macos-official-40b-gates.json)：check退出0／65.883秒，i18n11项通过／233.502秒，受影响Rust1809项中1808通过、1失败；没有构建main或执行真实CLI。失败为目录测试通知漏写必需的`availableCommands`数组，完成目录校验后在元数据解码处报错。发送保护7项全部通过。

source40c只补齐该测试通知字段并使用可诊断的`unwrap`断言，未放宽产品校验；冻结204文件，清单SHA 715f807e0b9d65791f4ce70b2ba706b8d2411db0a60e00283fd6fd23d1447f2a。[实际门禁](validation/macos-official-40c-gates.json)全部通过：check退出0／126.506秒，i18n11项／171.189秒，受影响Rust1809项全部通过／26.957秒（5317项跳过）。[桌面main](validation/macos-official-40c-bundle.json)构建退出0／216.236秒，严格签名及双语资源嵌入通过，测试二进制未变。source40与40b原失败及源码快照均保留，不能追改为成功。

英文与简体中文同步新增Windows依赖和手动复制提示。[Codex真实强杀恢复](validation/macos-official-40c-codex-journal-recovery.json)已使用此次测试二进制完成7个窗口：每次独立子进程SIGKILL退出-9、新进程恢复退出0，产物摘要前后未变；未运行Codex模型、未读取凭据，不证明断电或Windows恢复。source40至40c仅两处Rust测试文件变化，9组Python178项的输入仍逐字节一致，没有重复运行或回填其来源。原生目录、Windows安装与Grok复制入口双语布局仍待验证。完整工作区及最终同提交平台、SSH／tmux和全部三方链仍未满足，Goal保持进行中。

固定macOS Codex 0.147.0的[正式安装器验收](validation/macos-official-40c-codex-source-installer.metadata.json)也已通过：真实marketplace add／plugin add将受控rev3升级至rev4；完整源、缓存及限定配置通过验证，原缓存／用户配置／信任与编排禁用状态保留。重复安装成功但原生命令调用为false；已改旧源、混合缓存、未知来源与禁用目标四案均拒绝且字节和权限模式不变，未执行原生命令。测试库、固定CLI与清单摘要前后不变，私有目录已清理；没有凭据或模型输入，不证明hook授权／执行、GUI或Windows安装。

## source40c 原生与 GUI 验收

- [Claude 正式插件事务](validation/macos-official-40c-claude-plugin-audit.json)：固定2.1.273 CLI通过生产Rust安装入口，将插件2.1.0升级至2.2.0；另一独立私有HOME中，用真实目录写权限失败验证原配置、注册表和2.1.0活动缓存保持不变。两案分别11.865／10.161秒，模型请求0、凭据复制0，进程树清理确认；不是强杀事务或Windows证明。
- [Grok 原生目录预检](validation/macos-official-40c-catalog1-host-events.ndjson)：零模型输入下，真实拉取并验证builtin与本轮MCP六工具精确并集，`pull_verified=true`；没有观察到`availableCommands`并集通知，不能将拉取通过写成该通知路径已实测。进程和认证副本已清理。
- [Grok 父子链 Child5](validation/macos-official-40c-child5-host-events.ndjson)：失败。父任务派发及子任务读取审批允许已到达，但运行中父→子追加被应用错误路由为Grok不支持的`Steer`，持久消息从Sent变Failed，没有原生ACK。共进入2个原生输入；2个进程清理成功。修复还须补充Grok邮箱持久队列和跨代回执校验，不能只更换命令或放宽夹具。冷继续未执行。
- [Grok 文件工具 Files2](validation/macos-official-40c-files2-host-events.ndjson)：前四阶段写入允许／拒绝、编辑允许／拒绝均通过，实际文件与原生终态、历史、ACK一致。第五阶段审批等待中取消验收失败，已确认Cancelled、文件未创建、审批已取消以及产品最终历史验证成功。原生52条回放中，当前工具的两条事件均缺少status与rawOutput，末条是唯一MidTurnAbort回合终态；夹具却要求单独工具终态，导致失败。后续候选只针对这一严格形状验证回合取消闭合，不伪造工具状态，不放宽拒绝审批；原失败不追改为通过。第六阶段冷继续未执行，总体不能计为通过。各已执行进程和认证副本均已清理。
- [普通Grok双语GUI](validation/gui-official-40c-grok-input/report.json)：单独`grok`显示工具栏；Enter保留草稿，显式复制、清空再原生粘贴回富输入得到完整中文、多行及追加尾缀。英文提示可展开；中文提示截断且无展开按钮，双语布局失败，后续候选缩短两种语言文案。切换语言后现有空提示仍缓存英文，未重启复核。此轮未向Grok提交模型提示，但GUI历史摘要不证明零网络模型请求。

GUI清理时，关闭后查询可访问性树导致自动工具用默认环境重新启动测试应用。该次重启后没有启动CLI或执行验收输入，仅再次关闭；最终应用及原生进程为0，7个全局CLI配置及原认证身份未变，私有认证已删除。不据此声称整个应用配置绝无启动写入。后续关闭后仅用进程查询确认结束，不查询已关闭应用的可访问性树。

Windows Claude零模型通知探针已另行加入候选：24项离线测试通过；真实ConPTY通知、同提交Actions及正式Windows安装门控尚未通过。上述结果仍绑定source40c，不能自动覆盖后续邮箱修复、文案修改或Windows探针。

## source41 本地回归与后续审查

[source41清单](validation/macos-official-41-inputs.json)冻结206文件，清单SHA `a73bf864ea6488c3bb150a3457299de65e4df1109321c67d3cf35cd4a8ea3a9d`；相对40c更新13文件。包括Grok邮箱Submit及持久原代回执、严格待审批取消／diff预览校验、`cd <字面路径> && CLI`有限识别、中英文复制提示缩短和队列满文案、Windows Claude通知探针。复合识别不改变共享shell解析器，拒绝管道、任意前置任务和复杂展开；`cd`别名也保守拒绝，尚不宣称PowerShell默认别名支持。

[实际门禁](validation/macos-official-41-gates.json)：check退出0／66.435秒，i18n11项通过／172.784秒，受影响Rust1826项全部通过／27.370秒（5317项跳过）。[离线验证](validation/macos-official-41-offline.json)：Windows通知24项与私有验收驱动11项通过，均未执行原生CLI。私有驱动首次有一个遗留源码标签断言失败，改为验证新标签的实际路径输出后通过，没有修改生产保护。

静态复核发现仍未覆盖的时序缺陷：自动子结果在父Running期间进入原生队列后，父随后Failed或因审批拒绝Cancelled，Grok仍可能消费该结果并开始新一轮。现有测试只验证父先终态再尝试投递，不能证明相反顺序安全。后续候选将自动结果保留在SQLite，只有同进程父Completed且无待启动输入时才逐个派发；消息原recipient_generation不变，并核对原／现父代的原生会话和runtime token，兼顾父收到进度后继续回收原代子结果。不能通过修改Child5的六输入预期或忽略已执行的原生started事件掩盖问题。

source41通过上述回归不代表这个时序问题已修复。桌面构建及不依赖邮箱的取消夹具、双语布局继续独立验证；源41a修正、真实父子全链及最终同提交跨平台仍待验。


## source41 原生与双语界面收据

[桌面构建](validation/macos-official-41-bundle.json)退出0／162.653秒，严格签名和完整双语资源嵌入通过，测试库保持不变。

[Files2真实六阶段](validation/macos-official-41-files2-host-events.ndjson)运行192.572秒，整体失败；写入允许／拒绝、编辑允许／拒绝、待审批取消五阶段全部通过。第五阶段的精确缺status、MidTurnAbort及diff预览形状已被真实验证，文件仍不存在；这是新source41实测，不追改40c原失败。第六阶段cold_read已Ready、同原生ID与profile、无自动重放，并取得唯一输入ACK；但未取得审批或最终结果。

[冷继续诊断](validation/macos-official-41-files2-cold-read-diagnostic.json)证明六个阶段共用的本地代理32次CONNECT额度已耗尽：16次本地预算拒绝与7次原生HTTP重试的主机一致，不能称为上游403或生产恢复失败。后续仅修正有界测试连接预算，仍保持6输入、900秒和原结果判据；该次整体失败保留。所有阶段进程、认证副本和隧道已清理。

[GUI收据与六张截图](validation/gui-official-41-grok-input/report.json)验证`cd <字面路径> && grok`显示工具栏；英文提示两行完整、简体中文一行完整，按钮可见。英文与中文均通过应用内显式复制按钮、清空、原生Cmd-V回填，保留中文、英文、多行和唯一ASCII尾缀。首个英文复制操作因提示过期返回旧剪贴板，已排除；重新使用唯一尾缀并及时点击后取得独立通过。切换中文后真实退出、通过私有环境重启，中文占位提示完整。没有向Grok提交模型草稿，但不以GUI元数据证明网络请求数。两次启动均使用隔离环境；最后进程与原生会话为0、7个用户CLI全局配置和原认证身份不变，私有认证已删除。此为明确手动复制降级的布局验收，不等于自动PTY输入完成。

用户追加的最新版与消费者自动升级验收见[新增范围](CLI_AUTOUPDATE.md)，此处旧版本证据不能替代新版本结果。
