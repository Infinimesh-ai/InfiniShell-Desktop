# CI13 初始跨平台验证记录

CI13 已于 2026-09-17 19:09:33 UTC 实际派发一次，运行链接为 [35263203530](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35263203530)。GitHub API 返回实际 head SHA `e6873e2cbb12a12ffce0b8283ee2ac96b940da39`，分支为 `codex/cli-agent-parity`，与派发前 API 分支及 ls-remote 核验一致。参数为 run_linux=true、run_windows=true、full_workspace_tests=false，依据真实派发命令记录；run API 不返回 inputs，不能把它作为参数证明。本次是离线夹具修复后的迭代检查点，最终 full=true 仍待执行。

19:10:06.831162 UTC 的首轮公开 API 观察：run 为 in_progress，Linux x64（job105343794125）与 Windows x64（job105343794289）均为 in_progress，尚未取得终态。首轮 Linux 正在 checkout，Windows 正在准备 job-local Python；其他未完成步骤不得记作通过。首次查询从 workflow run 列表中按新建时间和精确 SHA 唯一定位，随后读取 jobs；每份实际 API 返回字节数量和 SHA 均保留在对应安全投影中，未复制作者、提交正文、认证或环境信息。

派发前 runner API 实际返回 Linux runner21 / Windows runner22 均 online、busy=false，并具备工作流所需 self-hosted、对应 OS、x64、infinishell-ci 标签。元数据见 runner-availability.json；这是当时的可用性观察，不保证后续作业或步骤成功，没有通过 SSH 操作 runner。

source28 已执行的公开本地门禁报告为 cargo check 退出0、i18n11、受影响 Rust1410、Python426/15模块，源清单 SHA `7bfe731c3a40207d8d428702deac95a8fa4fd9d6b603312c8ef4cb0310f6acd7`。111 项已独立核对：本次 e687 提交 Git 对象、candidate28 与该清单逐项一致。436→e687 的 app/script/crates/resources/.github 已提交源码改动集合恰为清单中的11项，无用户原有两个改动。原门禁发生在未提交 candidate28，本归档只证明这些受核源码字节在提交后相同，不能改写旧报告的 dirty_candidate 或执行时 same_commit_verified=false，也没有重跑本地门禁。

初窗硬截止为 19:24:33.063255 UTC，最多4次状态观察，相邻观察至少300秒；首轮后的最早查询为19:15:06.831162 UTC。不重派发、不取消，不把已存在的 CI12 失败回填为修后通过。窗口关闭仍在运行时保留 pending；只有真实终态和可取到的定向日志才能形成最终报告。

验收边界：步骤 success 只能说明对应工作流步骤通过，不等同每个 case 的独立证据。无认证的 native 准备、初始化、退出或协议边界不等同在线模型全链；SSH worker构建与参数测试不等同真实SSH/tmux交互；TUI单测不等同真实GUI双语布局。本轮 full=false 跳过项及原生任务全链缺口如实保留，Goal 未完成。CI执行调度与本轮归档由同一子代理完成，archive_auditor_independent_from_executor=false。无需本地化变更。

本窗于 2026-09-17T19:21:34.246136+00:00 UTC 提前关闭：实际状态查询3轮，时间依次为19:10:06.831162、19:15:17.258118、19:20:18.650813 UTC，相邻间隔310.426956与301.392695秒。第3轮run及两个job仍为in_progress，未发现失败步骤。Linux Python7、Windows Python8均completed/success；固定CLI准备及无认证native边界步骤均success。两平台cargo check（Linux18/Windows22）均completed/success；Linux正在受影响Rust测试19，Windows正在SSH worker构建23。Windows原生hooks18、ConPTY20及其上传21亦success。当前只具有API步骤范围证据，没有读取原始job日志，未归档逐case或完整Python/Rust计数。

下一合法查询为19:25:18.650813 UTC，已经超出初窗19:24:33.063255截止，因而不能在本窗发起第4轮。最终状态仍pending/cause_unknown；未创建最终报告或宣称部分步骤使CI13整体通过。追加有界观察窗须由根代理另行授权，并从该cursor之后继续。同一检查确认CI12初始4档及最终6档均保持原SHA，不改写原FAILED及计数。STOP。
