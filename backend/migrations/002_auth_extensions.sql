-- Talos Control System Database Migrations
-- Migration 002: Auth extensions and certificate management

ALTER TABLE users ADD COLUMN password_hash TEXT;
ALTER TABLE users ADD COLUMN auth_provider TEXT NOT NULL DEFAULT 'local';
ALTER TABLE users ADD COLUMN ldap_dn TEXT;
ALTER TABLE users ADD COLUMN password_needs_change INTEGER NOT NULL DEFAULT 0;

CREATE TABLE refresh_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_refresh_tokens_user_id ON refresh_tokens(user_id);

CREATE TABLE certificates (
    id TEXT PRIMARY KEY,
    domains TEXT NOT NULL,
    issuer TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'letsencrypt',
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_renewal_attempt TEXT,
    renewal_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_certificates_expires_at ON certificates(expires_at);
CREATE INDEX idx_certificates_issuer ON certificates(issuer);
