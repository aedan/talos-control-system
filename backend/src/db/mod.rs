pub mod models;
pub mod postgres;
pub mod repos;

use sqlx::SqlitePool;
use crate::config::{DatabaseBackend, DatabaseConfig};
use crate::AppError;

pub type Pool = SqlitePool;

pub async fn init_pool(config: &DatabaseConfig) -> Result<SqlitePool, AppError> {
    if config.backend == DatabaseBackend::Postgres {
        // Validate URL + apply greenfield schema, then refuse app runtime until
        // dual-backend query dialect abstraction lands. Operators can use this
        // to prepare a Postgres instance ahead of cutover.
        let pg = postgres::connect(config).await?;
        postgres::run_schema(&pg).await?;
        pg.close().await;
        return Err(AppError::Config(
            "PostgreSQL schema bootstrap succeeded, but application runtime still uses SQLite. \
             Set database.backend = \"sqlite\" for this release. See docs/POSTGRES.md."
                .to_string(),
        ));
    }

    // Ensure parent directory exists for the SQLite file
    if let Some(parent) = std::path::Path::new(&config.sqlite_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::Config(format!(
                    "Cannot create database directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }
    }

    let pool = SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.sqlite_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::default())
            .foreign_keys(true),
    )
    .await?;

    tracing::info!(backend = %config.backend, path = %config.sqlite_path, "Database pool initialized");
    Ok(pool)
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), AppError> {
    // Create migration tracking table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _tcs_migrations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"
    )
    .execute(pool)
    .await?;

    // Use embedded migrations (via include_str!) to avoid filesystem dependency
    let migrations = [
        ("001_initial.sql", include_str!("migrations/001_initial.sql")),
        ("002_auth_extensions.sql", include_str!("migrations/002_auth_extensions.sql")),
        ("003_features.sql", include_str!("migrations/003_features.sql")),
        ("004_talos_control.sql", include_str!("migrations/004_talos_control.sql")),
        ("005_control_plane.sql", include_str!("migrations/005_control_plane.sql")),
        ("006_backup_schedule.sql", include_str!("migrations/006_backup_schedule.sql")),
        ("007_cluster_access.sql", include_str!("migrations/007_cluster_access.sql")),
        ("008_deferred_mvps.sql", include_str!("migrations/008_deferred_mvps.sql")),
    ];

    let mut tx = pool.begin().await?;

    for (name, sql) in migrations {
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM _tcs_migrations WHERE name = ?)"
        )
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;

        if !existing {
            sqlx::query(sql).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO _tcs_migrations (name) VALUES (?)")
                .bind(name)
                .execute(&mut *tx)
                .await?;
            tracing::info!(file = %name, "Migration applied");
        }
    }

    tx.commit().await?;
    Ok(())
}
