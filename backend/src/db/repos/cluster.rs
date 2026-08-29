use crate::db::models::cluster::Cluster;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn create(pool: &DbPool, cluster: &Cluster) -> Result<Cluster, AppError> {
    if get(pool, cluster.id).await?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "Cluster {} already exists",
            cluster.id
        )));
    }
    let n = pool
        .execute(
            "INSERT INTO clusters (id, name, control_plane_version, talos_version, status, control_plane_size, worker_size, talosconfig, kubeconfig, backup_retention, backup_schedule_hours, last_auto_backup_at, created_at, updated_at, network_config, factory_modules)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(cluster.id),
                SqlVal::text(&cluster.name),
                SqlVal::text(&cluster.control_plane_version),
                SqlVal::text(&cluster.talos_version),
                SqlVal::text(&cluster.status),
                SqlVal::I32(cluster.control_plane_size),
                SqlVal::I32(cluster.worker_size),
                SqlVal::OptText(cluster.talosconfig.clone()),
                SqlVal::OptText(cluster.kubeconfig.clone()),
                SqlVal::OptI32(cluster.backup_retention),
                SqlVal::OptI32(cluster.backup_schedule_hours),
                SqlVal::OptDateTime(cluster.last_auto_backup_at),
                SqlVal::DateTime(cluster.created_at),
                SqlVal::DateTime(cluster.updated_at),
                SqlVal::OptText(cluster.network_config.clone()),
                SqlVal::OptText(cluster.factory_modules.clone()),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Internal("Failed to create cluster".into()));
    }
    Ok(cluster.clone())
}

pub async fn get(pool: &DbPool, id: uuid::Uuid) -> Result<Option<Cluster>, AppError> {
    pool.fetch_optional_as("SELECT * FROM clusters WHERE id = ?", &[SqlVal::Uuid(id)])
        .await
}

pub async fn list(pool: &DbPool) -> Result<Vec<Cluster>, AppError> {
    pool.fetch_all_as("SELECT * FROM clusters ORDER BY created_at DESC", &[])
        .await
}

pub async fn update(pool: &DbPool, cluster: &Cluster) -> Result<Cluster, AppError> {
    let n = pool
        .execute(
            "UPDATE clusters SET name = ?, status = ?, control_plane_size = ?, worker_size = ?, backup_retention = ?, backup_schedule_hours = ?, last_auto_backup_at = ?, network_config = COALESCE(?, network_config), factory_modules = ?, updated_at = ?
              WHERE id = ?",
            &[
                SqlVal::text(&cluster.name),
                SqlVal::text(&cluster.status),
                SqlVal::I32(cluster.control_plane_size),
                SqlVal::I32(cluster.worker_size),
                SqlVal::OptI32(cluster.backup_retention),
                SqlVal::OptI32(cluster.backup_schedule_hours),
                SqlVal::OptDateTime(cluster.last_auto_backup_at),
                SqlVal::OptText(cluster.network_config.clone()),
                SqlVal::OptText(cluster.factory_modules.clone()),
                SqlVal::DateTime(cluster.updated_at),
                SqlVal::Uuid(cluster.id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", cluster.id)));
    }
    Ok(cluster.clone())
}

pub async fn delete(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let n = pool
        .execute("DELETE FROM clusters WHERE id = ?", &[SqlVal::Uuid(id)])
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }
    Ok(())
}

pub async fn update_status(pool: &DbPool, id: uuid::Uuid, status: &str) -> Result<(), AppError> {
    pool.execute(
        "UPDATE clusters SET status = ?, updated_at = ? WHERE id = ?",
        &[
            SqlVal::text(status),
            SqlVal::DateTime(chrono::Utc::now()),
            SqlVal::Uuid(id),
        ],
    )
    .await?;
    Ok(())
}

pub async fn set_talosconfig(
    pool: &DbPool,
    id: uuid::Uuid,
    talosconfig: &str,
) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE clusters SET talosconfig = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(talosconfig),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }
    Ok(())
}

pub async fn set_kubeconfig(
    pool: &DbPool,
    id: uuid::Uuid,
    kubeconfig: &str,
) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE clusters SET kubeconfig = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(kubeconfig),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }
    Ok(())
}

pub async fn set_backup_schedule(
    pool: &DbPool,
    id: uuid::Uuid,
    schedule_hours: Option<i32>,
    retention: Option<i32>,
) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE clusters SET backup_schedule_hours = ?, backup_retention = COALESCE(?, backup_retention), updated_at = ?
             WHERE id = ?",
            &[
                SqlVal::OptI32(schedule_hours),
                SqlVal::OptI32(retention),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }
    Ok(())
}

pub async fn mark_auto_backup(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    pool.execute(
        "UPDATE clusters SET last_auto_backup_at = ?, updated_at = ? WHERE id = ?",
        &[SqlVal::DateTime(now), SqlVal::DateTime(now), SqlVal::Uuid(id)],
    )
    .await?;
    Ok(())
}

pub async fn set_network_config(
    pool: &DbPool,
    id: uuid::Uuid,
    network_config: &str,
) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE clusters SET network_config = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(network_config),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }
    Ok(())
}

pub async fn list_with_backup_schedule(pool: &DbPool) -> Result<Vec<Cluster>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM clusters
         WHERE backup_schedule_hours IS NOT NULL AND backup_schedule_hours > 0
         ORDER BY name",
        &[],
    )
    .await
}
