use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeJob {
    pub id: Uuid,
    pub scope: String,
    pub image: String,
    pub status: String,
    pub max_unavailable: i32,
    pub control_plane_last: bool,
    pub cancel_requested: bool,
    pub created_by: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeJobTarget {
    pub id: Uuid,
    pub job_id: Uuid,
    pub cluster_id: Uuid,
    pub machine_id: Uuid,
    pub address: Option<String>,
    pub machine_type: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub sort_order: i32,
    pub updated_at: DateTime<Utc>,
}

pub async fn create_job(pool: &SqlitePool, job: &UpgradeJob) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO upgrade_jobs (id, scope, image, status, max_unavailable, control_plane_last, cancel_requested, created_by, error, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job.id)
    .bind(&job.scope)
    .bind(&job.image)
    .bind(&job.status)
    .bind(job.max_unavailable)
    .bind(job.control_plane_last)
    .bind(job.cancel_requested)
    .bind(&job.created_by)
    .bind(&job.error)
    .bind(job.created_at)
    .bind(job.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_target(pool: &SqlitePool, t: &UpgradeJobTarget) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO upgrade_job_targets (id, job_id, cluster_id, machine_id, address, machine_type, status, error, sort_order, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(t.id)
    .bind(t.job_id)
    .bind(t.cluster_id)
    .bind(t.machine_id)
    .bind(&t.address)
    .bind(&t.machine_type)
    .bind(&t.status)
    .bind(&t.error)
    .bind(t.sort_order)
    .bind(t.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_job(pool: &SqlitePool, id: Uuid) -> Result<Option<UpgradeJob>, AppError> {
    Ok(sqlx::query_as::<_, UpgradeJob>("SELECT * FROM upgrade_jobs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn list_jobs(pool: &SqlitePool, limit: i64) -> Result<Vec<UpgradeJob>, AppError> {
    Ok(sqlx::query_as::<_, UpgradeJob>(
        "SELECT * FROM upgrade_jobs ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn list_targets(pool: &SqlitePool, job_id: Uuid) -> Result<Vec<UpgradeJobTarget>, AppError> {
    Ok(sqlx::query_as::<_, UpgradeJobTarget>(
        "SELECT * FROM upgrade_job_targets WHERE job_id = ? ORDER BY sort_order ASC",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?)
}

pub async fn update_job_status(
    pool: &SqlitePool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query("UPDATE upgrade_jobs SET status = ?, error = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(error)
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn request_cancel(pool: &SqlitePool, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE upgrade_jobs SET cancel_requested = 1, updated_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_target_status(
    pool: &SqlitePool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE upgrade_job_targets SET status = ?, error = ?, updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(error)
    .bind(Utc::now())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_pending_jobs(pool: &SqlitePool) -> Result<Vec<UpgradeJob>, AppError> {
    Ok(sqlx::query_as::<_, UpgradeJob>(
        "SELECT * FROM upgrade_jobs WHERE status IN ('pending', 'running') ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?)
}
