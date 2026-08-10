use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

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

pub async fn create(pool: &SqlitePool, a: &ProvisionArtifact) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO provision_artifacts (id, cluster_id, name, talos_version, kubernetes_version, secrets_enc, controlplane_config, worker_config, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(a.id)
    .bind(a.cluster_id)
    .bind(&a.name)
    .bind(&a.talos_version)
    .bind(&a.kubernetes_version)
    .bind(&a.secrets_enc)
    .bind(&a.controlplane_config)
    .bind(&a.worker_config)
    .bind(a.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &SqlitePool, id: Uuid) -> Result<Option<ProvisionArtifact>, AppError> {
    Ok(
        sqlx::query_as::<_, ProvisionArtifact>("SELECT * FROM provision_artifacts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<ProvisionArtifact>, AppError> {
    Ok(sqlx::query_as::<_, ProvisionArtifact>(
        "SELECT * FROM provision_artifacts ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(pool)
    .await?)
}
