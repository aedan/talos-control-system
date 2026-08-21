//! One-shot SQLite → Postgres data copy for cutover.
//!
//! ```text
//! TCS_SQLITE_PATH=/var/lib/tcs/data.db \
//! TCS_POSTGRES_URL=postgresql://... \
//!   tcs migrate-sqlite-to-postgres
//! ```

use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Column, PgPool, Row, SqlitePool};

use crate::AppError;

const TABLES: &[&str] = &[
    "users",
    "clusters",
    "machines",
    "tenant_branding",
    "config_patches",
    "cluster_backups",
    "cluster_access",
    "audit_logs",
    "upgrade_jobs",
    "upgrade_job_targets",
    "siderolink_peers",
    "siderolink_join_tokens",
    "provision_artifacts",
    "provision_jobs",
    "ha_locks",
    "oidc_states",
];

pub async fn run(sqlite_path: &str, postgres_url: &str) -> Result<(), AppError> {
    if sqlite_path.is_empty() || postgres_url.is_empty() {
        return Err(AppError::Config(
            "migrate-sqlite-to-postgres requires TCS_SQLITE_PATH and TCS_POSTGRES_URL".into(),
        ));
    }

    let sqlite = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(sqlite_path)
            .create_if_missing(false),
    )
    .await
    .map_err(|e| AppError::Config(format!("open sqlite {}: {}", sqlite_path, e)))?;

    let pg = PgPoolOptions::new()
        .max_connections(5)
        .connect(postgres_url)
        .await
        .map_err(|e| AppError::Config(format!("connect postgres: {}", e)))?;

    let schema = include_str!("postgres_schema.sql");
    for stmt in schema.split(';') {
        let lines: Vec<&str> = stmt
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("--"))
            .collect();
        if lines.is_empty() {
            continue;
        }
        let s = lines.join("\n");
        sqlx::query(&s)
            .execute(&pg)
            .await
            .map_err(AppError::Database)?;
    }

    for table in TABLES {
        copy_table(&sqlite, &pg, table).await?;
    }

    tracing::info!("SQLite → Postgres migration finished");
    Ok(())
}

async fn copy_table(sqlite: &SqlitePool, pg: &PgPool, table: &str) -> Result<(), AppError> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?",
    )
    .bind(table)
    .fetch_one(sqlite)
    .await
    .unwrap_or(0);
    if exists == 0 {
        tracing::info!(table, "skip missing sqlite table");
        return Ok(());
    }

    let rows = sqlx::query(&format!("SELECT * FROM \"{}\"", table))
        .fetch_all(sqlite)
        .await?;
    if rows.is_empty() {
        tracing::info!(table, "empty");
        return Ok(());
    }

    let columns: Vec<String> = rows[0]
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let col_list = columns
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders: String = (1..=columns.len())
        .map(|i| format!("${}", i))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES ({}) ON CONFLICT DO NOTHING",
        table, col_list, placeholders
    );

    let mut copied = 0u64;
    for row in &rows {
        let mut q = sqlx::query(&insert_sql);
        for col in &columns {
            q = bind_any(q, row, col);
        }
        match q.execute(pg).await {
            Ok(r) => copied += r.rows_affected(),
            Err(e) => tracing::warn!(table, error = %e, "row insert failed"),
        }
    }
    tracing::info!(table, copied, total = rows.len(), "copied");
    Ok(())
}

fn bind_any<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    row: &'q sqlx::sqlite::SqliteRow,
    col: &str,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    if let Ok(v) = row.try_get::<Option<String>, _>(col) {
        return q.bind(v);
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(col) {
        return q.bind(v);
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(col) {
        return q.bind(v);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(col) {
        return q.bind(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(col) {
        return q.bind(v);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(col) {
        return q.bind(v);
    }
    q.bind(Option::<String>::None)
}
