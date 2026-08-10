use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

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

pub async fn list_peers(pool: &SqlitePool) -> Result<Vec<SiderolinkPeer>, AppError> {
    Ok(sqlx::query_as::<_, SiderolinkPeer>(
        "SELECT * FROM siderolink_peers ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn upsert_peer(pool: &SqlitePool, peer: &SiderolinkPeer) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO siderolink_peers (id, system_uuid, public_key, assigned_ip, last_seen, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           public_key = excluded.public_key,
           assigned_ip = excluded.assigned_ip,
           last_seen = excluded.last_seen",
    )
    .bind(peer.id)
    .bind(&peer.system_uuid)
    .bind(&peer.public_key)
    .bind(&peer.assigned_ip)
    .bind(peer.last_seen)
    .bind(peer.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_by_uuid(
    pool: &SqlitePool,
    system_uuid: &str,
) -> Result<Option<SiderolinkPeer>, AppError> {
    Ok(sqlx::query_as::<_, SiderolinkPeer>(
        "SELECT * FROM siderolink_peers WHERE system_uuid = ?",
    )
    .bind(system_uuid)
    .fetch_optional(pool)
    .await?)
}

pub async fn touch(pool: &SqlitePool, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE siderolink_peers SET last_seen = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_token(
    pool: &SqlitePool,
    token: &str,
    label: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO siderolink_join_tokens (token, label, expires_at, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(token)
    .bind(label)
    .bind(expires_at)
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_tokens(pool: &SqlitePool) -> Result<Vec<SiderolinkJoinToken>, AppError> {
    Ok(sqlx::query_as::<_, SiderolinkJoinToken>(
        "SELECT * FROM siderolink_join_tokens ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn validate_token(pool: &SqlitePool, token: &str) -> Result<bool, AppError> {
    let row = sqlx::query_as::<_, SiderolinkJoinToken>(
        "SELECT * FROM siderolink_join_tokens WHERE token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        None => false,
        Some(t) => t.expires_at.map(|e| e > Utc::now()).unwrap_or(true),
    })
}

pub async fn delete_token(pool: &SqlitePool, token: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM siderolink_join_tokens WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn next_ip(pool: &SqlitePool, subnet_start: u32) -> Result<String, AppError> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM siderolink_peers")
        .fetch_one(pool)
        .await?;
    let ip = subnet_start.saturating_add(2 + count.0 as u32);
    let b = ip.to_be_bytes();
    Ok(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]))
}
