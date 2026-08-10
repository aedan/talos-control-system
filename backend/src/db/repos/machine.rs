use crate::db::models::machine::Machine;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn create(pool: &DbPool, machine: &Machine) -> Result<Machine, AppError> {
    if get(pool, machine.id).await?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "Machine {} already exists",
            machine.id
        )));
    }
    let n = pool
        .execute(
            "INSERT INTO machines (id, system_uuid, machine_type, cluster_id, status, talos_version, secure_boot, siderolink_connected, address, install_disk, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(machine.id),
                SqlVal::text(&machine.system_uuid),
                SqlVal::text(&machine.machine_type),
                SqlVal::OptUuid(machine.cluster_id),
                SqlVal::text(&machine.status),
                SqlVal::text(&machine.talos_version),
                SqlVal::Bool(machine.secure_boot),
                SqlVal::Bool(machine.siderolink_connected),
                SqlVal::text(&machine.address),
                SqlVal::text(&machine.install_disk),
                SqlVal::DateTime(machine.created_at),
                SqlVal::DateTime(machine.updated_at),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Internal("Failed to create machine".into()));
    }
    Ok(machine.clone())
}

pub async fn get(pool: &DbPool, id: uuid::Uuid) -> Result<Option<Machine>, AppError> {
    pool.fetch_optional_as("SELECT * FROM machines WHERE id = ?", &[SqlVal::Uuid(id)])
        .await
}

pub async fn get_by_system_uuid(
    pool: &DbPool,
    system_uuid: &str,
) -> Result<Option<Machine>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM machines WHERE system_uuid = ?",
        &[SqlVal::text(system_uuid)],
    )
    .await
}

pub async fn list(pool: &DbPool) -> Result<Vec<Machine>, AppError> {
    pool.fetch_all_as("SELECT * FROM machines ORDER BY created_at DESC", &[])
        .await
}

pub async fn list_by_cluster(
    pool: &DbPool,
    cluster_id: uuid::Uuid,
) -> Result<Vec<Machine>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM machines WHERE cluster_id = ? ORDER BY created_at DESC",
        &[SqlVal::Uuid(cluster_id)],
    )
    .await
}

pub async fn update(pool: &DbPool, machine: &Machine) -> Result<Machine, AppError> {
    let n = pool
        .execute(
            "UPDATE machines SET system_uuid = ?, machine_type = ?, cluster_id = ?, status = ?, talos_version = ?, secure_boot = ?, siderolink_connected = ?, address = ?, install_disk = ?, updated_at = ?
             WHERE id = ?",
            &[
                SqlVal::text(&machine.system_uuid),
                SqlVal::text(&machine.machine_type),
                SqlVal::OptUuid(machine.cluster_id),
                SqlVal::text(&machine.status),
                SqlVal::text(&machine.talos_version),
                SqlVal::Bool(machine.secure_boot),
                SqlVal::Bool(machine.siderolink_connected),
                SqlVal::text(&machine.address),
                SqlVal::text(&machine.install_disk),
                SqlVal::DateTime(machine.updated_at),
                SqlVal::Uuid(machine.id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Machine {} not found", machine.id)));
    }
    Ok(machine.clone())
}

pub async fn delete(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let n = pool
        .execute("DELETE FROM machines WHERE id = ?", &[SqlVal::Uuid(id)])
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Machine {} not found", id)));
    }
    Ok(())
}
