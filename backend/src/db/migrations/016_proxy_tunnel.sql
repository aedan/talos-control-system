-- Remote OOB proxy tunnel: join tokens + per-machine proxy routing.

ALTER TABLE machines ADD COLUMN proxy_id TEXT;

CREATE TABLE IF NOT EXISTS proxy_join_tokens (
    token TEXT PRIMARY KEY NOT NULL,
    label TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_machines_proxy ON machines(proxy_id);
