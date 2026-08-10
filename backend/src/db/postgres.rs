//! Experimental single-instance PostgreSQL support.
//!
//! Full dual-backend (every repo query on `PgPool`) is staged: this module
//! validates connectivity and applies a Postgres-compatible schema so HA
//! deployments can prepare a database. Application runtime still uses SQLite
//! until remaining `?` placeholders are dialect-abstracted.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::DatabaseConfig;
use crate::AppError;

/// Connect to Postgres and verify the URL is usable.
pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, AppError> {
    if config.postgres_url.trim().is_empty() {
        return Err(AppError::Config(
            "database.backend = \"postgres\" requires database.postgres_url".into(),
        ));
    }
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.postgres_url)
        .await
        .map_err(|e| AppError::Config(format!("PostgreSQL connection failed: {}", e)))?;
    Ok(pool)
}

/// Apply a consolidated schema suitable for a greenfield Postgres instance.
/// Idempotent (`IF NOT EXISTS`).
pub async fn run_schema(pool: &PgPool) -> Result<(), AppError> {
    let sql = r#"
CREATE TABLE IF NOT EXISTS _tcs_migrations (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS clusters (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    control_plane_version TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unknown',
    control_plane_size INTEGER NOT NULL DEFAULT 1,
    worker_size INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS machines (
    id TEXT PRIMARY KEY,
    system_uuid TEXT NOT NULL UNIQUE,
    machine_type TEXT NOT NULL DEFAULT 'worker',
    cluster_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    talos_version TEXT NOT NULL DEFAULT '',
    secure_boot INTEGER NOT NULL DEFAULT 0,
    siderolink_connected INTEGER NOT NULL DEFAULT 0,
    address TEXT NOT NULL DEFAULT '',
    hostname TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'reader',
    is_active INTEGER NOT NULL DEFAULT 1,
    password_hash TEXT,
    auth_provider TEXT NOT NULL DEFAULT 'local',
    ldap_dn TEXT,
    password_needs_change INTEGER NOT NULL DEFAULT 0,
    last_login TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tenant_branding (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL UNIQUE,
    name TEXT,
    short_name TEXT,
    tagline TEXT,
    primary_color TEXT,
    secondary_color TEXT,
    background_color TEXT,
    surface_color TEXT,
    text_color TEXT,
    text_muted_color TEXT,
    font_family TEXT,
    docs_url TEXT,
    support_url TEXT,
    logo_path TEXT,
    favicon_path TEXT
);

CREATE TABLE IF NOT EXISTS upgrade_jobs (
    id TEXT PRIMARY KEY,
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
    id TEXT PRIMARY KEY,
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

CREATE TABLE IF NOT EXISTS siderolink_peers (
    id TEXT PRIMARY KEY,
    system_uuid TEXT NOT NULL,
    public_key TEXT NOT NULL,
    assigned_ip TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS siderolink_join_tokens (
    token TEXT PRIMARY KEY,
    label TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provision_artifacts (
    id TEXT PRIMARY KEY,
    cluster_id TEXT,
    name TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    kubernetes_version TEXT NOT NULL,
    secrets_enc TEXT,
    controlplane_config TEXT,
    worker_config TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cluster_access (
    user_id TEXT NOT NULL,
    cluster_id TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, cluster_id)
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    action TEXT NOT NULL,
    resource TEXT,
    details TEXT,
    created_at TEXT NOT NULL
);
"#;

    for stmt in sql.split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        sqlx::query(stmt)
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e))?;
    }

    let applied: Option<String> = sqlx::query_scalar(
        "SELECT name FROM _tcs_migrations WHERE name = $1",
    )
    .bind("postgres_schema_v1")
    .fetch_optional(pool)
    .await?;

    if applied.is_none() {
        sqlx::query("INSERT INTO _tcs_migrations (name) VALUES ($1)")
            .bind("postgres_schema_v1")
            .execute(pool)
            .await?;
        tracing::info!("PostgreSQL schema bootstrap applied (postgres_schema_v1)");
    }

    Ok(())
}
