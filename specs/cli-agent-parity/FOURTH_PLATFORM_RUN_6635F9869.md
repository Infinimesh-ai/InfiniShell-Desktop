# 第四次跨平台运行：6635f9869

运行已于 2026-09-16 UTC 结束：**Linux x64 passed，Windows x64 failed，整体跨平台门禁 failed**。后续修复和第五轮不改变本次结果。

- 精确提交：`6635f98690a6cbd3c117f9d313cb19de83016418`。
- Actions：[35117735097](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35117735097)，Linux/Windows 均选择，`full_workspace_tests=false`。
- 完整日志、下载产物和本机独立对照保存在 `/tmp/infinishell-fourth-platform-6635f9869-h25ch_x9`。
- [汇总 JSON](validation/fourth-platform-run-6635f9869.json)保留精确 job/step 状态、artifact 元数据、文件摘要和原生结果摘要。

## Linux 同提交通过

Linux job `104867353227` 用时 37 分 7 秒，31 个步骤成功、2 个按配置跳过。

| 门禁 | 实际结果 |
| --- | --- |
| Grok Node 通知脚本 | 11 passed |
| Python 文件事务与探针 10 个套件 | 13 / 10 / 8 / 14 / 4 / 2 / 6 / 10 / 10 / 9，均 OK |
| Bash payload / Unix PTY | 13 / 4，均 OK |
| `cargo check -p warp --lib` | passed，3 分 44 秒 |
| warp 定向 nextest | 1655 passed，14.519 秒执行时间 |
| ai 共享契约/技能 | 71 passed |
| warp_cli | 128 passed |
| 双语 TUI label | 9 passed |
| command managed | 1 passed |
| 同提交 supervisor 构建 / host-crash cleanup | 构建通过，4 passed |
| 原生 Codex missing-session / idle-crash | 各恰好 1 passed，真实 Rust adapter |
| rust-genai | 81 passed |

GUI integration 编译与全工作区套件按本次配置跳过，没有计为通过。筛选未覆盖的测试和 ignored 数量保留在日志中。

原生报告中的源码均为干净 `6635f9869`：

- **Grok 1.0.30**：首次 Linux 原生固定资产与 ACP 边界通过；握手版本正确，新建明确 `blocked_by_auth`，取消只发送不存在会话的通知，load/resume 返回 `FS_NOT_FOUND`。stdio 在 EOF 后 2120ms 自行退出 0，leader 等待 5000ms 未退出后通过自持句柄强制回收，私有目录删除。它不证明活动取消、认证后运行或历史恢复，也没有系统 HTTP 流量测量。完整范围见 [GROK_FIXED_ACP_BOUNDARIES.md](GROK_FIXED_ACP_BOUNDARIES.md)。
- **Claude 2.1.273**：initialize 与空闲 EOF 通过，64ms 自行退出 0，未强杀，未发送模型输入。不包含 Rust adapter 或模型生命周期。
- **Codex 0.147.0**：原生插件注册失败恢复 8 项检查、持久来源 11 项检查均 true；包括 cache-only 回滚复现、受控来源跨重启保持、禁用及无关配置保留。它们没有调用产品 GUI 安装器。
- **Codex Rust adapter**：缺失会话未被新会话替代；空闲原生 CLI 退出后出现 Disconnected，并取得同代 `linux_subtree` 清理回执。libtest SHA-256 为 `c350a3a9255af9ea85bc7aa99fe85eaf987dea2594e4a0d6b5adc6bf563b310a`，supervisor 为 `99101c2e5e134aaf8104a42b9fe93595f2005e635ba836abfccc6adc23e35828`。不代表真实模型工具树或应用重启验收。

## Windows 失败与已增加的正向证据

Windows job `104867353676` 用时 6 分 38 秒，12 个步骤成功、3 个失败、22 个跳过。NuGet Python、Grok Node、前面的文件事务及固定资产离线套件均正常执行。

首个直接失败是步骤 8 中 `GrokFixedAcpTests.test_report_redacts_identity_and_private_paths_but_keeps_protocol_ids`：Windows `Path('/isolated')` 与夹具手写 `/isolated/project` 字符串不一致，导致脱敏断言失败。这一组其余 9 项通过；后续 CLI 下载、ACP、Rust 检查、恢复等步骤被跳过，步骤 27 上传不到这些产物是后果。

独立执行的 Git Bash payload 本次 **13 项全部通过**。Windows native-hook artifact 又确认 **PowerShell 5.1 语法和原生 argv 往返通过**，因此第三轮的相应夹具修复已取得真实 Windows 正向证据。

新的独立失败位于步骤 32 的 `native_bytes_and_cmd_boundary`：固定启动器的 stdin/stdout 字节严格检查失败。原报告没有保存两侧实际字节，所以不能断定本轮究竟在哪个流、哪个字节发生差异。`cases=[]`，真实 Codex hook case 尚未开始，不能将模型请求空记录当作已通过阻断验证。报告原样保留，Windows 自动安装和原生 ConPTY 通知仍未开放或验收。

下一检查点已修 Grok 路径夹具、增加固定字节诊断，并依据微软固定源码移除候选启动器的 StreamWriter 输入转发。该变更没有放宽原字节断言或修改全局控制台编码；详见 [Windows 夹具记录的第四轮章节](WINDOWS_VERIFICATION_FIXTURES.md#第四轮6635f9869新增字节边界)。第五轮仅 Windows，提交 `94a412eb89a4de57977072e6c45f4a692955d970`、[run 35122575386](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35122575386)；其结果不由本第四轮报告宣称。

## 采集边界

Windows job 完成前日志不可获取；job 完成后 `gh run view --log` 仍因整个 run 进行中拒绝，于是使用显式仓库路径的 job 日志 API。第一次原始输出因包含 ANSI 转义被 gh 拒绝，随后仅允许原始字节写文件并生成脱色阅读副本，没有执行日志内容。两平台最终日志与所有已产生原生 artifact 均已下载并计算摘要。

没有重复 dispatch、SSH、Cargo、用户配置或 GUI 操作。前三轮失败完整保留；第四轮 Linux 通过也不代替下一提交的 Linux 验证。完整认证生命周期、真实 Windows 通知 UI、GUI 双语布局及最终同一提交的全平台验收仍需分别完成。
