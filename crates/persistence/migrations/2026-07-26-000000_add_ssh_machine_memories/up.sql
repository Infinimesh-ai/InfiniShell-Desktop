CREATE TABLE ssh_machine_memories (
    machine_key    TEXT PRIMARY KEY NOT NULL,  -- 归一化 "host:port"
    content        TEXT NOT NULL DEFAULT '',   -- Markdown 记忆全文
    hostname_alias TEXT DEFAULT NULL,          -- DCS 回报的远端真实 hostname（可空）
    ssh_node_id    TEXT DEFAULT NULL,          -- 可选关联 ssh_servers.node_id
    last_review_at TEXT DEFAULT NULL,          -- 上次后台复盘时间（RFC3339）
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
