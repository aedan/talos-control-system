use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionJob {
    pub id: Uuid,
    pub cluster_id: Option<Uuid>,
    pub kind: String,
    pub status: String,
    pub desired_workers: i32,
    pub payload: Option<String>,
    pub error: Option<String>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn create(pool: &DbPool, job: &ProvisionJob) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO provision_jobs (id, cluster_id, kind, status, desired_workers, payload, error, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            SqlVal::Uuid(job.id),
            SqlVal::OptUuid(job.cluster_id),
            SqlVal::text(&job.kind),
            SqlVal::text(&job.status),
            SqlVal::I32(job.desired_workers),
            SqlVal::OptText(job.payload.clone()),
            SqlVal::OptText(job.error.clone()),
            SqlVal::OptText(job.created_by.clone()),
            SqlVal::DateTime(job.created_at),
            SqlVal::DateTime(job.updated_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn get(pool: &DbPool, id: Uuid) -> Result<Option<ProvisionJob>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM provision_jobs WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn list(pool: &DbPool) -> Result<Vec<ProvisionJob>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM provision_jobs ORDER BY created_at DESC LIMIT 100",
        &[],
    )
    .await
}

pub async fn list_active(pool: &DbPool) -> Result<Vec<ProvisionJob>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM provision_jobs WHERE status IN ('pending','running','waiting_pxe','waiting_installer','installing','bootstrapping') ORDER BY created_at ASC",
        &[],
    )
    .await
}

pub async fn update_status(
    pool: &DbPool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
    payload: Option<&str>,
) -> Result<(), AppError> {
    let now = Utc::now();
    if let Some(p) = payload {
        pool.execute(
            "UPDATE provision_jobs SET status = ?, error = ?, payload = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(status),
                SqlVal::OptText(error.map(|s| s.to_string())),
                SqlVal::text(p),
                SqlVal::DateTime(now),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    } else {
        pool.execute(
            "UPDATE provision_jobs SET status = ?, error = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(status),
                SqlVal::OptText(error.map(|s| s.to_string())),
                SqlVal::DateTime(now),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    }
    Ok(())
}
