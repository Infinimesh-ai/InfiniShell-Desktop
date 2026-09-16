# Windows 第三轮验证夹具修复

对象为提交 `328d5ed35227f67884013f8c403e2a692470151d` 的 [Actions run 35110822335](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35110822335)、Windows job `104843754255`。job 已失败结束；新的 NuGet Python 3.13.15 工具链成功，不能再把此次失败归因于缺少 Python。

本次只修验证夹具，没有修改通知插件、固定 PowerShell launcher、Rust、workflow、用户配置或凭据。后两项的具体机制来自固定上游源码和本地对照推断；原 job 没有上传相应 stderr / argv 差异，修复后仍需 Windows 原生复验。

## 最早直接失败：环境键大小写

step 8 的通知文件事务先完成 13 项（12 通过、1 个 Unix 链接测试跳过），受控来源完成 10 项（9 通过、1 个 Unix 模式测试跳过）。随后 `codex_adapter_runner_tests.py` 的 `test_missing_session_environment_has_no_user_credentials_or_configuration` 按 `environment["SystemRoot"]` 索引，在原生 Windows 的大写环境键映射上触发 `KeyError`。日志 405–422 行记录首个失败。

生产 `missing_session_environment` 已按 `key.upper()` 选择系统变量，只是测试错误地要求保留原始拼写。修复仅将断言改为规范键和值检查，保留凭据、代理、注入变量排除及私有目录断言，也保留同期新增的两个 macOS 清理边界用例。

step 9–24 因此被跳过；step 25 上传不到原生证据是其后果，不是另一个环境根因。

## 独立失败：Python 到 Git Bash 的多行夹具

always 执行的 step 29 中，13 项 payload 测试有 12 项通过；控制字符用例返回 3。原 traceback 保存了含中文、LF、ESC、BEL、C1 的参数，但 `CalledProcessError.stderr` 没有打印，不能称原 job 已记录了 jq 编译诊断。

