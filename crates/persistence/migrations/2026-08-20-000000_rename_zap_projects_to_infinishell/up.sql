-- 保留已有项目数据，仅把 Zap 时期的表名迁移到当前 InfiniShell 命名。
ALTER TABLE zap_projects RENAME TO infinishell_projects;
ALTER TABLE zap_project_servers RENAME TO infinishell_project_servers;

DROP INDEX IF EXISTS idx_zap_project_servers_node;
CREATE INDEX idx_infinishell_project_servers_node
  ON infinishell_project_servers(node_id);
