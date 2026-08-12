在当前项目的多台 SSH 主机上批量执行同一条 shell 命令(仅项目会话可用)。

适用场景:项目上下文(<project_context>)列出了多台主机,需要跨主机做部署、巡检、批量改配置等操作。单主机操作请继续用 run_shell_command。

参数说明:
- node_ids:目标主机的 node_id 列表,必须取自 <project_context> 中主机清单的 node_id 字段,一次最多 20 台。
- command:在每台主机上执行的完整命令行。逐台串行执行。
- canary(默认 true):金丝雀模式。先在列表第一台执行,若失败(非零退出码 / 错误 / 超时)则中止其余主机,剩余主机标记为 canary_aborted。确认无风险后可显式传 false 全量执行。
- timeout_seconds(默认 120,上限 600):单台主机的超时时间,超时返回当前输出快照。

行为约定:
- 未打开的主机会自动新开 SSH 会话页签并等待连接就绪(最多 60 秒),就绪失败标记为 session_not_ready。
- 目标主机上正有长时间运行的命令时不会插入执行,标记为 busy。
- 命令避免使用 pager(git log、man 等请加 | cat)与交互式程序,否则会等到超时。

输出结构(JSON):
{"status": "ok"|"error", "canary_aborted": bool, "results": [{"node_id", "host", "status": "ok"|"error"|"timeout"|"busy"|"session_not_ready"|"canary_aborted", "exit_code", "output", "duration_ms"}]}
其中 output 超过 10000 字符会被截断并附中文截断标记。
