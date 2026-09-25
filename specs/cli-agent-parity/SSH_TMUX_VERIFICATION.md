# SSH／tmux 验证结论

固定版本的现有覆盖是 macOS 到本机隔离 OpenSSH，包含三款 CLI 的原生通知和 tmux 断连重接；不能外推到远端 Linux、Windows、容器或 WSL 的全部组合。

`allow-passthrough=on` 由用户在目标 tmux 会话显式配置，安装器不改全局 tmux 配置。Codex 关闭透传时尚缺内层原生发送的独立证据；外层零接收不能单独证明正确拦截。远程图片传输仍有 G08 缺项。

当前结论见[验证报告](VALIDATION_REPORT.md)，补验与关闭条件见[统一缺项](KNOWN_GAPS.md)。早期脚本级正负例见[固定提交历史原文](https://github.com/Infinimesh-ai/InfiniShell-Desktop/blob/e3ef39689cd8b686dfe040b90217ed1067b4d268/specs/cli-agent-parity/SSH_TMUX_VERIFICATION.md)，不代替原生 CLI 或完整产品链。
