use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SiderolinkPeer {
    pub id: Uuid,
    pub system_uuid: String,
    pub public_key: String,
    pub assigned_ip: String,
    pub last_seen: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SiderolinkJoinToken {
    pub token: String,
    pub label: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub async fn list_peers(pool: &DbPool) -> Result<Vec<SiderolinkPeer>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM siderolink_peers ORDER BY created_at DESC",
        &[],
    )
    .await
}

pub async fn upsert_peer(pool: &DbPool, peer: &SiderolinkPeer) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO siderolink_peers (id, system_uuid, public_key, assigned_ip, last_seen, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           public_key = excluded.public_key,
           assigned_ip = excluded.assigned_ip,
           last_seen = excluded.last_seen",
        &[
            SqlVal::Uuid(peer.id),
            SqlVal::text(&peer.system_uuid),
            SqlVal::text(&peer.public_key),
            SqlVal::text(&peer.assigned_ip),
            SqlVal::DateTime(peer.last_seen),
            SqlVal::DateTime(peer.created_at),
        ],
    )
    .await?;
    Ok(())
}

pub async fn find_by_uuid(
    pool: &DbPool,
    system_uuid: &str,
) -> Result<Option<SiderolinkPeer>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM siderolink_peers WHERE system_uuid = ?",
        &[SqlVal::text(system_uuid)],
    )
    .await
}

pub async fn touch(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    pool.execute(
        "UPDATE siderolink_peers SET last_seen = ? WHERE id = ?",
        &[SqlVal::DateTime(Utc::now()), SqlVal::Uuid(id)],
    )
    .await?;
    Ok(())
}

pub async fn create_token(
    pool: &DbPool,
    token: &str,
    label: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    pool.execute(
        "INSERT INTO siderolink_join_tokens (token, label, expires_at, created_at) VALUES (?, ?, ?, ?)",
        &[
            SqlVal::text(token),
            SqlVal::OptText(label.map(|s| s.to_string())),
            SqlVal::OptDateTime(expires_at),
            SqlVal::DateTime(Utc::now()),
        ],
    )
    .await?;
    Ok(())
}

pub async fn list_tokens(pool: &DbPool) -> Result<Vec<SiderolinkJoinToken>, AppError> {
    pool.fetch_all_as(
        "SELECT * FROM siderolink_join_tokens ORDER BY created_at DESC",
        &[],
    )
    .await
}

pub async fn validate_token(pool: &DbPool, token: &str) -> Result<bool, AppError> {
    let row: Option<SiderolinkJoinToken> = pool
        .fetch_optional_as(
            "SELECT * FROM siderolink_join_tokens WHERE token = ?",
            &[SqlVal::text(token)],
        )
        .await?;
    Ok(match row {
        None => false,
        Some(t) => t.expires_at.map(|e| e > Utc::now()).unwrap_or(true),
    })
}

pub async fn delete_token(pool: &DbPool, token: &str) -> Result<(), AppError> {
    pool.execute(
        "DELETE FROM siderolink_join_tokens WHERE token = ?",
        &[SqlVal::text(token)],
    )
    .await?;
    Ok(())
}

pub async fn next_ip(pool: &DbPool, subnet_start: u32) -> Result<String, AppError> {
    let count = pool
        .fetch_scalar_i64("SELECT COUNT(*) FROM siderolink_peers", &[])
        .await?;
    let ip = subnet_start.saturating_add(2 + count as u32);
    let b = ip.to_be_bytes();
    Ok(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]))
}

pub async fn delete_peer(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    pool.execute(
        "DELETE FROM siderolink_peers WHERE id = ?",
        &[SqlVal::Uuid(id)],
    )
    .await?;
    Ok(())
}
