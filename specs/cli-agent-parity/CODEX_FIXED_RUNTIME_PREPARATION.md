# 固定 Codex 完整运行包

GUI 验证使用的私有 0.147.0 副本曾只恢复主程序。主程序版本检查成功，但真实 `exec` 调用无法启动同目录下的 `codex-code-mode-host`，所以文件读取和随后审批未发生。这是测试环境准备错误；原失败保留在 `validation/gui-7e065085/15-native-tool-host-missing.ndjson`，不能将它记为模型没有选择读取文件或审批拒绝。

准备器现使用固定官方 `rust-v0.147.0` 的完整 package，保留 `bin`、`codex-path`、`codex-resources` 和 `codex-package.json` 布局。Linux x64 包包含工具宿主、rg、bwrap 和 zsh；Windows x64/ARM64 包包含工具宿主、rg、command-runner 与 sandbox-setup。各平台主程序摘要仍与先前已受测固定主程序相同，没有修改用户系统安装。

固定上游提交为 `be6e8eac029b183056b7e4402879f15d2c85f61b`。[官方构建脚本](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/.github/scripts/build-codex-package-archive.sh)明确区分完整 `codex-package` 与独立入口；[官方布局及验证代码](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/scripts/codex_package/layout.py)要求工具宿主并保留平台资源。准备器不读取 `latest`，不以系统 0.154 或其它版本的辅助程序补缺。

