use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DhcpLease {
    pub mac: String,
    pub ip: String,
    pub hostname: String,
    pub machine_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert(pool: &DbPool, lease: &DhcpLease) -> Result<(), AppError> {
    // delete + insert for dialect simplicity
    let _ = pool
        .execute("DELETE FROM dhcp_leases WHERE mac = ?", &[SqlVal::text(&lease.mac)])
        .await;
    pool.execute(
        "INSERT INTO dhcp_leases (mac, ip, hostname, machine_id, expires_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        &[
            SqlVal::text(&lease.mac),
            SqlVal::text(&lease.ip),
            SqlVal::text(&lease.hostname),
            SqlVal::OptUuid(lease.machine_id),
            SqlVal::DateTime(lease.expires_at),
            SqlVal::DateTime(lease.created_at),
            SqlVal::DateTime(lease.updated_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn get_by_mac(pool: &DbPool, mac: &str) -> Result<Option<DhcpLease>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM dhcp_leases WHERE mac = ?",
        &[SqlVal::text(mac)],
    )
    .await
}

pub async fn list(pool: &DbPool) -> Result<Vec<DhcpLease>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM dhcp_leases ORDER BY updated_at DESC",
        &[],
    )
    .await
}

pub async fn list_active(pool: &DbPool, now: DateTime<Utc>) -> Result<Vec<DhcpLease>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM dhcp_leases WHERE expires_at > ? ORDER BY ip",
        &[SqlVal::DateTime(now)],
    )
    .await
}
