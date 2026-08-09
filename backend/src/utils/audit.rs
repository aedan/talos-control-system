//! Durable audit log backed by the `audit_logs` table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub user_email: String,
    pub action: String,
    pub resource: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFilter {
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
}

fn default_page() -> usize {
    1
}
fn default_per_page() -> usize {
    50
}

pub async fn log_action(
    pool: &SqlitePool,
    email: &str,
    action: &str,
    resource: &str,
    details: &str,
) {
    let id = Uuid::new_v4();
    let now = Utc::now();

    tracing::info!(
        audit_action = %action,
        audit_resource = %resource,
        audit_user = %email,
        "Audit"
    );

    if let Err(e) = sqlx::query(
        "INSERT INTO audit_logs (id, user_id, resource_type, resource_id, action, details, created_at)
         VALUES (?, NULL, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(resource)
    .bind("") // resource_id optional legacy column
    .bind(action)
    .bind(format!("{} | user={}", details, email))
    .bind(now)
    .execute(pool)
    .await
    {
        tracing::warn!(error = %e, "Failed to write audit log");
    }
}

pub async fn get_entries(
    pool: &SqlitePool,
    filter: &AuditFilter,
) -> Result<(Vec<AuditEntry>, usize), AppError> {
    // Load recent rows then filter in memory (alpha-scale)
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, action, resource_type, COALESCE(details, ''), created_at
         FROM audit_logs
         ORDER BY created_at DESC
         LIMIT 5000",
    )
    .fetch_all(pool)
    .await?;

    let mut entries: Vec<AuditEntry> = Vec::new();
    for (id, action, resource, details, created_at) in rows {
        let user_email = details
            .split(" | user=")
            .nth(1)
            .unwrap_or("system")
            .to_string();
        let clean_details = details
            .split(" | user=")
            .next()
            .unwrap_or(&details)
            .to_string();
        let timestamp = DateTime::parse_from_rfc3339(&created_at)
            .map(|d| d.with_timezone(&Utc))
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&created_at, "%Y-%m-%d %H:%M:%S")
                    .map(|n| DateTime::from_naive_utc_and_offset(n, Utc))
            })
            .unwrap_or_else(|_| Utc::now());

        let uuid = Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4());
        entries.push(AuditEntry {
            id: uuid,
            timestamp,
            user_email,
            action,
            resource,
            details: clean_details,
        });
    }

    let filtered: Vec<AuditEntry> = entries
        .into_iter()
        .filter(|entry| {
            if !filter.user.is_empty() && entry.user_email != filter.user {
                return false;
            }
            if !filter.action.is_empty() && entry.action != filter.action {
                return false;
            }
            if !filter.from.is_empty() {
                if let Ok(from) = DateTime::parse_from_rfc3339(&filter.from) {
                    if entry.timestamp < from.with_timezone(&Utc) {
                        return false;
                    }
                }
            }
            if !filter.to.is_empty() {
                if let Ok(to) = DateTime::parse_from_rfc3339(&filter.to) {
                    if entry.timestamp > to.with_timezone(&Utc) {
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    let total = filtered.len();
    let start = (filter.page.saturating_sub(1)) * filter.per_page;
    let end = start.saturating_add(filter.per_page).min(filtered.len());
    let page_entries = if start < filtered.len() {
        filtered[start..end].to_vec()
    } else {
        vec![]
    };

    Ok((page_entries, total))
}

pub async fn clear_all(pool: &SqlitePool) -> Result<(), AppError> {
    sqlx::query("DELETE FROM audit_logs").execute(pool).await?;
    Ok(())
}

pub async fn count(pool: &SqlitePool) -> Result<usize, AppError> {
    let (c,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(pool)
        .await?;
    Ok(c as usize)
}
