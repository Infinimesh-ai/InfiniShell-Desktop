# Claude 2.1.273 固定原生文件与无凭据控制握手验证

记录日期：2026-09-16。此验证只覆盖固定原生文件的来源、版本、`initialize` 控制响应和空闲 stdin EOF 退出。它不验证生产 Rust 适配器、模型回合、工具子进程或完整应用生命周期。

## 固定发布输入

官方[安装文档](https://code.claude.com/docs/en/setup)给出的原生下载源是 `downloads.claude.ai/claude-code-releases`。本次固定版本为 `2.1.273`，发布清单 commit 为 `d48ecfd7a41c16c42e0564f7a94947d6e4c50db1`，buildDate 为 `2026-09-15T17:19:22Z`。不使用 `latest`、安装器或 npm，也不修改已有 CLI。

| 平台 | 官方固定文件 | 字节数 | SHA256 |
| --- | --- | ---: | --- |
| Linux x64 | [linux-x64/claude](https://downloads.claude.ai/claude-code-releases/2.1.273/linux-x64/claude) | 228663608 | `6c752e2cc7c110c9df15f26d8d134d438c5ae95dbd610efc1a308bf7f9c5f6c1` |
| Windows x64 | [win32-x64/claude.exe](https://downloads.claude.ai/claude-code-releases/2.1.273/win32-x64/claude.exe) | 231776416 | `19654006672b6da7c945115eea99ca10051796016df563a65b3f0c7d72720ef0` |
| macOS arm64，本次已有原生文件 | [darwin-arm64/claude](https://downloads.claude.ai/claude-code-releases/2.1.273/darwin-arm64/claude) | 212228880 | `953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb` |

原始[发布清单](https://downloads.claude.ai/claude-code-releases/2.1.273/manifest.json)为 2161 字节，SHA256 为 `02aa2311fd5d9a4cc9a5aea017b89f5067eec78013bd340f62450719a5393fca`。清单、[分离签名](https://downloads.claude.ai/claude-code-releases/2.1.273/manifest.json.sig)和[官方公钥](https://downloads.claude.ai/keys/claude-code.asc)的原始内容已保存在 fixtures 中。

本机已下载完整 Linux 与 Windows 文件并分别核对字节数和摘要。文件类型检查显示 Linux 文件为 x86-64 ELF、使用 `/lib64/ld-linux-x86-64.so.2`；它是 glibc 资产，不能据此声称支持 Alpine/musl。Windows 文件为 x86-64 PE32+。这两个文件尚未在对应操作系统执行，也未验证 Windows Authenticode 或运行时系统依赖。

本次已验证清单签名：固定 OpenPGP v4 RSA 公钥指纹为 `31DD DE24 DDFA B679 F42D 7BD2 BAA9 29FF 1A7E CACE`，与官方文档一致。解析分离签名的 SHA512 二进制文档签名，重建签名数据后，系统 OpenSSL 返回 `Verified OK`、退出码 0。未导入用户 keyring。准备器运行时使用本次已验签清单的固定 SHA256 和内置二进制摘要，不在每次下载时额外依赖 GPG。可用独立私有 GNUPGHOME 对随附 key、sig 和 manifest 再做 GPG 验证。

对应证据：

- `fixtures/claude-2.1.273-release-manifest.json`
- `fixtures/claude-2.1.273-release-manifest.json.sig`
- `fixtures/claude-release-signing-public-key.asc`
- `fixtures/claude-2.1.273-release-signature-evidence.json`
- `fixtures/claude-2.1.273-release-download-evidence.json`

## 准备器与隔离约束

`script/cli-agent-parity/prepare_claude_cli.py` 需要 Python 3.11 或更新版本，只支持当前宿主为 Linux x64 或 Windows x64。版本、平台摘要和源地址不能通过调用参数覆盖。

下载目录必须是 `RUNNER_TEMP` 的子目录，或通过 `--private-directory` 显式指定源树外的新私有目录。准备器只复用自己标记的目录；拒绝已有陌生内容、符号链接、Windows 重解析点和多硬链接文件。Unix 私有目录须为当前用户独占权限。Windows 依赖 runner 私有临时目录本身的访问控制，不替用户修改系统 ACL。

下载先写临时文件并核对完整大小、摘要，再原子创建目标；摘要不符、目标内容被修改或不再可执行时直接失败，不覆盖用户内容。执行前后都核对原生文件，`--version` 必须精确返回 `2.1.273 (Claude Code)`。标准输出仅返回绝对可执行文件路径；下载目录的 `claude-fixed-inputs.json` 记录版本、源和摘要，明确 `installed=false`。

准备器的版本检查和控制探测均使用新的 HOME、USERPROFILE、APPDATA、LOCALAPPDATA、XDG 目录、CLAUDE_CONFIG_DIR、临时目录与空 settings。环境只继承运行进程所需的系统路径/语言字段，不复制账号文件，不继承 API key、token、代理或用户 CLI 配置变量；禁用更新和非必要流量。此步骤无需 Node/npm。

## 无凭据探测的成功条件

`script/cli-agent-parity/probe_claude_no_credentials.py` 使用固定 SHA256 的真实原生文件，参数固定为：

```text
--print --input-format stream-json --output-format stream-json --verbose
--permission-prompt-tool stdio --permission-prompts host
--setting-sources "" --strict-mcp-config --mcp-config {"mcpServers":{}}
```

唯一输入是带随机 request ID 的 `control_request/request.subtype=initialize`。必须在 30 秒内收到唯一 `control_response`，精确关联该 ID，并满足：

- `response.subtype=success`；响应中的整数 PID 等于本次持有的原生进程 PID。
- `account.tokenSource=none`、`account.apiProvider=firstParty`。
- `session_state=idle`、`current_permission_mode=default`。
- 待审批和待用户对话列表均为空；不出现原生 session ID。

成功响应后必须确认进程仍活着，再关闭 stdin；原生进程须在 5 秒内以退出码 0 自行退出。stdout 必须完整读完且恰好只有该响应；额外/重复响应、无效 UTF-8、超长输出和任何通道的 `user`、`assistant`、`result`、`stream_event` 都失败。超时后的自持子进程 kill/reap 仅用于清理，不能算 EOF 验证通过。

报告记录当前 checkout 的提交和 dirty 状态、原生 PID/request ID、真实退出时长、强制清理状态及脱敏控制消息。命令/模型/agent 内置目录只保留条目数量。缺少进程结果或只执行了测试代码不能计成功；不会把 `initialize` 的能力目录当作已关联会话或真实模型执行。

## 本机结果与待验证边界

macOS arm64 已使用已有受测原生二进制的独立普通文件副本执行，未复制认证或 GUI 配置。最后一次真实结果保存在 `fixtures/claude-2.1.273-no-credentials-initialize-eof-macos.json`：无账号、默认权限、空闲初始化成功；stdin EOF 后 19 ms 自行退出，退出码 0，没有强制清理，执行前后摘要一致。

这份证据的 checkout HEAD 是 `6921a9925955a1955503e259cd935eaea4ac2ac0` 且 `worktree_dirty=true`，不属于最终同提交跨平台验收。新增两个 Python 测试文件各 6 项，本机共 12 项通过；其中用于验证 UTF-8/EOF 收集器的 Python 子进程是明确的传输夹具，不计为原生 CLI 验证。

Linux/Windows 目前只有真实下载摘要证据，原生 `--version`、上述控制握手和 EOF 仍待对应 runner 执行。没有账号不是这项无模型探测的预期前置阻塞；平台依赖、固定版本行为差异或网络下载失败必须按实际失败报告，不允许跳过后算通过。Windows 不需要为此握手先配置 Git Bash；是否能在实际 runner 直接完成仍由该平台结果确认。

这项验证不覆盖：生产 Rust adapter/监督 worker、活跃回合 EOF、CLI 崩溃、工具子孙退出、权限允许/拒绝、追加指令、取消、同 session ID 继续、应用重启恢复、结果领取、真实 PTY/ConPTY 或 GUI/TUI。它不能替代 P0–P5 完整生命周期验收。无产品可见文案变化，无需本地化变更；本子任务未运行 Cargo 或远端任务。

## 同提交 runner 接入命令

在目标提交的仓库根目录执行，保存下载目录的 `claude-fixed-inputs.json` 和探测 JSON。Python 命令必须指向 3.11+；不能仅凭退出后的空日志判定通过。

Linux Bash：

```bash
set -euo pipefail
export PYTHONUTF8=1
python3 -B script/cli-agent-parity/prepare_claude_cli_tests.py
python3 -B script/cli-agent-parity/probe_claude_no_credentials_tests.py
claude_executable="$(python3 -B script/cli-agent-parity/prepare_claude_cli.py --download-dir "$RUNNER_TEMP/claude-2.1.273-inputs")"
python3 -B script/cli-agent-parity/probe_claude_no_credentials.py \
  --executable "$claude_executable" \
  --output "$RUNNER_TEMP/claude-no-credentials.json"
```

Windows PowerShell：

```powershell
$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
python -B script/cli-agent-parity/prepare_claude_cli_tests.py
if ($LASTEXITCODE -ne 0) { throw 'Claude 准备器测试失败' }
python -B script/cli-agent-parity/probe_claude_no_credentials_tests.py
if ($LASTEXITCODE -ne 0) { throw 'Claude 控制探测测试失败' }
$claudeExecutable = python -B script/cli-agent-parity/prepare_claude_cli.py --download-dir "$env:RUNNER_TEMP\claude-2.1.273-inputs"
if ($LASTEXITCODE -ne 0) { throw '固定 Claude 原生文件准备失败' }
python -B script/cli-agent-parity/probe_claude_no_credentials.py `
  --executable "$claudeExecutable" `
  --output "$env:RUNNER_TEMP\claude-no-credentials.json"
if ($LASTEXITCODE -ne 0) { throw 'Claude 无凭据 initialize/EOF 失败' }
```

两平台都必须保留失败报告；完整远端运行前，本表相应平台仍为待验。
