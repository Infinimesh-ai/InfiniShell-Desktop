# macOS bundle7 插件持久来源、原生授权与双语说明验证

日期：2026-09-16。真实界面通过 CUA 操作；所有 CLI 配置、应用 profile 和可写 HOME 均独立于用户普通应用。此次没有修改生产源码、FTL、共享脚本或运行 Cargo。

## 构建身份与范围

- 源应用：`target/debug/bundle/osx/InfiniShell.app`，源执行文件 SHA256 `56e3ce6d304c7c2bb040222c8ca7ddd5ba7c978820c49765d2a757fbb8b48b68`。
- 实际主验证副本 bundle ID：`dev.infinishell.InfiniShellParityPTY7`。独立重签后执行文件 SHA256 `a1af85fb5f0948e7454b6bc8294f3564b817ec478afc1b7e3efbcc51c8af6dc0`。
- CLI：Codex 0.147.0，普通 PTY 启动命令为单独的 `codex`，没有额外 sandbox 或审批绕过参数。
- 使用既有私有测试 CODEX_HOME；仅既有授权的测试认证可用于唯一一次模型通知探测。原生 Hook 的信任全部由测试者在 Codex `/hooks` 逐项操作，未以文件写入代替原生审批。

完整分阶段配置摘要、各文件哈希和原生事件记录见 [生命周期证据](macos-gui-bundle7-codex-plugin-lifecycle.json)。旧 Git 来源重启回退失败的 `macos-gui-pty-codex-patch-reverted*`、`macos-gui-pty-codex-source-records.json`、`macos-gui-pty-codex-plugin-tree.json` 原样保留。

## 已通过的实际路径

1. GUI 的“更新 Warp 插件”完成，提示文件已更新，需在 Codex 检查 `/hooks`；工具栏变为待授权提示。
2. 原生 marketplace 从 Git 来源切到受控的本地 `plugins/infinishell-sources/codex-warp-0.4.0-rev3/source`。安装后缓存 10 个文件、持久来源 37 个文件形成固定摘要。
3. 安装前后到第一次原生重启，`trusted_hash` 数量始终为 0。退出 Codex 后重新单独启动，来源与缓存摘要不变，没有重现旧 Git 来源覆盖修补的现象。
4. 在 Codex 原生 `/hooks` 逐项核对来源 `warp@codex-warp` 和脚本命令，分别授权 PermissionRequest、PostToolUse、SessionStart、UserPromptSubmit、Stop。每项显示 Trusted，汇总显示 Installed 1 / Active 1。
5. 五次原生信任操作后，配置恰有五项 `trusted_hash`，与本轮受控原生定义摘要一致。再次正常退出/启动 Codex，没有重复弹出未信任提示。
6. 正常退出隔离应用（退出码 0）并重新启动后，缓存和持久来源摘要继续保持不变。

截图：[更新后待授权](macos-gui-bundle7-plugin-updated-awaiting-trust.png)、[授权前原生定义](macos-gui-bundle7-native-permission-hook-before-trust.png)、[五项原生 Active](macos-gui-bundle7-native-hooks-active.png)。

## 当前通知到达仍未通过

授权后在真实富输入中提交一次：`只回复 GUI_BUNDLE7_NOTIFY_ONE，不调用任何工具。`。原生模型输出 `GUI_BUNDLE7_NOTIFY_ONE`，界面草稿清空，但工具栏仍显示 `Authorize notifications`。原生 session 为 `01a0aa15-3f19-71f3-8a69-5b5c4c327cf3`，turn 为 `01a0aa16-f07a-7180-b159-7209607007c5`。

私有原生日志显示三组 `hook/started` / `hook/completed`，不含 HookRunSummary、退出码、stdout 或 stderr；这些字段属于未获得，不能写成“为空”。`/hooks` 定义详情也没有执行记录入口。目标 Codex 环境中 TMUX / TMUX_PANE 均为空或不存在。没有重复发送模型提示来制造通过结果。

[真实输出与待确认工具栏](macos-gui-bundle7-native-output-notification-unconfirmed.png)仅证明原生输出及当前 UI 展示，不足以判定 OSC 是否抵达。另一个独立真实 app-server/控制 PTY 探针已能收到同脚本 OSC，因此不能直接把失败归因于原生 runner 或 `/dev/tty`。

接收状态还存在需单独核实的可见性因素：原生授权的版本预检依赖仅在进程内保存的 `VERIFIED_RUNTIMES`；渲染中的 `Required / Unknown` 优先于既有富通知标记。应用重启或不同 PATH 可能导致 Unknown。此处只是源码定位线索，需要结合真实 hook outcome 与接收端日志继续确定，不作为已修复或已通过。

bundle7 的普通 PTY Stop 绿态另有已确认的过早成功缺陷；本报告不将其计为真正任务完成。

## 英文与简体中文布局

在 1229×768 窗口、约 614 像素的标准说明分屏中，以下六种真实页面均完整显示标题、说明、命令、复制控件和尾部提示，没有截断或重叠：

| 页面 | 英文 | 简体中文 |
|---|---|---|
| 原生授权 | [截图](macos-gui-bundle7-hooks-instructions-en.png) | [截图](macos-gui-bundle7-hooks-instructions-zh-CN.png) |
| 插件更新 | [截图](macos-gui-bundle7-plugin-update-instructions-en.png) | [截图](macos-gui-bundle7-plugin-update-instructions-zh-CN.png) |
| 插件安装 | [截图](macos-gui-bundle7-plugin-install-instructions-en.png) | [截图](macos-gui-bundle7-plugin-install-instructions-zh-CN.png) |

原生授权页面来自上面的真实已安装主现场，中文在正常重启后验证。手动安装/更新页面使用额外的无账号 HOME 隔离布局夹具：真实受测 Codex 通过自定义命令 `codex-layout` 启动停在登录页，旧版插件元数据只用于进入产品已有的手动更新入口；随后保留配置备份并切到缺插件状态以查看安装入口。没有复制账号、发起模型请求、执行安装命令或修改真实授权现场。两份布局夹具都正常退出，退出码 0。

实际页面显示 `python3 apply_notification_patch.py --agent codex`，只有复制控件。更新恢复提示现在要求先查看错误再重试并保留恢复文件，不再指示删除/重新添加 Git 来源。原生安装本身不会应用 InfiniShell 通知修补，以及 SSH/容器需在目标主机校验、Windows 自动修补尚不支持等边界在两种语言中均可见。布局验证不代替这些平台的功能验证。
