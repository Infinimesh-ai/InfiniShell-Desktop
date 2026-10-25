# Windows 安装器重发约定

重发用于保持应用载荷、仅修复安装/卸载逻辑的场景。应用源码与安装器控制器源码分别绑定；相同应用版本不代表安装器字节相同。正常新版本构建使用[Windows 打包入口](../script/windows/README.md)。

## 控制器范围

现有 [reissue_release_installer.py](../script/windows/reissue_release_installer.py) 固定了 `APP_SOURCE`、`APP_VERSION`、原包/安装脚本摘要、编译器和载荷数量。这是受限重发控制器，**不能只更换 run ID 就用于任意新版本**。扩大适用范围前先审核并更新相应来源合同与回归，不删除核验条件来绕过拒绝。

相关入口：

- [verify_release_installer.ps1](../script/windows/verify_release_installer.ps1)：核验正式 CI 原件、producer、tag、ZIP 摘要及内嵌资源，不运行产品。
- [reissue_release_installer.ps1](../script/windows/reissue_release_installer.ps1)：要求正式仓库环境、精确提交及 Windows PowerShell 5.1；输出目录必须是新建普通路径。
- [PATH 原生回归](../script/windows/test_windows_environment.ps1)与[重发回归](../script/windows/test_windows_environment_reissue.ps1)：分别核对共享基线/旧逻辑混用与重发场景。
- [跨平台 preflight](../.github/workflows/cross-platform-preflight.yml)：`release-installer:<runid>` 只读核验；`reissue-installer:<runid>` 仅用于满足控制器固定合同的来源。

## 执行顺序

1. 核对产品变更范围、原件来源和输出身份，先通过适用的本地检查、i18n 门禁及原生回归。
2. 使用已核实的远端修复提交和来源 run 运行对应 preflight；不在 runner 上安装产品或替代交互桌面验收。
3. 验证编译器来源与签名，从原包提取载荷，使用本轮安装逻辑重编；原包、失败和负对照分别保留。
4. 解出重发包，对全部应用/资源载荷及 manifest 逐文件比较。记录实际 Authenticode，未签名仍为 `NotSigned`。
5. 记录新 Setup 摘要及其与已验包的差异。字节不同必须重新绑定实际交付包并执行适用[桌面六阶段](windows-final-installer-acceptance.zh-CN.md)，不能只凭同版本或应用载荷相同复用安装器结论。

## 发布和恢复边界

重发包及证据以独立身份保存，不自动覆盖发布页、移动标签或公开候选。源码门禁、CI 产物、实际桌面验收和公开发布分别记录；当前正式状态见[发布入口](releases/README.zh-CN.md)。

旧卸载逻辑不会因新包出现自动修复。未来产品使用的安装/卸载逻辑必须满足同一归属合同；任意旧包覆盖升级、全部旧日志迁移、管理员安装和并发安装需要独立证据。失败或归属未知时保留恢复资料，不为获得通过而清理现场。
