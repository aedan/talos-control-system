use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAccess {
    pub user_id: Uuid,
    pub cluster_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_for_user(
    pool: &SqlitePool,
    user_id: Uuid,
) -> Result<Vec<ClusterAccess>, AppError> {
    let rows = sqlx::query_as::<_, ClusterAccess>(
        "SELECT user_id, cluster_id, role, created_at FROM cluster_access WHERE user_id = ? ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_for_cluster(
    pool: &SqlitePool,
    cluster_id: Uuid,
) -> Result<Vec<ClusterAccess>, AppError> {
    let rows = sqlx::query_as::<_, ClusterAccess>(
        "SELECT user_id, cluster_id, role, created_at FROM cluster_access WHERE cluster_id = ? ORDER BY created_at",
    )
    .bind(cluster_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count_for_user(pool: &SqlitePool, user_id: Uuid) -> Result<i64, AppError> {
    let n: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM cluster_access WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(n.0)
}

pub async fn get(
    pool: &SqlitePool,
    user_id: Uuid,
    cluster_id: Uuid,
) -> Result<Option<ClusterAccess>, AppError> {
    let row = sqlx::query_as::<_, ClusterAccess>(
        "SELECT user_id, cluster_id, role, created_at FROM cluster_access WHERE user_id = ? AND cluster_id = ?",
    )
    .bind(user_id)
    .bind(cluster_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn upsert(
    pool: &SqlitePool,
    user_id: Uuid,
    cluster_id: Uuid,
    role: &str,
) -> Result<ClusterAccess, AppError> {
    let role = normalize_role(role)?;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO cluster_access (user_id, cluster_id, role, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(user_id, cluster_id) DO UPDATE SET role = excluded.role",
    )
    .bind(user_id)
    .bind(cluster_id)
    .bind(&role)
    .bind(now)
    .execute(pool)
    .await?;

    get(pool, user_id, cluster_id)
        .await?
        .ok_or_else(|| AppError::Internal("cluster_access upsert missing row".into()))
}

pub async fn delete(
    pool: &SqlitePool,
    user_id: Uuid,
    cluster_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM cluster_access WHERE user_id = ? AND cluster_id = ?")
        .bind(user_id)
        .bind(cluster_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_for_cluster(pool: &SqlitePool, cluster_id: Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM cluster_access WHERE cluster_id = ?")
        .bind(cluster_id)
        .execute(pool)
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

/// Effective role for a user on a cluster.
/// - Global admin → admin on every cluster
/// - No membership rows → global role (legacy open access)
/// - Has memberships → only listed clusters; role is the membership role
pub async fn effective_cluster_role(
    pool: &SqlitePool,
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

    Ok(get(pool, user_id, cluster_id)
        .await?
        .map(|a| a.role))
}
