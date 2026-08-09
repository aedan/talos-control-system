use sqlx::SqlitePool;
use crate::db::models::cluster_backup::ClusterBackup;
use crate::AppError;

pub async fn create(pool: &SqlitePool, backup: &ClusterBackup) -> Result<ClusterBackup, AppError> {
    let existing = get(pool, backup.id).await?;
    if existing.is_some() {
        return Err(AppError::InvalidInput(format!("Backup {} already exists", backup.id)));
    }

    let result = sqlx::query(
        "INSERT INTO cluster_backups (id, cluster_id, name, status, file_path, size_bytes, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(backup.id)
    .bind(backup.cluster_id)
    .bind(&backup.name)
    .bind(&backup.status)
    .bind(&backup.file_path)
    .bind(backup.size_bytes)
    .bind(backup.created_at)
    .bind(backup.updated_at)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Internal("Failed to create backup".to_string()));
    }

    Ok(backup.clone())
}

pub async fn get(pool: &SqlitePool, id: uuid::Uuid) -> Result<Option<ClusterBackup>, AppError> {
    let backup = sqlx::query_as(
        "SELECT * FROM cluster_backups WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(backup)
}

pub async fn list_by_cluster(pool: &SqlitePool, cluster_id: uuid::Uuid) -> Result<Vec<ClusterBackup>, AppError> {
    let backups = sqlx::query_as::<_, ClusterBackup>(
        "SELECT * FROM cluster_backups WHERE cluster_id = ? ORDER BY created_at DESC"
    )
    .bind(cluster_id)
    .fetch_all(pool)
    .await?;

    Ok(backups)
}

pub async fn delete(pool: &SqlitePool, id: uuid::Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        "DELETE FROM cluster_backups WHERE id = ?"
    )
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Backup {} not found", id)));
    }

    Ok(())
}

pub async fn update(pool: &SqlitePool, backup: &ClusterBackup) -> Result<ClusterBackup, AppError> {
    let result = sqlx::query(
        "UPDATE cluster_backups SET name = ?, status = ?, file_path = ?, size_bytes = ?, updated_at = ?
         WHERE id = ?"
    )
    .bind(&backup.name)
    .bind(&backup.status)
    .bind(&backup.file_path)
    .bind(backup.size_bytes)
    .bind(backup.updated_at)
    .bind(backup.id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Backup {} not found", backup.id)));
    }

    Ok(backup.clone())
}
