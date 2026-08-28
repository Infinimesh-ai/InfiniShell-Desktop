-- 一个 Project 可以配置多个 Git 仓库；仓库可进一步映射到项目已关联的 SSH 服务器。
CREATE TABLE infinishell_project_repositories (
  id         TEXT PRIMARY KEY NOT NULL,
  project_id TEXT NOT NULL REFERENCES infinishell_projects(id) ON DELETE CASCADE,
  git_url    TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0,
  UNIQUE (project_id, git_url)
);
CREATE INDEX idx_infinishell_project_repositories_project
  ON infinishell_project_repositories(project_id);

CREATE TABLE infinishell_project_repository_servers (
  repository_id TEXT NOT NULL REFERENCES infinishell_project_repositories(id) ON DELETE CASCADE,
  node_id       TEXT NOT NULL,
  sort_order    INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (repository_id, node_id)
);
CREATE INDEX idx_infinishell_project_repository_servers_node
  ON infinishell_project_repository_servers(node_id);

-- 把旧版单值 git_url 无损迁移为第一条仓库记录。旧列暂时保留，便于版本回退。
INSERT INTO infinishell_project_repositories (id, project_id, git_url, sort_order)
SELECT id || ':legacy-repository', id, trim(git_url), 0
FROM infinishell_projects
WHERE git_url IS NOT NULL AND trim(git_url) <> '';
