# Grok 0.1.1 配方迁移候选源码复核

已知 0.1.0 配方的升级识别与受控旧备份路径在源码上连通，未发现确定的 import/include 路径编译错误。**跨版本并发状态守卫和未来夹具字节不变证据仍有缺口**，详见下面两项。本轮只有只读源码审查，不把定义的 Rust 测试、ignored 六阶段或结构分析标为已通过。

审查对象只含明确授权的五个文件，输入最终 SHA 与开始读取时相同。没有读其他源码、runner、认证、数据库、原始日志、冻结树或 target；没有 Git、Cargo、CLI、GUI、网络或模型操作。source31 冻结仍是旧插件 0.1.0，不因本候选转成 source31 checked。统一 source32 门禁与新真实原生验证待根执行。

## 已确定的源码行为

- grok.rs:23 与新 manifest 均为 0.1.1；旧 README 夹具为真实 3153 字节，SHA256 2db65e9c54c35725ba1164edb39daf9645bfdf7f1d3cd2689b8e338853c2e8b7，和 grok.rs:26–27 固定值一致。
- grok.rs:627–649 先要求完整受控四文件树。旧 manifest 仅替换 version 后按 JSON 字段等值比较，其余字段仍匹配当前已知 manifest；hooks 两文件仍逐字节匹配。README 附加旧摘要例外仅用于 version=0.1.0，额外文件、目录、symlink 和 Unix 多硬链接拒绝。旧 README 配在其他版本、被追加内容或与修改 hooks 配套时不能进入此例外。
- 同一校验还允许较旧 version 使用当前完整 README/同配方内容；本例外没有将所有较旧版本收窄为仅 0.1.0。四个新负例只能证明旧 README 例外的范围，不等于所有未知较旧版本都拒绝。
- grok.rs:808–823 写入新的 0.1.1 目录；已有当前目录被改动时先校验失败，不覆盖；0.1.0 路径未被重写。208–212 校验应用拥有的旧来源及旧安装树，再复制四文件到 recovery；264 的恢复安装与278–281 的旧版本树识别能够接受旧 README 摘要，恢复来源仍可位于应用 root/recovery 下。
- 初始禁用在171拒绝；同版本修复688、713–725重读 enabled/registry/tree，734–758 回退不会覆盖无法归属本次修复的并发文件。未知旧配方/来源或额外文件在卸载前拒绝，明确外部恢复来源在239–260拒绝。此结论不扩展为跨版本全过程并发保护。
- grok_tests.rs:31–33 仅将派生安装夹具重定位并设为当前 PLUGIN_VERSION，没有改写其 include 的原始历史 JSON。本轮未获授权读取该历史 JSON，原生注册形状的现场匹配仍依赖已有证据和后续真实执行。

## P1：跨版本 await 之后缺少状态重读

apply 在 grok.rs:171 检查禁用，178、192 后仍继续使用起始结论，208–212 仅在卸载前做一次旧源/缓存校验。225、230 是原生 mutating await，234 可直接报告升级成功，没有同版本 repair 的713–725重读守卫。失败路径239只检查注册来源，263在 cleanup uninstall 之后才读 was_disabled。

可复现的最小设计是：在合成固定 CLI 的 --version 或 plugin validate 返回前，修改隔离 config 将此插件禁用；调用生产 update，并断言不会出现后续 uninstall/install，原 config/registry/source/cache 全字节保持。当前流程没有相应重查，因而不能保证新禁用意图被保留。另应覆盖安装失败期间缓存自定义内容或外部注册的变化，避免把始终相同 source_path 当作当前完整缓存仍由本操作拥有。

最小修复应在卸载前重读 disabled、注册与完整源/缓存快照，并在每个 native mutate await 后校准当前归属；未知变化拒绝操作与恢复。cleanup 前保留已经观察到的禁用意图，不恢复全局配置快照。原生命令执行过程中外部 CLI 变化的保证必须另用真实行为验证，本轮不凭一个新 boolean 声称已解决。

现有748–755只检查升级完成后预先禁用；776–790是同版本文件事务故障，不覆盖升级中禁用或旧版 install 失败原生回退。本项是静态守卫缺口，不宣称已发生真实原生失败。

## P2：未来 unchanged 标记缺少完整字节快照

grok_tests.rs:678创建旧源，706只重新 validate_expected_tree，710记录 legacy_source_unchanged=true。旧 manifest 校验 grok.rs:636–641 使用 JSON 语义等值；排版或字段序变化可通过。因此未来记录只能证明旧配方匹配，尚不能证明四文件原字节 unchanged。

创建后保存 plugin_tree(&legacy_source,false) 或逐文件 bytes/SHA，升级后比较全部字节，再写 true，即可补齐。85–98单测对 README 直接比较字节、对其余文件只做结构校验，也不能单独作为 manifest 原字节保持的证明。无需放宽成功判据。

## 编译与验收边界

顶层 Sha256/Digest 已引入，Sha256::digest(&Vec<u8>) 和现有 LowerHex 格式用法未见确定类型问题。测试从 super::* 引入 Path/PathBuf/Value/父模块 helper；macOS/Linux 的显式 sha2 import 与 glob 可共存，Windows 纯测试可经父模块取得无条件 import。include_str 的相对层级与已存在宏路径一致，新增旧 README 实际文件存在且 UTF-8 可读。真实编译仍待 Cargo，不以本审查代替。

四个新增纯 Rust 测试仅定义：完整旧配方及备份、修改旧 README、旧 README 放在其他版本、修改 hooks/额外文件。ignored 入口静态定义六阶段：当前生产 install、受控 native fixture 切旧版后生产 update、同版本 repair、disabled 拒绝、同版本文件事务 rollback、production update after rollback。其中 old→new正常升级由生产 manager.update；故障阶段明确 production_apply_call=false，**没有覆盖旧版本原生 install 失败恢复**。本轮未执行任何阶段。

六阶段产物相对历史五阶段发生变化；runner 的期望步数/步骤白名单不在本轮五域内，未读取，须根后续另核，不能猜其必然兼容或失败。未来新产物不得回填历史五阶段成功。

本轮没有修改 GUI/TUI 文案；现有本地化键保持，版本/协议值属于稳定配置边界。此审查不等于 i18n 门禁或双语布局完成，统一 source32 仍须必要检查。仅新增审查报告，**无需本地化变更**。未 stage/commit，不计三方能力、平台或 Goal 完成。

## 五个受审输入

| 精确路径 | 字节数 | SHA256 |
|---|---:|---|
| app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs | 31734 | 7ee545bda1ead0c8f628c2a3daf4332b3cc891850fa85aa7aed8f8a998880db1 |
| app/src/terminal/cli_agent_sessions/plugin_manager/grok_tests.rs | 32472 | d7c7f10136e17f81f113a06fa94e0b6993eaffb0851da7223a4e6c0523724cc6 |
| app/assets/bundled/cli-agent-plugins/grok/.grok-plugin/plugin.json | 148 | dce0d26f77b0e5a25da21418388d5f2a1d6d451da489e155b44699594f8cbdf3 |
| app/assets/bundled/cli-agent-plugins/grok/README.md | 3225 | 13a5451348ef9278f243c18ea3f8c8af714cebfe49e2eb0979263de841531167 |
| specs/cli-agent-parity/fixtures/grok-plugin-0.1.0-readme.md | 3153 | 2db65e9c54c35725ba1164edb39daf9645bfdf7f1d3cd2689b8e338853c2e8b7 |
