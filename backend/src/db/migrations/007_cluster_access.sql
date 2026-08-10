-- Per-user cluster membership / role (scopes global RBAC to specific clusters).
-- If a non-admin user has zero rows, they keep global role on all clusters (legacy).
-- If they have one or more rows, access is limited to those clusters with the granted role.

CREATE TABLE IF NOT EXISTS cluster_access (
    user_id TEXT NOT NULL,
    cluster_id TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, cluster_id)
);

CREATE INDEX IF NOT EXISTS idx_cluster_access_user ON cluster_access(user_id);
CREATE INDEX IF NOT EXISTS idx_cluster_access_cluster ON cluster_access(cluster_id);
