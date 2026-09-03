pub mod migrate_sqlite_to_postgres;
pub mod models;
pub mod pool;
pub mod postgres;
pub mod repos;

pub use pool::{DbPool, SqlVal};

use crate::config::DatabaseConfig;
use crate::AppError;

pub type Pool = DbPool;

pub async fn init_pool(config: &DatabaseConfig) -> Result<DbPool, AppError> {
    pool::connect(config).await
}

pub async fn run_migrations(pool: &DbPool) -> Result<(), AppError> {
    // Tracking table (dialect-aware)
    let create_tracking = match pool.backend() {
        crate::config::DatabaseBackend::Sqlite => {
            "CREATE TABLE IF NOT EXISTS _tcs_migrations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            )"
        }
        crate::config::DatabaseBackend::Postgres => {
            "CREATE TABLE IF NOT EXISTS _tcs_migrations (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )"
        }
    };
    pool.execute(create_tracking, &[]).await?;

    let migrations: &[(&str, &str)] = &[
        ("001_initial.sql", include_str!("migrations/001_initial.sql")),
        (
            "002_auth_extensions.sql",
            include_str!("migrations/002_auth_extensions.sql"),
        ),
        ("003_features.sql", include_str!("migrations/003_features.sql")),
        (
            "004_talos_control.sql",
            include_str!("migrations/004_talos_control.sql"),
        ),
        (
            "005_control_plane.sql",
            include_str!("migrations/005_control_plane.sql"),
        ),
        (
            "006_backup_schedule.sql",
            include_str!("migrations/006_backup_schedule.sql"),
        ),
        (
            "007_cluster_access.sql",
            include_str!("migrations/007_cluster_access.sql"),
        ),
        (
            "008_deferred_mvps.sql",
            include_str!("migrations/008_deferred_mvps.sql"),
        ),
        (
            "009_product_gaps.sql",
            include_str!("migrations/009_product_gaps.sql"),
        ),
        (
            "010_ha_locks_fix.sql",
            include_str!("migrations/010_ha_locks_fix.sql"),
        ),
        (
            "011_baremetal_provisioning.sql",
            include_str!("migrations/011_baremetal_provisioning.sql"),
        ),
        (
            "012_metal_provisioning.sql",
            include_str!("migrations/012_metal_provisioning.sql"),
        ),
        (
            "013_desired_machine_config.sql",
            include_str!("migrations/013_desired_machine_config.sql"),
        ),
        (
            "014_cluster_network_config.sql",
            include_str!("migrations/014_cluster_network_config.sql"),
        ),
        (
            "015_drop_machine_classes.sql",
            include_str!("migrations/015_drop_machine_classes.sql"),
        ),
        (
            "016_factory_modules.sql",
            include_str!("migrations/016_factory_modules.sql"),
        ),
        (
            "017_upgrade_jobs_v2.sql",
            include_str!("migrations/017_upgrade_jobs_v2.sql"),
        ),
        (
            "018_module_overrides.sql",
            include_str!("migrations/018_module_overrides.sql"),
        ),
        (
            "019_cluster_siderolink_token.sql",
            include_str!("migrations/019_cluster_siderolink_token.sql"),
        ),
        (
            "020_machine_muid.sql",
            include_str!("migrations/020_machine_muid.sql"),
        ),
    ];

    for (name, sql) in migrations {
        let existing = pool
            .fetch_scalar_i64(
                "SELECT COUNT(*) FROM _tcs_migrations WHERE name = ?",
                &[SqlVal::text(*name)],
            )
            .await?;
        if existing > 0 {
            continue;
        }

        let body = if pool.is_postgres() {
            pool::sqlite_ddl_to_postgres(sql)
        } else {
            sql.to_string()
        };

        // Apply statement-by-statement
        if let Err(e) = pool.execute_script(&body).await {
            // Postgres may fail on SQLite-specific ALTER patterns; fall back to consolidated schema once.
            if pool.is_postgres() {
                tracing::warn!(
                    file = %name,
                    error = %e,
                    "Migration statement failed on Postgres; applying consolidated schema"
                );
                postgres::run_schema_on_pool(pool).await?;
            } else {
                return Err(e);
            }
        }

        pool.execute(
            "INSERT INTO _tcs_migrations (name) VALUES (?)",
            &[SqlVal::text(*name)],
        )
        .await?;
        tracing::info!(file = %name, "Migration applied");
    }

    // One-time repair: audit_logs.id was historically written as a raw 16-byte
    // BLOB on SQLite (SqlVal::Uuid) but the column is TEXT and is read back as
    // String. Convert any legacy BLOB ids to hyphenated text. No-op on
    // Postgres (uuid params auto-cast to text) and idempotent once fixed.
    if !pool.is_postgres() {
        let _ = pool
            .execute(
                "UPDATE audit_logs SET id = substr(hex(id),1,8) || '-' || substr(hex(id),9,4) || '-' || substr(hex(id),13,4) || '-' || substr(hex(id),17,4) || '-' || substr(hex(id),21,12) WHERE typeof(id) = 'blob'",
                &[],
            )
            .await;
    }

    Ok(())
}
