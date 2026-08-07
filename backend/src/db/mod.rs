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
    let migration_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&migration_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".sql"))
        .collect();

    entries.sort_by_key(|e| e.file_name());

    let mut tx = pool.begin().await?;

    for entry in entries {
        let sql = std::fs::read_to_string(entry.path())?;
        sqlx::query(&sql).execute(&mut *tx).await?;
        tracing::info!(file = %entry.file_name().to_string_lossy(), "Migration applied");
    }

    tx.commit().await?;
    Ok(())
}
