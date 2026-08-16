CREATE TABLE ssh_routes (
  id                TEXT PRIMARY KEY NOT NULL,
  name              TEXT NOT NULL,
  target_node_id    TEXT REFERENCES ssh_nodes(id) ON DELETE SET NULL,
  created_at        TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at        TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_connected_at TIMESTAMP
);

CREATE TABLE ssh_route_hops (
  route_id        TEXT NOT NULL REFERENCES ssh_routes(id) ON DELETE CASCADE,
  position        INTEGER NOT NULL CHECK(position >= 0 AND position < 8),
  node_id         TEXT REFERENCES ssh_nodes(id) ON DELETE SET NULL,
  target_alias    TEXT NOT NULL CHECK(length(target_alias) > 0),
  port            INTEGER CHECK(port >= 1 AND port <= 65535),
  execution_scope TEXT NOT NULL CHECK(execution_scope = 'previous_hop'),
  PRIMARY KEY(route_id, position)
);

CREATE INDEX idx_ssh_routes_updated_at ON ssh_routes(updated_at);
CREATE INDEX idx_ssh_route_hops_node_id ON ssh_route_hops(node_id);