| 固定资产 | 字节数 | SHA-256 |
| --- | ---: | --- |
| [Linux x64 package](https://github.com/openai/codex/releases/download/rust-v0.147.0/codex-package-x86_64-unknown-linux-musl.tar.gz) | 119166922 | `bd758d53d56e41dc65e045f4589df79a038ed197a011adcb52a258e6ad64cfda` |
| [Windows x64 package](https://github.com/openai/codex/releases/download/rust-v0.147.0/codex-package-x86_64-pc-windows-msvc.tar.gz) | 126777040 | `c156c8feb8cb20197bf74d2c6daffed1fec0a8c21a03bc2ca90d7ff81927b0c5` |
| [Windows ARM64 package](https://github.com/openai/codex/releases/download/rust-v0.147.0/codex-package-aarch64-pc-windows-msvc.tar.gz) | 117548230 | `4533928d72ac4d7c19f16e8c4acdfd02dc255d2aeeb2f6d7dfd45493ec4c0806` |
| macOS ARM64 GUI 私有 package | 107229164 | `17b2984eb22b607e3d0c25728252fc90f510e476bad39a6d9f45cdb1aa685432` |

归档及每个成员的大小、摘要、模式和元数据均固定。提取拒绝越界或非规范路径、重复条目、链接、重解析点、设备、未知文件、缺失成员及超限内容；只写作业私有目录，在完整验证后发布。复用缓存时重新核对全树，不覆盖本地修改或补修残缺目录。另一个准备进程的锁不会被清理。

单个包最多 32 个成员，单文件及总提取内容均限制为 512 MiB。归档模式须匹配固定清单；解包后仅保留所有者读写/执行权限，Unix 复用还核对文件模式。运行包根目录、全部子目录及直接下载父目录必须属于当前有效 UID 且 mode 精确为 `0700`；不匹配时拒绝，不能静默 chmod 或覆盖。父路径逐级拒绝其它 UID 所有的目录，以及无 sticky 保护的 group/other 可写目录；可信祖先限 root 或当前 UID。macOS 只特许 root 拥有、实际指向 `/private/tmp` 的系统 `/tmp` 别名，未知缓存链接仍拒绝。Windows 只检查目录类型和重解析点，ACL 明确未验证，POSIX mode 不代表 Windows 权限证明；本轮也未评估额外的 macOS ACL。

完整树先写同目录私有暂存路径再 rename 发布，准备锁只串行化遵循本准备器协议的进程，不宣称能防止另一个同权限进程同时改写目录。残留锁不会被自动抢占；失败清理保留原异常并附加清理错误，不能把残缺目录当成可复用缓存。

本机验证使用真实三个异平台官方包执行完整提取和缓存复核，未执行其中的程序。离线回归覆盖缺失宿主、越界归档、损坏内容/元数据、已有缓存和准备锁。它们证明准备器边界，不证明 Windows/Linux 工具实际运行；后者必须由同提交原生验证提供。macOS 私有完整包已核验主程序、rg、zsh 的版本，并在干净 37b0bc732 的 Rust 适配器完成真实工具、审批、追加、取消、进程恢复及图片验证，见[原生复验](CODEX_COMPLETE_RUNTIME_NATIVE_VERIFICATION.md)；GUI 文件读取与审批仍待复验。

2026-09-17 第一版收口验证：22 项离线回归通过，包含三个实际平台布局、原 Windows x64/ARM64 主程序摘要保持、写入失败不发布、次生清理错误不遮盖首因，以及私有 `--version` 环境不继承 API 凭据、代理或动态库覆盖。旧准备器 SHA-256 为 `e48682fb8816aa2400f66621fcd68b3bb4e9a9a2b291c72738c95f255796bd3b`；以该版本重新展开三个真实官方包并验证无修改缓存复用。原始 JSON 逐字节归档为[静态报告](validation/codex-0.147.0-full-runtime-static-final.json)和[当时输入](validation/codex-0.147.0-full-runtime-static-final.inputs.json)，另附[审计说明](validation/codex-0.147.0-full-runtime-static-final.audit.json)。随后独立审阅发现旧版本没有检查 Unix 目录权限，旧报告不能用于追认下面的修复。旧 inputs 的文档摘要对应修改报告链接前的文档，不能当成本文件现在的摘要。

第二版修复 Unix 目录权限后，27 项离线回归通过（0.168 秒），新增运行包根目录和 `bin` 的 `0777` 拒绝、可写下载父目录、祖先 sticky 保护、外来 UID 及系统 `/tmp` 别名回归。新准备器 SHA-256 为 `f26231df0f34008a7966a21562a6ceeb07d9896ebdf8c08267e61b130a3ff7d0`；以新版再次真实展开全部三个官方包并复核未修改缓存，见[新版静态报告](validation/codex-0.147.0-full-runtime-static-final-2.json)、[新版输入](validation/codex-0.147.0-full-runtime-static-final-2.inputs.json)和[新版审计](validation/codex-0.147.0-full-runtime-static-final-2.audit.json)。每包 6 个普通文件，Linux 5 个目录、Windows 各 3 个目录；各报告的时间包含该平台的展开与缓存复核总耗时，不能拆成独立阶段计时。

以上均未执行异平台二进制；Windows ARM64 保留可准备能力，当前 x64 CI 不能作为 ARM64 执行证据。早期 13 项及其旧提取器摘要记录保留在[前次准备证据](validation/macos-codex-complete-packages-1.json)，不冒称为后续 22 或 27 项。此次归档没有重新运行库测试、同提交 CI 或模型流程。

离线测试命令保持为 `python3 -B script/cli-agent-parity/prepare_codex_cli_tests.py -v`。Linux 的现有工作流调用仍是 `python3 -B script/cli-agent-parity/prepare_codex_cli.py --download-dir "$RUNNER_TEMP/codex-0147-inputs"`；Windows 使用同一脚本和 `--download-dir "$env:RUNNER_TEMP\codex-0147-hook-inputs"`。准备器返回 `runtime-0.147.0-<平台>/bin/codex[.exe]` 的绝对路径，调用方必须保持其完整兄弟目录，不能只复制主程序。`--version` 使用新建私有 HOME/CODEX_HOME 并在结束后清理；本轮只通过模拟调用核对该环境，真实 Linux/Windows 版本执行留待平台门禁。

已有初始化、原生 hook、零模型边界证据保持原范围，不因补齐 package 而自动提升为完整模型生命周期通过。无需本地化变更。
