# Windows 验收修复整合

本次从已合并 PR #22 的 `main` 提交 `3e962df37074684401d8ae83f31cc476c17f1249` 整合 Windows 验收中发现的有效修复。原验收分支 `codex/acceptance-windows-v01-v02-v05` 的记录提交为 `2dd17d9c8cc1088e916207f1b2cf48547a77a926`，受测源码为 `fc635a3dfe628c0493beb8a20db1adedd39d9779`。这些不同来源的验证结果不能互相替代。

## 整合范围

| 内容 | 原修复提交 |
|---|---|
| 开发数据 profile 的注册表偏好、URI 注册和窗口身份隔离 | `dba3c83dfa6beaca60c866bf1851651ecdd9d591`、`ab03a6151cae1b799931589fe361d86b85b83838` |
| CLI 启动 I/O 阶段诊断、严格 Windows Job 继承、Claude 策略预检 | `b6637c242f0501efd35070c5e6b84a25bdce3595`、`3c27fe1f67c701d96d3beede07958be6f8b04c87`、`bbab69e8a78eec76e8f1751fe7be6bc753ac1b35` |
| 权限拒绝的受控诊断 | `eeafcae260841e21eac56f48a45be21050539ca8` |
| 恢复前置拒绝时还原旧代状态、展示原生失败原因 | `2d7f79dd6fe2fc105cc8ed6186c19c137b350989`、`bbf1f1dba6b79f7cb49598d0cd10aacb22aa6943` |
| 显式选择私有 PowerShell 启动时隔离历史和 profile | `065ec83f8ba9d956e6fb5953004f755f7fa50ee9` |
| Windows 重命名缓冲区显式保留宽字符终止符 | `fc635a3dfe628c0493beb8a20db1adedd39d9779` |

产品和对应回归按上述提交的净差异整合，保留主线后续修改。没有整批移植旧验收 CI 包装脚本，也没有覆盖主线的 `CURRENT_STATUS.json`、G09 记录或历史验收结论。源码门禁使用当前工作流并隔离本轮源码、构建输出与测试配置，补上私有 PowerShell 和注册表偏好的定向回归。

PowerShell 私有启动只有本地 Windows 会话显式设置 `WARP_POWERSHELL_PRIVATE_STARTUP_ROOT` 时才启用，不是普通终端 PATH 修复。此前仅针对隔离环境提出的 PATH 补丁已按用户指示撤回，本次不重新引入。

## 2026-10-08 应用内复测补充

用户要求缩减为实际应用内启动和单轮模型回复，随后结束验收。以下结果来自原 Build11、源码 `fc635a3dfe628c0493beb8a20db1adedd39d9779`，应用 SHA-256 为 `e1a20ed40843fde49b0651aff81a90dd48602ffce4bd2b8739554d795e997bdd`，不是本次整合代码的 GUI 复验。

- 官方 Codex 0.156.1：收到 `CODEX_APP_RECHECK_OK`，原生会话 `01a117fa-bfe5-7a52-9375-0e3e8735f0cb`，消息 acknowledged、任务 completed；此前额度错误本次未出现。
- 官方 Grok 1.0.41：初始化及真实模型回复成功，收到 `GROK_APP_RECHECK_OK`，原生会话 `01a11802-0158-7420-b411-4aea25628ea4`，消息 acknowledged、任务 completed；此前初始化超时本次未复现，旧失败原因不回填为已解释。
- 两项均经实际应用断开。最终检查未发现本任务的私有 Codex/Grok 进程，空测试项目没有文件改动。此检查不替代完整 Windows Job 身份审计。

小型结果及原件摘要见 [cli-recheck.safe.json](cli-recheck.safe.json)。原件路径相对私有证据根 `windows-28d0922-20261006`；截图、数据库、日志、程序和凭据不提交 Git。原失败仍保留在[原验收记录](https://github.com/Infinimesh-ai/InfiniShell-Desktop/tree/2dd17d9c8cc1088e916207f1b2cf48547a77a926/specs/cli-agent-parity/windows-acceptance-20261007-progress)。四个历史临时工作区的未提交内容已另行归档，原工作区保持原样。

## 验收边界与本地化

- V01：Claude R04 的完整记录继续绑定原 Build09；Codex/Grok 本次只证明启动及单轮回复，完整生命周期仍未补齐。
- V02：用户已人工确认输入法没有问题；没有补录原条件所需的连续物理输入录像，不据此将证据合同标为完整通过。
- V05：保留原英/中、常规/紧凑静态检查；真实 Grok 审批待决、允许、拒绝的完整布局验收未完成。
- Linux/Windows 源码门禁不等于真实桌面验收，单平台结果也不关闭双平台缺口。

无需本地化变更：沿用中英文已有的断开、无效启动、权限上限和版本不可用消息；新增阶段名及权限分类属于受控诊断，原生 CLI 的失败文本保持原提供方内容。未新增界面控件、标签或布局，未声称重新完成双语 GUI 验收。整合提交的编译、i18n 和相关回归结果记录在独立 PR 与同源 CI 中，不借用历史绿色结果。

## 整合代码的本地门禁

以下结果在独立整合工作区执行，均记录编译输入前后摘要一致；大型日志和完整收据保存在私有证据根的 `integration-20261008/`。

| 门禁 | 结果 | 私有收据 |
|---|---|---|
| `cargo check --locked -p warp` | 通过 | `cargo-check-02.json` |
| `cargo test --locked -p warp --lib i18n::tests` | 11 通过 | `i18n-01.json` |
| Claude、Codex、Grok、coordinator、PowerShell 和 WinGet 定向回归 | 824 通过 | `app-focused-02.json` |
| `command` 默认生产 feature 的托管进程回归 | 7 通过，包含严格父 Job | `command-managed-01.json` |
| 注册表 profile 隔离回归 | 3 通过 | `registry-01.json` |
| 工作流 YAML、内嵌脚本语法、隔离范围、23 项契约测试 | 通过；actionlint 仍有 11 项基线诊断，新增 0 | `workflow-static.safe.json` |

首次定向回归因缺少 nextest 未执行测试，原失败收据 `app-focused-01.json` 保留。其后在证据目录私有安装并校验 nextest 0.9.148 后重跑通过。Linux/Windows 同提交源码门禁以独立 PR 关联运行的最终结果为准。