[CPython 的 Windows argv 编码](https://github.com/python/cpython/blob/v3.13.15/Lib/subprocess.py#L608)只在参数含空格、Tab 或为空时加外层引号。该用例的 response 没有空格或 Tab，因此 LF 直接进入命令行。[MSYS2 固定源码的分隔符](https://github.com/msys2/msys2-runtime/blob/b54860d002ad85de2949bec05ca579aa62f0ef7c/winsup/cygwin/winsup.h#L132)包含 LF/CR，其 [build_argv](https://github.com/msys2/msys2-runtime/blob/b54860d002ad85de2949bec05ca579aa62f0ef7c/winsup/cygwin/dcrt0.cc#L303)会按这些字符分隔。它解释了为什么只含此种换行的额外参数会失败；该源码快照不是本次 runner DLL 构建提交的验证。

本机实际 Bash 对照中，将预期拆开的两段送给未修改的 `build_payload`，得到 jq 编译错误、退出 3；通过 stdin 读入同一完整文本再调用，则退出 0、原文完整且 JSON 中没有裸 ESC/BEL。此对照验证拆分的后果，不冒充 Windows MSYS 实测。

最小修复仅此控制字符用例：将文本以 UTF-8、NUL 分隔的 stdin 送入固定 Bash 脚本，读取为单个变量后调用真实 `build_payload`。完整中文、换行、C0/C1 原值与转义断言均保留；失败时输出 JSON 转义后的 stdout/stderr。其他 argv / 路径用例继续原样执行，没有放宽或替换产品脚本。

## 独立失败：PowerShell 5.1 JSON 数组包装

step 30 的 artifact 显示固定 Codex 下载摘要、原生 `codex-cli 0.147.0`、明文编码往返与 PowerShell 5.1 语法均通过，但在 `native_argv_roundtrip` 停止，`cases=[]`。因此这轮没有开始真实 Codex hook，也没到 stdin/stdout 字节与 cmd 8191 边界验证。

验证器临时脚本使用 `$values = @(Get-Content ... | ConvertFrom-Json)`。旧 PowerShell 将根 JSON 数组作为一个管道对象输出：固定 [v6.2.7 源码](https://github.com/PowerShell/PowerShell/blob/v6.2.7/src/Microsoft.PowerShell.Commands.Utility/commands/utility/WebCmdlet/ConvertFromJsonCommand.cs#L117)调用 `WriteObject(result)`；[v7.0.0](https://github.com/PowerShell/PowerShell/blob/v7.0.0/src/Microsoft.PowerShell.Commands.Utility/commands/utility/WebCmdlet/ConvertFromJsonCommand.cs#L123)才默认枚举。官方仓库 [issue 3424](https://github.com/PowerShell/PowerShell/issues/3424)也记录了 Windows PowerShell 5.1 的这一边界。

额外 `@()` 因此会包装成长度 1 的嵌套数组，再经 `[string]` 参数合并为一条 argv。本机 PowerShell 7 使用 `-NoEnumerate` 对照旧行为，观察到旧写法 count=1、一个合并参数；普通括号赋值 count=8，保留空串、中文、引号、尾反斜杠和 shell 字面字符。此为旧行为对照，不是运行 Windows PowerShell 5.1。

修复仅临时验证脚本的外层包装，增加固定数组形状检查；不改 `ConvertTo-NativeArgument` 或产品候选 launcher。原生往返仍逐项精确比较，失败消息现在保存这一组固定 fixture 的 expected/actual 数组，供现有 probe 异常报告写入 artifact。新增两个回归验证失败诊断完整及原值接收，替身不计为原生 argv 通过。

## 验证结果和下一步

macOS 本地执行：

```sh
python3 -B script/cli-agent-parity/codex_adapter_runner_tests.py -v
python3 -B script/cli-agent-parity/plugin_compatibility_tests.py -v
python3 -B script/cli-agent-parity/codex_windows_hook_tests.py -v
```

分别 8 项（0.007 秒）、13 项（4.330 秒）、14 项（0.011 秒）通过，另有实际 Bash stdin / 拆分对照和 PowerShell 7 数组对照。摘要证据见 [windows-328d5ed352-verification-fixtures.json](fixtures/windows-328d5ed352-verification-fixtures.json)，其中保留原失败与未执行边界。

修复提交尚未在 Windows 运行。下一轮必须重新执行原有全部门禁，确认 argv、原始字节、cmd 边界与真实 hook；不能只凭这三个本地套件通过就声称 Windows 通知支持。未运行 Cargo、SSH 或模型请求；无需本地化变更。

## 第四轮：6635f9869 新增字节边界

以上“尚未在 Windows 运行”是第三轮修复时的历史状态。[第四轮 35117735097](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/35117735097) 已在 `6635f98690a6cbd3c117f9d313cb19de83016418` 原生 Windows 执行：旧环境键断言通过、Git Bash payload 13 项通过、PowerShell 5.1 语法和原生 argv 往返通过。第四轮仍失败，不能被这些局部正向结果覆盖。

步骤 8 新的首因是 Grok 脱敏纯夹具混用了 `Path('/isolated')` 与手写 POSIX 字符串。在 Windows 上前者的字符串形式使用反斜杠；原生 probe 本来用 `str(project)`，生产 `clean()` 无需改变。修复测试使用实际路径字符串，同时分别用 `PurePosixPath` 和 `PureWindowsPath` 保留两类精确脱敏断言。其余 9 项原生 Python I/O/ACP 形状回归在本次 Windows 已通过；修后本机 10 项通过。

步骤 32 独立停止在 `native_bytes_and_cmd_boundary`，提示原始 stdin 或 stdout 字节变化，`cases=[]`。这次 artifact 未包含实际字节，不能把 BOM 当作已经从 runner 文件中测得的差异。原 artifact 和[补充 JSON](fixtures/windows-6635f9869-stdio-boundary.json)保留这一限制。

固定微软 referencesource 提交 `ec9fa9ae770d522a5b5f0607898044b7478574a3` 的 [Process.cs](https://github.com/microsoft/referencesource/blob/ec9fa9ae770d522a5b5f0607898044b7478574a3/System/services/monitoring/system/diagnosticts/Process.cs#L2153) 显示：`RedirectStandardInput=true` 会以 `Console.InputEncoding` 建立 StreamWriter，并立即设置 AutoFlush。[StreamWriter.Flush](https://github.com/microsoft/referencesource/blob/ec9fa9ae770d522a5b5f0607898044b7478574a3/mscorlib/system/io/streamwriter.cs#L306) 即使尚未写入文本，也可能写编码前导。原候选 launcher 随后直接 CopyTo BaseStream，并不能撤销已经写入的 BOM。这是该重定向路径的具体源码风险，仍不等于第四轮实际差异已定位。

最窄修复仅候选 `codex_windows_hook_command.ps1`：直接继承 stdin、stdout、stderr 的原始句柄，删除 StreamWriter 输入转发。没有修改用户/全局编码，没有剥除输入 BOM 或放宽预期字节。Python 验证器现在在失败时记录固定 case、期望/实际 stdin 与 stdout、stderr 的 Base64，区分前导字节、多余 CRLF 和未生成捕获文件。

本机 PowerShell 7.7.0-preview.3 原理对照确认：有 BOM 编码的 StreamWriter 在仅设置 AutoFlush 时已输出 `EF BB BF`；BaseStream 写入后该前导仍保留。直接继承句柄的本机原生 `cat` 则逐字保留 UTF-8、CRLF、NUL。它不是 Windows PowerShell 5.1 运行证据，补充 JSON 中 `native_windows_or_ps51_verified=false`。

候选新明文 LF UTF-8 SHA-256 为 `327603d72b007e9ea68a3902cee3f8b4cda4ebd70c6729826f58de79651d8952`，最长生成命令 7250 字符；编码与 8191/8192 静态边界回归保留。Windows helper 本机 15 项通过（0.022 秒），Grok 10 项通过（0.244 秒）。真实 cmd 边界、双向字节及原生 hook 必须由下一 Windows 提交继续验证。

这些修复已进入 `94a412eb89a4de57977072e6c45f4a692955d970`，第五轮只选择 Windows；本段不预报其结果。候选脚本仍在验证目录，未改产品安装门禁、随附插件、Rust 或 workflow；无需本地化变更。
