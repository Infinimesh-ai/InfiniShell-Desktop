# 第三次跨平台运行：328d5ed35

2026-09-16，运行已结束：**Linux x64 passed；Windows x64 failed；整次跨平台门禁 failed**。Windows 后续修复不改变本次提交的结果。

- 精确提交：`328d5ed35227f67884013f8c403e2a692470151d`。
- Actions：[35110822335](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35110822335)。
- 完整 Linux job 日志、初始/最终元数据及实际 artifacts：`/tmp/infinishell-third-platform-328d5ed35-ayckbnbw`。
- 汇总与每项产物摘要：[JSON 报告](validation/third-platform-run-328d5ed35.json)。

## Linux 已通过

| 门禁 | 实际执行 |
|---|---|
| Grok Node 通知脚本 | 11 passed |
| Python 文件事务等 8 个脚本 | 13 / 10 / 6 / 12 / 4 / 2 / 6 / 9，均 OK |
| Bash payload / Unix PTY | 13 / 4，均 OK |
| `cargo check -p warp --lib` | passed |
| warp 定向 nextest（含 i18n/任务/消息/输入/协议） | 1655 passed |
| ai 共享契约/技能 | 71 passed |
| warp_cli | 128 passed |
| 双语 TUI label | 9 passed |
| command managed | 1 passed |
| 同提交 supervisor 构建、host-crash cleanup | 构建通过；4 passed |
| 原生 Codex missing-session / idle-crash | 各恰好 1 passed，真实 Rust adapter |
| rust-genai | 81 passed |

GUI integration 编译与 full workspace 两个步骤按运行配置 **skipped**；没有计为已通过。nextest 日志中的 skipped 还包含筛选排除和 ignored，详细数值保留在 JSON。

## 原生证据

- Claude 2.1.273：`passed=true`；initialize 回复成功，关闭 stdin 后 64ms 退出 0，未强杀，模型命令数 0，输入文件摘要不变。官方 Linux 文件 SHA-256 `6c752e2cc7c110c9df15f26d8d134d438c5ae95dbd610efc1a308bf7f9c5f6c1`。不包含 Rust adapter、活动 turn 或应用重启。
- Codex 0.147.0 注册失败恢复：8 个检查均 true，失败重装保持配置和 cache，原生 add 重新启用行为明确记录后通过原生 disable 恢复原始禁用字节。无凭据、无模型，未执行产品安装器。
- Codex 持久来源：11 个检查均 true，先复现 cache-only 修补被原生启动还原，再验证完整受控来源跨重新安装/重启保持；禁用、其他来源及无自动 trust 均检查。未验证 GUI/PTY。
- Codex missing-session：真实错误 `no rollout found`，没有新建替代会话；同代 `linux_subtree` 完整退出回执。
- Codex idle-crash：SessionReady 后用 Linux pidfd 绑定目标并终止，收到原生退出及 adapter Disconnected，完整 `linux_subtree` 清理回执；不代表运行中工具树清理或应用重启验收。

两项 Rust 原生边界均来自干净 `328d5ed35`：libtest SHA-256 `b64cb242878d59d1e25df1e767d81051525d7ca59aa8ad51e39476592a52d2be`，supervisor SHA-256 `0f96e252d8bf65945def65d6179518c5cca2a0b6022a0eff8b5ba276b05e501e`。

## Windows 与采集边界

Windows 共 11 success / 4 failure / 20 skipped。失败步骤是文件事务、Git Bash payload、原生 hook 验证，以及没有可上传文件的原生边界 artifact 步骤；Rust check/定向测试/原生恢复等后续步骤均 skipped。对应已完成 Windows hook artifact 也保存在同一临时目录；根因与修复由另一代理处理，本报告不重复分析。

运行期间 Linux job log API 返回 404；完成后首次下载被 gh 的 ANSI 输出保护拒绝，仅将允许转义字节的重取结果写到文件，未执行日志内容。最终原始日志 785401 字节，另保留去 ANSI 阅读副本。没有重跑、dispatch、SSH、Cargo、workflow 修改或 Git 索引/提交操作。
