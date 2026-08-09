pub mod models;
pub mod repos;

use sqlx::SqlitePool;
use crate::config::{DatabaseBackend, DatabaseConfig};
use crate::AppError;

pub type Pool = SqlitePool;

pub async fn init_pool(config: &DatabaseConfig) -> Result<SqlitePool, AppError> {
    let pool = SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.sqlite_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::default())
            .foreign_keys(true)
    )
    .await?;

    tracing::info!(backend = %config.backend, "Database pool initialized");
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
