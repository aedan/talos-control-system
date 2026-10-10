-- Console settings. Secrets stay in vault_secrets. This table holds the
-- non-secret auth, TLS, and metal documents the UI used to write into
-- config.toml, tls.toml, and metal.toml.
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
