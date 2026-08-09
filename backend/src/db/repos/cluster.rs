use sqlx::SqlitePool;
use crate::db::models::cluster::Cluster;
use crate::db::models::machine::Machine;
use crate::AppError;

pub async fn create(pool: &SqlitePool, cluster: &Cluster) -> Result<Cluster, AppError> {
    let existing = get(pool, cluster.id).await?;
    if existing.is_some() {
        return Err(AppError::InvalidInput(format!("Cluster {} already exists", cluster.id)));
    }

    let result = sqlx::query(
        "INSERT INTO clusters (id, name, control_plane_version, talos_version, status, control_plane_size, worker_size, talosconfig, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(cluster.id)
    .bind(&cluster.name)
    .bind(&cluster.control_plane_version)
    .bind(&cluster.talos_version)
    .bind(&cluster.status)
    .bind(cluster.control_plane_size)
    .bind(cluster.worker_size)
    .bind(&cluster.talosconfig)
    .bind(cluster.created_at)
    .bind(cluster.updated_at)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Internal("Failed to create cluster".to_string()));
    }

    Ok(cluster.clone())
}

pub async fn get(pool: &SqlitePool, id: uuid::Uuid) -> Result<Option<Cluster>, AppError> {
    let cluster = sqlx::query_as(
        "SELECT * FROM clusters WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(cluster)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Cluster>, AppError> {
    let clusters = sqlx::query_as::<_, Cluster>(
        "SELECT * FROM clusters ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await?;

    Ok(clusters)
}

pub async fn update(pool: &SqlitePool, cluster: &Cluster) -> Result<Cluster, AppError> {
    let result = sqlx::query(
        "UPDATE clusters SET name = ?, status = ?, control_plane_size = ?, worker_size = ?, updated_at = ?
         WHERE id = ?"
    )
    .bind(&cluster.name)
    .bind(&cluster.status)
    .bind(cluster.control_plane_size)
    .bind(cluster.worker_size)
    .bind(cluster.updated_at)
    .bind(cluster.id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", cluster.id)));
    }

    Ok(cluster.clone())
}

pub async fn delete(pool: &SqlitePool, id: uuid::Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        "DELETE FROM clusters WHERE id = ?"
    )
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }

    Ok(())
}

pub async fn update_status(pool: &SqlitePool, id: uuid::Uuid, status: &str) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    sqlx::query(
        "UPDATE clusters SET status = ?, updated_at = ? WHERE id = ?"
    )
    .bind(status)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn set_talosconfig(
    pool: &SqlitePool,
    id: uuid::Uuid,
    talosconfig: &str,
) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    let result = sqlx::query(
        "UPDATE clusters SET talosconfig = ?, updated_at = ? WHERE id = ?"
    )
    .bind(talosconfig)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Cluster {} not found", id)));
    }
    Ok(())
}
