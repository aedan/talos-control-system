//! Multi-replica coordination via DB-backed locks.

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

/// Unique id for this process (set once at startup).
pub fn instance_id() -> String {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("TCS_INSTANCE_ID").unwrap_or_else(|_| format!("tcs-{}", Uuid::new_v4()))
    })
    .clone()
}

/// Try to acquire or renew a named lock for `ttl_secs`. Returns true if this instance holds it.
pub async fn try_acquire(pool: &DbPool, lock_name: &str, ttl_secs: i64) -> Result<bool, AppError> {
    let owner = instance_id();
    let now = Utc::now();
    let expires = now + Duration::seconds(ttl_secs.max(5));

    // Cleanup expired
    let _ = pool
        .execute(
            "DELETE FROM ha_locks WHERE expires_at < ?",
            &[SqlVal::DateTime(now)],
        )
        .await;

    // Try insert
    let inserted = pool
        .execute(
            "INSERT INTO ha_locks (lock_name, owner_id, expires_at, updated_at) VALUES (?, ?, ?, ?)",
            &[
                SqlVal::text(lock_name),
                SqlVal::text(&owner),
                SqlVal::DateTime(expires),
                SqlVal::DateTime(now),
            ],
        )
        .await;

    if inserted.is_ok() {
        return Ok(true);
    }

    // Renew if we already own it
    let n = pool
        .execute(
            "UPDATE ha_locks SET expires_at = ?, updated_at = ? WHERE lock_name = ? AND owner_id = ?",
            &[
                SqlVal::DateTime(expires),
                SqlVal::DateTime(now),
                SqlVal::text(lock_name),
                SqlVal::text(&owner),
            ],
        )
        .await?;
    Ok(n > 0)
}

pub async fn is_leader(pool: &DbPool, lock_name: &str) -> Result<bool, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        owner_id: String,
        expires_at: String,
    }
    let row: Option<Row> = pool
        .fetch_optional_as(
            "SELECT owner_id, expires_at FROM ha_locks WHERE lock_name = ?",
            &[SqlVal::text(lock_name)],
        )
        .await?;
    let Some(r) = row else {
        return Ok(false);
    };
    let exp = chrono::DateTime::parse_from_rfc3339(&r.expires_at)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(Utc::now() - Duration::seconds(1));
    Ok(r.owner_id == instance_id() && exp > Utc::now())
}
