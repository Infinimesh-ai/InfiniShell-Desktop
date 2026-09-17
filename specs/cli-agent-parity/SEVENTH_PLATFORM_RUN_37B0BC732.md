# 第七轮平台验收：37b0bc732

提交 `37b0bc73288aab5be4d0d151796eea557be4c0fb` 的 [Actions 运行](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35178775808) 已结束。Linux 成功，Windows 失败，整体不能计为跨平台通过。本轮没有开启完整工作区和 GUI integration 编译。

## 实际结果

| 检查 | Linux x64 | Windows x64 |
| --- | --- | --- |
| cargo check -p warp | 通过 | 通过 |
| warp 定向回归 | 1706 通过 | 1672 通过、5 失败 |
| ai 动作与技能 / warp_cli / 双语 TUI | 71 / 128 / 9 通过 | 71 / 128 / 9 通过 |
| command 托管进程 | 1 通过 | 1 失败，CreateProcess 返回 OS 5 |
| 监督 worker 构建 | 通过 | 通过 |
| 宿主崩溃清理 | 4 通过 | 前置失败，跳过 |
| rust-genai | 81 通过 | 81 通过 |
| 固定 CLI 准备与无凭据边界 | 全部所选探针通过 | 权限投影测试先失败，后续准备与探针跳过 |
| Codex Windows 原生 hooks | 不适用 | 通过，模型请求为 0 |
| Codex Windows ConPTY 通知 | 不适用 | 未到达，失败 |

完整阶段和日志摘要见 [Linux 证据](validation/linux-seventh-platform-37b0bc732.json)、[Windows 证据](validation/windows-seventh-platform-37b0bc732.json)。不同筛选存在重叠，计数不可累加为全部独立用例。

## 失败定位与候选

1. Python 权限观测夹具将 POSIX 字面路径交给当前平台 `Path`；Windows 反斜杠转换使替换断言失败。候选使用显式 `PurePosixPath`，并增加原生与正斜杠 Windows 路径的脱敏回归；两种原生输出均保留规则正文。
2. 五个 Claude 文件事务测试先调用不支持 Windows 的生产通知补丁入口，尚未执行事务断言就被平台门禁拒绝。候选从真实随附补丁构造缓存，并仍调用生产完整校验；五个事务用例继续在 Windows 执行，另验证 Windows 安装拒绝不会改动原文件和注册表。生产通知门禁保持关闭。
3. `managed_tests.rs:71` 是 `root_command.spawn()`，尚未进入 `ManagedTree::claim`。托管 worker 强制请求 `CREATE_BREAKAWAY_FROM_JOB`；禁止脱离的宿主 Job 会以 OS 5 拒绝此请求。候选改为继承监督者边界并在授权执行前加入严格嵌套 Job，保留 KILL_ON_JOB_CLOSE、后代退出确认和重复确认保护；增加真实严格父 Job 回归。第七轮未采集宿主 Job 属性，具体父 Job 来源仍是推断，不能宣称已原生证明。
4. ConPTY 的独立 CONOUT$ canary 正常，但 Git Bash 无法打开 `/dev/tty`，详见 [原生诊断](validation/windows-conpty-native-diagnostics-37b0bc732.json)。`c46714435` 已包含临时探针专用 Windows 控制台输出候选；原生执行与正式插件配方仍待验证。
5. Windows 最后上传原生边界报告时没有文件，是前置失败导致探针跳过的结果，不是额外的模型或协议失败。

本轮修复不改变用户界面文案，无需本地化变更。新的候选必须在包含实际修改的提交上重新验证，不能追溯把本轮失败改为成功。

## 候选本地门禁

以 c46714435 为基线的独立冻结验证树已通过 check、i18n 11 项、相关模块 987 项、command 原生 1 项以及权限投影 Python 15 项。Windows 专属分支仍须实际执行。逐文件摘要、命令、退出码和撤下的首个误目录检查见 [本地记录](validation/macos-windows-seventh-fixes-2.json)。
