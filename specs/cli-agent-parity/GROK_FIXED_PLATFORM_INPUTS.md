# Grok 1.0.30 固定 Linux / Windows 验证输入

本次新增独立准备器 `script/cli-agent-parity/prepare_grok_cli.py`，供 Linux x64 / Windows x64 的原生验证使用。默认只在当前目标平台的私有目录执行 `--version`。macOS 仅完成两份官方下载文件的完整大小、摘要、格式与架构验证，没有执行 Linux / Windows 文件，没有调用模型或安装到用户目录。

## 固定来源与完整字节

官方[入门文档](https://docs.x.ai/build/overview)同时提供 Unix 与 PowerShell 安装入口。本次核对固定源码快照 `482711333c7195dc16a272777f86086d615e2afb` 中的 [install.sh](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager/scripts/install.sh#L174) 和 [install.ps1](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager/scripts/install.ps1#L291)：平台名为 `linux-x86_64` / `windows-x86_64`，带版本下载名称分别不带扩展名和带 `.exe`。固定源码的两份脚本与本次 `https://x.ai/cli/install.sh`、`install.ps1` 下载字节相同；没有运行这些安装器。

| 目标 | 固定官方文件 | 完整大小（字节） | SHA-256 |
| --- | --- | ---: | --- |
| Linux x64 | [grok-1.0.30-linux-x86_64](https://x.ai/cli/grok-1.0.30-linux-x86_64) | 161725088 | `504dd6546ab991b75d36698242875ce461489cd1f8cd84285873cb55bd5c7d54` |
| Windows x64 | [grok-1.0.30-windows-x86_64.exe](https://x.ai/cli/grok-1.0.30-windows-x86_64.exe) | 150036808 | `ca24ea63272ba7881261f4a52498d1f5bd884b01da25845990422a10dd315266` |

这些 SHA-256 是对固定官方 HTTPS 文件完整下载后的观测摘要，不是官方签名清单。固定 [version.rs](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-update/src/version.rs#L27) 本身也说明其下载校验依靠运行 smoke，不能把我们的 pin 写成官方签名验证。源码快照提交不是二进制构建提交；两份文件中均能找到字符串 `04b7ffed98c6`，但这仍不替代目标平台实际 `--version`。

两份文件均为裸可执行文件，没有 ZIP/TAR 或归档成员需要解包：

- Linux：ELF64、小端、`e_machine=62`、PIE；`file` 报告静态 PIE，`objdump -p` 未见 `PT_INTERP` 或 `DT_NEEDED`。
- Windows：PE32+、`Machine=0x8664`、console subsystem；静态导入列表已记录，有延迟导入目录。这里只检查文件结构，没有在 Windows 加载、校验证书或验证 DLL 可用性。

完整下载响应的非敏感头、两份安装源码摘要和静态检查结果见 [grok-1.0.30-fixed-platform-inputs.json](fixtures/grok-1.0.30-fixed-platform-inputs.json)。不依赖 `latest`、`stable` 指针或第三方重打包；下载重定向也会拒绝，不能静默切换来源。

## 准备器边界与执行命令

只需当前任务中的 Python 3.11+ 标准库。下载目录默认必须是 `RUNNER_TEMP` 的独立子目录、位于源树外，且为空或具有本准备器的版本标记。Unix 要求私有目录权限；Windows 依赖 runner 私有工作目录，不把环境变量重定位当成 KnownFolder 沙箱。

Linux：

```sh
python script/cli-agent-parity/prepare_grok_cli.py --download-dir "$RUNNER_TEMP/grok-cli-fixed"
```

Windows PowerShell：

```powershell
python script/cli-agent-parity/prepare_grok_cli.py --download-dir (Join-Path $env:RUNNER_TEMP 'grok-cli-fixed')
```

成功时 stdout 仅输出绝对可执行文件路径，适合已有 workflow 收集。目录中保留 `grok` 或 `grok.exe`，以及 `grok-fixed-inputs.json`。正常模式必须退出 0 且报告 `grok 1.0.30 (04b7ffed98c6)` 才记录 `native_version_verified=true`；下载成功不能替代它。

准备器为 `--version` 创建全新的 HOME、USERPROFILE、APPDATA、LOCALAPPDATA、XDG、GROK_HOME、CODEX_HOME、CLAUDE_CONFIG_DIR 和临时目录；只保留 PATH、Windows 系统命令位置及语言环境，不继承令牌、API key、代理、用户配置位置或动态加载器注入变量。命令以固定 argv 执行，无 shell。输入文件校验拒绝链接、重解析点、硬链接及摘要不匹配的旧缓存；新下载完整核对大小与摘要，先在同目录写临时文件并 fsync，再原子创建目标，失败不覆盖已有文件。

macOS 允许显式只下载，不允许选择其他平台后执行：

```sh
python script/cli-agent-parity/prepare_grok_cli.py --private-directory /tmp/grok-linux-private --download-only --target linux-x64
python script/cli-agent-parity/prepare_grok_cli.py --private-directory /tmp/grok-windows-private --download-only --target win32-x64
```

此模式的 `native_version_verified` 必为 false。本次实际运行了这两个准备器路径，完整字节核验通过；不作为同提交 Linux / Windows 运行证据。

## 原生后续验证与组件边界

Windows 官方安装器还把 `grove.exe`、`grove-fsmonitor.exe`、`grove-credential.exe` 和 MinGit 作为 [best-effort 配套下载](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager/scripts/install.ps1#L127)。本准备器没有下载、安装或验证这些组件，也没有添加系统 PATH。是否阻断特定原生能力必须由真实 Windows 结果判断；不能由 `--version` 成功推定完整终端或插件功能已通过。

已有独立入口 `probe_protocol.py --cli grok --executable <绝对路径> --output <证据路径>` 的 Grok 分支发送 `initialize`、无认证 `session/new`、缺失历史 `session/load` / `session/resume`，不发送 `authenticate` 或 `session/prompt`。其 macOS 真实记录见 [grok-1.0.30-handshake.ndjson](fixtures/grok-1.0.30-handshake.ndjson)：ACP v1、仅 `grok.com` 认证方式、`session/new` 的 `-32000 Authentication required`，以及缺失历史错误。这是候选后续协议边界，不在准备器中增加 ACP 开关。

该旧 probe 不能仅凭进程退出 0 算通过：runner 还须明确核对上述响应、所有请求都有响应，以及私有 HOME / GROK_HOME 和独立 leader endpoint 实际生效；其默认环境过滤不等同本准备器的完整私有环境。Linux / Windows 尚未取得这些结果，Windows 也未确认该入口的 leader endpoint 与进程清理行为。本次不改旧 probe、不扩展认证或模型测试、不解除产品 Grok 托管执行门禁。

## 本地回归

```sh
python3 -B -m unittest discover -s script/cli-agent-parity -p prepare_grok_cli_tests.py -v
```

本机 10 项通过：固定下载证据、实际 ELF/PE 头与错误架构、环境隔离、目录归属、原子下载/重复不改、短下载/超长/错误摘要/重定向、旧缓存保留、硬链接拒绝、固定 `--version` argv，以及只下载模式不会执行外平台文件。版本调用单测使用受控替身，只验证调用约束；不能冒充真实原生 `--version`。

未运行 Cargo，未改 workflow、Rust、插件资源、用户配置或 GUI。新增内容是验证脚本与技术证据，无需本地化变更。

根代理已将准备器、对应 10 项回归和原生版本报告上传路径接入 Linux/Windows workflow，Python 回归及 actionlint 通过。此改动在 328d5ed35 之后，当前运行 35110822335 不包含它；必须等待后续实际提交运行，不计本轮目标平台原生通过。
