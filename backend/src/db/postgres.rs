//! PostgreSQL helpers (consolidated greenfield schema).

use crate::db::pool::DbPool;
use crate::AppError;

/// Apply a consolidated schema suitable for a greenfield Postgres instance.
pub async fn run_schema_on_pool(pool: &DbPool) -> Result<(), AppError> {
    if !pool.is_postgres() {
        return Ok(());
    }
    let sql = include_str!("postgres_schema.sql");
    pool.execute_script(sql).await?;
    let applied = pool
        .fetch_scalar_i64(
            "SELECT COUNT(*) FROM _tcs_migrations WHERE name = ?",
            &[crate::db::SqlVal::text("postgres_schema_v2")],
        )
        .await
        .unwrap_or(0);
    if applied == 0 {
        let _ = pool
            .execute(
                "INSERT INTO _tcs_migrations (name) VALUES (?)",
                &[crate::db::SqlVal::text("postgres_schema_v2")],
            )
            .await;
        tracing::info!("PostgreSQL consolidated schema applied (postgres_schema_v2)");
    }
    Ok(())
}
