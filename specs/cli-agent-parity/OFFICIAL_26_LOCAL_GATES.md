# source26 macOS 干净提交本地门禁

本次受测提交为 `436cc739234061c092cedd106432bbbfa3c2d645`，包含 source25 的实际修改。根原始输入报告记录验证树干净、111 路径清单与 source25 候选逐字节一致，输入 SHA-256 为 `3ff03385c6f9635c1e60304d3220c49cdb281f314e73735a7f90a3f3298875c9`。本独立归档核对报告关联和真实日志，没有读取源码树、执行 Git 或重算二进制；提交、源码干净与前后不变结论均保留根报告的来源边界。

| 门禁 | 独立核对的真实结果 | 墙钟秒数 | 日志内部秒数 |
| --- | --- | ---: | ---: |
| `cargo check -p warp` | 原报告退出 0，日志构建结束 | 61.973 | — |
| `cargo test -p warp --lib i18n::tests` | 11 PASS，0失败，6865 filtered out | 129.135 | 4.39 |
| 受影响模块 nextest | 1393 PASS，5483 skipped；逐条 PASS 计数一致 | 18.358 | 16.514 |
| Python 回归 | 15组、410项，15条 OK；各组计数与报告一致 | 5.091 | 各组分别记录 |

[输入清单](validation/macos-official-26-inputs.json)、[Cargo 门禁报告](validation/macos-official-26-gates.json)和[Python 门禁报告](validation/macos-official-26-python.json)均按原字节复制，三份记录关联同一清单与提交。Cargo/Python 原报告记录 `source_unchanged_after_gates=true`；本次没有独立读取111条实际源文件，不把报告一致性扩张为再次验证源码树。测试二进制 SHA-256 `bb019bc5e8e4c30582767e9a89685d46c515211c73b0b8914d04a5ee5b0e1cf1` 仅来自根报告，没有读取 target 或二进制。

[独立归档审计](validation/macos-official-26-archive-audit.json)记录三份 JSON、四份日志和新增档案的 SHA、报告关联、原字节一致性与六类凭据形状计数。四份日志只核对哈希和必要测试摘要，没有复制日志正文。原报告属性未改写，原始报告复制后再次核对；固定形状扫描零命中不保证识别所有秘密。

此专报只覆盖 source26 本地所列门禁。main 当时仍在独立构建，尚无本报告的构建／严格签名／原生 CLI／GUI／同提交平台完整验收结论；SSH／tmux、全工作区及三款 CLI 规定整链均不能记为通过，Goal 未完成。后续实际证据应另行归档，不回填本轮计数或旧 CI11 结果。

本次只新增开发验证档案，**无需本地化变更**。i18n 11 项通过不等于英文与简体中文布局检查；最终双语完整 GUI 布局仍待验。本次未运行 Cargo、CLI、模型、网络、Git 或 GUI，未读取冻结树、target、认证、模型日志或用户 HOME，没有修改主三文档及旧档案。

## source26 构建、严格签名与原生入口先验补档

随后同一实际提交 `436cc739234061c092cedd106432bbbfa3c2d645` 的 [main 构建报告](validation/macos-official-26-bundle.json)已完成，退出 0、墙钟 **241.004 秒**。独立核对构建日志 SHA-256 `729bec589c61544d38107c77142276a2c2d8679464e8254a17eb91aa95aaf3e5`，dev 构建结束及两份 bundle 完成标记均存在；不复制日志正文。构建报告内嵌清单按原JSON格式重构后的完整 SHA 与此前输入清单相同，111 路径、实际提交及 clean 字段一致；没有重读实际源文件。

main／监督 worker SHA-256 `d8102b9d02cd9f104d0d2c2098ca805c99abf6ac60441bd51b84356d3fd12de5` 来自根构建和入口先验报告。根报告记录英文资源 374895 字节／SHA `abc35589e8e30edb1f2c842105fb058a07d42d6baeae796dfe997a6a7f59f241`，简体中文资源 361806 字节／SHA `36541327e2148064b683bed14c2cdb5164c33752d991c080ed5d8fa04386179a`，两份完整原字节均在二进制找到；独立归档核对资源哈希与嵌入的输入清单匹配，未读取二进制。资源嵌入不证明双语布局通过。

[严格签名报告](validation/macos-official-26-signature.json)记录 `codesign --verify --deep --strict` 退出 0、耗时 **0.484430 秒**，stdout／stderr 均0字节且摘要为空字节的SHA。归档仅核对原报告和关联，不重复签名；本地调试 bundle 与严格校验不能视为发布签名、公证或公开发布。

[原生输入先验](validation/macos-official-26-native-inputs-precheck.json)绑定同一提交／clean111／输入SHA，四份受测产物身份匹配已记录报告。Claude待Edit取消、Grok SDK来源、Grok固定策略只读调查三个精确 ignored 入口的 `--exact --ignored --list` 均记录退出0、各列一次、`native_test_executed=false`。各入口 stderr 的258字节及摘要原样保留，不要求其为空、不归档其正文。该先验只证明身份和入口存在，没有执行原生测试；专用认证输入元数据核验也是根报告字段，独立归档没有读取认证材料。

三份新增JSON均原字节复制，原属性未回填；[构建补档独立审计](validation/macos-official-26-bundle-archive-audit.json)记录哈希、清单重构关联、日志摘要、有限追加以及六类形状扫描。本节保留前文“当时构建中”的历史状态，并补充随后真实完成的构建／签名／入口列表证据。正在运行的原生验收没有被读取或归档；本补档不计原生、GUI、同提交完整平台、SSH／tmux、全工作区或 Goal 完成。**无需本地化变更**，最新完整英文／简体中文GUI布局仍待验。
