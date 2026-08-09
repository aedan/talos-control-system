use sqlx::SqlitePool;
use crate::db::models::machine_class::{MachineClass, MachineClassRow};
use crate::AppError;

pub async fn create(pool: &SqlitePool, mc: &MachineClass) -> Result<MachineClass, AppError> {
    let existing = get(pool, mc.id).await?;
    if existing.is_some() {
        return Err(AppError::InvalidInput(format!(
            "Machine class {} already exists",
            mc.id
        )));
    }

    let allowed_roles = serde_json::to_string(&mc.allowed_roles)
        .map_err(|e| AppError::InvalidInput(format!("Failed to serialize allowed_roles: {}", e)))?;

    let result = sqlx::query(
        "INSERT INTO machine_classes (id, name, description, min_cpu, min_memory, min_disk, arch, secure_boot, allowed_roles, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(mc.id)
    .bind(&mc.name)
    .bind(&mc.description)
    .bind(mc.min_cpu)
    .bind(mc.min_memory)
    .bind(mc.min_disk)
    .bind(&mc.arch)
    .bind(mc.secure_boot)
    .bind(&allowed_roles)
    .bind(mc.created_at)
    .bind(mc.updated_at)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Internal("Failed to create machine class".to_string()));
    }

    Ok(mc.clone())
}

pub async fn get(pool: &SqlitePool, id: uuid::Uuid) -> Result<Option<MachineClass>, AppError> {
    let row = sqlx::query_as::<_, MachineClassRow>(
        "SELECT * FROM machine_classes WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(MachineClass::from))
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<MachineClass>, AppError> {
    let rows = sqlx::query_as::<_, MachineClassRow>(
        "SELECT * FROM machine_classes ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(MachineClass::from).collect())
}

pub async fn update(pool: &SqlitePool, mc: &MachineClass) -> Result<MachineClass, AppError> {
    let allowed_roles = serde_json::to_string(&mc.allowed_roles)
        .map_err(|e| AppError::InvalidInput(format!("Failed to serialize allowed_roles: {}", e)))?;

    let result = sqlx::query(
        "UPDATE machine_classes SET name = ?, description = ?, min_cpu = ?, min_memory = ?, min_disk = ?, arch = ?, secure_boot = ?, allowed_roles = ?, updated_at = ?
         WHERE id = ?"
    )
    .bind(&mc.name)
    .bind(&mc.description)
    .bind(mc.min_cpu)
    .bind(mc.min_memory)
    .bind(mc.min_disk)
    .bind(&mc.arch)
    .bind(mc.secure_boot)
    .bind(&allowed_roles)
    .bind(mc.updated_at)
    .bind(mc.id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Machine class {} not found",
            mc.id
        )));
    }

    Ok(mc.clone())
}

pub async fn delete(pool: &SqlitePool, id: uuid::Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        "DELETE FROM machine_classes WHERE id = ?"
    )
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Machine class {} not found",
            id
        )));
    }

    Ok(())
}

pub async fn update_name(pool: &SqlitePool, id: uuid::Uuid, name: &str) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    let result = sqlx::query(
        "UPDATE machine_classes SET name = ?, updated_at = ? WHERE id = ?"
    )
    .bind(name)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Machine class {} not found",
            id
        )));
    }

    Ok(())
}
