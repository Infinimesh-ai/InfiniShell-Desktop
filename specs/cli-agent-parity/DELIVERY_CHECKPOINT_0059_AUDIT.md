# 0059 检查点配套交付白名单审计

本轮仅审计已闭合公共档案，档案安全与原字节守卫结果为 **PASS**。受审清单共 257 个文件：28 份明确授权 Markdown、113 份 JSON、114 份图片和 2 份公开 AX 文本。逐文件精确路径、大小、SHA256、严格解析、安全扫描和拒绝项见 [机器白名单](validation/delivery-checkpoint-0059-allowlist-audit.json)。本轮未 stage、提交或推送。

公共身份档案记录实际提交 0059ef1bf8438c5af7545c3101b880ec2d0b10e9，父提交为 e6873e2cbb12a12ffce0b8283ee2ac96b940da39。该身份是此前执行者的公共记录；本代理没有读取源码、Git 对象、冻结树、target 或二进制进行本轮证明。文件名称中的 0059 不将较早证据追认为该提交的原生或跨平台验收。

## 验证边界

- source26 的公开本地门禁及 GUI 档案属于实际 436cc739。Claude basic 的独立审计实际位于 validation/gui-436cc739/archive-audit.json。
- source27、28、29 的候选门禁原 worktree_dirty=true、未提交候选身份及 same_commit_verified=false 语义保持。本轮不将候选成功追认成实际干净提交执行成功。
- CI12 原失败、CI13 初窗与次窗的 pending 保留；CI13 第三窗终态仅证明 e6873e2 的所选 Linux/Windows full=false 检查点。日志获取失败及逐 case 数量未知继续保留，不由 API 步骤组 success 回填。
- source29 的实际提交内容映射依据已有公共身份档案。source30 插件离线报告记录 actual0059、11/11 TAP PASS、exit0；首次默认统计解析失败仅留元信息，没有改成首次通过。该离线结果不证明真实 CLI、模型、插件原生安装或后续 policy3。
- Grok 原生失败诊断、协议与维护形状复核只证明各自记录的安全诊断和离线契约范围；string response ID、未知通知、业务来源、SDK/子任务/权限上限或沙箱未由此全面证明。旧 FAILED 不改为 PASS。
- 新 RELEASE_SUPPORT.md 明确未发布、Goal 未完成、固定验收版本、插件信任、恢复/回退及关闭未验证能力；本文不开放任何产品能力。

## 公共 GUI 档案

三个闭合目录分别按既有独立审计的 **65、79、31** 个原始文件逐项核对真实大小与 SHA，加上三个审计 JSON，共 178 个公共文件。原清单内 SHA 守卫 全部通过。图像解码结论只复用字节完全相同文件的已有审计记录；本轮没有图像重新解码、GUI 重演、OCR 或独立像素语义验证。凭据形状的图片原字节扫描不证明像素内没有秘密，也不证明 GUI 全链验收。

## 安全、链接和提交边界

JSON 使用严格重复键拒绝、NaN/Infinity 拒绝以及浮点溢出非有限值拒绝；扫描覆盖 JSON 原序列化文本、递归解码键和值、Markdown、公开 AX 文本及图片原始字节，六类固定凭据形状命中均为 0。仅记录计数，未复制敏感命中内容；固定形状扫描不是所有秘密不存在的证明。受审原文件在审计后逐 SHA 再核对不变。

本地文档链接只检查路径存在，不跟链接扩大正文读取域。可检查的坏本地链接共 **0** 个，完整明细仅记录在 JSON；坏链接不触发读取或修改未授权文件。外部 URL 不访问；specs 之外的源码、target、tmp、私有数据链接不访问。

CI12 两份 .log 摘录本轮明确不读取、不计算其 SHA、不纳入白名单；其他原始日志、RPC/native 正文、认证/私有数据库、用户两个 web 源文件、gui-7e065085 整域、根主三文档、新 policy3 域及另一代理的 SDK business mapping review 均排除。白名单外默认拒绝，禁止通过文档链接扩大 stage 范围。

是否 stage/commit 由根代理按下面精确清单与 JSON 中逐 SHA 守卫另行执行；本轮安全审计不替代 cargo check、同提交跨平台、完整工作区、在线三方验收、真实 SSH/tmux 或 Goal 验收。本代理未参与原本地 Cargo、CLI 或 GUI 执行；CI12/CI13 等部分公共档案由本代理先前生成，本轮不是这些档案的独立第二审计者。未独立重跑原测试，不能将配套清单复核计为独立执行验证。

仅新增交付审计文档和机器清单，无用户可见功能修改，**无需本地化变更**。

## 完整受审子文件清单

以下为受审输入清单，不包含本文及机器审计文件自身；这两个新增产物的最终 SHA 由完成消息单独交接。清单列出逐文件 SHA 和大小，不能按目录通配符直接 stage。

