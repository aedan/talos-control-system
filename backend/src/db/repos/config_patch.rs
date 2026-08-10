use crate::db::models::config_patch::ConfigPatch;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn create(pool: &DbPool, patch: &ConfigPatch) -> Result<ConfigPatch, AppError> {
    if get(pool, patch.id).await?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "ConfigPatch {} already exists",
            patch.id
        )));
    }
    let n = pool
        .execute(
            "INSERT INTO config_patches (id, cluster_id, machine_id, path, value, priority, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(patch.id),
                SqlVal::Uuid(patch.cluster_id),
                SqlVal::OptUuid(patch.machine_id),
                SqlVal::text(&patch.path),
                SqlVal::text(&patch.value),
                SqlVal::I32(patch.priority),
                SqlVal::DateTime(patch.created_at),
                SqlVal::DateTime(patch.updated_at),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Internal("Failed to create config patch".into()));
    }
    Ok(patch.clone())
}

pub async fn get(pool: &DbPool, id: uuid::Uuid) -> Result<Option<ConfigPatch>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM config_patches WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn list_by_cluster(
    pool: &DbPool,
    cluster_id: uuid::Uuid,
) -> Result<Vec<ConfigPatch>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM config_patches WHERE cluster_id = ? ORDER BY priority DESC, created_at DESC",
        &[SqlVal::Uuid(cluster_id)],
    )
    .await
}

pub async fn delete(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let n = pool
        .execute(
            "DELETE FROM config_patches WHERE id = ?",
            &[SqlVal::Uuid(id)],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("ConfigPatch {} not found", id)));
    }
    Ok(())
}
