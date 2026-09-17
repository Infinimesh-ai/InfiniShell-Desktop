# Windows 普通 Codex 通知的 ConPTY 运输边界

## 当前证据与接口

第五轮 Windows 预检已证明固定 Codex 0.147.0 可以选择候选 `commandWindows`，经 `cmd /C`、Windows PowerShell 5.1、Git Bash 调用受控插件，原生 SessionStart / UserPromptSubmit 及独立阻断 hook 完成，模型 HTTP 请求为零。该结果未读取 ConPTY 输出，不能证明终端收到了通知。

随附 `warp-notify.sh` 只写 `/dev/tty`，失败由 `2>/dev/null || true` 忽略；没有 `CONOUT$` 回退。[固定 Codex command_runner 源码](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/hooks/src/engine/command_runner.rs#L55) 将 hook 三路标准流设为管道，Windows 默认使用 `cmd /C`，此路径没有设置 `CREATE_NO_WINDOW`。候选 PowerShell 启动器使用 `UseShellExecute=false`、`CreateNoWindow=false` 并直接继承三路句柄；它不会把 hook 的 stdout 偷换为终端通知通道。

MSYS `/dev/tty` 依赖已分配的控制终端。[MSYS2 3.6.5 dtable](https://github.com/msys2/msys2-runtime/blob/msys2-3.6.5/winsup/cygwin/dtable.cc#L577) 按 ctty 选择 console/pty 处理器，这份固定源码解释条件，不能代替当前 runner 的 Git Bash 实测。Win32 `CONOUT$` 可以在 stdout 重定向后打开当前附着控制台的活动屏幕缓冲区；没有附着控制台时不构成可用回退。[微软 Console Handles](https://learn.microsoft.com/en-us/windows/console/console-handles)

产品 `app/src/terminal/local_tty/windows/mod.rs` 加载随附 `conpty.dll`，建立管道与 HPCON，使用 `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` 及 `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_BREAKAWAY_FROM_JOB` 启动终端，不设 `STARTF_USESTDHANDLES`。新增独立探针复用这些 flags、同一仓库 DLL 和 OpenConsole 二进制；通信使用官方示例的两条同步管道及独立输出线程，未调用产品 IOCP 事件循环。[微软创建伪控制台说明](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)

## 最窄新增探针

`probe_codex_windows_conpty.py` 只在 Windows x64 原生执行：

1. 复用 `codex_windows_hook_inputs.obtain_inputs` 固定下载/摘要缓存，不接受任意 Codex 版本。将当前提交的 DLL 和 OpenConsole 复制到私有临时目录，核对摘要并保留产品目录形状。
2. 创建无可见桌面的私有 HPCON，附着一个 Python driver。driver 只导入现有 `probe_codex_windows_hooks.one_case`，不修改原探针、候选 PS 或随附生产脚本；只在已有临时测试插件中保留原场景的插桩和独立阻断 hook。
3. driver 向子进程提供隔离 HOME / CODEX_HOME 和两个结构化通知能力变量，不复制任何认证。真实 app-server 的 RPC stdin/stdout 仍为独立管道。每次原生 initialize 握手返回后，`GetConsoleProcessList` 必须包含其实际 PID；句柄读取进程创建时间与可执行文件路径作为附着证据。对短暂 cmd / PS / Bash 的成员采样仅为诊断，缺采样不是“不附着”的证明。
4. `CONOUT$` 先发送独立诊断标记，临时开启私有控制台的 VT 模式后立即恢复；此标记不满足原生通知门槛。随后复用原生场景的两个含中文、空格及 shell 元字符的路径，原样执行 SessionStart / UserPromptSubmit 通知脚本。
5. 父探针并发保存 HPCON 真实输出原始字节。只接受与原生 hook 记录中的 session ID / turn ID 匹配的 `OSC 777;notify;warp://cli-agent` JSON；每个事件必须恰好一次。原生 hook 成功、stdout 文本、CONOUT$ 诊断标记或旧 session/turn 都不能代替通知。
6. driver 最长 120 秒，结束后关闭自持 HPCON 并有界等待输出 EOF。超时只终止自持进程句柄、关闭自己的控制台；不按名称/PID 批量终止、不改全局服务或控制台配置。任何 native、输出或关闭失败均返回非零，保留现场和否定证据。

Windows 预检接入命令（由主代理调度）：

```powershell
python -B "$env:GITHUB_WORKSPACE\script\cli-agent-parity\codex_windows_conpty_tests.py" -v
python -B "$env:GITHUB_WORKSPACE\script\cli-agent-parity\probe_codex_windows_conpty.py" --repo "$env:GITHUB_WORKSPACE" --download-dir "$env:RUNNER_TEMP\codex-0147-hook-inputs" --bash-executable "$env:INFINISHELL_GIT_BASH_EXE" --jq-executable "$env:INFINISHELL_JQ_EXE" --output "$env:RUNNER_TEMP\codex-windows-conpty.json"
```

输出为主 JSON、同名前缀 `.native.json` 和 `.pty.bin`。重复运行必须使用新输出名，不覆盖失败证据。原始日志包含临时路径与 runner 路径，入仓库前应脱敏。探针输出还保留私有目录路径，故障检查完成后由调用方清理。

## 判定与限制

- `native_conpty_notifications_verified=true` 只说明固定候选 commandWindows 经真实 Codex 原生 hooks 和随附 ConPTY 后，宿主确实读取到指定通知；不代表候选已经集成安装器，也不代表普通 Codex 交互 TUI、产品 ANSI 解析、GUI chip 或完整生命周期通过。
- 只有 CONOUT$ 诊断通过而原生 OSC 缺失时，应记录候选通知运输失败，进一步按 `/dev/tty` / MSYS ctty / 控制台附着链定位；不能把错误吞掉后的 exit 0 算通过。
- 连 CONOUT$ 诊断也缺失时，仍需区分 DLL/宿主建立失败、VT 转换与 OSC 透传行为；不能直接归因为 `/dev/tty`。未将源码中有限接口推断成平台能力结论。
- 读取到最终字节仍不证明产品真实 IOCP / 终端模型已经消费。现有 `parse_osc777_notification*` 属于解析器回归；完整产品接线仍需要单独的 Windows InfiniShell 终端验收。
- 离线测试只验证严格关联、重复/旧回调拒绝、环境块编码及 candidate 配置。当前 macOS 开发环境不能执行 Windows 原生验收；真实 Windows 结果见下一节，不能由离线测试覆盖其失败。

本机准备阶段执行 `python3 -B script/cli-agent-parity/codex_windows_conpty_tests.py -v`：9 项通过；`git diff --check` 通过。未执行 Windows 二进制、Cargo、模型请求或远端派发。

## 第六轮真实失败及下一步诊断

运行 `35126330599`、提交 `7e06508554ae64cdd9321e0a69274e3d7b2d55ce` 的 Windows 原生探针失败：首个场景的 `session_start` 匹配数为 0。原始三份产物保留于回收目录 `/tmp/infinishell-sixth-platform-7e0650855/windows-conpty/`；未覆盖或改写。它们包含 runner 私有路径，未直接复制入仓库。

| 已核验事实 | 结果 |
| --- | --- |
| `.pty.bin` 原始字节 | 802 字节；SHA-256 `cd07b30cb800f3d9ac66bff4cedc646b112b3767dfaf928a44e96a5372a91c5d` |
| CONOUT$ 诊断 | 唯一诊断 nonce 的完整 OSC777 已读取，`conout_canary_observed=true`；原控制台 mode 为 7 并已恢复 |
| 原生进程附着 | 四次实际 Codex initialize 后均在该私有控制台成员中；成员观察也包含 PS 5.1 与 Git Bash，无采样错误 |
| 原生 hooks | 两个路径场景均完成 SessionStart / UserPromptSubmit，独立阻断有效，marker 的 session/turn 正确；模型 HTTP 请求为 0 |
| 真实通知 | 读取到的唯一 OSC777 是诊断项；没有原生 `session_start` 或 `prompt_submit`。整体 `passed=false`，通知运输未通过 |

这些事实排除了本轮 ConPTY 普遍丢弃未知 OSC 的解释，但还不能确认通知在哪个前置步骤停止。发布脚本会在能力变量缺失时静默返回，也会吞掉 `/dev/tty` 写入错误；当前 marker 未记录这两个分支。进程附着不等于 MSYS 已建立 POSIX 控制终端。

后续探针只增加私有 `BASH_ENV` 观察器：在实际原始 SessionStart / UserPromptSubmit 脚本及 `warp-notify.sh` 入口，记录能力 gate、两项能力变量、Bash/MSYS 版本、标准流是否为 tty，以及 `/dev/tty` **仅打开、不写入**的状态与错误。通知入口额外记录已有 argv 载荷中的 session/turn/event，舍弃正文。观察器在子 shell 中执行，不读取 hook stdin，不改原脚本、候选 PS、信任摘要或标准流；诊断 JSON 写到私有目录，再收入 `.native.json`。缺诊断仍是信息缺口，不能自动证明 gate 或 tty 分支。

此修改用于取得首因证据，不是运输修复。唯有原来的真实 OSC 匹配才能通过。新增本机回归确认观察器保留含中文的 stdin/stdout/stderr，并拒绝把诊断 JSON 算作终端通知；11 项测试通过（0.035 秒）。本轮没有执行 Windows 二进制、Cargo、Git 或远端派发。