| 精确路径 | 字节数 | SHA256 |
|---|---:|---|
| specs/cli-agent-parity/CI_12_FINAL_VERIFICATION.md | 7559 | 182d3120bafc9f09f36d5cf1f478232f1595a1bbc59e79972e8c47cb77d58af6 |
| specs/cli-agent-parity/CI_12_INITIAL_VERIFICATION.md | 5404 | 602eca8829517d6554cdc73ae8dbe74ff9573429e08ba7d30e4ef8b1c19a0a82 |
| specs/cli-agent-parity/CI_13_FINAL_VERIFICATION.md | 3370 | f36d7d5d91a9915e7934edeceeb26255513754e294cd48d900a6791fc3026c75 |
| specs/cli-agent-parity/CI_13_INITIAL_VERIFICATION.md | 4095 | 7160eb4ebeb648b9a1b5d1a5a83136ce72e0a3822d1934c2b2ae88bc1c3b1af1 |
| specs/cli-agent-parity/CI_13_THIRD_WINDOW_VERIFICATION.md | 4427 | 7e7fc0830f63d3c88a8b5110acdd9b754881fadd0cafc71e1d566ed39e1c4a14 |
| specs/cli-agent-parity/GROK_26_PROTOCOL_FAILURE_DIAGNOSIS.md | 7158 | 1b04a898669bc1dd5d24949ed4e0ff9d289bf82dc47aeea2943b549db4bfccf1 |
| specs/cli-agent-parity/GROK_27_PROTOCOL_DIFFERENCE_REVIEW.md | 4735 | b369a089c19613433cdbaa6ddc4005834b67c26e7f6e94fd06d86d980e3d63d6 |
| specs/cli-agent-parity/GROK_FIXED_SOURCE_AVAILABILITY.md | 5908 | 89e84e3c54fd8f5a20a89b90e3f5b621fe715ee4bb62bf91b5bea88886a48e47 |
| specs/cli-agent-parity/GROK_HISTORY_SEND_REVIEW.md | 5427 | 8bff41cd2d951598503d965cc57bb07c2d41cb1bfb9da9fcfb39df36798cee84 |
| specs/cli-agent-parity/GROK_INTERNAL_MAINTENANCE_SHAPE_DIAGNOSTIC.md | 3249 | ef426f837f18287b339d43da5167e3d8def169d446fa3b6d1e17411ded3c4d70 |
| specs/cli-agent-parity/GROK_INTERNAL_SKILLS_RELOAD_NATIVE_SHAPE.md | 1882 | b083b1589ee9234740a340a62c435e20ce105f311c52ac3bc1553c1c0849cf90 |
| specs/cli-agent-parity/GROK_POLICY_GLOBAL_CATALOG_FIX.md | 4007 | 282ed091cb54190e26985ecf86b37af89f359a4e44fdbacc204316e6c7e88d26 |
| specs/cli-agent-parity/GROK_RESUME_NETWORK_CONTRACT_REVIEW.md | 6101 | 6cc42977b734601e49136d3e39cbc70f14378f20008ce8452453c90f9f469b38 |
| specs/cli-agent-parity/GROK_SKILLS_RELOAD_CONTRACT_REVIEW.md | 7554 | 585f7a98823e585945e3caf8fa14b9ac7678db4d6d84c8504e5afdebb42faef1 |
| specs/cli-agent-parity/GROK_SKILLS_RELOAD_OWNERSHIP_REVIEW.md | 9529 | 7eb3ab0498aca17c401faca8434aceceea5bb1485729b6764463d9630343cd7c |
| specs/cli-agent-parity/OFFICIAL_26_CLAUDE_PNG_GUI_ARCHIVE_AUDIT.md | 4465 | 2b8b06aef88ebde09b30333ddc81b6d14d2c02272833a633db619c82ccd7afeb |
| specs/cli-agent-parity/OFFICIAL_26_CLAUDE_PNG_GUI_VERIFICATION.md | 5944 | b641ffe93764878b153420fc787d94585d7d72ae53b1523552fb22806860f382 |
| specs/cli-agent-parity/OFFICIAL_26_GROK_GUI_ARCHIVE_AUDIT.md | 4570 | 438a9f056e5894128ceb0bd1f8b387d6ebeebee2a1e0bf56594c637e0c971300 |
| specs/cli-agent-parity/OFFICIAL_26_GROK_GUI_PARTIAL_VERIFICATION.md | 11831 | 9ce44af60716f7ee325c54dd6912d90e4a3fcfc7c75820ad09bf3b00edd8de61 |
| specs/cli-agent-parity/OFFICIAL_26_GUI_PARTIAL_VERIFICATION.md | 8546 | 624caccd5a6d86ac8e159cabaf1e659ec9ec947cb458bdb49f3a937ca975c277 |
| specs/cli-agent-parity/OFFICIAL_26_LOCAL_GATES.md | 5346 | 2e0406699b3cbf3ccf758843fc987c0cf65269a955b72e00f94a3f775f5619ac |
| specs/cli-agent-parity/OFFICIAL_26_NATIVE_PARTIAL_VERIFICATION.md | 9993 | e5da75b929cb80b8dd2997483ffdc093eac5924db360445154586115d83ce25a |
| specs/cli-agent-parity/OFFICIAL_27_LOCAL_GATES.md | 2853 | 9a7076c8f2f798d59ca546292c5d790447db9eae2904179b4d4ec1dd7f743cbb |
| specs/cli-agent-parity/OFFICIAL_27_NATIVE_DIAGNOSTIC.md | 7267 | f64604c84cbf4c10c779513fb61d623955d43e4e4618910c428099c9bda69334 |
| specs/cli-agent-parity/OFFICIAL_28_LOCAL_GATES.md | 3062 | b0c4379f984866b30f942bef1acce578976c6f8f11e55672056d7f658db44ad3 |
| specs/cli-agent-parity/OFFICIAL_29_LOCAL_GATES.md | 3109 | 164f2f8144f6352776442c2c41fbdd5cc5337ee3d02cb1c0b4249ba78def0e14 |
| specs/cli-agent-parity/RELEASE_SUPPORT.md | 6390 | a558602522c4162ca76195fb483deed929f5859de93ada3c4f98d0cd463ef864 |
| specs/cli-agent-parity/SOURCE28_CHANGE_REVIEW.md | 5829 | d693c89145f61ebfee8718a9044f275f5d46a85368ed5fadcc674ad6ffde8775 |
| specs/cli-agent-parity/validation/grok-26-protocol-static-audit.json | 13810 | 0af845a97d1f83aee71ae5b16d4aca9971f8f2863a5ec52b6a8bcea1b5e04b74 |
| specs/cli-agent-parity/validation/grok-27-native-response-field-shapes.json | 1013 | a0bc934fc9f285f2c49b704aef04b661282339e269cc9eb638bd2359a2db55c3 |
| specs/cli-agent-parity/validation/grok-27-protocol-difference-review.json | 8041 | 322b975069c656ce0c84d1e7945320d57e3fb441ac27d9e6932b91a50c3ee723 |
| specs/cli-agent-parity/validation/grok-fixed-source-availability-20260918.json | 29750 | e2fecea68e68b72049e0eeff4b9b32730ba59d2acc25b14ec17f753a5007d4f4 |
| specs/cli-agent-parity/validation/grok-history-send-review-source26.json | 3662 | 80b42d7bb15127d541c81230fb086228bef35b2659cfb70a62661af0117f49c9 |
| specs/cli-agent-parity/validation/grok-internal-maintenance-shape-diagnostic.json | 5585 | 3158a368e8ebee22cf26d44eae8aca07a05e03a25d309a8630ae69aa21327f44 |
| specs/cli-agent-parity/validation/grok-internal-skills-reload-native-shape.json | 5281 | 873c448e35d8be1a3d9452436f6fbe5c6963da8c6140e752a57ad180b25a5b1b |
| specs/cli-agent-parity/validation/grok-policy-global-catalog-fix.json | 5589 | ea72636a2121ab5261907a09360d6cdc064799ce0106444e93e0429ca182039e |
| specs/cli-agent-parity/validation/grok-resume-network-contract-review.json | 18866 | e5637025f647f0377223750ae6ae1e2eb8c5e9d322a011c6eb9421627175a19d |
| specs/cli-agent-parity/validation/grok-skills-reload-contract-review.json | 45151 | 56199e4ad6d914fa91f7435ecb8fce37ad470807563b8411fe35e2ef9dfa9626 |
| specs/cli-agent-parity/validation/grok-skills-reload-ownership-review.json | 58402 | 9ec83a27d709e3480511f6314d38c9e083009d078381f028abc7b571cac7f4d9 |
| specs/cli-agent-parity/validation/gui-436cc739/00-claude-api-environment-preparation.json | 992 | 2a80f73ddfc29019b29d5c1cd8de1e14f284d5c522446fd11268e1faa80b70bb |
| specs/cli-agent-parity/validation/gui-436cc739/00-empty-task-en-restart.json | 575 | c2f550a62fa7489a97931314dd101dfdc589680e0412537a97a79f24d04ad2f1 |
| specs/cli-agent-parity/validation/gui-436cc739/00-isolated-clone-preparation.json | 3119 | c27e61f530cc28991a05590d91190b3704bf18a563f45a169da36aa33412a1c6 |
| specs/cli-agent-parity/validation/gui-436cc739/00-persisted-task-app-restart.json | 1065 | c2fa735bbf8e8555063ca2b599236ad9236a4d9bb64bb956a57b66d998d79a7b |
| specs/cli-agent-parity/validation/gui-436cc739/00-processes-before-normal-exit.json | 498 | 02db024ea9057cc52d30fd27e7ce56a14c7c51da8acc656620669bc8a480b6be |
| specs/cli-agent-parity/validation/gui-436cc739/01-zh-titlebar-ax.txt | 699 | f6a117217944a2f66d014a76efd4d0216e904f6f0cbf4d2bbe3238293a93d463 |
| specs/cli-agent-parity/validation/gui-436cc739/01-zh-titlebar.png | 49846 | 9210db4624bfae37361cfcaa00f843517c66c23fdffd0296b7b94723baecaa7e |
| specs/cli-agent-parity/validation/gui-436cc739/02-zh-task-manager-ax.txt | 716 | 59e754e69dedd422e81bfcbe30bc0c2deb81d6fee7e8ec3c60e7b9d6ae4873be |
| specs/cli-agent-parity/validation/gui-436cc739/02-zh-task-manager.png | 119271 | 09ce8e7b882fd27d96f671d29cb8628b602c5403a7b7786b1677947cbda01230 |
| specs/cli-agent-parity/validation/gui-436cc739/03-zh-task-manager-scroll.png | 91718 | 12897230f220256f98c24ae706641f6391ec9928826558cd8816416a72743ebf |
| specs/cli-agent-parity/validation/gui-436cc739/04-zh-cli-settings.png | 114543 | 713c3e4e9e01646a7ee910ff80188973fec2700d373640ffa7e2de35c694b7b1 |
| specs/cli-agent-parity/validation/gui-436cc739/05-en-appearance-language.png | 69510 | 0b953d47652f080021a1cc5734f7f94bb26f2aa7086c034da3ee18a870f9e7e0 |
| specs/cli-agent-parity/validation/gui-436cc739/06-en-after-restart.png | 83396 | 1514a283f31fb35da05f5be0a0d32c42b05f2255cc2b79f5554b24542f255d07 |
| specs/cli-agent-parity/validation/gui-436cc739/07-en-cli-settings.png | 122926 | 16c53ace2ec28b1084f5cfcff5b1ae0ec55bbb942b2f222ad8b3768409b42212 |
| specs/cli-agent-parity/validation/gui-436cc739/08-en-task-manager.png | 126640 | 2aba616134d756a6daf97b9f9834caa8b952a36a3711a7ccb01412f52cc27ca0 |
| specs/cli-agent-parity/validation/gui-436cc739/09-en-task-manager-scroll.png | 103448 | 7ff4c3379468a78f07dfc12c89b95119e295ddd2362eaaefafcc6efdc6793981 |
| specs/cli-agent-parity/validation/gui-436cc739/10-claude-first-draft-multiline.png | 109367 | 08e462fb13af5e0bf5cbf4adb63674c3152a24e78fd7c3af2e798885147f072f |
| specs/cli-agent-parity/validation/gui-436cc739/11-claude-first-task-start.png | 108730 | 734c99410e9e7be1787ccb7111a28356584dd884996626285482cc49e441ebee |
| specs/cli-agent-parity/validation/gui-436cc739/12-claude-first-task-result-view.png | 108896 | 71bf31f462fe961e441288241d0ca740e2cebd04376a5d9afa0150af83b9684e |
| specs/cli-agent-parity/validation/gui-436cc739/13-claude-first-task-prelaunch-error.png | 126432 | b4eaded742954b0122a499b8d280566c2476e11e84352ca259a2d258512cda13 |
| specs/cli-agent-parity/validation/gui-436cc739/14-claude-first-task-bottom-check.png | 113308 | 3698bab7a22ae83d73908a38f4c494d4cf913c442a873b4743b82645af7bc815 |
| specs/cli-agent-parity/validation/gui-436cc739/15-claude-first-valid-start.png | 116922 | d48275c7386c4520f2d7bc8f6205c671d90b46e1b02760de3924ed58a8dda4d7 |
| specs/cli-agent-parity/validation/gui-436cc739/16-claude-first-current-state.png | 122429 | 0f86454d692e0a04fac37aa11f64da122fec8fd2ab910dafd0dce11e065fe880 |
| specs/cli-agent-parity/validation/gui-436cc739/17-claude-first-sqlite-proof.json | 1132 | 95e94a6ff77fb5c72ee1bf9751544309fc78391e82c3a98fe70ba2ce7772ee36 |
| specs/cli-agent-parity/validation/gui-436cc739/18-claude-first-output.png | 89060 | 7df7da511f3dc35174064907768ab6f25a02236bb1e471fad8ac1601c26174a5 |
| specs/cli-agent-parity/validation/gui-436cc739/19-claude-second-input-sent.png | 109242 | 34d03bfdedd9b54031fd120475a252cf77f4ba2e82792c8b83bfb68b6fd7b86c |
| specs/cli-agent-parity/validation/gui-436cc739/20-claude-second-sqlite-proof.json | 1371 | f71e55c6eeef53e12b3758ee34c3649851e609f0bf60e8866254b38257425373 |
| specs/cli-agent-parity/validation/gui-436cc739/21-claude-two-turn-results.png | 103193 | e5940cec66f3dd0dfa90091f147a9592aa46fb7707efa9df5a77c85f0ba33fe6 |
| specs/cli-agent-parity/validation/gui-436cc739/22-claude-write-input-sent.png | 126217 | c4c2831644571eaa2a073c4da6df9ee91a29a7fa5f4476bd51e8fdcf3fc53049 |
| specs/cli-agent-parity/validation/gui-436cc739/23-claude-approval-wait.png | 125273 | 7cb8cd38d61a9ecbfefc58f3a21ebcf41f81cc5e1bb103d1bdfe643f396047c6 |
| specs/cli-agent-parity/validation/gui-436cc739/24-claude-approval-allow-dialog.png | 137192 | 0bd2da6004f92942d43606fafb6d5bc5ec0079f0fde2610366304a8f3e0340e6 |
| specs/cli-agent-parity/validation/gui-436cc739/25-claude-allow-once-visible.png | 92075 | ac86ebdf77b2087c5f3d6d2541eca8a21f8ec35557dab9fc66df80e17b3ba306 |
| specs/cli-agent-parity/validation/gui-436cc739/26-claude-allow-once-clicked.png | 101538 | 2bbbf6e45b3cfb734ba35f1fe07a1a0f91c978af784e1e3966b146801e0c1753 |
| specs/cli-agent-parity/validation/gui-436cc739/27-claude-allow-sqlite-file-proof.json | 1285 | 2fab1f9e626beafb592d4e496fcf7406df0575ea091cc7dfff0197515fe9275a |
| specs/cli-agent-parity/validation/gui-436cc739/28-claude-allow-completed.png | 96642 | d884306a4fd7b445a32a4f4a7a55c94532ac2830cd0524dd2b7864324c0fc626 |
| specs/cli-agent-parity/validation/gui-436cc739/29-claude-deny-input-sent.png | 117852 | ae25be9c66661ffbe6c475f560b1b0b8986d63bf5b22952b7ab0981f78610cdf |
| specs/cli-agent-parity/validation/gui-436cc739/30-claude-deny-before-sqlite-file-proof.json | 589 | 6999bff55de1f7b88cf5c6f2fa0b2fd286bf8dfb8dff17069f5a205472bd76d1 |
| specs/cli-agent-parity/validation/gui-436cc739/31-claude-deny-once-visible.png | 119335 | 977c68831be96d8cc73c68e2dc51f4672fee36f26e1cae145ef1a47403c22269 |
| specs/cli-agent-parity/validation/gui-436cc739/32-claude-deny-once-clicked.png | 117920 | 49685fab43da8dc301e52f3cc415a8994ec4a7173091dfd93dfee2c49efd5986 |
| specs/cli-agent-parity/validation/gui-436cc739/33-claude-deny-after-sqlite-file-proof.json | 1170 | 3dbd0b927bc1be8b93104373bc4b2229fda7baa62d62faebfd18c4fbdee3256c |
| specs/cli-agent-parity/validation/gui-436cc739/34-claude-deny-completed.png | 117939 | 7f1db6e8bb7efafc3c5ddae6862fc80621498a77640a2979cb21dec6b935f0ab |
| specs/cli-agent-parity/validation/gui-436cc739/35-claude-cancel-input-sent.png | 141147 | d642aecc049d669264660a69540911cf06b6af6f97b9f98a1aa7ac3835ac18b6 |
| specs/cli-agent-parity/validation/gui-436cc739/36-claude-cancel-waiting-composer.png | 93391 | 89239c7ff9bdb9a380b0d16306e6fd94648e91ede2b3ded38ac4fdaad680d6f3 |
| specs/cli-agent-parity/validation/gui-436cc739/37-claude-running-append-draft.png | 101780 | ea0da63511dc5a0db515c1b9f8a25340c96729ffafac75826b857ddff16a6475 |
| specs/cli-agent-parity/validation/gui-436cc739/38-claude-running-append-queued.png | 104026 | 82c93260bacd7cc247e92696c85ca2e2a40ea42a9fc5e69b09b339c5d544493d |
| specs/cli-agent-parity/validation/gui-436cc739/39-claude-append-before-cancel-proof.json | 1738 | 3561c452239ecaf3eb91ec21976c6a441c67a93f07de3b4f6ff1ae6bf4ec132a |
| specs/cli-agent-parity/validation/gui-436cc739/40-claude-cancel-turn-clicked.png | 124327 | 8b698eb2ec4d9525eb389b0482fbf19baa494ab646096309ecfa5d0639433dec |
| specs/cli-agent-parity/validation/gui-436cc739/41-cancel-and-append-sql-proof.json | 6059 | 72418aa79314b589ea0fee676dca6ec2b2eb27a521141e5cce1190d05ed4ad5f |
| specs/cli-agent-parity/validation/gui-436cc739/42-claude-queued-append-completed.jpeg | 89647 | 7bb31442f19914e707e30881e7414778e36e9ef4f28815b6e65c3cb81e19c7b1 |
| specs/cli-agent-parity/validation/gui-436cc739/43-claude-cli-disconnected.jpeg | 88614 | 98017026f5fa8e2cec8490cb877ce46d0e7e7e28c1e9fa7ea826864da9b3b5f6 |
| specs/cli-agent-parity/validation/gui-436cc739/44-claude-resume-draft.jpeg | 100783 | aa7e6836dae7e25855adae104a9480677d7cff3709b34b79ca9e56a9c200126f |
| specs/cli-agent-parity/validation/gui-436cc739/45-claude-history-restarted.jpeg | 111265 | 8efb290e9fc5d267936dce5b6f210b5df523489c998babfdc20d222009aabff0 |
| specs/cli-agent-parity/validation/gui-436cc739/46-claude-history-connected-awaiting-explicit-send.jpeg | 112758 | 4b2687d31f0cb25ebb2884239cd5489d3525bb85a72e65763f3f3ca2a7302ca1 |
| specs/cli-agent-parity/validation/gui-436cc739/47-claude-history-explicit-input-sent.jpeg | 114452 | 0f833b73f503b7a28dd96ea7bf3175378c368a09dcd2d44c45a5f856c3f9fc89 |
| specs/cli-agent-parity/validation/gui-436cc739/48-claude-history-continue-sql-proof.json | 2016 | 6ac8121d25c4a0e45892541b9ddcd563b8f9e272a5e20f4a703ae1090f85933c |
| specs/cli-agent-parity/validation/gui-436cc739/49-claude-history-resume-result.jpeg | 91754 | 188e259a5bb4bbbaea226603f704e7a684f1ca7306e0eff127fc55f5002fc38d |
| specs/cli-agent-parity/validation/gui-436cc739/50-claude-task-restored-after-app-restart.jpeg | 126850 | 4249966432f6bd377254ff0e04a3cd7ca471be6fda90f99a6c58e11fea5821ba |
| specs/cli-agent-parity/validation/gui-436cc739/51-claude-restart-no-replay-sql-proof.json | 2182 | 36bc8fd221cefd19b86345d7160325a7178dda3f8255812715b4a8726d6b18dc |
| specs/cli-agent-parity/validation/gui-436cc739/52-claude-restored-history-controls.jpeg | 91236 | 5379186ddb9dc199fc1cd5d9defe2be87de8f6ba3e68ccc56f4621b52067de62 |
| specs/cli-agent-parity/validation/gui-436cc739/53-claude-restored-session-started-without-input.jpeg | 124394 | f9aa4a73f2c17f006b2a377b62c96ad09a69e26db71a956b691fbe89722372b1 |
| specs/cli-agent-parity/validation/gui-436cc739/54-claude-after-restart-new-input-draft.jpeg | 115101 | 501a6bcbca0712c810f185304d61040071b4dcb0e7ccacc773263ca1fce4880b |
| specs/cli-agent-parity/validation/gui-436cc739/55-claude-restarted-history-no-input-replay.json | 295 | 1a65441b983adc3d3a843a430e0a671303d41117d38a370d8788c00d8b81d600 |
| specs/cli-agent-parity/validation/gui-436cc739/56-claude-restart-explicit-input-sent.jpeg | 117223 | 13476c43573f5ea5360b7c0b2cce814faca106e8d787173edcaaac96fa88c542 |
| specs/cli-agent-parity/validation/gui-436cc739/57-claude-after-restart-final-result-sql-proof.json | 2280 | 86cd920878ad6a342f71c0d4ba0de07662767c1ac864d215f05454bb388a50ab |
| specs/cli-agent-parity/validation/gui-436cc739/58-claude-after-restart-final-result.jpeg | 92073 | 8f6910d9669117f6fa5baf329a9589292782afbba8823f2b3d6b85be4345c38e |
| specs/cli-agent-parity/validation/gui-436cc739/archive-audit.json | 68142 | 6b6c69b5cd625fb3a4bff99e02ccc990c9a5212e209d131d582aa824f26d60ca |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/00-preparation.json | 793 | a11bc3f4cd2cbe6c62d5e085f92f1b058eefc3470e8b2934f744997264d0aade |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/01-png-attached.jpeg | 103272 | 5950d4d7b56ffaff747498e4d3bdc4280077f364debf691fc62558a1bcebdc13 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/02-png-chinese-english-multiline-draft.jpeg | 117979 | f091536dc167d8bd4f4fbd7fce3e4d36e55bb7885c5f580e4982806ba50b16e5 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/03-png-task-start-request.jpeg | 110913 | 51b465140c93844ba936cb08230f6f21f4ba6cfee4d43fa6450359bc700cd4f5 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/04-png-native-colors-result.jpeg | 87288 | c7b2b56381ea3e39b75f529168c92f20b521988c2ebf7a1267188c032bedc9a9 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/05-png-first-sqlite-result.json | 2639 | aa4628d9a52f3631cd7014a0c972eb92eef1d27e47d6258433ae23fd20c855c4 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/06-png-disconnect-request.jpeg | 86477 | 62e26e0d0b45792d1951d6c1c29e8de81e213746ab56d083f677a57078c50d0a |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/07-png-history-connection-request.jpeg | 97058 | 8186a88619f9376bdb12c3c940057acdbcd031354c1d2959c588f4d8d1144c94 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/08-png-history-ready-no-replay.json | 2408 | 6c98e730ee3f6a66d34ceec86d6b9315544680fddc355b10422044789f1e3ba7 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/09-png-history-new-text-draft.jpeg | 120511 | 72ea8867be166eaa24a7d908f136da4668b892adb5f2ee21cdc6c8f2d223549c |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/10-png-history-new-text-send.jpeg | 119302 | bfb5a10c1be3af2431cfce2e7abcdbb2a745b07e0806adaaedd4c4b71122fa3c |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/11-png-history-new-text-native-ack.json | 2937 | 4ba6b3eead9977f7e36ddcba4d865e66323e9caacff625829c4c96726e123dbd |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/12-png-history-colors-result.json | 3851 | ee423c2fca5098ff94dbb23a2bc265b95e2439dbf6dd9841bc200be876f433a7 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/13-png-history-colors-visible.jpeg | 87757 | 8ee7b812d3abaf4a7c515cc5fc4b4ed18b2ec5000cc920e74fb45fd3cb902527 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/14-processes-before-normal-app-exit.json | 689 | 29f83d4ca70ee8f9fe6256449c070d2cccd4ae8c36b6fad3ebf4ebc09bcb11af |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/15-normal-app-restart-preparation.json | 892 | 0730bf5c64ee0c7000313fc2d63a243d9d21f283d2ad91b4b334bae8deec3407 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/16-restored-task-list.jpeg | 126205 | 3cd8d9515696559153535777ca30fafca6ea954f8e250a0d72f3a85bf33ca2c1 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/17-recovered-colors-result.jpeg | 157565 | b0e93ee1cc0c9f4ddcde7612be9b1c6e7918f0e756bfda9888f241ff6feb6728 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/18-restart-attachment-and-no-replay.json | 3851 | 29d97dbb4b08d03b55a1fb51b6793313f7b1c5eb41fa694606c913cf583d3fea |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/19-restart-history-control.jpeg | 87041 | 7d8659f8680bef526f8f617ab2b68cd376ec5c9ec813328e4d39be1777b9be61 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/20-restart-explicit-history-request.jpeg | 96236 | 029284f54ddf8d6144011b6837d83fc8f4e50c7f8778d48b1c9e87169b0c68d0 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/21-restart-history-no-input-replay.json | 3613 | ae803c737787ff5f25425ce9740672a31cc7087d9d133f57bdd591c3d77ba70e |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/22-restart-new-text-draft.jpeg | 122465 | c38dbb56e3194a91144300577feeba82d9e0ef58d46884d3ab2c58e383cb104f |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/23-restart-new-text-send.jpeg | 119691 | fd264c348cd37ae3200dfacfc712411c6e1982dfac752dcdbfccc2c1d14202d4 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/24-restart-new-text-native-ack.json | 4142 | 98be2a39cec2d5907da21c62db6d38420910885a424540831d375de00796d350 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/25-restart-image-memory-result.json | 5058 | 5c59fc71239506f060a6da582d7ce6ccde7e90a2e3ede9b27eb0fa8cd06b6747 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/26-restart-image-memory-visible.jpeg | 87710 | 7d81e2e7e93d0f15bbdb17e767ca79e409ed2e2e86eca2532334175607a57094 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/27-final-disconnect-request.jpeg | 87017 | d6e3ede292153e625e748ff0e54ae656dcfb2aaeb5dc50cad26e4e8da449e4a8 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/28-final-result-after-normal-app-exit.json | 5058 | eeecaab33cacfda9df03ec2a8d3bb6ac92ed540f8a4fcb53c8535d6ba3bb5c7c |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/29-picker-alias-cleanup.json | 307 | 1230d1d64ba1e6e52963d6d70a8b9f7be5b0f0f73fc76a7739a3af45f10d026c |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/archive-audit.json | 41318 | 69c6b45da86565a6eafd48510860e90d20210e84db31518e0ece4ca6aeda0474 |
| specs/cli-agent-parity/validation/gui-claude-png-436cc739/fixture-colors.png | 248 | 6e6f0c4608c8d97790950c80e966369767f0d6c7f043b65a16e8ec206000695d |
| specs/cli-agent-parity/validation/gui-grok-436cc739/00-preparation1-cleanup-after-socket-length-failure.json | 379 | e9ff3da71fe9bf25e0963765e3a094b98677fb4743262f60aeba8d105bb00c99 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/00-preparation2-official-private-environment.json | 1268 | aeb3f4b4355ec2b724fc5946af7cc3dd0e850bcdbbe7e7ee8e95e69f1b9a6490 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/01-manager-official-environment.jpeg | 124631 | d231ab25856d23080c33cc86b706db22f83ca8a38c7741c33d73acd5f1b3891f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/02-grok-project-and-inherit-policy.jpeg | 117192 | 16cef22b7ffafa6347d4d0b48f5a620d09aab6a23f4c18f146c993a2e40fc104 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/03-grok-first-chinese-english-multiline-draft.jpeg | 104150 | 2dd0f7d067a3b31670ad3ce6ba2f96abbad2fec896cd030affdae0cc5a95c55a |
| specs/cli-agent-parity/validation/gui-grok-436cc739/04-grok-first-task-started.jpeg | 107012 | 48fd356c8e0cedda150bd296a00b884dcd82d666978b3eed12da59f95173be51 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/05-grok-first-result-sql-proof.json | 810 | d14c1ead8a21fd4400a6a15776689e0930009d13c410e69bbb9d6c193d73c259 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/06-grok-first-result-visible.jpeg | 82661 | 0cedb1e8a3536c1940965c637329d2bd97818f70a314b1d4b7279593e1b805ed |
| specs/cli-agent-parity/validation/gui-grok-436cc739/07-grok-second-multiline-draft.jpeg | 91361 | 5ba801dd84ecfbf1c6b365ad1a279808f85a0310b108971d483890f0ee3d133d |
| specs/cli-agent-parity/validation/gui-grok-436cc739/08-grok-second-input-sent.jpeg | 123100 | f99b7d29b21ef891d8e414ef83415513c063242808e133f4a6ab8ce5ef0662e9 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/09-grok-second-result-sql-proof.json | 3194 | 4779f29c1f9c68bab238f16fade68901fa45904a0d8d54fed7e0aa631a80171a |
| specs/cli-agent-parity/validation/gui-grok-436cc739/10-grok-second-result-visible.jpeg | 83365 | 75a75752f5ec595d46136ffd07c100fe4f0bad728a7becd2631694fa5dbca207 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/11-approval-fixtures-before-input.json | 431 | ad6004198bb5f61bd9a3f174db65484f8154d1bf99fe6a3d4f4b65b34c0fa392 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/12-grok-write-allow-draft.jpeg | 103149 | fbbb9d5b4efbff41a0f270ed8d9476d431b78b250f6fe8dbf2e01dc5c3b0643f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/13-grok-write-input-sent.jpeg | 116468 | 9c9adbe21cb1c53364ca09998fe8b3ef5b0b8e62bcf56a788ab375b5bbe7cb8c |
| specs/cli-agent-parity/validation/gui-grok-436cc739/14-grok-before-allow-sql-proof.json | 3489 | 21e65675d8f204173dc2aad38e559cf22da138c30e974c1aca7a15ccad8a17b2 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/15-grok-native-approval-options-and-path.jpeg | 103343 | 14a0e55e6c76db63f64694204549316bf2226268f8b50b002a92fb50ba67f38d |
| specs/cli-agent-parity/validation/gui-grok-436cc739/16-grok-allow-once-controls.jpeg | 91229 | ccd6de574948f67063be87d067fe04754dcae4fde43c92f70a03b9802967f335 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/17-grok-allow-once-clicked.jpeg | 116583 | efcc6fa1ee5f3b5f9dd54e8142ff424f9998be5a10158b09ab77000a2097e87f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/18-grok-after-allow-sql-proof.json | 4287 | da0184c8f1716d31846c836839ae8394db8b6b0a35f80d94817bcc6f4661e2ce |
| specs/cli-agent-parity/validation/gui-grok-436cc739/19-grok-allow-final-result-visible.jpeg | 85083 | 1af9e78df3cf4a8d8fb83e5d4445ab292b0e73c7039e65f720f877dd5b2c3216 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/20-grok-deny-write-draft.jpeg | 101684 | 1801fe22f7623824198d07539b09f44837e57c376a040221d2dab1818362aab6 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/21-grok-deny-input-sent.jpeg | 111795 | 13da65edfd6a5b1e902e966b302644b3109374050088036299a8c51035464607 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/22-grok-before-deny-sql-proof.json | 4429 | 0aab2b0b02a5dac6e6a37f51c8e7c5d96a2efa619ef8f3171d7b01ae32808159 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/23-grok-native-deny-options-and-path.jpeg | 101673 | 061bcd8ccb180c5e905d5ec7377d752470362e476f56a4d141f5591ac5690234 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/24-grok-deny-once-controls.jpeg | 90949 | 279235e26ab02c6c84f61d6ee0418bb34f5eb2c09c285d4d644dfa253d2a216a |
| specs/cli-agent-parity/validation/gui-grok-436cc739/25-grok-deny-once-clicked.jpeg | 116470 | dd54d70f43352095c8f8007739b77c054d40ddea3fa8fa0d32fe207711b4015f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/26-grok-after-deny-sql-proof.json | 5139 | f57e50724e694b4c780dfe5c82df1ad8c80a6d591cc61f3555fce219179bd80b |
| specs/cli-agent-parity/validation/gui-grok-436cc739/27-grok-denied-native-cancelled-result.jpeg | 83527 | 9923a495ed83c5ba2968765af747539b8cdb0d3e63d01f03f4e51b5c5eb48748 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/28-grok-current-turn-cancel-draft.jpeg | 100214 | 0c37bc10a7297803441506f83e91cc8bdb809c3cedecf994683eebd52b304136 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/29-grok-cancel-pending-before-queue.json | 7109 | 71bb5b2716d051dfd5d57c13a12e4169750a706c9867f0e54b7cc71bdee5798b |
| specs/cli-agent-parity/validation/gui-grok-436cc739/30-grok-cancel-native-approval.jpeg | 117097 | 79397077851d9196c1965cbb243aa86383a1cc4daee4534e38e64d36da93f105 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/31-grok-cancel-native-write-payload.jpeg | 81561 | b137c3e60d12ac9c0d9ac07cb96575af8b9292d2986d854167183d03c34682f2 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/32-grok-queued-instruction-draft.jpeg | 102385 | 9bff1e5a69d40aa24f5a61f2ccc25d9f5b02172a9c522a0866af97ba5474518f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/33-grok-instruction-queued.jpeg | 102078 | 01757b90df4ef72c9d7db034e6104ef219cf019298f63b0691426bb4c008ae37 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/34-grok-queued-message-before-cancel.json | 7317 | 1897510f2c9315ba1208e40ff86cf6085bf267d6a28c558a667266b251efadc0 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/35-grok-cancel-clicked.jpeg | 104383 | 328691a82567218f585535edfd6afd5ef25cb60be161118ac40f1b5b54ba5676 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/36-grok-cancel-and-appended-result.json | 9205 | 3de9b7077a21d3095b7a07d6cf4e6ef2effbaa1b3bc9b3d5dafe176302fe2c73 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/37-grok-append-final-result.jpeg | 91969 | a9876e0e776a0998f9adb4a860b16bfe9073b6a81412a90a48cf595733da9e80 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/38-grok-disconnect-after-appended-result.jpeg | 90531 | e3ddd84acafd2d791fd4d05a271a256dfa3569e2e43a3f10e528dd91d3b167e4 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/39-grok-history-process-start-no-input.jpeg | 117178 | 16ee391a84b6a5535bf8e27f9d0aae10856a9b23a5293bb1d64f0528d018b34d |
| specs/cli-agent-parity/validation/gui-grok-436cc739/40-grok-history-connection-before-new-message.json | 8800 | b6e93d4fd77d0ab73361cbcca756d0637a5ce00f7e04ca5b680344f2f520964c |
| specs/cli-agent-parity/validation/gui-grok-436cc739/41-grok-history-new-instruction-draft.jpeg | 112513 | 2fa2db1e14e9156c5c522e1a8c29aa7abc49e4a515f9fb760aac2d5c23c1a820 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/42-grok-history-new-input-sent.jpeg | 111411 | c0d9ca59693b56a5a79bf85d6209fa08cd7034c9de627fa9239da179887255c8 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/44-grok-history-send-rechecked.jpeg | 111564 | 1e9cd04d397ff1e16ba0067b0e4677499cdfda248db0ea105486e87cde3d54d5 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/45-grok-history-send-validation-state.jpeg | 116583 | 1c90df5b7e9a47dd60782826396edac48fb8e812680dd854252baa1485c3681a |
| specs/cli-agent-parity/validation/gui-grok-436cc739/46-grok-history-send-after-editor-focus.jpeg | 125021 | f9973e205e5007993ac7e45db73eb4230970f638fe7f289c207e8b8c9b11bf5e |
| specs/cli-agent-parity/validation/gui-grok-436cc739/47-grok-history-active-progress.jpeg | 124948 | 043e6a31fef17683b51d932aa8f72585b0a1010714b777bd829728cb41e9db91 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/48-grok-processes-before-normal-app-exit.json | 693 | 9b5ccbae70c909cd1588a26b3b35b7be564b5a782adb065950f2b011b0a5ca00 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/49-grok-history-no-terminal-before-app-exit.json | 9553 | 77f4a75df879b3a5b910cf80c6b622d1659da1ccaca6bc4d9ead671514c43603 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/49-grok-normal-app-restart-preparation.json | 907 | 13ce94065c891ce4a43d35a7c33d04c581428d15278662c5e8519283322b00e5 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/50-grok-task-after-normal-app-exit.json | 9553 | 184667b9f7424d865cbf4ca45a2ad3e176cb5f67c5d172831cb9c52e703d8c79 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/51-grok-normal-app-restart.jpeg | 48174 | fabdc30a109621554eda999e46fb5edad2c25cbec2c9ef6c447d336d2fa45461 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/52-grok-tasks-restored-after-app-restart.jpeg | 122808 | b2c3bebb7b8dd2583bb1020bfb2af9e415fcbb8758ae6c4549c7c178574f7123 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/53-grok-restored-task-disconnected-without-replay.json | 9563 | 8aa37fd312190b796b098fb925343084aae4819e09dfa29142f762d9d10e092d |
| specs/cli-agent-parity/validation/gui-grok-436cc739/54-grok-history-control-after-app-restart.jpeg | 100943 | edbf7f8f2e8bdef72056443c6f4cde156b44c6c45033c9a4552186e184988563 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/55-grok-history-process-after-app-restart-no-input.jpeg | 116529 | 73d074dee549d5ebc643b80fe2cfbdadd4a93e9736cb591bde3d63d3b4db28ce |
| specs/cli-agent-parity/validation/gui-grok-436cc739/56-grok-restart-history-before-new-message.json | 9595 | 4016951c3e4634ed996bab821270ed25a0e941914ec2b588b00a2b5b83b573a2 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/57-grok-restart-new-instruction-draft.jpeg | 116985 | 988c8194f9d5980879cbafd8fa938587804f0614f757eb77deb493876a0a7766 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/58-grok-restart-new-input-send-observation.jpeg | 128750 | f73d0da98efa632baeef3f46a5aa06aa44f7f35982bbb8a05479ed81501f2a34 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/59-grok-restart-new-input-result.json | 11388 | 68204d3068d27c78653af0b5de5ae5d5dd1a38fe6635fc5dcf1c1422044bbd5f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/60-grok-restart-no-terminal-at-network-scope-end.json | 10348 | 495141e4828f3540d876642a26042f050103029a199a0c669244712ca8ab92c8 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/61-grok-network-scope2-final.json | 53305 | 8475280d217e5acf56a2966320eaaf009c2e3f2f3fae1777a1bfcf15b43727d1 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/62-grok-network-scope3-preparation.json | 684 | 976cf65e2112e24b3bd50b073eec53b1d804bc0ebf7bbca5577c926bfe737dac |
| specs/cli-agent-parity/validation/gui-grok-436cc739/63-grok-network-scope3-fixture-failure-cleanup.json | 453 | cfb4bafa39af1ce3b2b07da9041eee7159260d11639a5466dc99e3659b14cc09 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/64-grok-network-scope4-preparation.json | 684 | fdc5a2bd994045a6b13762af87237d84384c8e5d11188ba6fbfbee4e56bf0983 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/65-grok-restart-complete-result.jpeg | 120150 | 14a84bb54395bb402581f39903430ac18587082b8bb211984e7ec28b7ad8fc77 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/66-grok-restart-full-markers-visible.jpeg | 83929 | 1e71fa1d5230d2bd1d839ed30af8daea7fdefaf1e658c5991dcabcf4e8e2a50f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/67-grok-disconnect-before-separate-history-retest.jpeg | 83735 | 9408a2b1bd9e9c2e764e227d30d69a45394a0d8ad33489ed376d4231e4d70d3f |
| specs/cli-agent-parity/validation/gui-grok-436cc739/68-grok-history-nine-request.jpeg | 103540 | d783ae721cdfe730e3669eddbf1e9a3f42cc05e88fd7696d81e1de50148cd373 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/69-grok-history-nine-ready-no-replay.json | 10917 | f8479ec2ddbd65b3928faf0275bd6d7bc480f5bed48d7949cfbccae45c5e608b |
| specs/cli-agent-parity/validation/gui-grok-436cc739/70-grok-history-nine-new-draft.jpeg | 119502 | 48aadc3c87cbc2a0ed430868d77c550656eed60ab9641e1aa669636e66d5979c |
| specs/cli-agent-parity/validation/gui-grok-436cc739/71-grok-history-nine-send.jpeg | 130631 | 84f2f72970dad98fe60fe642636b2a59459b8342d5ae7232df4d447cb980aa3b |
| specs/cli-agent-parity/validation/gui-grok-436cc739/72-grok-history-nine-native-ack.json | 11670 | da2759938e153888062004ab709af6feddf11136c3519446458d606638739bbb |
| specs/cli-agent-parity/validation/gui-grok-436cc739/73-grok-history-nine-result.json | 12708 | 924351f458c47aeaf468988d4bb7dd456dcc282a2be26125f1659c78099f287c |
| specs/cli-agent-parity/validation/gui-grok-436cc739/74-grok-history-nine-result.jpeg | 83862 | 437dac8a71885de4c0f156906de9b1ffcae64997e09cc41f4bfac49d12b43106 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/75-grok-final-disconnect-request.jpeg | 83749 | 8a99cd77fd34753803e15e2db352f7705f6b6e86e2ff49f55a1fde489ec13d63 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/76-grok-result-after-normal-app-exit.json | 12708 | 12e53e42f618a174c79b0422c851a309529ce6d67876fb4deaea42e4b99dce8a |
| specs/cli-agent-parity/validation/gui-grok-436cc739/77-grok-network-scope4-final.json | 16245 | 278176adf6186e3d5eaee8cbf3b46d0d32ff222a3698469916129988ae72adf6 |
| specs/cli-agent-parity/validation/gui-grok-436cc739/archive-audit.json | 90477 | 40f20802130984d57064c910f7e5c76216e20c1714580a47e418e9976fcce286 |
| specs/cli-agent-parity/validation/macos-official-26-archive-audit.json | 7580 | d893a842b0f8ddd45b4b949e5f5dc67916379e49d4ec2de2de95d0e348d9e720 |
| specs/cli-agent-parity/validation/macos-official-26-bundle-archive-audit.json | 7325 | 35bb5f011f02460c3ace2588edae65457e2758eed6066decb33c5e7e3dd0ab75 |
| specs/cli-agent-parity/validation/macos-official-26-bundle.json | 21178 | 7ac15a5307f44e30d94d45af5a0098cc8d0145dc565278e4d4a3ad163255e9a9 |
| specs/cli-agent-parity/validation/macos-official-26-gates.json | 2286 | 34bc0f5ec4135211879504f6dd1245fa7bf8edb8ea267b0854e2d84be43b715c |
| specs/cli-agent-parity/validation/macos-official-26-inputs.json | 18933 | 3ff03385c6f9635c1e60304d3220c49cdb281f314e73735a7f90a3f3298875c9 |
| specs/cli-agent-parity/validation/macos-official-26-native-inputs-precheck.json | 3541 | 5bce6e8a4412d0dc7107ab7c9fb755af177b6ab7d0fc7d780cd670c2e4c7f163 |
| specs/cli-agent-parity/validation/macos-official-26-python.json | 4558 | c788a47d8f0f1159b6b037e627537758db0129df46e9c364d857e37239ab68b7 |
| specs/cli-agent-parity/validation/macos-official-26-signature.json | 674 | 16a9edf3865e7f3bf96461c5fdc1bfc45521ecb7be9f2d6751883ac8084cd885 |
| specs/cli-agent-parity/validation/macos-official-27-archive-audit.json | 9634 | 01c7dc502e6e4a8cc542a2a920bead7d002953819e8966137c8999138063571c |
| specs/cli-agent-parity/validation/macos-official-27-gates.json | 2966 | a09e9e383678f549757bc8bedb8f4cc5b8702a16a124d4227d5bf9b857a8c737 |
| specs/cli-agent-parity/validation/macos-official-27-inputs.json | 19745 | 730a9aa633d9565eed15bc85ec4bfdbff45377e411f97ab6a23544c6326aaa8e |
| specs/cli-agent-parity/validation/macos-official-27-python.json | 5809 | 3cedd4ba7f3a3fbc08bdca629b7e2923c92c18c862399509fdfe8681e3a43bc9 |
| specs/cli-agent-parity/validation/macos-official-28-archive-audit.json | 10503 | 6dde01dfe40737e762bf5b65f33720a9b32cce80f37cb6edeb1503485f45ea6b |
| specs/cli-agent-parity/validation/macos-official-28-gates.json | 3623 | dba2390698d390639229d3069edb784604f27b8fe06c99ee726393ad4980dfe1 |
| specs/cli-agent-parity/validation/macos-official-28-inputs.json | 19885 | 7bfe731c3a40207d8d428702deac95a8fa4fd9d6b603312c8ef4cb0310f6acd7 |
| specs/cli-agent-parity/validation/macos-official-28-python.json | 5807 | 6feaea896382d6581ced7c7e2f57dddfccc7d09ec553d618aef22e03fc5609dc |
| specs/cli-agent-parity/validation/macos-official-29-archive-audit.json | 11879 | 053b7a2077ba87d4687d6020bd1466eb6da8dd6e75d816386f2fc1a79c28c9cf |
| specs/cli-agent-parity/validation/macos-official-29-gates.json | 4535 | 89d068ee6769d32b8c34c07fbe2700b6535a49d2a114093d8031024ee073899a |
| specs/cli-agent-parity/validation/macos-official-29-inputs.json | 20646 | 4e45ec701b9b98cc9abe7cc7048a919648f9589b594af5d83e7277bae5fd700e |
| specs/cli-agent-parity/validation/macos-official-29-python.json | 5808 | e2462baa35c26918868e2512a1f449306800ae553db00756ae62f1dbfb486e57 |
| specs/cli-agent-parity/validation/runner-availability-436cc739-ci12.json | 957 | 34f3bc1daae9edda184930d2215a1ae737f373d186926ef719a1a819a05e6628 |
| specs/cli-agent-parity/validation/source28-change-review.json | 56535 | b931f252ef110103c07dba00a1161489731b3640be2a36adcad8a7d754d0c8dd |
| specs/cli-agent-parity/validation/source29-actual-commit-identity.json | 912 | b4276c24044f2ca73cdd754e8b81e6ce1bde82a8f9bf1b94fb88f8bf47895e1d |
| specs/cli-agent-parity/validation/source30-grok-plugin-offline.json | 1970 | a0613d8eb56e89aca06b343bcc4aefc9d3d3a1b6834b4dd44631670725f6aef1 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/followup-audit.json | 5923 | 012247af7e8f0ee4d3858f885158406068827c21f88afb136a93c7afe1a0237b |
| specs/cli-agent-parity/validation/thirteenth-e6873e/followup-jobs.json | 69126 | 9087855bf97b6828c1ab6eac68878827d1c829b86922b89e069ba112b9b35069 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/followup-run.json | 3872 | 24211709bf947c31b48e40ef90cd542c7d174733b543ff98e022ac1dab7cf6f4 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/initial-audit.json | 7756 | 4eb0c673a8d248c035be231351251ee11777414a935a181d069e8b12b89f4a3d |
| specs/cli-agent-parity/validation/thirteenth-e6873e/initial-jobs.json | 65637 | a272d41619c03fb20b316f779ada47df48be60b5582d64f4fdb30a41a1e15e87 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/initial-run.json | 4481 | bb93b8bda67307ca23ccf456bc06524ce6f2d15496644466bed6c5a3593c634e |
| specs/cli-agent-parity/validation/thirteenth-e6873e/runner-availability.json | 27687 | 8d6715f4281149cb1fb8b23d77d632a7486ce4d11e93477f872af87c93746d54 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/third-window-audit.json | 14700 | 2b12fd2ea8d709330c19741acea547c90a928a7441cc812c1308ada7dae32144 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/third-window-jobs.json | 25028 | 32f4793efea784017190daa9497ec573ce0fedffc7a409fabc3f7dc12a79e8b8 |
| specs/cli-agent-parity/validation/thirteenth-e6873e/third-window-run.json | 2100 | 125b3288dc5bb5559b39dd911390c4ff701e4c95bf14e97d0fed697c87a1731c |
| specs/cli-agent-parity/validation/twelfth-436cc7/final-audit.json | 9299 | 85f201cf10633e5f4876611bcb77e4f5a0075668629e1ba5eef190a49674268d |
| specs/cli-agent-parity/validation/twelfth-436cc7/final-jobs.json | 80888 | 01d42773ea3e4ef6950ff6306928e5a4a408431e668f49550f109182dab2f61a |
| specs/cli-agent-parity/validation/twelfth-436cc7/final-run.json | 5722 | 80fe19d2c8b0412ff94ceccad68af90bafa0f64552c7140da0bb6ff166116901 |
| specs/cli-agent-parity/validation/twelfth-436cc7/initial-audit.json | 8153 | 22edee3a66e3d6b73981d065caa909d6d6661909fb644467955c9bfa858288fa |
| specs/cli-agent-parity/validation/twelfth-436cc7/initial-jobs.json | 75457 | a4a3c223d8ec7ed9e3e1ea7aa5886f7207a39dd51d146ce016f60df04fed0132 |
| specs/cli-agent-parity/validation/twelfth-436cc7/initial-run.json | 5795 | 46319e10f74b4860e351bdd0e49bec819d3760e7ee5acac462cfaba14dba67c2 |
