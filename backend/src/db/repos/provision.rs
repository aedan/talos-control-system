use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionArtifact {
    pub id: Uuid,
    pub cluster_id: Option<Uuid>,
    pub name: String,
    pub talos_version: String,
    pub kubernetes_version: String,
    pub secrets_enc: Option<String>,
    pub controlplane_config: Option<String>,
    pub worker_config: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn create(pool: &DbPool, a: &ProvisionArtifact) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO provision_artifacts (id, cluster_id, name, talos_version, kubernetes_version, secrets_enc, controlplane_config, worker_config, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            SqlVal::Uuid(a.id),
            SqlVal::OptUuid(a.cluster_id),
            SqlVal::text(&a.name),
            SqlVal::text(&a.talos_version),
            SqlVal::text(&a.kubernetes_version),
            SqlVal::OptText(a.secrets_enc.clone()),
            SqlVal::OptText(a.controlplane_config.clone()),
            SqlVal::OptText(a.worker_config.clone()),
            SqlVal::DateTime(a.created_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn get(pool: &DbPool, id: Uuid) -> Result<Option<ProvisionArtifact>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM provision_artifacts WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn list(pool: &DbPool) -> Result<Vec<ProvisionArtifact>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM provision_artifacts ORDER BY created_at DESC LIMIT 50",
        &[],
    )
    .await
}

pub async fn delete(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    pool.execute(
        "DELETE FROM provision_artifacts WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await?;
    Ok(())
}
