use chrono::{Duration, Utc};

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn remember(pool: &DbPool, state: &str, ttl_secs: i64) -> Result<(), AppError> {
    let now = Utc::now();
    let exp = now + Duration::seconds(ttl_secs.max(60));
    // Best-effort cleanup
    let _ = pool
        .execute(
            "DELETE FROM oidc_states WHERE expires_at < ?",
            &[SqlVal::DateTime(now)],
        )
        .await;
    pool.execute(
        "INSERT INTO oidc_states (state, created_at, expires_at) VALUES (?, ?, ?)",
        &[
            SqlVal::text(state),
            SqlVal::DateTime(now),
            SqlVal::DateTime(exp),
        ],
    )
    .await?;
    Ok(())
}

/// Consume state (single-use). Returns true if valid.
pub async fn take(pool: &DbPool, state: &str) -> Result<bool, AppError> {
    let now = Utc::now();
    #[derive(sqlx::FromRow)]
    struct Row {
        expires_at: String,
    }
    let row: Option<Row> = pool
        .fetch_optional_as(
            "SELECT expires_at FROM oidc_states WHERE state = ?",
            &[SqlVal::text(state)],
        )
        .await?;
    let Some(r) = row else {
        return Ok(false);
    };
    let _ = pool
        .execute(
            "DELETE FROM oidc_states WHERE state = ?",
            &[SqlVal::text(state)],
        )
        .await;
    let exp = chrono::DateTime::parse_from_rfc3339(&r.expires_at)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::seconds(1));
    Ok(exp > now)
}
