# 固定 Codex 完整运行包

GUI 验证使用的私有 0.147.0 副本曾只恢复主程序。主程序版本检查成功，但真实 `exec` 调用无法启动同目录下的 `codex-code-mode-host`，所以文件读取和随后审批未发生。这是测试环境准备错误；原失败保留在 `validation/gui-7e065085/15-native-tool-host-missing.ndjson`，不能将它记为模型没有选择读取文件或审批拒绝。

准备器现使用固定官方 `rust-v0.147.0` 的完整 package，保留 `bin`、`codex-path`、`codex-resources` 和 `codex-package.json` 布局。Linux x64 包包含工具宿主、rg、bwrap 和 zsh；Windows x64/ARM64 包包含工具宿主、rg、command-runner 与 sandbox-setup。各平台主程序摘要仍与先前已受测固定主程序相同，没有修改用户系统安装。

| 固定资产 | 字节数 | SHA-256 |
| --- | ---: | --- |
| Linux x64 package | 119166922 | `bd758d53d56e41dc65e045f4589df79a038ed197a011adcb52a258e6ad64cfda` |
| Windows x64 package | 126777040 | `c156c8feb8cb20197bf74d2c6daffed1fec0a8c21a03bc2ca90d7ff81927b0c5` |
| Windows ARM64 package | 117548230 | `4533928d72ac4d7c19f16e8c4acdfd02dc255d2aeeb2f6d7dfd45493ec4c0806` |
| macOS ARM64 GUI 私有 package | 107229164 | `17b2984eb22b607e3d0c25728252fc90f510e476bad39a6d9f45cdb1aa685432` |

归档及每个成员的大小、摘要、模式和元数据均固定。提取拒绝越界或非规范路径、重复条目、链接、重解析点、设备、未知文件、缺失成员及超限内容；只写作业私有目录，在完整验证后发布。复用缓存时重新核对全树，不覆盖本地修改或补修残缺目录。另一个准备进程的锁不会被清理。

本机验证使用真实三个异平台官方包执行完整提取和缓存复核，未执行其中的程序。离线回归覆盖缺失宿主、越界归档、损坏内容/元数据、已有缓存和准备锁。它们证明准备器边界，不证明 Windows/Linux 工具实际运行；后者必须由同提交原生验证提供。macOS 私有完整包已核验主程序、rg、zsh 的版本，真实模型工具调用仍需 GUI 复验。

已有初始化、原生 hook、零模型边界证据保持原范围，不因补齐 package 而自动提升为完整模型生命周期通过。无需本地化变更。
