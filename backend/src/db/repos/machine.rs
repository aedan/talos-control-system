use sqlx::SqlitePool;
use crate::db::models::machine::Machine;
use crate::AppError;

pub async fn create(pool: &SqlitePool, machine: &Machine) -> Result<Machine, AppError> {
    let existing = get(pool, machine.id).await?;
    if existing.is_some() {
        return Err(AppError::InvalidInput(format!("Machine {} already exists", machine.id)));
    }

    let result = sqlx::query(
        "INSERT INTO machines (id, system_uuid, machine_type, cluster_id, status, talos_version, secure_boot, siderolink_connected, address, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(machine.id)
    .bind(&machine.system_uuid)
    .bind(&machine.machine_type)
    .bind(machine.cluster_id)
    .bind(&machine.status)
    .bind(&machine.talos_version)
    .bind(machine.secure_boot)
    .bind(machine.siderolink_connected)
    .bind(&machine.address)
    .bind(machine.created_at)
    .bind(machine.updated_at)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Internal("Failed to create machine".to_string()));
    }

    Ok(machine.clone())
}

pub async fn get(pool: &SqlitePool, id: uuid::Uuid) -> Result<Option<Machine>, AppError> {
    let machine = sqlx::query_as(
        "SELECT * FROM machines WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(machine)
}

pub async fn get_by_system_uuid(pool: &SqlitePool, system_uuid: &str) -> Result<Option<Machine>, AppError> {
    let machine = sqlx::query_as(
        "SELECT * FROM machines WHERE system_uuid = ?"
    )
    .bind(system_uuid)
    .fetch_optional(pool)
    .await?;

    Ok(machine)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Machine>, AppError> {
    let machines = sqlx::query_as::<_, Machine>(
        "SELECT * FROM machines ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await?;

    Ok(machines)
}

pub async fn list_by_cluster(pool: &SqlitePool, cluster_id: uuid::Uuid) -> Result<Vec<Machine>, AppError> {
    let machines = sqlx::query_as::<_, Machine>(
        "SELECT * FROM machines WHERE cluster_id = ? ORDER BY created_at DESC"
    )
    .bind(cluster_id)
    .fetch_all(pool)
    .await?;

    Ok(machines)
}

pub async fn update(pool: &SqlitePool, machine: &Machine) -> Result<Machine, AppError> {
    let result = sqlx::query(
        "UPDATE machines SET system_uuid = ?, machine_type = ?, cluster_id = ?, status = ?, talos_version = ?, secure_boot = ?, siderolink_connected = ?, address = ?, updated_at = ?
         WHERE id = ?"
    )
    .bind(&machine.system_uuid)
    .bind(&machine.machine_type)
    .bind(machine.cluster_id)
    .bind(&machine.status)
    .bind(&machine.talos_version)
    .bind(machine.secure_boot)
    .bind(machine.siderolink_connected)
    .bind(&machine.address)
    .bind(machine.updated_at)
    .bind(machine.id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Machine {} not found", machine.id)));
    }

    Ok(machine.clone())
}

pub async fn delete(pool: &SqlitePool, id: uuid::Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        "DELETE FROM machines WHERE id = ?"
    )
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Machine {} not found", id)));
    }

    Ok(())
}
