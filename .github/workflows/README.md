# 发布配置

本目录的 `release_configurations.json` 供 [InfiniShell Release](infinishell_release.yml) 工作流使用。当前只发布 `oss` 通道，通过 `v*` tag push 或手动 `workflow_dispatch` 触发；配置由 `../actions/get_channel_config/action.yml` 读取。

工作流先构建 Windows/Linux 桌面包和各平台 SSH 扩展，再创建 Draft Release。本地签名、公证的 macOS arm64 DMG 加入同一候选后，仍需完成资产和适用验收门禁才能公开。CI 成功或 Draft 创建不等于发布完成。

## 字段

| 字段 | 当前用途 |
|------|----------|
| `channel` | 配置选择键，当前为 `oss` |
| `release_base_name` | GitHub Release 名称前缀 |
| `release_body_text` | 生成发布说明时使用的前置说明 |
| `type`、`is_prerelease`、`is_autopush` | 保留的上游兼容字段；当前 InfiniShell 工作流不据此调度发布或自动公开候选 |
| `changelog_slack_channel`、`gcs_cache_control_value`、`web_gcs_bucket_prefix` | 保留的上游兼容字段；当前 OSS 配置为空，工作流不发布到 Slack 或 GCS |

## macOS 架构范围

OSS 桌面发布只支持 Apple Silicon（`arm64`），资产名为 `InfiniShell-arm64.dmg`。macOS `x86_64` 仅保留 SSH 扩展，默认在 Intel runner 原生构建；可选 preflight 保留 Apple Silicon 交叉编译及 Intel 原生验收入口。该范围不代表 Intel macOS 桌面支持。
