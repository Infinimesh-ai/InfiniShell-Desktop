# source23 干净提交本地门禁独立归档

包含实际修改的提交 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 在隔离验证树通过本地 check、i18n、受影响模块及 Python 门禁。原报告注明工作树干净。本记录只证明这份提交的所列本地门禁，不能计作 Goal 完成、全工作区或完整三方 CLI 验收。

## 输入与独立核对

- 原始输入清单 SHA-256：`89596f95bca52dd9b446670c6319fd6ca644f922852f411c6d9ce9e356125e27`，共 93 个源码／资源文件。
- 冻结树的 93 个文件连续两次重算 SHA，与清单均完全一致，每次 0 个差异。`frozen_reads_done=true`，此后不再读取冻结树。
- 原报告声明这 93 份源码／资源与 source22 逐字节相同，前置切换只涉及保存清单范围，且门禁后源码保持不变。这些前置和历史一致性事实来自根代理报告；本归档未运行 Git 或重新读取 source22 档案作跨快照比较。
- gates 和 Python 报告均关联相同清单与提交，且标明工作树不脏。三份原始 JSON 按原字节复制，复制后再次核对源字节。
- 根主工作区后续修复不在本提交的验收范围，本归档没有读取该主工作区源码。

## 本地门禁

| 门禁 | 原报告耗时 | 独立日志摘要 |
|---|---:|---|
| `cargo check -p warp` | 63.578 秒 | exit 0，dev profile 完成 |
| `cargo test -p warp --lib i18n::tests` | 125.829 秒 | 11 通过、0 失败、6783 筛除；内部 4.67 秒 |
| 受影响模块 nextest | 18.094 秒 | 1301 通过、5493 跳过；内部 15.306 秒 |
| Python 回归 | 4.888 秒 | 14 组、363 项、14 个 OK，各组 exit 0 |

focused 日志逐行独立计数 1301 个 PASS，其中 `ai::agent_sdk::driver::harness::` 有 31 个 PASS。Python 各组计数依序为 11、7、56、18、25、25、20、10、13、27、19、85、20、27。

check 保留 `agent_events` 的两条 unused-import 警告与一条 Cargo 汇总警告行。它们没有导致 check 失败；本归档没有修改源码或隐藏警告。

测试二进制身份 `f01f049248141ab288484c3b776e11f5e609e8d7d5a7667d76487c8f7fbdf33d` 只来自根报告字段，本归档未读取 target 或二进制。

## 尚未满足的验收

本归档完成时没有 source23 main 构建、严格签名、真实 CLI 新建／审批／取消／恢复全链、GUI、完整 SSH/tmux 或全工作区测试证据。任何仍在运行的构建和 CI 均不计为通过；本归档没有查询网络或独立核实平台状态，也不将提交身份一致等同于平台验收通过。后续结果必须单独归档。

英文与简体中文资源属于这 93 个已核对输入，i18n 单元门禁通过；本轮真实双语 GUI 布局尚未验证，不能沿用旧快照布局证据。本归档自身仅新增文档和原始报告复制，无需产品本地化变更。

## 原始档案与审计

- [输入清单](validation/macos-official-23-inputs.json)
- [Rust 门禁报告](validation/macos-official-23-gates.json)
- [Python 门禁报告](validation/macos-official-23-python.json)
- [独立归档审计](validation/macos-official-23-archive-audit.json)

明确授权的三报告、四日志与五份新档案均按固定六类凭据形状扫描，匹配数量为 0，不输出匹配正文。零匹配不保证识别所有凭据格式。日志仅核对 SHA 和摘要，不复制全文。

本归档未执行 Git、Cargo、真实 CLI、模型、网络或 GUI；未读取 auth、真实用户 HOME、target 或其他私有文件；未修改旧档案或根三份计划／矩阵／验证文档。

## 同提交构建、签名与原生输入预检补档

在首次归档后，根代理完成同一干净提交 `dec067d2d53fb65f11b37b66a72fa9e11e82cf04` 的调试 app 构建：exit 0，总耗时 254.795 秒。构建日志独立核对 SHA-256 为 `a6d4639a9269968253a39efd1082d4c9a5191607405a1646df624f722f52055e`，dev profile 完成行显示编译阶段 3 分 26 秒；日志 3 个 warning 前缀行、0 个 error 前缀行，不将编译阶段耗时误作整个打包耗时。报告标明构建前后工作树均干净。

构建报告内的完整 `source_manifest` 对象与已归档的 93 文件输入对象一致。本次没有重读冻结树或读取 target。worker 身份 `6bbd280f4e874784f7f33cd72048006ea16c7a3081207b435d7043a3f9256fde` 及下面的完整语言资源嵌入均只来自根报告字段，本归档没有重新检查二进制。

| 资源 | 字节数 | 原报告 SHA-256 | 根报告嵌入结果 |
|---|---:|---|---|
| 英文 `warp.ftl` | 374895 | `abc35589e8e30edb1f2c842105fb058a07d42d6baeae796dfe997a6a7f59f241` | 完整字节嵌入 true |
| 简体中文 `warp.ftl` | 361806 | `36541327e2148064b683bed14c2cdb5164c33752d991c080ed5d8fa04386179a` | 完整字节嵌入 true |

两项资源 SHA 与已归档输入中的对应路径一致。完整资源嵌入不证明真实 GUI 双语布局通过。

根代理的 `codesign --verify --deep --strict` 验证 exit 0，耗时 0.481798 秒，stdout/stderr 各 0 字节；签名报告关联同一输入清单 SHA。本次归档没有运行 codesign。此产物仍是调试 ad-hoc 构建，没有发布签名、公证、发布身份或公开发布验收。

原生输入预检报告关联同一提交、同一清单及 93 个验证输入；其中 lib/main 身份与门禁／构建报告字段一致，Claude/Grok CLI 文件哈希也只来自根报告。本次没有读取这些二进制、私有模型诊断或认证正文，也没有启动原生 CLI。预检是启动前条件记录，不包含 PNG3、SDK8 或待 Edit 取消的实际结果；本次不读取或归档仍在运行的原生证据，不计任何原生整链通过。

- [构建原始报告](validation/macos-official-23-bundle.json)
- [严格签名原始报告](validation/macos-official-23-signature.json)
- [原生输入预检原始报告](validation/macos-official-23-native-inputs-precheck.json)
- [补档独立审计](validation/macos-official-23-bundle-archive-audit.json)

三份新原始报告按原字节复制并再次核对源字节；新报告、明确授权的输入清单、构建日志与补档执行固定六类凭据形状扫描，匹配数量均为 0。首次归档的“尚未满足”段落记录当时状态，本补档仅更新已结束的构建、签名和输入预检；原生 CLI、GUI、同提交跨平台、全工作区、SSH/tmux 完整链与 Goal 整体仍未计为通过。
