use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PxeProfile {
    pub id: Uuid,
    pub name: String,
    pub talos_version: String,
    pub arch: String,
    pub kernel_url: String,
    pub initramfs_url: String,
    pub cmdline: String,
    pub enabled: bool,
    pub assets_ready: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn create(pool: &DbPool, p: &PxeProfile) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO pxe_profiles (id, name, talos_version, arch, kernel_url, initramfs_url, cmdline, enabled, assets_ready, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            SqlVal::Uuid(p.id),
            SqlVal::text(&p.name),
            SqlVal::text(&p.talos_version),
            SqlVal::text(&p.arch),
            SqlVal::text(&p.kernel_url),
            SqlVal::text(&p.initramfs_url),
            SqlVal::text(&p.cmdline),
            SqlVal::Bool(p.enabled),
            SqlVal::Bool(p.assets_ready),
            SqlVal::DateTime(p.created_at),
            SqlVal::DateTime(p.updated_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn get(pool: &DbPool, id: Uuid) -> Result<Option<PxeProfile>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM pxe_profiles WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn list(pool: &DbPool) -> Result<Vec<PxeProfile>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM pxe_profiles ORDER BY created_at DESC",
        &[],
    )
    .await
}

pub async fn update(pool: &DbPool, p: &PxeProfile) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE pxe_profiles SET name = ?, talos_version = ?, arch = ?, kernel_url = ?, initramfs_url = ?, cmdline = ?, enabled = ?, assets_ready = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(&p.name),
                SqlVal::text(&p.talos_version),
                SqlVal::text(&p.arch),
                SqlVal::text(&p.kernel_url),
                SqlVal::text(&p.initramfs_url),
                SqlVal::text(&p.cmdline),
                SqlVal::Bool(p.enabled),
                SqlVal::Bool(p.assets_ready),
                SqlVal::DateTime(p.updated_at),
                SqlVal::Uuid(p.id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("PXE profile {} not found", p.id)));
    }
    Ok(())
}

pub async fn delete(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    pool.execute("DELETE FROM pxe_profiles WHERE id = ?", &[SqlVal::Uuid(id)])
        .await?;
    Ok(())
}
