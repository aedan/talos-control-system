use crate::db::models::machine_class::{MachineClass, MachineClassRow};
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn create(pool: &DbPool, mc: &MachineClass) -> Result<MachineClass, AppError> {
    if get(pool, mc.id).await?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "Machine class {} already exists",
            mc.id
        )));
    }
    let allowed_roles = serde_json::to_string(&mc.allowed_roles)
        .map_err(|e| AppError::InvalidInput(format!("Failed to serialize allowed_roles: {}", e)))?;
    let n = pool
        .execute(
            "INSERT INTO machine_classes (id, name, description, min_cpu, min_memory, min_disk, arch, secure_boot, allowed_roles, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(mc.id),
                SqlVal::text(&mc.name),
                SqlVal::text(&mc.description),
                SqlVal::I32(mc.min_cpu),
                SqlVal::I64(mc.min_memory),
                SqlVal::I64(mc.min_disk),
                SqlVal::text(&mc.arch),
                SqlVal::Bool(mc.secure_boot),
                SqlVal::text(allowed_roles),
                SqlVal::DateTime(mc.created_at),
                SqlVal::DateTime(mc.updated_at),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Internal("Failed to create machine class".into()));
    }
    Ok(mc.clone())
}

pub async fn get(pool: &DbPool, id: uuid::Uuid) -> Result<Option<MachineClass>, AppError> {
    let row: Option<MachineClassRow> = pool
        .fetch_optional_as("SELECT * FROM machine_classes WHERE id = ?", &[SqlVal::Uuid(id)])
        .await?;
    Ok(row.map(MachineClass::from))
}

pub async fn list(pool: &DbPool) -> Result<Vec<MachineClass>, AppError> {
    let rows: Vec<MachineClassRow> = pool
        .fetch_all_as(
            "SELECT * FROM machine_classes ORDER BY created_at DESC",
            &[],
        )
        .await?;
    Ok(rows.into_iter().map(MachineClass::from).collect())
}

pub async fn update(pool: &DbPool, mc: &MachineClass) -> Result<MachineClass, AppError> {
    let allowed_roles = serde_json::to_string(&mc.allowed_roles)
        .map_err(|e| AppError::InvalidInput(format!("Failed to serialize allowed_roles: {}", e)))?;
    let n = pool
        .execute(
            "UPDATE machine_classes SET name = ?, description = ?, min_cpu = ?, min_memory = ?, min_disk = ?, arch = ?, secure_boot = ?, allowed_roles = ?, updated_at = ?
             WHERE id = ?",
            &[
                SqlVal::text(&mc.name),
                SqlVal::text(&mc.description),
                SqlVal::I32(mc.min_cpu),
                SqlVal::I64(mc.min_memory),
                SqlVal::I64(mc.min_disk),
                SqlVal::text(&mc.arch),
                SqlVal::Bool(mc.secure_boot),
                SqlVal::text(allowed_roles),
                SqlVal::DateTime(mc.updated_at),
                SqlVal::Uuid(mc.id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!(
            "Machine class {} not found",
            mc.id
        )));
    }
    Ok(mc.clone())
}

pub async fn delete(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let n = pool
        .execute("DELETE FROM machine_classes WHERE id = ?", &[SqlVal::Uuid(id)])
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Machine class {} not found", id)));
    }
    Ok(())
}

pub async fn update_name(pool: &DbPool, id: uuid::Uuid, name: &str) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE machine_classes SET name = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(name),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Machine class {} not found", id)));
    }
    Ok(())
}
