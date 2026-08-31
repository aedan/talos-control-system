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
    #[serde(default)]
    pub target_talos_version: Option<String>,
    #[serde(default)]
    pub target_k8s_version: Option<String>,
    #[serde(default = "default_phase")]
    pub phase: String,
    #[serde(default)]
    pub steps: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_phase() -> String {
    "talos".to_string()
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
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub k8s_version: Option<String>,
    #[serde(default = "default_phase")]
    pub phase: String,
    #[serde(default)]
    pub completed_steps: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub async fn create_job(pool: &DbPool, job: &UpgradeJob) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO upgrade_jobs (id, scope, image, status, max_unavailable, control_plane_last, cancel_requested, created_by, error, target_talos_version, target_k8s_version, phase, steps, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            SqlVal::OptText(job.target_talos_version.clone()),
            SqlVal::OptText(job.target_k8s_version.clone()),
            SqlVal::text(&job.phase),
            SqlVal::OptText(job.steps.clone()),
            SqlVal::DateTime(job.created_at),
            SqlVal::DateTime(job.updated_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn insert_target(pool: &DbPool, t: &UpgradeJobTarget) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO upgrade_job_targets (id, job_id, cluster_id, machine_id, address, machine_type, status, error, sort_order, image, k8s_version, phase, completed_steps, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            SqlVal::OptText(t.image.clone()),
            SqlVal::OptText(t.k8s_version.clone()),
            SqlVal::text(&t.phase),
            SqlVal::OptText(t.completed_steps.clone()),
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

pub async fn list_jobs_for_cluster(
    pool: &DbPool,
    cluster_id: Uuid,
    limit: i64,
) -> Result<Vec<UpgradeJob>, AppError> {
    pool.fetch_all_as(
        "SELECT DISTINCT j.* FROM upgrade_jobs j
         JOIN upgrade_job_targets t ON t.job_id = j.id
         WHERE t.cluster_id = ?
         ORDER BY j.created_at DESC LIMIT ?",
        &[SqlVal::Uuid(cluster_id), SqlVal::I64(limit)],
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

pub async fn update_job_phase(pool: &DbPool, id: Uuid, phase: &str) -> Result<(), AppError> {
    pool.execute(
        "UPDATE upgrade_jobs SET phase = ?, updated_at = ? WHERE id = ?",
        &[
            SqlVal::text(phase),
            SqlVal::DateTime(Utc::now()),
            SqlVal::Uuid(id),
        ],
    )
    .await?;
    Ok(())
}

pub async fn update_target_fields(
    pool: &DbPool,
    id: Uuid,
    status: Option<&str>,
    error: Option<&str>,
    k8s_version: Option<&str>,
    phase: Option<&str>,
    completed_steps: Option<&str>,
) -> Result<(), AppError> {
    let now = SqlVal::DateTime(Utc::now());
    if let Some(status) = status {
        pool.execute(
            "UPDATE upgrade_job_targets SET status = ?, updated_at = ? WHERE id = ?",
            &[SqlVal::text(status), now.clone(), SqlVal::Uuid(id)],
        )
        .await?;
    }
    if let Some(error) = error {
        pool.execute(
            "UPDATE upgrade_job_targets SET error = ?, updated_at = ? WHERE id = ?",
            &[SqlVal::text(error), now.clone(), SqlVal::Uuid(id)],
        )
        .await?;
    }
    if let Some(k8s_version) = k8s_version {
        pool.execute(
            "UPDATE upgrade_job_targets SET k8s_version = ?, updated_at = ? WHERE id = ?",
            &[SqlVal::text(k8s_version), now.clone(), SqlVal::Uuid(id)],
        )
        .await?;
    }
    if let Some(phase) = phase {
        pool.execute(
            "UPDATE upgrade_job_targets SET phase = ?, updated_at = ? WHERE id = ?",
            &[SqlVal::text(phase), now.clone(), SqlVal::Uuid(id)],
        )
        .await?;
    }
    if let Some(completed_steps) = completed_steps {
        pool.execute(
            "UPDATE upgrade_job_targets SET completed_steps = ?, updated_at = ? WHERE id = ?",
            &[SqlVal::text(completed_steps), now, SqlVal::Uuid(id)],
        )
        .await?;
    }
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
