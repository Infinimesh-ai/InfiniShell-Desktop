# official-11 本地门禁

official-11 的本地检查、受影响模块测试和离线运行器回归通过。独立归档核对了67个源文件的 SHA-256、四份日志的 SHA-256，以及日志中的真实测试计数，没有重新执行 Cargo、线上 CLI 或 GUI。

本轮基线为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`，工作区有既存改动，是中间脏快照。冻结输入覆盖 Grok 终态间隙排队、已确认接收但尚未 started 的真实失败、整条 FIFO 交付核验、本地化错误和历史诊断，以及测试专用 SDK 名称安全分类；SDK 与本地子任务生产门禁未开放。这不是包含本次修改的干净提交，也不计最终同提交跨平台验收。

## 已通过的本地检查

| 门禁 | 原始报告耗时 | 从日志核对的结果 |
| --- | --- | --- |
| `cargo check -p warp` | 58.241秒 | 退出码0，完成 dev profile |
| `cargo test -p warp --lib i18n::tests` | 148.298秒 | 11项通过、0项失败、0项忽略；另有6,713项未选中 |
| 受影响模块 `cargo nextest` | 18.148秒 | 1,194项运行并全部通过；另有5,530项 skipped |
| 8组 Python 离线运行器回归 | 合计3.776秒 | 181项通过，8组均显示 OK |

Python各组实际计数依次为：Claude适配器11、Claude权限profile 7、Claude协调器56、Claude批量取消18、Codex来源10、Grok适配器13、Grok官方适配器19、Grok SDK来源探针47。离线运行器测试不计真实 CLI 生命周期；未选中的 Rust 测试也不计已验证。

## 归档与独立核对

[冻结输入](validation/macos-official-11-inputs.json)、[Rust门禁](validation/macos-official-11-gates.json)和[Python门禁](validation/macos-official-11-python.json)均在凭据模式扫描通过后逐字节复制。冻结输入 SHA-256 为 `31619a72300d6a38e82ade3d40e2329039cee864e5c0ca38c29ceb38037e504a`；67条路径唯一，当前源文件散列全部匹配。各原始报告与日志散列、逐文件核对结果、测试计数及验收边界保存在[独立审计](validation/macos-official-11-archive-audit.json)。未复制原始日志。

Rust测试二进制 SHA-256 由原始门禁报告记录为 `8f0e7063c37e7f110d3c022ccb932ca9cc73514b1675592ed8570b59b84ef484`；归档任务没有重新读取二进制计算散列。前述本地门禁审计保留了写入时 main11 构建尚未纳入的历史状态；下文单独追加构建与签名结果。应用启动、双语布局、SSH/tmux、Linux、Windows以及真实 SDK 来源验证均不包含在本地门禁归档范围中，不能推定这些验收通过或 Goal 完成。

## 追加：source11 构建与严格签名

[构建原始报告](validation/macos-official-11-bundle.json)记录 `./script/run --dont-open --features local_cli_managed_tasks,rust-embed/debug-embed` 退出码0、耗时281.855秒。构建内的源 manifest 与上述67文件输入完整一致，manifest SHA-256 仍为 `31619a72300d6a38e82ade3d40e2329039cee864e5c0ca38c29ceb38037e504a`。构建前后工作区均有改动，基线仍为 `b3d8b0f11657b0ed1ba34035dda8d363eb52747f`；不是包含本次修改的干净提交。

主应用 SHA-256 由原始构建报告记录为 `8026e131aad657207690144d31e3e5c01b654039c4ed60eb1d5502dc9cb14fbb`。[严格签名报告](validation/macos-official-11-signature.json)中的 `codesign --verify --deep --strict --verbose=2` 退出码0、耗时0.485秒，确认 valid on disk 和 Designated Requirement；签名报告中的主应用散列及源 manifest 散列均与构建报告匹配。归档任务没有重跑 codesign，也没有读取大 main 重新计算散列。

| 完整资源 | 源文件字节数 | 独立核对的源 SHA-256 | 原始报告的二进制完整字节扫描 |
| --- | --- | --- | --- |
| `app/i18n/en/warp.ftl` | 374,280 | `9442a4b062945cf3f5fbc5160193f4a14ae715ae9d35b7f5d5bc14be55980c3c` | true |
| `app/i18n/zh-CN/warp.ftl` | 361,234 | `2291d0796862e703a2869e174d38138f014183730d380d56d6d97ce4772a0d98` | true |

英中完整 FTL 源字节数和散列均独立核对，与源 manifest 及构建报告相符。显式 `rust-embed/debug-embed` 将两份资源固定在构建产物中；完整字节嵌入及不可变资源结论来自根代理原始二进制扫描，归档任务未重读二进制。资源固定不证明英文或简体中文界面已经完成截断、换行和控件尺寸检查。

两份报告经凭据模式扫描后逐字节归档；构建日志 SHA-256 也已独立核对，未复制日志。对应源文件核对、签名字段及证据边界保存于[构建独立审计](validation/macos-official-11-bundle-archive-audit.json)。本段仅计本地构建、签名与语言资源固定，不计公证、发布、真实 CLI runner 结果、GUI验收或最终同提交跨平台门禁。此前四份门禁原件及审计保持不变。
