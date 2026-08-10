use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
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

pub async fn create_job(pool: &DbPool, job: &UpgradeJob) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO upgrade_jobs (id, scope, image, status, max_unavailable, control_plane_last, cancel_requested, created_by, error, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            SqlVal::Uuid(job.id),
            SqlVal::text(&job.scope),
            SqlVal::text(&job.image),
            SqlVal::text(&job.status),
            SqlVal::I32(job.max_unavailable),
            SqlVal::Bool(job.control_plane_last),
            SqlVal::Bool(job.cancel_requested),
            SqlVal::OptText(job.created_by.clone()),
            SqlVal::OptText(job.error.clone()),
            SqlVal::DateTime(job.created_at),
            SqlVal::DateTime(job.updated_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn insert_target(pool: &DbPool, t: &UpgradeJobTarget) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO upgrade_job_targets (id, job_id, cluster_id, machine_id, address, machine_type, status, error, sort_order, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            SqlVal::Uuid(t.id),
            SqlVal::Uuid(t.job_id),
            SqlVal::Uuid(t.cluster_id),
            SqlVal::Uuid(t.machine_id),
            SqlVal::OptText(t.address.clone()),
            SqlVal::OptText(t.machine_type.clone()),
            SqlVal::text(&t.status),
            SqlVal::OptText(t.error.clone()),
            SqlVal::I32(t.sort_order),
            SqlVal::DateTime(t.updated_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn get_job(pool: &DbPool, id: Uuid) -> Result<Option<UpgradeJob>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM upgrade_jobs WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn list_jobs(pool: &DbPool, limit: i64) -> Result<Vec<UpgradeJob>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM upgrade_jobs ORDER BY created_at DESC LIMIT ?",
        &[SqlVal::I64(limit)],
    )
    .await
}

pub async fn list_targets(pool: &DbPool, job_id: Uuid) -> Result<Vec<UpgradeJobTarget>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM upgrade_job_targets WHERE job_id = ? ORDER BY sort_order ASC",
        &[SqlVal::Uuid(job_id)],
    )
    .await
}

pub async fn update_job_status(
    pool: &DbPool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    pool.execute(
        "UPDATE upgrade_jobs SET status = ?, error = ?, updated_at = ? WHERE id = ?",
        &[
            SqlVal::text(status),
            SqlVal::OptText(error.map(|s| s.to_string())),
            SqlVal::DateTime(Utc::now()),
            SqlVal::Uuid(id),
        ],
    )
    .await?;
    Ok(())
}

pub async fn request_cancel(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    pool.execute(
        "UPDATE upgrade_jobs SET cancel_requested = 1, updated_at = ? WHERE id = ?",
        &[SqlVal::DateTime(Utc::now()), SqlVal::Uuid(id)],
    )
    .await?;
    Ok(())
}

pub async fn update_target_status(
    pool: &DbPool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    pool.execute(
        "UPDATE upgrade_job_targets SET status = ?, error = ?, updated_at = ? WHERE id = ?",
        &[
            SqlVal::text(status),
            SqlVal::OptText(error.map(|s| s.to_string())),
            SqlVal::DateTime(Utc::now()),
            SqlVal::Uuid(id),
        ],
    )
    .await?;
    Ok(())
}

pub async fn list_pending_jobs(pool: &DbPool) -> Result<Vec<UpgradeJob>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM upgrade_jobs WHERE status IN ('pending', 'running') ORDER BY created_at ASC",
        &[],
    )
    .await
}
