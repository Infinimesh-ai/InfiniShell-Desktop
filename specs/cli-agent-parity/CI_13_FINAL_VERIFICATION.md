# CI13 最终验证记录（终态待确认）

截至最后一次实际API查询2026-09-17 19:36:12.183109 UTC，CI13 run [35263203530](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35263203530)及Linux/Windows两个job仍为in_progress，未观察到失败步骤。**最终结论为pending，final=false，不能声明CI13整体通过。** 原因诊断仍unknown；尚未观察到失败，不能猜测任何失败原因。

唯一运行的实际head SHA在初窗及本接续窗共6轮中均为 `e6873e2cbb12a12ffce0b8283ee2ac96b940da39`。本次只有19:09:33 UTC的一次真实派发，run_linux=true、run_windows=true、full_workspace_tests=false；参数证明来自已归档派发命令，不来自没有inputs字段的run API。source28公开本地报告check0/i18n11/Rust1410/Python426及111项已提交源码字节等价链已在初窗归档，原dirty candidate测试报告未回填为执行时同提交验证。

| 平台 | 最新实际API成功步骤范围 | 仍在执行及缺口 |
| --- | --- | --- |
| Linux x64，job105343794125 | Python7，固定CLI准备8–10，无认证原生边界11–15，通知脚本及PTY16–17，cargo check18，受影响Rust19，共享契约20，harness参数21，TUI22，进程ownership23 | supervisor构建24仍in_progress，后续host crash、恢复边界、上传、其他门禁及job终态未确认。 |
| Windows x64，job105343794289 | Python8，固定CLI准备9–11，无认证原生边界12–16，Git Bash17，native hooks18及上传19，ConPTY20及上传21，cargo check22，SSH worker构建23及PowerShell参数24，受影响Rust25，共享契约26，harness参数27 | TUI28仍in_progress，后续进程ownership、supervisor、host crash、恢复边界、上传、其他门禁及job终态未确认。 |

此表只表达API工作流步骤的success范围，没有取得原始job日志或产物，没有归档各模块本平台真实测试计数。步骤组成功不能作为每个case独立通过的证据；SSH构建和参数测试不等同实际SSH/tmux交互，TUI测试不等同GUI双语布局，无认证native边界不等同在线模型任务全链。完整工作区full=true仍待最终阶段执行。

初窗于19:20:18的第3轮之后封存。根代理随后单独授权一个15分钟接续窗，首次不得早于19:25:18.650813 UTC。接续窗实际3轮分别为19:25:25.427648、19:30:57.108284、19:36:12.183109 UTC，相邻间隔331.680636及315.074825秒，实际run/jobs API请求时间、响应字节和SHA均记录在独立followup文件。该窗硬截止为19:40:25.427648 UTC，下一合法cursor19:41:12.183109 UTC已超出截止，因而没有第4轮合法查询。本窗于 2026-09-17T19:37:32.223480+00:00 UTC提前封存并STOP，不在剩余时间密集poll。

初窗文档及runneravailability/run/jobs/audit共5档逐项SHA复核全未变，本接续窗不改它们。初始审计已核CI12原始10档未变，本次不回填CI12失败及原计数，也不重派发、不取消workflow、不改代码/refs/索引或主三文档。终态尚未取得，所以每job原始log请求及fallback均为0，没有复制raw日志或读取认证/模型正文。执行调度和归档由同一子代理完成，archive_auditor_independent_from_executor=false。无需本地化变更，Goal未完成。后续如需监看，从上述cursor之后的新授权有界窗继续。
