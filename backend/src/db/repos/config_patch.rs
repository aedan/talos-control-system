use sqlx::SqlitePool;
use crate::db::models::config_patch::ConfigPatch;
use crate::AppError;

pub async fn create(pool: &SqlitePool, patch: &ConfigPatch) -> Result<ConfigPatch, AppError> {
    let existing = get(pool, patch.id).await?;
    if existing.is_some() {
        return Err(AppError::InvalidInput(format!("ConfigPatch {} already exists", patch.id)));
    }

    let result = sqlx::query(
        "INSERT INTO config_patches (id, cluster_id, machine_id, path, value, priority, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(patch.id)
    .bind(patch.cluster_id)
    .bind(patch.machine_id)
    .bind(&patch.path)
    .bind(&patch.value)
    .bind(patch.priority)
    .bind(patch.created_at)
    .bind(patch.updated_at)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Internal("Failed to create config patch".to_string()));
    }

    Ok(patch.clone())
}

pub async fn get(pool: &SqlitePool, id: uuid::Uuid) -> Result<Option<ConfigPatch>, AppError> {
    let patch = sqlx::query_as(
        "SELECT * FROM config_patches WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(patch)
}

pub async fn list_by_cluster(pool: &SqlitePool, cluster_id: uuid::Uuid) -> Result<Vec<ConfigPatch>, AppError> {
    let patches = sqlx::query_as::<_, ConfigPatch>(
        "SELECT * FROM config_patches WHERE cluster_id = ? ORDER BY priority DESC, created_at DESC"
    )
    .bind(cluster_id)
    .fetch_all(pool)
    .await?;

    Ok(patches)
}

pub async fn delete(pool: &SqlitePool, id: uuid::Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        "DELETE FROM config_patches WHERE id = ?"
    )
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("ConfigPatch {} not found", id)));
    }

    Ok(())
}
