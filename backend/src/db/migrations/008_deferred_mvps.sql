-- Rolling upgrade jobs
CREATE TABLE IF NOT EXISTS upgrade_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    scope TEXT NOT NULL,
    image TEXT NOT NULL,
    status TEXT NOT NULL,
    max_unavailable INTEGER NOT NULL DEFAULT 1,
    control_plane_last INTEGER NOT NULL DEFAULT 1,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    created_by TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS upgrade_job_targets (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL,
    cluster_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    address TEXT,
    machine_type TEXT,
    status TEXT NOT NULL,
    error TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_upgrade_targets_job ON upgrade_job_targets(job_id);
CREATE INDEX IF NOT EXISTS idx_upgrade_jobs_status ON upgrade_jobs(status);

-- Siderolink inventory (WG data path is later)
CREATE TABLE IF NOT EXISTS siderolink_peers (
    id TEXT PRIMARY KEY NOT NULL,
    system_uuid TEXT NOT NULL,
    public_key TEXT NOT NULL,
    assigned_ip TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS siderolink_join_tokens (
    token TEXT PRIMARY KEY NOT NULL,
    label TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

-- Greenfield config factory artifacts
CREATE TABLE IF NOT EXISTS provision_artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    cluster_id TEXT,
    name TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    kubernetes_version TEXT NOT NULL,
    secrets_enc TEXT,
    controlplane_config TEXT,
    worker_config TEXT,
    created_at TEXT NOT NULL
);
