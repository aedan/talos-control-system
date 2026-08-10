use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAccess {
    pub user_id: Uuid,
    pub cluster_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_for_user(pool: &DbPool, user_id: Uuid) -> Result<Vec<ClusterAccess>, AppError> {
    pool.fetch_all_as(
        "SELECT user_id, cluster_id, role, created_at FROM cluster_access WHERE user_id = ? ORDER BY created_at",
        &[SqlVal::Uuid(user_id)],
    )
    .await
}

pub async fn list_for_cluster(
    pool: &DbPool,
    cluster_id: Uuid,
) -> Result<Vec<ClusterAccess>, AppError> {
    pool.fetch_all_as(
        "SELECT user_id, cluster_id, role, created_at FROM cluster_access WHERE cluster_id = ? ORDER BY created_at",
        &[SqlVal::Uuid(cluster_id)],
    )
    .await
}

pub async fn count_for_user(pool: &DbPool, user_id: Uuid) -> Result<i64, AppError> {
    pool.fetch_scalar_i64(
        "SELECT COUNT(*) FROM cluster_access WHERE user_id = ?",
        &[SqlVal::Uuid(user_id)],
    )
    .await
}

pub async fn get(
    pool: &DbPool,
    user_id: Uuid,
    cluster_id: Uuid,
) -> Result<Option<ClusterAccess>, AppError> {
    pool.fetch_optional_as(
        "SELECT user_id, cluster_id, role, created_at FROM cluster_access WHERE user_id = ? AND cluster_id = ?",
        &[SqlVal::Uuid(user_id), SqlVal::Uuid(cluster_id)],
    )
    .await
}

pub async fn upsert(
    pool: &DbPool,
    user_id: Uuid,
    cluster_id: Uuid,
    role: &str,
) -> Result<ClusterAccess, AppError> {
    let role = normalize_role(role)?;
    let now = Utc::now();
    // Postgres uses ON CONFLICT (cols); SQLite ON CONFLICT(cols) — both accept this form.
    pool.execute(
        "INSERT INTO cluster_access (user_id, cluster_id, role, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(user_id, cluster_id) DO UPDATE SET role = excluded.role",
        &[
            SqlVal::Uuid(user_id),
            SqlVal::Uuid(cluster_id),
            SqlVal::text(role),
            SqlVal::DateTime(now),
        ],
    )
    .await?;
    get(pool, user_id, cluster_id)
        .await?
        .ok_or_else(|| AppError::Internal("cluster_access upsert missing row".into()))
}

pub async fn delete(pool: &DbPool, user_id: Uuid, cluster_id: Uuid) -> Result<(), AppError> {
    pool.execute(
        "DELETE FROM cluster_access WHERE user_id = ? AND cluster_id = ?",
        &[SqlVal::Uuid(user_id), SqlVal::Uuid(cluster_id)],
    )
    .await?;
    Ok(())
}

pub async fn delete_for_cluster(pool: &DbPool, cluster_id: Uuid) -> Result<(), AppError> {
    pool.execute(
        "DELETE FROM cluster_access WHERE cluster_id = ?",
        &[SqlVal::Uuid(cluster_id)],
    )
    .await?;
    Ok(())
}

fn normalize_role(role: &str) -> Result<String, AppError> {
    match role.trim().to_ascii_lowercase().as_str() {
        "admin" | "operator" | "reader" => Ok(role.trim().to_ascii_lowercase()),
        "viewer" => Ok("reader".to_string()),
        other => Err(AppError::InvalidInput(format!(
            "Invalid cluster role '{}': use admin, operator, or reader",
            other
        ))),
    }
}

pub async fn effective_cluster_role(
    pool: &DbPool,
    user_id: Uuid,
    global_role: &str,
    cluster_id: Uuid,
) -> Result<Option<String>, AppError> {
    if global_role == "admin" {
        return Ok(Some("admin".to_string()));
    }
    let n = count_for_user(pool, user_id).await?;
    if n == 0 {
        return Ok(Some(global_role.to_string()));
    }
    Ok(get(pool, user_id, cluster_id).await?.map(|a| a.role))
}
