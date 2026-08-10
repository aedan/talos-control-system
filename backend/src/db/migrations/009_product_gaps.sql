-- HA leadership, shared OIDC state, provision jobs

CREATE TABLE IF NOT EXISTS ha_locks (
    lock_name TEXT PRIMARY KEY NOT NULL,
    owner_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS oidc_states (
    state TEXT PRIMARY KEY NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provision_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    cluster_id TEXT,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    desired_workers INTEGER NOT NULL DEFAULT 0,
    payload TEXT,
    error TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provision_jobs_status ON provision_jobs(status);
CREATE INDEX IF NOT EXISTS idx_oidc_states_expires ON oidc_states(expires_at);
