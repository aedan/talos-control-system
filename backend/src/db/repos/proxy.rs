//! Remote OOB proxy join tokens (mirror of `siderolink` token model).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProxyJoinToken {
    pub token: String,
    pub label: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub async fn create_token(
    pool: &DbPool,
    token: &str,
    label: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO proxy_join_tokens (token, label, expires_at, created_at) VALUES (?, ?, ?, ?)",
        &[
            SqlVal::text(token),
            SqlVal::OptText(label.map(|s| s.to_string())),
            SqlVal::OptDateTime(expires_at),
            SqlVal::DateTime(Utc::now()),
        ],
    )
    .await?;
    Ok(())
}

pub async fn list_tokens(pool: &DbPool) -> Result<Vec<ProxyJoinToken>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM proxy_join_tokens ORDER BY created_at DESC",
        &[],
    )
    .await
}

pub async fn validate_token(pool: &DbPool, token: &str) -> Result<bool, AppError> {
    let row: Option<ProxyJoinToken> = pool
        .fetch_optional_as(
            "SELECT * FROM proxy_join_tokens WHERE token = ?",
            &[SqlVal::text(token)],
        )
        .await?;
    Ok(match row {
        None => false,
        Some(t) => t.expires_at.map(|e| e > Utc::now()).unwrap_or(true),
    })
}

pub async fn delete_token(pool: &DbPool, token: &str) -> Result<(), AppError> {
    pool.execute(
        "DELETE FROM proxy_join_tokens WHERE token = ?",
        &[SqlVal::text(token)],
    )
    .await?;
    Ok(())
}
