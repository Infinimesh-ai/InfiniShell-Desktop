# Windows 安装器重发与验收

本次修复仅更新旧版安装、卸载的 PATH 归属逻辑，应用版本仍为 `v0.2026.10.14.16.45.oss_00`，应用源提交为 `5068dae9c75b79628865a0872684113647c204d8`。原发行安装器及全部历史证据保留，不覆盖旧资产、不移动旧标签。无需本地化变更。

## 本地已验候选

- 原发行 run：`38537451605`，attempt 1；原 Setup SHA-256：`1ad878fb098c8cc7e49c7dc29aa6afec7ceb34d80fcb9fc8220643f35ec3086a`。
- 修复 helper SHA-256：`dea6f701a22b3dff9197401410ddedeaaa70d40c909fc5e7ee01b6626e322cfd`。
- 本地重发 Setup：580325911 字节，SHA-256 `52c987801e27310ffb08351d13c255ca114f05a40d8e0ccb815b638b042a0cc7`，Authenticode `NotSigned`。
- 150 项应用载荷与原包一致；六阶段桌面验收通过，详情见[执行宿主及回归记录](windows-acceptance-host-context.zh-CN.md)。

本地通过不等于远端来源验证完成。只有远端实际输出、工件提供方摘要、准确提交及完整证据均校验后，才能记录对应发布候选通过。源文件时间等打包元数据可能改变 Setup 摘要；CI 分别记录 150 文件内容一致、与已验包摘要是否一致，以及是否需要新的桌面验收。若远端 Setup 与已验本地候选不同，必须保留差异并重新验收实际交付包；不得只凭相同版本复用旧结论。CI 构建与回归通过不自动改变 `desktop_six_stage_acceptance` 状态。

## 最小远端门禁

在本地 `cargo check -p warp`、`cargo test -p warp --lib i18n::tests` 和两套原生 PATH 回归通过后，将确切提交推送到独立分支。通过唯一 preflight 控制面执行：

```sh
gh workflow run cross-platform-preflight.yml --ref <已推送修复分支> \
  -f run_linux=false -f run_windows=true -f full_workspace_tests=false \
  -f windows_atomic_debug_scope=reissue-installer:38537451605
```

该模式先复用 `verify_release_installer.ps1` 验证旧版正式 CI 原件、成功 producer、annotated tag、提供方 ZIP 摘要和内嵌 SSH 资源，再由 `reissue_release_installer.ps1`：

1. 下载并核验固定 Inno Setup 6.7.1，仅解包工具，不在 runner 安装产品。
2. 在 PowerShell 5.1 运行完整 PATH 混用及未来产品重发回归；检查真实 PATH 未变化、预期负对照和清理结果。失败证据及负对照隔离键保留。
3. 从原包提取应用载荷，以当前提交的安装、卸载逻辑重编，保留旧版应用版本。
4. 解出重发包，逐项验证 150 文件及原始 manifest，记录安装器真实签名状态和源码、编译器、回归、原件绑定。
5. 分别上传独立重发包及回归证据，使用 run ID、attempt 和源码提交命名；不修改源码、不提交、不推送、不覆盖任何 Release 资产。

Windows 安装脚本是本次唯一产品改动平台，旧应用及其它平台载荷不重新编译；Linux/macOS 历史验收不能被称为该修复的新增实测。远端 runner 不代替普通 Explorer 桌面验收。

## 发布边界

独立重发包使用不同资产名称和来源绑定，原 Draft 的 12 件资产保持不变。Windows 修复候选的正式验收与全平台 Draft 公开是不同状态：公开前仍需明确选择正确的 Windows 资产，保留其它平台已有门禁，不把未签名包标为已签名。原 R4 失败对照、任意覆盖升级和同时安装竞争的边界继续保留。
