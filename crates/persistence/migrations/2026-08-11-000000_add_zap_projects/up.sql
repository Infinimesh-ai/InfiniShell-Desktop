-- 项目实体:聚合 SSH 服务器、Git 地址、项目规则/习惯,支撑项目级 Agent 模式。
CREATE TABLE zap_projects (
  id                 TEXT PRIMARY KEY NOT NULL,        -- uuid v4
  name               TEXT NOT NULL,
  git_url            TEXT,                             -- 仓库地址(展示 + 注入 Agent 上下文)
  root_path          TEXT,                             -- 可选本地目录;存在则 WARP.md 文件规则自动生效
  rules              TEXT NOT NULL DEFAULT '',         -- 项目级规则/配置习惯,直接注入 prompt
  notes              TEXT NOT NULL DEFAULT '',
  default_profile_id TEXT,                             -- 项目默认 AIExecutionProfile id(可空)
  sort_order         INTEGER NOT NULL DEFAULT 0,
  created_at         TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at         TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  deleted_at         TIMESTAMP                         -- 软删,为 gist 同步预留 tombstone
);

-- 项目 ↔ SSH 服务器 多对多。node_id 引用 ssh_nodes.id,但不加 FK:
-- ssh_* 表由 warp_ssh_manager 的独立连接写入,跨写路径的级联删除行为不可控,
-- 悬挂引用由读取侧过滤。
CREATE TABLE zap_project_servers (
  project_id TEXT NOT NULL REFERENCES zap_projects(id) ON DELETE CASCADE,
  node_id    TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, node_id)
);
CREATE INDEX idx_zap_project_servers_node ON zap_project_servers(node_id);
