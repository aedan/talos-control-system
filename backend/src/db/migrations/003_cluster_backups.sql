-- Talos Control System Database Migrations
-- Migration 003: Cluster backups table

CREATE TABLE IF NOT EXISTS cluster_backups (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    file_path TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (cluster_id) REFERENCES clusters(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_cluster_backups_cluster_id ON cluster_backups(cluster_id);
CREATE INDEX IF NOT EXISTS idx_cluster_backups_status ON cluster_backups(status);
