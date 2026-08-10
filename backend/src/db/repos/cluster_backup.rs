use crate::db::models::cluster_backup::ClusterBackup;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn create(pool: &DbPool, backup: &ClusterBackup) -> Result<ClusterBackup, AppError> {
    if get(pool, backup.id).await?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "Backup {} already exists",
            backup.id
        )));
    }
    let n = pool
        .execute(
            "INSERT INTO cluster_backups (id, cluster_id, name, status, file_path, size_bytes, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(backup.id),
                SqlVal::Uuid(backup.cluster_id),
                SqlVal::text(&backup.name),
                SqlVal::text(&backup.status),
                SqlVal::OptText(backup.file_path.clone()),
                SqlVal::I64(backup.size_bytes),
                SqlVal::DateTime(backup.created_at),
                SqlVal::DateTime(backup.updated_at),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Internal("Failed to create backup".into()));
    }
    Ok(backup.clone())
}

pub async fn get(pool: &DbPool, id: uuid::Uuid) -> Result<Option<ClusterBackup>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM cluster_backups WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn list_by_cluster(
    pool: &DbPool,
    cluster_id: uuid::Uuid,
) -> Result<Vec<ClusterBackup>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM cluster_backups WHERE cluster_id = ? ORDER BY created_at DESC",
        &[SqlVal::Uuid(cluster_id)],
    )
    .await
}

pub async fn delete(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let n = pool
        .execute(
            "DELETE FROM cluster_backups WHERE id = ?",
            &[SqlVal::Uuid(id)],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Backup {} not found", id)));
    }
    Ok(())
}

pub async fn update(pool: &DbPool, backup: &ClusterBackup) -> Result<ClusterBackup, AppError> {
    let n = pool
        .execute(
            "UPDATE cluster_backups SET name = ?, status = ?, file_path = ?, size_bytes = ?, updated_at = ?
             WHERE id = ?",
            &[
                SqlVal::text(&backup.name),
                SqlVal::text(&backup.status),
                SqlVal::OptText(backup.file_path.clone()),
                SqlVal::I64(backup.size_bytes),
                SqlVal::DateTime(backup.updated_at),
                SqlVal::Uuid(backup.id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Backup {} not found", backup.id)));
    }
    Ok(backup.clone())
}
