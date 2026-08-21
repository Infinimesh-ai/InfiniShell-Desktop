DROP INDEX IF EXISTS idx_infinishell_project_servers_node;

ALTER TABLE infinishell_project_servers RENAME TO zap_project_servers;
ALTER TABLE infinishell_projects RENAME TO zap_projects;

CREATE INDEX idx_zap_project_servers_node ON zap_project_servers(node_id);
